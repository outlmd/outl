//! Unit tests for the `remind::` grammar and its inverse.
//!
//! Split out of `remind.rs` the way `parse/tests.rs` was split out of
//! the grammar it covers: the module stays one piece, the file stays
//! under the size guard.

use super::*;

fn rule(s: &str) -> RemindRule {
    parse_remind(s).rule.expect("rule should parse")
}

#[test]
fn bare_time_is_a_single_shot() {
    let r = rule("10am");
    assert_eq!(
        r.anchor,
        RemindAnchor::At {
            hour: 10,
            minute: 0
        }
    );
    assert_eq!(r.every_minutes, None);
    assert_eq!(r.stop(), RemindStop::Done);
    assert!(!r.repeats());
}

#[test]
fn every_clause_sets_the_interval() {
    assert_eq!(rule("10am every 1h").every_minutes, Some(60));
    assert_eq!(rule("3pm every 30min").every_minutes, Some(30));
    assert_eq!(rule("15:00 every 2d").every_minutes, Some(2880));
}

#[test]
fn until_done_is_the_implicit_default() {
    assert_eq!(rule("10am every 1h").stop(), RemindStop::Done);
    assert_eq!(rule("10am every 1h until done").stop(), RemindStop::Done);
}

#[test]
fn until_accepts_time_and_iso_date() {
    assert_eq!(
        rule("10am every 1h until 6pm").until,
        Some(RemindStop::Time {
            hour: 18,
            minute: 0
        })
    );
    assert_eq!(
        rule("10am every 1d until 2026-12-20").until,
        Some(RemindStop::Date(
            NaiveDate::from_ymd_opt(2026, 12, 20).expect("valid date")
        ))
    );
}

#[test]
fn now_anchor_parses() {
    assert_eq!(rule("now every 15min until DONE").anchor, RemindAnchor::Now);
}

#[test]
fn am_pm_edge_hours_map_to_24h() {
    assert_eq!(rule("12am").anchor, RemindAnchor::At { hour: 0, minute: 0 });
    assert_eq!(
        rule("12pm").anchor,
        RemindAnchor::At {
            hour: 12,
            minute: 0
        }
    );
    assert_eq!(
        rule("1:30pm").anchor,
        RemindAnchor::At {
            hour: 13,
            minute: 30
        }
    );
}

#[test]
fn missing_anchor_is_rejected() {
    let p = parse_remind("every 1h");
    assert!(p.rule.is_none());
    assert_eq!(p.warnings, vec![ParseWarningKind::RemindMissingAnchor]);
    assert_eq!(
        parse_remind("").warnings,
        vec![ParseWarningKind::RemindMissingAnchor]
    );
}

#[test]
fn invalid_time_is_rejected() {
    for bad in ["25:00", "10:70", "13am", "0pm", "banana"] {
        let p = parse_remind(bad);
        assert!(p.rule.is_none(), "{bad} should not parse");
        assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidTime]);
    }
}

#[test]
fn sub_minute_interval_is_rejected() {
    let p = parse_remind("10am every 30s");
    assert!(p.rule.is_none());
    assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidInterval]);
}

#[test]
fn unparseable_stop_is_rejected() {
    let p = parse_remind("10am until yesterday");
    assert!(p.rule.is_none());
    assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidStop]);
}

#[test]
fn max_above_the_cap_is_clamped_not_rejected() {
    let p = parse_remind("10am every 1h max 50");
    assert_eq!(p.rule.expect("still schedules").max_fires, Some(10));
    assert_eq!(p.warnings, vec![ParseWarningKind::RemindMaxClamped]);
    assert_eq!(rule("10am every 1h max 5").max_fires, Some(5));
}

#[test]
fn until_before_the_anchor_drops_the_clause() {
    let p = parse_remind("3pm every 1h until 10am");
    let r = p.rule.expect("rule survives, stop does not");
    assert_eq!(r.until, None);
    assert_eq!(p.warnings, vec![ParseWarningKind::RemindInvalidStop]);
}

#[test]
fn dangling_keyword_does_not_panic() {
    assert!(parse_remind("10am every").rule.is_none());
    assert!(parse_remind("10am until").rule.is_none());
    assert!(parse_remind("10am max").rule.is_none());
}

#[test]
fn parsing_is_case_insensitive() {
    assert_eq!(
        rule("3PM EVERY 1H UNTIL DONE").anchor,
        RemindAnchor::At {
            hour: 15,
            minute: 0
        }
    );
}

/// The property the pair exists for: anything the parser accepts
/// survives being written back out and read again.
///
/// Spelled over strings rather than over hand-built rules on
/// purpose — the domain `format_remind` promises to cover is "what
/// `parse_remind` produces", and a generated `RemindRule` would be
/// testing a wider claim than the function makes.
///
/// The comparison goes through `stop()` rather than `until`,
/// because an explicit `until DONE` and no clause at all are one
/// schedule with one short spelling. Comparing the raw `Option`
/// would pin the longer spelling as if it carried meaning.
#[test]
fn every_rule_the_parser_accepts_survives_a_round_trip() {
    let same = |a: &RemindRule, b: &RemindRule| {
        a.anchor == b.anchor
            && a.every_minutes == b.every_minutes
            && a.stop() == b.stop()
            && a.max_fires == b.max_fires
    };
    for src in [
        "now",
        "10am",
        "3pm every 1h",
        "15:00 every 30min until 18:00",
        "1:30pm every 2d until 2026-12-20",
        "now every 15min until DONE",
        "9:05 every 1h max 5",
        "12am every 90min",
        "10am every 1h max 50",
    ] {
        let parsed = rule(src);
        let written = format_remind(&parsed);
        let back = parse_remind(&written).rule.expect("written form parses");
        assert!(
            same(&parsed, &back),
            "{src} -> {written} did not round-trip: {parsed:?} vs {back:?}"
        );
    }
}

#[test]
fn the_written_form_is_the_shortest_one_that_means_it() {
    // `until DONE` is implicit, 24h is unambiguous, and an
    // interval takes the largest unit that spells it exactly.
    assert_eq!(
        format_remind(&rule("3pm every 1h until done")),
        "15:00 every 1h"
    );
    assert_eq!(format_remind(&rule("10am every 2d")), "10:00 every 2d");
    assert_eq!(
        format_remind(&rule("10am every 90min")),
        "10:00 every 90min"
    );
    assert_eq!(format_remind(&rule("now")), "now");
}

/// An interval too large to spell as hours or days, and too large
/// for the count cap, has no grammar — and the written form must
/// then fail to parse rather than mean a different schedule.
#[test]
fn an_unspellable_interval_re_parses_as_invalid_not_as_something_else() {
    let bogus = RemindRule {
        anchor: RemindAnchor::Now,
        every_minutes: Some(1001),
        until: None,
        max_fires: None,
    };
    let written = format_remind(&bogus);
    assert_eq!(written, "now every 1001min");
    assert!(parse_remind(&written).rule.is_none());
}

#[test]
fn rule_is_read_off_the_property_list() {
    let props = vec![
        ("id".to_string(), "x".to_string()),
        ("remind".to_string(), "10am".to_string()),
    ];
    assert!(rule_from_properties(&props)
        .expect("has remind")
        .rule
        .is_some());
    assert!(rule_from_properties(&[]).is_none());
}
