//! The `remind::` block property: an English-shaped notification rule.
//!
//! ```text
//! - TODO #fup [[@joão]] about project abc [[2026-12-12]]
//!   remind:: 3pm every 1h until DONE
//! ```
//!
//! Grammar (see `docs/reminders.md` for the user-facing spec):
//!
//! ```ebnf
//! remind     ::= TIME ("every" INTERVAL)? ("until" STOP)? ("max" N)?
//! TIME       ::= H_AMPM | HHMM | HHMM_AMPM | "now"
//! INTERVAL   ::= N ("min" | "h" | "d")
//! STOP       ::= "DONE" | TIME | ISO_DATE
//! N          ::= 1..999
//! ```
//!
//! This module owns **syntax only**: text in, [`RemindRule`] out, plus the
//! [`ParseWarningKind`] records for anything it had to reject or clamp.
//! *When* a rule actually fires is `outl_actions::reminders` — one owner
//! for the schedule math, wrapped by every OS bridge.
//!
//! Parsing is permissive in the same sense as the rest of [`mod@crate::parse`]:
//! an invalid `remind::` never removes the property and never drops the
//! block. It just yields `None` (no scheduling) plus a warning the client
//! surfaces in the parse banner.

use crate::parse::ParseWarningKind;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Property key that carries a reminder rule on a block.
pub const REMIND_KEY: &str = "remind";

/// Smallest repeat interval we accept, in minutes. Anything below this
/// (`every 30s`) is rejected rather than clamped — a sub-minute nag is
/// never what the user meant, and silently rewriting it to 1min would
/// hide the typo.
pub const MIN_INTERVAL_MINUTES: u32 = 1;

/// Hard ceiling on `max N`. A rule asking for more is clamped down to
/// this and reported via [`ParseWarningKind::RemindMaxClamped`].
pub const MAX_FIRES_CAP: u32 = 10;

/// Where the first fire is anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemindAnchor {
    /// `now` — fire as soon as the rule is seen.
    Now,
    /// A wall-clock time of day, on the block's resolved date.
    At {
        /// Hour, 0-23.
        hour: u32,
        /// Minute, 0-59.
        minute: u32,
    },
}

/// When the repetition stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemindStop {
    /// `until DONE` — stop when the block's TODO flips to DONE. Also the
    /// implicit default when the user writes no `until` clause.
    Done,
    /// `until 6pm` — stop at a wall-clock time on the anchor's date.
    Time {
        /// Hour, 0-23.
        hour: u32,
        /// Minute, 0-59.
        minute: u32,
    },
    /// `until 2026-12-20` — stop at the end of that ISO date.
    Date(NaiveDate),
}

/// A parsed, validated `remind::` rule.
///
/// Every field is already clamped to the hard caps, so a consumer can
/// schedule straight off it without re-validating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemindRule {
    /// First fire.
    pub anchor: RemindAnchor,
    /// Repeat interval in minutes. `None` = single shot.
    pub every_minutes: Option<u32>,
    /// Stop condition. `None` means the implicit `until DONE`.
    pub until: Option<RemindStop>,
    /// Cap on the number of fires. `None` = uncapped (bounded by `until`).
    pub max_fires: Option<u32>,
}

impl RemindRule {
    /// The effective stop condition, resolving the implicit default.
    pub fn stop(&self) -> RemindStop {
        self.until.unwrap_or(RemindStop::Done)
    }

    /// Whether this rule repeats at all.
    pub fn repeats(&self) -> bool {
        self.every_minutes.is_some()
    }
}

/// Outcome of parsing one `remind::` value.
///
/// `rule` is `None` when the value could not be understood — scheduling
/// is disabled for that block, but the property text stays on disk
/// untouched so the user can fix the typo in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemindParse {
    /// The rule, when the value parsed.
    pub rule: Option<RemindRule>,
    /// Everything the parser rejected or clamped, in encounter order.
    pub warnings: Vec<ParseWarningKind>,
}

