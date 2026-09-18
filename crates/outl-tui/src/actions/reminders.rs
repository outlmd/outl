//! The reminders overlay: author a `remind::` rule, inspect every
//! scheduled one, snooze or complete from the list.
//!
//! **The TUI does deliver, while it's open.** A terminal session has no
//! background presence, so a rule that comes due with outl closed is
//! genuinely lost to this client. But the user sitting in the TUI all
//! day is exactly who should be interrupted, so a due reminder fires an
//! OSC 9 desktop notification (the sibling of the OSC 52 the yank path
//! already uses) plus a toast, which works even where OSC 9 doesn't.
//!
//! Every schedule question routes to `outl_actions::reminders` — the
//! same function the GUI clients call. Nothing about *when* a rule
//! fires is decided in this file.

use std::io::{IsTerminal, Write};

use crate::state::{App, Focus, Mode, Overlay, RemindersState, ToastKind};
use outl_actions::reminders::{scan_reminders, snooze_until, take_due, FiredLog, SnoozePreset};

/// The "nag me" preset behind `g R`, spelled in the `remind::`
/// grammar. The desktop hardcodes the same string in
/// `action-handlers.ts` — a constant can't cross the Rust/TS boundary
/// any more than a DTO field can, and a Tauri round-trip to fetch a
/// literal would be worse than the duplication.
const NAG_PRESET: &str = "now every 1h until DONE";

/// Default rule `g r` writes — one fire, this morning. Small on
/// purpose: the user is expected to edit the property line right
/// after, and a bare anchor is the shortest rule that does something.
const DEFAULT_RULE: &str = "9am";

impl App {
    /// `g r` — attach a starter `remind::` to the selected block.
    ///
    /// Idempotent in spirit: on a block that already has a rule this
    /// reports the existing one instead of overwriting it, so the
    /// chord can't silently discard a carefully typed schedule.
    pub(crate) fn insert_remind(&mut self) {
        self.write_remind(DEFAULT_RULE, false);
    }

    /// `g R` — the "nag me" preset, in one chord.
    ///
    /// Unlike `g r` this **does** overwrite: asking for the nag preset
    /// is an explicit escalation of whatever was there.
    pub(crate) fn insert_remind_nag(&mut self) {
        self.write_remind(NAG_PRESET, true);
    }

    fn write_remind(&mut self, rule: &str, overwrite: bool) {
        if !matches!(self.focus, Focus::Outline) || !matches!(self.mode, Mode::Normal) {
            return;
        }
        if !overwrite {
            if let Some(existing) = self.property_on_current_block(outl_md::remind::REMIND_KEY) {
                self.toast(ToastKind::Info, format!("already reminding: {existing}"));
                return;
            }
        }
        // Route through the same AST-first path `:prop` uses. Writing
        // straight to the op log left the `.md` (and therefore the
        // outline) without the property until some later save happened
        // to project it: the chord "worked" and rendered nothing. The
        // crate's rule is explicit about this — see the CLAUDE.md
        // "never mutate the op log directly" entry.
        self.set_property_on_current_block(outl_md::remind::REMIND_KEY, rule);
        // Point at the editor, because `g r` writes a starter rule the
        // user almost always wants to change and the property line
        // alone doesn't say how.
        self.toast(
            ToastKind::Info,
            format!("remind:: {rule} — edit with :prop remind <rule>"),
        );
    }

    /// Deliver anything that came due, on the event loop's tick.
    ///
    /// Cheap on the common path: `take_due` short-circuits before
    /// touching the workspace when the device has reminders off, and
    /// the device-local fired log means ticking often never
    /// double-buzzes.
    ///
    /// Deferred during Insert, for the same reason `pending_reload` is:
    /// a toast repaint mid-keystroke is the kind of interruption that
    /// makes people turn the feature off. The fire isn't lost, the next
    /// tick outside Insert picks it up (the fired log only records what
    /// actually went out).
    pub(crate) fn deliver_due_reminders(&mut self) {
        if matches!(self.mode, Mode::Insert { .. }) {
            return;
        }
        // Throttle before touching disk. The event loop ticks every
        // 750ms normally, every 16ms while an index rebuild is pending,
        // and once per keystroke; the sweep below reads `config.toml`,
        // reads the fired log, and scans the property map, so running
        // it per tick put two file reads in the keystroke path. A rule
        // resolves to the minute, so this cadence loses nothing.
        let now_instant = std::time::Instant::now();
        if let Some(last) = self.last_reminder_sweep {
            if now_instant.duration_since(last) < REMINDER_SWEEP_INTERVAL {
                return;
            }
        }
        self.last_reminder_sweep = Some(now_instant);

        let cfg = outl_config::load();
        if !cfg.reminders.enabled {
            return;
        }
        let now = outl_actions::clock::now_local().naive_local();
        let due = take_due(
            &self.workspace,
            &self.workspace_root,
            cfg.reminders.quiet_window(),
            now,
        );
        for r in due {
            // The flattened body, not the raw one: an OSC 9 banner
            // reading `ship it [[2026-12-12]] #fup` looks broken.
            let body = if r.plain_text.is_empty() {
                r.page_title.clone()
            } else {
                r.plain_text.clone()
            };
            emit_osc9(&format!("outl · {body}"));
            self.toast_for(
                ToastKind::Warning,
                format!("{} {body}", self.icons.bell),
                REMINDER_TOAST_MS,
            );
        }
    }

