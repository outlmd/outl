//! `impl App` blocks, organised by responsibility.
//!
//! Rust accepts as many `impl App { ... }` blocks as you write — they
//! all merge into the same type at link time. We use that to split the
//! ~1.4k LOC of behaviour into single-purpose files:
//!
//! - `lifecycle` — load / save / external-edit polling / `App::new`
//! - `nav` — page/journal jumps, cursor inside a block, selection
//!   between blocks, opening `[[refs]]`
//! - `block` — Insert mode, create / indent / outdent / delete /
//!   move block, TODO cycle
//! - `history` — undo / redo snapshots
//! - `visual` — Visual mode + its delete / indent / outdent
//! - `yank` — yank register, paste after / before
//! - `exec` — run code block under cursor via `outl_exec`
//! - `overlay` — quick switcher, workspace search, command palette
//! - `autocomplete` — Insert-mode inline `[[`/`#`/`((`/`/`/`@` popup
//! - `properties` — the `g p` property editor overlay
//!
//! Anything cross-cutting (constructors, `pub(crate)` free helpers
//! consumed by `input` / `view`) is re-exported from this file.

pub(crate) mod autocomplete;
pub(crate) mod block;
pub(crate) mod collapsed;
pub(crate) mod exec;
pub(crate) mod history;
pub(crate) mod lifecycle;
pub(crate) mod mouse;
pub(crate) mod nav;
pub(crate) mod overlay;
pub(crate) mod paste;
pub(crate) mod plugins;
pub(crate) mod properties;
pub(crate) mod reminders;
pub(crate) mod sidebar;
pub(crate) mod text_ops;
pub(crate) mod toast;
pub(crate) mod visual;
pub(crate) mod yank;
pub(crate) mod zoom;

// Re-exports for crate-internal consumers in *non-test* code paths.
// `app.rs` tests reach for `cycle_todo_state` / `detect_trigger` /
// `read_page_title` directly via the submodule path — listing them
// here too would warn-as-unused under `--release` / non-test builds.
pub(crate) use block::cycle_todo_inline;
