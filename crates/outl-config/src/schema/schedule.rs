//! How outl resolves wall-clock time, and when it may interrupt.
//!
//! One concept read from two angles: `[calendar]` fixes the zone every
//! "now" and "today" is answered in, and `[reminders]` is the window
//! inside that zone where a scheduled fire is held back. Both are
//! device-local — the rule itself (`remind:: 3pm every 1h`) lives in the
//! markdown and converges through the op log.

use serde::{Deserialize, Serialize};

/// Calendar / time section — controls how outl renders "now" and
/// "today".
///
/// `timezone` is an optional IANA name (`"Europe/London"`,
/// `"America/Sao_Paulo"`). When unset, outl uses the operating
/// system's local timezone — the right default on a normally
/// configured machine. Set it explicitly when the OS clock lies about
/// the zone: containers and Chrome OS **Crostini** run in UTC even
/// though the user's real timezone isn't, which pushes the journal
/// date and the status-line clock an hour (or more) off. See issue
/// #107.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarCfg {
    /// IANA timezone name, e.g. `"Europe/London"`. `None` (the
    /// default) means "use the OS local timezone". An unknown or
    /// unparseable name is ignored when the clock initializes (logged)
    /// and also falls back to local.
    pub timezone: Option<String>,
}

/// Reminder delivery preferences.
///
/// **Device-local on purpose.** Quiet hours are a property of *this*
/// phone / *this* laptop, not of the workspace — the rule itself
/// (`remind:: 3pm every 1h`) lives in the markdown and converges
/// through the op log, and so does a snooze
/// (`outl_core::op::Op::SnoozeRemind`). Putting quiet hours in the
/// op log would silence a laptop because a phone was asleep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemindersCfg {
    /// Master switch, **on by default**.
    ///
    /// Writing `remind:: 3pm` on a block is already the explicit
    /// opt-in — a bare `[[date]]` schedules nothing. Defaulting this
    /// to `false` meant the user wrote the rule, waited, got nothing,
    /// and had to go find a toggle: the feature silently broken out of
    /// the box. A device that never gets a `remind::` never fires, so
    /// the default costs someone who doesn't use reminders nothing,
    /// not even the OS permission prompt (which only appears on the
    /// first actual notification).
    ///
    /// Set it to `false` to keep the rules tracked and listed on this
    /// device while never being interrupted by them.
    pub enabled: bool,

    /// `"22:00-07:00"` — a fire that would land inside this window is
    /// pushed to the window's end instead of dropped. `None` (the
    /// default) means no quiet hours. A window that wraps midnight is
    /// the normal case and is handled.
    ///
    /// An unparseable value is ignored (treated as `None`) rather than
    /// failing the whole config load — a typo here must never keep the
    /// app from opening.
    pub quiet_hours: Option<String>,
}

impl Default for RemindersCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_hours: None,
        }
    }
}

impl RemindersCfg {
    /// Parse [`Self::quiet_hours`] into `(start_minutes, end_minutes)`
    /// past midnight, or `None` when unset / unparseable.
    ///
    /// `start == end` is rejected: a zero-width window is a typo, and
    /// reading it as "quiet all day" would silence every reminder the
    /// user asked for.
    pub fn quiet_window(&self) -> Option<(u32, u32)> {
        let raw = self.quiet_hours.as_deref()?.trim();
        let (start, end) = raw.split_once('-')?;
        let start = parse_hhmm(start.trim())?;
        let end = parse_hhmm(end.trim())?;
        (start != end).then_some((start, end))
    }
}

/// `"22:00"` -> minutes past midnight.
fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn calendar_section_parses_timezone() {
        let c: Config = toml::from_str("[calendar]\ntimezone = \"Europe/London\"\n").unwrap();
        assert_eq!(c.calendar.timezone.as_deref(), Some("Europe/London"));
    }

    #[test]
    fn missing_calendar_section_leaves_timezone_unset() {
        // No [calendar] → timezone None → clock uses OS local (previous behaviour).
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert!(c.calendar.timezone.is_none());
    }

    #[test]
    fn reminders_deliver_by_default() {
        // `remind::` on a block IS the opt-in. A device that never
        // gets a rule never fires, so defaulting off only bought the
        // user a rule that silently did nothing.
        let c: Config = toml::from_str("").unwrap();
        assert!(c.reminders.enabled);
        assert_eq!(c.reminders.quiet_window(), None);
    }

    #[test]
    fn reminders_can_still_be_switched_off() {
        let c: Config = toml::from_str("[reminders]\nenabled = false\n").unwrap();
        assert!(!c.reminders.enabled);
    }

    #[test]
    fn quiet_hours_parses_a_wrapping_window() {
        let c: Config =
            toml::from_str("[reminders]\nenabled = true\nquiet_hours = \"22:00-07:00\"\n").unwrap();
        assert!(c.reminders.enabled);
        assert_eq!(c.reminders.quiet_window(), Some((22 * 60, 7 * 60)));
    }

    #[test]
    fn unparseable_quiet_hours_is_ignored_not_fatal() {
        // A typo here must never keep the app from opening.
        for bad in ["22:00", "banana", "25:00-07:00", "22:00-22:00", ""] {
            let c: Config =
                toml::from_str(&format!("[reminders]\nquiet_hours = \"{bad}\"\n")).unwrap();
            assert_eq!(c.reminders.quiet_window(), None, "{bad} should not parse");
        }
    }
}
