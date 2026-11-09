//! The trash overlay (`/trash`, `:trash`): every block deleted out of
//! this workspace, and the way back.
//!
//! Invariant 6 makes delete a `Move(node, TRASH_ROOT)`, so this client
//! has always *preserved* deletions and could never show one. Until now
//! `Capability::Trash`'s nudge told the user to run `outl trash list` in
//! a terminal — against a workspace this very process holds an exclusive
//! actor lock on, so the honest instruction was "quit first" (issue
//! #287).
//!
//! **Opened from the command registry, not a chord.** `/trash` and
//! `:trash` are one registration (`commands::builtins::TrashCommand`),
//! the same surface `:sync`, `/template` and `/plugin-settings` use. A
//! chord would need an `Action` variant, and `outl_shortcuts::support`'s
//! exhaustive `match` is frozen at its size baseline — the three
//! per-client verdicts it would force are the mechanism invariant 12
//! exists for, not a reason to skip it, so this is a deferral with a
//! file behind it rather than a design.
//!
//! Every verdict belongs to `outl_actions::trash`. The overlay carries
//! `refusal_for`'s own sentence per row and never re-words or re-derives
//! it, and a restore the mutation declines lands in [`TrashState`]'s
//! `message` — see [`App::restore_selected_trash`].
//!
//! **The row's sentence is worded for `Surface::Tui`, the message is
//! not.** `trash::list` takes the surface, so a row's way out is one
//! this client can actually offer — never `outl block tree <ULID>`
//! against ids the overlay does not show, and never "move that block
//! out" about a block that sits inside the deletion and so appears in no
//! outline. The mutation's error stays the bare diagnosis on purpose:
//! `restore_selected_trash` re-lists immediately, so the worded half
//! arrives in the row rather than twice in two voices.

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crossterm::event::{KeyCode, KeyEvent};

use outl_actions::refusal::Surface;
use outl_actions::trash::{self, TrashEntry};
use outl_actions::{
    commit_page, ActionError, CommitHooks, PageKind, PageMeta, ProjectionRefusal, SyncTransport,
};
use outl_md::ParsedPage;

use crate::state::{App, Overlay, ToastKind};

/// What the overlay shows: the listing plus a cursor and a status line.
///
/// A snapshot taken when the overlay opens, for the reason
/// `RemindersState` gives: the list is a fold of the op log per entry,
/// and nothing can be deleted while the overlay owns the keyboard.
/// A restore re-reads it, because that one *does* change the list.
#[derive(Debug, Default)]
pub(crate) struct TrashState {
    /// Every top-level deletion, in `trash::list`'s order.
    pub(crate) entries: Vec<TrashEntry>,
    /// Index into [`Self::entries`] of the highlighted row.
    pub(crate) selected: usize,
    /// Transient line at the bottom: what a restore did, or the refusal
    /// it came back with. Same shape as `PropertiesState::message`.
    pub(crate) message: Option<String>,
}

/// Where a trash row came from, and what opening it would mean.
///
/// The distinction is the whole point: for a block the page is alive and
/// opening it only navigates, while for a page row the `.md` went with
/// the delete and opening it would **revive** the page. Returning one
/// `Option<String>` for both made those two the same call, which is how
/// `Enter` on a deleted page undid the delete.
enum Origin {
    /// A deleted block, and the live page it sat on.
    ///
    /// The whole [`PageMeta`] rather than a slug, because both halves of
    /// opening it branch on `kind`: a journal's `.md` lives under
    /// `journals/` and is opened as a date, a page's under `pages/` and
    /// is opened by slug. Carrying only the slug is what made the first
    /// guard here ask about a file the opener would never look at.
    Block(PageMeta),
    /// The deleted page itself. Its `.md` is gone with it.
    Page(String),
    /// Its parent is in the trash too — the dominant refused shape.
    ParentTrashed,
    /// The log holds no move that put it there, or cannot say where from.
    Unknown,
    /// It was never on a page: deleted from directly under `root()`.
    NoPage,
}

impl TrashState {
    /// Read the listing for `workspace`.
    pub(crate) fn load(workspace: &outl_core::workspace::Workspace) -> Self {
        Self {
            entries: trash::list(workspace, Surface::Tui),
            selected: 0,
            message: None,
        }
    }

    /// Clamped cursor move.
    pub(crate) fn step(&mut self, delta: i32) {
        let last = self.entries.len().saturating_sub(1) as i32;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
    }

