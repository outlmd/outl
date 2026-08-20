//! The workspace's property-key catalogue.
//!
//! Every client offers "add a property" and every client then has to
//! answer the same question: *which key?* Typing it from memory is the
//! wrong default. In a real graph the keys are few and repeat, so the
//! useful affordance is a ranked list of what this workspace already
//! uses, not a blank field.
//!
//! One owner, three consumers (TUI overlay, desktop chip row, mobile
//! sheet). A per-client list would drift, which is the failure the
//! shared task-state work already paid to remove.

use std::collections::HashMap;

use outl_core::workspace::Workspace;

/// Property keys used anywhere in the workspace, most-used first.
///
/// Ties break alphabetically so the order is stable across runs — the
/// underlying map is a `HashMap`, and a list that reshuffles under the
/// user between two openings of the same menu is unusable.
///
/// **Grouped case-insensitively**, matching
/// [`outl_core::tree::Tree::nodes_with_property`]: the dialect accepts
/// `Remind::` and `remind::` as one property, so the catalogue must
/// not offer them as two. The spelling returned is the most frequent
/// one, and the tie there breaks alphabetically too, so a workspace
/// with one `Related::` and one `related::` always suggests the same.
///
/// `O(total properties)` — a map scan, no block text touched, no tree
/// walk.
pub fn known_keys(workspace: &Workspace) -> Vec<(String, usize)> {
    // Fold by lowercase key, keeping a count per exact spelling so the
    // winner can be the one the user actually types. ASCII folding, not
    // Unicode: the dialect's own matching is `eq_ignore_ascii_case`, so
    // a Unicode fold here would merge keys the tree treats as distinct.
    let mut folded: HashMap<String, HashMap<&str, usize>> = HashMap::new();
    for (_, key, _) in workspace.tree().iter_properties() {
        // Bookkeeping is not metadata the user authors. Offering it in
        // an "add a property" menu invites editing a field the app owns
        // — `from-template` is how a template instance is traced, and
        // `id` / `collapsed` ride in from imported graphs.
        if !is_suggestable_key(key) {
            continue;
        }
        *folded
            .entry(key.to_ascii_lowercase())
            .or_default()
            .entry(key)
            .or_insert(0) += 1;
    }

    let mut out: Vec<(String, usize)> = folded
        .into_values()
        .map(|spellings| {
            let total: usize = spellings.values().sum();
            let best = spellings
                .into_iter()
                .max_by(|(a_key, a_n), (b_key, b_n)| {
                    // Higher count wins; equal counts fall back to the
                    // alphabetically first spelling.
                    a_n.cmp(b_n).then_with(|| b_key.cmp(a_key))
                })
                .map(|(key, _)| key.to_string())
                .unwrap_or_default();
            (best, total)
        })
        .collect();

    out.sort_by(|(a_key, a_n), (b_key, b_n)| b_n.cmp(a_n).then_with(|| a_key.cmp(b_key)));
    out
}

/// Whether a key belongs in an "add a property" menu.
///
/// Excludes the page model's own fields (via
/// [`crate::tree::is_page_model_key`]) plus the bookkeeping a user
/// never authors by hand: `from-template` (how a template instance is
/// traced back), and `id` / `collapsed`, which arrive with imported
/// Logseq graphs and mean nothing to outl's own model.
///
/// This is about *suggesting*, not about permission: a user who types
/// `collapsed` still gets it written. The menu just does not propose it.
fn is_suggestable_key(key: &str) -> bool {
    if crate::tree::is_page_model_key(key) {
        return false;
    }
    !matches!(
        key.to_lowercase().as_str(),
        crate::template::FROM_TEMPLATE_KEY | "id" | "collapsed"
    )
}

/// Clean up what a user typed into a property key.
///
/// Strips the trailing `::` (the dialect's separator, which the user
/// sees as part of the key everywhere it is written) and trims. That
/// is all: a key is otherwise taken literally, because an existing
/// key is edited and deleted through the same string.
///
/// **Call this at the edge that reads the user's keystrokes**, never
/// inside the writer. `set_property` is also how a chip is edited and
/// deleted, and normalising there makes a legitimate `foo:` key (which
/// the parser accepts, and imported graphs contain) impossible to
/// touch: the delete would target `foo` and leave `foo:` on screen.
pub fn normalize_key(raw: &str) -> String {
    raw.trim().trim_end_matches(':').trim().to_string()
}

