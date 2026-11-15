//! What a launch reports, and what it waits on.
//!
//! `boot_mark` is the frontend half of the `boot phases` line — the only
//! way the backend learns when the first frame actually landed — and it
//! is also what releases the interactive phase (`workspace_open.rs`).
//! `wait_workspace_ready` replaces `App.tsx`'s 500 ms poll.
outl_tauri_shared::boot_commands!(crate::state::AppState);
