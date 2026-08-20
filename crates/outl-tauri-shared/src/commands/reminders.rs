//! `remind::` command bodies — shared by desktop and mobile.
//!
//! Every one of these is a thin shell over `outl_actions::reminders`.
//! The scheduling decision (*when does this fire*) is made there, once,
//! for every client; this module only translates DTOs and routes the
//! mutation through the usual `finish_in_page` path so the `.md` and
//! sidecar stay in step.
//!
//! Delivering the notification is **not** here: that's per-OS and lives
//! in each client's Tauri layer. What is shared is the answer to "what
//! should fire and when", which both clients read from
//! [`list_reminders`].

use outl_actions::reminders::{
    scan_reminders, snooze, snooze_until, FiredLog, Reminder, SnoozePreset, Urgency,
};
use outl_actions::todo::{set_todo, TodoState};
use outl_actions::{clock, edit_text, set_property};
use outl_core::property::PropValue;
use serde::{Deserialize, Serialize};

use crate::helpers::{finish_in_page, parse_node_id, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::PageView;

/// Wire shape of one scheduled reminder.
///
/// Times are ISO-8601 **local** strings (`2026-12-12T15:00:00`), not
/// epoch numbers: the frontend renders them verbatim in the user's
/// wall clock, and re-deriving a local time from an epoch in JS would
/// re-introduce the timezone bug `outl_actions::clock` exists to fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderDto {
    pub block_id: String,
    pub page_slug: String,
    pub page_title: String,
    pub text: String,
    /// The same body with inline syntax flattened — what a
    /// notification banner shows, so no `[[brackets]]` reach a lock
    /// screen.
    pub plain_text: String,
    /// The `remind::` value verbatim, e.g. `"3pm every 1h until DONE"`.
    pub rule: String,
    /// `YYYY-MM-DD` the rule is anchored to.
    pub anchor_date: String,
    pub done: bool,
    /// Local ISO datetime of the next fire, or `null` when finished.
    pub next_fire: Option<String>,
    /// Local ISO datetime the snooze runs until, or `null`.
    pub snoozed_until: Option<String>,
    /// `"overdue"` / `"soon"` / `"later"` / `"finished"` — computed in
    /// Rust so every client paints the same row the same colour.
    pub urgency: Urgency,
}

impl From<Reminder> for ReminderDto {
    fn from(r: Reminder) -> Self {
        Self {
            block_id: r.block_id.to_string(),
            page_slug: r.page_slug,
            page_title: r.page_title,
            text: r.text,
            plain_text: r.plain_text,
            rule: r.rule_text,
            anchor_date: r.anchor_date.format("%Y-%m-%d").to_string(),
            done: r.done,
            next_fire: r
                .next_fire
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()),
            urgency: r.urgency,
            snoozed_until: r
                .snoozed_until_ms
                .and_then(outl_actions::reminders::epoch_ms_to_local_naive)
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string()),
        }
    }
}

/// Device-local delivery preferences, surfaced so the frontend can
/// show "reminders are off" instead of an empty list the user can't
/// explain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderSettingsDto {
    pub enabled: bool,
    /// `"22:00-07:00"` or `""`.
    pub quiet_hours: String,
}

/// Every reminder in the workspace, soonest first.
///
/// Read-only, so it passes an empty [`FiredLog`]: the list answers
/// "what is scheduled", not "what has this device already delivered".
/// The delivery loop passes its real fired cache.
pub fn list_reminders<S: AppHost>(state: &S) -> Result<Vec<ReminderDto>, String> {
    let cfg = outl_config::load();
    let quiet = cfg.reminders.quiet_window();
    let now = clock::now_local().naive_local();
    with_ws(state, |ws| {
        Ok(scan_reminders(ws, &FiredLog::new(), quiet, now)
            .into_iter()
            .map(ReminderDto::from)
            .collect())
    })
}

/// This device's reminder settings. Reads `config.toml`, so it needs
/// no workspace and no host.
pub fn reminder_settings() -> ReminderSettingsDto {
    let cfg = outl_config::load();
    ReminderSettingsDto {
        enabled: cfg.reminders.enabled,
        quiet_hours: cfg.reminders.quiet_hours.unwrap_or_default(),
    }
}

/// Write this device's reminder settings back to `config.toml`.
///
/// Exists because **mobile has no settings screen at all**. Without it
/// the sheet could tell the user "notifications are off on this
/// device" and offer no way to change that, with `config.toml` sitting
/// inside the iOS sandbox where they can't reach it. The desktop
/// writes the same two keys through its settings modal; this is the
/// narrow path for a client that has nowhere else to put them.
///
/// Reads the file first and writes back only these two fields, so it
/// can't clobber a hand-set timezone or relay URL (the same
/// restore-on-save policy the desktop's `Settings` adapter follows).
pub fn set_reminder_settings(
    enabled: bool,
    quiet_hours: &str,
) -> Result<ReminderSettingsDto, String> {
    let mut cfg = outl_config::load();
    cfg.reminders.enabled = enabled;
    cfg.reminders.quiet_hours = Some(quiet_hours.trim().to_string()).filter(|q| !q.is_empty());
    outl_config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(reminder_settings())
}

/// The snooze options, in render order, straight off
/// [`outl_actions::reminders::SnoozePreset`].
///
/// The clients render `label` and send `id` back to [`snooze_reminder`]
/// — they never compute the instant. "Tomorrow 9am" is a wall time,
/// not an offset, and a client doing its own arithmetic gets it wrong
/// the moment you tap it at 3am.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnoozePresetDto {
    pub id: String,
    pub label: String,
}