/// Why a key cannot be used, or `None` when it can.
///
/// Separate from [`normalize_key`] because these are refusals the user
/// has to see, not silent repairs. Whitespace is the sharp one: the
/// `.md` grammar rejects a key containing any (`outl_md`'s
/// `is_valid_key`), so a key like `date captured` would take the op,
/// render, and then fail to parse back — the property would simply
/// vanish on the next read, and in a page header it takes every
/// property below it along.
pub fn key_rejection(key: &str) -> Option<String> {
    if key.is_empty() {
        return Some("property key cannot be empty".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Some(format!(
            "`{key}` cannot be a property key: keys cannot contain spaces (try `{}`)",
            key.split_whitespace().collect::<Vec<_>>().join("-")
        ));
    }
    if crate::tree::is_page_model_key(key) {
        return Some(format!(
            "`{key}` defines the page and cannot be edited as a property; rename the page instead"
        ));
    }
    None
}

/// A page's user-facing properties (`icon::`, `type::`, `title::`, …),
/// alpha-sorted, with the structural keys removed.
///
/// Same shape as `OutlineNode.properties`, so a client can hand both
/// to the same editor component.
pub fn page_properties(
    workspace: &Workspace,
    page: outl_core::id::NodeId,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = workspace
        .tree()
        .properties_of(page)
        // `page-slug` / `page-kind` live in this same map but *are*
        // the page's identity, not user metadata. `crate::tree` is the
        // single owner of that distinction; the page and block
        // renderers filter by the same predicate.
        .filter(|(k, _)| !crate::tree::is_page_model_key(k))
        .map(|(k, v)| (k.to_string(), crate::outline::prop_value_to_string(v)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::set_property;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::property::PropValue;

    fn workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn block_with(ws: &mut Workspace, hlc: &HlcGenerator, props: &[(&str, &str)]) {
        let n = append_block(ws, hlc, None, Some("a block")).unwrap();
        for (k, v) in props {
            set_property(ws, hlc, n, k, Some(PropValue::Text((*v).into()))).unwrap();
        }
    }

    #[test]
    fn ranks_keys_by_how_often_they_are_used() {
        let (mut ws, hlc) = workspace();
        block_with(&mut ws, &hlc, &[("related", "x"), ("work", "y")]);
        block_with(&mut ws, &hlc, &[("related", "z")]);
        block_with(&mut ws, &hlc, &[("related", "w")]);

        let keys = known_keys(&ws);
        assert_eq!(keys[0], ("related".to_string(), 3));
        assert_eq!(keys[1], ("work".to_string(), 1));
    }

    #[test]
    fn one_key_in_two_casings_is_one_entry() {
        // The dialect resolves `Remind::` and `remind::` to the same
        // property, so a catalogue that lists both offers the user a
        // choice that does not exist.
        let (mut ws, hlc) = workspace();
        block_with(&mut ws, &hlc, &[("Related", "x")]);
        block_with(&mut ws, &hlc, &[("related", "y")]);
        block_with(&mut ws, &hlc, &[("related", "z")]);

        let keys = known_keys(&ws);
        assert_eq!(keys.len(), 1);
        // Three uses total, and the majority spelling is what a user
        // gets suggested.
        assert_eq!(keys[0], ("related".to_string(), 3));
    }

    #[test]
    fn the_order_is_stable_when_counts_tie() {
        // Backed by a HashMap, so without an explicit tiebreak the
        // menu reshuffles between two openings.
        let (mut ws, hlc) = workspace();
        block_with(&mut ws, &hlc, &[("zebra", "1"), ("alpha", "2")]);

        for _ in 0..8 {
            let keys: Vec<String> = known_keys(&ws).into_iter().map(|(k, _)| k).collect();
            assert_eq!(keys, vec!["alpha".to_string(), "zebra".to_string()]);
        }
    }

    #[test]
    fn an_empty_workspace_has_no_keys() {
        let (ws, _) = workspace();
        assert!(known_keys(&ws).is_empty());
    }

    #[test]
    fn page_properties_hide_the_keys_that_define_the_page() {
        // `page-slug` is what the filename and every `[[ref]]` resolve
        // through; offering it as an editable chip is offering the user
        // a way to break every link into the page.
        let (mut ws, hlc) = workspace();
        let page = crate::page::open_or_create(
            &mut ws,
            &hlc,
            "notes",
            "notes",
            crate::page::PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut ws,
            &hlc,
            page,
            "icon",
            Some(PropValue::Text("📌".into())),
        )
        .unwrap();

        let props = page_properties(&ws, page);
        let keys: Vec<&str> = props.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"icon"));
        assert!(
            !keys.contains(&"page-slug"),
            "structural key leaked: {keys:?}"
        );
        assert!(
            !keys.contains(&"page-kind"),
            "structural key leaked: {keys:?}"
        );
    }

    #[test]
    fn normalize_key_covers_what_a_user_actually_types() {
        assert_eq!(normalize_key("oura-date::"), "oura-date");
        assert_eq!(normalize_key("  related::  "), "related");
        assert_eq!(normalize_key("related"), "related");
        // A single colon is the same slip.
        assert_eq!(normalize_key("related:"), "related");
        // Whitespace is NOT repaired here: `is_valid_key` in outl-md
        // rejects any key containing it, so a collapsed `date captured`
        // would take the op, render, and then fail to parse back — the
        // property vanishing on the next read. `key_rejection` refuses
        // it out loud instead.
        assert_eq!(normalize_key("date  captured"), "date  captured");
        // Nothing left is nothing — the caller rejects an empty key.
        assert_eq!(normalize_key("::"), "");
        assert_eq!(normalize_key("   "), "");
    }

    #[test]
    fn a_property_key_keeps_its_inner_colons() {
        // Only the trailing separator goes. A key that legitimately
        // holds a colon inside (a namespaced key someone imported)
        // must survive, or normalising silently renames it.
        assert_eq!(normalize_key("ns:key"), "ns:key");
    }

    #[test]
    fn bookkeeping_keys_are_not_suggested() {
        // The catalogue feeds an "add a property" menu. `from-template`
        // is written by the template engine and `id` / `collapsed` come
        // in with imported graphs; proposing them invites the user to
        // hand-edit fields the app owns.
        let (mut ws, hlc) = workspace();
        block_with(
            &mut ws,
            &hlc,
            &[
                ("related", "x"),
                ("from-template", "journal"),
                ("id", "abc"),
                ("collapsed", "true"),
            ],
        );

        let keys: Vec<String> = known_keys(&ws).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["related".to_string()], "got {keys:?}");
    }

    #[test]
    fn a_key_with_spaces_is_refused_rather_than_repaired() {
        let why = key_rejection("date captured").expect("must be refused");
        assert!(why.contains("cannot contain spaces"), "{why}");
        // The message names the shape that works.
        assert!(why.contains("date-captured"), "{why}");
    }

    #[test]
    fn the_structural_guard_survives_the_colons_a_user_copies() {
        // `page-slug::` is not equal to `page-slug`, so a guard that
        // reads the raw text lets it through and the writer then
        // stores the real key, repointing every `[[ref]]` into the
        // page. Normalising first is what closes it.
        for typed in ["page-slug::", "  page-kind:: ", "page-slug:"] {
            let key = normalize_key(typed);
            assert!(
                key_rejection(&key).is_some(),
                "{typed:?} normalised to {key:?} and slipped past the guard"
            );
        }
    }

    #[test]
    fn an_empty_key_is_refused_after_normalising_too() {
        assert!(key_rejection(&normalize_key("::")).is_some());
        assert!(key_rejection(&normalize_key("   ")).is_some());
    }

    #[test]
    fn a_key_that_legitimately_ends_in_a_colon_stays_editable() {
        // `foo::: bar` parses as key `foo:` and round-trips. Since the
        // writer no longer normalises, deleting that chip targets the
        // key that is actually stored.
        let (mut ws, hlc) = workspace();
        let n = append_block(&mut ws, &hlc, None, Some("a block")).unwrap();
        set_property(
            &mut ws,
            &hlc,
            n,
            "foo:",
            Some(PropValue::Text("bar".into())),
        )
        .unwrap();
        assert_eq!(
            ws.tree()
                .properties_of(n)
                .map(|(k, _)| k)
                .collect::<Vec<_>>(),
            vec!["foo:"]
        );
        set_property(&mut ws, &hlc, n, "foo:", None).unwrap();
        assert_eq!(
            ws.tree().properties_of(n).count(),
            0,
            "delete must hit the stored key"
        );
    }
}
