//! `outl compact` — drop provably-inert ops from the op log.
//!
//! ## Read-only by default
//!
//! A plain run reports what it *would* drop and writes nothing. `--apply`
//! is the only writing mode, and it is the only command in this binary
//! that rewrites `ops/*.jsonl` — the file root `CLAUDE.md` invariant 1
//! calls the source of truth. `outl doctor --repair`, by contrast, will
//! not touch `ops/` at all, on purpose: it repairs *projections*, and a
//! corrupt op is something it can only report.
//!
//! The difference is what compaction can prove. It removes exactly one
//! shape — a `Move` that restates the placement its own `Create` made
//! immediately before it, with nothing between them in the merged HLC
//! order — and it verifies against a full replay that applying that
//! `Move` changes nothing. Every other op is copied through byte for
//! byte. The predicate, its soundness argument and the residual risk
//! live in `docs/rfcs/0256-op-log-compaction.md`.
//!
//! ## It rewrites this device's log, not the fleet's
//!
//! `ops/` holds one `ops-<actor>.jsonl` per device, and `docs/storage.md`
//! states the premise the whole layout rests on: *"Each device's file is
//! append-only and owned by exactly one writer."* That is what makes
//! `transport = "file"` (iCloud Drive, Syncthing, a shared filesystem)
//! safe — those transports reconcile **per path**, last-write-wins.
//!
//! A device that shortens a peer's file therefore publishes a competing,
//! shorter version of that path. The peer's longer copy loses the merge
//! and every op it had not yet shipped dies silently. The locks cannot
//! arbitrate: `flock(2)` is advisory and machine-local.
//!
//! So `--apply` narrows the plan to this device's own actor. The dry run
//! still reports the whole log's dead weight, because the predicate is
//! decided against the merged log either way; only the rewrite narrows.
//! Running `outl compact --apply` on each device is the intended usage
//! (RFC 0256 → "Re-pairing a device does not undo it").
//!
//! `--force` lifts the narrowing for a workspace the user knows no file
//! transport carries. It is also the only route to an
//! `ops-<ephemeral>.jsonl` this device wrote in a past session: those are
//! ours in fact but not provably, and "provably" is the bar for rewriting
//! a log.
//!
//! ## What `--apply` guarantees
//!
//! - It refuses while any other `outl` process holds the workspace: a
//!   live client caches byte offsets into the files this renumbers.
//! - It copies every file it will rewrite into
//!   `.outl/compact-backup/<timestamp>-<id>/` first. Restoring is `cp`
//!   back. The generation is per *run*, not per second: two valid runs
//!   inside one second (`--apply` then the `--no-horizon` re-run the dry
//!   run recommends) would otherwise share a directory, and the second
//!   would copy the already-compacted file over the only way back.
//! - Nothing prunes those generations. `.outl/repair-backup/` has two
//!   guards (age *and* count) and this has neither, so the run reports
//!   the directory and says it is the user's to delete — a command that
//!   advertises reclaiming disk must not grow `.outl/` in silence.
//! - It replaces each file via temp + `rename`, so a reader sees the
//!   whole old file or the whole new one.
//! - It deletes every index sidecar it invalidated, so the next boot
//!   rebuilds them instead of seeking to stale offsets. The set comes
//!   from `outl_core::storage::sidecar` rather than two names spelled out
//!   here, and it covers the dead generations too: an undotted
//!   `ops-<actor>.idx` and an abandoned `*.idx.tmp.<ulid>` hold offsets
//!   into the same renumbered file.
//! - It refuses outright on a log holding a record it could not parse.
//!   A damaged log is reported, never rewritten.

use std::path::Path;

use anyhow::{Context, Result};
use outl_core::device::DeviceStore;
use outl_core::id::ActorId;
use outl_core::storage::compact::{
    apply_compaction, apply_compaction_as, plan_compaction, CompactOptions, CompactReport,
    DEFAULT_HORIZON_MS,
};

