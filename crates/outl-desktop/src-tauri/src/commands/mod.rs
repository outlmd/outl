//! Tauri command surface for `outl-desktop`.
//!
//! Split by responsibility so the file-size guard stays happy and
//! each module has one job:
//!
//! - [`workspace`] — pick / open / reload the workspace, surface
//!   stats, resolve refs.
//! - [`page`] — open and navigate pages and journals.
//! - [`block`] — every block mutation (create, edit, todo, indent,
//!   move, paste, collapsed).
//! - [`history`] — undo / redo of committed block mutations.
//! - [`timeline`] — a page's history out of the op log (read-only).
//!   Distinct from [`history`]: that is this session's undo stack.
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
pub(crate) mod shortcuts;
pub(crate) mod template;
pub(crate) mod theme;
pub(crate) mod timeline;
pub(crate) mod workspace;

pub(crate) use asset::*;
pub(crate) use block::*;
pub(crate) use exec::*;
pub(crate) use history::*;
pub(crate) use page::*;
pub(crate) use peers::*;
pub(crate) use plugin::*;
pub(crate) use property::*;
pub(crate) use reminders::*;
pub(crate) use shortcuts::*;
pub(crate) use template::*;
pub(crate) use theme::*;
pub(crate) use timeline::*;
pub(crate) use workspace::*;
