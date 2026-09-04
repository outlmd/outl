//! Plugin integration shared by the GUI clients.
//!
//! [`outl_plugins::PluginHost`] embeds a Boa `Context`, which is **not
//! `Send`**. Tauri's `AppState` must be `Send + Sync`, so the host can
//! never live in `AppState` directly. Instead it lives on a **dedicated
//! plugin thread** (`plugin_thread.rs`) that owns it for the
//! process lifetime; [`PluginService`] (which *is* `Send + Sync`) holds
//! only the [`std::sync::mpsc::Sender`] end of a request channel and goes
//! into `AppState`.
//!
//! The `Workspace` itself **is** `Send` and is shared through the same
//! `Arc<Mutex<Option<Workspace>>>` every Tauri command already locks. The
//! plugin thread locks it per request, runs the host, re-projects the
//! `.md` of every page when a plugin mutated anything, then replies over
//! the request's reply channel. The Boa `Context` therefore never crosses
//! a thread boundary.
//!
//! ## Per-client parameters
//!
//! [`PluginService::spawn`] takes the client id (`"desktop"` /
//! `"mobile"` — the host filters keybinding / toolbar contributions by
//! it), a capability-set factory (the desktop honors `keybinding`, the
//! mobile doesn't), and a [`StorageRootProvider`]: the desktop's
//! swap-capable `Arc<Mutex<Option<PathBuf>>>` makes the host reload
//! against a new root after a workspace swap; the mobile's fixed
//! `PathBuf` loads exactly once.
//!
//! Best-effort end to end: a host that can't be built, a plugin that
//! fails to load, or a re-projection error never blocks the editor.

use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_plugins::{ClientCapabilities, MarketplaceItem};
use parking_lot::Mutex;

use crate::host::StorageRootProvider;
use crate::plugin_dto::{
    PluginCommandDto, PluginKeybindingDto, PluginRunDto, ToolbarButtonDto, TransformResultDto,
    TransformerDto,
};
use crate::plugin_thread::run_plugin_thread;

/// A request handed to the plugin thread. Each carries a one-shot reply
/// channel the thread sends the result back on; the caller blocks on the
/// matching `Receiver` (`recv()`), so the Tauri command stays synchronous
/// and never holds the workspace `Mutex` across the wait.
pub(crate) enum PluginRequest {
    /// List every command contributed by a loaded plugin.
    ListCommands {
        reply: Sender<Vec<PluginCommandDto>>,
    },
    /// List every `keybinding` a plugin contributed for this client.
    /// Only the desktop registers a command for it; on a client without
    /// the `keybinding` capability the host returns an empty list.
    ListKeybindings {
        reply: Sender<Vec<PluginKeybindingDto>>,
    },
    /// List every `toolbar-button` a plugin contributed for this client.
    ListToolbar {
        reply: Sender<Vec<ToolbarButtonDto>>,
    },
    /// Run a plugin command. Reply is `Err(String)` on a host-level
    /// failure (no such plugin, engine error); `Ok` carries the per-run
    /// notifications / errors even when individual intents were denied.
    RunCommand {
        plugin_id: String,
        command_id: String,
        reply: Sender<Result<PluginRunDto, String>>,
    },
    /// Run every plugin's `onOp` hook over ops applied since the last
    /// sweep. Reply carries the number of intents the hooks applied (so
    /// the caller knows whether to re-render) plus any `ui-render` views
    /// the hooks emitted (the confetti path: a DONE toggle fires `onOp`,
    /// the plugin emits HTML, the client plays it as an overlay).
    /// Best-effort.
    SyncHooks { reply: Sender<SyncHooksOutcome> },
    /// List every content transformer a plugin declared for a code-fence
    /// language (gated by `content-transformer:text` / `:rich` upstream).
    ListTransformers { reply: Sender<Vec<TransformerDto>> },
    /// Run a plugin's content transformer for `lang` against `input` (a
    /// fence body). Reply is `Err(String)` on a host-level failure;
    /// `Ok(None)` when the transformer declined or no plugin owns `lang`.
    /// Read-only — never mutates the workspace, so no re-projection.
    TransformBlock {
        plugin_id: String,
        lang: String,
        input: String,
        reply: Sender<Result<Option<TransformResultDto>, String>>,
    },
    /// Fetch the official registry and cross-reference it with this
    /// workspace's lockfile → the marketplace rows. Network + lockfile
    /// read; runs on the plugin (non-tokio) thread so blocking HTTP is
    /// fine.
    RegistryList {
        reply: Sender<Result<Vec<MarketplaceItem>, String>>,
    },
    /// Download + install an official plugin by id (tap-to-install), then
    /// reload the host so it's live. Reply is the installed name.
    InstallOfficial {
        id: String,
        reply: Sender<Result<String, String>>,
    },
    /// Flip a plugin's `enabled` flag in the lockfile, then reload.
    SetEnabled {
        id: String,
        enabled: bool,
        reply: Sender<Result<(), String>>,
    },
    /// Uninstall a plugin (delete its dir + lockfile entry), then reload.
    Uninstall {
        id: String,
        reply: Sender<Result<bool, String>>,
    },
    /// Force the host to reload from the lockfile before the next request —
    /// used after a config edit so the running plugin picks up the new config.
    Reload { reply: Sender<Result<(), String>> },
}

