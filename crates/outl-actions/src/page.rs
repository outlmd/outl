//! Pages and their journal variant.
//!
//! A **page** is a direct child of [`NodeId::root`] whose
//! `page-slug` property identifies it. The page's text is its
//! user-facing title; the page's tree children are its blocks.
//!
//! Journals are pages with `page-kind = "journal"` and a
//! date-shaped slug (`YYYY-MM-DD`). The rest of the schema is
//! identical to regular pages.
//!
//! Why properties instead of a separate node kind: keeping pages as
//! ordinary nodes means the tree CRDT still owns all the move /
//! delete / re-parent semantics for free, and the wire format stays
//! one Op log. Pages are just nodes with a marker property.

use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::block::create_with_explicit_id;
use crate::dates::{journal_slug, journal_title};
use crate::error::ActionError;
use crate::tree::{children_of, position_for_new_last_child};

/// Property key marking a node as a page root and recording its slug.
pub const SLUG_KEY: &str = "page-slug";
/// Property key recording whether a page is a regular page or a
/// journal.
pub const KIND_KEY: &str = "page-kind";
/// Property key holding a page's human-readable title.
///
/// The title lives in a **property** (`Op::SetProp`, last-write-wins by
/// HLC) rather than the root node's Yrs text on purpose: with
/// deterministic root ids two devices that create the same slug offline
/// mint the same node, so writing the title into the root's text made two
/// concurrent Yrs inserts concatenate (`"2026-06-252026-06-25"`). A
/// property converges to a single value instead. See [`open_or_create`]
/// and [`crate::page_repair_titles`].
pub const TITLE_KEY: &str = "title";
// `TYPE_KEY` / `PERSON_TYPE` / `search_persons` / `ensure_person_by_name`
// live in the sibling `person` module so this file stays focused on the
// page-model primitives (slug, kind, title, journal). The constants and
// `search_persons` are re-exported at the crate root via `lib.rs` for
// back-compat with callers that imported them from `outl_actions::page`.

/// Whether a page is a regular named page or a date-keyed journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    /// Regular named page.
    Page,
    /// Date-keyed journal page.
    Journal,
}

impl PageKind {
    /// Wire value used in the stored property.
    pub fn as_str(self) -> &'static str {
        match self {
            PageKind::Page => "page",
            PageKind::Journal => "journal",
        }
    }

    /// Parse from the stored property value, defaulting to
    /// [`PageKind::Page`] when the value is missing or unknown.
    pub fn parse(value: Option<&PropValue>) -> Self {
        match value {
            Some(PropValue::Text(s)) if s == "journal" => PageKind::Journal,
            _ => PageKind::Page,
        }
    }
}

