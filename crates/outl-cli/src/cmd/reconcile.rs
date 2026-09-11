//! `outl reconcile` — the `.md → tree` direction.
//!
//! Two modes, both explicit:
//!
//! - **No flags:** read-only listing of `.outl/orphans.log`.
//! - **`--ahead-of-log`:** reconcile the pages whose `.md` holds content
//!   that exists in no op, bypassing the sidecar hash gate.
//!
//! ## Why the second mode has to exist
//!
//! A page can end up hash-faithful (its sidecar agrees with the bytes on
//! disk) while carrying content the op log never saw — see
//! [RFC 0210](../../../../docs/rfcs/0210-md-content-outside-op-log.md) and
//! issue #210. Fixing the parser that produced that state is necessary
//! but not sufficient: `needs_reconcile` compares the sidecar hash
//! against the file, the sidecar already carries the hash of the file
//! *with* the content, so the page reads as in-sync and the ordinary
//! reconcile never looks at it. Measured on a 2,560-page workspace:
//! `serve --once` applied **0 ops** to 233 such pages.
//!
//! So the content needs a path that ignores the hash and reconciles
//! anyway. That path must stay opt-in, because it emits ops for content
//! the log has never seen — a deliberate write, not a repair.
//!
//! ## Ordering that matters
//!
//! Run this only on a build whose parser preserves the content. A
//! reconcile against a parser that still discards prose after a block
//! property emits the **truncated** text as `Op::Edit`, making the loss
//! permanent in the op log — the one place it currently is not.

use crate::workspace_layout::Paths;
use crate::ws;
use anyhow::{Context, Result};
use outl_md::matching::guard::OrphanGuard;
use std::fs;
use std::path::Path;

/// Run the `reconcile` subcommand.
///
/// `allow_bulk_delete` is the one place in the binary that reaches
/// [`OrphanGuard::Disabled`]. The guard refuses a pass that would trash
/// most of a page, and the refusal is only defensible while there is a
/// way to say the deletion was meant — a guard with no escape hatch is a
/// wall (root `CLAUDE.md` invariant 9).
pub fn run(path: &Path, ahead_of_log: bool, allow_bulk_delete: bool) -> Result<()> {
    let guard = if allow_bulk_delete {
        OrphanGuard::Disabled
    } else {
        OrphanGuard::Enforced
    };
    if ahead_of_log {
        return run_ahead_of_log(path, guard);
    }
    if allow_bulk_delete {
        return run_allow_bulk_delete(path);
    }
    list_orphans(path)
}

