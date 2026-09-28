//! Plugin JS engine — the seam that lets the engine be swapped.
//!
//! `outl` already embeds Boa (pure-Rust, runs on every client including iOS)
//! for code-block execution in `outl-exec`. The plugin runtime reuses Boa, but
//! behind this [`PluginEngine`] trait so the engine can move to QuickJS later
//! **if** gas/perf/async ever becomes a measured blocker — not on a guess.
//!
//! The engine never touches the workspace. It runs the plugin's JS in turns:
//! `load` evaluates the bundle and activates it; `run_command` / `dispatch_op`
//! each take a fresh read-only [`ReadModel`] plus the plugin config and return
//! the [`TurnOutput`] (intents + logs + notifications) the host then applies.

use std::rc::Rc;

use serde_json::Value;

use crate::model::{LogOpView, ReadModel, TurnOutput};
use crate::permission::NetGrant;
use crate::secrets::SecretStore;

/// Anything that can go wrong evaluating plugin source.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine raised an uncaught error while evaluating.
    #[error("plugin script error: {0}")]
    Script(String),
    /// The engine was interrupted by the host (timeout / gas / depth guard).
    #[error("plugin interrupted: {0}")]
    Interrupted(String),
    /// Serializing to / from the JS boundary failed.
    #[error("host bridge: {0}")]
    Bridge(String),
}

/// A JS engine able to load a plugin and run its commands / op hooks.
///
/// Not `Send`: a Boa `Context` is single-threaded. The host that owns the
/// engine is therefore single-threaded too (fine for TUI/CLI; GUI clients run
/// it on a dedicated plugin thread).
pub trait PluginEngine {
    /// Evaluate the bundle and activate the plugin (registers its commands and
    /// op hooks). Called once per plugin load.
    fn load(&mut self, source: &str) -> Result<(), EngineError>;

    /// Invoke a registered command by id, against a fresh read model + config.
    fn run_command(
        &mut self,
        id: &str,
        read_model: &ReadModel,
        config: &Value,
    ) -> Result<TurnOutput, EngineError>;

    /// Dispatch one applied op to the plugin's `onOp` hooks.
    fn dispatch_op(
        &mut self,
        op: &LogOpView,
        read_model: &ReadModel,
        config: &Value,
    ) -> Result<TurnOutput, EngineError>;

    /// Grant the plugin the network domains it may `fetch` (derived from its
    /// approved `network:<domain>` permissions). Called once at load; a fetch
    /// to a host outside this set is refused inside the engine.
    ///
    /// The grant carries the plugin id too, so a refusal can name who
    /// attempted it — the host log is the only witness a plugin cannot
    /// `catch (e) {}` away, and "some plugin tried to exfiltrate" is not
    /// actionable.
    fn set_network(&mut self, grant: NetGrant);

    /// Load the plugin's local KV for this turn. `enabled` mirrors the
    /// `storage:local` permission — when false, `ctx.storage.*` throws.
    fn set_storage(&mut self, enabled: bool, kv: serde_json::Map<String, Value>);

    /// If the plugin mutated `ctx.storage` this turn, return the new KV for the
    /// host to persist; `None` when nothing changed.
    fn take_dirty_storage(&mut self) -> Option<serde_json::Map<String, Value>>;

    /// Configure `ctx.secrets` for the turn. `enabled` mirrors the `secrets`
    /// permission (when false, `ctx.secrets.*` throws); `service` is the
    /// keychain namespace for this plugin (`outl-plugin:<id>`); `store` is the
    /// backing keychain (`None` keeps secrets inert). The host calls this before
    /// each turn, same as `set_storage`.
    fn set_secrets(&mut self, enabled: bool, service: String, store: Option<Rc<dyn SecretStore>>);

    /// Run a content transformer registered for `lang` against `input`,
    /// returning the descriptor JSON it produced (`{kind, content}`), or `None`
    /// when the plugin has no transformer for that language. Unlike the other
    /// turns this is a pure function — it returns a value instead of buffering
    /// intents.
    fn transform(
        &mut self,
        lang: &str,
        input: &str,
        config: &Value,
    ) -> Result<Option<String>, EngineError>;

    /// Hand the plugin's sync transport the JSONL of locally-produced ops to
    /// ship to its backend. No-op if the plugin registered no transport.
    fn sync_push(&mut self, ops_jsonl: &str, config: &Value) -> Result<(), EngineError>;

    /// Ask the plugin's sync transport for remote ops to apply, returning the
    /// JSONL it fetched (`None` if it registered no transport or has nothing).
    /// The host parses each line into a `LogOp` and routes it through
    /// `Workspace::apply` — the plugin only transports bytes, it never injects
    /// into the tree directly.
    fn sync_pull(&mut self, config: &Value) -> Result<Option<String>, EngineError>;
}
