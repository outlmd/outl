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
/// What an engine needs wired before it runs a turn of plugin JS.
///
/// Carried as one value so the two halves cannot be wired separately — four
/// of six entry points once wired only the keychain, and `ctx.storage` throws
/// when it was never set up. See [`PluginHost::prepare_turn`].
struct TurnInputs {
    storage_enabled: bool,
    kv: serde_json::Map<String, Value>,
    secrets_enabled: bool,
    service: String,
    store: Option<Rc<dyn SecretStore>>,
}

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

    /// Everything an engine needs wired before it runs plugin JS.
    ///
    /// A struct rather than two calls because the two halves must travel
    /// together: see [`PluginHost::prepare_turn`].
    fn turn_inputs(&self, id: &str, perms: &PermissionSet) -> TurnInputs {
        let storage_enabled = perms.check(&Permission::StorageLocal);
        let kv = if storage_enabled {
            self.storage_path(id)
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };

        let secrets_enabled = perms.check(&Permission::Secrets);
        TurnInputs {
            storage_enabled,
            kv,
            secrets_enabled,
            service: plugin_service(id),
            store: secrets_enabled.then(|| Rc::clone(&self.secret_store)),
        }
    }

    /// Apply them. An associated function, so it can run against an engine
    /// that is not in `self.plugins` yet — which is the case during load.
    fn apply_inputs(engine: &mut Box<dyn PluginEngine>, inputs: TurnInputs) {
        engine.set_storage(inputs.storage_enabled, inputs.kv);
        engine.set_secrets(inputs.secrets_enabled, inputs.service, inputs.store);
    }

    /// Hand the engine everything a turn needs before it runs plugin JS: the
    /// plugin's local KV and its keychain access, each gated by its own
    /// permission.
    ///
    /// **Every entry point that runs JS goes through this or `apply_inputs`**,
    /// and that is the point. The two halves used to be wired at each call
    /// site, so each new entry point had to remember both — and four of the
    /// six remembered only secrets. `ctx.storage.get` *throws* when it was
    /// never set up, with the same message it uses for a missing permission,
    /// so a plugin touching its own cache failed with "needs the
    /// `storage:local` permission" while holding that permission. One door,
    /// so the next entry point cannot half-arrive.
    ///
    /// Pair it with [`PluginHost::finish_turn`] on the way out.
    fn prepare_turn(&mut self, idx: usize) {
        let id = self.plugins[idx].manifest.id.clone();
        let perms = self.plugins[idx].perms.clone();
        let inputs = self.turn_inputs(&id, &perms);
        Self::apply_inputs(&mut self.plugins[idx].engine, inputs);
    }

    /// Persist whatever the turn changed. The counterpart to
    /// [`PluginHost::prepare_turn`]; a no-op when the plugin wrote nothing.
    fn finish_turn(&mut self, idx: usize) {
        self.flush_storage(idx);
    }

    /// Tell the host where to persist per-plugin `ctx.storage` KVs (the
    /// `.outl/plugins` directory). The loader sets this so `storage:local`
    /// survives restarts.
    pub fn set_storage_dir(&mut self, dir: std::path::PathBuf) {
        self.storage_dir = Some(dir);
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
        // `engine.load` evaluates the bundle *and* calls `activate(ctx)`, so
        // this is the sixth entry point that runs the author's JS — and the
        // harshest one to get wrong: a throw here fails the load outright, so
        // a plugin that reads its cache in `activate` would never register at
        // all, blaming a permission it holds.
        let inputs = self.turn_inputs(&manifest.id, &approved);
        Self::apply_inputs(&mut engine, inputs);

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
        // `activate` may have written to storage; it is the last plugin now.
        self.finish_turn(self.plugins.len() - 1);
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
        self.prepare_turn(idx);
        let config = self.plugins[idx].config.clone();
        let produced = self.plugins[idx].engine.transform(lang, input, &config);
        // Before the `?`: the next `prepare_turn` overwrites the engine's KV
        // from disk and clears the dirty flag, so a write the plugin made
        // before it threw would not merely go unsaved — it would be discarded.
        self.finish_turn(idx);
        let json = produced.map_err(|e| PluginError::Engine(e.to_string()))?;
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
        self.prepare_turn(idx);
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
        self.finish_turn(idx);

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
                self.prepare_turn(i);
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
                self.finish_turn(i);
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
