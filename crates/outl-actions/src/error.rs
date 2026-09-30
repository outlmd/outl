//! Errors surfaced by action functions.

use outl_core::workspace::WorkspaceError;
use thiserror::Error;

/// The command that brings a page's unlogged `.md` content into the op
/// log — the recovery for [`ActionError::PageMarkdownAheadOfLog`].
///
/// One constant so every surface that names it (this variant's own
/// `Display`, the MCP's structured refusal, `outl doctor`'s listing)
/// points at something that actually runs. RFC 0255.
pub const AHEAD_OF_LOG_RECOVERY_COMMAND: &str = "outl reconcile --ahead-of-log";

/// Reasons an action may fail. UI layers convert these to their own
/// error surface (toasts, returned strings, panics in tests).
#[derive(Debug, Error)]
pub enum ActionError {
    /// The referenced block is not part of the materialised tree.
    /// Either it was never created, was already moved to trash, or
    /// the caller passed a stale id.
    #[error("block {0} is not in the tree")]
    NotInTree(String),

    /// The block is in the tree but is missing a position record.
    /// Should never happen; if it does, the tree state is corrupt.
    #[error("block {0} has no position in the tree")]
    MissingPosition(String),

    /// `trash::restore` was asked to put a block back under a parent
    /// that is itself in the trash.
    ///
    /// Deleting a child on its own and then deleting its parent leaves
    /// both as direct children of the trash root — 89 of the 393
    /// top-level deletions on the workspace issue #287 was measured
    /// against. Restoring the child alone would succeed structurally
    /// and change nothing the user can see, which is worse than a
    /// refusal: it reports success for a block that stays invisible.
    /// The ancestor is restorable too, so the refusal names it.
    #[error(
        "cannot restore {node}: the block it was deleted from ({parent}) is in the trash too — \
         restore {parent} first"
    )]
    TrashParentTrashed {
        /// The block the caller asked to restore.
        node: String,
        /// Its parent at deletion time, itself trashed.
        parent: String,
    },

    /// `trash::restore` was asked to restore a deleted **page**.
    ///
    /// A block comes back with one `Move`. A page needs that plus a
    /// re-projected `.md`, and the slug it answered to is very often
    /// taken by now — 16 of the 18 deleted pages on the workspace issue
    /// #287 was measured against. Inventing a free slug would make
    /// `trash::restore` a second owner of the slug rule, so v1 refuses
    /// and says where the content still is.
    #[error(
        "cannot restore the page `{slug}` yet — only blocks can be restored today. \
         Nothing is lost: the content is still in the op log, and \
         `outl page history {slug}` shows it"
    )]
    TrashPageRestoreUnsupported {
        /// The deleted page's slug.
        slug: String,
    },

    /// `trash::restore` folded a parent out of the op log that is not
    /// in the materialized tree at all.
    ///
    /// Zero occurrences on the workspace issue #287 was measured
    /// against, and it is not hypothetical: the ops that *create* a node
    /// can be missing while ops that reference it survive, which is
    /// exactly [#301](https://github.com/outlmd/outl/issues/301). Naming
    /// it as its own refusal keeps that diagnosis separate from "your
    /// parent is in the trash", which has a fix the user can act on.
    #[error(
        "cannot restore {node}: the block it was deleted from ({parent}) is not in the tree — \
         run `outl doctor` to see whether ops for it are missing"
    )]
    TrashParentMissing {
        /// The block the caller asked to restore.
        node: String,
        /// Its parent at deletion time, absent from the tree.
        parent: String,
    },

    /// `trash::restore` was asked to restore a node the op log never
    /// moved to the trash. Distinct from [`ActionError::NotInTree`]:
    /// the block is fine, there is simply nothing to undo.
    ///
    /// **Only for a node that is genuinely outside the trash.** It used
    /// to double as the answer when the *fold* came back empty, which
    /// contradicted the `is_trashed` check made moments earlier: the
    /// listing showed the block and the line under it said the block
    /// was not in the trash. That case is
    /// [`ActionError::TrashOriginUnknown`] now.
    #[error("block {0} is not in the trash — there is nothing to restore")]
    NotTrashed(String),

    /// The node is in the trash, but the op log cannot say where it was
    /// before.
    ///
    /// Three shapes reach this, and they share an answer because the
    /// user's options are the same in all three — the block's text is
    /// readable in the listing, and putting it back is a copy/paste:
    ///
    /// - the log holds no move that placed it (a node created straight
    ///   under the trash root);
    /// - reading its ops failed, which is the damaged-log class
    ///   `outl doctor` reports;
    /// - the only move on record is one the tree **refused** as a cycle.
    ///   Invariant 4 keeps such an op in the log, so a fold that replays
    ///   it lands inside the subtree being restored. Believing it would
    ///   produce "restore X first" naming X itself.
    #[error("cannot restore {node}: the op log cannot say where it was ({why})")]
    TrashOriginUnknown {
        /// The block the caller asked to restore.
        node: String,
        /// Which of the three shapes this is, in the user's words.
        why: String,
    },

    /// The block is at the top of its sibling list and cannot be
    /// indented under a previous sibling.
    #[error("cannot indent {0}: no previous sibling")]
    NoPreviousSibling(String),

    /// The block is already at the root level and cannot be promoted
    /// further.
    #[error("cannot outdent {0}: already at root level")]
    AlreadyAtRoot(String),

    /// The parent does not have a parent of its own (used by outdent
    /// when walking up two levels).
    #[error("cannot outdent {0}: parent has no grandparent")]
    NoGrandparent(String),

    /// A move (e.g. cut-and-paste of a block) would drop the node
    /// inside its own subtree, creating a cycle. The CRDT treats such
    /// a move as a deterministic no-op on the materialised tree, so
    /// we reject it up front and let the client nudge the user
    /// instead of emitting an op that does nothing visible.
    #[error("cannot move {0}: target is inside the block's own subtree")]
    WouldCreateCycle(String),

    /// The page slug failed validation (empty, too long, contains a
    /// path separator, `..`, or a control character). The slug ends
    /// up joined into a filesystem path, so we reject anything that
    /// could escape its directory before it reaches storage.
    #[error("invalid page slug `{0}`")]
    InvalidSlug(String),

    /// No page in the workspace carries the requested slug. Returned
    /// by `page::delete` (and any caller that resolves a slug before
    /// acting on a page that must already exist) so the UI can surface
    /// "page not found" instead of a generic `NotInTree`.
    #[error("page `{0}` not found")]
    PageNotFound(String),

    /// `page::toggle_pin` was asked to pin a journal page. A journal
    /// auto-rotates daily, so pinning one would silently dilute every
    /// sidebar's `Pinned` list with date-shaped junk instead of the
    /// canonical entry points a user actually wants pinned. Mirrors
    /// the TUI's `g P` refusal (`outl-tui/src/actions/block/metadata.rs`).
    #[error("cannot pin `{0}`: journal pages auto-rotate and can't be pinned")]
    CannotPinJournal(String),

    /// A page's `.md` is gone while its `.outl` sidecar is still there.
    ///
    /// The sidecar is only ever written next to a `.md` this device
    /// projected, so it is proof the page existed. Treating that as "a
    /// new page" and rendering over it rebuilds the sidecar from the
    /// fresh content, and the next `reconcile_md` trashes every block
    /// whose id just disappeared. Refusing keeps the loss recoverable:
    /// the op log still holds the content, and re-projecting the page
    /// brings the `.md` back.
    #[error(
        "refusing to rewrite `{0}`: the .md is missing but its sidecar is still there, \
         so this is a lost file, not a new page — re-project the page from the op log"
    )]
    PageMarkdownVanished(String),

    /// A page's `.md` is an iCloud placeholder whose bytes have not been
    /// downloaded to this device yet.
    ///
    /// On iOS and legacy iCloud Drive the un-downloaded form is
    /// `.foo.md.icloud` and the real name does **not** exist, so the
    /// read comes back `NotFound` rather than as an I/O error. Writing
    /// then replaces a file that was never lost.
    #[error("refusing to rewrite `{0}`: iCloud has not downloaded this file to this device yet")]
    PageMarkdownNotDownloaded(String),

    /// A page's `.md` holds content that exists in no op, while its
    /// sidecar still declares those bytes a faithful projection.
    ///
    /// The hash gate that guards re-projection asks one question — does
    /// the sidecar agree with the bytes on disk? — and a `.md` in this
    /// state answers yes. So it reads as a merely *stale* projection,
    /// and re-rendering the tree over it deletes the unlogged content
    /// while every consistency check afterwards agrees the page is
    /// healthy: the new sidecar is built from the same render.
    ///
    /// That is the shape of a silent loss, so this is an error rather
    /// than a skipped write. The `.md` is left byte-for-byte alone;
    /// `outl reconcile` owns the `.md → tree` direction and is what
    /// brings the content into the log.
    #[error(
        "refusing to rewrite `{path}`: the .md holds {lines} line(s) that exist in no op \
         (e.g. {sample}) — run `{AHEAD_OF_LOG_RECOVERY_COMMAND}` so they enter the op log first"
    )]
    PageMarkdownAheadOfLog {
        /// The `.md` that would have been overwritten.
        path: String,
        /// How many content lines exist only on disk.
        lines: usize,
        /// One of those lines, quoted, so the message names what is at
        /// risk instead of only counting it.
        sample: String,
    },

    /// A page's `.md` is present, but its sidecar cannot be read at all
    /// — missing, corrupt, or written by a newer binary.
    ///
    /// The sidecar is what answers "does the op log know this line", so
    /// without it the honest answer is *I cannot tell*, and a write on
    /// that answer is how the loss [`Self::PageMarkdownAheadOfLog`]
    /// guards against reopens on a different hinge. Refusing is
    /// therefore the same decision, but it is **not** the same
    /// condition: nothing here says the file holds unlogged content, and
    /// `outl reconcile --ahead-of-log` is not the recovery. The sidecar
    /// is rebuilt by the orphan pass (`sync::needs_reconcile` maps an
    /// unreadable sidecar to `true`), and the page projects on the pass
    /// after — so this is a local, transient condition, which is exactly
    /// why it must not be reported to the user as the permanent one.
    #[error(
        "refusing to rewrite `{0}`: its sidecar is missing or unreadable, so there is no way \
         to tell which of these lines the op log holds — the next reconcile pass rebuilds it"
    )]
    PageSidecarUnreadable(String),

    /// A guarded writer detected that the `.md` changed after it was
    /// inspected but before it could replace the file.
    ///
    /// Editors do not participate in outl's advisory page lock, so the
    /// writer re-reads immediately before rename and refuses rather than
    /// knowingly overwrite a save that landed during projection work.
    #[error(
        "refusing to rewrite `{0}`: the markdown changed while the projection was being prepared"
    )]
    PageMarkdownChangedDuringProjection(String),

    /// Underlying workspace failure (storage, etc).
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),

    /// I/O while rendering the journal markdown.
    #[error("journal io: {0}")]
    Io(#[from] std::io::Error),

    /// Sidecar (`.outl`) read/write failure when keeping it in sync
    /// with the rendered `.md` projection.
    #[error("sidecar: {0}")]
    Sidecar(#[from] outl_md::sidecar::SidecarError),

    /// `.md` ↔ ops reconcile failure while restoring an undo / redo
    /// snapshot (`history::restore_page_md`).
    #[error("reconcile: {0}")]
    Reconcile(#[from] outl_md::ReconcileError),

    /// An asset upload was refused because the source file exceeds the
    /// configured `[assets] max_bytes` cap. Carries the actual size and
    /// the limit so the UI can tell the user by how much.
    #[error("asset is {size} bytes, over the {limit}-byte limit")]
    AssetTooLarge {
        /// Size of the rejected file, in bytes.
        size: u64,
        /// The configured cap, in bytes.
        limit: u64,
    },

    /// An asset link resolved to a path outside `<workspace>/assets/`
    /// (absolute, `..` traversal, or a different subtree). Rejected
    /// before any file is opened — `.md` arrives from untrusted peers,
    /// so a crafted link must never reach outside the assets dir.
    #[error("invalid asset path `{0}`")]
    InvalidAssetPath(String),

    /// Code-block execution orchestration failed (sidecar IO, op log
    /// apply, `.md` reconcile during the run). Runtime-level failures
    /// (`unknown language`, timeout) come back through the success
    /// payload's `error` field instead — they are user-visible
    /// diagnostics, not bugs.
    #[error("exec: {0}")]
    Exec(String),

    /// An OS "Open With → outl" gesture handed us a file whose
    /// extension is not one [`crate::open_with::SUPPORTED_EXTENSIONS`]
    /// accepts. Refused before the file is read: outl imports text
    /// into an outline, and a `.pdf` turned into a page of mojibake is
    /// worse than a named refusal.
    #[error("`{0}` isn't a text file outl can open — it reads .md, .markdown, .txt and .text")]
    UnsupportedExternalFile(String),

    /// An externally opened file is over
    /// [`crate::open_with::MAX_IMPORT_BYTES`]. The import parses the
    /// whole file synchronously into one block per non-blank line, so an
    /// unbounded file is a hang, not a slow load.
    #[error("`{path}` is over the {limit}-byte limit for opening a file into outl")]
    ExternalFileTooLarge {
        /// The file that was refused.
        path: String,
        /// The cap it exceeded, in bytes.
        limit: u64,
    },

    /// An externally opened file has a text extension but is not valid
    /// UTF-8.
    #[error("`{0}` isn't valid UTF-8 text, so there's nothing to import")]
    ExternalFileNotText(String),

    /// A hundred pages already sit under this file's name, every one
    /// of them imported from a *different* path. Refusing beats
    /// merging into one of them (see `open_with`'s module docs).
    #[error("too many different files already opened as `{0}` — rename one and try again")]
    ExternalFileNameExhausted(String),

    /// An [`crate::open_with::OpenWithTarget::New`] was resolved against a
    /// workspace that has since gained a page at that slug, imported
    /// from a *different* path. The target is stale: importing into it
    /// would merge two files into one page. The caller re-resolves under
    /// the lock it holds and gets `open-in/<stem> 2`.
    #[error("`{0}` was taken by another file while it was being opened — try again")]
    ExternalFileTargetTaken(String),
}