    /// `g n` — open the reminders overlay.
    pub(crate) fn open_reminders(&mut self) {
        self.show_reminders(0);
    }

    /// Move the overlay cursor. `delta` is signed; the cursor clamps
    /// rather than wrapping, matching every other TUI list.
    pub(crate) fn move_reminders_cursor(&mut self, delta: i32) {
        let Some(Overlay::Reminders(ref mut r)) = self.overlay else {
            return;
        };
        if r.all.is_empty() {
            return;
        }
        let last = r.all.len() - 1;
        let next = (r.selected as i32 + delta).clamp(0, last as i32);
        r.selected = next as usize;
    }

    /// `g s` in Normal — snooze the selected block's reminder an hour.
    ///
    /// Separate from the overlay path because the block under the
    /// cursor is the common case: you're looking at the nag, you want
    /// it to stop for an hour, opening a list first is friction.
    /// Silent when the block has no rule, rather than snoozing
    /// something that was never going to fire.
    pub(crate) fn snooze_selected_block(&mut self) {
        if !matches!(self.focus, Focus::Outline) || !matches!(self.mode, Mode::Normal) {
            return;
        }
        let Some(&node) = self.id_by_flat.get(self.selected) else {
            return;
        };
        // Ask the AST, same as `g r`: a rule written moments ago lives
        // there until the next save reconciles it into the op log, and
        // "this block has no rule" about a rule the user can see on
        // screen is the worst answer available.
        if self
            .property_on_current_block(outl_md::remind::REMIND_KEY)
            .is_none()
        {
            self.toast(ToastKind::Info, "this block has no remind:: rule");
            return;
        }
        self.snooze_block(node, SnoozePreset::OneHour);
    }

    /// `s` in the overlay — snooze the highlighted reminder.
    ///
    /// `which` indexes [`SnoozePreset::all`], so `s` / `S` / `Ctrl+S`
    /// offer the same three options the GUI clients show, resolved by
    /// the same code.
    ///
    /// Writes `Op::SnoozeRemind`, so the user's phone goes quiet too.
    pub(crate) fn snooze_selected_reminder(&mut self, which: usize) {
        let Some(node) = self.selected_reminder_block() else {
            return;
        };
        let Some(&preset) = SnoozePreset::all().get(which) else {
            return;
        };
        self.snooze_block(node, preset);
        self.refresh_reminders();
    }

    /// The one snooze both entry points route through, so `g s` and
    /// the overlay keys can't drift on which instant they resolve to.
    /// `SnoozePreset` owns that; this only applies the op.
    fn snooze_block(&mut self, node: outl_core::id::NodeId, preset: SnoozePreset) {
        let until = preset.resolve(outl_actions::clock::now_local().naive_local());
        let hlc = self.hlc.clone();
        match snooze_until(&mut self.workspace, &hlc, node, until) {
            Ok(()) => self.toast(
                ToastKind::Info,
                format!("snoozed: {} (every device)", preset.label()),
            ),
            Err(e) => self.toast(ToastKind::Error, format!("could not snooze — {e}")),
        }
    }

    /// `Enter` in the overlay — jump to the reminder's page.
    pub(crate) fn open_selected_reminder(&mut self) {
        let slug = match &self.overlay {
            Some(Overlay::Reminders(r)) => r.all.get(r.selected).map(|x| x.page_slug.clone()),
            _ => None,
        };
        let Some(slug) = slug else { return };
        self.overlay = None;
        if let Err(e) = self.open_page_by_name(&slug) {
            self.toast(ToastKind::Error, format!("could not open {slug} — {e}"));
        }
    }

