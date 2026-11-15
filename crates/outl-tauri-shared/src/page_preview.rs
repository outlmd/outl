//! Today's journal, read straight off disk, before the workspace opens.
//!
//! # Why this exists
//!
//! The screen a user waits on says "Opening your workspace…", and what
//! it is waiting for is the op log: the index sidecars, the snapshot
//! decode, the delta replay, the boot repairs. On a real 730k-op
//! workspace that is 163 ms of snapshot boot plus the index load, and on
//! a cold or snapshotless one it is seconds.
//!
//! None of it is needed to *show* today's journal. The `.md` on disk is
//! the projection of exactly that page, the sidecar beside it carries
//! the block ids, and reading the pair costs **8-11 ms** regardless of
//! how large the workspace is. That is the whole idea: content visible
//! stops depending on vault size, and only *editable* keeps depending
//! on it.
//!
//! So the opener reads that pair first and emits it, and the client
//! paints real text while the open runs behind it.
//!
//! # It holds no lock, so it writes nothing
//!
//! The preview runs **before** `open_workspace_at` takes the workspace
//! and actor locks — that is the point of it — so every read here goes
//! through `outl_actions::read_page_outline_readonly`. The ordinary
//! read resolves the sidecar through a path that *renames* a legacy
//! dotted `.<name>.outl` to its modern name, which is a write into a
//! workspace no process has claimed yet.
//!
//! What remains, and is accepted: the `.md` and its sidecar are read as
//! an unlocked pair, so a concurrent `outl serve` replacing both can be
//! observed mid-projection. The cost is a preview whose block ids do
//! not match its text for one frame. Nothing mutable reads those ids,
//! and the real view replaces the whole thing.
//!
//! # Why this is safe to show
//!
//! Markdown is the source of truth for what the user last saw
//! (`CLAUDE.md` invariant 2), so the preview is never invented: it is
//! the same bytes the workspace is about to re-project. What it is
//! *not* is authoritative about anything the op log knows and the file
//! does not yet — a peer's edit that arrived as ops, a collapsed flag
//! (`Op::SetCollapsed` lives in the log, so `read_page_outline` leaves
//! every node expanded). The real view replaces it wholesale the moment
//! the workspace publishes.
//!
//! # The two rules
//!
//! - **The preview is never editable.** Every mutation needs the
//!   workspace, which by definition is not open yet; a client that let a
//!   keystroke through here would be writing into `ERR_LOADING`.
//!   Nothing on the wire enforces that and nothing can: the guarantee
//!   lives in the renderer, `@outl/shared/boot`'s `<PreviewOutline />`,
//!   which has no interaction in it at all. A flag saying "do not edit
//!   this" would be a second, weaker statement of the same thing — one
//!   a client can ignore, where a renderer with nothing to make
//!   editable cannot be ignored.
//! - **It is best-effort and silent.** A missing `.md` (a fresh
//!   workspace, a day with no journal yet), an unreadable one, a missing
//!   sidecar — all of them mean "no preview", never an error. The open
//!   behind it is the real answer and it reports its own failures.

use std::path::Path;

use outl_actions::{PageKind, PageMeta};
use tauri::Emitter;
use tracing::{debug, info, warn};

use crate::state::PageView;

/// The Tauri event this is emitted on.
///
/// The payload is a plain [`PageView`], the same shape every open
/// command returns, so a client renders it with the component it
/// already has and the existing wire pin covers it.
pub const PAGE_PREVIEW_EVENT: &str = "page-preview";

/// Read today's journal off disk, with no workspace.
///
/// `None` when there is nothing to show: no `.md`, an unreadable one, or
/// a parse that yields no blocks. Never an error — see the module doc.
///
/// The page id is derived, not looked up: `NodeId::from_slug` is the
/// deterministic page-root id every path that materialises a page root
/// already uses, so the preview's id is the one the real view will
/// carry. That is what lets a client swap the preview for the real view
/// without the outline remounting under the user.
fn today_journal_preview(root: &Path) -> Option<PageView> {
    let slug = outl_actions::dates::journal_slug(outl_actions::clock::today());
    page_preview(root, &slug, PageKind::Journal)
}

/// Read today's journal and emit it, best-effort.
///
/// The opener's one-line entry point, shared because both clients call
/// it with the same arguments for the same reason — a second copy is
/// how one of them starts forgetting (this crate's whole premise).
///
/// Silent on every failure: a fresh workspace, a day with no journal
/// yet, an unreadable file. The open running behind this is the real
/// answer and reports its own failures; a preview that could fail a
/// launch would be worse than no preview.
pub fn emit_today_journal_preview(app: &tauri::AppHandle, root: &Path) {
    let at = std::time::Instant::now();
    let Some(view) = today_journal_preview(root) else {
        return;
    };
    let blocks = view.outline.len();
    if let Err(e) = app.emit(PAGE_PREVIEW_EVENT, view) {
        warn!("emit page-preview: {e}");
        return;
    }
    info!(
        "boot: page preview emitted ({blocks} blocks) in {}ms",
        at.elapsed().as_millis()
    );
}

