//! Resolution of user-typed page **names** and **ref targets** into
//! page nodes — the "user tapped `[[something]]`" decision tree.
//!
//! Split out of `page` so the page-model primitives (slug, kind,
//! title, journal) stay separate from the resolution *policy*. Both
//! the generic ref path here and the person module's `@` mention path
//! consume the same `resolve_or_create_by_name` ladder, so the two
//! cannot drift.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::dates::date_from_slug;
use crate::error::ActionError;
use crate::page::{find_by_slug, list_all, open_journal, open_or_create, PageKind};

/// Open (or create) a page identified by a user-typed **name** —
/// the kind of string that arrives from a `[[ref]]`, a `#tag`, or a
/// page-picker text field.
///
/// The name flows through [`outl_md::slug::slugify`] before reaching
/// [`open_or_create`], so anything filesystem-hostile (`/`, `\`,
/// spaces, accented letters, control chars) is normalised into a
/// safe single path component. The **original** `name` is kept as the
/// page's `title` so the user-facing rendering stays verbatim
/// (`[[avelino/outl]]` displays as `avelino/outl` even though the
/// disk slug is `avelino-outl`).
///
/// Use this whenever the caller has a human-typed string and wants
/// "open it or create it on the fly" semantics — the [`open_or_create`]
/// path requires a pre-validated slug and rejects raw user input. The
/// TUI's `Enter`-on-ref handler and the mobile's `open_page_by_slug`
/// command are both fed by this function so the two clients can't
/// drift on what counts as a valid ref target.
pub fn open_or_create_by_name(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    name: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
    let slug = outl_md::slug::slugify(name);
    open_or_create(workspace, hlc, &slug, name, kind)
}

/// Open (or create) whatever page a user-typed **ref target** points
/// at — `[[avelino/outl]]`, `[[2026-06-04]]`, `[[São Paulo]]`,
/// `[[Q4 plan]]`, a `#tag` body, a picker query. Routes through one
/// of:
///
/// 1. **Date-shaped target → journal**. [`date_from_slug`] is the
///    semantic validator (not the regex-shape one the mobile
///    frontend used to use), so `2026-13-01` and `2026-02-30` fall
///    through instead of erroring out.
/// 2. **`@`-prefixed target → person page** via the `person` module
///    (resolve + idempotently mark `type:: person`).
/// 3. **Everything else → `resolve_or_create_by_name`** — the
///    shared resolution ladder (literal slug → slugified → title →
///    create). Always succeeds for any ref a user could plausibly
///    type — the surface should never bubble `invalid …` back to a
///    tap.
///
/// One canonical decision tree, used by every client, so the
/// "Journal vs Page" discrimination cannot drift between frontend
/// regex and backend parser the way it did in the
/// `[[2026-13-01]] → invalid date slug` toast bug.
pub fn open_or_create_by_ref(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    target: &str,
) -> Result<NodeId, ActionError> {
    if let Some(date) = date_from_slug(target) {
        return open_journal(workspace, hlc, date);
    }
    // **Mention sugar — must run BEFORE the generic slug/title match.**
    //
    // `slugify("@avelino")` strips the `@`, returning `"avelino"`. If we
    // ran the generic `find_by_slug(slugify(target))` branch first, a
    // pre-existing `pages/avelino.md` (created before this feature, or
    // by an external editor without `type:: person`) would resolve via
    // the generic path and return early — never reaching the arm that
    // marks it as a person. The autocomplete popup would then never
    // surface that page on the next `@` keystroke, even though the
    // user just resolved a mention against it. Keep this arm first.
    if let Some(rest) = target.strip_prefix('@') {
        if !rest.is_empty() {
            return crate::person::ensure_person_by_name(workspace, hlc, rest);
        }
        // `[[@]]` — empty name. Fall through to the generic path,
        // which will create an `untitled` page via slugifier. Not
        // great UX but consistent with `[[]]`; refusing here would
        // require an extra error path the callers don't model.
    }
    resolve_or_create_by_name(workspace, hlc, target)
}