/// Reconcile the pages an ordinary pass refuses because the deletion is
/// too large, with the volume guard turned off.
///
/// **This mode has to exist separately from `--ahead-of-log`, and the
/// reason is that the two select opposite sets.**
/// `--ahead-of-log` visits pages where `content_lines_missing_from > 0`
/// — pages whose `.md` holds *more* than the log. A page the guard
/// refused is the mirror: its `.md` holds *less*, every line on disk is
/// accounted for, and `missing` is zero, so it never appeared in that
/// list. Wiring the flag only into `--ahead-of-log` therefore made it
/// unreachable for every page it existed to unblock, which is the
/// "guard with no escape hatch is a wall" failure root `CLAUDE.md`
/// invariant 9 names — arrived at through the door marked "fixed".
///
/// The selection here is deliberately the plain one: any page whose
/// `.md` no longer matches its sidecar hash, i.e. exactly what an
/// ordinary reconcile would look at. The guard is what changes, not the
/// scope.
fn run_allow_bulk_delete(path: &Path) -> Result<()> {
    let mut ctx = ws::open(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let root = ctx.root.clone();
    let orphan_log = outl_actions::sync::orphans_log_path(&root);

    let scan = collect_stale(&ctx.workspace, &root);
    let stale = &scan.picked;
    if stale.is_empty() {
        println!(
            "no page has an unreconciled external edit ({} page(s) judged)",
            scan.judged
        );
        print_unjudged(&scan.skipped);
        return Ok(());
    }

    println!(
        "{} page(s) have an unreconciled `.md`; reconciling with the \
         orphan-volume guard OFF (deletions apply at any size):",
        stale.len()
    );
    println!();

    let mut ops_total = 0usize;
    let mut failed = 0usize;
    for (md_path, slug) in stale {
        match outl_md::reconcile_md_with_guard(
            &mut ctx.workspace,
            &ctx.hlc,
            md_path,
            Some(orphan_log.as_path()),
            &OrphanGuard::Disabled,
        ) {
            Ok(report) => {
                ops_total += report.ops_applied;
                if report.orphans > 0 {
                    println!(
                        "  {:>4} op(s)  {slug} — {} block(s) moved to the trash",
                        report.ops_applied, report.orphans
                    );
                } else {
                    println!("  {:>4} op(s)  {slug}", report.ops_applied);
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("  FAILED    {slug} — {e}");
            }
        }
    }

    println!();
    println!(
        "reconciled {} page(s), {ops_total} op(s) applied, {failed} failed",
        stale.len() - failed
    );
    println!("Deleted blocks are in the trash, not gone — `outl doctor` lists them.");
    print_unjudged(&scan.skipped);
    if failed > 0 {
        anyhow::bail!("{failed} page(s) failed to reconcile");
    }
    Ok(())
}

/// Pages whose `.md` no longer matches the hash its sidecar recorded —
/// an external edit the `.md → tree` direction has not taken in yet.
fn collect_stale(
    ws: &outl_core::workspace::Workspace,
    root: &Path,
) -> Scan<(std::path::PathBuf, String)> {
    let mut scan = scan_pages(ws, root, |meta, md_path, disk, sidecar| {
        if sidecar.last_synced_hash == outl_md::sidecar::file_hash(disk) {
            return Verdict::Clean;
        }
        Verdict::Take((md_path.to_path_buf(), meta.slug.clone()))
    });
    scan.picked.sort_by(|a, b| a.1.cmp(&b.1));
    scan
}

/// The original read-only listing.
fn list_orphans(path: &Path) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());
    if !paths.orphans.exists() {
        println!("no orphans recorded");
        return Ok(());
    }
    let text = fs::read_to_string(&paths.orphans)
        .with_context(|| format!("reading {}", paths.orphans.display()))?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        println!("no orphans recorded");
        return Ok(());
    }
    println!("{} orphan(s) pending manual resolution:", lines.len());
    for line in &lines {
        println!("  {line}");
    }
    println!();
    println!("Interactive resolution in the TUI is not yet available.");
    Ok(())
}

/// A page whose `.md` carries content the op log does not have.
///
/// No `page_root` here on purpose: `reconcile_md` re-derives the page
/// identity from the `.md` path and its sidecar, so carrying an id we
/// resolved a moment earlier would just be a second opinion about which
/// page this is.
struct AheadPage {
    md_path: std::path::PathBuf,
    slug: String,
    /// Content lines present on disk and absent from the render.
    missing: usize,
}

