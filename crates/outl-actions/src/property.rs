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
}
