//! Person-typed pages — the `type:: person` page-level convention plus
//! the resolution helpers the `@` mention autocomplete depends on.
//!
//! Lives in a sibling module to `page` so the page-model surface stays
//! focused on the slug / kind / title primitives. Every consumer that
//! used to import from `outl_actions::page` continues to compile —
//! `lib.rs` re-exports the public symbols at the crate root, and
//! `page::open_or_create_by_ref` delegates the `@` arm here.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::page::{list_all, set_property, PageMeta};
use crate::resolve::resolve_or_create_by_name;

/// Page-level property key carrying the user-defined semantic type of
/// the page (`type:: person`, `type:: project`, …). Read by the `@`
/// mention autocomplete to filter to person pages only.
///
/// **Canonical for outl** is the bare `type` key — same shape Logseq
/// and similar tools use, so an imported workspace lights up without
/// rewriting.
pub const TYPE_KEY: &str = "type";

/// Canonical [`TYPE_KEY`] value marking a page as a person. The `@`
/// mention autocomplete in every client surfaces pages where
/// `page_type == PERSON_TYPE` (case-insensitive at the index level —
/// see [`outl_md::index::WorkspaceIndex::pages_by_type`]).
pub const PERSON_TYPE: &str = "person";

/// Pages with `type:: person`, ranked against `query`. Powers the `@`
/// mention autocomplete every client surfaces (TUI, desktop, mobile).
///
/// Ranking uses the same shape as the desktop's `search_pages` Tauri
/// ranking (exact → prefix → contains), against the title, the slug,
/// and every `alias::` name. An empty query returns the first 25 person
/// pages in
/// **slug** order (the order `list_all` produces — slugs sort
/// lexicographically). Returns at most 25 results.
///
/// The filter is `page_type == PERSON_TYPE` exactly — the index already
/// lowercased the property value, so we compare against the canonical
/// lowercase form. Other types (`project`, …) and untyped pages are
/// skipped.
pub fn search_persons(workspace: &Workspace, query: &str) -> Vec<PageMeta> {
    let q = query.trim().to_lowercase();
    let persons = list_all(workspace)
        .into_iter()
        .filter(|p| p.page_type.as_deref() == Some(PERSON_TYPE));
    if q.is_empty() {
        return persons.take(25).collect();
    }
    let mut scored: Vec<(u8, PageMeta)> = persons
        .filter_map(|p| {
            let title = p.title.to_lowercase();
            let slug = p.slug.to_lowercase();
            let names = std::iter::once(title)
                .chain(std::iter::once(slug))
                .chain(p.aliases.iter().map(|a| a.to_lowercase()));
            let score = names.fold(3u8, |best, n| {
                let s = if n == q {
                    0
                } else if n.starts_with(&q) {
                    1
                } else if n.contains(&q) {
                    2
                } else {
                    3
                };
                best.min(s)
            });
            if score == 3 {
                return None;
            }
            Some((score, p))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    scored.into_iter().map(|(_, p)| p).take(25).collect()
}

/// Resolve `name` to its person page, creating one if missing **and
/// idempotently marking the resulting node as `type:: person`**.
///
/// The "idempotent mark" is the load-bearing piece: when `name` resolves
/// to a page that already existed (created before the feature shipped,
/// authored by an external editor, or imported via fixtures), the user's
/// `[[@name]]` gesture must promote it to a person — otherwise the
/// `@` autocomplete `pages_by_type(PERSON_TYPE)` filter won't list it,
/// and the user sees the same empty popup forever despite the page
/// being right there on disk.
///
/// Re-emitting `Op::SetProp { key: "type", value: "person" }` when the
/// property is already set is a no-op via the CRDT's HLC-ordered LWW
/// (the new value matches the existing one), so calling this on every
/// `@` gesture is cheap.
pub(crate) fn ensure_person_by_name(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    name: &str,
) -> Result<NodeId, ActionError> {
    // Resolution is `page`'s shared ladder (literal slug → slugified →
    // case-insensitive title → create with `name` as the title). This
    // module only layers the person policy on top — the ladder used to
    // be duplicated here and drifted from `open_or_create_by_ref`'s.
    let id = resolve_or_create_by_name(workspace, hlc, name)?;
    set_property(
        workspace,
        hlc,
        id,
        TYPE_KEY,
        Some(PropValue::Text(PERSON_TYPE.to_string())),
    )?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{open_or_create, page_meta, set_property, PageKind};
    use crate::resolve::open_or_create_by_ref;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn page_meta_surfaces_page_type_lowercased() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        // Untyped page: `page_type == None`.
        assert_eq!(page_meta(&w, id).unwrap().page_type, None);
        // Setting `type:: Person` lands as Some("person") on the meta.
        set_property(
            &mut w,
            &hlc,
            id,
            TYPE_KEY,
            Some(PropValue::Text("Person".into())),
        )
        .unwrap();
        assert_eq!(
            page_meta(&w, id).unwrap().page_type.as_deref(),
            Some("person")
        );
    }

    #[test]
    fn search_persons_filters_to_person_pages_only() {
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let maria = open_or_create(&mut w, &hlc, "maria", "Maria", PageKind::Page).unwrap();
        let projeto = open_or_create(&mut w, &hlc, "projeto", "Projeto", PageKind::Page).unwrap();
        for id in [avelino, maria] {
            set_property(
                &mut w,
                &hlc,
                id,
                TYPE_KEY,
                Some(PropValue::Text(PERSON_TYPE.into())),
            )
            .unwrap();
        }
        set_property(
            &mut w,
            &hlc,
            projeto,
            TYPE_KEY,
            Some(PropValue::Text("project".into())),
        )
        .unwrap();

        let all_persons = search_persons(&w, "");
        let titles: Vec<&str> = all_persons.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles.len(), 2);
        assert!(titles.contains(&"Avelino"));
        assert!(titles.contains(&"Maria"));

        // Fuzzy query matches the title prefix.
        let hits = search_persons(&w, "av");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Avelino");

        // Query that doesn't match any person returns empty.
        assert!(search_persons(&w, "zzz").is_empty());
    }

    #[test]
    fn open_or_create_by_ref_strips_at_to_find_person() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        set_property(
            &mut w,
            &hlc,
            id,
            TYPE_KEY,
            Some(PropValue::Text(PERSON_TYPE.into())),
        )
        .unwrap();
        // `[[@avelino]]` resolves to the same `avelino` page even
        // though the slug does NOT carry the `@`.
        let resolved = open_or_create_by_ref(&mut w, &hlc, "@avelino").unwrap();
        assert_eq!(id, resolved);
    }

    #[test]
    fn open_or_create_by_ref_marks_preexisting_page_as_person() {
        // Regression for the slug-strip-`@` order-of-operations bug:
        // `slugify("@avelino")` returns `"avelino"`, so a generic
        // `find_by_slug(slugify(target))` branch would resolve before
        // the `@` arm ever ran — leaving the pre-existing page (no
        // `type:: person` set) silently un-marked. After accepting
        // `@avelino` from the autocomplete, the page MUST carry
        // `type:: person` so the next `@` mention surfaces it in the
        // popup. This is the bug that prompted the rewrite.
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        // Page deliberately starts WITHOUT `type:: person`.
        assert_eq!(page_meta(&w, id).unwrap().page_type, None);

        let resolved = open_or_create_by_ref(&mut w, &hlc, "@avelino").unwrap();
        assert_eq!(id, resolved, "must resolve to the same page");
        assert_eq!(
            page_meta(&w, id).unwrap().page_type.as_deref(),
            Some("person"),
            "resolving via `@` must idempotently mark the page as person"
        );
    }

    #[test]
    fn open_or_create_by_ref_creates_missing_person_with_type() {
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "@avelino").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "avelino");
        assert_eq!(meta.title, "avelino");
        assert_eq!(meta.page_type.as_deref(), Some("person"));
    }

    #[test]
    fn open_or_create_by_ref_composite_name_slugifies_title_preserved() {
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "@Thiago Avelino").unwrap();
        let meta = page_meta(&w, id).unwrap();
        // Slug folded; title kept verbatim.
        assert_eq!(meta.slug, "thiago-avelino");
        assert_eq!(meta.title, "Thiago Avelino");
        assert_eq!(meta.page_type.as_deref(), Some("person"));

        // Idempotent — same target resolves to the same node.
        let again = open_or_create_by_ref(&mut w, &hlc, "@Thiago Avelino").unwrap();
        assert_eq!(id, again);
    }

    /// Edge cases the `@` prefix should handle without panicking or
    /// creating bizarre slugs. The arm runs **before** the generic
    /// `find_by_slug` / `slugify` branches, so it owns the policy for
    /// everything that starts with `@`.
    #[test]
    fn open_or_create_by_ref_at_edge_cases() {
        // --- @Mixed case resolves to existing lowercase slug ---
        let (mut w, hlc) = ws();
        let pre = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        assert_eq!(page_meta(&w, pre).unwrap().page_type, None);
        let resolved = open_or_create_by_ref(&mut w, &hlc, "@Avelino").unwrap();
        assert_eq!(pre, resolved);
        assert_eq!(
            page_meta(&w, resolved).unwrap().page_type.as_deref(),
            Some("person")
        );

        // --- @ followed by space ---
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "@ avelino").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.page_type.as_deref(), Some("person"));
        assert!(!meta.slug.is_empty() && meta.slug != "untitled");

        // --- @ with a slash in the name ---
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "@avelino/outl").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "avelino-outl");
        assert_eq!(meta.title, "avelino/outl");
        assert_eq!(meta.page_type.as_deref(), Some("person"));

        // --- @@avelino (double @) ---
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "@@avelino").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.page_type.as_deref(), Some("person"));
        assert!(!meta.slug.is_empty() && meta.slug != "untitled");

        // --- Empty `@` ([[@]]) — falls through, must not panic ---
        let (mut w, hlc) = ws();
        let _ = open_or_create_by_ref(&mut w, &hlc, "@").unwrap();
    }

    /// Reproduction of the user-reported scenario: a page authored via
    /// `vim` (no `Op::Create`/`Op::Move` ever emitted for the page
    /// node), with `type:: person` set in the file. After
    /// `reconcile_md` runs, `search_persons` must find it.
    #[test]
    fn externally_authored_md_with_type_person_appears_in_search_persons() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let pages_dir = dir.path().join("pages");
        std::fs::create_dir_all(&pages_dir).unwrap();
        std::fs::write(
            pages_dir.join("samara.md"),
            "title:: Samara\ntype:: person\nrole:: PM\n\n- focused on FY26\n",
        )
        .unwrap();

        let actor = ActorId::new();
        let mut workspace = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);

        outl_md::reconcile::reconcile_md(&mut workspace, &hlc, &pages_dir.join("samara.md"), None)
            .expect("reconcile_md should succeed");

        let persons = search_persons(&workspace, "sam");
        assert_eq!(
            persons.len(),
            1,
            "search_persons(\"sam\") must surface samara; got {:?}",
            persons.iter().map(|p| &p.slug).collect::<Vec<_>>()
        );
        assert_eq!(persons[0].slug, "samara");
        assert_eq!(persons[0].page_type.as_deref(), Some("person"));
    }
}