/// Find every page whose `.md` runs ahead of the log, then reconcile it.
///
/// The detection is the same `outl_actions::content_lines_missing_from`
/// the doctor and the re-projection guard use — one owner for the verdict,
/// so this command can never disagree with what `doctor` reported.
fn run_ahead_of_log(path: &Path, guard: OrphanGuard) -> Result<()> {
    let mut ctx = ws::open(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let root = ctx.root.clone();
    let orphan_log = outl_actions::sync::orphans_log_path(&root);

    let scan = collect_ahead(&ctx.workspace, &root);
    let ahead = &scan.picked;
    if ahead.is_empty() {
        println!(
            "no page holds content outside the op log ({} page(s) judged)",
            scan.judged
        );
        print_unjudged(&scan.skipped);
        return Ok(());
    }

    let total_lines: usize = ahead.iter().map(|p| p.missing).sum();
    println!(
        "{} page(s) hold {total_lines} line(s) of content that exist in no op.",
        ahead.len()
    );
    println!("Reconciling each one (this writes ops for that content):");
    println!();

    let mut ops_total = 0usize;
    let mut failed = 0usize;
    for page in ahead {
        // `reconcile_md` short-circuits on the recorded hash, which is
        // exactly the state these pages are in — that is why the ordinary
        // reconcile skips them.
        if let Err(e) = invalidate_synced_hash(&page.md_path) {
            failed += 1;
            eprintln!(
                "  FAILED    {} — could not clear sidecar hash: {e}",
                page.slug
            );
            continue;
        }
        match outl_md::reconcile_md_with_guard(
            &mut ctx.workspace,
            &ctx.hlc,
            &page.md_path,
            Some(orphan_log.as_path()),
            &guard,
        ) {
            Ok(report) => {
                ops_total += report.ops_applied;
                println!(
                    "  {:>4} op(s)  {} ({} line(s) were outside the log)",
                    report.ops_applied, page.slug, page.missing
                );
            }
            Err(e) => {
                failed += 1;
                // Never swallow: a page that could not be reconciled still
                // holds unlogged content, and the user has to know which.
                eprintln!("  FAILED    {} — {e}", page.slug);
            }
        }
    }

    println!();
    println!(
        "reconciled {} page(s), {ops_total} op(s) applied, {failed} failed",
        ahead.len() - failed
    );
    print_unjudged(&scan.skipped);
    if failed > 0 {
        println!("Re-run to retry the failures; their `.md` is untouched.");
        // Exit non-zero. A partial migration that reports success is how a
        // script marks the recovery done and moves on, leaving those pages
        // outside the log with nobody watching.
        anyhow::bail!("{failed} page(s) still hold content outside the op log");
    }
    println!("Run `outl doctor` to confirm nothing is left outside the log.");
    Ok(())
}

/// Clear a sidecar's `last_synced_hash` so `reconcile_md` stops
/// short-circuiting on it.
///
/// Safe to write: that field is a "when did I last sync this" marker, not
/// content. It rewrites only that one field, through the same
/// `outl_md::sidecar::{read,write}` pair every other writer uses, so the
/// block entries — the ids and ref handles that actually matter — are
/// preserved byte-for-byte. A crash between this call and the reconcile
/// leaves the page looking dirty, so the next boot reconciles it: the
/// same outcome, later.
///
/// A missing sidecar needs no action — without one there is no hash to
/// short-circuit on.
fn invalidate_synced_hash(md_path: &Path) -> Result<()> {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = match outl_md::sidecar::read(&sidecar_path) {
        Ok(sc) => sc,
        Err(_) => return Ok(()),
    };
    sc.last_synced_hash = String::new();
    outl_md::sidecar::write(&sidecar_path, &sc)
        .with_context(|| format!("rewriting {}", sidecar_path.display()))?;
    Ok(())
}

/// Walk every page, rendering it from the tree and comparing against the
/// `.md` on disk. A page is "ahead" when disk holds content lines the
/// render does not account for.
fn collect_ahead(ws: &outl_core::workspace::Workspace, root: &Path) -> Scan<AheadPage> {
    let mut scan = scan_pages(ws, root, |meta, md_path, disk, sidecar| {
        // Same reference the write-side guard uses: the sidecar's blocks,
        // not a render. A render answers "do disk and tree disagree",
        // which every remote edit also answers yes to, and reconciling
        // those would write the pre-edit text back as ops — reverting the
        // peer, permanently, since the log is append-only.
        // A sidecar that cannot answer does not get a vote — asked
        // here rather than inside the verdict, so the render-based
        // caller in `doctor` is not silenced by the same rule.
        if !outl_actions::sidecar_can_answer(&sidecar.blocks) {
            // A sidecar that cannot answer only matters when there is a
            // question to answer. `outl init` leaves two pages holding a
            // single empty block, and every page created for a `[[link]]`
            // that was never filled is the same shape: their sidecars
            // record no text, so they can answer nothing — but their
            // `.md` holds no text either, so nothing on disk could be
            // outside the log.
            //
            // Reporting those would make the unjudged list mostly empty
            // pages on a real workspace, which teaches the user to skim
            // past the entries that mean something. Asked through the
            // same owner with an empty reference — "what is on disk that
            // nothing accounts for" — so this is not a second opinion
            // about what a content line is.
            let on_disk = outl_actions::content_lines_missing_from(disk, &[]);
            if on_disk.iter().all(String::is_empty) {
                return Verdict::Clean;
            }
            return Verdict::Unjudged(Unjudged::SidecarCannotAnswer);
        }
        let missing = outl_actions::content_lines_missing_from(disk, &sidecar.blocks).len();
        if missing == 0 {
            return Verdict::Clean;
        }
        Verdict::Take(AheadPage {
            md_path: md_path.to_path_buf(),
            slug: meta.slug.clone(),
            missing,
        })
    });
    // Stable order so two runs report the same list.
    scan.picked.sort_by(|a, b| a.slug.cmp(&b.slug));
    scan
}

/// Why a page could not be judged.
///
/// The third state, and it needs a name of its own. "Scanned and clean"
/// and "never read" are different facts, and only one of them is good
/// news — collapsing them is how a recovery command reports a healthy
/// workspace it never looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unjudged {
    /// The `.md` exists and could not be read (permissions, an
    /// undownloaded iCloud placeholder, a mid-write truncation).
    UnreadableMd,
    /// No sidecar could be read next to an existing `.md`.
    MissingSidecar,
    /// A sidecar is there and will not parse.
    UnparseableSidecar,
    /// The sidecar parses but records no text for any of its blocks, so
    /// it cannot say what the op log held.
    SidecarCannotAnswer,
}

