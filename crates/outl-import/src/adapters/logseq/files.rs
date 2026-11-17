//! Which files in a Logseq graph the adapter reads — and what it says
//! about the ones it doesn't.
//!
//! The previous scan was a depth-1 listing of `pages/` and `journals/`
//! that matched `.md`, named `.org` in `skipped`, and dropped
//! everything else through a bare `_ => {}`. That is the exact
//! anti-pattern this crate's CLAUDE.md prohibits ("never `continue`
//! past a construct you're dropping"), and it hid three real things on
//! a real graph: whiteboards, pages filed into a subdirectory, and any
//! other extension sitting in `pages/`.
//!
//! Two categories, because the reconciliation depends on the
//! difference:
//!
//! - A **page** the adapter cannot read is counted in `source_pages`
//!   and listed in `skipped`. `ImportReport::finalize` subtracts
//!   `skipped.len()` from the denominator, so every skip entry has to
//!   be something the denominator counted — otherwise the books stop
//!   closing and the `UNACCOUNTED CONTENT` block cries wolf.
//! - A file that was never a page (`logseq/config.edn`, `assets/`,
//!   `draws/`) is in neither. It goes into one aggregate `(graph)`
//!   warning, the same shape `logseq_db` uses for structural nodes.

use crate::adapter::ImportError;
use crate::report::ImportReport;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One source file that holds a page, and where it belongs.
pub(crate) struct PageFile {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// `journals/` (a daily note) rather than `pages/`.
    pub is_journal: bool,
}

/// Directories Logseq keeps beside the graph that hold no pages.
/// Named so the aggregate warning can say what it saw rather than
/// "some files".
const NON_PAGE_DIRS: &[&str] = &["assets", "logseq", "draws", "version-files"];

/// Every `.md` page in the graph, in a stable order, with every other
/// file accounted for in the report.
///
/// Bumps `source_pages` for each page it finds *and* each page it
/// refuses, so the reconciliation's denominator covers both.
///
/// A directory under `pages/`, `journals/` or `whiteboards/` that cannot
/// be listed fails the import: its pages would enter neither the
/// denominator nor `skipped`, so the books would balance over a missing
/// subtree.
pub(crate) fn collect(src: &Path, report: &mut ImportReport) -> Result<Vec<PageFile>, ImportError> {
    let mut pages = Vec::new();
    for (dir, is_journal) in [("pages", false), ("journals", true)] {
        let d = src.join(dir);
        if !d.is_dir() {
            continue;
        }
        for path in walk(&d, usize::MAX)? {
            let rel = rel_label(src, &path);
            // Everything in these two directories is a page Logseq
            // would show, read or not, so all of it counts.
            match path.extension().and_then(|x| x.to_str()) {
                Some("md") => {
                    report.count_source(1, 0);
                    pages.push(PageFile { path, is_journal });
                }
                Some("org") => refuse_page(
                    report,
                    rel,
                    "org-mode files are not supported — convert the graph to markdown \
                     in Logseq first",
                ),
                other => refuse_page(
                    report,
                    rel,
                    format!(
                        "a .{} file under {dir}/ — the importer only reads markdown pages",
                        other.unwrap_or("(no extension)")
                    ),
                ),
            }
        }
    }
    pages.sort_by(|a, b| a.path.cmp(&b.path));
    survey_the_rest(src, report)?;
    Ok(pages)
}

/// A page the importer will not read: into the page **denominator**
/// and into `skipped`, together.
///
/// The two are one action on purpose. `finalize` subtracts
/// `skipped.len()` from `source_pages`, so a skip entry that was never
/// counted in the denominator makes the books stop closing and
/// `UNACCOUNTED CONTENT` cry wolf — and the way to write that bug is to
/// remember one of these two calls and forget the other.
fn refuse_page(report: &mut ImportReport, path: String, reason: impl Into<String>) {
    report.count_source(1, 0);
    report.skip(path, reason, 0);
}

/// Whiteboards are pages (Logseq lists them beside `pages/`), so each
/// one is a skip entry; everything else beside the graph is not, and
/// goes into one aggregate warning.
fn survey_the_rest(src: &Path, report: &mut ImportReport) -> Result<(), ImportError> {
    let whiteboards = src.join("whiteboards");
    if whiteboards.is_dir() {
        for path in walk(&whiteboards, usize::MAX)? {
            refuse_page(
                report,
                rel_label(src, &path),
                "whiteboards are an EDN canvas with no outl equivalent yet",
            );
        }
    }

    // These hold no pages, so a listing failure here costs no content:
    // it is named in the warning rather than failing the import.
    let mut unread: BTreeMap<&str, String> = BTreeMap::new();
    for dir in NON_PAGE_DIRS {
        let d = src.join(dir);
        if d.is_dir() {
            match walk(&d, usize::MAX) {
                Ok(files) if files.is_empty() => {}
                Ok(files) => {
                    unread.insert(*dir, files.len().to_string());
                }
                Err(_) => {
                    unread.insert(*dir, "could not be listed".to_string());
                }
            }
        }
    }
    let loose = walk(src, 1)?.len();
    if unread.is_empty() && loose == 0 {
        return Ok(());
    }
    let mut parts: Vec<String> = unread
        .iter()
        .map(|(dir, n)| format!("{dir}/ ({n})"))
        .collect();
    if loose > 0 {
        parts.push(format!("{loose} loose in the graph root"));
    }
    report.warn(
        "(graph)",
        format!(
            "not read, and holds no pages: {} (file counts). Config, backups, drawings and \
             loose files never become pages; an attachment a block links to is still pulled \
             into assets/",
            parts.join(", ")
        ),
    );
    Ok(())
}

