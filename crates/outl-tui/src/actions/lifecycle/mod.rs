//! Lifecycle methods for [`App`]: construction, view loading, disk
//! persistence, peer-ops sync, external-edit polling.
//!
//! Each concern lives in its own file and contributes its own
//! `impl App { … }` block. The split lets the size of any single
//! file stay under the comfort threshold even as we grow more
//! orchestration around the editor's truth (workspace + page +
//! mtime).
//!
//! | Submodule        | What's in it                                                    |
//! |------------------|-----------------------------------------------------------------|
//! | `mod.rs` (here)  | `App::new` and the shared `file_mtime` helper                   |
//! | `index_build`    | Workspace index rebuild and the background worker poller        |
//! | `peer_sync`      | `ops/<actor>.jsonl` watcher and orphan-`.md` scanner            |
//! | `external`       | Detect/handle edits a different process made to the open `.md`  |
//! | `loading`        | Switch view, parse current `.md`, refresh page list, LRU recent |
//! | `persistence`    | Render `ParsedPage` → `.md`, reconcile, refresh index/cache     |

use crate::commands::CommandRegistry;
use crate::state::{App, Focus, Mode, View};
use crate::theme::Theme;
use anyhow::Result;
use outl_actions::clock;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::ParsedPage;
use std::path::{Path, PathBuf};

pub(crate) mod external;
pub(crate) mod index_build;
pub(crate) mod loading;
pub(crate) mod peer_sync;
pub(crate) mod persistence;

impl App {
    /// Build the editor's initial state and arm the background
    /// pollers (index rebuild, peer-ops watcher, orphan-`.md`
    /// scanner). The first paintable frame is one
    /// `load_current()` away.
    pub(crate) fn new(
        workspace_root: PathBuf,
        workspace: Workspace,
        actor: ActorId,
        theme: Theme,
        shared_workspace: bool,
    ) -> Result<Self> {
        let orphans_log = workspace_root.join(".outl").join("orphans.log");
        let mut s = Self {
            hlc: HlcGenerator::new(actor),
            workspace_root,
            workspace,
            orphans_log,
            view: View::Journal(clock::today()),
            page: ParsedPage::default(),
            selected: 0,
            flat_len: 0,
            cursor_col: 0,
            page_list: Vec::new(),
            mode: Mode::Normal,
            show_help: false,
            help_tab: 0,
            help_scroll: 0,
            pending_chord: None,
            pending_input_op: None,
            pending_plugin_chord: None,
            last_visual: None,
            status: String::new(),
            parse_warnings: Vec::new(),
            load_failed: false,
            overlay: None,
            autocomplete: None,
            last_search: None,
            yank_register: Vec::new(),
            last_yanked_ref: None,
            index: WorkspaceIndex::default(),
            index_rx: None,
            backlink_index: std::cell::RefCell::new(None),
            backlink_index_rx: None,
            shared_workspace,
            jsonl_rx: None,
            sync_transport: None,
            pending_reload: false,
            orphan_md_rx: None,
            show_backlinks: true,
            // Product default; the runtime overrides it from
            // `[display] backlinks_order` right after construction.
            backlinks_newest_first: true,
            show_sidebar: false,
            sidebar_focus: None,
            sidebar_cursor: 0,
            pending_sidebar_delete: None,
            recent_paths: Vec::new(),
            toasts: Vec::new(),
            last_reminder_sweep: None,
            focus: Focus::Outline,
            zoom_stack: Vec::new(),
            scroll_y: 0,
            viewport_height: 0,
            outline_area: None,
            block_starts: Vec::new(),
            outline_line_count: 0,
            mouse_anchor: None,
            last_mtime: None,
            last_saved_at: None,
            dirty_since: None,
            undo: Vec::new(),
            redo: Vec::new(),
            theme,
            exec_registry: RuntimeRegistry::with_builtins(),
            command_registry: CommandRegistry::with_builtins(),
            plugin_host: None,
            collapsed: std::collections::HashSet::new(),
            id_by_flat: Vec::new(),
            hidden_by_collapse: Vec::new(),
            transform_cache: std::collections::HashMap::new(),
        };
        // Repair split-brain page/journal roots (two roots sharing one slug,
        // e.g. a sidecar-less `.md` reconciled to a fresh id) before the first
        // render, so the outline never flickers between the duplicates.
        // Idempotent + a no-op when clean; the merge is Ops, so it converges to
        // every device via the op log.
        match outl_actions::merge_duplicate_slug_roots(&mut s.workspace, &s.hlc) {
            Ok(0) => {}
            Ok(n) => s.status = format!("repaired {n} duplicate page root(s)"),
            Err(e) => s.status = format!("duplicate-root repair failed: {e}"),
        }
        s.refresh_page_list();
        s.ensure_view_file_exists()?;
        s.load_current();
        // Load JS plugins from `<root>/.outl/plugins/`. Best-effort: any
        // failure leaves `plugin_host = None` and the TUI runs normally.
        s.load_plugins();
        // The first `load_current` above ran before the plugin host
        // existed, so its transform pass was a no-op. Now that plugins
        // are loaded, populate the content-transformer cache for the
        // page already on screen.
        s.recompute_transforms();
        // Wire the optional iroh transport BEFORE spawning the poller —
        // `spawn_jsonl_poller` reads `sync_transport` to decide between
        // iroh-driven detection and the FileSyncTransport fallback.
        s.wire_sync_transport();
        // Build the workspace index off the critical path so the TUI
        // can paint immediately. Backlinks/icons fill in once the
        // worker thread completes (usually < 100ms for small
        // workspaces, longer for big ones — but the user is already
        // typing).
        s.spawn_index_rebuild();
        // Backlinks are read off a from-disk index that walks every
        // page's `.md` — seconds on a large vault. Build it on a worker
        // thread too, so the journal paints immediately and the
        // "Linked from" panel fills in a beat later instead of freezing
        // the open.
        s.spawn_backlink_index_rebuild();
        s.spawn_jsonl_poller();
        s.spawn_orphan_md_scanner();
        Ok(s)
    }
}

/// Last-modified time of `path`, or `None` if the file isn't there
/// (or we lack permission to stat it). Used by the external-edit
/// polling loop and by `save`/`load_current` to keep `last_mtime`
/// in sync with what we just wrote/read.
///
/// Shared by `loading`, `external`, and `persistence` — kept on the
/// parent module so each submodule can `use super::file_mtime`.
pub(super) fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