/// Resolve a human-typed page **name** against existing pages, falling
/// back to creating a fresh page when nothing matches. The shared
/// resolution ladder:
///
/// 1. **Literal slug match → existing page**. Covers picker-style
///    callers that already passed a clean slug.
/// 2. **Slugified match → existing page**. So `avelino/outl` finds
///    the page stored as `avelino-outl` even when the user typed the
///    ref before the page existed.
/// 3. **Case-insensitive title match → existing page**. So
///    `Thiago Avelino` matches a pre-existing page whose user-typed
///    title was `Thiago Avelino` even before the slug got computed.
/// 4. **Create via [`open_or_create_by_name`]** with the typed string
///    as the title (slugified internally, [`PageKind::Page`]).
///
/// Pure resolution policy — no date discrimination, no `@` handling,
/// no `type::` property writes. [`open_or_create_by_ref`] layers the
/// journal / mention arms on top, and `person::ensure_person_by_name`
/// layers the `type:: person` mark. Both consume **this** ladder so
/// the two resolution paths cannot drift.
pub(crate) fn resolve_or_create_by_name(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    name: &str,
) -> Result<NodeId, ActionError> {
    if let Some(id) = find_by_slug(workspace, name) {
        return Ok(id);
    }
    let normalised = outl_md::slug::slugify(name);
    if normalised != name {
        if let Some(id) = find_by_slug(workspace, &normalised) {
            return Ok(id);
        }
    }
    // The slugified lookup runs **before** the exact-title match, and
    // swapping them is a trap worth naming because it looks like the
    // more correct order: an exact title is more specific than a
    // derived slug, and a page whose title no longer derives its slug
    // (`outl_actions::open_with::slug_for`'s fallback) is reached by
    // the wrong page here.
    //
    // It is still the wrong trade. The title match below costs a
    // `list_all`, which is `page_meta` per page and therefore
    // `block_text` per page — the `O(blocks)` walk that materializes a
    // lazy-boot vault in full (the issue #179 freeze). Today it only
    // runs when the page is about to be created, a cold path. Moving
    // it up puts it on every `[[São Paulo]]` / `[[Q4 plan]]` click,
    // which is the hot one.
    //
    // The caller that has a title/slug mismatch links by slug instead
    // (see `open_with::import_into`). Fixing it here needs a cheap
    // title index first, not a reordering.
    let lower = name.to_lowercase();
    if let Some(existing) = list_all(workspace)
        .into_iter()
        .find(|p| p.title.to_lowercase() == lower)
    {
        use std::str::FromStr;
        if let Ok(id) = ulid::Ulid::from_str(&existing.id) {
            return Ok(NodeId(id));
        }
    }
    open_or_create_by_name(workspace, hlc, name, PageKind::Page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::page_meta;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn open_or_create_by_ref_routes_valid_date_to_journal() {
        // Use a fixed past date so the intent ("any valid YYYY-MM-DD
        // → journal") is obvious. Today's date would compile and pass
        // but suggests a system-clock coupling the function does not
        // actually have.
        let (mut w, hlc) = ws();
        let id = open_or_create_by_ref(&mut w, &hlc, "2020-01-01").unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.kind, PageKind::Journal);
        assert_eq!(meta.slug, "2020-01-01");
    }

    #[test]
    fn open_or_create_by_ref_falls_through_invalid_date_to_page() {
        // Regression: the mobile frontend used to gate this with the
        // shape regex `/^\d{4}-\d{2}-\d{2}$/`. `2026-13-01` (month 13)
        // and `2026-02-30` (day 30 in Feb) passed the shape, then
        // `parse_date` rejected them and the user saw an
        // `invalid date slug` toast. The shared helper has the only
        // discrimination path, so an invalid date becomes a regular
        // page just like any other ref.
        let (mut w, hlc) = ws();
        let bogus = "2026-13-01";
        let id = open_or_create_by_ref(&mut w, &hlc, bogus).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.kind, PageKind::Page);
        assert_eq!(meta.title, bogus);
        assert_eq!(meta.slug, "2026-13-01");
    }

    #[test]
    fn open_or_create_by_ref_resolves_existing_via_slugified_form() {
        let (mut w, hlc) = ws();
        // Create with the human name first.
        let created = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        // Subsequent tap of `[[avelino/outl]]` finds the same node
        // (not a fresh ULID under a different slug).
        let resolved = open_or_create_by_ref(&mut w, &hlc, "avelino/outl").unwrap();
        assert_eq!(created, resolved);
    }

    #[test]
    fn open_or_create_by_ref_matches_title_case_insensitively() {
        let (mut w, hlc) = ws();
        let created = open_or_create_by_name(&mut w, &hlc, "Ideas", PageKind::Page).unwrap();
        let resolved = open_or_create_by_ref(&mut w, &hlc, "ideas").unwrap();
        assert_eq!(created, resolved);
    }

    #[test]
    fn open_or_create_by_name_slugifies_filesystem_hostile_input() {
        // Regression: clicking `[[avelino/outl]]` on mobile used to
        // bubble the `/` straight into `is_valid_slug` and surface
        // `invalid page slug` as a toast. The helper must normalise
        // the disk slug while keeping the typed name as the title so
        // the ref renders verbatim everywhere.
        let (mut w, hlc) = ws();
        let id = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "avelino-outl");
        assert_eq!(meta.title, "avelino/outl");
        // Calling again with the same human-typed name must return the
        // same node (idempotent on the slugified form, not the raw input).
        let second = open_or_create_by_name(&mut w, &hlc, "avelino/outl", PageKind::Page).unwrap();
        assert_eq!(id, second);
    }

    // Person-typed resolution (`@` mentions, `type:: person` marking)
    // is tested in `crate::person::tests`.
}
