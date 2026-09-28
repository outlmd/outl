//! The data model a parsed page is expressed in.
//!
//! Pure data: the outline tree ([`OutlineNode`]), the page that owns
//! it ([`ParsedPage`]), and the non-fatal recoveries the parser
//! records while reading it ([`ParseWarning`] / [`ParseWarningKind`]).
//! No parsing logic lives here — the grammar and the reader are in
//! [`crate::parse`], which re-exports every type below so
//! `outl_md::parse::OutlineNode` (and `outl_md::OutlineNode`) keep
//! resolving.

use serde::{Deserialize, Serialize};

/// One node in the outline AST. Same shape regardless of depth.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineNode {
    /// Block content (markdown inline, no `- ` prefix, no property lines).
    pub text: String,
    /// Properties attached to this block.
    pub properties: Vec<(String, String)>,
    /// Children of this block (depth-first).
    pub children: Vec<OutlineNode>,
}

/// Parsed page: top-level properties plus the outline tree.
///
/// `warnings` accumulates non-fatal grammar deviations the parser
/// recovered from at the **top level** — a markdown heading
/// (`# title`) where outl expects a `- bullet`, a free paragraph,
/// imported markdown, an over-indented snippet that landed before
/// its parent bullet. Each such line is preserved verbatim as a
/// regular block and the recovery is recorded in [`ParseWarning`]
/// (see [`ParseWarningKind`] for the catalog).
///
/// Scope today is top-level only. Lines nested under a bullet that
/// the grammar can't classify (and aren't valid continuation /
/// property / child) are still skipped by `parse_block_list`'s
/// inner loop — they don't surface as warnings yet. Expanding the
/// catalog to cover nested recovery is a follow-up; the catalog
/// here documents what the parser actually emits today, nothing more.
///
/// Surfaces (`outl-tui`, `outl-mobile`, `outl-desktop`, `outl doctor`)
/// render the warning list so the user can clean the file. outl
/// keeps working in the meantime.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPage {
    /// Verbatim body of a leading YAML frontmatter fence, delimiters
    /// stripped — `None` for a file that has none (the normal case).
    ///
    /// Outl does not model YAML, and does not read these keys: the fence
    /// is **preserved**, not interpreted. It is split off by
    /// [`crate::frontmatter::split_frontmatter_counted`], written back
    /// byte-for-byte by [`crate::render::render`], and carried through the
    /// op log under [`crate::frontmatter::PAGE_FRONTMATTER_KEY`] so a
    /// projection from the tree re-emits it.
    ///
    /// Deliberately **not** a member of `properties`. The `key:: value`
    /// dialect is single-line, user-visible and user-editable, and every
    /// consumer of `properties` (the TUI's property overlay, the Hugo
    /// export, the paste normalizer) assumes all three. A multi-line
    /// opaque blob wearing that shape would have to be filtered out of
    /// each of them by hand, and a hand-maintained exemption in three
    /// places is the drift this field prevents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<String>,
    /// Page-level properties (the lines above the first outline item).
    pub properties: Vec<(String, String)>,
    /// Root-level outline blocks.
    pub blocks: Vec<OutlineNode>,
    /// Lines the parser preserved verbatim because they didn't match
    /// the outl dialect. Empty on a clean file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ParseWarning>,
}

/// A non-fatal recovery the parser performed while reading a `.md`.
///
/// Every warning carries the **1-based** source line number and the
/// raw line text, so a surface can highlight the exact offending row
/// without re-scanning the file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    /// 1-based line number in the source `.md`.
    pub line: usize,
    /// The offending line, verbatim (no trim).
    pub raw: String,
    /// Why the parser had to recover.
    pub kind: ParseWarningKind,
}

/// Catalog of recoveries the parser may perform.
///
/// Add a variant here when a new shape of "user wrote something the
/// dialect doesn't natively support" is detected. Keep the variant
/// name descriptive — UIs render it verbatim as a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseWarningKind {
    /// A line at the top level (or at a block's expected child slot)
    /// that doesn't start with `- ` and isn't a recognized property
    /// — typically a markdown heading (`# title`), a paragraph, an
    /// HTML snippet, or a table. The parser preserves it as a block
    /// with the raw text so a later edit + save doesn't drop content.
    UnrecognizedBlockMarker,

    /// `remind:: every 1h` — a repeat with nothing to repeat from.
    /// The rule needs an explicit anchor (`10am`, `15:00`, `now`).
    RemindMissingAnchor,
    /// `remind:: 25:00` — the anchor (or a `until` time) isn't a
    /// wall-clock time this dialect recognises.
    RemindInvalidTime,
    /// `remind:: 10am every 30s` — interval below the 1min floor, or
    /// a unit outside `min` / `h` / `d`.
    RemindInvalidInterval,
    /// `remind:: 10am until yesterday` — the stop clause is neither
    /// `DONE`, a time, nor an ISO date. Also emitted when a `until
    /// TIME` lands at-or-before the anchor: the clause is dropped and
    /// the rest of the rule still schedules.
    RemindInvalidStop,
    /// `remind:: 10am max 50` — clamped down to the 10-fire ceiling.
    /// The rule still schedules; only the count changed.
    RemindMaxClamped,
}
