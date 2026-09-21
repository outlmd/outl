//! Execution engine — filter, sort and collect matching blocks.

use super::dsl::{Filter, KindFilter, PropFilter, Query, SortKey, StatusFilter, TagFilter};
use chrono::{Duration, NaiveDate};
use outl_md::block_index::BlockEntry;
use outl_md::index::WorkspaceIndex;

/// Task state of a block, as read off its text prefix.
///
/// **This mirrors `outl_actions::TodoState`, which is the owner of
/// the marker vocabulary.** It cannot be imported: `outl-actions`
/// depends on this crate (for `run_code_block`), so the arrow only
/// points one way. Adding a state there means adding it here in the
/// same change — the pair is convention, not a compiler check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    /// Lowercase wire form used in `QueryHit.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
        }
    }
}

/// One query hit — the data we need to render an embed.
pub struct Hit {
    /// Block ref handle (`blk-XXXXXX`) for embed rendering.
    pub handle: String,
    /// Slug of the page hosting the block.
    pub page_slug: String,
    /// Task state, or `None` when the block is not a task.
    pub status: Option<Status>,
    /// Block text with the task prefix stripped.
    pub text: String,
}

/// Run `query` against `index`, returning all matching blocks.
pub fn run(index: &WorkspaceIndex, query: &Query) -> Vec<Hit> {
    let today = chrono::Local::now().date_naive();

    index
        .iter_blocks()
        .filter_map(|entry| {
            let (status, body) = split_todo(&entry.text);
            let page = index.by_slug(&entry.source_slug);

            for f in &query.filters {
                if !matches(f, entry, status, page.map(|p| p.is_journal), &today) {
                    return None;
                }
            }

            Some(Hit {
                handle: entry.ref_handle.clone(),
                page_slug: entry.source_slug.clone(),
                status,
                text: body.to_string(),
            })
        })
        .collect()
}

/// Sort hits by the given criteria, in priority order (last key first
/// so the first key dominates after stable sort).
pub fn sort_hits(hits: &mut [Hit], keys: &[SortKey]) {
    for key in keys.iter().rev() {
        match key {
            SortKey::Page => hits.sort_by(|a, b| a.page_slug.cmp(&b.page_slug)),
            // Unfinished work first, in the order it moves through:
            // TODO, DOING, DONE. A non-task sorts with TODO, which
            // is where it sat before DOING existed.
            SortKey::Status => hits.sort_by(|a, b| {
                let rank = |s: Option<Status>| s.unwrap_or(Status::Todo);
                rank(a.status).cmp(&rank(b.status))
            }),
            SortKey::Text => hits.sort_by(|a, b| a.text.cmp(&b.text)),
        }
    }
}

fn matches(
    f: &Filter,
    entry: &BlockEntry,
    status: Option<Status>,
    is_journal: Option<bool>,
    today: &NaiveDate,
) -> bool {
    match f {
        Filter::Status(sf) => match sf {
            StatusFilter::Todo => status == Some(Status::Todo),
            StatusFilter::Doing => status == Some(Status::Doing),
            StatusFilter::Done => status == Some(Status::Done),
            // `open` has always meant "is a task", DONE included —
            // kept as-is so existing queries don't change meaning
            // under the user on an upgrade.
            StatusFilter::Open => status.is_some(),
        },
        Filter::Tag(t) => has_tag(entry, t),
        Filter::Prop(p) => has_prop(entry, p),
        Filter::Kind(kf) => match kf {
            KindFilter::Journal => is_journal == Some(true),
            KindFilter::Page => is_journal != Some(true),
        },
        Filter::Since(days) => {
            is_journal == Some(true)
                && parse_journal_date(&entry.source_slug)
                    .map(|d| d >= *today - Duration::days(*days as i64))
                    .unwrap_or(false)
        }
        Filter::Text(needle) => entry.text_fold.contains(&needle.to_lowercase()),
        // Every `not-<key>` lands here. One `!` over the positive's
        // own arm is the whole implementation, which is what makes
        // `tag: x` plus `not-tag: x` return nothing: there is no
        // second matcher to disagree with the first.
        Filter::Not(inner) => !matches(inner, entry, status, is_journal, today),
    }
}

/// True when the block carries `#<name>` or a tag nested under it.
///
/// Routed through `outl_md::text_contains_tag_or_child` rather than a
/// `contains("#name")` on the cached fold: the substring form also
/// matches `#nameless`, which is harmless for `tag:` (an extra hit)
/// and silent data-hiding for `not-tag:` (a dropped one).
///
/// The fold is still the first gate. A boundary match is a subset of a
/// substring match, so a block whose text does not contain the needle
/// at all cannot match, and never reaches the tokenizer — which is
/// almost every block, on a filter that auto-runs on page load.
fn has_tag(entry: &BlockEntry, t: &TagFilter) -> bool {
    entry.text_fold.contains(t.needle())
        && outl_md::text_contains_tag_or_child(&entry.text, t.name())
}

