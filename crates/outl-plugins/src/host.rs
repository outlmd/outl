//! `PluginHost` — loads plugins and drives them on behalf of a client.
//!
//! The host is the only thing that holds both a [`PluginEngine`] and (briefly,
//! per call) `&mut Workspace`. It:
//!
//! - loads a plugin (manifest + bundle), intersecting capabilities with the
//!   client and freezing the approved [`PermissionSet`];
//! - lists the commands a plugin contributes (for a palette / slash menu);
//! - runs a command, applying the emitted intents through `outl-actions`;
//! - dispatches applied ops to `onOp` hooks via [`PluginHost::sync_hooks`].
//!
//! Anti-loop: the host tracks how far into the op log it has dispatched
//! (`last_seen`). Ops a plugin itself produces advance `last_seen` too, so they
//! never re-trigger hooks — no plugin → op → plugin cycle.
//!
//! # Where the rest of it lives
//!
//! This file keeps the plugin's *lifetime* — load it, hand it what it needs for
//! a turn, run the turn, persist what it changed. Four jobs with their own
//! rules moved out to siblings:
//!
//! - `project` — `&Workspace` → the read-only shapes the JS side sees.
//! - `intents` — the only place the host mutates a workspace, permission-gated.
//! - `contributions` — `manifest.contributes` → client chrome, capability-gated.
//! - `sync` — the `sync-transport` trust boundary (non-negotiable 7).

mod contributions;
mod intents;
mod project;
mod sync;

use std::rc::Rc;

use serde_json::Value;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;

use crate::capability::{self, Capability, CapabilityMatch, ClientCapabilities};
use crate::error::{PluginError, Result};
use crate::manifest::PluginManifest;
use crate::model::{LogOpView, TransformResult};
use crate::permission::{Permission, PermissionSet};
use crate::runtime::PluginEngine;
use crate::secrets::{plugin_service, KeyringStore, SecretStore};

use self::intents::apply_intents;
use self::project::{build_read_model, project_op};

pub use self::contributions::{CommandEntry, PluginBinding, ToolbarButtonEntry, TransformerEntry};

/// One loaded, activated plugin.
struct LoadedPlugin {
    manifest: PluginManifest,
    caps: CapabilityMatch,
    perms: PermissionSet,
    config: Value,
    engine: Box<dyn PluginEngine>,
}

impl LoadedPlugin {
    fn has(&self, cap: Capability) -> bool {
        self.caps.granted.contains(&cap)
    }
}

/// The result of running a command or a hook sweep.
#[derive(Debug, Clone, Default)]
pub struct PluginRun {
    /// Number of intents successfully applied.
    pub applied: usize,
    /// `console.log` / `ctx.log` lines.
    pub logs: Vec<String>,
    /// `ctx.ui.notify` messages.
    pub notifications: Vec<String>,
    /// `ctx.ui.render` payloads (author-written HTML/JS) — only populated for
    /// plugins granted the `ui-render` capability on this client.
    pub views: Vec<String>,
    /// Non-fatal errors (denied permission, bad node id, action failure).
    pub errors: Vec<String>,
}

/// Loads plugins and runs them against a client's workspace.
pub struct PluginHost {
    client_caps: ClientCapabilities,
    plugins: Vec<LoadedPlugin>,
    last_seen: usize,
    last_pushed: usize,
    dispatching: bool,
    /// `<root>/.outl/plugins` — where each plugin's `storage.json` lives. When
    /// unset (tests), `ctx.storage` stays in-memory and is not persisted.
    storage_dir: Option<std::path::PathBuf>,
    /// Backing keychain for `ctx.secrets`. Defaults to the OS keychain
    /// ([`KeyringStore`]); tests swap in an in-memory store via
    /// [`PluginHost::set_secret_store`].
    secret_store: Rc<dyn SecretStore>,
}

impl PluginHost {
    /// Create a host for a client that implements `client_caps`.
    pub fn new(client_caps: ClientCapabilities) -> Self {
        Self {
            client_caps,
            plugins: Vec::new(),
            last_seen: 0,
            last_pushed: 0,
            dispatching: false,
            storage_dir: None,
            secret_store: Rc::new(KeyringStore::new()),
        }
    }

    /// Swap the backing secret store (tests use an in-memory store so
    /// `ctx.secrets` never touches the OS keychain or prompts in CI).
    pub fn set_secret_store(&mut self, store: Rc<dyn SecretStore>) {
        self.secret_store = store;
    }