    fn selected_entry(&self) -> Option<&TrashEntry> {
        self.entries.get(self.selected)
    }
}

impl App {
    /// `/trash` — open the overlay.
    pub(crate) fn open_trash(&mut self) {
        // The listing reads the workspace tree, and this client edits an
        // in-memory AST that only reaches the tree at a save boundary.
        // Without this, a block deleted a keystroke ago is absent from
        // the trash the user is looking at. Same reason
        // `open_properties` flushes first.
        self.flush_pending_save();
        let state = TrashState::load(&self.workspace);
        self.overlay = Some(Overlay::Trash(state));
    }

    pub(crate) fn move_trash_cursor(&mut self, delta: i32) {
        if let Some(Overlay::Trash(t)) = self.overlay.as_mut() {
            t.step(delta);
        }
    }

    /// `Enter` — open the page a deleted **block** came from.
    ///
    /// Read-only for a block row, which is the only row it acts on: the
    /// block's page is still alive, so `open_page_by_slug` finds its
    /// `.md` and only navigates. "Where is my content" stays answerable
    /// on a row whose restore is refused, which is the question a
    /// refusal leaves the user holding.
    ///
    /// **A page row is refused, and that is not tidiness.**
    /// `delete_page` drops the projection, so a deleted page has no
    /// `.md`; `open_page_slug` reads a missing file as "new page", writes
    /// a `title:: <slug>` seed and reconciles it. `reconcile_md`'s
    /// `ensure_page_root_in_tree` then moves the root out of the trash
    /// and leaves the children live, and the sidecar-less matching
    /// orphan-trashes each child on its own. So `Enter` on a deleted
    /// 40-block page would undo the delete, overwrite the real `title::`
    /// with the slug, and turn one trash row into forty.
    ///
    /// Reviving a page deliberately is a real operation and this is not
    /// it: `page::open_or_create` owns that (`page/revive.rs`), the user
    /// reaches it by opening the page **by name**, and the file whose
    /// content `reconcile_md` treats as evidence of intent would here
    /// have been written by this client two instructions earlier. The
    /// nudge names the deliberate route rather than performing it.
    pub(crate) fn open_selected_trash_origin(&mut self) {
        match self.selected_trash_origin() {
            Origin::Page(slug) => {
                self.set_trash_message(format!(
                    "`{slug}` is the deleted page itself — opening it here would bring it \
                     back. Close this and open `{slug}` by name if that is what you want."
                ));
            }
            Origin::ParentTrashed => {
                // The dominant refused shape — 89 of 393 top-level
                // deletions on the reference workspace. It *was* on a
                // page; that page is in the trash too, which is a
                // different fact from having had none.
                self.set_trash_message(
                    "the block this sat under is in the trash too — restore that one first"
                        .to_string(),
                );
            }
            Origin::Unknown => {
                self.set_trash_message(
                    "the log can't say where this came from, so there's nowhere to open"
                        .to_string(),
                );
            }
            Origin::NoPage => {
                self.set_trash_message(
                    "this block wasn't on a page — there's nowhere to open".to_string(),
                );
            }
            Origin::Block(meta) => {
                // Item 3's sibling on this path: the page is live in the
                // tree, but a page a peer created and nobody has opened
                // has no local `.md` yet. `open_page_slug` reads a
                // missing file as "new page" and seeds it — the same
                // seed-and-reconcile that revives a deleted page, here
                // overwriting a real `title::` with the slug and
                // reconciling a one-bullet page over the peer's blocks.
                //
                // The page-row guard above keys on row *kind*, which does
                // not catch this. The existence check does — and it has
                // to ask `page_md_path`, the owner of where a page's
                // `.md` lives, because journals live under `journals/`
                // and pages under `pages/`. Checking "either folder" let
                // a journal through to `open_page_by_slug`, which only
                // ever looks at `pages/{slug}.md` and so seeded a
                // spurious `pages/` copy of a journal — the exact hazard
                // this guard exists to stop, and in a journal-first app
                // the common case rather than an edge.
                let md = outl_actions::page_md_path(&self.workspace_root, &meta);
                if !md.exists() {
                    let slug = &meta.slug;
                    self.set_trash_message(format!(
                        "`{slug}` hasn't been downloaded to this device yet — open it from \
                         the switcher (Ctrl+P) once it syncs."
                    ));
                    return;
                }
                self.overlay = None;
                if let Err(e) = self.open_origin_page(&meta) {
                    self.status = format!("could not open {}: {e}", meta.slug);
                }
            }
        }
    }

