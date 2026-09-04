//! Shared Tauri backend for the outl GUI clients (desktop + mobile).
//!
//! Both `outl-desktop/src-tauri` and `outl-mobile/src-tauri` used to keep
//! near-identical copies of the same nine files (state DTOs, helpers,
//! command bodies, workspace-open primitives, iroh wiring, plugin
//! service). This crate is the single owner of that surface; each client
//! keeps only thin `#[tauri::command]` wrappers plus what is genuinely
//! client-specific (desktop: settings / fs watcher / undo history; mobile:
//! iOS background sync / workspace picker).
//!
//! ## The two abstractions that absorb the real divergence
//!
//! - [`AppHost`] — implemented by each client's `AppState`. It hides the
//!   one structural difference between the clients: the desktop's storage
//!   root is `Arc<Mutex<Option<PathBuf>>>` (the workspace can be swapped at
//!   runtime), the mobile's is a plain `PathBuf` (a folder swap is a
//!   relaunch). Command bodies are generic over `S: AppHost`.
//! - [`StorageRootProvider`] — the same difference, but as an owned value
//!   the plugin thread can move into itself (see [`plugin_service`]).
//!
//! ## What this crate does NOT own
//!
//! - The `AppState` structs themselves (fields differ per client).
//! - `#[tauri::command]` functions — Tauri's `generate_handler!` needs
//!   concrete fns in the app crate, so each client registers 1–3 line
//!   wrappers that delegate here.
//! - The boot openers (`spawn_workspace_opener`) and the iroh *wiring*
//!   (which slot / return value the transport lands in) — those
//!   orchestrations are thin and genuinely divergent; only their
//!   primitives live here.
//! - Business logic. Everything that mutates the workspace shape still
//!   delegates to `outl-actions` — this crate is glue, same hard rule as
//!   the client crates.

pub mod commands;
pub mod helpers;
pub mod host;
pub mod iroh_sync;
pub mod plugin_dto;
pub mod plugin_service;
mod plugin_thread;
pub mod projection;
pub mod reminder_runtime;
pub mod state;
pub mod workspace_open;

pub use host::{AppHost, StorageRootProvider};
pub use plugin_service::PluginService;
pub use projection::ProjectionWriter;
pub use state::{
    CreateBlockReply, CutBlockReply, PageView, TemplateDto, WorkspaceSummary, ERR_LOADING,
};