    /// Configure `ctx.secrets` for the plugin at `idx` this turn: grant the
    /// keychain store only when the plugin holds the `secrets` permission, and
    /// namespace it to this plugin's service so it can never read another's.
    fn prepare_secrets(&mut self, idx: usize) {
        let p = &self.plugins[idx];
        let enabled = p.perms.check(&Permission::Secrets);
        let service = plugin_service(&p.manifest.id);
        let store = enabled.then(|| Rc::clone(&self.secret_store));
        self.plugins[idx]
            .engine
            .set_secrets(enabled, service, store);
    }

    /// Tell the host where to persist per-plugin `ctx.storage` KVs (the
    /// `.outl/plugins` directory). The loader sets this so `storage:local`
    /// survives restarts.
    pub fn set_storage_dir(&mut self, dir: std::path::PathBuf) {
        self.storage_dir = Some(dir);
    }

    /// Load a plugin's local KV from disk and hand it to its engine for this
    /// turn (no-op / disabled when `storage:local` isn't granted).
    fn prepare_storage(&mut self, idx: usize) {
        let p = &self.plugins[idx];
        let enabled = p.perms.check(&crate::permission::Permission::StorageLocal);
        let kv = if enabled {
            self.storage_path(&p.manifest.id)
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        self.plugins[idx].engine.set_storage(enabled, kv);
    }

    /// Persist a plugin's KV if it changed this turn.
    fn flush_storage(&mut self, idx: usize) {
        if let Some(kv) = self.plugins[idx].engine.take_dirty_storage() {
            let id = self.plugins[idx].manifest.id.clone();
            if let Some(path) = self.storage_path(&id) {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(json) = serde_json::to_vec_pretty(&kv) {
                    let _ = std::fs::write(path, json);
                }
            }
        }
    }

    fn storage_path(&self, id: &str) -> Option<std::path::PathBuf> {
        self.storage_dir
            .as_ref()
            .map(|d| d.join(id).join("storage.json"))
    }

    /// Load and activate a plugin from an already-read manifest + bundle.
    ///
    /// `approved` is the permission set the user approved (from the lockfile);
    /// `config` is the plugin's stored config. The bundle is evaluated and
    /// `activate(ctx)` runs, registering the plugin's commands and hooks.
    pub fn load_plugin(
        &mut self,
        manifest: PluginManifest,
        bundle: &str,
        approved: PermissionSet,
        config: Value,
    ) -> Result<()> {
        let caps = capability::intersect(&manifest.capabilities, &self.client_caps);
        let mut engine = new_engine()?;
        // Grant the engine the network domains the user approved, so a
        // `ctx.net.fetch` to anything else is refused inside the engine.
        let net_domains: Vec<crate::permission::NetworkDomain> = approved
            .as_slice()
            .iter()
            .filter_map(|p| match p {
                crate::permission::Permission::Network(d) => Some(d.clone()),
                _ => None,
            })
            .collect();
        engine.set_network(crate::permission::NetGrant {
            plugin_id: manifest.id.clone(),
            domains: net_domains,
        });
        engine
            .load(bundle)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        self.plugins.push(LoadedPlugin {
            manifest,
            caps,
            perms: approved,
            config,
            engine,
        });
        Ok(())
    }

    /// Capabilities a plugin declared but this client can't honor — surface
    /// these to the user as warnings.
    pub fn missing_capabilities(&self, plugin_id: &str) -> Vec<Capability> {
        self.plugins
            .iter()
            .find(|p| p.manifest.id == plugin_id)
            .map(|p| p.caps.missing.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Run a plugin's content transformer for `lang` against `input`, returning
    /// the descriptor (`{kind, content}`) it produced, or `None` when it
    /// declined / has no transformer for that language.
    pub fn transform_block(
        &mut self,
        plugin_id: &str,
        lang: &str,
        input: &str,
    ) -> Result<Option<TransformResult>> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| PluginError::Manifest(format!("no such plugin `{plugin_id}`")))?;
        self.prepare_secrets(idx);
        let config = self.plugins[idx].config.clone();
        let json = self.plugins[idx]
            .engine
            .transform(lang, input, &config)
            .map_err(|e| PluginError::Engine(e.to_string()))?;
        match json {
            None => Ok(None),
            Some(s) => Ok(Some(serde_json::from_str::<TransformResult>(&s)?)),
        }
    }

    /// Mark the host as caught up with the current log — call after loading so
    /// pre-existing ops don't fire hooks on startup.
    pub fn mark_synced(&mut self, workspace: &Workspace) {
        self.last_seen = workspace.log().len();
    }

    /// Run a plugin command, applying whatever intents it emits.
    pub fn run_command(
        &mut self,
        workspace: &mut Workspace,
        hlc: &HlcGenerator,
        plugin_id: &str,
        command_id: &str,
    ) -> Result<PluginRun> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| PluginError::Manifest(format!("no such plugin `{plugin_id}`")))?;