/// Parse the value side of a `remind:: …` property line.
///
/// Never panics, never allocates on the happy path beyond the token
/// split. See the module docs for the grammar.
pub fn parse_remind(value: &str) -> RemindParse {
    let lowered = value.trim().to_ascii_lowercase();
    let tokens: Vec<&str> = lowered.split_whitespace().collect();
    let mut warnings = Vec::new();

    let Some((&first, mut rest)) = tokens.split_first() else {
        warnings.push(ParseWarningKind::RemindMissingAnchor);
        return RemindParse {
            rule: None,
            warnings,
        };
    };

    // The anchor is mandatory: `remind:: every 1h` has no start.
    if first == "every" || first == "until" || first == "max" {
        warnings.push(ParseWarningKind::RemindMissingAnchor);
        return RemindParse {
            rule: None,
            warnings,
        };
    }
    let anchor = match parse_anchor(first) {
        Some(a) => a,
        None => {
            warnings.push(ParseWarningKind::RemindInvalidTime);
            return RemindParse {
                rule: None,
                warnings,
            };
        }
    };

    let mut every_minutes = None;
    let mut until = None;
    let mut max_fires = None;

    while let Some((&keyword, tail)) = rest.split_first() {
        let Some((&arg, after)) = tail.split_first() else {
            // Dangling keyword with no argument.
            warnings.push(match keyword {
                "every" => ParseWarningKind::RemindInvalidInterval,
                "until" => ParseWarningKind::RemindInvalidStop,
                _ => ParseWarningKind::RemindInvalidTime,
            });
            return RemindParse {
                rule: None,
                warnings,
            };
        };
        match keyword {
            "every" => match parse_interval(arg) {
                Some(m) => every_minutes = Some(m),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidInterval);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            "until" => match parse_stop(arg) {
                Some(s) => until = Some(s),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidStop);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            "max" => match arg.parse::<u32>().ok().filter(|n| (1..=999).contains(n)) {
                Some(n) if n > MAX_FIRES_CAP => {
                    warnings.push(ParseWarningKind::RemindMaxClamped);
                    max_fires = Some(MAX_FIRES_CAP);
                }
                Some(n) => max_fires = Some(n),
                None => {
                    warnings.push(ParseWarningKind::RemindInvalidInterval);
                    return RemindParse {
                        rule: None,
                        warnings,
                    };
                }
            },
            _ => {
                warnings.push(ParseWarningKind::RemindInvalidTime);
                return RemindParse {
                    rule: None,
                    warnings,
                };
            }
        }
        rest = after;
    }

    // `until 6pm` earlier than the anchor is unschedulable on the same
    // day. Drop the clause rather than the whole rule — the user still
    // gets their reminder, just without the stop.
    if let (
        RemindAnchor::At { hour, minute },
        Some(RemindStop::Time {
            hour: sh,
            minute: sm,
        }),
    ) = (anchor, until)
    {
        if (sh, sm) <= (hour, minute) {
            warnings.push(ParseWarningKind::RemindInvalidStop);
            until = None;
        }
    }

    RemindParse {
        rule: Some(RemindRule {
            anchor,
            every_minutes,
            until,
            max_fires,
        }),
        warnings,
    }
}

/// Write a rule back out in the grammar [`parse_remind`] reads.
///
/// The missing half of the pair this crate's CLAUDE.md asks for.
/// A parser with no inverse makes the next consumer write one, and
/// that consumer arrived as the mobile "Remind me…" sheet: it turns a
/// time picker and a repeat row back into `15:00 every 1h`, which
/// without this would have made a webview the second owner of the
/// grammar.
///
/// Times come out in the 24-hour form, so the output never depends on
/// an am/pm the user did not write. `until DONE` is left off: it is
/// the implicit default, and spelling it adds length, not meaning.
///
/// **Round-trips the rule's meaning, not its `Option`s.** An explicit
/// `until: Some(RemindStop::Done)` comes back as `None`, because the
/// two are one schedule — [`RemindRule::stop`] resolves both to
/// [`RemindStop::Done`] — and the grammar has one short spelling for
/// it. Compare a round-trip through `stop()`, never through `until`.
///
/// **Round-trips every rule [`parse_remind`] can produce.** It cannot
/// round-trip a hand-built [`RemindRule`] outside that domain — an
/// `every_minutes` that is neither under 1000 minutes nor a whole
/// number of hours or days has no spelling in the grammar at all — and
/// such a value re-parses as an invalid interval rather than as a
/// different schedule.
pub fn format_remind(rule: &RemindRule) -> String {
    let mut out = match rule.anchor {
        RemindAnchor::Now => "now".to_string(),
        RemindAnchor::At { hour, minute } => format!("{hour:02}:{minute:02}"),
    };
    if let Some(every) = rule.every_minutes {
        out.push_str(" every ");
        out.push_str(&format_interval(every));
    }
    match rule.until {
        None | Some(RemindStop::Done) => {}
        Some(RemindStop::Time { hour, minute }) => {
            out.push_str(&format!(" until {hour:02}:{minute:02}"));
        }
        Some(RemindStop::Date(d)) => {
            out.push_str(&format!(" until {}", d.format("%Y-%m-%d")));
        }
    }
    if let Some(max) = rule.max_fires {
        out.push_str(&format!(" max {max}"));
    }
    out
}

