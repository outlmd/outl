//! Command **bodies** shared by every GUI client.
//!
//! Each function here is the full implementation of one Tauri command,
//! generic over [`crate::AppHost`]. The client crates register thin
//! `#[tauri::command]` wrappers (Tauri's `generate_handler!` needs
//! concrete fns in the app crate) that parse nothing and just delegate —
//! the body lives exactly once.
//!
//! Split by responsibility, mirroring the historical per-client layout:
//!
//! - [`block`] — every block mutation (create, edit, todo, indent,
//!   move, paste, collapsed, clipboard).
//! - [`page`] — open / navigate pages and journals, search, refs.
//! - [`peers`] — peer list / status / removal + force-sync (pairing
//!   stays client-side: the two clients return different wire shapes).
//! - [`plugin`] — the run / sync-hooks replies that combine the
//!   [`crate::PluginService`] with a refreshed page view.
//! - [`exec`] — `run_code_block` over `outl_actions::exec`.
//! - [`reminders`] — list / snooze / author `remind::` rules. The
//!   *delivery* of the notification stays per-client (each OS has its
//!   own scheduler); only the "what and when" is shared.

pub mod asset;
pub mod block;
pub mod exec;
pub mod page;
pub mod peers;
pub mod plugin;
pub mod property;
pub mod reminders;
pub mod template;