/// What a `SyncHooks` sweep produced: how many intents the op-hooks
/// applied (drives the re-render decision) and the HTML views they
/// emitted via `ctx.ui.render` (played as sandboxed iframe overlays).
#[derive(Debug, Clone, Default)]
pub struct SyncHooksOutcome {
    pub applied: usize,
    pub views: Vec<String>,
    /// Projection failures after hook intents were durably applied.
    pub(crate) projection_errors: Vec<crate::state::ProjectionFailure>,
}

/// `Send + Sync` handle to the plugin thread. Stored in `AppState`.
///
/// Cloneable: every clone shares the same `mpsc::Sender`, so any Tauri
/// command can reach the single plugin thread.
#[derive(Clone)]
pub struct PluginService {
    tx: Sender<PluginRequest>,
}

impl PluginService {
    /// Spawn the plugin thread and return a handle to it.
    ///
    /// The thread loads every installed plugin from
    /// `<root>/.outl/plugins/` on the first request after the workspace
    /// opens, marks the host synced (so pre-existing ops don't fire
    /// `onOp` at boot), then serves requests until the `Sender` is
    /// dropped. When `storage_root` reports a *different* root later (a
    /// desktop workspace swap), the host is rebuilt against the new root.
    ///
    /// `capabilities` is a factory (not a value) so the thread can build
    /// a fresh host on every (re)load without requiring
    /// `ClientCapabilities: Clone`.
    pub fn spawn<R: StorageRootProvider>(
        client: &'static str,
        capabilities: fn() -> ClientCapabilities,
        workspace: Arc<Mutex<Option<Workspace>>>,
        storage_root: R,
        hlc: HlcGenerator,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<PluginRequest>();
        thread::Builder::new()
            .name("outl-plugin-host".into())
            .spawn(move || {
                run_plugin_thread(rx, client, capabilities, workspace, storage_root, hlc)
            })
            .expect("spawn plugin host thread");
        Self { tx }
    }

    /// List plugin-contributed commands. Returns an empty vec if the
    /// plugin thread is gone or nothing is loaded yet.
    pub fn list_commands(&self) -> Vec<PluginCommandDto> {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::ListCommands { reply }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// List plugin-contributed keybindings for this client. Empty if the
    /// plugin thread is gone, nothing is loaded yet, or the client
    /// doesn't declare the `keybinding` capability (mobile).
    pub fn list_keybindings(&self) -> Vec<PluginKeybindingDto> {
        let (reply, rx) = mpsc::channel();
        if self
            .tx
            .send(PluginRequest::ListKeybindings { reply })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// List plugin-contributed toolbar buttons for this client. Empty if
    /// the plugin thread is gone or nothing is loaded yet.
    pub fn list_toolbar(&self) -> Vec<ToolbarButtonDto> {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::ListToolbar { reply }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// Run a plugin command and wait for its result.
    pub fn run_command(
        &self,
        plugin_id: String,
        command_id: String,
    ) -> Result<PluginRunDto, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::RunCommand {
                plugin_id,
                command_id,
                reply,
            })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// List plugin-contributed content transformers. Empty if the plugin
    /// thread is gone or nothing is loaded yet.
    pub fn list_transformers(&self) -> Vec<TransformerDto> {
        let (reply, rx) = mpsc::channel();
        if self
            .tx
            .send(PluginRequest::ListTransformers { reply })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// Run a content transformer for `lang` against a fence `input`.
    ///
    /// `Ok(None)` means the transformer declined or no plugin owns
    /// `lang`; `Ok(Some(_))` carries the `{kind, content}` descriptor.
    /// Read-only: the plugin thread never locks the workspace for
    /// mutation here, so a transform can't race a concurrent edit.
    pub fn transform_block(
        &self,
        plugin_id: String,
        lang: String,
        input: String,
    ) -> Result<Option<TransformResultDto>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::TransformBlock {
                plugin_id,
                lang,
                input,
                reply,
            })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Fire the `onOp` hook sweep and return the outcome: how many
    /// intents the hooks applied (so the caller re-renders only when
    /// something changed) plus any `ui-render` views they emitted.
    ///
    /// Blocks until the sweep is done so a follow-up page render sees any
    /// hook mutation, but a dead thread is a silent empty outcome
    /// (plugins must never block editing). The caller must NOT hold the
    /// workspace `Mutex` — the plugin thread locks it to run the hooks.
    pub fn sync_hooks(&self) -> SyncHooksOutcome {
        let (reply, rx) = mpsc::channel();
        if self.tx.send(PluginRequest::SyncHooks { reply }).is_err() {
            return SyncHooksOutcome::default();
        }
        rx.recv().unwrap_or_default()
    }

    /// Marketplace rows: the official registry crossed with the lockfile.
    pub fn registry_list(&self) -> Result<Vec<MarketplaceItem>, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::RegistryList { reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Tap-to-install an official plugin by id; returns its display name.
    pub fn install_official(&self, id: String) -> Result<String, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::InstallOfficial { id, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Enable / disable an installed plugin.
    pub fn set_enabled(&self, id: String, enabled: bool) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::SetEnabled { id, enabled, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Uninstall a plugin; `true` if anything was removed.
    pub fn uninstall(&self, id: String) -> Result<bool, String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::Uninstall { id, reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }

    /// Force a host reload before the next request. Call after editing a
    /// plugin's config so the running host picks up the new values.
    pub fn reload(&self) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(PluginRequest::Reload { reply })
            .map_err(|_| "plugin host unavailable".to_string())?;
        rx.recv()
            .map_err(|_| "plugin host did not reply".to_string())?
    }
}