/// True when the block carries property `key` — narrowed to one value
/// when the filter names one.
///
/// `BlockEntry::properties` is already lowercased, and so is
/// [`PropFilter`], so this compares folded against folded.
fn has_prop(entry: &BlockEntry, p: &PropFilter) -> bool {
    entry
        .properties
        .iter()
        .any(|(k, v)| k == &p.key && p.value.as_ref().is_none_or(|want| v == want))
}

/// Split a block's text into `(status, body)`, accepting both the
/// canonical word form (`"TODO body"`) and the CommonMark checkbox
/// form (`"[ ] body"`). Mirrors `outl_actions::split_todo` — see
/// [`Status`] for why it is a mirror and not a call.
///
/// The checkbox spellings have to be here too, or a block the user
/// wrote as `- [ ] ship it` renders a checkbox everywhere and then
/// fails to match `status: todo`, which is the same "it's a task
/// except where it isn't" split issue #230 was filed about.
fn split_todo(raw: &str) -> (Option<Status>, &str) {
    const PREFIXES: [(&str, Status); 7] = [
        ("TODO ", Status::Todo),
        ("DOING ", Status::Doing),
        ("DONE ", Status::Done),
        ("[ ] ", Status::Todo),
        ("[/] ", Status::Doing),
        ("[x] ", Status::Done),
        ("[X] ", Status::Done),
    ];
    // The marker may also sit after a single `"> "` quote prefix —
    // the legacy authoring shape (`"> TODO foo"`) that the TUI's
    // `split_block_prefixes` renders as a checkbox. The canonical
    // order (`"TODO > foo"`) already matches marker-first, and only
    // one quote marker is unwrapped, mirroring the "no nested
    // quotes" policy of `outl_actions::quote`.
    let after_quote = raw.strip_prefix("> ").unwrap_or(raw);
    for (prefix, status) in PREFIXES {
        if let Some(rest) = after_quote.strip_prefix(prefix) {
            return (Some(status), rest);
        }
    }
    (None, raw)
}