    /// Open the page a deleted block came from, by its kind.
    ///
    /// `open_page_by_slug` is the `pages/` opener and nothing else —
    /// handing it a journal slug makes it seed `pages/<date>.md`. A
    /// journal is a date in `View::Journal`, which is how every other
    /// journal open in this crate reaches it.
    fn open_origin_page(&mut self, meta: &PageMeta) -> anyhow::Result<()> {
        match meta.kind {
            PageKind::Journal => {
                let date = chrono::NaiveDate::parse_from_str(&meta.slug, "%Y-%m-%d")?;
                self.view = crate::state::View::Journal(date);
                self.selected = 0;
                self.cursor_col = 0;
                self.ensure_view_file_exists()?;
                self.load_current();
                Ok(())
            }
            PageKind::Page => self.open_page_by_slug(&meta.slug),
        }
    }

    fn set_trash_message(&mut self, message: String) {
        if let Some(Overlay::Trash(t)) = self.overlay.as_mut() {
            t.message = Some(message);
        }
    }

    /// Where the selected deletion came from, and whether opening it is
    /// a navigation, a revive, or not possible.
    ///
    /// **`page_slug_of` answering `None` has three causes and they are
    /// not one message.** The dominant one by far is a parent that is
    /// itself in the trash — 89 of 393 top-level deletions on the
    /// reference workspace — where "this block wasn't on a page" is
    /// simply false: it was, and that page is in the trash too.
    fn selected_trash_origin(&self) -> Origin {
        let Some(Overlay::Trash(t)) = &self.overlay else {
            return Origin::Unknown;
        };
        let Some(entry) = t.selected_entry() else {
            return Origin::Unknown;
        };
        // A deleted *page* carries its own slug, and its `.md` is gone
        // with it — see this function's caller for what opening that
        // would do.
        if let Some(slug) = &entry.page {
            return Origin::Page(slug.clone());
        }
        let parent = entry.parent_at_deletion.or_else(|| {
            trash::parent_at_deletion(&self.workspace, entry.node)
                .ok()
                .flatten()
        });
        let Some(parent) = parent else {
            // No move in the log put it there, or the log cannot say
            // where from.
            return Origin::Unknown;
        };
        if parent == NodeId::root() {
            return Origin::NoPage;
        }
        if outl_actions::is_trashed(&self.workspace, parent) {
            return Origin::ParentTrashed;
        }
        let meta = outl_actions::enclosing_page_id(&self.workspace, parent)
            .and_then(|page| outl_actions::page_meta(&self.workspace, page));
        match meta {
            Some(meta) => Origin::Block(meta),
            // Present, untrashed, on no page: a pre-page-model block
            // `migrate_legacy_into_today` has not swept yet.
            None => Origin::NoPage,
        }
    }

    /// `r` — put the selected block back where it was deleted from.
    ///
    /// Two refusals reach the user through one line, deliberately. The
    /// row's own `refusal` is a snapshot: the verdict depends on the live
    /// tree, so a peer can trash the landing parent between the listing
    /// and the keystroke. `trash::restore` asks `refusal_for` again under
    /// this process's own lock, and its answer is the **same verdict**
    /// the row would have carried. So the message is written where the
    /// user is looking and the list is re-read, which settles the row
    /// into the state it should have been in.
    ///
    /// **The message is the bare diagnosis; the row carries the way
    /// out.** They used to be one string and that is what let a desktop
    /// user be told to run `outl block tree <ULID>`. `trash::list` words
    /// each row for `Surface::Tui`, and this line deliberately does not:
    /// the re-list a few lines down is what delivers the worded half, to
    /// the row the sentence is about, instead of printing the advice
    /// twice.
    pub(crate) fn restore_selected_trash(&mut self) {
        let Some(Overlay::Trash(t)) = &self.overlay else {
            return;
        };
        let Some(node) = t.selected_entry().map(|entry| entry.node) else {
            return;
        };
        let keep = t.selected;

        let outcome = self.restore_from_trash(node);
        let mut state = TrashState::load(&self.workspace);
        state.selected = keep;
        state.step(0);
        state.message = Some(match outcome {
            // The backend owns the whole sentence; prefixing it here
            // would state the refusal twice, the way `outl trash list`'s
            // human output used to.
            Err(why) => why,
            Ok(entry) if entry.subtree_len > 1 => {
                format!("restored {} blocks", entry.subtree_len)
            }
            Ok(_) => "restored".to_string(),
        });
        self.overlay = Some(Overlay::Trash(state));
    }

