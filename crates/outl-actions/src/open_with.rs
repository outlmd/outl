//! Opening an external `.md` / `.txt` file *into* the workspace.
//!
//! The OS-level "Open With → outl" gesture hands a client a path to a
//! file that lives **outside** the workspace. outl is not a text
//! editor and never writes back to that file: the gesture means
//! *import a copy of this into my outline*, so what lands is a normal
//! page whose content arrived through the op log like any other.
//!
//! # Where it lands
//!
//! Under the [`OPEN_WITH_NAMESPACE`] namespace — `open-in/<file stem>`
//! — which is a **title**, not a slug. The slash is the whole point:
//! `open-in` becomes a real parent page listing everything ever opened
//! this way (root `CLAUDE.md` invariant on nested pages, issue #275),
//! and the slug it projects to (`open-in-<stem>`) stays a single path
//! component the way [`crate::page::is_valid_slug`] requires.
//!
//! # Re-opening the same file
//!
//! Importing twice into one page would duplicate every block, and
//! overwriting the page would delete whatever the user wrote after the
//! import. Neither is acceptable, so the page records where it came
//! from in a `source::` property and this module **refuses to import
//! again** — [`resolve_target`] returns [`OpenWithTarget::Existing`]
//! and the client just navigates there.
//!
//! That property is also what keeps two *different* files with the
//! same name apart: `~/a/notes.md` and `~/b/notes.md` both want
//! `open-in/notes`, and the second one gets `open-in/notes 2` rather
//! than being merged into the first. Without `source::` the merge
//! would be silent, which is the failure mode this crate's invariants
//! keep being written about.

use std::path::Path;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::page::open_or_create;
use crate::page::{find_by_slug, page_id_from_slug, read_text_prop, PageKind};
use crate::paste::{paste_markdown, PasteAnchor};

/// Namespace every externally opened file lands under.
///
/// A title prefix, not a slug prefix — see the module docs.
pub const OPEN_WITH_NAMESPACE: &str = "open-in";

/// Page property recording the absolute path the page was imported
/// from. Read back by [`resolve_target`] to tell "the user re-opened
/// the same file" from "two different files share a name".
pub const SOURCE_KEY: &str = "source";

/// File extensions the import accepts.
///
/// Matched case-insensitively. Anything else is refused up front with
/// [`ActionError::UnsupportedExternalFile`] rather than being read and
/// turned into a page of mojibake — a `.pdf` dropped on the app is a
/// mistake worth naming, not content worth importing.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "text"];

/// Hard cap on an imported file, in bytes.
///
/// Importing is a synchronous parse into one block per line, so
/// an unbounded file is a hang. 16 MiB is far past any hand-written
/// note and far short of the size where the outline stops being
/// usable.
pub const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;

/// How many `open-in/<stem> N` variants to try before giving up.
const MAX_DISAMBIGUATION: usize = 100;

/// Stem used when the file's own name has nothing a slug can keep.
const UNTITLED_STEM: &str = "untitled";

/// Where an "Open With" gesture should land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenWithTarget {
    /// This exact file was already imported. Navigate, import nothing.
    Existing {
        /// The existing page's root node.
        page: NodeId,
        /// Its slug, for the client's page view.
        slug: String,
        /// Its namespaced title (`open-in/<stem>`).
        title: String,
    },
    /// No page holds this file yet. The client creates it via
    /// [`import_into`].
    New {
        /// Slug the page will project to (`slugify(title)`).
        slug: String,
        /// Namespaced title to create the page under.
        title: String,
        /// The `source::` value this target was resolved against.
        ///
        /// Carried rather than recomputed at import time, so the value
        /// written is byte-for-byte the one [`resolve_target`]
        /// compared. Recomputing it meant a file that vanished between
        /// the two calls fell back to the uncanonicalised path, and the
        /// next open then failed to match its own page and minted
        /// `open-in/<stem> 2` — the exact duplicate `source::` exists
        /// to prevent.
        source: String,
    },
}

impl OpenWithTarget {
    /// The page's root node id, known ahead of creation because ids
    /// are derived from the slug ([`page_id_from_slug`]).
    ///
    /// This is what lets a client run the whole import inside one
    /// `commit_page` — it needs the node before the mutation runs.
    pub fn page_id(&self) -> NodeId {
        match self {
            Self::Existing { page, .. } => *page,
            Self::New { slug, .. } => page_id_from_slug(slug),
        }
    }

    /// The page's slug either way.
    pub fn slug(&self) -> &str {
        match self {
            Self::Existing { slug, .. } | Self::New { slug, .. } => slug,
        }
    }

    /// The page's namespaced title either way.
    pub fn title(&self) -> &str {
        match self {
            Self::Existing { title, .. } | Self::New { title, .. } => title,
        }
    }
}

