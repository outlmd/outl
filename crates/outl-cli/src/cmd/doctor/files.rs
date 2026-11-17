//! Filesystem-side checks: `.md` ↔ sidecar pairing, parse warnings,
//! orphan block refs, orphan sidecars, and sync-conflict files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use outl_md::index::WorkspaceIndex;
use outl_md::inline::{tokenize, InlineTok};
use outl_md::sidecar::{self, sidecar_path_for};

use super::Builder;

/// Cap on how many conflict files we name individually.
const MAX_REPORTED_CONFLICTS: usize = 30;

pub(super) fn check_md_files(
    b: &mut Builder,
    md_files: &[PathBuf],
    known_node_ids: &HashSet<outl_core::id::NodeId>,
) {
    for md in md_files {
        let scp = sidecar_path_for(md);
        if !scp.exists() {
            b.warn(format!(
                "{}: no sidecar (next `outl serve` or TUI commit will create one)",
                md.display()
            ));
            continue;
        }
        match sidecar::read(&scp) {
            Ok(sc) if sc.version == sidecar::SIDECAR_VERSION => {
                let mut unknown = 0;
                for sb in &sc.blocks {
                    if !known_node_ids.is_empty() && !known_node_ids.contains(&sb.id) {
                        unknown += 1;
                    }
                }
                if unknown == 0 {
                    b.ok(format!(
                        "{} (sidecar v{}, {} blocks, all IDs known)",
                        md.display(),
                        sc.version,
                        sc.blocks.len()
                    ));
                } else {
                    b.warn(format!(
                        "{}: {} block id(s) in sidecar not present in op log (workspace partially de-synced)",
                        md.display(),
                        unknown
                    ));
                }
            }
            Ok(sc) => {
                b.warn(format!(
                    "{}: sidecar version {} unsupported by this build",
                    md.display(),
                    sc.version
                ));
            }
            Err(e) => {
                b.err(format!("{}: sidecar unreadable: {e}", md.display()));
            }
        }
    }
}

/// Walk every indexed block, tokenize its text, and warn for every
/// `((blk-XXXXXX))` or `!((blk-XXXXXX))` whose handle does not resolve
/// to an indexed block.
///
/// ## Why a dangling `![[page]]` is deliberately **not** reported here
///
/// A page embed (`![[name]]`, issue #190) naming a page that does not
/// exist is not a defect, and the two shapes this check does report are
/// not the same kind of thing as it.
///
/// A `blk-` handle is **derived** from a `NodeId` (`derive_ref_handle`),
/// so a user cannot author one that points at nothing on purpose: an
/// unresolvable handle always means something broke downstream of the
/// user — a block left the tree, a sidecar desynced, a handle rotated —
/// and there is nothing in the `.md` the user could read to repair it.
/// Naming the file and the literal is the only way they find out.
///
/// A page **name** is the opposite on both counts. The user typed it, it
/// is legible in the file, and naming a page before creating it is the
/// core journal-first affordance: `[[next quarter]]` is how the page
/// gets made. Nothing in this report mentions a dangling `[[name]]`
/// today, and that is right — on a real workspace it is thousands of
/// rows, every one of them intentional.
///
/// Which leaves one question, and it is the only one that matters here:
/// is `![[name]]` enough *more* than `[[name]]` to answer differently?
/// It is not. Both resolve a page by name through the same ladder, both
/// produce a backlink (`outl_actions::mentions::collect`), and both are
/// respelled by a rename (`outl_actions::link_rewrite`). The sole
/// difference is how a client draws the target, which is a render
/// decision and not a workspace-integrity one. Reporting one spelling
/// and staying silent about the other would make this check a second
/// opinion on what a dangling page reference means.
///
/// If an unresolved embed ever needs to be *visible*, that belongs in
/// the client that renders it — the resolve reply omits a name it could
/// not resolve, so each client already knows — not in a filesystem
/// check the user runs by hand.
pub(super) fn check_orphan_block_refs(b: &mut Builder, idx: &WorkspaceIndex) {
    let mut orphans = 0usize;
    for block in idx.iter_blocks() {
        for tok in tokenize(&block.text) {
            let (handle, literal) = match tok {
                InlineTok::BlockRef { handle } => (handle, format!("(({handle}))")),
                InlineTok::Embed { handle } => (handle, format!("!(({handle}))")),
                _ => continue,
            };
            if idx.resolve_block_ref(handle).is_none() {
                orphans += 1;
                b.warn(format!(
                    "{}: orphan block ref {} — source block missing or not indexed",
                    block.source_path.display(),
                    literal,
                ));
            }
        }
    }
    if orphans == 0 {
        b.ok("no orphan ((blk-XXXXXX)) / !((blk-XXXXXX)) references");
    }
}