        let read_model = build_read_model(workspace);
        self.prepare_storage(idx);
        self.prepare_secrets(idx);
        let (config, turn) = {
            let p = &mut self.plugins[idx];
            let config = p.config.clone();
            let turn = p
                .engine
                .run_command(command_id, &read_model, &config)
                .map_err(|e| PluginError::Engine(e.to_string()))?;
            (config, turn)
        };
        let _ = config;
        self.flush_storage(idx);

        let perms = self.plugins[idx].perms.clone();
        let has_ui = self.plugins[idx].has(Capability::UiRender);
        let mut run = PluginRun {
            logs: turn.logs,
            notifications: turn.notifications,
            views: if has_ui { turn.views } else { Vec::new() },
            ..Default::default()
        };
        apply_intents(workspace, hlc, &perms, plugin_id, &turn.intents, &mut run);
        // Command-applied ops should not re-fire op hooks on the next sweep.
        self.last_seen = workspace.log().len();
        Ok(run)
    }

    /// Dispatch every op applied since the last sweep to plugins' `onOp` hooks,
    /// applying any intents they emit. Idempotent and loop-safe.
    pub fn sync_hooks(
        &mut self,
        workspace: &mut Workspace,
        hlc: &HlcGenerator,
    ) -> Result<PluginRun> {
        let mut run = PluginRun::default();
        if self.dispatching {
            return Ok(run);
        }
        let total = workspace.log().len();
        if total <= self.last_seen {
            self.last_seen = total;
            return Ok(run);
        }

        // Project the new ops before touching the workspace mutably.
        let views: Vec<LogOpView> = workspace
            .log()
            .iter()
            .skip(self.last_seen)
            .filter_map(|lo| project_op(workspace, lo))
            .collect();
        // Mark seen now so plugin-applied ops below don't get re-dispatched.
        self.last_seen = total;

        self.dispatching = true;
        let read_model = build_read_model(workspace);
        let plugin_ids: Vec<usize> = (0..self.plugins.len())
            .filter(|&i| self.plugins[i].has(Capability::OpHook))
            .collect();

        for view in &views {
            for &i in &plugin_ids {
                self.prepare_storage(i);
                self.prepare_secrets(i);
                let (perms, plugin_id, has_ui, turn) = {
                    let p = &mut self.plugins[i];
                    let config = p.config.clone();
                    match p.engine.dispatch_op(view, &read_model, &config) {
                        Ok(turn) => (
                            p.perms.clone(),
                            p.manifest.id.clone(),
                            p.has(Capability::UiRender),
                            turn,
                        ),
                        Err(e) => {
                            run.errors.push(format!("{}: {e}", p.manifest.id));
                            continue;
                        }
                    }
                };
                self.flush_storage(i);
                run.logs.extend(turn.logs);
                run.notifications.extend(turn.notifications);
                if has_ui {
                    run.views.extend(turn.views);
                }
                apply_intents(workspace, hlc, &perms, &plugin_id, &turn.intents, &mut run);
            }
        }

        // Plugin-applied ops advanced the log; swallow them so they don't loop.
        self.last_seen = workspace.log().len();
        self.dispatching = false;
        Ok(run)
    }
}

#[cfg(feature = "js")]
fn new_engine() -> Result<Box<dyn PluginEngine>> {
    crate::engine::BoaEngine::new()
        .map(|e| Box::new(e) as Box<dyn PluginEngine>)
        .map_err(|e| PluginError::Engine(e.to_string()))
}

#[cfg(not(feature = "js"))]
fn new_engine() -> Result<Box<dyn PluginEngine>> {
    Err(PluginError::NoEngine)
}

#[cfg(all(test, feature = "js"))]
#[path = "host_tests.rs"]
mod tests;