/// Every snooze preset, same list on every client.
pub fn snooze_presets() -> Vec<SnoozePresetDto> {
    SnoozePreset::all()
        .into_iter()
        .map(|p| SnoozePresetDto {
            id: p.id().to_string(),
            label: p.label().to_string(),
        })
        .collect()
}

/// Silence a block's reminder until the instant `preset` resolves to.
///
/// Goes through `Op::SnoozeRemind`, so the same block goes quiet on
/// every paired device — snoozing on the phone must not leave the
/// laptop buzzing.
///
/// Takes a preset **id**, not a duration: resolution lives in
/// `outl_actions::reminders::SnoozePreset` so every client snoozes to
/// the same instant. An unknown id errors rather than falling back —
/// snoozing for a different span than the button said is worse than
/// doing nothing.
///
/// Takes no page id because it touches no `.md`: the snooze lives only
/// in the op log, by design (writing it into the markdown would put a
/// device-local *time* into the user's clean notes).
pub fn snooze_reminder<S: AppHost>(state: &S, block_id: &str, preset: &str) -> Result<(), String> {
    let node = parse_node_id(block_id)?;
    let preset =
        SnoozePreset::from_id(preset).ok_or_else(|| format!("unknown snooze preset: {preset}"))?;
    let until = preset.resolve(clock::now_local().naive_local());
    let hlc = state.hlc().clone();
    with_ws_mut(state, |ws| {
        snooze_until(ws, &hlc, node, until).map_err(|e| e.to_string())
    })
}

/// Clear a block's snooze so it resumes on its normal schedule.
pub fn clear_reminder_snooze<S: AppHost>(state: &S, block_id: &str) -> Result<(), String> {
    let node = parse_node_id(block_id)?;
    let hlc = state.hlc().clone();
    with_ws_mut(state, |ws| {
        snooze(ws, &hlc, node, None).map_err(|e| e.to_string())
    })
}

/// Set (or clear, with an empty `value`) any `key:: value` property on
/// a block, and return the refreshed page.
///
/// Generic on purpose. This started as a `remind::`-only command,
/// which meant the property chips every client now renders had nothing
/// to write through — you could *see* `priority:: high` and not change
/// it. A property is a property; the key is an argument.
///
/// Editing a `remind::` rule reschedules from scratch, which falls out
/// for free: the schedule is derived on every scan, never cached.
pub fn set_block_property<S: AppHost>(
    state: &S,
    page_id: &str,
    block_id: &str,
    key: &str,
    value: &str,
) -> Result<PageView, String> {
    let page = parse_node_id(page_id)?;
    let node = parse_node_id(block_id)?;
    let key = key.trim();
    if key.is_empty() {
        return Err("property key cannot be empty".to_string());
    }
    let value = {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| PropValue::Text(trimmed.to_string()))
    };
    let hlc = state.hlc().clone();
    finish_in_page(state, page, |ws| set_property(ws, &hlc, node, key, value))
}

/// Set (or, with an empty value, clear) a property on the **page**
/// itself: `icon::`, `type::`, `title::`, anything the user keeps as
/// page metadata.
///
/// Structural keys are refused rather than silently written. The page
/// root holds `page-slug` and `page-kind` in the same property map,
/// and those are the page's identity: `page-slug` is what the filename
/// and every `[[ref]]` resolve through. Letting an "edit property"
/// surface reach them turns a typo into a page that no link finds.
/// Renaming is `page_rename`, which moves the projection too.
pub fn set_page_property<S: AppHost>(
    state: &S,
    page_id: &str,
    key: &str,
    value: &str,
) -> Result<PageView, String> {
    let page = parse_node_id(page_id)?;
    let key = key.trim();
    if key.is_empty() {
        return Err("property key cannot be empty".to_string());
    }
    if outl_actions::tree::is_page_model_key(key) {
        return Err(format!(
            "`{key}` defines the page and cannot be edited as a property; rename the page instead"
        ));
    }
    let value = {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| PropValue::Text(trimmed.to_string()))
    };
    let hlc = state.hlc().clone();
    finish_in_page(state, page, |ws| set_property(ws, &hlc, page, key, value))
}

/// Mark a block DONE, which cancels every pending fire of its rule.
///
/// Deliberately **not** `toggle_todo`. The reminders list offers this
/// as "mark done (cancels the reminder)", and a rule can sit on a
/// block with no marker at all (`g r` attaches to whatever is
/// selected, and `remind:: 3pm` needs no task). Toggling that block
/// advanced it to `TODO` and the nag kept going — the button did the
/// opposite of its label. Setting the state outright is idempotent
/// and says what it means.
pub fn mark_block_done<S: AppHost>(
    state: &S,
    page_id: &str,
    block_id: &str,
) -> Result<PageView, String> {
    let page = parse_node_id(page_id)?;
    let node = parse_node_id(block_id)?;
    let hlc = state.hlc().clone();
    finish_in_page(state, page, |ws| {
        let current = ws.block_text(node).unwrap_or_default();
        let next = set_todo(&current, Some(TodoState::Done));
        edit_text(ws, &hlc, node, &next)
    })
}

/// Set (or clear) a block's `remind::` rule. Thin alias over
/// [`set_block_property`] kept because the authoring chords name the
/// reminder specifically and shouldn't have to know the key string.
pub fn set_block_remind<S: AppHost>(
    state: &S,
    page_id: &str,
    block_id: &str,
    rule: &str,
) -> Result<PageView, String> {
    set_block_property(state, page_id, block_id, outl_md::remind::REMIND_KEY, rule)
}
