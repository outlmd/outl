//! Terminal UI for the outl outliner.
//!
//! Journal-first: opening the TUI lands on today's journal. Navigation
//! between dates and between pages works, and blocks are editable in place
//! (text editing, create, indent / outdent, move, delete).
//!
//! Reused by the `outl` binary so that `outl` with no subcommand opens
//! the TUI in the current directory. See `crates/outl-tui/CLAUDE.md`.
//!
//! ## Crate layout
//!
//! - [`state`]   — plain data: `App`, modes, overlays, snapshots.
//! - [`actions`] — methods on `App` that mutate state (the bulk).
//! - [`input`]   — key handlers; route a `KeyEvent` to an action.
//! - [`view`]    — ratatui rendering.
//! - [`runtime`] — `pub fn run`, terminal lifecycle, event loop.
//! - [`keymap`]  — keymap documentation (no code).
//! - [`app`]     — thin re-export shim + TUI-side tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod actions;
pub mod app;
pub mod commands;
pub mod edit_buffer;
pub mod editor;
pub mod fuzzy;
pub mod input;
pub mod keymap;
pub mod outline_ops;
pub mod runtime;
pub mod state;
pub mod theme;
pub mod ui;
pub mod view;

pub use app::{run, run_with_theme_override};
pub use edit_buffer::EditBuffer;
// `PRESETS` is deliberately NOT re-exported here. It belongs to
// `outl-theme`; callers that need the list (the CLI's `outl theme
// list`) take it from there, so there is no second name for it.
pub use theme::{by_name as theme_by_name, default_theme, Theme};
