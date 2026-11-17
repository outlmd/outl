//! Vault discovery + slug-collision disambiguation for the Obsidian
//! adapter.
//!
//! outl is flat, so two source files whose H1 (or frontmatter
//! `title`, or filename stem) produce the same slug would silently
//! overwrite each other. [`discover`] walks the vault once up front
//! and computes each file's base slug; [`assign_unique_stems`] then
//! gives every colliding file a slug-safe suffix derived from its
//! source path (`ideas` + `Docs/Ideas/Ideas.md` → `ideas-ideas`). The
//! lex-smallest relative path wins the bare slug. The user-visible
//! `title::` is unaffected — only the on-disk filename changes.

use crate::adapter::ImportError;
use crate::report::ImportReport;
use outl_md::frontmatter::{extract_leading_h1, split_frontmatter};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Vault files that are documents in their own right, so a user counts
/// them as content the import either took or refused. Each lands in
/// `skipped` by name, and in the reconciliation's page denominator.
///
/// `.canvas` is Obsidian's JSON graph; `.base` is the Bases format.
/// Neither has an outl equivalent, which is fine — being skipped
/// silently is not.
const DOCUMENT_EXTENSIONS: &[(&str, &str)] = &[
    (
        "canvas",
        "an Obsidian canvas (a JSON graph) has no outl equivalent yet",
    ),
    (
        "base",
        "an Obsidian base (the Bases format) has no outl equivalent yet",
    ),
];

/// One source file we discovered during the walk, plus the slug we'd
/// assign it in the absence of collisions.
pub(super) struct DiscoveredFile {
    pub path: PathBuf,
    pub base_slug: String,
}

/// Walk the vault and compute the base slug for every `.md` file,
/// accounting for every other file on the way past it.
///
/// The `continue` that used to skip a non-markdown file is the reason
/// `board.canvas` and `sub/deep.base` left no trace in a dry run: not
/// in `skipped`, not in `warnings`, and with no page denominator the
/// reconciliation never ran either (issue #299).
pub(super) fn discover(
    src: &Path,
    report: &mut ImportReport,
) -> Result<Vec<DiscoveredFile>, ImportError> {
    let mut discovered: Vec<DiscoveredFile> = Vec::new();
    let mut attachments: BTreeMap<String, usize> = BTreeMap::new();
    for entry in walkdir::WalkDir::new(src)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
        if ext != "md" {
            let rel = path.strip_prefix(src).unwrap_or(path).display().to_string();
            match DOCUMENT_EXTENSIONS.iter().find(|(e, _)| *e == ext) {
                Some((_, why)) => {
                    // A document the vault holds and outl will not: in
                    // the denominator, and named in `skipped`.
                    report.count_source_pages(1);
                    report.skip(rel, *why, 0);
                }
                // An attachment is not a page, so it stays out of the
                // page denominator — but it is still said out loud, in
                // one aggregate line rather than one per file.
                None => {
                    *attachments
                        .entry(if ext.is_empty() {
                            "(no extension)".to_string()
                        } else {
                            format!(".{ext}")
                        })
                        .or_insert(0) += 1;
                }
            }
            continue;
        }
        let text = fs::read_to_string(path).map_err(|e| ImportError::io(path, e))?;
        let title = derive_title(path, &text);
        let slug = outl_md::slug::slugify(&title);
        report.count_source_pages(1);
        discovered.push(DiscoveredFile {
            path: path.to_path_buf(),
            base_slug: slug,
        });
    }
    if !attachments.is_empty() {
        let kinds: Vec<String> = attachments
            .iter()
            .map(|(ext, n)| format!("{n} {ext}"))
            .collect();
        report.warn(
            "(graph)",
            format!(
                "not read, and not notes: {}. A file a note links to is still pulled into \
                 assets/; `.obsidian/` and other dotfiles are vault config",
                kinds.join(", ")
            ),
        );
    }
    Ok(discovered)
}

/// True for entries that should not be visited at all (and not
/// descended into, when the entry is a directory). Anything starting
/// with a dot at depth > 0 is treated as app metadata — covers
/// `.obsidian/`, `.trash/`, `.git/`, per-file dotfiles, etc.
fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.') && entry.depth() > 0)
}

/// Derive the page title for a discovered file, using the same
/// resolution order as the converter: frontmatter `title` → leading
/// H1 → filename stem.
fn derive_title(src: &Path, text: &str) -> String {
    let (frontmatter, body_after_fm) = split_frontmatter(text);
    if let Some(yaml) = frontmatter.as_deref() {
        if let Some(fm) = super::parse_obsidian_frontmatter(yaml) {
            if let Some(title) = fm.title {
                return title;
            }
        }
    }
    if let (Some(h1), _) = extract_leading_h1(&body_after_fm) {
        return h1;
    }
    src.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

/// Build a `source_path → final_stem` map. For each collision group
/// the entry whose relative path is lex-smallest keeps the bare slug;
/// every other entry gets a path-derived suffix until its stem is
/// unique inside the map.
pub(super) fn assign_unique_stems(
    files: &[DiscoveredFile],
    vault_root: &Path,
) -> HashMap<PathBuf, String> {
    let mut by_slug: HashMap<String, Vec<&DiscoveredFile>> = HashMap::new();
    for f in files {
        by_slug.entry(f.base_slug.clone()).or_default().push(f);
    }

    let mut assigned: HashMap<PathBuf, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();

    // Deterministic order across runs and platforms.
    let mut slug_keys: Vec<String> = by_slug.keys().cloned().collect();
    slug_keys.sort();

    for slug in &slug_keys {
        let group = by_slug.get(slug).expect("key came from by_slug");
        let mut sorted: Vec<&DiscoveredFile> = group.clone();
        sorted.sort_by(|a, b| {
            a.path
                .strip_prefix(vault_root)
                .unwrap_or(&a.path)
                .cmp(b.path.strip_prefix(vault_root).unwrap_or(&b.path))
        });

        for (i, f) in sorted.iter().enumerate() {
            let stem = if i == 0 && !used.contains(slug) {
                slug.clone()
            } else {
                disambiguate_stem(slug, &f.path, vault_root, &used)
            };
            used.insert(stem.clone());
            assigned.insert(f.path.clone(), stem);
        }
    }

    assigned
}

/// Compute a path-derived alternative stem for `path` whose base slug
/// collided. Tries, in order: `<base>-<immediate-folder>`,
/// `<base>-<filename-stem>`, `<base>-<folder>-<filename-stem>`,
/// `<base>-<N>` (numeric fallback).
fn disambiguate_stem(base: &str, path: &Path, vault_root: &Path, used: &HashSet<String>) -> String {
    let rel = path.strip_prefix(vault_root).unwrap_or(path);
    let parent_name = rel
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| outl_md::slug::slugify(&n.to_string_lossy()))
        .unwrap_or_default();
    let file_stem = rel
        .file_stem()
        .map(|s| outl_md::slug::slugify(&s.to_string_lossy()))
        .unwrap_or_default();

    let candidates: [String; 3] = [
        format!("{base}-{parent_name}"),
        format!("{base}-{file_stem}"),
        format!("{base}-{parent_name}-{file_stem}"),
    ];
    for candidate in &candidates {
        if !candidate.ends_with('-') && !used.contains(candidate) {
            return candidate.clone();
        }
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !used.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}
