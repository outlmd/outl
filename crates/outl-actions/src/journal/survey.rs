//! The `tree → .md` direction, surveyed and swept.
//!
//! Two things live here, and the split between them is the whole point:
//!
//! - [`survey_page_projections`] — a **read-only** classification of
//!   every page's `.md` against the op log. It selects candidates.
//! - [`reproject_stale_pages`] — the executor, which hands every
//!   candidate to [`super::apply_page_md_with_sidecar_if_stale`]. That
//!   function re-asks the verdict itself and is still the authority;
//!   nothing here overrides it.
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
//! ## What the executor will and will not write
//!
//! It writes a page whose re-projection removes **nothing** from disk,
//! and only that. A write that would remove content lines — a peer
//! genuinely deleted blocks — is [`ReprojectionSweep::withheld`] and
//! belongs to `outl doctor --repair`, which backs every file up first
//! and applies volume ceilings. A background pass that deletes is a
//! background pass that needs an undo, and the command with the undo
//! already exists.
//!
//! That gate is also what makes a **torn op log** safe here without this
//! module knowing anything about op-log health: a truncated replay
//! renders *less* than the disk holds, so every page it touches reports
//! `lines_removed > 0` and is withheld. (Root `CLAUDE.md` invariant 8's
//! precedence order — a damaged log is reported as a damaged log — stays
//! with `outl doctor`, which is the surface that can diagnose it.)
//!
//! ## Refusals are returned, never swallowed
//!
//! A page whose `.md` holds content the op log never saw is frozen in
//! both directions until `outl reconcile --ahead-of-log` runs. Every
//! refusal lands in [`ReprojectionSweep::refused`] with its
//! [`crate::ActionError`] so the caller can name the page — invariant 8,
//! and [`docs/clients.md` → Surfacing a page that stopped
//! syncing](../../../../docs/clients.md#surfacing-a-page-that-stopped-syncing).
//!
//! ## A skip is not automatically silent
//!
//! Most states this pass declines to write belong to someone who *will*
//! act: an unreconciled external edit and a withheld hash are both the
//! `.md → tree` watcher's next job, and a sidecar that cannot vouch for
//! its page is named by `outl doctor`. Those are quiet on purpose.
//!
//! A `.md` the pass could not read is not one of them — nothing else is
//! going to notice it — so it lands in [`ReprojectionSweep::unreadable`]
//! with the reason, alongside a page whose bytes are simply not on this
//! device yet.

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{file_hash, sidecar_path_for};

use super::apply::{
    apply_page_md_with_sidecar_if_stale, content_lines_missing_from, sidecar_can_answer,
    ProjectionFailure,
};
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
            return match super::apply::guard_absent_markdown(path, &sidecar_path_for(path)) {
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
    outl_md::unlogged::content_lines_missing_from_texts(disk, flat.iter().map(|b| b.text)).len()
}

/// One page the sweep could not look at.
#[derive(Debug, Clone)]
pub struct UnreadablePage {
    /// The `.md` that was skipped.
    pub path: PathBuf,
    /// Why its bytes could not be read — an I/O error, or the evidence
    /// that they are simply not on this device yet.
    pub reason: String,
}

/// One page the executor deliberately did not write.
#[derive(Debug, Clone)]
pub struct WithheldPage {
    /// The `.md` left untouched.
    pub path: PathBuf,
    /// Content lines the write would have removed from disk.
    pub lines_removed: usize,
}

/// What one [`reproject_stale_pages`] pass did.
#[derive(Debug, Default)]
pub struct ReprojectionSweep {
    /// Pages classified — the denominator for every count below.
    pub surveyed: usize,
    /// `.md` paths re-projected from the op log.
    pub written: Vec<PathBuf>,
    /// Stale pages whose write would have removed content. Left for
    /// `outl doctor --repair`, which backs up first.
    pub withheld: Vec<WithheldPage>,
    /// Pages the write guard refused. Each one has **stopped syncing**
    /// and the caller owes the user its name (invariant 8).
    pub refused: Vec<ProjectionFailure>,
    /// Pages whose `.md` the sweep could not read: an I/O error, or
    /// bytes that are not on this device yet (an iCloud placeholder, a
    /// `.md` gone from beside its sidecar). Each is a skip the user
    /// cannot otherwise see — the failure invariant 8 exists to prevent.
    pub unreadable: Vec<UnreadablePage>,
    /// Selected by the survey and declined by the guard anyway: the page
    /// changed between the two reads, or the two disagree. Never silent
    /// — a skip nobody can see is the failure invariant 8 exists to
    /// prevent.
    pub declined: Vec<PathBuf>,
}

