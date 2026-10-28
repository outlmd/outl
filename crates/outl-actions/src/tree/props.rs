//! Which properties belong to the user, and how a value renders.
//!
//! The page model stores its own book-keeping (`page-slug`, `page-kind`,
//! `page-source`, `page-frontmatter`) as ordinary properties on the page
//! node, so every surface that shows properties to a user — the renderer,
//! the index, the clipboard, the property panel, the history — has to
//! agree on which ones to hide. That agreement is this module, and it
//! exists because those surfaces disagreed once: a `page-slug` rendered
//! into the `.md` is rewritten on the next reconcile, `page-source` is a
//! local absolute path that `outl export hugo` would copy into published
//! front matter, and `page-frontmatter` reached the property panel as an
//! editable chip whose deletion froze the page (issue #281).

use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::page::{KIND_KEY, SLUG_KEY};

/// Whether `key` is page-model book-keeping rather than a user
/// property.
///
/// `page-slug` / `page-kind` are written by [`crate::page`] through its
/// own ops; surfacing them in a rendered `.md` would rewrite the slug on
/// every reconcile, and surfacing them in the index would show the user
/// a property they never typed.
///
/// `page-source` ([`crate::open_with::SOURCE_KEY`]) joins them for a
/// second reason: its value is a local absolute path, so rendering it
/// would put the user's directory structure into a file that is often
/// in git and that `outl export hugo` copies wholesale into published
/// front matter.
///
/// [`outl_md::PAGE_FRONTMATTER_KEY`] joins them for a third: the value is
/// a page's verbatim YAML fence, riding `Op::SetProp` because page-level
/// state that must converge belongs in an op (issue #281). The `.md` does
/// carry it — but written by [`outl_md::render()`] as `---` delimiters, not
/// as a `key:: value` line — so **every reader of this predicate wants it
/// hidden, including the renderer**, which lifts it into
/// `ParsedPage::frontmatter` *before* filtering. Getting that order wrong
/// deletes the fence from every projection, which is what issue #281
/// reported; `the_render_lifts_the_fence_before_hiding_the_page_model_keys`
/// is the pin.
///
/// Leaving it out was worse than a naming slip. The property panel showed
/// the fence as an editable chip, the suggestion menu offered the key, and
/// deleting the chip emitted `SetProp(page-frontmatter, None)` — after
/// which the render carries no fence, the `.md` still does, and every
/// projection is refused as `PageMarkdownAheadOfLog`. The page stops
/// syncing in both directions until `outl reconcile --ahead-of-log` runs,
/// and that recovery re-reads the fence and puts the chip back.
///
/// **Consequence, accepted deliberately:** a frontmatter edit is also
/// absent from the page's history ([`crate::timeline`] filters by this
/// predicate). That matches `page-slug` / `page-kind`, and the fence is
/// not part of what the page *says* — outl never reads it. Admitting this
/// one key back in `timeline` would make that module a second owner of
/// "which properties are the user's", which is exactly the split that let
/// the leak exist. Pinned by
/// `a_frontmatter_edit_is_not_a_page_history_event`.
pub fn is_page_model_key(key: &str) -> bool {
    key == SLUG_KEY
        || key == KIND_KEY
        || key == crate::open_with::SOURCE_KEY
        || key == outl_md::PAGE_FRONTMATTER_KEY
}

/// A property value as the `.md` dialect renders it, or `None` when it
/// has no render syntax.
///
/// `Text`, `PageRef` and `Tag` all render as their string form. The
/// `.md` parser only ever yields `Text`, so a `PageRef` or `Tag` comes
/// back as `Text` with the same string. `List` has no syntax and is dropped.
///
/// The single owner of that mapping. It exists because three callers
/// need it — the page renderer, the block renderer, and the tree-derived
/// index — and a block's properties must read the same whether they
/// arrive through the renderer or through the index.
///
/// Note this is **not** the rule [`text_properties_of`] applies: that
/// one keeps `Text` only. The two have disagreed since before either was
/// documented as an owner; whether the clipboard should be dropping
/// `PageRef` / `Tag` is a separate question from this one.
pub(crate) fn renderable_prop_value(value: &PropValue) -> Option<String> {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => Some(s.clone()),
        PropValue::List(_) => None,
    }
}

/// Put a block's or page's properties in the order every surface shows
/// them: **alphabetical by key**.
///
/// The single owner of that order, and it needs one because the order
/// has no other home. Properties live in [`outl_core::tree::Tree`]'s
/// `HashMap<(NodeId, String), PropValue>`, whose iteration order is
/// seeded per process, so a projection that imposes no order renders
/// different bytes from the same op log on every boot. The `.md` is what
/// the user reads, diffs and commits (root `CLAUDE.md` invariant 2), so
/// "same content, different bytes" is a whole-workspace diff — measured
/// on a real 2,865-page workspace as **840 pages reported stale by
/// `outl doctor`**, 85 % of them differing in nothing but this order
/// ([issue #285](https://github.com/outlmd/outl/issues/285)).
///
/// **Alphabetical is a decision, not a fallback, because declaration
/// order is not recoverable.** `Op::SetProp` carries `(node, key,
/// value)` and no slot, so the sequence a user typed reaches the log
/// only as each op's HLC — which is the order the properties were last
/// *edited*, so a peer fixing one typo would reorder the page. Deriving
/// presentation from it means a new op field, and `undo_op` has to be
/// that field's exact inverse (`outl-core/CLAUDE.md` invariant 6) while
/// the prior order is not in the op. Paying a CRDT field for the order of
/// formatting lines is the trade this rejects; the user-facing half of
/// the decision is [`docs/markdown-format.md` → Property
/// order](../../../../docs/markdown-format.md#property-order).
///
/// Eight callers kept their own copy of this `sort_by` — one of them in
/// another crate — so "what order do properties render in" had eight
/// answers that happened to agree. `tests/property_order_owner.rs` is
/// the ratchet that stops a ninth.
pub fn sort_in_presentation_order(properties: &mut [(String, String)]) {
    properties.sort_by(|a, b| a.0.cmp(&b.0));
}

