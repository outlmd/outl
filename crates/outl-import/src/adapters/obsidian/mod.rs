//! Obsidian adapter (vault directory).
//!
//! An Obsidian vault is free markdown, not an outline — so this
//! adapter produces [`PageBody::Raw`] bodies: text-level rewrites
//! only, outl's permissive parser owns the block structure after
//! emission. What gets translated:
//!
//! - **YAML frontmatter** between `---` fences → `key:: value` page
//!   properties via [`outl_md::frontmatter`] (the generic owner).
//!   Obsidian policy lives here: `aliases`, `cssclass`, `publish`,
//!   `scroll` are dropped + counted; a scalar `date` is normalized to
//!   ISO. Malformed YAML is restored verbatim into the body — never
//!   silently lost.
//! - **Wiki-link variants** (`[[Note|alias]]`, `[[Note#heading]]`,
//!   `[[Note^block-id]]`, `[[folder/Note]]`) collapse to `[[Note]]`
//!   via [`outl_md::wikilink::rewrite_wikilinks`]; image wiki-links
//!   become CommonMark links via
//!   [`outl_md::wikilink::convert_image_links`].
//! - **Title**: frontmatter `title` → leading `# H1` → filename stem.
//! - **Daily notes**: filename stem or resolved title parsing as a
//!   date routes to `journals/<iso>.md`; a non-date file inside a
//!   daily-notes folder stays a regular page (re-titling it would be
//!   data loss).
//! - **Nested folders** → a `path::` page property; outl is flat.
//! - **Slug collisions** → the `stems` pre-pass assigns unique
//!   on-disk stems (lex-smallest path wins the bare slug); the
//!   emitter honours them via `stem_override`.
//! - **`.obsidian/`, `.trash/`, dotfiles** → skipped wholesale.

mod stems;

use crate::adapter::{ImportError, SourceAdapter};
use crate::adapters::asset_scan::scan_assets;
use crate::ir::{ImportGraph, ImportPage, PageBody, PageName};
use crate::report::ImportReport;
use outl_md::frontmatter::{extract_leading_h1, split_frontmatter, Frontmatter};
use outl_md::wikilink::{convert_image_links, rewrite_wikilinks};
use std::fs;
use std::path::Path;

/// The Obsidian adapter.
pub struct ObsidianAdapter;

impl SourceAdapter for ObsidianAdapter {
    fn id(&self) -> &'static str {
        "obsidian"
    }

    fn detect(src: &Path) -> bool {
        src.is_dir() && src.join(".obsidian").is_dir()
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        if !src.is_dir() {
            return Err(ImportError::Parse(format!(
                "obsidian source must be the vault directory (got {})",
                src.display()
            )));
        }

        // Pass 1+2: discover every file and resolve slug collisions
        // before anything is converted.
        let discovered = stems::discover(src)?;
        let stem_map = stems::assign_unique_stems(&discovered, src);
        let collisions = discovered
            .iter()
            .filter(|d| stem_map.get(&d.path).is_some_and(|s| s != &d.base_slug))
            .count();
        report.slug_collisions += collisions;

        let mut graph = ImportGraph::default();
        for d in &discovered {
            let stem = stem_map
                .get(&d.path)
                .cloned()
                .unwrap_or_else(|| d.base_slug.clone());
            convert_file(&d.path, src, stem, &mut graph, report)?;
        }
        Ok(graph)
    }
}

// --- frontmatter policy ----------------------------------------------------

/// Frontmatter keys that are Obsidian-only app metadata with no outl
/// equivalent. Dropped (and counted in the import report).
const OBSIDIAN_DROPPED_KEYS: &[&str] = &["aliases", "cssclass", "publish", "scroll"];

/// Parse an Obsidian YAML frontmatter block: the generic
/// [`outl_md::frontmatter::parse_frontmatter`] does the structural
/// work; this wrapper supplies the Obsidian drop-list and normalizes
/// a scalar `date` to ISO. Returns `None` when the YAML fails to
/// parse — callers restore the fenced block verbatim into the body.
fn parse_obsidian_frontmatter(yaml: &str) -> Option<Frontmatter> {
    let mut fm = outl_md::frontmatter::parse_frontmatter(yaml, OBSIDIAN_DROPPED_KEYS)?;
    for (key, value) in fm.props.iter_mut() {
        if key == "date" {
            if let Some(iso) = outl_actions::parse_date_label(value) {
                *value = iso;
            }
        }
    }
    Some(fm)
}

// --- per-file conversion ---------------------------------------------------

fn convert_file(
    src: &Path,
    vault_root: &Path,
    assigned_stem: String,
    graph: &mut ImportGraph,
    report: &mut ImportReport,
) -> Result<(), ImportError> {
    let text = fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;

    let rel = src.strip_prefix(vault_root).unwrap_or(src);
    let parent_rel: Option<String> = rel
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty() && s != ".");

    let file_stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // 1. Split + parse frontmatter. Malformed YAML → restored
    //    verbatim into the body so no user content is lost.
    let (frontmatter, body_after_fm) = split_frontmatter(&text);
    let (fm_title, fm_props, body_after_fm) = match frontmatter.as_deref() {
        Some(yaml) => match parse_obsidian_frontmatter(yaml) {
            Some(fm) => {
                report.artifacts_stripped += fm.dropped;
                (fm.title, fm.props, body_after_fm)
            }
            None => {
                report.artifacts_stripped += 1;
                report.warn(
                    &file_stem,
                    "malformed YAML frontmatter restored verbatim into the body",
                );
                let restored = format!("---\n{yaml}---\n\n{body_after_fm}");
                (None, Vec::new(), restored)
            }
        },
        None => (None, Vec::new(), body_after_fm),
    };

    // 2. Title: frontmatter `title` → leading H1 → filename. The H1
    //    is only stripped when it actually becomes the title.
    let (title, body_after_title) = if let Some(t) = fm_title {
        (t, body_after_fm)
    } else {
        match extract_leading_h1(&body_after_fm) {
            (Some(h1), rest) => (h1, rest),
            (None, _) => (file_stem.clone(), body_after_fm),
        }
    };

    // 3. Journal vs page: filename stem first (canonical Obsidian
    //    daily-note shape), title as fallback.
    let name = match outl_actions::parse_flexible_date(&file_stem)
        .or_else(|| outl_actions::parse_flexible_date(&title))
    {
        Some(d) => PageName::Journal(d),
        None => PageName::Named(title),
    };
    let is_journal = matches!(name, PageName::Journal(_));

    // 4. Text-level rewrites: image wiki-links → CommonMark, then the
    //    remaining wiki-link variants collapse to `[[Note]]`. Finally,
    //    scan the resulting CommonMark links for assets to pull in —
    //    relative targets resolve against the note's own directory.
    let body = rewrite_wikilinks(&convert_image_links(&body_after_title));
    let note_dir = src.parent().unwrap_or(vault_root);
    let body = scan_assets(&body, note_dir, &mut graph.assets);

    // 5. Page properties: frontmatter props + `path::` (non-journal —
    //    journals already live in `journals/`).
    let mut props = fm_props;
    if let Some(p) = parent_rel {
        if !is_journal {
            props.push(("path".to_string(), p));
        }
    }

    graph.pages.push(ImportPage {
        name,
        props,
        body: PageBody::Raw(body),
        stem_override: (!is_journal).then_some(assigned_stem),
        created: None,
        edited: None,
    });
    Ok(())
}
