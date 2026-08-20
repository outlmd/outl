//! Wire types shared by every GUI client.
//!
//! These are the reply shapes the Solid frontends (via `@outl/shared`)
//! deserialize — field names and shapes are part of the wire contract.
//! The `AppState` structs themselves stay in the client crates (their
//! fields differ); only what crosses the Tauri bridge lives here.

use outl_actions::{Backlink, OutlineNode, PageMeta};
use serde::Serialize;

/// Sentinel error returned by workspace-touching commands while the
/// workspace is still being opened (background thread) or while the
/// user hasn't picked one yet. The frontend retries on a short interval.
pub const ERR_LOADING: &str = "workspace_loading";

/// Returned by the `workspace_stats` command.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSummary {
    pub blocks: usize,
    pub ops: usize,
    pub actor: String,
    pub storage_root: String,
    /// `true` when a workspace is loaded; `false` while the picker is
    /// still up or the background opener is in flight.
    pub ready: bool,
}

/// Reply shape for every "open page / open journal" command. Bundles
/// the page meta with the outline so the frontend gets everything in
/// one trip.
///
/// `warnings` is the verbatim `outl_md::ParseWarning` list surfaced by
/// `outl_actions::read_page_outline_with_workspace`; the shared
/// `<ParseWarningsBanner />` consumes it. Empty (or absent) on a clean
/// file — `skip_serializing_if` keeps the JSON quiet.
#[derive(Debug, Clone, Serialize)]
pub struct PageView {
    pub page: PageMeta,
    pub outline: Vec<OutlineNode>,
    pub backlinks: Vec<Backlink>,
    /// Direction `backlinks` was sorted in (`[display] backlinks_order`,
    /// issue #142). Carried on the view so a client's direction toggle
    /// shows the right arrow at boot without a separate settings read.
    /// Serialises as `"newest"` / `"oldest"`.
    pub backlinks_order: outl_config::BacklinksOrder,
    /// The page's own `key:: value` properties, alpha-sorted, with the
    /// structural keys (`page-slug`, `page-kind`) filtered out by
    /// `outl_actions::property::page_properties`.
    ///
    /// Same shape as `OutlineNode.properties`, so both feed the one
    /// editor component. Without this the GUI clients could not even
    /// *see* `icon::` / `type::`, let alone change them, which left the
    /// TUI as the only way to touch page metadata.
    #[serde(default)]
    pub page_properties: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<outl_md::ParseWarning>,
    /// Set when this page's `.md` could not be refreshed from the op log
    /// because it holds content no op accounts for
    /// (`ActionError::PageMarkdownAheadOfLog`). `None` on every healthy
    /// page — `skip_serializing_if` keeps the JSON quiet.
    ///
    /// The page still opens: the guard only withholds the *write*, so
    /// the view below is the `.md` as it stands on disk. What the user
    /// loses is convergence — see [`MdAheadOfLog`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md_ahead_of_log: Option<MdAheadOfLog>,
    /// `true` when this reply actually ran the re-projection check, so
    /// `md_ahead_of_log` is authoritative **in both directions**.
    ///
    /// Only the open commands attempt the re-projection that discovers
    /// the condition; a mutation reply is built from the tree and could
    /// never carry the flag. A client therefore has to keep the banner
    /// across mutation replies (otherwise the user's first edit clears
    /// the warning about editing), and this bit is what tells it when
    /// clearing *is* right: an open reply with no notice means the page
    /// is healthy again — `outl reconcile --ahead-of-log` ran — and a
    /// banner that outlives the condition is the mirror of the silence
    /// [`MdAheadOfLog`] exists to end.
    ///
    /// Deliberately on the wire instead of re-derived per client: both
    /// GUI frontends were guessing it from "same page id / same slug",
    /// and two guesses drift.
    #[serde(default)]
    pub md_ahead_of_log_checked: bool,
}

impl PageView {
    /// Stamp the open path's re-projection verdict onto the view.
    ///
    /// The one place that sets both halves, so an open command cannot
    /// report the notice while leaving the reply looking unchecked (the
    /// clients would then never clear the banner) or the reverse.
    #[must_use]
    pub fn with_ahead_of_log_check(mut self, ahead: Option<MdAheadOfLog>) -> Self {
        self.md_ahead_of_log = ahead;
        self.md_ahead_of_log_checked = true;
        self
    }
}

