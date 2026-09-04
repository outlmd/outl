//! Re-exports of the shared command glue.
//!
//! The cross-client helpers (id parsing, workspace-lock acquisition, the
//! `finish_in_page` mutation funnel, the post-commit announce, surgical
//! undo invalidation across a peer reload) live in
//! `outl_tauri_shared::helpers` — generic over the `AppHost` trait
//! `AppState` implements. This module re-exports them so the rest of the
//! crate keeps one import path.
//!
//! `invalidate_changed_history` moved to the shared crate in RFC 0254
//! phase 1, when mobile's `reload_workspace` gained the same undo
//! stacks to protect against a stale-snapshot revert.

pub(crate) use outl_tauri_shared::helpers::{invalidate_changed_history, storage_root_or_err};