impl Unjudged {
    /// The sentence the user reads. Written here rather than at each
    /// call site so two modes cannot describe the same skip differently.
    fn reason(self) -> &'static str {
        match self {
            Self::UnreadableMd => "its `.md` could not be read",
            Self::MissingSidecar => {
                "it has a `.md` but no sidecar, so nothing records what the log held"
            }
            Self::UnparseableSidecar => "its sidecar will not parse",
            Self::SidecarCannotAnswer => {
                "its sidecar records no block text, so it cannot say what the log held"
            }
        }
    }
}

/// What `pick` says about one page.
enum Verdict<T> {
    /// The page qualifies for this mode.
    Take(T),
    /// Read end to end, nothing to do.
    Clean,
    /// Could not be looked at. Never folded into `Clean`.
    Unjudged(Unjudged),
}

/// The outcome of one whole-workspace walk.
struct Scan<T> {
    /// The pages this mode will act on.
    picked: Vec<T>,
    /// Pages that were never read, with the reason, sorted by slug.
    skipped: Vec<(String, Unjudged)>,
    /// How many pages this walk could answer for — `picked` included.
    /// A page with no `.md` counts: "there is no file" is an answer.
    judged: usize,
}

impl<T> Default for Scan<T> {
    fn default() -> Self {
        Self {
            picked: Vec::new(),
            skipped: Vec::new(),
            judged: 0,
        }
    }
}

/// Walk every page, handing `pick` the ones whose `.md` **and** sidecar
/// both read cleanly, and recording the ones that did not.
///
/// The skip rule is the reason this is one function rather than two
/// copies: **a `.md` we cannot read is not a page in any interesting
/// state — it is a read failure**, and guessing "empty" there is exactly
/// how content gets deleted (RFC 0210). A sidecar that will not parse is
/// skipped for the same reason: without it there is no record of what
/// the log held, so no question here can be answered about the page.
///
/// What changed once the skips were counted: **a page with no `.md` at
/// all is answerable and is not a skip.** There is no file, so it cannot
/// hold content the log lacks and cannot carry an unreconciled external
/// edit. That is the ordinary state of every page on a freshly paired
/// device, and reporting thousands of them as unjudged would bury the
/// handful that mean something. Only an `.md` that exists and refuses to
/// be read is a skip.
fn scan_pages<T>(
    ws: &outl_core::workspace::Workspace,
    root: &Path,
    pick: impl Fn(&outl_actions::PageMeta, &Path, &str, &outl_md::sidecar::Sidecar) -> Verdict<T>,
) -> Scan<T> {
    let mut scan = Scan::default();
    for meta in outl_actions::list_pages(ws) {
        let md_path = outl_actions::page_md_path(root, &meta);
        let disk = match fs::read_to_string(&md_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                scan.judged += 1;
                continue;
            }
            Err(_) => {
                scan.skipped
                    .push((meta.slug.clone(), Unjudged::UnreadableMd));
                continue;
            }
        };
        let sidecar = match outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)) {
            Ok(sidecar) => sidecar,
            Err(e) => {
                scan.skipped.push((meta.slug.clone(), unjudged_sidecar(&e)));
                continue;
            }
        };
        match pick(&meta, &md_path, &disk, &sidecar) {
            Verdict::Take(item) => {
                scan.judged += 1;
                scan.picked.push(item);
            }
            Verdict::Clean => scan.judged += 1,
            Verdict::Unjudged(why) => scan.skipped.push((meta.slug.clone(), why)),
        }
    }
    scan.skipped.sort_by(|a, b| a.0.cmp(&b.0));
    scan
}