/// Why a page stopped syncing, in the shape a client can render.
///
/// The re-projection guard (root `CLAUDE.md` invariant 8) refuses to
/// overwrite a `.md` holding content that exists in no op, because that
/// write would delete it for good. The cost of refusing is that the page
/// is frozen in *both* directions: those lines never reach another
/// device, and a peer's edits never reach this `.md`.
///
/// Before this DTO existed the refusal died in a `tracing::warn!` and
/// the page simply stopped updating with nothing said — the exact
/// "silence is the defect" failure [RFC 0210] names. `outl reconcile
/// --ahead-of-log` is the recovery, and a mobile-only device has no
/// binary to run it with, which is why the copy is per-client and lives
/// in `@outl/shared/warnings`.
///
/// [RFC 0210]: https://github.com/outlmd/outl/blob/main/docs/rfcs/0210-md-content-outside-op-log.md
#[derive(Debug, Clone, Serialize)]
pub struct MdAheadOfLog {
    /// The `.md` that was left alone, for the "which file" question.
    pub path: String,
    /// How many content lines exist only on disk.
    pub lines: usize,
    /// One of those lines (already quoted by `ActionError`), so the
    /// banner names what is at risk instead of only counting it.
    pub sample: String,
}

/// Reply shape for the lazy `page_backlinks` command.
///
/// Backlinks are **not** bundled into [`PageView`] anymore:
/// `backlinks_for_page` is an `O(blocks-in-workspace)` scan, and computing
/// it inside `build_page_view` blocked every page open and every block
/// edit synchronously (a ~66k-node workspace made the first journal paint
/// take seconds on desktop, more on mobile). The frontend now fetches
/// backlinks lazily after the outline paints — the same lazy/cached policy
/// the TUI has always used for its backlinks panel. `PageView.backlinks`
/// stays in the wire shape but comes back empty from the open commands.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinksReply {
    pub backlinks: Vec<Backlink>,
    /// Direction the list was sorted in (`[display] backlinks_order`).
    pub backlinks_order: outl_config::BacklinksOrder,
}

/// One hit from `search_blocks` — the `((…))` block-ref autocomplete.
///
/// The frontend inserts `handle` wrapped in `((…))` (never the display
/// `text`: block refs resolve by handle, not by content) and shows
/// `text` + `source_slug` as the suggestion label.
#[derive(Debug, Clone, Serialize)]
pub struct BlockHit {
    /// Ref handle to insert, e.g. `blk-r6s4a1`.
    pub handle: String,
    /// Block text (snippet) for the popup label.
    pub text: String,
    /// Slug of the page hosting the block, for context.
    pub source_slug: String,
}

/// One structural template surfaced by `list_templates` — the `/template`
/// picker in every GUI client. Mirrors the invocation `name` (what the
/// user picks) and the `slug` of the page that defines the body.
///
/// Deliberately narrower than `outl_actions::TemplateEntry`: the GUIs
/// only need the name (label) + slug (secondary label / dedupe key);
/// `page_id` and `params` are backend detail the pick doesn't carry.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateDto {
    /// Invocation name (the value of the page's `template::` property).
    pub name: String,
    /// Slug of the page that defines the template.
    pub slug: String,
    /// `true` when another page shares this `template:: <name>` — the
    /// picker surfaces it so the user knows a duplicate silently
    /// shadows the rest (resolution picks the first in tree order).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
}

/// Reply for `create_block`. Pairs the refreshed [`PageView`] with the
/// id of the freshly-inserted block so the frontend can focus / start
/// editing it without re-discovering the id via a DFS diff (the diff
/// path mis-identified the new block when the anchor had children
/// — `flat[idx+1]` would land on `children[0]` instead of the new
/// sibling, and the eventual `edit_block` would target a stale id and
/// surface the `block <ULID> is not in the tree` toast).
#[derive(Debug, Clone, Serialize)]
pub struct CreateBlockReply {
    pub view: PageView,
    pub new_id: String,
}