/// For every `.md` in `md_files`, parse it and emit a warning per
/// `ParseWarning` the parser had to recover from.
///
/// Under `write_log` it also appends a structured row per warning to
/// `orphans_log` (the same `.outl/orphans.log` reconcile uses; the rows
/// are tagged `parse-warning` to stay distinguishable from level-3
/// matching orphans).
///
/// ## Why the log is gated, and deduplicated
///
/// `write_log` is `--repair`, and it used to be unconditional. Two
/// separate problems came out of that:
///
/// 1. **A read-only diagnostic wrote to the user's workspace.** The same
///    code path serves the `outl_workspace_doctor` MCP tool, which
///    documents "never repairs" — every agent call appended rows.
/// 2. **It appended the same rows every single time.** Three runs, three
///    identical copies. On a freshly imported graph — where a leading
///    `# heading` and free prose are the normal shape of an imported
///    page, not a defect — one run adds thousands of lines, and the
///    level-3 matching orphans drown in them. Those are the record of
///    *blocks that could not be matched back into the log*, which is the
///    most important thing that file ever holds.
///
/// So the rows land only under `--repair`, and only if an equivalent row
/// (same file, same line, same kind) is not already there.
///
/// Returns how many warnings it found. The caller sums across every
/// scanned directory and prints the all-clear once — this function
/// only ever sees one directory, so it cannot know whether the
/// workspace as a whole is clean.
pub(super) fn check_parse_warnings(
    b: &mut Builder,
    md_files: &[PathBuf],
    orphans_log: &Path,
    write_log: bool,
) -> usize {
    use std::fmt::Write as _;
    use std::io::Write as _;

    // Rows already on disk, so re-running never duplicates. Read once:
    // the log can be large and this runs per scanned directory.
    let existing = if write_log {
        std::fs::read_to_string(orphans_log).unwrap_or_default()
    } else {
        String::new()
    };

    let mut total = 0usize;
    let mut log_buf = String::new();
    for md in md_files {
        let text = match std::fs::read_to_string(md) {
            Ok(t) => t,
            Err(e) => {
                b.warn(format!("{}: unreadable for parse check: {e}", md.display()));
                continue;
            }
        };
        let parsed = outl_md::parse::parse(&text);
        if parsed.warnings.is_empty() {
            continue;
        }
        total += parsed.warnings.len();
        let summary = match parsed.warnings.len() {
            1 => format!(
                "{}: 1 line outside outl dialect — preserved (line {})",
                md.display(),
                parsed.warnings[0].line
            ),
            n => {
                let lines: Vec<String> =
                    parsed.warnings.iter().map(|w| w.line.to_string()).collect();
                format!(
                    "{}: {n} line(s) outside outl dialect — preserved (lines {})",
                    md.display(),
                    lines.join(", ")
                )
            }
        };
        b.warn(summary);

        if !write_log {
            continue;
        }

        // One row per warning into the orphans log so the user can
        // grep later. Format: `parse-warning <iso> <path>:<line> <kind> <raw>`.
        // Truncate `raw` so a pathological line doesn't blow the log
        // up.
        for w in &parsed.warnings {
            let mut raw_preview: String = w.raw.chars().take(120).collect();
            if w.raw.chars().count() > 120 {
                raw_preview.push('…');
            }
            let kind = match w.kind {
                outl_md::ParseWarningKind::UnrecognizedBlockMarker => "unrecognized_block_marker",
                outl_md::ParseWarningKind::RemindMissingAnchor => "remind_missing_anchor",
                outl_md::ParseWarningKind::RemindInvalidTime => "remind_invalid_time",
                outl_md::ParseWarningKind::RemindInvalidInterval => "remind_invalid_interval",
                outl_md::ParseWarningKind::RemindInvalidStop => "remind_invalid_stop",
                outl_md::ParseWarningKind::RemindMaxClamped => "remind_max_clamped",
            };
            // Identity of a warning for dedup purposes: which file,
            // which line, which kind. The timestamp and the raw preview
            // are deliberately outside the key — the same defect
            // re-observed tomorrow is still the same defect, and a
            // second row for it buys the user nothing.
            let key = format!("{}:{} {}", md.display(), w.line, kind);
            if existing.contains(&key) || log_buf.contains(&key) {
                continue;
            }
            let _ = writeln!(
                log_buf,
                "parse-warning {} {} {}",
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%z"),
                key,
                raw_preview,
            );
        }
    }

    // Append best-effort. Failure here is non-fatal — the warnings
    // already showed up in the doctor report itself; the log is just
    // a persisted breadcrumb trail.
    if !log_buf.is_empty() {
        if let Some(parent) = orphans_log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(orphans_log)
        {
            let _ = f.write_all(log_buf.as_bytes());
        }
    }
    total
}