/// [`today_journal_preview`] for any slug.
///
/// Private: today's journal is the only landing page a launch has. A
/// client that restores a different one would call this, and making it
/// public before that client exists is a guess about its signature.
fn page_preview(root: &Path, slug: &str, kind: PageKind) -> Option<PageView> {
    let meta = PageMeta {
        id: outl_core::id::NodeId::from_slug(slug).to_string(),
        slug: slug.to_string(),
        // The `.md` carries `title::` as a page property, and resolving
        // it properly needs the parsed page we are about to read. The
        // slug is the honest placeholder until then: a journal's slug
        // *is* its date, which is what the header renders anyway.
        title: slug.to_string(),
        kind,
        icon: None,
        // All three live in the page's properties, which the real view
        // resolves through the tree. A preview that guessed would be a
        // second opinion about page metadata.
        page_type: None,
        aliases: Vec::new(),
        pinned: false,
    };
    // `_readonly`: this runs before `open_workspace_at` installs the
    // workspace guards, so it must not take the legacy-sidecar rename
    // that the ordinary read performs. See that function's doc.
    let outline = match outl_actions::read_page_outline_readonly(root, &meta) {
        Ok(page) => page,
        Err(e) => {
            // Expected on a fresh workspace and on the first visit to a
            // day. `debug`, not `warn`: the open behind this is the real
            // answer, and a line per launch about a file that does not
            // exist yet is noise that trains people past the log.
            debug!("no preview for {slug}: {e}");
            return None;
        }
    };
    if outline.nodes.is_empty() {
        return None;
    }
    Some(PageView {
        page: meta,
        outline: outline.nodes,
        // Backlinks are an O(blocks) scan the real open already fetches
        // lazily, so the preview carries none rather than a wrong none:
        // the client's backlinks section has its own placeholder and
        // reads `lateSectionVisible`, which draws nothing on a first
        // load (`@outl/shared/outline`).
        backlinks: Vec::new(),
        backlinks_order: outl_config::BacklinksOrder::default(),
        // Page properties live in the `.md` frontmatter the parser
        // already read, but resolving them the way the real view does
        // needs `outl_actions::property::page_properties`, which reads
        // the tree. Empty here, correct one view later.
        page_properties: Vec::new(),
        warnings: outline.warnings,
        // The ahead-of-log check needs the op log, so this reply is
        // explicitly **unchecked**: a client's sticky banner must not be
        // cleared by a preview that never asked the question.
        md_ahead_of_log: None,
        md_ahead_of_log_checked: false,
        projection_error: None,
        dropped_edit: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn workspace_with_journal(slug: &str, md: &str) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let journals = dir.path().join("journals");
        std::fs::create_dir_all(&journals).expect("journals dir");
        std::fs::write(journals.join(format!("{slug}.md")), md).expect("write md");
        dir
    }

    /// The point of the whole module: text on disk reaches a client
    /// without a `Workspace` existing anywhere.
    #[test]
    fn a_page_on_disk_previews_without_a_workspace() {
        let dir = workspace_with_journal("2026-11-15", "- first line\n- second line\n");
        let preview =
            page_preview(dir.path(), "2026-11-15", PageKind::Journal).expect("preview exists");

        assert_eq!(preview.outline.len(), 2);
        assert_eq!(preview.outline[0].text, "first line");
    }

    /// The id has to be the one the real view will carry, or swapping
    /// the preview for it remounts the outline under the user.
    #[test]
    fn the_page_id_matches_the_deterministic_page_root() {
        let dir = workspace_with_journal("2026-11-15", "- a\n");
        let preview = page_preview(dir.path(), "2026-11-15", PageKind::Journal).expect("preview");
        assert_eq!(
            preview.page.id,
            outl_core::id::NodeId::from_slug("2026-11-15").to_string()
        );
    }

    #[test]
    fn a_missing_page_is_no_preview_rather_than_an_error() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("journals")).expect("journals dir");
        assert!(page_preview(dir.path(), "2026-11-15", PageKind::Journal).is_none());
    }

    /// A page whose `.md` is there but holds nothing has no content to
    /// show, and an empty outline painted over the loading state reads
    /// as "my notes are gone".
    #[test]
    fn an_empty_page_is_no_preview() {
        let dir = workspace_with_journal("2026-11-15", "");
        assert!(page_preview(dir.path(), "2026-11-15", PageKind::Journal).is_none());
    }

    /// Block ids come from the sidecar when there is one; with none, the
    /// preview still renders (transient ids), because showing the text
    /// is the job.
    /// The preview runs before any lock exists, so it must not perform
    /// the legacy-sidecar migration the ordinary read does. A rename
    /// here is a write into a workspace no process has claimed.
    #[test]
    fn a_legacy_sidecar_is_read_but_never_renamed() {
        let dir = workspace_with_journal("2026-11-15", "- from the legacy sidecar\n");
        let legacy = dir.path().join("journals").join(".2026-11-15.outl");
        // Shape does not matter: an unreadable sidecar degrades to
        // transient ids, and what is asserted is the filesystem.
        std::fs::write(&legacy, "{}").expect("write legacy sidecar");

        let preview = page_preview(dir.path(), "2026-11-15", PageKind::Journal).expect("preview");
        assert_eq!(preview.outline[0].text, "from the legacy sidecar");
        assert!(
            legacy.exists(),
            "the preview renamed a legacy sidecar while holding no workspace lock"
        );
        assert!(
            !dir.path().join("journals").join("2026-11-15.outl").exists(),
            "the preview created the modern sidecar path"
        );
    }

    #[test]
    fn a_page_with_no_sidecar_still_previews() {
        let dir = workspace_with_journal("2026-11-15", "- no sidecar here\n");
        let preview = page_preview(dir.path(), "2026-11-15", PageKind::Journal).expect("preview");
        assert_eq!(preview.outline[0].text, "no sidecar here");
        assert!(!preview.outline[0].id.is_empty());
    }
}