/// Every file under `dir`, down to `depth` levels, sorted.
///
/// `usize::MAX` for `pages/` and `journals/`, because Logseq reads them
/// recursively and a page filed into a subdirectory is a page; `1` for
/// the graph root, where only the loose files are this pass's business.
///
/// A traversal error is returned, never skipped: dropping it would read
/// an unreadable directory as an empty one.
fn walk(dir: &Path, depth: usize) -> Result<Vec<PathBuf>, ImportError> {
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(depth)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|e| {
            let path = e.path().unwrap_or(dir).to_path_buf();
            ImportError::io(&path, e.into())
        })?;
        if entry.file_type().is_file() && !is_hidden(entry.path()) {
            out.push(entry.into_path());
        }
    }
    out.sort();
    Ok(out)
}

/// `.DS_Store`, `.gitignore`, editor droppings — never a page, and
/// listing them would bury the files that matter.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Graph-relative path for the report, falling back to the full path.
fn rel_label(src: &Path, path: &Path) -> String {
    path.strip_prefix(src).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for f in files {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(&p, b"- x\n").expect("write");
        }
        dir
    }

    /// Both directories, recursively: Logseq reads a page filed into a
    /// subdirectory, and silently not reading it was straight loss.
    #[test]
    fn markdown_pages_are_collected_from_both_dirs_recursively() {
        let g = graph(&["pages/a.md", "pages/sub/deep.md", "journals/2026_01_01.md"]);
        let mut report = ImportReport::new("logseq");
        let pages = collect(g.path(), &mut report).expect("collect");
        assert_eq!(pages.len(), 3);
        assert_eq!(report.source_pages, 3);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    /// The bug this module exists for: an unsupported extension used to
    /// vanish through a bare `_ => {}`.
    #[test]
    fn an_unsupported_page_file_is_named_in_skipped() {
        let g = graph(&["pages/a.md", "pages/board.canvas", "pages/old.org"]);
        let mut report = ImportReport::new("logseq");
        let pages = collect(g.path(), &mut report).expect("collect");
        assert_eq!(pages.len(), 1);
        assert_eq!(report.source_pages, 3, "refused pages are still counted");
        let reasons: Vec<&str> = report.skipped.iter().map(|s| s.path.as_str()).collect();
        assert!(
            reasons.iter().any(|p| p.ends_with("board.canvas")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|p| p.ends_with("old.org")),
            "{reasons:?}"
        );
    }

    #[test]
    fn whiteboards_are_pages_and_land_in_skipped() {
        let g = graph(&["pages/a.md", "whiteboards/w.edn"]);
        let mut report = ImportReport::new("logseq");
        collect(g.path(), &mut report).expect("collect");
        assert_eq!(report.source_pages, 2);
        assert!(
            report.skipped.iter().any(|s| s.path.ends_with("w.edn")),
            "{:?}",
            report.skipped
        );
    }

    /// Config and assets are not pages, so they must not enter the
    /// denominator — but they must still be said out loud.
    #[test]
    fn non_page_files_are_warned_about_not_skipped() {
        let g = graph(&["pages/a.md", "logseq/config.edn", "assets/pic.png"]);
        let mut report = ImportReport::new("logseq");
        collect(g.path(), &mut report).expect("collect");
        assert_eq!(report.source_pages, 1, "config and assets are not pages");
        assert!(report.skipped.is_empty());
        let w = report
            .warnings
            .iter()
            .find(|w| w.detail.contains("not read, and holds no pages"))
            .expect("aggregate warning");
        assert!(w.detail.contains("logseq/"), "{}", w.detail);
        assert!(w.detail.contains("assets/"), "{}", w.detail);
    }

    /// One warning for the whole graph, never one per file.
    #[test]
    fn the_unread_warning_is_aggregated() {
        let g = graph(&[
            "pages/a.md",
            "assets/one.png",
            "assets/two.png",
            "assets/three.png",
        ]);
        let mut report = ImportReport::new("logseq");
        collect(g.path(), &mut report).expect("collect");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0].detail.contains("assets/ (3)"),
            "{}",
            report.warnings[0].detail
        );
    }

    /// A file dropped in the graph root is not a page either, and the
    /// warning has to name it rather than print an empty list.
    #[test]
    fn a_loose_file_in_the_root_is_counted_too() {
        let g = graph(&["pages/a.md", "README.md"]);
        let mut report = ImportReport::new("logseq");
        collect(g.path(), &mut report).expect("collect");
        assert_eq!(report.source_pages, 1, "the root file is not a page");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0]
                .detail
                .contains("1 loose in the graph root"),
            "{}",
            report.warnings[0].detail
        );
    }

    /// An unreadable directory under `pages/` is not an empty one: its
    /// pages would enter neither the denominator nor `skipped`, and the
    /// books would balance over the missing subtree.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_page_directory_fails_the_import() {
        use std::os::unix::fs::PermissionsExt;
        let g = graph(&["pages/a.md", "pages/private/secret.md"]);
        let private = g.path().join("pages/private");
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        // Root reads through any mode bit; there is nothing to test then.
        let readable = std::fs::read_dir(&private).is_ok();
        let mut report = ImportReport::new("logseq");
        let result = collect(g.path(), &mut report);
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755))
            .expect("chmod back");
        if readable {
            return;
        }
        let err = result
            .err()
            .expect("an unlistable page directory is an error");
        assert!(err.to_string().contains("private"), "{err}");
    }

    #[test]
    fn a_clean_graph_warns_about_nothing() {
        let g = graph(&["pages/a.md", "journals/2026_01_01.md"]);
        let mut report = ImportReport::new("logseq");
        collect(g.path(), &mut report).expect("collect");
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }
}