/// A `<stem>.outl` with no `<stem>.md` next to it.
///
/// `sidecar_files` carries both the modern un-hidden `foo.outl` and the
/// legacy dotted `.foo.outl`, so both spellings are stripped before the
/// `.md` lookup.
pub(super) fn check_orphan_sidecars(
    b: &mut Builder,
    sidecar_files: &[PathBuf],
    md_files: &[PathBuf],
) {
    let md_stems: HashSet<String> = md_files
        .iter()
        .filter_map(|p| p.file_stem().and_then(|n| n.to_str()).map(String::from))
        .collect();
    for scp in sidecar_files {
        let Some(name) = scp.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".outl") else {
            continue;
        };
        // Legacy dotted spelling; the un-hidden one has no prefix.
        let stem = stem.strip_prefix('.').unwrap_or(stem);
        if !md_stems.contains(stem) {
            b.warn(format!(
                "{}: orphaned sidecar (no matching {stem}.md on disk)",
                scp.display(),
            ));
        }
    }
}

/// Detect files a file-sync transport forked instead of merging.
///
/// iCloud Drive resolves a concurrent write by keeping both sides:
/// `foo 2.md` next to `foo.md`, or `foo (conflicted copy).md`.
/// Syncthing writes `foo.sync-conflict-20260805-101500-ABCDEFG.md`.
/// Dropbox writes `foo (Avelino's conflicted copy 2026-08-05).md`.
///
/// **All of them are user content sitting outside the op log.** Nothing
/// in outl reads those files, so the blocks in them are invisible to
/// the CRDT, to search, and to every peer — and the user has no signal
/// at all today. A conflict under `ops/` is worse still: it is a forked
/// op log, so the ops in it never reach the tree.
///
/// This is deliberately *not* repairable. Merging two forks of a page
/// is a judgement call about which side wins; the doctor's job is to
/// make sure the user knows the fork exists.
///
/// Returns every conflict found. The caller cares specifically about the
/// ones under `ops/`: a forked op log means the replayed tree is missing
/// whatever the fork holds, which is what gates projection repair.
pub(super) fn check_sync_conflicts(b: &mut Builder, dirs: &[&Path]) -> Vec<PathBuf> {
    let mut found: Vec<(PathBuf, &'static str)> = Vec::new();

    for dir in dirs {
        for entry in walkdir::WalkDir::new(dir).max_depth(3) {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(kind) = conflict_kind(p, name) {
                found.push((p.to_path_buf(), kind));
            }
        }
    }

    if found.is_empty() {
        b.ok("no sync-conflict copies (iCloud / Syncthing / Dropbox) in the workspace");
        return Vec::new();
    }

    found.sort();
    for (path, kind) in found.iter().take(MAX_REPORTED_CONFLICTS) {
        b.err(format!(
            "{}: {kind} conflict copy — user content outside the op log, outl never reads it. \
             Move the wanted lines into the real file (or import it) and delete the copy",
            path.display()
        ));
    }
    if found.len() > MAX_REPORTED_CONFLICTS {
        b.err(format!(
            "… and {} more sync-conflict file(s)",
            found.len() - MAX_REPORTED_CONFLICTS
        ));
    }
    found.into_iter().map(|(path, _)| path).collect()
}

/// Classify `name` as a sync-conflict copy, or `None` when it looks
/// like a normal file.
///
/// The numeric-suffix rule (`foo 2.md`) only fires when the base file
/// exists next to it — plenty of real notes are legitimately called
/// `sprint 2.md`, and a false positive here would train the user to
/// ignore the loudest finding the doctor emits.
fn conflict_kind(path: &Path, name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    if lower.contains(".sync-conflict-") {
        return Some("Syncthing");
    }
    if lower.contains("conflicted copy") {
        return Some("iCloud/Dropbox");
    }

    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem, Some(ext)),
        None => (name, None),
    };
    // `.jsonl` / `.md` / `.outl` are the only extensions whose fork
    // costs the user data; ignore everything else.
    if !matches!(ext, Some("md") | Some("outl") | Some("jsonl")) {
        return None;
    }
    let (base, suffix) = stem.rsplit_once(' ')?;
    if base.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let sibling = match ext {
        Some(ext) => path.with_file_name(format!("{base}.{ext}")),
        None => path.with_file_name(base),
    };
    sibling.exists().then_some("iCloud")
}