/// UI-friendly summary of a page.
#[derive(Debug, Clone, Serialize)]
pub struct PageMeta {
    /// Stringified [`NodeId`] of the page root.
    pub id: String,
    /// Stable slug identifying the page (filename-safe).
    pub slug: String,
    /// Human-readable title. Resolved most-specific-first: the
    /// `title::` page property (where ingest / reconcile park the name),
    /// then the page node's own text (set by in-app creation), then the
    /// slug as a last resort. See `page_meta`.
    pub title: String,
    /// `page` or `journal`.
    pub kind: PageKind,
    /// Optional emoji / icon string the user set on the page (via the
    /// `icon::` page property). `None` when unset — clients pick their
    /// own fallback (mobile uses the page kind to decide between 📄
    /// and 📅; TUI uses `📄` for everything by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `pinned:: true` page-level property. Surfaces that ship a
    /// sidebar (TUI, desktop) list pinned pages prominently so the
    /// user can pin their canonical workspace entry points (people,
    /// "inbox", "weekly review", …). Default `false` and serialised
    /// only when `true` so the wire stays small.
    ///
    /// Mirrors the `WorkspaceIndex::PageEntry.pinned` field already
    /// owned by `outl-md` — but lifted onto `PageMeta` so every
    /// client that lists pages (`list_all_pages`) sees the flag
    /// without needing to also consult the index.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// `type::` page-level property (lowercased+trimmed). `None` when
    /// unset. Drives the `@` mention autocomplete: clients filter the
    /// list to `Some("person")` candidates without re-reading the
    /// workspace index. Mirrors `WorkspaceIndex::PageEntry.page_type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_type: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Look up a page by slug. Returns the page root [`NodeId`] when found.
///
/// When the split-brain bug (see [`merge_duplicate_slug_roots`]) has
/// left **more than one** root carrying the same slug, this picks a
/// deterministic winner so every device — and every read before the
/// merge repair runs — resolves to the *same* root and the UI stops
/// flickering: the root whose id equals [`page_id_from_slug`] if one is
/// present, otherwise the numerically smallest [`NodeId`]. The
/// single-match fast path is unchanged.
pub fn find_by_slug(workspace: &Workspace, slug: &str) -> Option<NodeId> {
    let candidates = children_of(workspace, NodeId::root())
        .into_iter()
        .filter(|(id, _)| {
            matches!(workspace.tree().property(*id, SLUG_KEY), Some(PropValue::Text(s)) if s == slug)
        })
        .map(|(id, _)| id);
    canonical_root_for_slug(slug, candidates)
}

/// Pick the one root that represents `slug` out of every candidate
/// carrying it.
///
/// The single owner of the split-brain tie-break: the root whose id
/// equals [`page_id_from_slug`] when present, otherwise the numerically
/// smallest [`NodeId`]. Deterministic, so every device and every reader
/// resolves to the same root while [`merge_duplicate_slug_roots`] has
/// not run yet.
///
/// Takes the candidates rather than finding them, so a caller already
/// walking the root's children (the workspace-index derivation) applies
/// the same rule without paying a scan per slug — and, more to the
/// point, without writing a second copy of the rule that could drift
/// from this one.
pub fn canonical_root_for_slug(
    slug: &str,
    candidates: impl IntoIterator<Item = NodeId>,
) -> Option<NodeId> {
    let canonical = page_id_from_slug(slug);
    let mut best: Option<NodeId> = None;
    for id in candidates {
        if id == canonical {
            return Some(id);
        }
        best = Some(match best {
            Some(current) if current <= id => current,
            _ => id,
        });
    }
    best
}

/// List every page in the workspace, sorted by slug.
pub fn list_all(workspace: &Workspace) -> Vec<PageMeta> {
    let mut pages: Vec<PageMeta> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| page_meta(workspace, id))
        .collect();
    pages.sort_by(|a, b| a.slug.cmp(&b.slug));
    pages
}

