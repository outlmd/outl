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
/// `alias::` page-level property — comma-separated alternative names matched beside the title (issue #302).
pub const ALIAS_KEY: &str = "alias";
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
    /// `alias::` — alternative names; serialised only when non-empty. Mirrors `outl_md::index::PageEntry.aliases`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
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
        .filter(|(id, _)| claims_slug(workspace, *id, slug))
        .map(|(id, _)| id);
    canonical_root_for_slug(slug, candidates)
}

/// Whether `id` carries `slug` in its `page-slug` property.
///
/// One predicate for both ways the question gets asked: [`find_by_slug`]
/// scanning `children_of(root())`, and [`revive::revive_if_trashed`]
/// asking it of the deterministic id — a node that scan cannot see.
fn claims_slug(workspace: &Workspace, id: NodeId, slug: &str) -> bool {
    matches!(workspace.tree().property(id, SLUG_KEY), Some(PropValue::Text(s)) if s == slug)
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
    let aliases = match workspace.tree().property(id, ALIAS_KEY) {
        Some(PropValue::Text(s)) => outl_md::parse_alias_list(s),
        _ => Vec::new(),
    };
    Some(PageMeta {
        id: id.to_string(),
        slug,
        title,
        kind,
        icon,
        pinned,
        page_type,
        aliases,
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
    if slug.is_empty() || slug.len() > outl_md::slug::MAX_SLUG_BYTES {
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
/// **A trashed root for this slug is revived, not re-created**: the
/// deterministic id means a deleted page's slug still resolves to that
/// node, and `Op::Create` is idempotent, so this emits `Op::Move(id,
/// root, position)` to bring it back — plus one `Op::Move` per direct
/// child, into the trash, so a revive does not undo the user's delete
/// (issue #473). A caller that expected this function to be a pure read
/// for an existing page, or a `Create` for a missing one, gets a third
/// case; `revive` has the full contract.
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
    // Before the `Create`, never after. Page ids are deterministic, so a
    // deleted page's root is still the node this slug resolves to, and
    // `Op::Create` keeps an existing node's current parent — the trash. The
    // page then opened normally and dropped every keystroke (issue #473);
    // see [`revive`] for why it is a `Move` and what stays behind.
    //
    // Returns without re-asserting the slug / kind / title props: they
    // survived the `Move`, and a second `open_or_create` deliberately does
    // not rename a page that is already there
    // (`open_or_create_is_idempotent`).
    if let Some(id) = revive::revive_if_trashed(workspace, hlc, slug)? {
        return Ok(id);
    }
    let node_id = page_id_from_slug(slug);
    // `find_by_slug` only scans children of `NodeId::root`, so a
    // deleted-then-reopened page (same slug -> same deterministic id,
    // sitting under `TRASH_ROOT`) falls through to here instead of
    // being found. See `page_revive` (issue #473).
    if crate::tree::is_trashed(workspace, node_id) {
        return crate::page_revive::revive_trashed_page(workspace, hlc, node_id, slug, title, kind);
    }
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

/// Toggle the `pinned:: true` page-level property (TUI's `g P` chord;
/// `outl-tauri-shared`'s `toggle_pin` command, RFC 0254 phase 4).
///
/// `pinned` already lives in the op log as an ordinary `Op::SetProp` —
/// it converges the same way any other page property does, so this is
/// not new shared state per root `CLAUDE.md` invariant 7, just the
/// first place two GUI clients write it instead of only reading it off
/// [`PageMeta::pinned`].
///
/// Refuses journal pages (see [`ActionError::CannotPinJournal`]).
/// Returns the page's new pinned state.
pub fn toggle_pin(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    page: NodeId,
) -> Result<bool, ActionError> {
    if PageKind::parse(workspace.tree().property(page, KIND_KEY)) == PageKind::Journal {
        return Err(ActionError::CannotPinJournal(page.to_string()));
    }
    let was_pinned = match workspace.tree().property(page, "pinned") {
        Some(PropValue::Text(s)) => is_truthy(s),
        _ => false,
    };
    let new_value = (!was_pinned).then(|| PropValue::Text("true".to_string()));
    set_property(workspace, hlc, page, "pinned", new_value)?;
    Ok(!was_pinned)
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

mod revive;

#[cfg(test)]
mod tests;