/// Re-project every page whose `.md` is a stale projection the op log
/// has moved past — and **only** where that write removes nothing.
///
/// This is the `tree → .md` executor. The `.md → tree` direction has had
/// one for as long as `outl serve` has existed; this side had none, so
/// ops arriving by sync left the `.md` wrong until a human opened the
/// page.
///
/// The survey selects; [`apply_page_md_with_sidecar_if_stale`] decides.
/// Calling it again rather than trusting the survey is deliberate: it
/// re-reads the file under the page lock, so a page that changed between
/// the two is refused rather than raced, and there is exactly one owner
/// of "may this `.md` be overwritten".
pub fn reproject_stale_pages(workspace: &Workspace, root: &Path) -> ReprojectionSweep {
    let mut sweep = ReprojectionSweep::default();
    for page in survey_page_projections(workspace, root, false) {
        sweep.surveyed += 1;
        match page.state {
            // Someone else's job, and a job that gets done: the
            // `.md → tree` watcher next door reconciles an external edit
            // and restamps a withheld hash, and `outl doctor` names a
            // sidecar that cannot vouch for its page. Nothing here is a
            // page that silently stopped converging.
            PageProjectionState::InSync
            | PageProjectionState::SidecarMissingButFaithful
            | PageProjectionState::PendingExternalEdit
            | PageProjectionState::SidecarMissingAndDrifted
            | PageProjectionState::SidecarCannotAnswer
            | PageProjectionState::HashWithheldButClean => {}
            // A `.md` the pass could not look at **is** such a page, and
            // it used to fold into the arm above — so `outl serve` said
            // nothing at all about it.
            PageProjectionState::Unreadable { error } => sweep.unreadable.push(UnreadablePage {
                path: page.path,
                reason: error,
            }),
            PageProjectionState::MarkdownNotHereYet { reason } => {
                sweep.unreadable.push(UnreadablePage {
                    path: page.path,
                    reason,
                })
            }
            PageProjectionState::Stale { lines_removed } if lines_removed > 0 => {
                sweep.withheld.push(WithheldPage {
                    path: page.path,
                    lines_removed,
                });
            }
            // `AheadOfLog` is handed to the guard too, precisely so the
            // refusal that reaches the caller is the guard's own error
            // and not a second opinion phrased here.
            PageProjectionState::Absent
            | PageProjectionState::Stale { .. }
            | PageProjectionState::AheadOfLog { .. } => {
                match apply_page_md_with_sidecar_if_stale(workspace, root, page.page_root) {
                    Ok(Some(path)) => sweep.written.push(path),
                    Ok(None) => sweep.declined.push(page.path),
                    Err(error) => sweep.refused.push(ProjectionFailure {
                        path: page.path,
                        error,
                    }),
                }
            }
        }
    }
    sweep
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::block::append_block;
    use crate::journal::{
        apply_page_md_with_sidecar, page_md_path, reproject_stale_pages, survey_page_projections,
        PageProjectionState,
    };
    use crate::page::{open_or_create, page_meta, PageKind};
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::workspace::Workspace;
    use tempfile::TempDir;

    /// A workspace with one projected page holding `lines`, plus the
    /// path of its `.md`.
    fn projected_page(lines: &[&str]) -> (TempDir, Workspace, HlcGenerator, NodeId, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        for line in lines {
            append_block(&mut ws, &hlc, Some(page), Some(line)).unwrap();
        }
        apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
        let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
        (tmp, ws, hlc, page, md_path)
    }

    /// Rewrite the sidecar the way every pre-0.11 build did: block ids
    /// and hashes, no text.
    fn blank_sidecar_texts(md_path: &Path) {
        let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        for block in &mut sc.blocks {
            block.text = String::new();
        }
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    }

    /// The sentinel `reconcile_md` writes when it read content it could
    /// not turn into ops: an empty `last_synced_hash`, block entries
    /// untouched.
    fn withhold_sidecar_hash(md_path: &Path) {
        let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        sc.last_synced_hash = String::new();
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    }

    fn restamp_sidecar_as_faithful(md_path: &Path) {
        let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
        let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
        let disk = std::fs::read_to_string(md_path).unwrap();
        sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
        outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    }

    #[test]
    fn survey_reports_a_page_whose_tree_ran_ahead_as_stale() {
        let (tmp, mut ws, hlc, page, _md) = projected_page(&["first"]);
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        assert_eq!(
            found.state,
            PageProjectionState::Stale { lines_removed: 0 },
            "an append-only tree advance removes nothing from disk"
        );
    }

    #[test]
    fn survey_counts_the_lines_a_reprojection_would_remove() {
        let (tmp, mut ws, hlc, page, _md) = projected_page(&["keep", "peer deleted this"]);
        let doomed = crate::tree::children_of(&ws, page)[1].0;
        crate::block::delete(&mut ws, &hlc, doomed).unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        assert_eq!(
            found.state,
            PageProjectionState::Stale { lines_removed: 1 },
            "a peer delete is a stale projection that removes one disk line"
        );
    }

    #[test]
    fn survey_reports_a_page_holding_unlogged_content_as_ahead_of_the_log() {
        let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
        std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
        restamp_sidecar_as_faithful(&md_path);

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        match &found.state {
            PageProjectionState::AheadOfLog { lines, sample } => {
                assert_eq!(*lines, 1);
                assert!(sample.contains("only ever on disk"), "got {sample:?}");
            }
            other => panic!("expected AheadOfLog, got {other:?}"),
        }
    }

    #[test]
    fn the_sweep_reprojects_a_stale_page_that_loses_nothing() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert_eq!(sweep.written, vec![md_path.clone()]);
        assert!(std::fs::read_to_string(&md_path)
            .unwrap()
            .contains("synced-in"));
    }

    #[test]
    fn the_sweep_withholds_a_page_whose_reprojection_would_remove_content() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["keep", "peer deleted this"]);
        let doomed = crate::tree::children_of(&ws, page)[1].0;
        crate::block::delete(&mut ws, &hlc, doomed).unwrap();
        let before = std::fs::read_to_string(&md_path).unwrap();

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert!(sweep.written.is_empty(), "nothing may be written");
        assert_eq!(sweep.withheld.len(), 1);
        assert_eq!(sweep.withheld[0].lines_removed, 1);
        assert_eq!(
            std::fs::read_to_string(&md_path).unwrap(),
            before,
            "a content-removing write belongs to `outl doctor --repair`, not to a \
             background pass"
        );
    }

    #[test]
    fn the_sweep_surfaces_a_page_that_stopped_syncing_as_a_refusal() {
        let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
        std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
        restamp_sidecar_as_faithful(&md_path);

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert!(sweep.written.is_empty());
        assert_eq!(sweep.refused.len(), 1, "a refusal has to reach the user");
        assert_eq!(sweep.refused[0].path, md_path);
        assert!(matches!(
            sweep.refused[0].error,
            crate::ActionError::PageMarkdownAheadOfLog { .. }
        ));
        let _ = page;
    }

    #[test]
    fn the_sweep_leaves_a_page_with_a_pending_external_edit_alone() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
        // A hand edit nobody reconciled yet: the sidecar hash no longer
        // matches the file.
        std::fs::write(&md_path, "- first\n- typed by hand\n").unwrap();
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert!(sweep.written.is_empty());
        assert_eq!(
            std::fs::read_to_string(&md_path).unwrap(),
            "- first\n- typed by hand\n",
            "`.md → tree` owns an unreconciled edit; the projection pass must not clobber it"
        );
    }

    /// Every sidecar written before 0.11 carries `text: ""`, so it can
    /// say which blocks a page had and nothing about what they said.
    /// That is **not** "nothing at risk" — it is "I cannot tell", and a
    /// background pass that read the two as the same thing would
    /// overwrite exactly the pages nobody can vouch for.
    #[test]
    fn the_sweep_never_writes_a_page_whose_sidecar_cannot_answer() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
        blank_sidecar_texts(&md_path);
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();
        let before = std::fs::read_to_string(&md_path).unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);
        assert_eq!(
            survey.iter().find(|p| p.page_root == page).unwrap().state,
            PageProjectionState::SidecarCannotAnswer
        );

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert!(sweep.written.is_empty());
        assert_eq!(std::fs::read_to_string(&md_path).unwrap(), before);
    }

    /// An undownloaded iCloud file reads as `NotFound` — the real name
    /// does not exist, only `.notes.md.icloud` does — and the survey
    /// mapped `NotFound` straight to `Absent`, which is routed to a
    /// write.
    #[test]
    fn survey_does_not_call_an_undownloaded_page_absent() {
        let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
        std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path)).unwrap();
        std::fs::remove_file(&md_path).unwrap();
        std::fs::write(md_path.with_file_name(".notes.md.icloud"), "").unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        assert!(
            !matches!(found.state, PageProjectionState::Absent),
            "a file iCloud has not fetched yet is not an absent page, got {:?}",
            found.state
        );
        let sweep = reproject_stale_pages(&ws, tmp.path());
        assert!(sweep.written.is_empty());
        assert!(!md_path.exists(), "nothing may be written over the page");
    }

    /// The same absence, with the evidence that actually survives iCloud:
    /// the placeholder is a dotfile (dropped in cross-device sync), the
    /// `.outl` sidecar is not. A sidecar is only ever written beside a
    /// `.md` this device projected, so it is proof the page existed here.
    #[test]
    fn survey_does_not_call_a_vanished_md_absent() {
        let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
        std::fs::remove_file(&md_path).unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        assert!(
            !matches!(found.state, PageProjectionState::Absent),
            "a missing .md beside a live sidecar is a lost file, got {:?}",
            found.state
        );
        let sweep = reproject_stale_pages(&ws, tmp.path());
        assert!(sweep.written.is_empty());
        assert!(!md_path.exists(), "nothing may be written over the page");
    }

    /// `ReprojectionSweep::declined`'s own doc: "a skip nobody can see is
    /// the failure invariant 8 exists to prevent." An unreadable `.md`
    /// folded into the same empty arm as `InSync` and the sweep had no
    /// field for it, so `outl serve` said nothing at all about a page it
    /// could not read.
    #[test]
    fn the_sweep_names_a_page_it_could_not_read() {
        let (tmp, ws, _hlc, _page, md_path) = projected_page(&["first"]);
        std::fs::write(&md_path, [0xff, 0xfe, 0x00]).unwrap();

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert!(sweep.written.is_empty());
        assert_eq!(
            sweep.unreadable.len(),
            1,
            "a page the sweep could not read has to be nameable"
        );
        assert_eq!(sweep.unreadable[0].path, md_path);
        assert!(
            !sweep.unreadable[0].reason.is_empty(),
            "the reason is what the daemon prints"
        );
    }

    /// A withheld hash over a `.md` that happens to equal the render.
    ///
    /// `reconcile_md` writes `last_synced_hash = ""` when it read content
    /// it could not log. The `InSync` shortcut returned before the
    /// unlogged question was ever asked, so the `withheld` branch above
    /// was dead for the *dominant* shape: the page is frozen in both
    /// directions and neither `doctor` nor `serve` ever names it.
    #[test]
    fn survey_reports_a_withheld_hash_page_whose_render_matches_disk() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
        append_block(&mut ws, &hlc, Some(page), Some("only ever on disk")).unwrap();
        // Disk equals the render, so the hash gate below it would read
        // `InSync` — but the sidecar still holds only the logged block.
        let rendered = crate::journal::render_page_md(&ws, page);
        std::fs::write(&md_path, &rendered).unwrap();
        withhold_sidecar_hash(&md_path);

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        match &found.state {
            PageProjectionState::AheadOfLog { lines, sample } => {
                assert_eq!(*lines, 1);
                assert!(sample.contains("only ever on disk"), "got {sample:?}");
            }
            other => panic!("expected AheadOfLog, got {other:?}"),
        }
    }

    /// The mirror, and the listing-promises-what-the-writer-refuses shape
    /// invariant 8 names: a withheld hash whose unlogged set is now
    /// empty classified as `Stale` — an offered repair — while
    /// `_if_stale` stops at `last_synced_hash != disk_hash` and returns
    /// `Ok(None)` every time. The user was then told the file "changed
    /// underneath", which is not true either.
    ///
    /// It needs a `.md → tree` reconcile to restamp the hash, not a
    /// re-projection, so no repair is offered and nothing is declined.
    #[test]
    fn survey_offers_no_repair_for_a_withheld_hash_with_nothing_unlogged() {
        let (tmp, mut ws, hlc, page, md_path) = projected_page(&["first"]);
        withhold_sidecar_hash(&md_path);
        // The tree moves ahead, so the render differs from disk: the
        // exact input that used to read as a repairable `Stale`.
        append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

        let survey = survey_page_projections(&ws, tmp.path(), false);

        let found = survey.iter().find(|p| p.page_root == page).unwrap();
        assert!(
            !matches!(found.state, PageProjectionState::Stale { .. }),
            "a withheld hash cannot be repaired by a re-projection, got {:?}",
            found.state
        );

        let sweep = reproject_stale_pages(&ws, tmp.path());
        assert!(sweep.written.is_empty());
        assert!(
            sweep.declined.is_empty(),
            "a state the writer structurally refuses must not be offered to it"
        );
    }

    #[test]
    fn the_sweep_projects_a_page_that_has_no_md_on_disk_at_all() {
        let (tmp, ws, _hlc, page, md_path) = projected_page(&["first"]);
        std::fs::remove_file(&md_path).unwrap();
        std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path)).unwrap();

        let sweep = reproject_stale_pages(&ws, tmp.path());

        assert_eq!(sweep.written, vec![md_path.clone()]);
        assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));
        let _ = page;
    }
}
