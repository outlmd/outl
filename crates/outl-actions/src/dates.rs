//! Pure date domain — human-typed date **parsing**, journal label
//! **formatting**, and calendar **arithmetic**.
//!
//! ## Parsing
//!
//! [`parse_flexible_date`] used to live as four near-identical copies
//! (paste normalization, `outl daily`, `outl import`, the Obsidian
//! frontmatter importer), each accepting a slightly different subset
//! of spellings. The unified parser is the **superset** of all four:
//!
//! - ISO: `2026-04-22`
//! - Slashed: `2026/04/22`, `22/04/2026` (day-first, Obsidian style)
//! - Long form with ordinal: `April 22nd, 2026`, `Apr 22nd, 2026`,
//!   `Sept 3rd, 2025` (Roam / Logseq journal titles)
//! - Day-first long form: `22 April 2026`, `22nd Apr 2026`
//!
//! Every accepted form is validated through chrono, so
//! `February 30th, 2026` is *not* a date (the old paste copy happily
//! produced the invalid label `2026-02-30`). Non-ISO forms also
//! require a year in 1900–2999 so prose fragments like `12/04/26`
//! never round-trip into a bogus first-century journal slug.
//!
//! [`parse_date_arg`] layers **relative offsets** (`+3d`, `-2w`,
//! `+1m`) on top for the slash-command / CLI-argument surfaces.
//!
//! ## Journal labels & arithmetic
//!
//! [`journal_slug`] / [`journal_title`] / [`journal_ref`] /
//! [`date_from_slug`] own the canonical `YYYY-MM-DD` shape journals
//! use everywhere (slug, title, `[[date]]` refs). [`week_tag`] and
//! [`days_until_next_weekday`] cover the week-oriented arithmetic the
//! date slash-commands need.
//!
//! Everything here is pure (`&str` / [`NaiveDate`] in, value out).
//! No `Workspace`, no clock: "what does *today* mean" stays in
//! [`crate::clock`] — functions that need an anchor date take it as a
//! parameter. Keyword shortcuts like `today` / `yesterday` stay in the
//! caller that owns them.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};

/// Plausible year window for the non-ISO forms. Anything outside is
/// treated as "not a date" so ambiguous fragments don't parse.
const YEAR_RANGE: std::ops::RangeInclusive<i32> = 1900..=2999;

/// Parse a human-typed date in any of the supported spellings.
///
/// Returns `None` when the input doesn't look like a date — callers
/// must fall back to their own error / verbatim-text path.
pub fn parse_flexible_date(raw: &str) -> Option<NaiveDate> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Some(d) = parse_slashed(s) {
        return Some(d);
    }
    if let Some(d) = parse_long_form(s) {
        return Some(d);
    }
    parse_day_first(s)
}

/// Like [`parse_flexible_date`], but returns the ISO `YYYY-MM-DD`
/// label outl uses for journal slugs and `[[date]]` refs.
pub fn parse_date_label(raw: &str) -> Option<String> {
    parse_flexible_date(raw).map(journal_slug)
}

/// Parse a user-supplied date **argument**: relative to `today`
/// (`+3d`, `-2w`, `+1m` — also bare `5d`, treated as positive) or
/// absolute in any spelling [`parse_flexible_date`] accepts.
///
/// This is the superset the `/date` slash command exposes. Relative
/// offsets are tried first; the suffix grammar (`digits` + `d`/`w`/`m`)
/// is disjoint from every absolute form, so the order can't
/// misclassify. Returns `None` on unrecognized input so the caller can
/// surface a usage hint.
pub fn parse_date_arg(arg: &str, today: NaiveDate) -> Option<NaiveDate> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    if let Some(d) = parse_relative_offset(arg, today) {
        return Some(d);
    }
    parse_flexible_date(arg)
}

