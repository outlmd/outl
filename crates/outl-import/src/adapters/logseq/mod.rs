//! Logseq adapter (markdown graph directory).
//!
//! A Logseq graph is `pages/` + `journals/` of bullet-outlined
//! markdown. This adapter **parses the outline** — every bullet
//! becomes a typed `ImportBlock`, so `id::` UIDs map to real
//! `((blk-XXXXXX))` handles, `collapsed::` survives as
//! `Op::SetCollapsed`, and per-block properties land as outl property
//! lines instead of stray text.
//!
//! One concern per module:
//!
//! - `files` — which files are read, and what is said about the ones
//!   that are not (the reconciliation denominator lives here)
//! - `names` — filename → page title, in either of Logseq's two
//!   encodings (`___` is a **namespace separator**, not a space)
//! - `outline` — a file's text → page properties + block tree
//! - `inline` — the inline dialect: refs, embeds, components
//! - `templates` — a block carrying `template::` → an outl template
//!   page (issue #563)

pub(crate) mod inline;

mod files;
mod names;
mod outline;
mod templates;

#[cfg(test)]
mod tests;

use crate::adapter::{ImportError, SourceAdapter};
use crate::ir::{ImportGraph, ImportPage, PageBody, PageName};
use crate::report::ImportReport;
use names::NameFormat;
use outl_md::slug::slugify;
use std::path::Path;

/// The Logseq adapter.
pub struct LogseqAdapter;

impl SourceAdapter for LogseqAdapter {
    fn id(&self) -> &'static str {
        "logseq"
    }

    /// A graph is recognised by `logseq/` (the config directory every
    /// graph has) or by either content directory.
    ///
    /// Requiring **both** `pages/` and `journals/` used to turn away
    /// two real shapes: a graph whose owner never used journals, and
    /// the directory you get from unzipping Logseq's own markdown
    /// export, which only carries the directories it had content for.
    fn detect(src: &Path) -> bool {
        src.is_dir()
            && (src.join("logseq").is_dir()
                || src.join("pages").is_dir()
                || src.join("journals").is_dir())
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        if !src.is_dir() {
            return Err(ImportError::Parse(unreadable_source_message(src)));
        }
        // A DB-version graph holds db.sqlite and none of pages/ + journals/;
        // scanning it as a file graph would report 0 pages with no error.
        if super::logseq_db::is_db_graph_dir(src) {
            return Err(ImportError::Parse(super::logseq_db::db_graph_dir_message(
                src,
            )));
        }

        let format = names::read_name_format(src);
        let mut graph = ImportGraph::default();
        for file in files::collect(src, report) {
            convert_file(&file.path, file.is_journal, format, &mut graph, report)?;
        }
        // A Logseq block template is a block; an outl template is a
        // page. Lifting runs after every page is parsed so a template
        // can be recognised against the page-level ones already there.
        templates::lift(&mut graph, report);

        if graph.pages.is_empty() && report.skipped.is_empty() {
            report.warn(
                "(graph)",
                "no markdown pages found — check that this is the graph directory (the one \
                 holding pages/ and journals/)",
            );
        }
        Ok(graph)
    }
}

/// What to say when the source is not a graph directory.
///
/// A `.zip` is worth naming on its own: Logseq's "Export graph >
/// Export as standard Markdown" hands you one, and `outl import
/// logseq <that zip>` is the obvious next thing to type.
fn unreadable_source_message(src: &Path) -> String {
    if src.extension().and_then(|e| e.to_str()) == Some("zip") {
        return format!(
            "{} is a zip archive. Logseq's \"Export as standard Markdown\" produces one — \
             unzip it and point outl at the directory it contains (the one holding pages/ \
             and journals/). For a DB graph, \"Export graph > Export EDN file\" and `outl \
             import logseq-db <export.edn>` keeps more: refs, task state and properties",
            src.display()
        );
    }
    format!(
        "logseq source must be the graph directory (got {})",
        src.display()
    )
}

/// One Logseq `.md` file → one [`ImportPage`].
fn convert_file(
    src: &Path,
    is_journal: bool,
    format: NameFormat,
    graph: &mut ImportGraph,
    report: &mut ImportReport,
) -> Result<(), ImportError> {
    let text = std::fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let decoded = names::decode_page_name(stem, format);

    let label = decoded.clone();
    // Relative asset links (`![](../assets/x.png)`) resolve against the
    // `.md` file's own directory (`pages/` or `journals/`), so
    // `../assets/x.png` lands on `<graph>/assets/x.png`.
    let base_dir = src.parent().unwrap_or(src).to_path_buf();
    let (mut props, blocks) = outline::parse_outline(&text, &base_dir, &mut graph.assets, report);
    report.count_source(0, blocks.iter().map(outline::count_blocks).sum::<usize>());

    // `title::` / `#+title:` overrides the display name; the file
    // keeps its filename-derived slug so existing `[[refs]]` resolve.
    let mut stem_override = None;
    let title_prop = props
        .iter()
        .position(|(k, _)| k == "title")
        .map(|i| props.remove(i).1);

    let name = if is_journal {
        match journal_date(stem) {
            Some(d) => PageName::Journal(d),
            None => {
                report.warn(
                    &label,
                    "journal filename doesn't parse as a date — imported as a regular page",
                );
                PageName::Named(decoded)
            }
        }
    } else {
        match title_prop {
            Some(t) if t != decoded => {
                stem_override = Some(slugify(&decoded));
                PageName::Named(t)
            }
            _ => PageName::Named(decoded),
        }
    };

    graph.pages.push(ImportPage {
        name,
        props,
        body: PageBody::Outline(blocks),
        stem_override,
        created: None,
        edited: None,
        source_uid: None,
    });
    Ok(())
}

/// A journal filename → its date.
///
/// `:journal/file-name-format` is configurable, so the stem is tried as
/// written first (`2026-05-25`, `Oct 20th, 2026`) and then with its
/// separators normalised, which covers Logseq's `yyyy_MM_dd` default
/// and the `yyyy.MM.dd` some graphs use.
fn journal_date(stem: &str) -> Option<chrono::NaiveDate> {
    outl_actions::parse_flexible_date(stem)
        .or_else(|| outl_actions::parse_flexible_date(&stem.replace(['_', '.'], "-")))
}
