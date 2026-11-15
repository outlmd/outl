//! Slash commands that **insert text at the cursor** — the `/date-today`
//! family, `/time-now`, `/week-num`.
//!
//! ## Why this is here and not in a client
//!
//! These nineteen commands are a different shape from everything in
//! `outl_shortcuts`: they take no chord, mutate nothing, and answer one
//! question — *what string does this put where the caret is?* The
//! arithmetic behind them already lived in [`crate::dates`]; what did
//! not was the catalog, which sat in nineteen `SlashCommand` structs
//! inside `outl-tui`.
//!
//! So the TUI was the only client that could type `/date-today`. A
//! desktop user typed it, got nothing, and the obvious repair was to
//! write the same nineteen strings again in TypeScript — a second owner
//! of "what `/date-today` inserts", drifting the moment either side is
//! edited. That is the drift
//! `outl_shortcuts::command_name` was added to end for the *names*;
//! this module is the same answer for the *text*.
//!
//! ## Shape
//!
//! [`ALL`] is the catalog. [`resolve`] turns one entry plus the caller's
//! clock reading into the string to splice in. `resolve` takes `now`
//! rather than reading [`crate::clock`] itself, so every case is
//! testable against a fixed instant instead of whatever day CI runs on.

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Weekday};

use crate::dates;

/// What an entry produces, once it has a clock reading.
///
/// Private: a client picks a command by name and asks [`resolve`]. The
/// variants are an implementation of "what text", which is exactly the
/// fact this module exists to keep in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Produces {
    /// `[[YYYY-MM-DD]]` for today plus this many days.
    JournalRef(i64),
    /// Bare `YYYY-MM-DD` for today plus this many days, for `due::`.
    IsoDate(i64),
    /// `[[YYYY-MM-DD]]` for the next occurrence of this weekday.
    NextWeekday(Weekday),
    /// `HH:MM` wall clock.
    TimeNow,
    /// `[[YYYY-MM-DD]] HH:MM` — the "stamp this moment" combo.
    DateTimeNow,
    /// `#YYYY-Www`, an ISO week **tag** rather than a ref, so it routes
    /// through tag indexing and shows up in the week page's backlinks.
    WeekTag,
    /// A journal ref for whatever the user's argument resolves to
    /// (`+3d`, `-2w`, `2026-06-15`).
    JournalRefFromArg,
}

/// One text-inserting slash command.
pub struct InsertCommand {
    /// What the user types after `/`.
    pub name: &'static str,
    /// Short spellings the TUI's `:` palette also accepts.
    pub aliases: &'static [&'static str],
    /// The one-liner a command menu shows beside the name.
    pub description: &'static str,
    produces: Produces,
}

impl InsertCommand {
    /// Whether [`resolve`] needs a non-empty `args` to answer.
    ///
    /// Only `/date` does. A client with no way to collect an argument
    /// (the desktop's inline menu closes its trigger on the first
    /// space) should leave those rows out rather than offer a command
    /// that can only fail.
    pub fn needs_args(&self) -> bool {
        self.produces == Produces::JournalRefFromArg
    }
}

/// Why [`resolve`] could not produce text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertError {
    /// No command in [`ALL`] answers to that name or alias.
    UnknownCommand(String),
    /// `/date` got an argument it could not read as a date.
    ///
    /// Carries the usage line, so no client writes its own.
    BadDateArg(&'static str),
}

impl std::fmt::Display for InsertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsertError::UnknownCommand(name) => write!(f, "unknown command: {name}"),
            InsertError::BadDateArg(usage) => write!(f, "{usage}"),
        }
    }
}

impl std::error::Error for InsertError {}

/// The usage line `/date` surfaces on an argument it cannot parse.
pub const DATE_USAGE: &str = "usage: date +Nd | -Nw | +Nm | YYYY-MM-DD";

