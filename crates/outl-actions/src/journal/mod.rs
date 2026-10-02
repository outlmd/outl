//! Page-level `.md` projections.
//!
//! The `.md` file is a **projection** of the materialised tree —
//! never the source of truth. Clients regenerate the projection after
//! every workspace mutation so the user can read it from Finder /
//! Files app / `cat`.
//!
//! Layout inside the workspace root — `root` is the directory whose
//! immediate children are `journals/`, `pages/`, `ops/` (the caller
//! resolves it; see `paths::journals_dir` for the contract — re-joining
//! `Documents/outl` here once double-nested the layout):
//!
//! ```text
//! <workspace root>/
//! ├── journals/
//! │   └── YYYY-MM-DD.md            ← journal pages (page-kind = "journal")
//! └── pages/
//!     └── <slug>.md                ← regular pages (page-kind = "page")
//! ```
//!
//! Split by responsibility, one owner each:
//!
//! - `paths` — directory layout + atomic write + projection removal
//! - `render` — tree → markdown string (page and single-block forms)
//! - `guard` — the "may these bytes be overwritten" verdicts
//!   (invariant 8, both write gates ask the same three questions)
//! - `write` — the locked on-disk transaction (`.md` + sidecar)
//! - `apply` — the `apply_*` family + the workspace-wide sweep: which
//!   guard runs when, and what the caller gets back
//! - `mutate` — the `.md`-as-source-of-truth rewrite path
//! - `survey` — where every page's `.md` stands, and the executor that
//!   re-projects the stale ones
//!
//! The public surface is re-exported here so every existing
//! `outl_actions::journal::*` (and crate-root) path keeps compiling
//! unchanged.

pub(crate) mod apply;
mod guard;
mod mutate;
mod paths;
mod render;
mod sidecar;
mod survey;
pub(crate) mod write;

#[cfg(test)]
mod tests;

pub use apply::{
    apply_all_pages_md, apply_page_md, apply_page_md_with_sidecar,
    apply_page_md_with_sidecar_guarded, apply_page_md_with_sidecar_if_absent,
    apply_page_md_with_sidecar_if_stale, ProjectionFailure, ProjectionSweep,
};
pub use guard::{content_lines_missing_from, sidecar_can_answer, sidecar_can_vouch_for};
pub use mutate::mutate_page_md;
pub use paths::{journals_dir, page_md_path, pages_dir, remove_page_projection, write_md_atomic};
pub use render::{render_block_md, render_page_md};
pub use survey::{
    reproject_stale_pages, survey_page_projections, PageProjection, PageProjectionState,
    ReprojectionSweep, UnreadablePage, WithheldPage,
};