/// Minutes as the largest unit that spells them exactly and keeps the
/// count inside the grammar's `1..=999`: `2880` is `2d`, not `48h`.
fn format_interval(minutes: u32) -> String {
    for (unit, size) in [("d", 60 * 24), ("h", 60)] {
        if minutes.is_multiple_of(size) && (1..=999).contains(&(minutes / size)) {
            return format!("{}{unit}", minutes / size);
        }
    }
    format!("{minutes}min")
}

/// Pull the rule out of a block's property list, if it carries one.
///
/// Returns `None` when the block has no `remind::` at all — an invalid
/// rule also yields `None` on the rule side, with its warnings attached.
pub fn rule_from_properties(properties: &[(String, String)]) -> Option<RemindParse> {
    properties
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(REMIND_KEY))
        .map(|(_, v)| parse_remind(v))
}

fn parse_anchor(tok: &str) -> Option<RemindAnchor> {
    if tok == "now" {
        return Some(RemindAnchor::Now);
    }
    let (hour, minute) = parse_time(tok)?;
    Some(RemindAnchor::At { hour, minute })
}

fn parse_stop(tok: &str) -> Option<RemindStop> {
    if tok == "done" {
        return Some(RemindStop::Done);
    }
    if let Some((hour, minute)) = parse_time(tok) {
        return Some(RemindStop::Time { hour, minute });
    }
    NaiveDate::parse_from_str(tok, "%Y-%m-%d")
        .ok()
        .map(RemindStop::Date)
}

/// `10am`, `3pm`, `15:00`, `1:30pm`, `9:05`.
fn parse_time(tok: &str) -> Option<(u32, u32)> {
    if let Some(body) = tok.strip_suffix("am") {
        let (h, m) = split_hm(body)?;
        if !(1..=12).contains(&h) {
            return None;
        }
        return Some((if h == 12 { 0 } else { h }, m));
    }
    if let Some(body) = tok.strip_suffix("pm") {
        let (h, m) = split_hm(body)?;
        if !(1..=12).contains(&h) {
            return None;
        }
        return Some((if h == 12 { 12 } else { h + 12 }, m));
    }
    // 24h form requires the colon; a bare `15` is too ambiguous to guess.
    let (h, m) = tok.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// `10` -> (10, 0); `1:30` -> (1, 30). Used for the am/pm branch only.
fn split_hm(body: &str) -> Option<(u32, u32)> {
    match body.split_once(':') {
        Some((h, m)) => {
            let hour: u32 = h.parse().ok()?;
            let minute: u32 = m.parse().ok()?;
            (minute < 60).then_some((hour, minute))
        }
        None => body.parse::<u32>().ok().map(|h| (h, 0)),
    }
}

/// `30min`, `1h`, `2d`.
fn parse_interval(tok: &str) -> Option<u32> {
    let (digits, unit) = tok.split_at(tok.find(|c: char| !c.is_ascii_digit())?);
    let n: u32 = digits.parse().ok()?;
    if !(1..=999).contains(&n) {
        return None;
    }
    let minutes = match unit {
        "min" | "m" => n,
        "h" => n.checked_mul(60)?,
        "d" => n.checked_mul(60 * 24)?,
        _ => return None,
    };
    (minutes >= MIN_INTERVAL_MINUTES).then_some(minutes)
}

#[cfg(test)]
mod tests;