/// Every text-inserting slash command, in menu order.
pub const ALL: &[InsertCommand] = &[
    InsertCommand {
        name: "date-today",
        aliases: &["dt"],
        description: "Insert today's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::JournalRef(0),
    },
    InsertCommand {
        name: "date-tomorrow",
        aliases: &["dtm"],
        description: "Insert tomorrow's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::JournalRef(1),
    },
    InsertCommand {
        name: "date-yesterday",
        aliases: &["dy"],
        description: "Insert yesterday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::JournalRef(-1),
    },
    InsertCommand {
        name: "date-next-week",
        aliases: &["dnw"],
        description: "Insert next week's journal ref (today + 7 days)",
        produces: Produces::JournalRef(7),
    },
    InsertCommand {
        name: "date-last-week",
        aliases: &["dlw"],
        description: "Insert last week's journal ref (today − 7 days)",
        produces: Produces::JournalRef(-7),
    },
    InsertCommand {
        name: "date-next-monday",
        aliases: &["dnmon"],
        description: "Insert next Monday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Mon),
    },
    InsertCommand {
        name: "date-next-tuesday",
        aliases: &["dntue"],
        description: "Insert next Tuesday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Tue),
    },
    InsertCommand {
        name: "date-next-wednesday",
        aliases: &["dnwed"],
        description: "Insert next Wednesday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Wed),
    },
    InsertCommand {
        name: "date-next-thursday",
        aliases: &["dnthu"],
        description: "Insert next Thursday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Thu),
    },
    InsertCommand {
        name: "date-next-friday",
        aliases: &["dnfri"],
        description: "Insert next Friday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Fri),
    },
    InsertCommand {
        name: "date-next-saturday",
        aliases: &["dnsat"],
        description: "Insert next Saturday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Sat),
    },
    InsertCommand {
        name: "date-next-sunday",
        aliases: &["dnsun"],
        description: "Insert next Sunday's journal ref — `[[YYYY-MM-DD]]`",
        produces: Produces::NextWeekday(Weekday::Sun),
    },
    InsertCommand {
        name: "iso-date-today",
        aliases: &["isod"],
        description: "Insert today's date as plain ISO — `YYYY-MM-DD` (no brackets, for `due::` etc)",
        produces: Produces::IsoDate(0),
    },
    InsertCommand {
        name: "iso-date-tomorrow",
        aliases: &["isodtm"],
        description: "Insert tomorrow's date as plain ISO — `YYYY-MM-DD` (no brackets)",
        produces: Produces::IsoDate(1),
    },
    InsertCommand {
        name: "iso-date-yesterday",
        aliases: &["isody"],
        description: "Insert yesterday's date as plain ISO — `YYYY-MM-DD` (no brackets)",
        produces: Produces::IsoDate(-1),
    },
    InsertCommand {
        name: "time-now",
        aliases: &["now", "tn"],
        description: "Insert the current local time — `HH:MM` (no brackets)",
        produces: Produces::TimeNow,
    },
    InsertCommand {
        name: "datetime-now",
        aliases: &["dtn", "stamp"],
        description: "Insert journal ref + time — `[[YYYY-MM-DD]] HH:MM`",
        produces: Produces::DateTimeNow,
    },
    InsertCommand {
        name: "week-num",
        aliases: &["wn", "week"],
        description: "Insert current ISO week as a tag — `#YYYY-Www`",
        produces: Produces::WeekTag,
    },
    InsertCommand {
        name: "date",
        aliases: &[],
        description: "Insert a journal ref by offset or ISO date — `date +3d` · `date -2w` · `date +1m` · `date 2026-06-15`",
        produces: Produces::JournalRefFromArg,
    },
];

/// Look up a command by its name or one of its aliases.
///
/// Both, because the TUI's `:` palette has always accepted `:dt` for
/// `:date-today` and dropping that on the way through a shared catalog
/// would be a silent regression in the client that had the feature.
pub fn find(name: &str) -> Option<&'static InsertCommand> {
    ALL.iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// The text `command` inserts, given the caller's clock reading.
///
/// `now` is a parameter rather than a [`crate::clock`] read so a test
/// can fix the instant. Callers pass `clock::now_local()`.
pub fn resolve(
    command: &InsertCommand,
    args: &str,
    now: DateTime<FixedOffset>,
) -> Result<String, InsertError> {
    let today = now.date_naive();
    let ref_at = |d: NaiveDate| dates::journal_ref(d);
    Ok(match command.produces {
        Produces::JournalRef(days) => ref_at(today + Duration::days(days)),
        Produces::IsoDate(days) => dates::journal_slug(today + Duration::days(days)),
        Produces::NextWeekday(day) => {
            ref_at(today + Duration::days(dates::days_until_next_weekday(today, day)))
        }
        Produces::TimeNow => now.format("%H:%M").to_string(),
        Produces::DateTimeNow => format!("{} {}", ref_at(today), now.format("%H:%M")),
        Produces::WeekTag => dates::week_tag(today),
        Produces::JournalRefFromArg => match dates::parse_date_arg(args, today) {
            Some(d) => ref_at(d),
            None => return Err(InsertError::BadDateArg(DATE_USAGE)),
        },
    })
}

/// Resolve by name in one step, for a caller that has a string rather
/// than a catalog entry (a Tauri command, the TUI's `:` palette).
pub fn resolve_named(
    name: &str,
    args: &str,
    now: DateTime<FixedOffset>,
) -> Result<String, InsertError> {
    let command = find(name).ok_or_else(|| InsertError::UnknownCommand(name.to_string()))?;
    resolve(command, args, now)
}

