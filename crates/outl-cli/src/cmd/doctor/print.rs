//! What the user sees: the human listing, the `--json` envelope, and
//! the exit code each of them implies.
//!
//! Both entry points wrap [`collect_scoped`] and decide nothing — every
//! verdict they print was reached by the pass. The exit code is the
//! part worth keeping in one file: [`run`] exits 1 from inside the
//! printer, and [`run_json`] has to re-derive the same verdict from the
//! serialized report, because `emit` only knows whether the *call*
//! succeeded, not whether the report it carried holds errors.
//!
//! **The human listing is capped; `--json` is not.** A real 2,865-page
//! workspace produced 840 repairable items and 840 repair actions, one
//! page each, which is a scroll rather than a report
//! ([#286](https://github.com/outlmd/outl/issues/286)). So this file
//! prints the first [`LIST_CAP`] and says how many it withheld, and the
//! full list stays in the serialized report that `--json` emits.
//!
//! One rule that is not a preference: **cap the successes, never the
//! refusals.** A skipped repair action is something the pass declined to
//! do, and a decline the user does not hear about is the silence root
//! `CLAUDE.md` invariant 8 exists to prevent. So every `RepairAction`
//! whose `ok` is false is printed in full however long it is, and
//! [`LIST_CAP`] applies only to the `ok` arm.

use anyhow::{Context, Result};
use std::path::Path;

use crate::output::{emit, ApiError};

use super::{collect_scoped, RepairScope, Severity};

/// How many per-item lines the human listing prints before it switches
/// to a count.
///
/// Twenty is above every healthy workspace's whole plan and far below
/// the bulk cases that made this necessary, so an ordinary run is
/// unchanged and a 840-page one stays one screen.
const LIST_CAP: usize = 20;

/// How many lines a listing of `total` items withheld, or `None` when it
/// printed all of them.
fn withheld(total: usize) -> Option<usize> {
    total.checked_sub(LIST_CAP).filter(|n| *n > 0)
}

/// CLI entry point with human output. Exits with status 1 when the
/// report has errors so scripts can detect failure.
pub fn run(path: &Path, do_repair: bool, scope: RepairScope) -> Result<()> {
    let report = collect_scoped(path, do_repair, scope)
        .with_context(|| format!("running doctor on {}", path.display()))?;
    println!("workspace: {}", report.workspace);
    println!("actor:     {}", report.actor);
    println!();
    for finding in &report.findings {
        let tag = match finding.severity {
            Severity::Ok => "ok:  ",
            Severity::Info => "info:",
            Severity::Warn => "warn:",
            Severity::Error => "err: ",
        };
        println!("{tag} {}", finding.message);
    }

    if !report.repairable.is_empty() {
        println!();
        let suffix = if do_repair {
            ""
        } else {
            " — run `outl doctor --repair`"
        };
        println!("{} repairable item(s){suffix}:", report.repairable.len());
        for line in report.repairable.iter().take(LIST_CAP) {
            println!("  - {line}");
        }
        if let Some(withheld) = withheld(report.repairable.len()) {
            println!("  … and {withheld} more (`outl doctor --json` lists every one)");
        }
    }
    if let Some(rep) = &report.repair {
        println!();
        println!("repair: backups under {}", rep.backup_dir);
        // Refusals first and uncapped: they are the half of this listing
        // the user cannot reconstruct from the counts below. `rep.repaired`
        // is the count of the `ok` arm, so the withheld figure and the
        // `N fixed` line below cannot disagree.
        for action in rep
            .actions
            .iter()
            .filter(|a| !a.ok)
            .chain(rep.actions.iter().filter(|a| a.ok).take(LIST_CAP))
        {
            let tag = if action.ok { "done" } else { "SKIP" };
            println!(
                "  {tag} {} {} ({})",
                action.kind, action.path, action.detail
            );
        }
        if let Some(withheld) = withheld(rep.repaired) {
            println!("  … and {withheld} more done (`outl doctor --repair --json` lists them)");
        }
        println!("repair: {} fixed, {} not fixed", rep.repaired, rep.failed);
    }

    println!();
    match (report.error_count, report.warn_count) {
        (0, 0) => println!("integrity OK"),
        (0, w) => println!("integrity OK with {w} warning(s)"),
        (e, w) => {
            println!("{e} error(s), {w} warning(s) — see lines above");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `outl doctor --json` shape — emits the envelope and exits 1 when
/// the report has errors.
pub fn run_json(path: &Path, do_repair: bool, scope: RepairScope) -> i32 {
    let result = collect_scoped(path, do_repair, scope)
        .and_then(|r| serde_json::to_value(&r).map_err(ApiError::internal));
    let exit = emit(true, result.clone(), |_| {});
    // `emit` already used the JSON branch; force an error exit when
    // the report itself carried errors even though the call succeeded.
    if exit == 0
        && result
            .ok()
            .and_then(|v| v.get("error_count").and_then(|n| n.as_u64()))
            .unwrap_or(0)
            > 0
    {
        return 1;
    }
    exit
}