/// Run the `compact` subcommand.
pub fn run(path: &Path, apply: bool, no_horizon: bool, force: bool) -> Result<()> {
    let opts = CompactOptions {
        horizon_ms: if no_horizon { 0 } else { DEFAULT_HORIZON_MS },
    };
    let plan = plan_compaction(path, &opts)
        .with_context(|| format!("planning compaction for {}", path.display()))?;
    let report = plan.report();

    if plan.is_empty() {
        println!(
            "nothing to compact — {} op(s), {} across {} actor file(s)",
            report.ops_total,
            human_bytes(report.bytes_total),
            report.actors.len()
        );
        if !no_horizon {
            println!(
                "History newer than {} days is left alone. Re-run with `--no-horizon` to include it.",
                DEFAULT_HORIZON_MS / (24 * 60 * 60 * 1000)
            );
        }
        return Ok(());
    }

    print_plan(report);

    if !apply {
        println!();
        println!("Nothing was written. Re-run with `--apply` to rewrite the op log.");
        println!(
            "Every rewritten file is copied to `.outl/compact-backup/<timestamp>-<id>/` first, \
             and `--apply` refuses while any other outl process has this workspace open."
        );
        println!(
            "`--apply` rewrites only this device's `ops-<actor>.jsonl`; run it on each device \
             (or pass `--force`, which also rewrites theirs)."
        );
        return Ok(());
    }

    let done = if force {
        println!();
        println!(
            "--force: rewriting every actor file, including other devices'. If any file \
             transport (iCloud, Syncthing, a shared filesystem) carries this workspace, a \
             shortened copy of a peer's log wins last-write-wins and takes that device's \
             unshipped ops with it."
        );
        apply_compaction(path, &plan).with_context(|| format!("compacting {}", path.display()))?
    } else {
        let own = this_devices_actor(path)?;
        let skipped = plan.foreign_actors(own);
        let mine = plan.restricted_to(own);
        if mine.is_empty() {
            println!();
            println!(
                "Nothing in this device's own `ops-{own}.jsonl` is droppable. The {} inert \
                 op(s) above live in {} other actor file(s), which belong to other devices \
                 (or to past sessions of this one).",
                report.ops_dropped,
                skipped.len()
            );
            print_foreign_note(&skipped);
            return Ok(());
        }
        let done = apply_compaction_as(path, &mine, own)
            .with_context(|| format!("compacting {}", path.display()))?;
        if !skipped.is_empty() {
            println!();
            println!(
                "{} other actor file(s) were left alone — they are other devices' logs.",
                skipped.len()
            );
            print_foreign_note(&skipped);
        }
        done
    };

    println!();
    println!(
        "Dropped {} op(s), {} reclaimed.",
        done.ops_dropped,
        human_bytes(done.bytes_dropped)
    );
    if let Some(dir) = &done.backup_dir {
        println!("Pre-compaction op log copied to {}", dir.display());
        // That copy is a whole file, and this run removed a fraction of
        // one — so the pass costs more disk than it reclaims until the
        // generation goes, and nothing here ever removes it. Saying so
        // is the honest answer to invariant 9's fourth question while
        // the prune does not exist (RFC 0256 -> Scope).
        println!(
            "That generation is a full copy of the files this run rewrote, and nothing \
             prunes it — delete it once you are satisfied with the result."
        );
    }
    println!(
        "Every index sidecar for the rewritten actors was deleted — including the dead \
         generations that held offsets into the same files. The next boot rebuilds them."
    );
    Ok(())
}

/// The actor this device writes under for the workspace at `root`.
///
/// The same resolution every other command uses (`outl_ws::actor`), so
/// "the file compaction may rewrite" and "the file this device appends
/// to" can never be two different answers.
fn this_devices_actor(root: &Path) -> Result<ActorId> {
    let paths = outl_ws::layout::Paths::at(root.to_path_buf());
    let cfg = outl_ws::layout::read_or_init_config(&paths)
        .with_context(|| format!("reading the workspace config at {}", paths.config.display()))?;
    outl_ws::actor::resolve_device_actor(&paths, &cfg, &DeviceStore::open_default())
        .with_context(|| format!("resolving this device's actor for {}", root.display()))
}

fn print_foreign_note(skipped: &[ActorId]) {
    for actor in skipped {
        println!("  ops-{actor}.jsonl");
    }
    println!(
        "Run `outl compact --apply` on each of those devices. `--force` rewrites them from \
         here, which is safe only if no file transport (iCloud, Syncthing, a shared \
         filesystem) carries this workspace — those reconcile per path, last-write-wins."
    );
}

fn print_plan(report: &CompactReport) {
    println!(
        "{} of {} op(s) are provably inert — {} of {} ({:.1}%).",
        report.ops_dropped,
        report.ops_total,
        human_bytes(report.bytes_dropped),
        human_bytes(report.bytes_total),
        report.percent()
    );
    println!();
    println!(
        "{:<28} {:>9} {:>10} {:>9} {:>7}",
        "actor", "ops", "drop ops", "reclaim", "share"
    );
    for actor in &report.actors {
        if actor.ops_dropped == 0 {
            continue;
        }
        println!(
            "{:<28} {:>9} {:>10} {:>9} {:>6.1}%",
            actor.actor.to_string(),
            actor.ops_total,
            actor.ops_dropped,
            human_bytes(actor.bytes_dropped),
            actor.percent()
        );
    }
}

/// Bytes in the shortest unit that keeps one decimal meaningful.
fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_switches_unit_at_each_threshold() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    }
}