/// Whether `path`'s extension is one this import accepts.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Read `path` as UTF-8 text, refusing an unsupported extension, an
/// oversized file, or bytes that are not text.
///
/// Lives here rather than in each client so the three refusals are
/// worded once. The size check reads one byte past the cap so "exactly
/// at the limit" and "over it" stay distinguishable (same shape as
/// [`crate::asset::import_asset`]).
pub fn read_source(path: &Path) -> Result<String, ActionError> {
    use std::io::Read as _;

    if !is_supported(path) {
        return Err(ActionError::UnsupportedExternalFile(display_path(path)));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_IMPORT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(ActionError::ExternalFileTooLarge {
            path: display_path(path),
            limit: MAX_IMPORT_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|_| ActionError::ExternalFileNotText(display_path(path)))
}

/// Decide where `path` should open, without mutating anything.
///
/// Walks `open-in/<stem>`, `open-in/<stem> 2`, … until it finds either
/// the page already holding this exact `source::` (→
/// [`OpenWithTarget::Existing`]) or a free slug (→
/// [`OpenWithTarget::New`]).
pub fn resolve_target(workspace: &Workspace, path: &Path) -> Result<OpenWithTarget, ActionError> {
    let stem = file_stem(path);
    let source = source_key(path);
    let base = format!("{OPEN_WITH_NAMESPACE}/{stem}");

    for attempt in 1..=MAX_DISAMBIGUATION {
        let title = if attempt == 1 {
            base.clone()
        } else {
            format!("{base} {attempt}")
        };
        let slug = slug_for(&title);
        match find_by_slug(workspace, &slug) {
            None => {
                return Ok(OpenWithTarget::New {
                    slug,
                    title,
                    source,
                })
            }
            Some(page) => {
                if read_text_prop(workspace, page, SOURCE_KEY).as_deref() == Some(source.as_str()) {
                    return Ok(OpenWithTarget::Existing { page, slug, title });
                }
            }
        }
    }
    Err(ActionError::ExternalFileNameExhausted(base))
}

/// Create the page `target` names and import `contents` into it.
///
/// Only valid for [`OpenWithTarget::New`]; an `Existing` target is a
/// no-op returning its own page, because re-importing is exactly what
/// this module refuses to do (see the module docs).
///
/// Every mutation goes through the op log: the page root and its
/// `slug::` / `kind::` / `title::` properties via
/// [`open_or_create`], the `source::` marker via
/// [`crate::page::set_property`], and the content via
/// [`paste_markdown`] — which is the same pipeline a clipboard paste
/// uses, so a bulleted `.md` lands as an outline and prose lands as
/// one block per non-blank line.
///
/// Takes no path: the `source::` value rides on the target, so what is
/// written is what [`resolve_target`] matched against.
pub fn import_into(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    target: &OpenWithTarget,
    contents: &str,
) -> Result<NodeId, ActionError> {
    let OpenWithTarget::New {
        slug,
        title,
        source,
    } = target
    else {
        return Ok(target.page_id());
    };
    // The slug comes off the target, never re-derived from the title.
    // `resolve::open_or_create_by_name` would slugify again, which is how a
    // non-ASCII stem reached the namespace page (see `slug_for`), and
    // a second derivation of one fact is the bug shape this module
    // already paid for once with `source::`.
    let page = open_or_create(workspace, hlc, slug, title, PageKind::Page)?;
    crate::page::set_property(
        workspace,
        hlc,
        page,
        SOURCE_KEY,
        Some(PropValue::Text(source.clone())),
    )?;
    if !contents.trim().is_empty() {
        paste_markdown(workspace, hlc, PasteAnchor::AsLastChildOf(page), contents)?;
    }
    Ok(page)
}

/// The slug `title` projects to, refusing to land on the namespace's
/// own page.
///
/// `slugify` drops every character it cannot fold to ASCII, so a stem
/// written in a non-Latin script — `会議メモ`, `Проект`, `📝` — folds
/// away entirely and `open-in/<stem>` collapses to bare `open-in`.
/// That is the **parent page**: importing there would replace the
/// namespace index with one file's contents, and every later
/// `open-in/<other>` would hang under it. For a user who names files
/// in their own script that is not an edge case, it is every file.
///
/// The title keeps the original stem either way — only the slug falls
/// back, so the page still reads as `open-in/会議メモ` in the UI while
/// projecting to a filename that exists.
fn slug_for(title: &str) -> String {
    let slug = outl_md::slug::slugify(title);
    let namespace = outl_md::slug::slugify(OPEN_WITH_NAMESPACE);
    if slug == namespace {
        format!("{namespace}-{UNTITLED_STEM}")
    } else {
        slug
    }
}

/// The file's display stem, with a fallback so a dotfile or a path
/// that ends in a separator still produces a nameable page.
fn file_stem(path: &Path) -> String {
    let raw = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .unwrap_or_default();
    if raw.is_empty() {
        UNTITLED_STEM.to_string()
    } else {
        raw.to_string()
    }
}

/// Canonical form of the source path, used as the `source::` value.
///
/// Canonicalised when the file is reachable so a symlink and its
/// target do not read as two different sources; falls back to the path
/// as given when it is not (the file may have moved since the page was
/// created — that must not turn into a duplicate import).
fn source_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Path rendered for an error message.
fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests;