    /// The mutation half: restore through the shared commit pipeline.
    ///
    /// **All five steps, not step 5 alone.** The first version called
    /// `apply_page_md_with_sidecar_guarded` directly, and each of the
    /// four it skipped had a concrete cost: no undo snapshot, so `u`
    /// after a restore applied the page's *previous* snapshot and
    /// re-trashed the block while reverting an unrelated edit; no
    /// backlink invalidation, so a restored `see [[infra]]` stayed
    /// missing from `infra`'s backlinks until a rebuild; no announce, so
    /// peers waited for the next catch-up sweep. `CaptureCommit` in this
    /// crate is the pattern — [`RestoreCommit`] is its sibling.
    fn restore_from_trash(&mut self, node: NodeId) -> Result<TrashEntry, String> {
        // A pending local save renders the AST over the `.md`, and the
        // commit below writes the page from the tree — flushing first is
        // what stops the two from racing over one file.
        self.flush_pending_save();

        // The landing page has to be resolved *before* the mutation:
        // `commit_page` is scoped to one page, and a trashed node's own
        // ancestor walk reaches the trash root rather than a page.
        // `trash::landing_page` is the one owner of that question — it
        // asks `page_meta`, so a pre-page-model root child answers `None`
        // instead of reaching `commit_page` and projecting `NotInTree`,
        // which this client would then report as a page that stopped
        // syncing. `None` means "nothing to project"; an unplaceable node
        // is `trash::restore`'s to refuse.
        let landing = trash::landing_page(&self.workspace, node);

        let hlc = self.hlc.clone();
        let Some(page) = landing else {
            let entry =
                trash::restore(&mut self.workspace, &hlc, node).map_err(|err| err.to_string())?;
            self.load_current();
            return Ok(entry);
        };

        let landing_path = outl_actions::page_meta(&self.workspace, page)
            .map(|meta| outl_actions::page_md_path(&self.workspace_root, &meta));
        let mut hooks = RestoreCommit {
            root: self.workspace_root.clone(),
            hlc: hlc.clone(),
            transport: self.sync_transport.clone(),
            landing_path,
            undo: None,
            backlinks_stale: false,
            projection_failure: None,
        };
        let entry = commit_page(&mut self.workspace, &mut hooks, page, |ws| {
            trash::restore(ws, &hlc, node)
        })
        .map_err(|err| err.to_string())?;

        // The snapshot carries the *landing* page's path, not the open
        // view's, so `top_applies_here` only offers it once the user is
        // on that page — the cross-page rule `snapshot_page_for_undo`
        // owns ([#431](https://github.com/outlmd/outl/issues/431)).
        if let (Some(path), Some(before)) = (hooks.landing_path.clone(), hooks.undo.take()) {
            self.snapshot_page_for_undo(path, before);
        }
        if hooks.backlinks_stale {
            self.spawn_backlink_index_rebuild();
        }
        if let Some(e) = hooks.projection_failure.take() {
            self.report_restore_projection_failure(page, e);
        }
        // Re-read the open view: when the restore landed on the page the
        // user is looking at, the block has to appear.
        self.load_current();
        Ok(entry)
    }

