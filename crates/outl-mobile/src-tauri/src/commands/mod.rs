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
//!
//! Every command is re-exported at this level so
//! `tauri::generate_handler!` in `lib.rs` doesn't have to know about
//! the file split.

pub(crate) mod asset;
pub(crate) mod block;
pub(crate) mod exec;
pub(crate) mod page;
pub(crate) mod peers;
pub(crate) mod plugin;
pub(crate) mod property;
pub(crate) mod reminders;
pub(crate) mod template;
pub(crate) mod workspace;

pub(crate) use asset::*;
pub(crate) use block::*;
pub(crate) use page::*;
pub(crate) use peers::*;
pub(crate) use plugin::*;
pub(crate) use property::*;
pub(crate) use reminders::*;
pub(crate) use template::*;
pub(crate) use workspace::*;
