//! Where every page's `.md` stands relative to the op log.
//!
//! The `tree → .md` direction has two halves, and the split between
//! them is the whole point:
//!
//! - [`survey_page_projections`] — a **read-only** classification of
//!   every page's `.md` against the op log. It selects candidates, and
//!   it is what this module owns.
//! - [`reproject_stale_pages`] — the executor, which lives in `sweep.rs`
//!   and hands every candidate to
//!   [`super::apply_page_md_with_sidecar_if_stale`]. That function
//!   re-asks the verdict itself and is still the authority; nothing on
//!   either side of this split overrides it.
//!
//! ## Why the survey exists at all
//!
//! `outl doctor` had this classification inline and private. So did
//! nothing else, which is why the `tree → .md` direction had no
//! executor: the only code that knew which pages were safe to
//! re-project was a read-only report a human had to run by hand. Ops
//! arriving by sync, or a render-affecting fix landing in the renderer,
//! left a page's `.md` wrong until somebody happened to open it.
//! Measured on a real 2,574-page workspace: **704 stale pages**, 702 of
//! them safe to re-project with zero content lines removed.
//!
//! ## A state is a verdict, never a repair offer
//!
//! Every variant of [`PageProjectionState`] has to be reachable by the
//! writing pass with the same answer, because a listing that promises a
//! repair the writer then refuses is root `CLAUDE.md` invariant 8's
//! "one owner per verdict" broken from the reading side. That is why
//! `NotFound` is routed through the *writer's* own
//! `guard::guard_absent_markdown`, and why a withheld hash whose
//! unlogged set is empty is [`PageProjectionState::HashWithheldButClean`]
//! rather than [`PageProjectionState::Stale`].

mod sweep;

#[cfg(test)]
mod tests;

pub use sweep::{reproject_stale_pages, ReprojectionSweep, UnreadablePage, WithheldPage};

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{file_hash, sidecar_path_for};

use super::guard::{content_lines_missing_from, sidecar_can_answer};
use super::paths::page_md_path;
use super::render::render_page_md_with;
use crate::outline::ChildrenIndex;
use crate::page::{list_all as list_pages, PageMeta};

/// Where one page's `.md` stands relative to the op log.
///
/// The single owner of that classification: `outl doctor`'s read-only
/// listing and the background executor both read it, so a listing can
/// never promise a repair the writing pass then refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageProjectionState {
    /// The bytes on disk are exactly what the tree renders.
    InSync,
    /// The page is in the op log and has no `.md` here at all. Nothing
    /// on disk to lose.
    Absent,
    /// A faithful projection the tree has moved past. `lines_removed`
    /// counts the content lines on disk the new render does **not**
    /// reproduce — what re-projecting this page costs.
    Stale {
        /// Content lines on disk the new render does not reproduce.
        lines_removed: usize,
    },
    /// The `.md` holds content that exists in no op. Frozen in both
    /// directions until `outl reconcile --ahead-of-log` runs.
    AheadOfLog {
        /// How many disk lines exist in no op.
        lines: usize,
        /// One of them, so the message can name what is at risk.
        sample: String,
    },
    /// The `.md` no longer matches its sidecar: an unreconciled external
    /// edit. The `.md → tree` direction owns it.
    PendingExternalEdit,
    /// No sidecar, but the bytes already equal the render — only the
    /// sidecar is missing.
    SidecarMissingButFaithful,
    /// No sidecar and the bytes differ from the render. Nothing
    /// establishes that outl wrote them; `outl reconcile` first.
    SidecarMissingAndDrifted,
    /// A sidecar that cannot answer "does the op log know this line"
    /// (every one written before 0.11 carries `text: ""`). Not a
    /// verdict of safety — a refusal to give one. See
    /// [`sidecar_can_answer`].
    SidecarCannotAnswer,
    /// The `.md` exists and could not be read — a permission error,
    /// non-UTF8 bytes. Deliberately not treated as absent: that is how a
    /// transient I/O error turns into an overwrite.
    ///
    /// It does **not** cover the undownloaded-iCloud case, and claiming
    /// it did was worse than saying nothing: a placeholder answers
    /// `NotFound`, never an I/O error, so it never reached this arm.
    /// [`Self::MarkdownNotHereYet`] is the one that does.
    Unreadable {
        /// The I/O error, verbatim.
        error: String,
    },
    /// The `.md` is not on disk, but something beside it says its bytes
    /// exist and are simply not here yet: an iCloud placeholder sibling
    /// (`.foo.md.icloud`, with the real name absent), or a live `.outl`
    /// sidecar — only ever written next to a `.md` this device
    /// projected, so proof the page existed here.
    ///
    /// Not [`Self::Absent`], which is routed to a write: projecting over
    /// this absence creates a file the real bytes collide with when they
    /// land. The verdict comes from the same guard the writer uses, so
    /// this listing cannot offer what the writer refuses.
    MarkdownNotHereYet {
        /// Why the absence is not an absence.
        reason: String,
    },
    /// `reconcile_md` withheld `last_synced_hash` (invariant 8) and the
    /// unlogged content it was withheld for is no longer on disk.
    ///
    /// A *re-projection* cannot fix it: every downstream gate tests
    /// hash-equality, so the write path stops at the withheld sentinel
    /// and returns "nothing to do" every time. What it needs is a
    /// `.md → tree` reconcile, which restamps the hash — so no repair is
    /// offered for it here. Classifying it as [`Self::Stale`] made the
    /// listing promise a repair the writing pass structurally refuses.
    HashWithheldButClean,
}

