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
//! - [`page`] — navigate pages and journals, search, refs,
//!   `toggle_pin` (RFC 0254 phase 4: flips the `pinned::` page
//!   property, already op-log-backed), `delete_page`.
//! - [`page_open`] — the three *open* commands. Split out because they
//!   are a concurrency problem rather than navigation logic: each takes
//!   the workspace mutex that the background reconcile pass holds for
//!   ~314 ms right after `workspace-ready`, and the one the user is
//!   waiting on is today's journal. That module's doc carries the
//!   measurements.
//! - [`peers`] — peer list / status / removal + force-sync (pairing
//!   stays client-side: the two clients return different wire shapes).
//! - [`plugin`] — the run / sync-hooks replies that combine the
//!   [`crate::PluginService`] with a refreshed page view.
//! - [`exec`] — `run_code_block` over `outl_actions::exec`.
//! - [`open_with`] — the OS "Open With → outl" gesture: import an
//!   external `.md` / `.txt` as a page under `open-in/`. The policy is
//!   `outl_actions::open_with`'s; this is the path + lock adapter.
//! - [`history`] — undo / redo of committed block mutations, for any
//!   client whose [`crate::AppHost::history`] returns `Some` (RFC 0254
//!   phase 1: desktop and mobile both do).
//! - [`reminders`] — list / snooze / author `remind::` rules. The
//!   *delivery* of the notification stays per-client (each OS has its
//!   own scheduler); only the "what and when" is shared.
//! - [`tasks`] and [`journal_days`]: the overview mobile's bottom bar
//!   and week strip read: every open task, and which days hold text.
//! - [`telemetry`] — read / write `[telemetry] enabled`, the
//!   crash-reporting opt-out (RFC 0377). Pure config I/O, no workspace
//!   access, and the **only** writer of that flag: the desktop's flat
//!   `Settings` restores the section from disk on save, the way it does
//!   for `[display] backlinks_order`.
//! - [`theme`] — resolve a palette preset. Pure; no workspace access.
//! - [`timeline`] — a page's history, read out of the op log.
//! - [`trust`] — per-device activity and the plain Markdown export.
//!   Distinct from [`history`]: that is this session's undo stack.
//!   Read-only; there is no restore command.

pub mod asset;
pub mod block;
pub mod exec;
pub mod history;
pub mod journal_days;
pub mod open_with;
pub mod page;
mod page_backlinks;
pub mod page_open;
pub mod peers;
pub mod plugin;
pub mod property;
pub mod reminders;
pub mod shortcuts;
pub mod tasks;
pub mod telemetry;
pub mod template;
pub mod theme;
pub mod timeline;
pub mod trust;
