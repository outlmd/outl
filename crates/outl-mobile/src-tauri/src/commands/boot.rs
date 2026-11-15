//! What a launch reports, and what it waits on.
//!
//! `boot_mark` is the frontend half of the `boot phases` line — on a
//! phone the only way to know when the first frame landed, since no
//! Rust-side timer can see the webview — and it is what releases the
//! interactive phase (`workspace_open.rs`). `wait_workspace_ready`
//! replaces `Journal.boot.ts`'s 50 × 200ms retry.
outl_tauri_shared::boot_commands!(crate::state::AppState);