/// One page, its `.md`, and where that `.md` stands.
#[derive(Debug, Clone)]
pub struct PageProjection {
    /// The page's root node in the materialized tree.
    pub page_root: NodeId,
    /// The page slug, for messages that name it.
    pub slug: String,
    /// Path of the `.md` this state describes.
    pub path: PathBuf,
    /// Where that `.md` stands relative to the op log.
    pub state: PageProjectionState,
}

/// Classify every page in the workspace.
///
/// `log_damaged` comes from the caller's own op-log health check (only
/// `outl doctor` has one). A torn log replays a truncated tree, which
/// makes every page look like it holds unlogged content — so when it is
/// set, the unlogged-content question stands down and pages report as
/// [`PageProjectionState::Stale`] with their real `lines_removed`. The
/// caller is then responsible for reporting the log, not the pages.
/// Everything without such a check passes `false`, which is the
/// conservative direction: more pages classify as `AheadOfLog` and are
/// left alone.
pub fn survey_page_projections(
    workspace: &Workspace,
    root: &Path,
    log_damaged: bool,
) -> Vec<PageProjection> {
    // **One children index for the whole sweep.** Every page here is
    // rendered (an in-sync page still renders, to prove it is in sync)
    // and almost none is written, so this pass is pure CPU with no I/O
    // to hide behind — the one shape where a whole-workspace map
    // amortises. Per page it would be the wrong call: ~5 ms to build
    // against ~0.5 ms for the subtree-scoped one `render_page_md` uses.
    let children = crate::backlinks_index::build_children_index(workspace);
    list_pages(workspace)
        .into_iter()
        .filter_map(|meta| survey_one(workspace, root, &meta, log_damaged, &children))
        .collect()
}

fn survey_one(
    workspace: &Workspace,
    root: &Path,
    meta: &PageMeta,
    log_damaged: bool,
    children: &ChildrenIndex,
) -> Option<PageProjection> {
    let page_root = meta.id.parse::<ulid::Ulid>().map(NodeId).ok()?;
    let path = page_md_path(root, meta);
    let state = classify(workspace, page_root, &path, log_damaged, children);
    Some(PageProjection {
        page_root,
        slug: meta.slug.clone(),
        path,
        state,
    })
}