/// `+Nd`, `-Nw`, `+Nm` (and bare `Nd` / `Nw` / `Nm`) relative to
/// `today`.
fn parse_relative_offset(arg: &str, today: NaiveDate) -> Option<NaiveDate> {
    let (sign, rest) = if let Some(rest) = arg.strip_prefix('+') {
        (1i64, rest)
    } else if let Some(rest) = arg.strip_prefix('-') {
        (-1i64, rest)
    } else {
        (1i64, arg)
    };
    let last = rest.chars().last()?;
    if !"dwm".contains(last) {
        return None;
    }
    let num_str = &rest[..rest.len() - last.len_utf8()];
    let n: i64 = num_str.parse().ok()?;
    let signed = sign * n;
    // Checked, like the month branch below: `NaiveDate + TimeDelta`
    // panics on overflow, and this parser is fed straight from the TUI's
    // `/date`, so a typo with enough digits took the app down instead of
    // being rejected. `try_days` catches the `TimeDelta` that cannot be
    // built; `checked_add_signed` catches the date that cannot hold it.
    match last {
        'd' => Duration::try_days(signed).and_then(|d| today.checked_add_signed(d)),
        'w' => Duration::try_weeks(signed).and_then(|d| today.checked_add_signed(d)),
        'm' => {
            // Months need calendar-aware arithmetic — `chrono::Months`
            // clamps on overflow (Jan 31 + 1 month → Feb 28/29).
            let months = u32::try_from(signed.unsigned_abs()).ok()?;
            if signed >= 0 {
                today.checked_add_months(Months::new(months))
            } else {
                today.checked_sub_months(Months::new(months))
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Journal labels
// ---------------------------------------------------------------------------

/// Slug for `date` using the canonical `YYYY-MM-DD` shape.
pub fn journal_slug(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Display title for `date`. We use ISO `YYYY-MM-DD` because it
/// matches the slug 1:1, sorts naturally, and stays compact in
/// constrained UI (mobile header).
pub fn journal_title(date: NaiveDate) -> String {
    journal_slug(date)
}

/// `[[YYYY-MM-DD]]` — the journal ref for `date` as it appears inside
/// block text. One owner so every surface that inserts a date link
/// (slash commands, autocomplete, importers) emits the same shape.
pub fn journal_ref(date: NaiveDate) -> String {
    format!("[[{}]]", journal_slug(date))
}

/// Parse a `YYYY-MM-DD` slug back into a `NaiveDate`.
pub fn date_from_slug(slug: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(slug, "%Y-%m-%d").ok()
}

/// Previous calendar day relative to `date`.
pub fn previous_journal_date(date: NaiveDate) -> NaiveDate {
    date - Duration::days(1)
}

/// Next calendar day relative to `date`.
pub fn next_journal_date(date: NaiveDate) -> NaiveDate {
    date + Duration::days(1)
}

// ---------------------------------------------------------------------------
// Week arithmetic
// ---------------------------------------------------------------------------

/// `#YYYY-Www` tag for the ISO week of `date` (e.g. `#2026-W21`).
/// Uses ISO 8601 week numbering — weeks start on Monday and `%G`
/// is the ISO week-numbering year (which can differ from `%Y` for
/// a handful of days near year boundaries).
pub fn week_tag(date: NaiveDate) -> String {
    format!("#{}", date.format("%G-W%V"))
}

/// Number of days from `today` to the next occurrence of `weekday`,
/// strictly in the future. If today already is that weekday, returns
/// 7 (next week's same day), not 0 — `/date-next-monday` on a Monday
/// should mean "next Monday", not "today".
pub fn days_until_next_weekday(today: NaiveDate, weekday: Weekday) -> i64 {
    let today_n = today.weekday().num_days_from_monday() as i64;
    let target_n = weekday.num_days_from_monday() as i64;
    let diff = (target_n - today_n).rem_euclid(7);
    if diff == 0 {
        7
    } else {
        diff
    }
}

// ---------------------------------------------------------------------------
// Flexible-parse internals
// ---------------------------------------------------------------------------

/// `2026/04/22` (ISO-with-slashes) or `22/04/2026` (day-first).
fn parse_slashed(s: &str) -> Option<NaiveDate> {
    for fmt in ["%Y/%m/%d", "%d/%m/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            if YEAR_RANGE.contains(&d.year()) {
                return Some(d);
            }
        }
    }
    None
}

/// Roam / Logseq long form: `<Month> <day><ordinal>, <year>`.
///
/// The month table is deliberately manual (not chrono's `%B`) because
/// it also accepts the 4-letter `Sept` abbreviation Logseq emits,
/// which chrono rejects.
fn parse_long_form(s: &str) -> Option<NaiveDate> {
    let comma = s.rfind(", ")?;
    let (left, year_str) = (&s[..comma], &s[comma + 2..]);
    let year: i32 = year_str.trim().parse().ok()?;
    if !YEAR_RANGE.contains(&year) {
        return None;
    }
    let space = left.find(' ')?;
    let (month_name, day_part) = (&left[..space], &left[space + 1..]);
    let month = month_number(month_name)?;
    let day = strip_ordinal_suffixes(day_part)
        .trim()
        .parse::<u32>()
        .ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Obsidian / moment.js day-first long form: `22 April 2026`
/// (optionally with an ordinal suffix on the day).
fn parse_day_first(s: &str) -> Option<NaiveDate> {
    let stripped = strip_ordinal_suffixes(s);
    // chrono's `%B` accepts both full and 3-letter month names.
    let d = NaiveDate::parse_from_str(stripped.trim(), "%d %B %Y").ok()?;
    if YEAR_RANGE.contains(&d.year()) {
        Some(d)
    } else {
        None
    }
}

fn month_number(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

/// Remove `1st` / `2nd` / `3rd` / `4th`…`31st` ordinal suffixes so the
/// day token parses as a plain number.
fn strip_ordinal_suffixes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if !c.is_ascii_digit() {
            continue;
        }
        // Peek the next two chars; drop them when they form a known
        // ordinal pair immediately following a digit.
        let mut clone = chars.clone();
        let pair = (clone.next(), clone.next());
        let is_suffix = matches!(
            pair,
            (Some('s'), Some('t'))
                | (Some('n'), Some('d'))
                | (Some('r'), Some('d'))
                | (Some('t'), Some('h'))
        );
        if is_suffix {
            chars.next();
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn iso_parses() {
        assert_eq!(parse_flexible_date("2026-04-22"), Some(d(2026, 4, 22)));
        assert_eq!(parse_flexible_date(" 2026-04-22 "), Some(d(2026, 4, 22)));
    }

    #[test]
    fn slashed_forms_parse() {
        assert_eq!(parse_flexible_date("2026/04/22"), Some(d(2026, 4, 22)));
        assert_eq!(parse_flexible_date("22/04/2026"), Some(d(2026, 4, 22)));
    }

    #[test]
    fn roam_long_form_parses() {
        assert_eq!(
            parse_flexible_date("April 22nd, 2026"),
            Some(d(2026, 4, 22))
        );
        assert_eq!(
            parse_flexible_date("January 1st, 2025"),
            Some(d(2025, 1, 1))
        );
        assert_eq!(
            parse_flexible_date("October 3rd, 2025"),
            Some(d(2025, 10, 3))
        );
        assert_eq!(parse_flexible_date("July 11th, 2026"), Some(d(2026, 7, 11)));
    }

    #[test]
    fn abbreviated_months_parse() {
        assert_eq!(parse_flexible_date("Apr 22nd, 2026"), Some(d(2026, 4, 22)));
        // `Sept` is the 4-letter form chrono rejects but Logseq emits.
        assert_eq!(parse_flexible_date("Sept 3rd, 2025"), Some(d(2025, 9, 3)));
    }

    #[test]
    fn month_names_are_case_insensitive() {
        assert_eq!(
            parse_flexible_date("april 22nd, 2026"),
            Some(d(2026, 4, 22))
        );
    }

    #[test]
    fn day_first_forms_parse() {
        assert_eq!(parse_flexible_date("22 April 2026"), Some(d(2026, 4, 22)));
        assert_eq!(parse_flexible_date("22 Apr 2026"), Some(d(2026, 4, 22)));
        assert_eq!(parse_flexible_date("22nd April 2026"), Some(d(2026, 4, 22)));
    }

    #[test]
    fn impossible_dates_are_rejected() {
        // The old paste-normalization copy produced the invalid label
        // `2026-02-30` here; chrono validation must reject it.
        assert_eq!(parse_flexible_date("February 30th, 2026"), None);
        assert_eq!(parse_flexible_date("April 31st, 2026"), None);
    }

    #[test]
    fn implausible_years_are_rejected() {
        // Ambiguous fragment: neither `%Y/%m/%d` (year 12) nor
        // `%d/%m/%Y` (year 26) has a plausible year.
        assert_eq!(parse_flexible_date("12/04/26"), None);
        assert_eq!(parse_flexible_date("April 22nd, 26"), None);
    }

    #[test]
    fn non_dates_are_rejected() {
        assert_eq!(parse_flexible_date(""), None);
        assert_eq!(parse_flexible_date("not a date"), None);
        assert_eq!(parse_flexible_date("Avelino"), None);
        assert_eq!(parse_flexible_date("June 2nd"), None); // missing year
        assert_eq!(parse_flexible_date("Project X"), None);
    }

    #[test]
    fn label_form_returns_iso() {
        assert_eq!(
            parse_date_label("April 22nd, 2026").as_deref(),
            Some("2026-04-22")
        );
        assert_eq!(
            parse_date_label("2026/04/22").as_deref(),
            Some("2026-04-22")
        );
        assert_eq!(parse_date_label("plain ref"), None);
    }

    #[test]
    fn ordinal_stripper_handles_edge_cases() {
        assert_eq!(strip_ordinal_suffixes("1st"), "1");
        assert_eq!(strip_ordinal_suffixes("22nd"), "22");
        assert_eq!(strip_ordinal_suffixes("3rd"), "3");
        assert_eq!(strip_ordinal_suffixes("4th"), "4");
        assert_eq!(strip_ordinal_suffixes("plain text"), "plain text");
    }

    // -- parse_date_arg ------------------------------------------------------

    #[test]
    fn parse_arg_absolute_iso_date() {
        let today = d(2026, 5, 26);
        assert_eq!(parse_date_arg("2026-06-15", today), Some(d(2026, 6, 15)));
    }

    #[test]
    fn parse_arg_absolute_delegates_to_flexible_forms() {
        // The unified arg parser accepts every `parse_flexible_date`
        // spelling, not just ISO (the old TUI copy was ISO-only).
        let today = d(2026, 5, 26);
        assert_eq!(
            parse_date_arg("April 22nd, 2026", today),
            Some(d(2026, 4, 22))
        );
        assert_eq!(parse_date_arg("22/04/2026", today), Some(d(2026, 4, 22)));
    }

    #[test]
    fn parse_arg_positive_days_offset() {
        assert_eq!(parse_date_arg("+3d", d(2026, 5, 26)), Some(d(2026, 5, 29)));
    }

    #[test]
    fn parse_arg_negative_weeks_offset() {
        assert_eq!(parse_date_arg("-2w", d(2026, 5, 26)), Some(d(2026, 5, 12)));
    }

    #[test]
    fn parse_arg_months_offset_clamps_in_short_months() {
        // Jan 31 + 1 month → Feb 28 (or 29 in leap years). chrono::Months
        // clamps to the last valid day, which is exactly what we want.
        assert_eq!(parse_date_arg("+1m", d(2026, 1, 31)), Some(d(2026, 2, 28)));
    }

    #[test]
    fn parse_arg_bare_offset_is_positive() {
        // No sign means `+` — a UX kindness.
        assert_eq!(parse_date_arg("5d", d(2026, 5, 26)), Some(d(2026, 5, 31)));
    }

    #[test]
    fn parse_arg_rejects_garbage() {
        let today = d(2026, 5, 26);
        assert!(parse_date_arg("nope", today).is_none());
        assert!(parse_date_arg("+3x", today).is_none());
        assert!(parse_date_arg("", today).is_none());
        assert!(parse_date_arg("2026-13-99", today).is_none());
    }

    // -- journal labels ------------------------------------------------------

    #[test]
    fn journal_labels_round_trip() {
        let date = d(2026, 5, 27);
        assert_eq!(journal_slug(date), "2026-05-27");
        assert_eq!(journal_title(date), "2026-05-27");
        assert_eq!(journal_ref(date), "[[2026-05-27]]");
        assert_eq!(date_from_slug("2026-05-27"), Some(date));
        assert_eq!(date_from_slug("not-a-date"), None);
    }

    #[test]
    fn previous_and_next_journal_dates() {
        let date = d(2026, 1, 1);
        assert_eq!(previous_journal_date(date), d(2025, 12, 31));
        assert_eq!(next_journal_date(date), d(2026, 1, 2));
    }

    // -- week arithmetic -----------------------------------------------------

    #[test]
    fn next_weekday_on_same_weekday_jumps_seven() {
        // 2026-05-25 is a Monday.
        assert_eq!(days_until_next_weekday(d(2026, 5, 25), Weekday::Mon), 7);
    }

    #[test]
    fn next_weekday_in_same_week() {
        // Monday → Friday is 4 days.
        assert_eq!(days_until_next_weekday(d(2026, 5, 25), Weekday::Fri), 4);
    }

    #[test]
    fn next_weekday_wraps_across_weekend() {
        // Friday → Monday is 3 days.
        assert_eq!(days_until_next_weekday(d(2026, 5, 29), Weekday::Mon), 3);
    }

    #[test]
    fn next_weekday_one_day_forward() {
        // Tuesday → Wednesday is 1 day.
        assert_eq!(days_until_next_weekday(d(2026, 5, 26), Weekday::Wed), 1);
    }

    #[test]
    fn week_tag_format_is_hash_year_w_week() {
        // 2026-05-25 is Monday of ISO week 22 of year 2026.
        assert_eq!(week_tag(d(2026, 5, 25)), "#2026-W22");
    }

    #[test]
    fn week_tag_uses_iso_week_year_at_boundary() {
        // 2025-12-31 (Wed) is ISO week 1 of 2026 — `%G` (ISO year)
        // differs from `%Y` (calendar year) here. Confirms we used %G.
        assert_eq!(week_tag(d(2025, 12, 31)), "#2026-W01");
    }

    /// `NaiveDate + Duration` panics on overflow, and this parser is fed
    /// straight from the TUI's `/date` command — so a typo with enough
    /// digits took the app down rather than being rejected. The `m`
    /// branch already used `checked_*`; `d` and `w` did not.
    #[test]
    fn an_absurd_offset_is_refused_rather_than_panicking() {
        let today = d(2026, 10, 24);
        for arg in [
            "99999999999d",
            "-99999999999d",
            "99999999999w",
            "999999999999999999m",
        ] {
            assert_eq!(parse_date_arg(arg, today), None, "{arg}");
        }
    }
}
