//! Doctor test battery.
//!
//! Every check gets the same shape of test: build a real workspace in a
//! `TempDir`, break exactly one thing on purpose, and assert the doctor
//! *names* it. A check that fires on a healthy workspace is as bad as
//! one that stays silent on a broken one, so the happy path is asserted
//! alongside.
//!
//! This file is the harness that shape needs, and nothing else: a
//! seeded workspace, a doctor run that owns its own device store, and
//! the predicates over a [`DoctorReport`]. The tests sit one file per
//! subject, each named after the production module it drives.
//!
//! Detection — does the doctor *see* the defect:
//!
//! - [`global_config`] — the unreadable `~/.config/outl/config.toml`,
//!   the one finding phrased in `super` rather than by a check.
//! - [`oplog`] — corrupt lines, glued lines, stale offset indexes.
//! - [`files`] — sync conflicts, orphan sidecars, parse warnings.
//! - [`tree`] — trash contents, projection drift, the sidecar rebuild.
//! - [`snapshots`] and [`index_sidecars`] — the two caches `--repair`
//!   is allowed to reclaim.
//! - [`device_store`] — the subject that lives outside the workspace.
//!
//! The other half — what the doctor is allowed to **write** while it
//! looks — is [`safety`], with [`volume`] on how much one `--repair`
//! may remove before it stops and asks.

mod device_store;
mod files;
mod global_config;
mod index_sidecars;
mod oplog;
mod safety;
mod snapshots;
mod tree;
mod volume;

use super::*;
use crate::workspace_layout::{init, Paths};
use outl_core::id::NodeId;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ---------------------------------------------------------------- setup

/// A fresh, initialized workspace.
fn fresh() -> (TempDir, PathBuf, Paths) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("notes");
    let paths = Paths::at(&root);
    init(&paths).expect("init");
    (dir, root, paths)
}

/// Create page `slug` with one block per entry of `blocks`, and project
/// it (`.md` + sidecar) so the workspace starts consistent.
fn seed_page(root: &Path, slug: &str, blocks: &[&str]) -> NodeId {
    let mut ctx = crate::ws::open(root).expect("open workspace");
    let page = outl_actions::open_or_create_page(
        &mut ctx.workspace,
        &ctx.hlc,
        slug,
        slug,
        outl_actions::PageKind::Page,
    )
    .expect("create page");
    for text in blocks {
        outl_actions::append_block(&mut ctx.workspace, &ctx.hlc, Some(page), Some(text))
            .expect("append block");
    }
    outl_actions::apply_page_md_with_sidecar(&ctx.workspace, root, page).expect("project page");
    page
}

/// The single `ops-<actor>.jsonl` a freshly-seeded workspace has.
fn ops_file(paths: &Paths) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(&paths.ops)
        .expect("read ops dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ops-") && n.ends_with(".jsonl"))
        })
        .collect();
    found.sort();
    found
        .pop()
        .expect("an ops-*.jsonl must exist after seeding")
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open ops file for append");
    f.write_all(bytes).expect("append");
}

fn messages(report: &DoctorReport) -> Vec<String> {
    report.findings.iter().map(|f| f.message.clone()).collect()
}

/// A doctor run with the default authority — no `--force`.
///
/// The whole battery goes through this rather than `collect_scoped` so
/// that a test which *wants* the forced scope has to say so, and reads
/// as the exception it is.
fn collect(path: &Path, do_repair: bool) -> Result<DoctorReport, ApiError> {
    collect_with_scope(path, do_repair, RepairScope::Guarded)
}

/// A doctor run against a device store **this call owns**.
///
/// Not a detail. `doctor` now reads (and under `--repair`, prunes) the
/// device store's actor bindings, and that store is machine-global: the
/// repo's `.cargo/config.toml` points every cargo-spawned process at one
/// shared `.dev-device-store`. Going through `collect_scoped` here would
/// make every test in this battery judge — and delete from — a directory
/// that other tests, other test binaries and the developer's own
/// `cargo run` are all writing to. The binding count would drift, the
/// `repairable[]` assertions with it, and the result is issue #211's
/// flaky doctor tests reintroduced by the fix for issue #211.
///
/// A `TempDir` per call is safe *here* precisely because it is not an
/// env-var mutation: `collect_internal` takes the store as an argument,
/// so nothing about this is process-wide (root `CLAUDE.md` invariant 9,
/// third question).
fn collect_with_scope(
    path: &Path,
    do_repair: bool,
    scope: RepairScope,
) -> Result<DoctorReport, ApiError> {
    let store_dir = tempfile::TempDir::new().expect("temp device store");
    collect_with_store(
        path,
        do_repair,
        scope,
        &outl_core::device::DeviceStore::at(store_dir.path()),
    )
}

/// A doctor run against a device store **the caller owns**.
///
/// Needed wherever a test cares about the actor the doctor resolves. A
/// fresh store mints a fresh actor (`resolve_device_actor` only adopts
/// `config.toml`'s when `actor_claimed_by` matches the store's machine
/// id, which a `TempDir` store never does), so two `collect` calls that
/// each build their own store are two different devices. Anything about
/// `snap-<own actor>.bin` is unanswerable under that.
fn collect_with_store(
    path: &Path,
    do_repair: bool,
    scope: RepairScope,
    store: &outl_core::device::DeviceStore,
) -> Result<DoctorReport, ApiError> {
    super::collect_internal(
        path,
        true,
        do_repair,
        scope,
        store,
        // The battery is not exercising `[theme]` validation — that check
        // has its own unit tests in `doctor::theme`. A default `ThemeCfg`
        // (no `preset_dark`) is never flagged, so it stays inert here
        // rather than reading the developer's real global config. Same
        // reason for the `None` notice: reading the real
        // `~/.config/outl/config.toml` here would make every finding count in
        // this battery depend on whether the developer's own file parses.
        // `a_config_that_does_not_parse_is_reported` passes `Some` directly.
        &outl_config::ThemeCfg::default(),
        None,
    )
}

/// Backdate a path's mtime by `days`, so a TTL can be exercised without
/// sleeping through it.
///
/// Shared: the backup-generation prune and the snapshot scratch-file
/// prune both have one, and they must agree about what "old" means.
fn backdate(dir: &Path, days: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
    let times = std::fs::FileTimes::new()
        .set_accessed(when)
        .set_modified(when);
    std::fs::File::open(dir)
        .expect("open the path to backdate")
        .set_times(times)
        .expect("backdate");
}

/// Every file under `dir`, by relative path and content.
///
/// The `ops/` assertions compare **directories**, not one file. The
/// original version of `repair_never_touches_the_op_log` diffed only the
/// `.jsonl` while its message promised "not a single byte into ops/" —
/// and the doctor was meanwhile creating `.ops-<actor>.idx` /
/// `.ops-<actor>.nodes.idx` next to it on every run, which the assert
/// could not see.
fn dir_snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(dir) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(dir).unwrap_or(path).to_path_buf();
        out.insert(rel, std::fs::read(path).unwrap_or_default());
    }
    out
}

fn has(report: &DoctorReport, needle: &str) -> bool {
    report.findings.iter().any(|f| f.message.contains(needle))
}