/// Read page metadata from a node. Returns `None` when the node is
/// not a page (no `page-slug` property).
pub fn page_meta(workspace: &Workspace, id: NodeId) -> Option<PageMeta> {
    let slug = match workspace.tree().property(id, SLUG_KEY)? {
        PropValue::Text(s) => s.clone(),
        _ => return None,
    };
    let kind = PageKind::parse(workspace.tree().property(id, KIND_KEY));
    // Title resolution, most-specific first:
    //   1. the `title::` page property — where ingest / reconcile parks
    //      the human-readable name (`diff_to_ops_with_page_props` emits
    //      it as `Op::SetProp` key `"title"`, and the page root's text
    //      stays empty for disk-sourced pages);
    //   2. the page node's own text — set when a page is created
    //      in-app via `open_or_create_by_name` (the typed name);
    //   3. the slug — last resort so a title is never empty.
    // Reading `block_text` alone (the old behaviour) made every
    // ingested page surface its slug in pickers / autocomplete.
    let title = match workspace.tree().property(id, TITLE_KEY) {
        Some(PropValue::Text(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => workspace
            .block_text(id)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| slug.clone()),
    };
    // `icon::` is a free-form page property. It survives as a
    // `PropValue::Text` after `reconcile_md` applies the page's
    // properties; we surface only the textual variant so the wire
    // format stays a plain string. Other PropValue shapes (PageRef /
    // Tag / List) are not legal for an icon and would silently
    // mismatch the renderer, so we treat them as absent.
    let icon = match workspace.tree().property(id, "icon") {
        Some(PropValue::Text(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    // `pinned::` is a page-level boolean property. We accept the
    // **exact** set of truthy literals `outl-md::index::is_truthy`
    // does (`true`, `yes`, `1`, `on`) so a hand-edited `.md`
    // matches what the workspace index would also pick up. Don't
    // add new tokens here without updating outl-md in the same
    // commit — drift breaks the "every list of pages agrees on
    // pinned" invariant.
    let pinned = match workspace.tree().property(id, "pinned") {
        Some(PropValue::Text(s)) => is_truthy(s),
        _ => false,
    };
    // `type::` is a free-form page-level property (`person`, `project`,
    // …). We surface it lowercased + trimmed so callers can compare
    // against `PERSON_TYPE` without re-normalising. Same shape as the
    // workspace index — see `outl_md::index::PageEntry.page_type`.
    let page_type = match workspace.tree().property(id, crate::person::TYPE_KEY) {
        Some(PropValue::Text(s)) => {
            let normalised = s.trim().to_lowercase();
            if normalised.is_empty() {
                None
            } else {
                Some(normalised)
            }
        }
        _ => None,
    };
    Some(PageMeta {
        id: id.to_string(),
        slug,
        title,
        kind,
        icon,
        pinned,
        page_type,
    })
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
}

/// Deterministic [`NodeId`] derived from a page slug.
///
/// Two devices independently creating the same slug — even offline,
/// before iCloud / Syncthing / shared FS has had a chance to
/// reconcile — end up with the **same** [`NodeId`]. Without this, each
/// peer would mint a fresh random ULID and we'd have two divergent
/// page subtrees with no way to merge them (the CRDT only merges
/// concurrent edits to the *same* node).
///
/// We use `sha256("outl-page:" + slug)[..16]` as the ULID's 128-bit
/// payload. The constant prefix avoids accidental collisions with any
/// other content-derived id scheme that might enter the workspace
/// later. Output is deterministic and stable across releases.
///
/// The derivation itself lives in `outl_core::id::NodeId::from_slug` so
/// the three page-root creation paths — in-app `open_or_create`,
/// `outl_md::reconcile` of an external `.md`, and `desync` recovery —
/// share one owner and cannot drift into minting different ids for the
/// same slug (the split-brain that had two roots claim `2026-07-10`).
pub fn page_id_from_slug(slug: &str) -> NodeId {
    NodeId::from_slug(slug)
}

/// Whether `slug` is safe to use as a single path component for the
/// `.md` / `.outl` projection.
///
/// The slug is joined into `pages/<slug>.md` (or `journals/...`) and
/// shows up in `[[refs]]`, in `block-ref` handles, and in the export
/// pipelines. Anything that would escape its directory (`..`, `/`,
/// `\`) or smuggle control characters (`\0`, newline) is rejected
/// here so a single check covers every downstream surface. Leading /
/// trailing whitespace is also rejected because it silently breaks
/// filename equality across iCloud / git / external editors.
pub fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 255 {
        return false;
    }
    if slug != slug.trim() {
        return false;
    }
    if slug == "." || slug == ".." {
        return false;
    }
    for ch in slug.chars() {
        match ch {
            '/' | '\\' | '\0' | '\n' | '\r' | '\t' => return false,
            c if c.is_control() => return false,
            _ => {}
        }
    }
    // No `..` segment hidden in something like `foo/../bar` even though
    // we already reject `/`. Belt and suspenders for any caller that
    // routes the slug through a path join.
    !slug.split(['/', '\\']).any(|c| c == "..")
}

/// Open the page for `slug`, creating it if it doesn't exist yet.
///
/// The page's [`NodeId`] is derived deterministically from the slug
/// (see [`page_id_from_slug`]), so a peer that locally creates the
/// same slug ends up writing to the same node. iCloud sync then
/// merges the two creators' edits into one page instead of leaving
/// two competing copies.
///
/// Rejects slugs that fail [`is_valid_slug`] — the slug ends up
/// joined into a filesystem path, so anything that could escape its
/// directory (`..`, `/`, `\`, control chars) stays out of the
/// workspace entirely.
pub fn open_or_create(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
    title: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
    if !is_valid_slug(slug) {
        return Err(ActionError::InvalidSlug(slug.to_string()));
    }
    if let Some(id) = find_by_slug(workspace, slug) {
        return Ok(id);
    }
    let node_id = page_id_from_slug(slug);
    let position = position_for_new_last_child(workspace, NodeId::root());
    // Create the root with NO text. The title used to be written into the
    // root's Yrs text, but with deterministic ids two devices that create
    // the same slug offline mint the same node: the `Op::Create` converges
    // to one root, yet each device's `edit_text(root, title)` is a
    // concurrent Yrs insert at position 0, and Yrs concatenates them
    // (`"2026-06-252026-06-25"`). The title now lives in the `title::`
    // property below (`Op::SetProp`, last-write-wins by HLC), which
    // converges to a single value under the same concurrency.
    // Creating a page is one user-visible action (root + slug/kind/title
    // props). Batch it so those consecutive `apply`s flush once per
    // destination instead of fsyncing per op.
    let mut batch = workspace.begin_batch();
    let node = create_with_explicit_id(&mut batch, hlc, node_id, NodeId::root(), position, None)?;
    set_prop(
        &mut batch,
        hlc,
        node,
        SLUG_KEY,
        PropValue::Text(slug.to_string()),
    )?;
    set_prop(
        &mut batch,
        hlc,
        node,
        KIND_KEY,
        PropValue::Text(kind.as_str().to_string()),
    )?;
    // Store the title only when it carries information beyond the slug.
    // Journals (`journal_title == slug`) fall through to `page_meta`'s slug
    // fallback, so their `.md` never grows a redundant `title:: 2026-06-25`
    // line.
    if title != slug {
        set_prop(
            &mut batch,
            hlc,
            node,
            TITLE_KEY,
            PropValue::Text(title.to_string()),
        )?;
    }
    batch.commit()?;
    Ok(node)
}

/// Delete a page by moving its root node to the trash.
///
/// The page's whole subtree (its blocks) travels with it in a single
/// `Op::Move` to [`NodeId::trash()`] (via [`crate::block::delete`]),
/// so the page disappears from listings in one op. The op stays in the
/// log forever — that's what lets deletion converge across devices and
/// keeps future reordering correct (see `docs/crdt.md`).
///
/// This function is the **CRDT half** of "delete a page". It does not
/// touch the on-disk `.md` + `.outl` projection: filesystem cleanup
/// stays in the caller (CLI / Tauri command / TUI each own a different
/// workspace root path). After this returns, callers should drop the
/// projection via [`crate::journal::remove_page_projection`] so a peer
/// that hasn't received the op yet doesn't keep reading a stale page.
///
/// Returns the deleted page's [`PageMeta`] so the caller can derive
/// the projection path and navigate the user away.
pub fn delete(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
) -> Result<PageMeta, ActionError> {
    let id =
        find_by_slug(workspace, slug).ok_or_else(|| ActionError::PageNotFound(slug.to_string()))?;
    let meta = page_meta(workspace, id).ok_or_else(|| ActionError::NotInTree(id.to_string()))?;
    crate::block::delete(workspace, hlc, id)?;
    Ok(meta)
}

// `merge_duplicate_slug_roots` (split-brain repair for duplicate slug
// roots) lives in the sibling `page_merge` module and is re-exported at
// the crate root via `lib.rs`. The `#[cfg(test)]` battery for it stays
// below, next to the rest of the page-model tests.
pub use crate::page_merge::merge_duplicate_slug_roots;

// resolution ladder live in the sibling `resolve` module; `person`
// owns the `type:: person` policy. Both are re-exported at the crate
// root via `lib.rs`.

fn set_prop(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    key: &str,
    value: PropValue,
) -> Result<(), ActionError> {
    set_property(workspace, hlc, node, key, Some(value))
}

/// Set (or clear, with `value = None`) a property on `node`.
///
/// One-liner over `Op::SetProp` exposed so every client can write
/// properties without reaching into `outl-core::Op` directly. The
/// `set_prop` private alias above keeps backward-compat with the
/// internal call sites in this module.
pub fn set_property(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    key: &str,
    value: Option<PropValue>,
) -> Result<(), ActionError> {
    let ts = hlc.next();
    workspace.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::SetProp {
            node,
            key: key.to_string(),
            value,
            old_value: None,
        },
    })?;
    Ok(())
}

/// Read a property as a plain `String`. Returns `None` when the
/// property is unset or is a structured value the caller would have
/// to unwrap (`List`); in those cases, read `workspace.tree().property`
/// directly.
///
/// Convenience for the export / display surfaces that just want the
/// raw text of `icon::`, `title::`, single-tag `tags::`, etc.
pub fn read_text_prop(workspace: &Workspace, node: NodeId, key: &str) -> Option<String> {
    match workspace.tree().property(node, key)? {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => Some(s.clone()),
        PropValue::List(items) => {
            let joined = items
                .iter()
                .filter_map(|v| match v {
                    PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => {
                        Some(s.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Journal helpers
// ---------------------------------------------------------------------------
// The pure date labels (`journal_slug`, `journal_title`, `journal_ref`,
// `date_from_slug`, `previous_journal_date`, `next_journal_date`) live
// in `crate::dates` — this module keeps only the workspace-touching
// journal operations.

/// Today's date in the user's configured timezone (falling back to the
/// OS local timezone). Delegates to [`crate::clock`] so the journal's
/// "today" honours `[calendar] timezone` — see issue #107.
pub fn today() -> NaiveDate {
    crate::clock::today()
}

/// Open today's journal page, creating it if needed.
pub fn open_today(workspace: &mut Workspace, hlc: &HlcGenerator) -> Result<NodeId, ActionError> {
    open_journal(workspace, hlc, today())
}

/// Open the journal page for `date`, creating it if needed.
///
/// When the daily note is created fresh **and** the workspace defines a
/// `journal` template (a page with `template:: journal`), its outline is
/// stamped into the new note automatically — the property-based
/// successor to the old `templates/journal.md` seed. The stamp is
/// best-effort (a broken template never blocks opening today) and
/// untraced: every daily comes from the same template, so a
/// `from-template::` marker would be pure noise on every note.
pub fn open_journal(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    date: NaiveDate,
) -> Result<NodeId, ActionError> {
    let slug = journal_slug(date);
    // An existing journal is returned as-is — never re-stamp it.
    if let Some(id) = find_by_slug(workspace, &slug) {
        return Ok(id);
    }
    let id = open_or_create(
        workspace,
        hlc,
        &slug,
        &journal_title(date),
        PageKind::Journal,
    )?;
    if crate::template::list::find_template_by_name(
        workspace,
        crate::template::JOURNAL_TEMPLATE_NAME,
    )
    .is_some()
    {
        let _ = crate::template::instantiate::instantiate_template_traced(
            workspace,
            hlc,
            crate::template::JOURNAL_TEMPLATE_NAME,
            id,
            &slug,
            Some(date),
            false,
        );
    }
    Ok(id)
}

/// Sweep any legacy children of [`NodeId::root`] that aren't pages
/// (have no `page-slug` property) under today's journal. Used once
/// during the migration from pre-page-model workspaces.
pub fn migrate_legacy_into_today(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<usize, ActionError> {
    let today_id = open_today(workspace, hlc)?;
    let stragglers: Vec<NodeId> = children_of(workspace, NodeId::root())
        .into_iter()
        .filter_map(|(id, _)| {
            if id == today_id {
                return None;
            }
            if workspace.tree().property(id, SLUG_KEY).is_some() {
                return None;
            }
            Some(id)
        })
        .collect();

    let mut moved = 0usize;
    for node in stragglers {
        crate::tree::position_for_new_last_child(workspace, today_id);
        let pos = crate::tree::position_for_new_last_child(workspace, today_id);
        let old_parent = workspace
            .tree()
            .parent(node)
            .ok_or_else(|| ActionError::NotInTree(node.to_string()))?;
        let old_position = workspace
            .tree()
            .position(node)
            .cloned()
            .ok_or_else(|| ActionError::MissingPosition(node.to_string()))?;
        let ts = hlc.next();
        workspace.apply(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Move {
                node,
                new_parent: today_id,
                position: pos,
                old_parent,
                old_position,
            },
        })?;
        moved += 1;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn create_and_find_by_slug() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
        assert_eq!(find_by_slug(&w, "ideas"), Some(id));
        let pages = list_all(&w);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].slug, "ideas");
        assert_eq!(pages[0].title, "Ideas");
        assert_eq!(pages[0].kind, PageKind::Page);
    }

    /// Define a `journal` template page with one body block.
    fn make_journal_template(w: &mut Workspace, hlc: &HlcGenerator) {
        let tpl = open_or_create(w, hlc, "templates-journal", "journal", PageKind::Page).unwrap();
        set_property(
            w,
            hlc,
            tpl,
            crate::template::TEMPLATE_KEY,
            Some(PropValue::Text(
                crate::template::JOURNAL_TEMPLATE_NAME.into(),
            )),
        )
        .unwrap();
        append_block(w, hlc, Some(tpl), Some("**Standup:**")).unwrap();
    }

    #[test]
    fn open_journal_stamps_template_on_fresh_daily() {
        let (mut w, hlc) = ws();
        make_journal_template(&mut w, &hlc);
        let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let j = open_journal(&mut w, &hlc, date).unwrap();
        let bodies: Vec<String> = children_of(&w, j)
            .into_iter()
            .filter_map(|(id, _)| w.block_text(id))
            .collect();
        assert!(
            bodies.iter().any(|t| t == "**Standup:**"),
            "fresh daily should carry the journal template body"
        );
    }

    #[test]
    fn open_journal_does_not_restamp_existing_daily() {
        let (mut w, hlc) = ws();
        make_journal_template(&mut w, &hlc);
        let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let first = open_journal(&mut w, &hlc, date).unwrap();
        let count_after_first = children_of(&w, first).len();
        // Re-opening the same daily must return it as-is, never re-stamp.
        let second = open_journal(&mut w, &hlc, date).unwrap();
        assert_eq!(first, second);
        assert_eq!(children_of(&w, second).len(), count_after_first);
    }

    #[test]
    fn open_journal_without_template_is_empty() {
        let (mut w, hlc) = ws();
        let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let j = open_journal(&mut w, &hlc, date).unwrap();
        assert!(
            children_of(&w, j).is_empty(),
            "no journal template → empty daily"
        );
    }

    // Resolution of user-typed names / ref targets (`open_or_create_by_name`,
    // `open_or_create_by_ref`, the shared ladder) is tested in
    // `crate::resolve::tests`; the person-typed arm in `crate::person::tests`.

    #[test]
    fn page_meta_prefers_title_property_over_node_text() {
        // Regression (issue #88): disk-sourced / ingested pages park the
        // human-readable name in the `title::` property
        // (`diff_to_ops_with_page_props` emits it as `Op::SetProp` key
        // "title") while the page root node's text stays empty. The
        // mobile / desktop autocomplete then rendered the slug because
        // `page_meta` read `block_text` only. `title::` must win.
        let (mut w, hlc) = ws();
        // Create with an empty node text, mimicking an ingested page.
        let id = open_or_create(&mut w, &hlc, "avelino-outl", "", PageKind::Page).unwrap();
        set_prop(
            &mut w,
            &hlc,
            id,
            "title",
            PropValue::Text("avelino/outl".to_string()),
        )
        .unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.title, "avelino/outl");
        assert_eq!(meta.slug, "avelino-outl");
    }

    #[test]
    fn page_meta_falls_back_to_slug_when_no_title_or_text() {
        // No `title::` property and empty node text → slug is the last
        // resort so a title is never blank.
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "orphan", "", PageKind::Page).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.title, "orphan");
    }

    #[test]
    fn open_or_create_is_idempotent() {
        let (mut w, hlc) = ws();
        let a = open_or_create(&mut w, &hlc, "foo", "Foo", PageKind::Page).unwrap();
        let b = open_or_create(&mut w, &hlc, "foo", "Foo Renamed", PageKind::Page).unwrap();
        assert_eq!(a, b);
        // Title is NOT updated by a second open_or_create; the caller
        // is expected to use edit_text if they want to rename.
        let pages = list_all(&w);
        assert_eq!(pages[0].title, "Foo");
    }

    #[test]
    fn journal_round_trip() {
        let (mut w, hlc) = ws();
        let date = NaiveDate::from_ymd_opt(2026, 5, 27).unwrap();
        let id = open_journal(&mut w, &hlc, date).unwrap();
        let meta = page_meta(&w, id).unwrap();
        assert_eq!(meta.slug, "2026-05-27");
        assert_eq!(meta.kind, PageKind::Journal);
        assert!(meta.title.contains("2026"));
    }

    #[test]
    fn migration_moves_legacy_blocks() {
        let (mut w, hlc) = ws();
        let a = append_block(&mut w, &hlc, None, Some("legacy 1")).unwrap();
        let b = append_block(&mut w, &hlc, None, Some("legacy 2")).unwrap();
        let moved = migrate_legacy_into_today(&mut w, &hlc).unwrap();
        assert_eq!(moved, 2);
        let today_id = open_today(&mut w, &hlc).unwrap();
        assert_eq!(w.tree().parent(a), Some(today_id));
        assert_eq!(w.tree().parent(b), Some(today_id));
    }

    #[test]
    fn delete_removes_page_from_listings() {
        // `page::delete` moves the page root to `NodeId::trash()`, so
        // `find_by_slug` / `list_all` stop surfacing it. The op stays
        // in the log; only the materialised tree drops the node.
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "doomed", "Doomed", PageKind::Page).unwrap();
        // Sanity: exists before delete.
        assert_eq!(find_by_slug(&w, "doomed"), Some(id));
        assert_eq!(list_all(&w).len(), 1);

        let meta = delete(&mut w, &hlc, "doomed").unwrap();
        assert_eq!(meta.slug, "doomed");

        // After delete: gone from slug lookup and from the listing.
        assert_eq!(find_by_slug(&w, "doomed"), None);
        assert!(list_all(&w).is_empty());
    }

    #[test]
    fn delete_unknown_slug_is_page_not_found() {
        // Deleting a slug that doesn't resolve surfaces
        // `ActionError::PageNotFound` — the caller decides how to
        // present it (toast / status line / silent no-op).
        let (mut w, hlc) = ws();
        let err = delete(&mut w, &hlc, "never-existed").unwrap_err();
        assert!(matches!(err, ActionError::PageNotFound(s) if s == "never-existed"));
    }

    /// Mint a SECOND root for `slug` with a fresh (non-deterministic)
    /// id — the split-brain shape the merge repairs. Returns its id.
    fn spawn_duplicate_root(
        w: &mut Workspace,
        hlc: &HlcGenerator,
        slug: &str,
        kind: PageKind,
    ) -> NodeId {
        let dup_id = NodeId(ulid::Ulid::new());
        let pos = position_for_new_last_child(w, NodeId::root());
        create_with_explicit_id(w, hlc, dup_id, NodeId::root(), pos, Some(slug)).unwrap();
        set_prop(w, hlc, dup_id, SLUG_KEY, PropValue::Text(slug.to_string())).unwrap();
        set_prop(
            w,
            hlc,
            dup_id,
            KIND_KEY,
            PropValue::Text(kind.as_str().to_string()),
        )
        .unwrap();
        dup_id
    }

    #[test]
    fn merge_duplicate_slug_roots_consolidates_two_journal_roots() {
        let (mut w, hlc) = ws();
        let slug = "2026-07-10";
        // Canonical root via the normal path (deterministic id) + one child.
        let canonical =
            open_or_create(&mut w, &hlc, slug, "2026-07-10", PageKind::Journal).unwrap();
        assert_eq!(canonical, page_id_from_slug(slug));
        let c0 = append_block(&mut w, &hlc, Some(canonical), Some("canonical child")).unwrap();

        // Second root, same slug, fresh id, its own two children.
        let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Journal);
        let d0 = append_block(&mut w, &hlc, Some(dup), Some("dup child 1")).unwrap();
        let d1 = append_block(&mut w, &hlc, Some(dup), Some("dup child 2")).unwrap();

        let merged = merge_duplicate_slug_roots(&mut w, &hlc).unwrap();
        assert_eq!(merged, 1, "exactly one duplicate root merged");

        // Only the canonical root remains for the slug.
        let remaining: Vec<NodeId> = children_of(&w, NodeId::root())
            .into_iter()
            .filter(|(id, _)| {
                matches!(
                    w.tree().property(*id, SLUG_KEY),
                    Some(PropValue::Text(s)) if s == slug
                )
            })
            .map(|(id, _)| id)
            .collect();
        assert_eq!(remaining, vec![canonical]);

        // Canonical holds ALL children from both roots, dup's order preserved
        // and appended after the canonical's own child.
        let kids: Vec<NodeId> = children_of(&w, canonical)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(kids, vec![c0, d0, d1], "no child lost, order preserved");

        // The emptied duplicate is trashed.
        assert_eq!(w.tree().parent(dup), Some(NodeId::trash()));
        // find_by_slug now resolves to the single survivor.
        assert_eq!(find_by_slug(&w, slug), Some(canonical));
    }

    #[test]
    fn merge_duplicate_slug_roots_is_idempotent_and_noop_when_clean() {
        let (mut w, hlc) = ws();
        // Clean workspace: nothing to merge.
        assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 0);

        let slug = "ideas";
        open_or_create(&mut w, &hlc, slug, "Ideas", PageKind::Page).unwrap();
        let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Page);
        append_block(&mut w, &hlc, Some(dup), Some("stray")).unwrap();

        // First run merges the duplicate.
        assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 1);
        // Second run is a no-op: the workspace is clean again.
        assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 0);
    }

    /// Mint the canonical root for `slug` directly (deterministic id),
    /// bypassing `open_or_create`'s `find_by_slug` short-circuit — which
    /// would otherwise return an already-present duplicate.
    fn spawn_canonical_root(
        w: &mut Workspace,
        hlc: &HlcGenerator,
        slug: &str,
        kind: PageKind,
    ) -> NodeId {
        let id = page_id_from_slug(slug);
        let pos = position_for_new_last_child(w, NodeId::root());
        create_with_explicit_id(w, hlc, id, NodeId::root(), pos, Some(slug)).unwrap();
        set_prop(w, hlc, id, SLUG_KEY, PropValue::Text(slug.to_string())).unwrap();
        set_prop(
            w,
            hlc,
            id,
            KIND_KEY,
            PropValue::Text(kind.as_str().to_string()),
        )
        .unwrap();
        id
    }

    #[test]
    fn find_by_slug_is_deterministic_with_duplicate_roots() {
        let (mut w, hlc) = ws();
        let slug = "2026-07-10";
        // Duplicate root FIRST (would win a naive "first match"), then the
        // canonical deterministic-id root minted directly (bypassing the
        // `open_or_create` short-circuit that would return the dup).
        let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Journal);
        let canonical = spawn_canonical_root(&mut w, &hlc, slug, PageKind::Journal);
        assert_eq!(canonical, page_id_from_slug(slug));
        assert_ne!(dup, canonical);

        // Deterministic: always the canonical id, stable across repeated calls.
        assert_eq!(find_by_slug(&w, slug), Some(canonical));
        assert_eq!(find_by_slug(&w, slug), Some(canonical));
    }

    // Person-typed page tests (`type:: person`, `@` mention resolution,
    // edge cases) live in `crate::person::tests`. Keep this module
    // focused on the page-model primitives (slug, kind, title, journal).
}
