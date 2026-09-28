//! What the user sees: the human listing, the `--json` envelope, and
//! the exit code each of them implies.
//!
//! Both entry points wrap [`collect_scoped`] and decide nothing — every
//! verdict they print was reached by the pass. The exit code is the
//! part worth keeping in one file: [`run`] exits 1 from inside the
//! printer, and [`run_json`] has to re-derive the same verdict from the
//! serialized report, because `emit` only knows whether the *call*
//! succeeded, not whether the report it carried holds errors.

use anyhow::{Context, Result};
use std::path::Path;

use crate::output::{emit, ApiError};

use super::{collect_scoped, RepairScope, Severity};

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
        for line in &report.repairable {
            println!("  - {line}");
        }
    }
    if let Some(rep) = &report.repair {
        println!();
        println!("repair: backups under {}", rep.backup_dir);
        for action in &rep.actions {
            let tag = if action.ok { "done" } else { "SKIP" };
            println!(
                "  {tag} {} {} ({})",
                action.kind, action.path, action.detail
            );
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