    /// Report a withheld projection — **classified**, because the two
    /// cases want opposite surfaces.
    ///
    /// An invariant-8 refusal means the landing page has stopped
    /// converging until `outl reconcile --ahead-of-log` runs, and the
    /// user has to hear it. Everything else (I/O, permissions, an
    /// unreadable sidecar) is local and retryable, and this crate's rule
    /// for those is already written down: a `tracing::warn!` and nothing
    /// else, because "nagging about it trains the user past the banner
    /// that matters". Toasting both put a permissions error in the same
    /// voice as a frozen page, which costs exactly what the banner buys.
    ///
    /// `ProjectionRefusal::from_error` is the classifier; `loading.rs`
    /// asks the same one on the page-open path, so the two cannot reach
    /// different verdicts.
    ///
    /// **Only a cross-page landing is announced here.** `load_current`
    /// below re-derives the persistent banner for the page on screen, so
    /// a restore into the open page is already covered by the surface
    /// this crate says the notice belongs on. What that surface cannot
    /// reach is a page the user is not looking at.
    fn report_restore_projection_failure(&mut self, page: NodeId, e: ActionError) {
        match ProjectionRefusal::from_error(&e) {
            ProjectionRefusal::Retryable(message) => {
                tracing::warn!("restore: re-projection failed: {message}");
            }
            ProjectionRefusal::AheadOfLog { lines, sample, .. } => {
                if self.page_is_open(page) {
                    return;
                }
                // `one_line` is the catalog's, including the join. This
                // used to assemble `title`+`action`+`command` here and
                // the result was a run-on, which is what a client
                // authoring prose costs even when every word is
                // borrowed. The slug stays the client's: the notice
                // cannot name *which* page when it is not the page on
                // screen.
                let notice =
                    outl_actions::refusal::ahead_of_log_notice(Surface::Tui, lines, &sample);
                let slug = outl_actions::page_meta(&self.workspace, page)
                    .map(|meta| meta.slug)
                    .unwrap_or_else(|| "that page".to_string());
                self.toast(
                    ToastKind::Warning,
                    format!("`{slug}`: {}", notice.one_line()),
                );
            }
        }
    }

    /// Whether `page` is the view the user is looking at.
    fn page_is_open(&self, page: NodeId) -> bool {
        outl_actions::find_by_slug(&self.workspace, &self.current_slug()) == Some(page)
    }
}

/// This crate's [`CommitHooks`] for a restore.
///
/// The sibling of `CaptureCommit` (`actions/lifecycle/capture.rs`), and
/// it differs in one way: a restore can land on a page the user is not
/// looking at, so the undo snapshot is carried back out with the page's
/// own path instead of being taken from the open view.
struct RestoreCommit {
    root: PathBuf,
    hlc: HlcGenerator,
    transport: Option<Arc<dyn SyncTransport>>,
    /// Where the landing page's `.md` lives.
    ///
    /// Resolved by the caller, which still holds the workspace —
    /// `record_undo` is handed a `NodeId` and a string, and re-deriving
    /// a path from an id needs a `page_meta` lookup this hook has no
    /// workspace to run.
    landing_path: Option<PathBuf>,
    /// The landing page's pre-mutation parse, for the undo stack.
    undo: Option<ParsedPage>,
    /// Set when the commit changed what some page's backlinks are. A
    /// restored `see [[infra]]` is missing from `infra`'s backlinks
    /// until the index is rebuilt.
    backlinks_stale: bool,
    projection_failure: Option<ActionError>,
}

impl CommitHooks for RestoreCommit {
    fn project(&mut self, workspace: &Workspace, page: NodeId) {
        // Guarded: a mutation must project, but projecting must not
        // delete content the op log never saw (invariant 8). The restore
        // is already in the op log either way.
        if let Err(e) =
            outl_actions::apply_page_md_with_sidecar_guarded(workspace, &self.root, page)
        {
            self.projection_failure = Some(e);
        }
    }

    fn records_undo(&self) -> bool {
        true
    }

    fn record_undo(&mut self, _page: NodeId, before: String) {
        // `commit_page` hands over the pre-mutation **render**, and this
        // crate's stack holds a `ParsedPage` keyed by the file it came
        // from — so parse it back rather than keeping a second notion of
        // what a snapshot is. The pipeline has already decided the render
        // actually changed, which is the gate every `snapshot_for_undo`
        // caller otherwise applies by hand.
        self.undo = Some(outl_md::parse(&before));
    }

    fn invalidate_backlinks(&mut self) {
        // A flag, not the rebuild: the index lives on `App`, which this
        // hook cannot borrow while `commit_page` holds the workspace.
        // Same shape as `CaptureCommit`'s.
        self.backlinks_stale = true;
    }

    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        let Some(transport) = &self.transport else {
            return;
        };
        if let Some(meta) = outl_actions::page_meta(workspace, page) {
            transport.announce_local_ops(&meta.slug, self.hlc.next());
        }
    }
}

/// Trash overlay keys: move, restore, open the origin page.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.overlay = None,
        KeyCode::Enter => app.open_selected_trash_origin(),
        KeyCode::Char('r') => app.restore_selected_trash(),
        KeyCode::Up | KeyCode::Char('k') => app.move_trash_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_trash_cursor(1),
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
