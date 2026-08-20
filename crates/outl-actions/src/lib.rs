//! # outl-actions
//!
//! UI-agnostic workspace operations.
//!
//! Every outl client (`outl-tui`, `outl-mobile`, the future Tauri
//! desktop, future plugins) needs the *same* high-level operations on
//! the workspace: edit a block's text, toggle its TODO state, indent
//! or outdent it, append a new block, render today's journal as
//! `.md`. This crate is where those operations live so we never
//! duplicate them per client.
//!
//! ## Layering
//!
//! ```text
//! outl-core (CRDT, op log, storage trait)
//!     ↑
//! outl-md   (.md parse/render, sidecar, matching)
//!     ↑
//! outl-actions  ← you are here
//!     ↑
//! outl-cli / outl-tui / outl-mobile / future clients
//! ```
//!
//! ## Contract
//!
//! - Functions take a `&mut Workspace` and a `&HlcGenerator` so callers
//!   stay in control of mutation timing and HLC ordering.
//! - All ops go through `Workspace::apply`, never around it — invariant
//!   #1 from `crates/outl-core/CLAUDE.md` ("op log is source of truth").
//! - TODO/DONE state is encoded as a prefix in the block's text
//!   (`"TODO foo"` / `"DONE foo"`), matching the wire format the TUI
//!   already uses. See [`mod@todo`].
//! - The `.md` projection (see [`journal::apply_page_md`] /
//!   [`journal::apply_all_pages_md`]) is a *projection* of the
//!   materialised tree — never read it back to reconstruct state.
//!   Always read the op log.
//!
//! ## What this crate does NOT own
//!
//! - Anything UI-state related (selection cursors, modal modes,
//!   keymaps, toasts) — those stay in `outl-tui` / `outl-mobile`.
//! - In-flight outline AST manipulation (the user editing a `.md`
//!   buffer that hasn't been parsed yet) — that's `outl-tui`'s
//!   `outline_ops.rs`, which operates on `Vec<OutlineNode>` not on a
//!   workspace.
//! - Storage backends (sqlite, iCloud, ChronDB) — those implement
//!   `outl_core::Storage` and live in the binary that needs them.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod asset;
pub mod backlinks;
pub mod backlinks_index;
pub mod backlinks_sort;
pub mod backup;
pub mod block;
pub mod clipboard;
pub mod clock;
pub mod collapsed;
pub mod dates;
pub mod deeplink;
pub mod desync;
pub mod error;
pub mod exec;
pub mod history;
pub mod index;
pub mod journal;
pub mod outline;
pub mod page;
pub mod page_merge;
pub mod page_repair_titles;
pub mod paste;
pub mod person;
pub mod property;
pub mod quote;
pub mod recover;
pub mod reminders;
pub mod resolve;
pub mod storage_scope;
pub mod sync;
pub mod template;
mod text;
pub mod todo;
pub mod tree;

pub use asset::{assets_dir, import_asset, import_asset_bytes, resolve_asset_path, ImportedAsset};
pub use backlinks::{
    backlinks_for_page, backlinks_for_target, extract_refs, Backlink, BacklinkCrumb,
};
pub use backlinks_index::{build_backlink_index, build_backlink_index_from_disk, BacklinkIndex};
pub use backlinks_sort::sort_backlinks;
pub use block::{
    append_block, append_forest, append_tree, create_after, create_after_or_append, create_before,
    create_before_or_append, create_under, delete, edit_text, indent, move_after, move_down,
    move_under, move_up, outdent, split_block, toggle_quote, toggle_todo, BlockTreeOutcome,
    BlockTreeSpec,
};
pub use clipboard::{copy_markdown, copy_markdown_nodes};
pub use collapsed::{set_block_collapsed, toggle_block_collapsed};
pub use dates::{
    date_from_slug, days_until_next_weekday, journal_ref, journal_slug, journal_title,
    next_journal_date, parse_date_arg, parse_date_label, parse_flexible_date,
    previous_journal_date, week_tag,
};
pub use deeplink::{parse_deep_link, DeepLinkError, DeepLinkTarget, DEEP_LINK_SCHEME};
pub use desync::{recover_desynced_projection, scan_for_desynced_projections};
pub use error::ActionError;
pub use exec::{run_code_block, ExecOutputDto, RunCodeBlockOutcome};
pub use history::{restore_page_md, HistoryStacks, DEFAULT_HISTORY_CAP};
pub use journal::{
    apply_all_pages_md, apply_page_md, apply_page_md_with_sidecar,
    apply_page_md_with_sidecar_guarded, apply_page_md_with_sidecar_if_absent,
    apply_page_md_with_sidecar_if_stale, apply_page_md_with_sidecar_rendered,
    content_lines_missing_from, journals_dir, mutate_page_md, page_md_path, pages_dir,
    remove_page_projection, render_block_md, render_page_md, sidecar_can_answer, write_md_atomic,
};
pub use outl_md::parse::{ParseWarning, ParseWarningKind};
pub use outline::{
    flat_index_for_block, flatten_subtree_paths, project_outline, project_outline_node,
    project_parsed_subtree, read_page_outline, read_page_outline_with_workspace, read_page_view,
    read_page_view_with_workspace, OutlineNode, PageOutline,
};
pub use page::{
    delete as delete_page, find_by_slug, is_valid_slug, list_all as list_pages,
    merge_duplicate_slug_roots, migrate_legacy_into_today, open_journal,
    open_or_create as open_or_create_page, open_today, page_meta, read_text_prop, set_property,
    today, PageKind, PageMeta,
};
pub use page_repair_titles::repair_doubled_journal_titles;
pub use paste::{
    looks_like_outline, normalize_external_syntax, paste_markdown, paste_plain, PasteAnchor,
    PasteOutcome,
};
pub use person::{search_persons, PERSON_TYPE, TYPE_KEY};
pub use property::known_keys;
pub use recover::{restore_truncated_block, scan_truncated_blocks, TruncatedBlock};
pub use reminders::{
    epoch_ms_to_local_naive, local_naive_to_epoch_ms, next_fire_at, scan_reminders, snooze,
    snooze_until, take_due, FiredLog, FiredRecord, Reminder, ReminderState, SnoozePreset, Urgency,
};
pub use resolve::{open_or_create_by_name, open_or_create_by_ref};
pub use sync::{
    FileSyncTransport, OpsFileSnapshot, PeerHealthSnapshot, SyncEngine, SyncProgress, SyncTransport,
};
pub use template::{
    instantiate_template, list_templates, parse_call_invocation, resolve_call, run_callable_block,
    TemplateEntry, FROM_TEMPLATE_KEY, JOURNAL_TEMPLATE_NAME, PARAMS_KEY, TEMPLATE_KEY,
};
pub use todo::{cycle_todo, split_todo, TodoState, DOING_PREFIX, DONE_PREFIX, TODO_PREFIX};
pub use tree::{
    children_of, enclosing_page_id, position_after, position_for_new_last_child, walk_subtree,
};