/// Textual properties of `node`, minus the page model's book-keeping
/// ([`is_page_model_key`]), in [`sort_in_presentation_order`].
///
/// Only `PropValue::Text` survives — `PageRef` / `Tag` / `List` shapes
/// have no `.md` render syntax and are dropped silently. The single
/// owner of the "which block properties round-trip through the `.md`
/// dialect" rule; `clipboard::build_node` and any other serializer share
/// it instead of re-deriving the filter + sort.
pub(crate) fn text_properties_of(workspace: &Workspace, node: NodeId) -> Vec<(String, String)> {
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .filter(|(k, _)| !is_page_model_key(k))
        .filter_map(|(k, v)| match v {
            PropValue::Text(s) => Some((k.to_string(), s.clone())),
            _ => None,
        })
        .collect();
    sort_in_presentation_order(&mut properties);
    properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    use crate::page::{open_or_create, set_property, PageKind};

    fn workspace() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).expect("in-memory workspace"),
            HlcGenerator::new(actor),
        )
    }

    /// The set every surface agrees to hide. Spelled out so adding a key to
    /// the predicate without deciding what it means here is a failing test.
    #[test]
    fn the_page_model_owns_four_keys_and_no_others() {
        for key in [
            SLUG_KEY,
            KIND_KEY,
            crate::open_with::SOURCE_KEY,
            outl_md::PAGE_FRONTMATTER_KEY,
        ] {
            assert!(is_page_model_key(key), "{key} must be hidden");
        }
        for key in ["title", "icon", "pinned", "page", "frontmatter"] {
            assert!(!is_page_model_key(key), "{key} is the user's");
        }
    }

    /// Declaration order is not what renders. The op log carries the
    /// order these were *written* in and the output does not follow it,
    /// which is the decision `sort_in_presentation_order` documents —
    /// the alternative reorders a page whenever a peer fixes one typo.
    #[test]
    fn properties_render_alphabetically_whatever_order_they_were_written_in() {
        let (mut ws, hlc) = workspace();
        let page =
            open_or_create(&mut ws, &hlc, "ord", "ord", PageKind::Page).expect("page is created");
        for key in ["zeta", "alpha", "mid"] {
            set_property(
                &mut ws,
                &hlc,
                page,
                key,
                Some(PropValue::Text(format!("v-{key}"))),
            )
            .expect("property is set");
        }

        let keys: Vec<String> = text_properties_of(&ws, page)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(keys, ["alpha", "mid", "zeta"]);
    }

    /// The whole reason the order is imposed rather than inherited: the
    /// tree's property map is a `HashMap` whose iteration order is
    /// seeded per process, so without this the same op log renders
    /// different bytes on every boot and every page reads as stale.
    #[test]
    fn the_order_does_not_depend_on_the_hash_maps_iteration() {
        let mut properties: Vec<(String, String)> = (0..64)
            .map(|i| (format!("k{i:02}"), String::new()))
            .collect();
        properties.reverse();

        let mut once = properties.clone();
        sort_in_presentation_order(&mut once);
        let mut again = properties;
        again.rotate_left(17);
        sort_in_presentation_order(&mut again);

        assert_eq!(once, again);
        assert!(once.windows(2).all(|w| w[0].0 < w[1].0));
    }

    /// Hiding the fence also hides it from the page's history, and that is
    /// the accepted consequence — see [`is_page_model_key`] for why the
    /// alternative (a second opinion inside `crate::timeline`) is worse.
    ///
    /// Lives here rather than in `timeline/tests.rs` because the decision
    /// belongs to the predicate; `timeline` only reads it.
    #[test]
    fn a_frontmatter_edit_is_not_a_page_history_event() {
        let (mut ws, hlc) = workspace();
        let page =
            open_or_create(&mut ws, &hlc, "fm", "fm", PageKind::Page).expect("page is created");
        for yaml in ["title: One", "title: Two"] {
            set_property(
                &mut ws,
                &hlc,
                page,
                outl_md::PAGE_FRONTMATTER_KEY,
                Some(PropValue::Text(yaml.into())),
            )
            .expect("the fence is written as a page property");
        }

        let timeline =
            crate::timeline::page_timeline(&ws, page, "fm", 50).expect("the page has a history");
        assert!(
            timeline
                .events
                .iter()
                .all(|e| !matches!(e.change, crate::timeline::Change::PropertySet { .. })),
            "a frontmatter edit surfaced as a property change: {:?}",
            timeline.events
        );
    }
}