fn parse_journal_date(slug: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(slug, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standalone indexed block, so filter behaviour can be asserted
    /// without a workspace on disk.
    fn entry(text: &str, properties: &[(&str, &str)]) -> BlockEntry {
        BlockEntry {
            id: outl_core::NodeId::new(),
            ref_handle: "blk-aaaaaa".into(),
            source_slug: "notes".into(),
            source_path: std::path::PathBuf::from("pages/notes.md"),
            source_block_path: vec![0],
            text: text.into(),
            text_fold: text.to_lowercase(),
            // The index folds these on the way in; mirror that here so
            // the test exercises the same comparison production does.
            properties: properties
                .iter()
                .map(|(k, v)| (k.to_lowercase(), v.to_lowercase()))
                .collect(),
            children: Vec::new(),
        }
    }

    fn hits(e: &BlockEntry, f: &Filter) -> bool {
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let (status, _) = split_todo(&e.text);
        matches(f, e, status, None, &today)
    }

    fn tag(name: &str) -> TagFilter {
        TagFilter::new(name).expect("a non-empty tag name")
    }

    fn prop(raw: &str) -> PropFilter {
        PropFilter::new(raw).expect("a well-formed prop filter")
    }

    /// What every `not-<key>` directive parses to.
    fn not(f: Filter) -> Filter {
        Filter::Not(Box::new(f))
    }

    #[test]
    fn split_todo_open() {
        assert_eq!(
            split_todo("TODO buy milk"),
            (Some(Status::Todo), "buy milk")
        );
    }

    #[test]
    fn split_todo_doing() {
        assert_eq!(
            split_todo("DOING buy milk"),
            (Some(Status::Doing), "buy milk")
        );
    }

    #[test]
    fn split_todo_done() {
        assert_eq!(
            split_todo("DONE buy milk"),
            (Some(Status::Done), "buy milk")
        );
    }

    #[test]
    fn split_todo_none() {
        assert_eq!(split_todo("just text"), (None, "just text"));
    }

    #[test]
    fn split_todo_sees_a_marker_behind_a_quote_marker() {
        // Legacy authoring order. The TUI draws it as a checkbox,
        // so `status: todo` has to find it too.
        assert_eq!(
            split_todo("> TODO buy milk"),
            (Some(Status::Todo), "buy milk")
        );
        assert_eq!(
            split_todo("> [ ] buy milk"),
            (Some(Status::Todo), "buy milk")
        );
        assert_eq!(
            split_todo("> [x] buy milk"),
            (Some(Status::Done), "buy milk")
        );
        // Canonical order still works, quote and all.
        assert_eq!(
            split_todo("TODO > buy milk"),
            (Some(Status::Todo), "> buy milk")
        );
        // One quote marker only, matching `outl_actions::quote`.
        assert_eq!(split_todo("> > TODO x"), (None, "> > TODO x"));
        // A plain quote is not a task.
        assert_eq!(split_todo("> just a quote"), (None, "> just a quote"));
    }

    #[test]
    fn split_todo_reads_the_checkbox_spelling() {
        // A block the user typed as `- [ ] ship it` has to match
        // `status: todo`, or it renders a checkbox everywhere and
        // then goes missing from the query (issue #230).
        assert_eq!(split_todo("[ ] buy milk"), (Some(Status::Todo), "buy milk"));
        assert_eq!(
            split_todo("[/] buy milk"),
            (Some(Status::Doing), "buy milk")
        );
        assert_eq!(split_todo("[x] buy milk"), (Some(Status::Done), "buy milk"));
        // A link is not a checkbox.
        assert_eq!(
            split_todo("[x](https://example.com)"),
            (None, "[x](https://example.com)")
        );
    }

    #[test]
    fn doing_is_neither_todo_nor_done_to_a_filter() {
        // The whole point of the state: a query for open work must
        // not sweep up started work, and `status: done` must not
        // count something nobody finished.
        let e = entry("DOING ship the parser", &[]);
        for (filter, expected) in [
            (StatusFilter::Todo, false),
            (StatusFilter::Doing, true),
            (StatusFilter::Done, false),
            (StatusFilter::Open, true),
        ] {
            assert_eq!(
                hits(&e, &Filter::Status(filter)),
                expected,
                "{filter:?} against a DOING block"
            );
        }
    }

    #[test]
    fn not_tag_drops_the_block_carrying_the_tag() {
        let e = entry("TODO read the paper #research", &[]);
        assert!(hits(&e, &Filter::Tag(tag("research"))));
        assert!(!hits(&e, &not(Filter::Tag(tag("research")))));
    }

    #[test]
    fn not_tag_keeps_a_block_without_the_tag() {
        let e = entry("TODO ship the parser #work", &[]);
        assert!(hits(&e, &not(Filter::Tag(tag("research")))));
    }

    #[test]
    fn not_tag_stops_at_the_tag_boundary() {
        // Issue #323's open question 1. A substring negative would
        // drop this block for `not-tag: work`, hiding live work
        // behind a filter the user wrote to hide something else.
        let e = entry("TODO document the #workflow", &[]);
        assert!(hits(&e, &not(Filter::Tag(tag("work")))));
        assert!(!hits(&e, &Filter::Tag(tag("work"))));
    }

    #[test]
    fn a_tag_filter_still_answers_for_its_namespace_children() {
        // Documented behaviour of `tag:` since it shipped —
        // `#ops/deploy` matches `tag: ops` — and the negative has to
        // agree, or `tag: ops` plus `not-tag: ops` returns something.
        let e = entry("TODO roll it out #ops/deploy", &[]);
        assert!(hits(&e, &Filter::Tag(tag("ops"))));
        assert!(!hits(&e, &not(Filter::Tag(tag("ops")))));
    }

    #[test]
    fn tag_and_not_tag_on_the_same_name_can_never_both_match() {
        // The complement law, over the shapes most likely to break it.
        for text in [
            "plain text",
            "TODO ship #ops",
            "TODO ship #ops/deploy",
            "TODO ship #opsec",
            "TODO ship **#ops** bold",
            "TODO ship `#ops` in code",
            "TODO ship #Ops uppercase",
        ] {
            let e = entry(text, &[]);
            assert_ne!(
                hits(&e, &Filter::Tag(tag("ops"))),
                hits(&e, &not(Filter::Tag(tag("ops")))),
                "tag/not-tag disagree on {text:?}"
            );
        }
    }

    #[test]
    fn the_fold_gate_stays_a_superset_of_the_tokenizer() {
        // `has_tag` short-circuits on `text_fold.contains(needle)`, and
        // that is only sound while a boundary match implies a substring
        // match on the folded text. `to_lowercase` is context-free per
        // character with one exception — `Σ` folds to `ς` at the end of
        // a word and `σ` otherwise, decided by what follows. The needle
        // is folded from the name alone, the haystack from the whole
        // block, so in principle they could disagree and the gate could
        // hide a block the tokenizer matches (a dropped `tag:` result,
        // a kept `not-tag:` one).
        //
        // They cannot, and this is why: every character that keeps a
        // preceding `Σ` non-final is also a character the tokenizer
        // accepts *into* the tag name, so the name never ends there and
        // both sides fold the same bytes. Brute-forced rather than
        // argued, because it rests on Unicode's Cased set lining up
        // with Rust's `is_alphanumeric` — true today, nobody's promise.
        let divergent: Vec<char> = (0u32..0x11_0000)
            .filter_map(char::from_u32)
            .filter(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/')))
            .filter(|c| format!("ΑΣ{c}").to_lowercase().starts_with("ασ"))
            .collect();
        assert!(
            divergent.is_empty(),
            "the gate can now hide a tag the tokenizer matches, after: {divergent:?}"
        );

        // The ordinary Greek path, so the test is not vacuous.
        let e = entry("TODO ship #ΑΣ", &[]);
        assert!(hits(&e, &Filter::Tag(tag("ΑΣ"))));
        assert!(!hits(&e, &not(Filter::Tag(tag("ΑΣ")))));
    }

    #[test]
    fn a_hash_in_a_code_span_is_not_a_tag_to_either_side() {
        let e = entry("TODO run `#research` literally", &[]);
        assert!(!hits(&e, &Filter::Tag(tag("research"))));
        assert!(hits(&e, &not(Filter::Tag(tag("research")))));
    }

    #[test]
    fn not_prop_on_a_bare_key_drops_any_block_carrying_it() {
        let e = entry("TODO ship it", &[("status", "parked")]);
        assert!(hits(&e, &Filter::Prop(prop("status"))));
        assert!(!hits(&e, &not(Filter::Prop(prop("status")))));
    }

    #[test]
    fn not_prop_on_a_key_value_pair_only_drops_that_pair() {
        let e = entry("TODO ship it", &[("status", "parked")]);
        // A different value on the same key is not the pair excluded.
        assert!(hits(&e, &not(Filter::Prop(prop("status: done")))));
        assert!(!hits(&e, &not(Filter::Prop(prop("status: parked")))));
    }

    #[test]
    fn prop_matching_folds_case_on_both_halves() {
        let e = entry("TODO ship it", &[("Status", "Done")]);
        assert!(hits(&e, &Filter::Prop(prop("status: done"))));
        assert!(hits(&e, &Filter::Prop(prop("STATUS: DONE"))));
    }

    #[test]
    fn a_block_with_no_properties_survives_every_not_prop() {
        let e = entry("TODO ship it", &[]);
        assert!(hits(&e, &not(Filter::Prop(prop("status")))));
        assert!(hits(&e, &not(Filter::Prop(prop("status: done")))));
        assert!(!hits(&e, &Filter::Prop(prop("status"))));
    }

    #[test]
    fn no_filter_and_its_negation_can_both_match() {
        // The complement law, over every directive rather than the two
        // that happened to ship first. `Filter::Not` makes it true by
        // construction; this is what fails if somebody reintroduces a
        // hand-written `NotFoo` arm with its own matcher.
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let cases = [
            Filter::Status(StatusFilter::Todo),
            Filter::Status(StatusFilter::Open),
            Filter::Tag(tag("ops")),
            Filter::Prop(prop("status: done")),
            Filter::Kind(KindFilter::Journal),
            Filter::Since(7),
            Filter::Text("ship".into()),
        ];
        let blocks = [
            entry("TODO ship #ops", &[("status", "done")]),
            entry("DONE ship #opsec", &[]),
            entry("plain text", &[("status", "parked")]),
        ];
        for f in &cases {
            for e in &blocks {
                let (status, _) = split_todo(&e.text);
                for journal in [Some(true), Some(false), None] {
                    let yes = matches(f, e, status, journal, &today);
                    let no = matches(&not(f.clone()), e, status, journal, &today);
                    assert_ne!(yes, no, "{f:?} vs its negation on {:?}", e.text);
                }
            }
        }
    }

    #[test]
    fn prop_and_not_prop_on_the_same_target_can_never_both_match() {
        for props in [
            vec![],
            vec![("status", "done")],
            vec![("status", "parked")],
            vec![("priority", "p1")],
            vec![("status", "done"), ("status", "parked")],
        ] {
            let e = entry("TODO ship it", &props);
            for target in ["status", "status: done"] {
                assert_ne!(
                    hits(&e, &Filter::Prop(prop(target))),
                    hits(&e, &not(Filter::Prop(prop(target)))),
                    "prop/not-prop disagree on {props:?} for {target:?}"
                );
            }
        }
    }
}