fn classify(
    workspace: &Workspace,
    page_root: NodeId,
    path: &Path,
    log_damaged: bool,
    children: &ChildrenIndex,
) -> PageProjectionState {
    let disk = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // **`NotFound` is not the same question as "this page has no
        // `.md`".** An undownloaded iCloud file answers `NotFound` too —
        // the real name does not exist, only `.foo.md.icloud` does — and
        // so does a `.md` that went missing beside a live sidecar.
        // `guard_absent_markdown` is the single owner of that
        // distinction and it is the *writer's* own guard, so the listing
        // and the pass cannot reach different verdicts here.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return match super::guard::guard_absent_markdown(path, &sidecar_path_for(path)) {
                Ok(()) => PageProjectionState::Absent,
                Err(reason) => PageProjectionState::MarkdownNotHereYet {
                    reason: reason.to_string(),
                },
            };
        }
        Err(e) => {
            return PageProjectionState::Unreadable {
                error: e.to_string(),
            }
        }
    };
    let disk_hash = file_hash(&disk);
    let sidecar_path = sidecar_path_for(path);
    // Read once: the hash gate needs `last_synced_hash` and the
    // unlogged-content question needs `blocks`, and nothing holds a lock
    // between them. Two reads describe two revisions.
    let sidecar = outl_md::sidecar::read(&sidecar_path).ok();

    if !sidecar_path.exists() {
        // The one branch that has to render before it can answer.
        return if file_hash(&render_page_md_with(workspace, page_root, children)) == disk_hash {
            PageProjectionState::SidecarMissingButFaithful
        } else {
            PageProjectionState::SidecarMissingAndDrifted
        };
    }

    // The sidecar file is present (checked above) yet unreadable: "I
    // cannot tell", which is not permission to write.
    let Some(sidecar) = sidecar else {
        return PageProjectionState::PendingExternalEdit;
    };
    let faithful = sidecar.last_synced_hash == disk_hash;
    // **The empty hash is not a stale projection, it is a withheld
    // one.** `reconcile_md` writes that sentinel when it read content it
    // could not log (invariant 8). Reading it as an ordinary external
    // edit is how the page the producer flagged becomes the one page no
    // report ever names. A torn op log makes every page look that way,
    // so the sentinel only counts when the log is intact.
    let withheld = !faithful && sidecar.last_synced_hash.is_empty() && !log_damaged;
    if !faithful && !withheld {
        return PageProjectionState::PendingExternalEdit;
    }

    // Rendered once and reused: measuring what a re-projection removes
    // needs the same string.
    let rendered = render_page_md_with(workspace, page_root, children);
    let projection_matches_disk = file_hash(&rendered) == disk_hash;
    // **A withheld page does not get the in-sync shortcut.** Equal bytes
    // mean the tree and the file agree; they say nothing about the
    // sentinel, and returning here is what made the `withheld` branch
    // above dead for the commonest shape of a frozen page — the tree
    // holds every line, the `.md` is a faithful render, and only the
    // hash is held back. Neither `doctor` nor `serve` ever named it.
    if projection_matches_disk && !withheld {
        return PageProjectionState::InSync;
    }

    // The hash proves outl wrote these bytes. It does **not** prove the
    // op log holds them, and the sidecar is what answers that — its
    // blocks are what the log held at the last agreement. Asking the
    // render instead answers "do disk and tree disagree", which every
    // remote edit and every remote delete also answers yes to (#166).
    if log_damaged {
        return PageProjectionState::Stale {
            lines_removed: lines_removed_by(&disk, &rendered),
        };
    }
    if !sidecar_can_answer(&sidecar.blocks) {
        return PageProjectionState::SidecarCannotAnswer;
    }
    let unlogged = content_lines_missing_from(&disk, &sidecar.blocks);
    if let Some(sample) = unlogged.first() {
        return PageProjectionState::AheadOfLog {
            lines: unlogged.len(),
            sample: sample.clone(),
        };
    }
    if withheld {
        // The sentinel outlived what it was withheld for. Only a
        // `.md → tree` reconcile clears it, and the write path here
        // stops at the hash gate — so offering a repair would be a
        // listing promising what the pass refuses.
        return PageProjectionState::HashWithheldButClean;
    }
    PageProjectionState::Stale {
        lines_removed: lines_removed_by(&disk, &rendered),
    }
}

/// How many content lines on disk the render would **not** reproduce.
///
/// Routed through `content_lines_missing_from`, the single owner of
/// "which of these disk lines does the reference not account for". Only
/// the reference differs: the guard asks it of the *sidecar's* blocks
/// ("does the op log know this line"), and this asks it of the blocks
/// the new projection will contain ("will this line survive the write").
///
/// Both questions are correct, and they are not the same one: content a
/// peer legitimately deleted is not unlogged, but it is still content
/// this write removes, and nobody should be asked to authorise that
/// without the number.
fn lines_removed_by(disk: &str, rendered: &str) -> usize {
    let ast = outl_md::parse(rendered);
    let flat = outl_md::matching::flatten(&ast.blocks);
    // The **texts** entry point, because that is all the comparison
    // reads. Wrapping each rendered line in a throwaway `SidecarBlock`
    // minted a ULID and a SHA-256 per block to feed a function that
    // looks at neither — tens of thousands of both per sweep on a
    // 2.5k-page graph, every 30 seconds. Same verdict by construction
    // (`content_lines_missing_from` is this call with `.text` applied),
    // and pinned as such by
    // `the_texts_entry_point_agrees_with_the_block_one`.
    // Plus the YAML frontmatter fence, which travels the page-property
    // channel and so is invisible to a block-text comparison. The writing
    // pass refuses a page whose render would drop it
    // (`guard::frontmatter_loss_error`), so counting it here is what keeps
    // this listing from promising a repair that pass then refuses — root
    // `CLAUDE.md` invariant 8's "one owner per verdict", applied to the
    // second channel.
    outl_md::unlogged::content_lines_missing_from_texts(disk, flat.iter().map(|b| b.text)).len()
        + outl_md::unlogged::frontmatter_lines_missing_from(disk, rendered)
}
