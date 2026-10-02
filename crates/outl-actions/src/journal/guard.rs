//! "May these bytes be overwritten?" — the verdicts, phrased once.
//!
//! Root `CLAUDE.md` invariant 8 lives here: a sidecar hash match proves
//! outl wrote a `.md` last, never that the op log holds what it says. So
//! before any re-projection, three separate questions are asked, each on
//! its own channel and each with exactly one owner:
//!
//! - [`unlogged_content_error`] — does the file hold **block text** that
//!   exists in no op? Asked of the *sidecar's* blocks, never of a fresh
//!   render: the sidecar is what the log held when the two last agreed,
//!   so it answers "does the log know this line", while a render answers
//!   "do disk and tree disagree" — which is also yes for every remote
//!   edit, remote delete and reorder (#166, reintroduced once by exactly
//!   that substitution).
//! - [`frontmatter_loss_error`] — would the write delete the page's YAML
//!   fence? A second channel: the fence rides `Op::SetProp` on the page
//!   root, so a block list cannot answer for it (#281).
//! - [`guard_absent_markdown`] — is an absent `.md` really an absent
//!   page, or a file whose bytes are not on this device yet?
//!
//! `super::survey` routes the third one so that a read-only listing and
//! the writing pass cannot reach different verdicts.
//!
//! ## Who asks, and who does not
//!
//! `super::apply` holds six public writers, and **two of them are
//! gates** — this is the census, because the sentence it replaces ("both
//! write gates run all three") was true and still read as one. A writer
//! the doc does not name is a writer a reader assumes is one of the ones
//! it does, and that is how `apply_page_md_with_sidecar_rendered`
//! shipped public, exported and guard-free next to this paragraph, with
//! no caller in the repo: during the #281 upgrade window — an op log
//! carrying a page's frontmatter fence as bullets, a render that does
//! not — that call is the write that deletes the fence.
//!
//! The gates, which run all three checks:
//!
//! - `apply_page_md_with_sidecar_guarded` — after a mutation. Step 5 of
//!   `crate::commit::commit_page`. It writes *because there is a real
//!   mutation to project*, so it refuses only what would delete bytes.
//! - `apply_page_md_with_sidecar_if_stale` — before a read, and on
//!   `outl serve`'s sweep. It writes *only to catch a `.md` up with the
//!   tree*, so it also declines every state it cannot vouch for.
//!
//! And the writers that ask nothing, each because its caller has already
//! established there is nothing on disk to lose:
//!
//! - `apply_page_md` — renders to a `.md` with no sidecar at all, so
//!   there is no reference to ask. No caller in the repo.
//! - `apply_page_md_with_sidecar` — the unconditional projection. Test
//!   fixtures only; every production path takes a gate.
//! - `apply_page_md_with_sidecar_if_absent` — writes only when the `.md`
//!   is absent. It does **not** ask `guard_absent_markdown`, so it can
//!   still write over bytes that are merely not downloaded yet; the
//!   `_if_stale` gate subsumes it and is what the read paths call.
//! - `apply_all_pages_md` — not a writer of its own, a sweep over the
//!   post-mutation gate. Refusals come back per page in
//!   `super::apply::ProjectionSweep::failures`.
//!
//! `crates/outl-actions/tests/projection_writer_gates.rs` is what keeps
//! this list honest: a new `pub fn` in `super::apply` does not pass until
//! its verdict is written down.

use std::path::{Path, PathBuf};

use outl_md::sidecar::SidecarBlock;

use crate::error::ActionError;

/// `Some(error)` when re-projecting over `disk` would delete content the
/// op log cannot account for.
///
/// The one place that phrases this verdict as an `ActionError`, so the
/// two callers cannot drift on what the message says or which fields it
/// carries.
///
/// **`None` covers two different situations on purpose**, and the caller
/// decides what each one means:
///
/// - nothing is at risk;
/// - the sidecar cannot answer at all (every one written before 0.11).
///
/// [`apply_page_md_with_sidecar_guarded`] treats both as "go ahead" —
/// there is a real mutation to project and refusing every pre-0.11 page
/// would freeze the app. [`apply_page_md_with_sidecar_if_stale`] asks
/// [`sidecar_can_vouch_for`] *first* and declines the second case, because
/// re-projecting a page it cannot vouch for is how bytes go missing.
/// Reading one policy as the other is the bug this whole module guards.
///
/// It asks `sidecar_can_vouch_for` and not [`sidecar_can_answer`]: the
/// second case is "could not check **and** there is something to check",
/// and conflating it with "could not check" froze every page holding
/// only bare bullets (issue #332).
pub(super) fn unlogged_content_error(
    path: &Path,
    disk: &str,
    blocks: &[SidecarBlock],
) -> Option<ActionError> {
    if !sidecar_can_answer(blocks) {
        return None;
    }
    let unlogged = content_lines_missing_from(disk, blocks);
    let sample = unlogged.first()?;
    Some(ActionError::PageMarkdownAheadOfLog {
        path: path.display().to_string(),
        lines: unlogged.len(),
        sample: format!("{sample:?}"),
    })
}

