//! Tauri command surface for `outl-mobile`.
//!
//! Split by responsibility so the file-size guard stays happy and
//! each module has one job:
//!
//! - [`workspace`] — reload + workspace stats.
//! - [`page`] — open / navigate pages and journals, search.
//! - [`block`] — every block mutation (create, edit, todo, indent,
//!   move, paste, collapsed).
//! - [`exec`] — `run_code_block` Tauri shim over `outl_actions::exec`.
//! - [`history`] — undo / redo of committed block mutations, thin
//!   wrappers over `outl_tauri_shared::commands::history` (RFC 0254
//!   phase 1 — previously desktop-only).
//! - [`theme`] — thin wrappers over `outl_tauri_shared::commands::theme`.
//!
//! Every command is re-exported at this level so
//! `tauri::generate_handler!` in `lib.rs` doesn't have to know about
//! the file split.

pub(crate) mod asset;
pub(crate) mod block;
pub(crate) mod exec;
pub(crate) mod history;
pub(crate) mod page;
pub(crate) mod peers;
pub(crate) mod plugin;
pub(crate) mod property;
pub(crate) mod reminders;
pub(crate) mod template;
pub(crate) mod theme;
pub(crate) mod workspace;

pub(crate) use asset::*;
pub(crate) use block::*;
pub(crate) use history::*;
pub(crate) use page::*;
pub(crate) use peers::*;
pub(crate) use plugin::*;
pub(crate) use property::*;
pub(crate) use reminders::*;
pub(crate) use template::*;
pub(crate) use theme::*;
pub(crate) use workspace::*;
