//! Workspace scan: every block carrying a `remind::`, with its next
//! fire already resolved.
//!
//! Driven off the **tree**, not the `.md` projection, and deliberately
//! so on both counts:
//!
//! - **Not disk.** The `.md` is written asynchronously by the GUI
//!   clients' projection writer, so a rule authored a moment ago is
//!   not there yet. Pressing "remind me" and then opening the list
//!   would show nothing — which the user reads as "it didn't save".
//!   The op log is the source of truth; the scan reads that.
//! - **Not a tree walk.** [`outl_core::tree::Tree::nodes_with_property`]
//!   finds the handful of blocks carrying a `remind::` by scanning the
//!   property map, so the scan materializes text for **those blocks
//!   only**. Walking every block to ask "do you have a rule" would
//!   force a lazy-boot vault (#179) to materialize entirely under the
//!   workspace lock — the same freeze that moved the backlinks index
//!   off the workspace.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime};
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use super::schedule::{next_fire_at, ReminderState};
use crate::dates::date_from_slug;
use crate::page::page_meta;
use crate::todo::{split_todo, TodoState};
use crate::tree::enclosing_page_id;

/// How late a reminder is, for surfaces that colour the row.
///
/// Computed in Rust so the TUI, the desktop panel and the mobile sheet
/// paint the same thing. Resolved against the `now` the scan was given,
/// so it goes stale as the clock moves — bounded by the caller's poll
/// interval, which is fine for a colour and wrong for a decision (that
/// is [`super::schedule::next_fire_at`]'s job).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// Came due and is still not done. Every task app paints this red,
    /// and a reminder that already nagged you reads very differently
    /// from one that hasn't.
    Overdue,
    /// Fires within the hour.
    Soon,
    /// Fires later.
    Later,
    /// Never fires again (done, expired, out of `max`).
    Finished,
}

/// One scheduled reminder: a block, an anchor date, and when it next
/// interrupts the user.
///
/// A block with two `[[date]]`s yields two entries — the user wrote
/// both dates on purpose, and firing only on the first would silently
/// drop half of what they asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// The block carrying the `remind::` property.
    pub block_id: NodeId,
    /// Slug of the page the block lives on.
    pub page_slug: String,
    /// Display title of that page.
    pub page_title: String,
    /// Block body with the `TODO `/`DONE ` prefix already stripped,
    /// markup intact — what a list renders through the inline
    /// tokenizer.
    pub text: String,
    /// The same body with the inline syntax flattened away
    /// ([`outl_md::plain_text`]) — what a notification banner shows.
    /// A lock screen reading `ship it [[2026-12-12]] #fup` with the
    /// brackets intact looks like a bug, so the delivery paths take
    /// this instead of `text`.
    pub plain_text: String,
    /// The `remind::` value verbatim — what every surface echoes back
    /// to the user (the TUI overlay, the desktop panel, the DTO).
    /// The parsed rule isn't carried: `next_fire` already answers the
    /// only question a consumer asks of it.
    pub rule_text: String,
    /// Day the rule is anchored to: the block's `[[YYYY-MM-DD]]`, or
    /// today when it carries none.
    pub anchor_date: NaiveDate,
    /// The block's TODO flipped to DONE — this entry is finished.
    pub done: bool,
    /// Converged snooze instant from `Op::SnoozeRemind`, epoch ms.
    pub snoozed_until_ms: Option<u64>,
    /// Next fire in local wall clock, or `None` when the rule is done
    /// firing (completed, expired, out of `max`).
    pub next_fire: Option<NaiveDateTime>,
    /// How late this is, for row styling.
    pub urgency: Urgency,
}

/// Classify a next-fire instant against `now`.
///
/// `<= now` is overdue rather than "due right now": by the time a
/// surface renders, an instant that already passed is something the
/// user was supposed to have seen.
fn urgency_of(next_fire: Option<NaiveDateTime>, now: NaiveDateTime) -> Urgency {
    let Some(at) = next_fire else {
        return Urgency::Finished;
    };
    if at <= now {
        Urgency::Overdue
    } else if at <= now + chrono::Duration::hours(1) {
        Urgency::Soon
    } else {
        Urgency::Later
    }
}

/// Device-local record of what this device already delivered.
///
/// Deliberately **not** in the op log: "I already buzzed you" is true
/// of one device, not of the workspace. Each client persists this
/// however it likes (a 7-day TTL cache today) and hands it to the
/// scan; a read-only surface can pass an empty map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiredRecord {
    /// How many fires this device delivered for this (block, date).
    pub count: u32,
    /// When the last one went out, local wall clock.
    pub last: NaiveDateTime,
}

/// Keyed per `(block, anchor_date)` because one block with two dates
/// is two independent schedules.
pub type FiredLog = HashMap<(NodeId, NaiveDate), FiredRecord>;