    /// Re-scan after a mutation so the overlay reflects what landed.
    fn refresh_reminders(&mut self) {
        let Some(Overlay::Reminders(r)) = &self.overlay else {
            return;
        };
        self.show_reminders(r.selected);
    }

    /// Scan the workspace and (re)open the overlay on it.
    ///
    /// `want` is the row to land on; it is clamped, because a snooze
    /// re-sorts the list and the old index may now point past the end.
    /// The scan passes an empty [`FiredLog`] — the overlay answers
    /// "what is scheduled", never "what did this device deliver".
    fn show_reminders(&mut self, want: usize) {
        let quiet = outl_config::load().reminders.quiet_window();
        let now = outl_actions::clock::now_local().naive_local();
        let all = scan_reminders(&self.workspace, &FiredLog::new(), quiet, now);
        let selected = want.min(all.len().saturating_sub(1));
        self.overlay = Some(Overlay::Reminders(RemindersState { all, selected }));
    }

    fn selected_reminder_block(&self) -> Option<outl_core::id::NodeId> {
        match &self.overlay {
            Some(Overlay::Reminders(r)) => r.all.get(r.selected).map(|x| x.block_id),
            _ => None,
        }
    }
}

/// How often the event loop sweeps for due reminders.
///
/// `remind::` resolves to the minute, so anything under a minute is
/// already finer than the schedule. 20s keeps a fire visibly prompt
/// while keeping the sweep's two file reads out of the keystroke path.
const REMINDER_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);

/// How long a reminder toast stays up. Longer than the default 2.5s
/// because a reminder is the one toast the user must not miss while
/// looking away from the terminal.
const REMINDER_TOAST_MS: u64 = 10_000;

/// Emit an OSC 9 desktop-notification escape on stdout.
///
/// The sibling of the OSC 52 clipboard escape in `actions/yank.rs`,
/// and the same contract: only write when stdout is a real terminal so
/// a headless / piped run never spews the escape into captured output,
/// and treat it as best-effort. Success means "the terminal got it",
/// not "the terminal showed it" — iTerm2, kitty, WezTerm and ghostty
/// honour OSC 9; others ignore it. That's why a toast goes out too.
fn emit_osc9(text: &str) {
    let mut stdout = std::io::stdout();
    if !stdout.is_terminal() {
        return;
    }
    let seq = osc9_sequence(text);
    let _ = stdout
        .write_all(seq.as_bytes())
        .and_then(|()| stdout.flush());
}

/// Build the `OSC 9 ; <text> BEL` sequence.
///
/// Pure so the wire format is testable: a malformed escape doesn't
/// fail quietly, it paints garbage into the user's terminal.
///
/// OSC 9 carries no payload encoding (unlike OSC 52's base64), so a
/// BEL or ESC inside a block's text would terminate the sequence early
/// and spill the remainder as literal characters. Both are stripped
/// rather than escaped: a block whose text holds a control character
/// has nothing meaningful to show in a notification anyway.
fn osc9_sequence(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| *c != '\u{7}' && *c != '\u{1b}')
        .collect();
    format!("\x1b]9;{clean}\x07")
}

#[cfg(test)]
mod osc9_tests {
    use super::osc9_sequence;

    #[test]
    fn wraps_the_body_in_osc9_and_bel() {
        assert_eq!(
            osc9_sequence("outl · call the bank"),
            "\u{1b}]9;outl · call the bank\u{7}"
        );
    }

    #[test]
    fn strips_terminators_so_a_block_cannot_truncate_the_sequence() {
        // A block carrying a raw BEL would end the escape early and
        // spill the remainder into the terminal as literal text.
        let seq = osc9_sequence("before\u{7}after\u{1b}end");
        assert_eq!(seq, "\u{1b}]9;beforeafterend\u{7}");
        assert_eq!(
            seq.matches('\u{7}').count(),
            1,
            "exactly one BEL, the terminator"
        );
        assert_eq!(
            seq.matches('\u{1b}').count(),
            1,
            "exactly one ESC, the introducer"
        );
    }

    #[test]
    fn an_empty_body_is_still_a_well_formed_sequence() {
        assert_eq!(osc9_sequence(""), "\u{1b}]9;\u{7}");
    }
}
