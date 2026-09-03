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
//!   move, paste, collapsed, clipboard). `cut_block` (RFC 0254 phase 4)
//!   renders the block to markdown then moves it to the trash root;
//!   `copy_block_ref` (issue #18) produces its `((blk-XXXXXX))` handle.
//! - [`page`] — open / navigate pages and journals, search, refs,
//!   `toggle_pin` (RFC 0254 phase 4: flips the `pinned::` page
//!   property, already op-log-backed).
//! - [`peers`] — peer list / status / removal + force-sync (pairing
//!   stays client-side: the two clients return different wire shapes).
//! - [`plugin`] — the run / sync-hooks replies that combine the
//!   [`crate::PluginService`] with a refreshed page view.
//! - [`exec`] — `run_code_block` over `outl_actions::exec`.
//! - [`history`] — undo / redo of committed block mutations, for any
//!   client whose [`crate::AppHost::history`] returns `Some` (RFC 0254
//!   phase 1: desktop and mobile both do).
//! - [`reminders`] — list / snooze / author `remind::` rules. The
//!   *delivery* of the notification stays per-client (each OS has its
//!   own scheduler); only the "what and when" is shared.
//! - [`theme`] — resolve a palette preset. Pure; no workspace access.
//! - [`timeline`] — a page's history, read out of the op log.
//!   Distinct from [`history`]: that is this session's undo stack.
//!   Read-only; there is no restore command.

pub mod asset;
pub mod block;
pub mod exec;
pub mod history;
pub mod page;
pub mod peers;
pub mod plugin;
pub mod property;
pub mod reminders;
pub mod template;
pub mod theme;
pub mod timeline;