/// `Some(error)` when writing `rendered` over `disk` would delete the
/// page's YAML frontmatter fence.
///
/// The sibling of [`unlogged_content_error`], and a separate function
/// because it is a separate **question about a separate channel**: the
/// fence lives in the op log as one `Op::SetProp` on the page root, so a
/// block list cannot answer for it and `content_lines_missing_from` skips
/// it. The references here are the render, which carries the fence exactly
/// when the log knows it, and the sidecar's `last_synced_hash`, which says
/// whether the fence on disk is the one the log held at the last agreement.
///
/// Same error and same recovery as its sibling on purpose —
/// `outl reconcile --ahead-of-log` re-reads the fence into the log, which
/// is precisely what clears this. `outl_md::frontmatter_lines_missing_from`
/// owns the narrowness (a *differing* fence over bytes outl wrote last is a
/// peer edit, not a loss); see its doc before widening this.
pub(super) fn frontmatter_loss_error(
    path: &Path,
    disk: &str,
    rendered: &str,
    last_synced_hash: &str,
) -> Option<ActionError> {
    let lines = outl_md::unlogged::frontmatter_lines_missing_from(disk, rendered, last_synced_hash);
    if lines == 0 {
        return None;
    }
    Some(ActionError::PageMarkdownAheadOfLog {
        path: path.display().to_string(),
        lines,
        sample: "\"--- (YAML frontmatter)\"".to_string(),
    })
}
/// The content lines in `disk` that **no block the op log knows** can
/// account for.
///
/// Re-exported from [`outl_md::unlogged`], which is where it lives so
/// that `reconcile_md` — the *producer* of the unlogged state, one
/// crate down — can ask the same question before it advances
/// `last_synced_hash`. Every existing `outl_actions::` path still
/// resolves through this re-export.
///
/// Public because `outl doctor` must reach the *same* verdict in its
/// read-only listing that `--repair` reaches when it writes; two opinions
/// about which pages are safe is how a listing promises a repair the pass
/// then silently skips.
pub use outl_md::unlogged::content_lines_missing_from;

pub use outl_md::unlogged::sidecar_can_answer;
pub use outl_md::unlogged::sidecar_can_vouch_for;
/// Decide whether an absent `.md` really means "this page does not
/// exist yet".
///
/// **Both writers that can create a `.md` ask this**, and for a while
/// only one did. [`mutate_page_md`] rewrites the parsed AST, so an
/// unguarded absence recreated the page as a single block;
/// [`apply_page_md_with_sidecar_if_stale`] renders from the op log, so
/// an unguarded absence writes a file the bytes still arriving then
/// collide with. Different damage, same misreading of `NotFound`, and
/// the second one runs unattended inside `outl serve`'s projection
/// sweep.
///
/// Two ways an absence is not an absence, both of which used to end in
/// a write:
///
/// - **A sidecar is present.** The sidecar is only ever written next to
///   a `.md` this device projected, so its existence is proof the page
///   existed. A missing `.md` beside it is a lost file — a half-finished
///   sync, an editor that deleted-and-recreated, a user emptying a
///   folder — never a new page. Rewriting over it converts a recoverable
///   loss (the `.md` is a projection; the op log still has the content)
///   into an unrecoverable one, because the rewrite rebuilds the sidecar
///   from one block and the next reconcile emits `Move`→`TRASH_ROOT` for
///   every id it can no longer find.
/// - **An iCloud placeholder sibling is present.** On iOS and on legacy
///   iCloud Drive, a file whose bytes have not been downloaded is
///   `.foo.md.icloud` and *the real name does not exist* — so the read
///   is `NotFound`, not the permission/IO error the `read_for_rewrite`
///   contract assumes. Same outcome, on a file that is not lost at all
///   and will materialise on its own.
pub(super) fn guard_absent_markdown(
    md_path: &Path,
    sidecar_path: &Path,
) -> Result<(), ActionError> {
    // Re-check existence rather than trusting the empty read: a page
    // that legitimately renders to an empty string is not absent.
    if md_path.exists() {
        return Ok(());
    }
    if icloud_placeholder(md_path).is_some() {
        return Err(ActionError::PageMarkdownNotDownloaded(
            md_path.display().to_string(),
        ));
    }
    if sidecar_path.exists() {
        return Err(ActionError::PageMarkdownVanished(
            md_path.display().to_string(),
        ));
    }
    Ok(())
}

/// The iCloud placeholder that stands in for `md_path` while its bytes
/// are still in the cloud: `pages/foo.md` → `pages/.foo.md.icloud`.
fn icloud_placeholder(md_path: &Path) -> Option<PathBuf> {
    let name = md_path.file_name()?.to_str()?;
    let placeholder = md_path.with_file_name(format!(".{name}.icloud"));
    placeholder.exists().then_some(placeholder)
}