/// Every reminder in the workspace, sorted by next fire (soonest
/// first); entries that will never fire again sort last.
///
/// `quiet` is the device-local quiet-hours window
/// (`outl_config::RemindersCfg::quiet_window`) and `now` the current
/// local wall clock. No workspace root: nothing here touches the
/// filesystem.
pub fn scan_reminders(
    workspace: &Workspace,
    fired: &FiredLog,
    quiet: Option<(u32, u32)>,
    now: NaiveDateTime,
) -> Vec<Reminder> {
    // Collect first: `nodes_with_property` borrows the tree, and the
    // body of the loop takes its own borrows of the workspace.
    let carriers: Vec<(NodeId, String)> = workspace
        .tree()
        .nodes_with_property(outl_md::remind::REMIND_KEY)
        .filter_map(|(node, value)| match value {
            PropValue::Text(v) => Some((node, v.clone())),
            _ => None,
        })
        .collect();

    let mut out = Vec::new();
    for (block_id, rule_text) in carriers {
        // An unreadable rule already surfaced as a `ParseWarning` on
        // the page; it simply doesn't schedule.
        let Some(rule) = outl_md::remind::parse_remind(&rule_text).rule else {
            continue;
        };
        // A trashed block keeps its properties, but a deleted block
        // must never nag — no enclosing page means it's gone.
        let Some(page_id) = enclosing_page_id(workspace, block_id) else {
            continue;
        };
        let Some(meta) = page_meta(workspace, page_id) else {
            continue;
        };
        let raw = workspace.block_text(block_id).unwrap_or_default();
        let (todo, body) = split_todo(&raw);
        let done = todo == Some(TodoState::Done);
        let snoozed_until_ms = workspace.tree().snoozed_until(block_id);

        for anchor_date in anchor_dates(body, now.date()) {
            let seen = fired.get(&(block_id, anchor_date));
            let state = ReminderState {
                done,
                fired_count: seen.map_or(0, |f| f.count),
                last_fired: seen.map(|f| f.last),
                snoozed_until: snoozed_until_ms.and_then(super::epoch_ms_to_local_naive),
            };
            let next_fire = next_fire_at(&rule, anchor_date, &state, quiet, now);
            out.push(Reminder {
                block_id,
                page_slug: meta.slug.clone(),
                page_title: meta.title.clone(),
                text: body.to_string(),
                plain_text: outl_md::plain_text(body),
                rule_text: rule_text.clone(),
                anchor_date,
                done,
                snoozed_until_ms,
                next_fire,
                urgency: urgency_of(next_fire, now),
            });
        }
    }

    // `None` (finished) sorts after every real instant so a UI list
    // reads top-down as "what's coming". The `block_id` / `anchor_date`
    // tiebreaks make the order **total**: the carriers came out of a
    // `HashMap` with no order of its own, and a list that reshuffles
    // under the cursor between scans is unusable.
    out.sort_by(|a, b| {
        match (a.next_fire, b.next_fire) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.text.cmp(&b.text))
        .then_with(|| a.block_id.cmp(&b.block_id))
        .then_with(|| a.anchor_date.cmp(&b.anchor_date))
    });
    out
}

/// Every `[[YYYY-MM-DD]]` in the block, or `[today]` when it has none.
///
/// Implicit-today is the low-friction path for a quick capture
/// (`TODO call the bank` + `remind:: 3pm`). An explicit `[[date]]` is
/// what a plan-ahead reminder uses.
fn anchor_dates(text: &str, today: NaiveDate) -> Vec<NaiveDate> {
    let mut dates: Vec<NaiveDate> = crate::backlinks::extract_refs(text)
        .iter()
        .filter_map(|r| date_from_slug(r))
        .collect();
    dates.sort_unstable();
    dates.dedup();
    if dates.is_empty() {
        dates.push(today);
    }
    dates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    #[test]
    fn a_block_without_a_date_anchors_to_today() {
        let today = day(2026, 8, 2);
        assert_eq!(anchor_dates("call the bank", today), vec![today]);
    }

    #[test]
    fn every_date_in_the_block_gets_its_own_schedule() {
        let today = day(2026, 8, 2);
        assert_eq!(
            anchor_dates("ping [[2026-12-12]] and [[2026-12-15]]", today),
            vec![day(2026, 12, 12), day(2026, 12, 15)]
        );
    }

    #[test]
    fn a_non_date_ref_is_not_an_anchor() {
        let today = day(2026, 8, 2);
        assert_eq!(
            anchor_dates("[[@joão]] about [[project abc]]", today),
            vec![today]
        );
    }

    #[test]
    fn a_date_inside_a_code_span_does_not_anchor_a_reminder() {
        // `anchor_dates` rides `extract_refs`, which now reads the same
        // inline grammar the renderer does. A `[[date]]` the user
        // escaped in backticks is prose about a date, not a date.
        //
        // **Nothing goes silent.** Losing the only explicit anchor falls
        // back to implicit-today, so the reminder still fires — it moves
        // from the quoted date to today. A reminder that stops firing
        // would be the worse failure, and this path cannot produce one.
        let today = day(2026, 8, 2);
        assert_eq!(
            anchor_dates("ship it, see `[[2026-12-12]]` for why", today),
            vec![today]
        );
        // A real anchor alongside a quoted one keeps the real one.
        assert_eq!(
            anchor_dates("ping [[2026-12-15]], not `[[2026-12-12]]`", today),
            vec![day(2026, 12, 15)]
        );
    }

    #[test]
    fn a_nested_ref_does_not_anchor_to_the_inner_date() {
        // `[[a [[2026-12-12]] ]]` is one ref named `a [[2026-12-12`,
        // which is not a date. The byte scan returned the inner `b`-side
        // and scheduled a real notification off it.
        let today = day(2026, 8, 2);
        assert_eq!(anchor_dates("[[nota [[2026-12-12]] ]]", today), vec![today]);
    }

    #[test]
    fn an_anchor_inside_emphasis_still_counts() {
        // The transparency half: a bolded date is still a date.
        let today = day(2026, 8, 2);
        assert_eq!(
            anchor_dates("**[[2026-12-12]]**: reunião", today),
            vec![day(2026, 12, 12)]
        );
    }

    #[test]
    fn the_same_date_twice_schedules_once() {
        let today = day(2026, 8, 2);
        assert_eq!(
            anchor_dates("[[2026-12-12]] and again [[2026-12-12]]", today),
            vec![day(2026, 12, 12)]
        );
    }
}