/// Whether `date` is the weekday `/date-next-<weekday>` names.
///
/// Only used by this module's tests, which assert the *weekday* rather
/// than a hardcoded date, so they keep passing on any day CI runs.
#[cfg(test)]
fn weekday_of(label: &str) -> Option<Weekday> {
    use chrono::Datelike;
    let slug = label.trim_start_matches("[[").trim_end_matches("]]");
    dates::date_from_slug(slug).map(|d| d.weekday())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A Wednesday, so the next-weekday cases have a non-trivial answer
    /// in both directions.
    fn at(h: u32, m: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .expect("UTC is a valid offset")
            .with_ymd_and_hms(2026, 11, 18, h, m, 0)
            .single()
            .expect("the instant is unambiguous in a fixed offset")
    }

    #[test]
    fn a_journal_ref_is_bracketed_and_an_iso_date_is_not() {
        assert_eq!(
            resolve_named("date-today", "", at(9, 30)).unwrap(),
            "[[2026-11-18]]"
        );
        assert_eq!(
            resolve_named("iso-date-today", "", at(9, 30)).unwrap(),
            "2026-11-18"
        );
    }

    #[test]
    fn tomorrow_and_yesterday_move_one_day() {
        assert_eq!(
            resolve_named("date-tomorrow", "", at(9, 30)).unwrap(),
            "[[2026-11-19]]"
        );
        assert_eq!(
            resolve_named("date-yesterday", "", at(9, 30)).unwrap(),
            "[[2026-11-17]]"
        );
    }

    #[test]
    fn the_clock_commands_read_the_offset_they_were_given() {
        assert_eq!(resolve_named("time-now", "", at(14, 5)).unwrap(), "14:05");
        assert_eq!(
            resolve_named("datetime-now", "", at(14, 5)).unwrap(),
            "[[2026-11-18]] 14:05"
        );
    }

    #[test]
    fn a_week_number_is_a_tag_not_a_ref() {
        let out = resolve_named("week-num", "", at(9, 0)).unwrap();
        assert!(out.starts_with('#'), "{out} should be a tag");
        assert!(!out.contains("[["), "{out} should not be a ref");
    }

    /// `days_until_next_weekday` returns 7 on the same weekday, so every
    /// one of these lands in the future. Asserted on the weekday rather
    /// than a literal date, which is the property that matters.
    #[test]
    fn every_next_weekday_lands_on_that_weekday_in_the_future() {
        for (name, want) in [
            ("date-next-monday", Weekday::Mon),
            ("date-next-tuesday", Weekday::Tue),
            ("date-next-wednesday", Weekday::Wed),
            ("date-next-thursday", Weekday::Thu),
            ("date-next-friday", Weekday::Fri),
            ("date-next-saturday", Weekday::Sat),
            ("date-next-sunday", Weekday::Sun),
        ] {
            let out = resolve_named(name, "", at(9, 0)).unwrap();
            assert_eq!(weekday_of(&out), Some(want), "/{name} produced {out}");
            assert!(out.as_str() > "[[2026-11-18]]", "/{name} is not ahead");
        }
    }

    #[test]
    fn date_reads_an_offset_or_an_absolute_day() {
        assert_eq!(
            resolve_named("date", "+3d", at(9, 0)).unwrap(),
            "[[2026-11-21]]"
        );
        assert_eq!(
            resolve_named("date", "2026-06-15", at(9, 0)).unwrap(),
            "[[2026-06-15]]"
        );
    }

    /// The usage line belongs to the catalog, so no client writes its
    /// own wording for the same refusal.
    #[test]
    fn date_refuses_an_argument_it_cannot_read() {
        assert_eq!(
            resolve_named("date", "whenever", at(9, 0)),
            Err(InsertError::BadDateArg(DATE_USAGE))
        );
    }

    #[test]
    fn an_alias_resolves_to_the_same_text_as_its_name() {
        for (alias, name) in [
            ("dt", "date-today"),
            ("isod", "iso-date-today"),
            ("now", "time-now"),
            ("stamp", "datetime-now"),
            ("week", "week-num"),
            ("dnmon", "date-next-monday"),
        ] {
            assert_eq!(
                resolve_named(alias, "", at(9, 0)),
                resolve_named(name, "", at(9, 0)),
                "/{alias} and /{name} disagree"
            );
        }
    }

    /// A menu dispatches on these strings, so a collision is a row
    /// whose behaviour depends on `find`'s iteration order.
    #[test]
    fn no_name_or_alias_is_claimed_twice() {
        let mut seen: Vec<&str> = Vec::new();
        for command in ALL {
            for spelling in std::iter::once(&command.name).chain(command.aliases) {
                assert!(
                    !seen.contains(spelling),
                    "`{spelling}` is claimed twice in the catalog"
                );
                seen.push(spelling);
            }
        }
    }

    /// The name rides a `/` trigger whose alphabet stops at the first
    /// character outside `[A-Za-z0-9_-]`, so anything else is a command
    /// the user cannot finish typing.
    #[test]
    fn every_name_is_typeable_after_a_slash() {
        for command in ALL {
            for spelling in std::iter::once(&command.name).chain(command.aliases) {
                assert!(
                    spelling
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                    "`{spelling}` is not slash-typeable"
                );
            }
        }
    }

    /// Only `/date` collects an argument. A client that cannot collect
    /// one filters on this, so a wrong answer here is a row that can
    /// only ever fail.
    #[test]
    fn date_is_the_only_command_that_needs_an_argument() {
        let needing: Vec<_> = ALL
            .iter()
            .filter(|c| c.needs_args())
            .map(|c| c.name)
            .collect();
        assert_eq!(needing, vec!["date"]);
    }

    #[test]
    fn an_unknown_name_is_reported_rather_than_guessed() {
        assert_eq!(
            resolve_named("date-next-caturday", "", at(9, 0)),
            Err(InsertError::UnknownCommand("date-next-caturday".into()))
        );
    }
}