/// Missing and corrupt are both unjudgeable and they are not the same
/// advice: one wants a projection, the other wants `doctor`.
fn unjudged_sidecar(e: &outl_md::sidecar::SidecarError) -> Unjudged {
    match e {
        outl_md::sidecar::SidecarError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            Unjudged::MissingSidecar
        }
        _ => Unjudged::UnparseableSidecar,
    }
}

/// How many unjudged pages to name before eliding.
const UNJUDGED_SAMPLE: usize = 20;

/// Report the pages the walk could not read.
///
/// Silence here is the defect this prints against. Both modes used to
/// drop an unreadable `.md`, an unparseable sidecar and a sidecar that
/// cannot answer without counting any of them, so a workspace where
/// **every** page was skipped printed "no page holds content outside the
/// op log" and exited 0 — a refusal that never reached the user, on the
/// command they run after a refusal already cost them content (root
/// `CLAUDE.md` invariant 8).
fn print_unjudged(skipped: &[(String, Unjudged)]) {
    if skipped.is_empty() {
        return;
    }
    println!();
    println!(
        "{} page(s) could NOT be judged and were not reconciled:",
        skipped.len()
    );
    for (slug, why) in skipped.iter().take(UNJUDGED_SAMPLE) {
        println!("  {slug} — {}", why.reason());
    }
    if skipped.len() > UNJUDGED_SAMPLE {
        println!("  … {} more", skipped.len() - UNJUDGED_SAMPLE);
    }
    println!("None of these is clean — each was skipped before its content was read.");
    println!("`outl doctor` reports an unreadable `.md` and a broken sidecar separately.");
}

#[cfg(test)]
mod tests {
    use outl_md::matching::guard::OrphanGuard;

    /// End-to-end proof that the escape hatch reaches the state the
    /// guard creates.
    ///
    /// The first version of this wiring passed a clap test and was still
    /// broken: `--allow-bulk-delete` only reached `--ahead-of-log`, which
    /// selects pages where `content_lines_missing_from > 0`. A page the
    /// guard refused is the opposite — its `.md` holds *less* than the
    /// log, so `missing` is zero and it never appeared. The flag was
    /// unreachable for every page it existed for.
    ///
    /// So this test does not assert on argument parsing. It builds a page
    /// the guard genuinely refuses, confirms the refusal, and then
    /// confirms the flag's guard value applies it.
    #[test]
    fn the_escape_hatch_applies_a_deletion_the_guard_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let md_path = dir.path().join("big.md");

        let full: String = (0..60).map(|i| format!("- block {i}\n")).collect();
        std::fs::write(&md_path, &full).expect("write");

        let actor = outl_core::id::ActorId::new();
        let mut ws = outl_core::workspace::Workspace::open_in_memory(actor).expect("ws");
        let hlc = outl_core::hlc::HlcGenerator::new(actor);
        outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("seed");

        // The user selects everything and deletes: 59 of 60 blocks go.
        std::fs::write(&md_path, "- block 0\n").expect("truncate");

        let refused = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None);
        assert!(
            matches!(refused, Err(outl_md::ReconcileError::BulkDelete(_))),
            "the guard must refuse 59 of 60 blocks, got {refused:?}"
        );

        // Same page, same bytes, guard off — which is what
        // `outl reconcile --allow-bulk-delete` passes.
        let applied =
            outl_md::reconcile_md_with_guard(&mut ws, &hlc, &md_path, None, &OrphanGuard::Disabled)
                .expect("the explicit opt-in must apply the deletion");
        assert!(
            applied.orphans >= 59,
            "the opt-in must apply the whole deletion, not part of it — got {} orphans",
            applied.orphans
        );
    }
}
