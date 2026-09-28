//! Boa-backed [`PluginEngine`].
//!
//! Boa is the same pure-Rust JS engine `outl-exec` uses for code blocks (runs
//! on iOS, no JIT). The plugin runtime layers a small bridge on top:
//!
//! - Four native functions capture a shared `Rc<RefCell<EngineShared>>`:
//!   `__outl_read` (read the [`ReadModel`]), `__outl_emit` (push a
//!   [`HostIntent`]), `__outl_log`, `__outl_notify`.
//! - A JS prelude (`PRELUDE`) builds the `ctx` object the plugin's
//!   `activate(ctx)` receives, and the `__outl_dispatch*` entry points the host
//!   calls each turn. Plugin handlers live in JS-land (on `globalThis.__OUTL`),
//!   so the engine never has to store a `JsFunction` in Rust.

// Boa's capturing native functions need `NativeFunction::from_closure`, which
// is `unsafe` (the closure must not let `!Send` captures escape the Context
// lifetime). Our closures capture an `Rc<RefCell<EngineShared>>` we own for the
// engine's whole life, so the invariant holds — same pattern as
// `outl-exec/src/runtimes/js.rs`.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use boa_engine::{js_string, Context, JsValue, NativeFunction, Source};
use serde_json::Value;

use crate::model::{HostIntent, LogOpView, ReadModel, TurnOutput};
use crate::runtime::{EngineError, PluginEngine};
use crate::secrets::SecretStore;

/// State the native bridge functions read and write. Lives behind an `Rc<
/// RefCell<>>` shared between the `Context` closures and the engine.
#[derive(Default)]
struct EngineShared {
    read_model: ReadModel,
    config: Value,
    out: TurnOutput,
    /// Who this plugin is + the domains it may `fetch` (approved permissions).
    net: crate::permission::NetGrant,
    /// Whether `storage:local` is granted. When false, `ctx.storage.*` throws.
    storage_enabled: bool,
    /// Per-plugin local KV, loaded by the host before the turn.
    storage: serde_json::Map<String, Value>,
    /// Set when the plugin mutated `storage` this turn, so the host persists it.
    storage_dirty: bool,
    /// Whether `secrets` is granted. When false, `ctx.secrets.*` throws.
    secrets_enabled: bool,
    /// Keychain service namespace for this plugin's secrets (`outl-plugin:<id>`).
    secret_service: String,
    /// Backing keychain store, injected by the host. `None` keeps `ctx.secrets`
    /// inert (returns null) even when a permission is somehow set — belt and
    /// suspenders against reading an unconfigured store.
    secret_store: Option<Rc<dyn SecretStore>>,
}

/// A plugin engine backed by Boa.
pub struct BoaEngine {
    context: Context,
    shared: Rc<RefCell<EngineShared>>,
}

impl BoaEngine {
    /// Create an engine with the bridge natives and prelude installed.
    pub fn new() -> Result<Self, EngineError> {
        let mut context = Context::default();

        // Gas: cap loop iterations, recursion, and stack so a runaway plugin
        // (infinite loop, unbounded recursion) is interrupted with a JS error
        // instead of hanging the thread. Boa enforces these cooperatively.
        let limits = context.runtime_limits_mut();
        limits.set_loop_iteration_limit(20_000_000);
        limits.set_recursion_limit(2_000);
        limits.set_stack_size_limit(16 * 1024);

        let shared = Rc::new(RefCell::new(EngineShared::default()));

        register_natives(&mut context, &shared)?;
        context
            .eval(Source::from_bytes(PRELUDE))
            .map_err(|e| EngineError::Bridge(format!("prelude: {e}")))?;

        Ok(Self { context, shared })
    }

    /// Set the per-turn read model + config, clearing last turn's output.
    fn begin_turn(&mut self, read_model: &ReadModel, config: &Value) {
        let mut s = self.shared.borrow_mut();
        s.read_model = read_model.clone();
        s.config = config.clone();
        s.out = TurnOutput::default();
    }

    /// Run a JS snippet, then flush the microtask queue (async handlers).
    fn eval_and_drain(&mut self, js: &str) -> Result<(), EngineError> {
        self.context
            .eval(Source::from_bytes(js))
            .map_err(|e| EngineError::Script(e.to_string()))?;
        // Async command/hook bodies park their continuation on the job queue;
        // run it so the intents actually get emitted before we read them back.
        let _ = self.context.run_jobs();
        Ok(())
    }

    /// Take the output accumulated this turn.
    fn take_output(&mut self) -> TurnOutput {
        std::mem::take(&mut self.shared.borrow_mut().out)
    }
}

impl PluginEngine for BoaEngine {
    fn load(&mut self, source: &str) -> Result<(), EngineError> {
        self.context
            .eval(Source::from_bytes(source))
            .map_err(|e| EngineError::Script(e.to_string()))?;
        // `definePlugin` stashed the def on `globalThis.__OUTL.def`; activate it.
        self.eval_and_drain("__outl_activate();")
    }

    fn run_command(
        &mut self,
        id: &str,
        read_model: &ReadModel,
        config: &Value,
    ) -> Result<TurnOutput, EngineError> {
        self.begin_turn(read_model, config);
        let id_json = serde_json::to_string(id).map_err(|e| EngineError::Bridge(e.to_string()))?;
        self.eval_and_drain(&format!("__outl_dispatchCommand({id_json});"))?;
        Ok(self.take_output())
    }

    fn dispatch_op(
        &mut self,
        op: &LogOpView,
        read_model: &ReadModel,
        config: &Value,
    ) -> Result<TurnOutput, EngineError> {
        self.begin_turn(read_model, config);
        let op_json = serde_json::to_string(op).map_err(|e| EngineError::Bridge(e.to_string()))?;
        self.eval_and_drain(&format!("__outl_dispatchOp({op_json});"))?;
        Ok(self.take_output())
    }

    fn set_network(&mut self, grant: crate::permission::NetGrant) {
        self.shared.borrow_mut().net = grant;
    }

    fn set_storage(&mut self, enabled: bool, kv: serde_json::Map<String, Value>) {
        let mut s = self.shared.borrow_mut();
        s.storage_enabled = enabled;
        s.storage = kv;
        s.storage_dirty = false;
    }

    fn take_dirty_storage(&mut self) -> Option<serde_json::Map<String, Value>> {
        let mut s = self.shared.borrow_mut();
        if s.storage_dirty {
            s.storage_dirty = false;
            Some(s.storage.clone())
        } else {
            None
        }
    }

    fn set_secrets(&mut self, enabled: bool, service: String, store: Option<Rc<dyn SecretStore>>) {
        let mut s = self.shared.borrow_mut();
        s.secrets_enabled = enabled;
        s.secret_service = service;
        s.secret_store = store;
    }

    fn sync_push(&mut self, ops_jsonl: &str, config: &Value) -> Result<(), EngineError> {
        self.begin_turn(&ReadModel::default(), config);
        let arg =
            serde_json::to_string(ops_jsonl).map_err(|e| EngineError::Bridge(e.to_string()))?;
        self.eval_and_drain(&format!("__outl_syncPush({arg});"))
    }

    fn sync_pull(&mut self, config: &Value) -> Result<Option<String>, EngineError> {
        self.begin_turn(&ReadModel::default(), config);
        let value = self
            .context
            .eval(Source::from_bytes("__outl_syncPull()"))
            .map_err(|e| EngineError::Script(e.to_string()))?;
        let _ = self.context.run_jobs();
        if value.is_null() || value.is_undefined() {
            return Ok(None);
        }
        let s = value
            .to_string(&mut self.context)
            .map_err(|e| EngineError::Bridge(e.to_string()))?
            .to_std_string_escaped();
        Ok(Some(s))
    }

    fn transform(
        &mut self,
        lang: &str,
        input: &str,
        config: &Value,
    ) -> Result<Option<String>, EngineError> {
        self.begin_turn(&ReadModel::default(), config);
        let lang_json =
            serde_json::to_string(lang).map_err(|e| EngineError::Bridge(e.to_string()))?;
        let input_json =
            serde_json::to_string(input).map_err(|e| EngineError::Bridge(e.to_string()))?;
        let js = format!("__outl_dispatchTransform({lang_json}, {input_json})");
        let value = self
            .context
            .eval(Source::from_bytes(&js))
            .map_err(|e| EngineError::Script(e.to_string()))?;
        let _ = self.context.run_jobs();
        if value.is_null() || value.is_undefined() {
            return Ok(None);
        }
        let s = value
            .to_string(&mut self.context)
            .map_err(|e| EngineError::Bridge(e.to_string()))?
            .to_std_string_escaped();
        Ok(Some(s))
    }
}

/// Register the four native bridge functions on the context.
fn register_natives(
    context: &mut Context,
    shared: &Rc<RefCell<EngineShared>>,
) -> Result<(), EngineError> {
    // __outl_read(queryJson) -> resultJson
    let read_shared = shared.clone();
    let read_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let query = arg_string(args, ctx);
            let result = read_query(&read_shared.borrow().read_model, &query);
            Ok(JsValue::from(js_string!(result)))
        })
    };
    reg(context, "__outl_read", 1, read_fn)?;

    // __outl_config() -> configJson
    let cfg_shared = shared.clone();
    let cfg_fn = unsafe {
        NativeFunction::from_closure(move |_, _args, _ctx| {
            let json = serde_json::to_string(&cfg_shared.borrow().config)
                .unwrap_or_else(|_| "null".into());
            Ok(JsValue::from(js_string!(json)))
        })
    };
    reg(context, "__outl_config", 0, cfg_fn)?;

    // __outl_emit(intentJson) -> undefined
    let emit_shared = shared.clone();
    let emit_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let json = arg_string(args, ctx);
            if let Ok(intent) = serde_json::from_str::<HostIntent>(&json) {
                emit_shared.borrow_mut().out.intents.push(intent);
            }
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_emit", 1, emit_fn)?;

    // __outl_log(msg) and __outl_notify(msg)
    let log_shared = shared.clone();
    let log_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            log_shared.borrow_mut().out.logs.push(arg_string(args, ctx));
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_log", 1, log_fn)?;

    let notify_shared = shared.clone();
    let notify_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            notify_shared
                .borrow_mut()
                .out
                .notifications
                .push(arg_string(args, ctx));
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_notify", 1, notify_fn)?;

    // __outl_render(html) -> undefined — author-written markup for the GUI
    // client to run in a sandboxed iframe. The engine only transports it.
    let render_shared = shared.clone();
    let render_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            render_shared
                .borrow_mut()
                .out
                .views
                .push(arg_string(args, ctx));
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_render", 1, render_fn)?;

    // __outl_fetch(url, optsJson) -> resultJson. Blocking HTTP gated by the
    // plugin's approved network domains. Refused (not thrown) for an unapproved
    // host so the plugin gets a structured `{ ok: false, error }` it can handle.
    let fetch_shared = shared.clone();
    let fetch_fn = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let url = arg_string(args, ctx);
            let opts = args
                .get(1)
                .and_then(|v| v.to_string(ctx).ok())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let result = crate::net::fetch(&fetch_shared.borrow().net, &url, &opts);
            Ok(JsValue::from(js_string!(result)))
        })
    };
    reg(context, "__outl_fetch", 2, fetch_fn)?;

    // __outl_storage_get(key) -> valueJson | null. Throws if storage:local
    // is not granted.
    let get_shared = shared.clone();
    let storage_get = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let s = get_shared.borrow();
            storage_guard(&s)?;
            let key = arg_string(args, ctx);
            let v = s.storage.get(&key).cloned().unwrap_or(Value::Null);
            Ok(JsValue::from(js_string!(v.to_string())))
        })
    };
    reg(context, "__outl_storage_get", 1, storage_get)?;

    // __outl_storage_set(key, valueJson) -> undefined.
    let set_shared = shared.clone();
    let storage_set = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let mut s = set_shared.borrow_mut();
            storage_guard(&s)?;
            let key = arg_string(args, ctx);
            let raw = args
                .get(1)
                .and_then(|v| v.to_string(ctx).ok())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "null".into());
            let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            s.storage.insert(key, value);
            s.storage_dirty = true;
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_storage_set", 2, storage_set)?;

    // __outl_storage_delete(key) -> undefined.
    let del_shared = shared.clone();
    let storage_del = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let mut s = del_shared.borrow_mut();
            storage_guard(&s)?;
            let key = arg_string(args, ctx);
            if s.storage.remove(&key).is_some() {
                s.storage_dirty = true;
            }
            Ok(JsValue::undefined())
        })
    };
    reg(context, "__outl_storage_delete", 1, storage_del)?;

    // __outl_secret_get(key) -> valueJson | null. Reads the plugin's own secret
    // from the OS keychain (namespaced by `secret_service`). Throws if the
    // `secrets` permission is not granted; returns null when the key is unset or
    // the keychain read fails (the plugin treats both as "not configured").
    let secret_shared = shared.clone();
    let secret_get = unsafe {
        NativeFunction::from_closure(move |_, args, ctx| {
            let s = secret_shared.borrow();
            secret_guard(&s)?;
            let key = arg_string(args, ctx);
            let value = match &s.secret_store {
                Some(store) => store.get(&s.secret_service, &key).unwrap_or(None),
                None => None,
            };
            let json = match value {
                Some(v) => serde_json::to_string(&v).unwrap_or_else(|_| "null".into()),
                None => "null".into(),
            };
            Ok(JsValue::from(js_string!(json)))
        })
    };
    reg(context, "__outl_secret_get", 1, secret_get)?;

    Ok(())
}

/// Reject a storage call when `storage:local` was not granted.
fn storage_guard(s: &EngineShared) -> boa_engine::JsResult<()> {
    if s.storage_enabled {
        Ok(())
    } else {
        Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(
            "ctx.storage needs the `storage:local` permission"
        ))))
    }
}

/// Reject a secrets call when the `secrets` permission was not granted.
fn secret_guard(s: &EngineShared) -> boa_engine::JsResult<()> {
    if s.secrets_enabled {
        Ok(())
    } else {
        Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(
            "ctx.secrets needs the `secrets` permission"
        ))))
    }
}

fn reg(
    context: &mut Context,
    name: &str,
    arity: usize,
    f: NativeFunction,
) -> Result<(), EngineError> {
    context
        .register_global_callable(js_string!(name), arity, f)
        .map_err(|e| EngineError::Bridge(format!("register {name}: {e}")))
}

/// Pull the first arg as a Rust `String` (defaults to empty).
fn arg_string(args: &[JsValue], ctx: &mut Context) -> String {
    args.first()
        .and_then(|v| v.to_string(ctx).ok())
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default()
}

/// Run a `{kind, ...}` read query against the model, returning JSON.
fn read_query(model: &ReadModel, query_json: &str) -> String {
    let q: Value = serde_json::from_str(query_json).unwrap_or(Value::Null);
    let kind = q.get("kind").and_then(Value::as_str).unwrap_or("");
    let result: Value = match kind {
        "blocks" => {
            let filter = q.get("filter").cloned().unwrap_or(Value::Null);
            let matched: Vec<_> = model
                .blocks
                .iter()
                .filter(|b| block_matches(b, &filter))
                .collect();
            serde_json::to_value(matched).unwrap_or(Value::Null)
        }
        "block" => {
            let id = q.get("id").and_then(Value::as_str).unwrap_or("");
            serde_json::to_value(model.blocks.iter().find(|b| b.id == id)).unwrap_or(Value::Null)
        }
        "pages" => serde_json::to_value(&model.pages).unwrap_or(Value::Null),
        "templates" => serde_json::to_value(&model.templates).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    serde_json::to_string(&result).unwrap_or_else(|_| "null".into())
}

/// Apply a `{ page?, todo?, textContains? }` filter to a block.
fn block_matches(b: &crate::model::BlockView, filter: &Value) -> bool {
    if let Some(page) = filter.get("page").and_then(Value::as_str) {
        if b.page != page {
            return false;
        }
    }
    if let Some(todo) = filter.get("todo").and_then(Value::as_str) {
        if b.todo.as_deref() != Some(todo) {
            return false;
        }
    }
    if let Some(needle) = filter.get("textContains").and_then(Value::as_str) {
        if !b.text.contains(needle) {
            return false;
        }
    }
    true
}

/// JS glue the host controls: builds the `ctx` object and the dispatch entry
/// points. Plugin handlers register onto `globalThis.__OUTL`.
const PRELUDE: &str = r#"
globalThis.__OUTL = { def: null, commands: {}, opHooks: [], transformers: {}, sync: null };

// definePlugin (from @outl/plugin-sdk) calls this in the bundle.
globalThis.__outl_register = (def) => { globalThis.__OUTL.def = def; };

globalThis.console = {
    log:   (...a) => __outl_log(a.map(String).join(' ')),
    warn:  (...a) => __outl_log(a.map(String).join(' ')),
    error: (...a) => __outl_log(a.map(String).join(' ')),
    info:  (...a) => __outl_log(a.map(String).join(' ')),
};

function __outl_ctx() {
    const O = globalThis.__OUTL;
    const read = (q) => JSON.parse(__outl_read(JSON.stringify(q)));
    return {
        ops: { onOp: (cb) => { O.opHooks.push(cb); } },
        commands: { register: (id, h) => { O.commands[id] = h; } },
        content: { register: (lang, fn) => { O.transformers[lang] = fn; } },
        sync: { register: (transport) => { O.sync = transport; } },
        blocks: {
            query: (f) => read({ kind: 'blocks', filter: f || {} }),
            get: (id) => read({ kind: 'block', id }),
            edit: (id, text) => __outl_emit(JSON.stringify({ op: 'edit-text', node: id, text })),
            create: (parent, text) => __outl_emit(JSON.stringify({ op: 'create-under', parent, text })),
            createAfter: (after, text) => __outl_emit(JSON.stringify({ op: 'create-after', after, text })),
            move: (id, target) => __outl_emit(JSON.stringify({ op: 'move', node: id, target })),
            toggleTodo: (id) => __outl_emit(JSON.stringify({ op: 'toggle-todo', node: id })),
            delete: (id) => __outl_emit(JSON.stringify({ op: 'delete', node: id })),
            appendTree: (parent, tree) => __outl_emit(JSON.stringify({ op: 'append-tree', target: { toParent: parent }, tree })),
        },
        page: {
            list: () => read({ kind: 'pages' }),
            create: (slug) => __outl_emit(JSON.stringify({ op: 'ensure-page', slug })),
            appendTree: (slug, tree) => __outl_emit(JSON.stringify({ op: 'append-tree', target: { toPage: slug }, tree })),
            open: () => { throw new Error('ctx.page.open is not available yet in this outl version (roadmap)'); },
            today: () => { throw new Error('ctx.page.today is not available yet in this outl version (roadmap)'); },
        },
        template: {
            list: () => read({ kind: 'templates' }),
            instantiate: (name, targetBlock) => __outl_emit(JSON.stringify({ op: 'instantiate-template', name, under: targetBlock })),
        },
        config: { get: () => JSON.parse(__outl_config()) },
        log: { info: (m) => __outl_log(String(m)), warn: (m) => __outl_log(String(m)), error: (m) => __outl_log(String(m)) },
        ui: {
            notify: (m) => __outl_notify(String(m)),
            render: (html) => __outl_render(String(html)),
        },
        // Declared in the SDK + manifest permissions, not yet wired in the
        // engine. Stubbed to fail loudly (clear message > `undefined is not a
        // function`) so authors know it's roadmap, not a typo.
        storage: {
            get: (key) => JSON.parse(__outl_storage_get(String(key))),
            set: (key, value) => __outl_storage_set(String(key), JSON.stringify(value === undefined ? null : value)),
            delete: (key) => __outl_storage_delete(String(key)),
        },
        // Read-only from the plugin's side: the value lives in the OS keychain,
        // set out-of-band by the user (`outl plugin secret set` / a client's
        // plugin settings). Returns null when unset. Gated by `secrets`.
        secrets: {
            get: (key) => JSON.parse(__outl_secret_get(String(key))),
        },
        net: {
            // Blocking under the hood (on the plugin's own thread). The native
            // returns { ok, status, body } (or { ok:false, error }); we wrap it
            // in a FetchResponse with text()/json() so the SDK shape holds.
            // Host must be covered by an approved network:<domain> permission.
            fetch: (url, opts) => {
                const raw = JSON.parse(__outl_fetch(String(url), JSON.stringify(opts || {})));
                return {
                    ok: !!raw.ok,
                    status: raw.status || 0,
                    headers: raw.headers || {},
                    error: raw.error,
                    text: () => raw.body || '',
                    json: () => JSON.parse(raw.body || 'null'),
                };
            },
        },
    };
}

globalThis.__outl_activate = () => {
    const O = globalThis.__OUTL;
    if (O.def && typeof O.def.activate === 'function') {
        O.def.activate(__outl_ctx());
    }
};

globalThis.__outl_dispatchCommand = (id) => {
    const h = globalThis.__OUTL.commands[id];
    if (h) { h(); }
};

globalThis.__outl_dispatchOp = (op) => {
    for (const cb of globalThis.__OUTL.opHooks) { cb(op); }
};

// Returns the transformer's descriptor object ({ kind, content }) or null.
// The host stringifies/None-checks the return value (not the buffer).
globalThis.__outl_dispatchTransform = (lang, input) => {
    const fn = globalThis.__OUTL.transformers[lang];
    if (!fn) { return null; }
    try {
        const r = fn(input);
        return r == null ? null : JSON.stringify(r);
    } catch (e) {
        return null;
    }
};

// Sync transport: hand the plugin local ops to ship; ask it for remote ops.
// `push`/`pull` are the methods on the object passed to ctx.sync.register.
globalThis.__outl_syncPush = (opsJsonl) => {
    const s = globalThis.__OUTL.sync;
    if (s && typeof s.push === 'function') { try { s.push(opsJsonl); } catch (e) {} }
};
globalThis.__outl_syncPull = () => {
    const s = globalThis.__OUTL.sync;
    if (!s || typeof s.pull !== 'function') { return null; }
    try {
        const r = s.pull();
        return r == null ? null : String(r);
    } catch (e) {
        return null;
    }
};
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockView, MoveTarget};

    fn rm_with(blocks: Vec<BlockView>) -> ReadModel {
        ReadModel {
            blocks,
            ..Default::default()
        }
    }

    /// A plugin written against the raw `definePlugin` contract, no SDK bundle.
    const PLUGIN: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('archive', () => {
                    const done = ctx.blocks.query({ todo: 'DONE' });
                    for (const b of done) {
                        ctx.blocks.move(b.id, { toPage: ctx.config.get().archivePage });
                    }
                    ctx.ui.notify(done.length + ' archived');
                });
                ctx.ops.onOp((op) => {
                    if (op.kind === 'Edit') ctx.log.info('edited ' + op.node);
                });
            }
        });
    "#;

    #[test]
    fn command_reads_model_and_emits_intents() {
        let mut e = BoaEngine::new().unwrap();
        e.load(PLUGIN).unwrap();

        let rm = rm_with(vec![
            BlockView {
                id: "a".into(),
                text: "x".into(),
                todo: Some("DONE".into()),
                parent: None,
                page: "p".into(),
            },
            BlockView {
                id: "b".into(),
                text: "y".into(),
                todo: Some("TODO".into()),
                parent: None,
                page: "p".into(),
            },
            BlockView {
                id: "c".into(),
                text: "z".into(),
                todo: Some("DONE".into()),
                parent: None,
                page: "p".into(),
            },
        ]);
        let config = serde_json::json!({ "archivePage": "archive" });
        let out = e.run_command("archive", &rm, &config).unwrap();

        // Two DONE blocks → two move intents to `archive`.
        assert_eq!(out.intents.len(), 2);
        assert_eq!(
            out.intents[0],
            HostIntent::Move {
                node: "a".into(),
                target: MoveTarget::ToPage {
                    to_page: "archive".into()
                },
            }
        );
        assert_eq!(out.notifications, vec!["2 archived"]);
    }

    #[test]
    fn unknown_command_is_a_noop() {
        let mut e = BoaEngine::new().unwrap();
        e.load(PLUGIN).unwrap();
        let out = e
            .run_command("nope", &ReadModel::default(), &Value::Null)
            .unwrap();
        assert!(out.intents.is_empty());
    }

    #[test]
    fn op_hook_fires_and_logs() {
        let mut e = BoaEngine::new().unwrap();
        e.load(PLUGIN).unwrap();
        let op = LogOpView {
            kind: "Edit".into(),
            node: "n7".into(),
            text: Some("hi".into()),
            todo: None,
        };
        let out = e
            .dispatch_op(&op, &ReadModel::default(), &Value::Null)
            .unwrap();
        assert_eq!(out.logs, vec!["edited n7"]);
    }

    #[test]
    fn script_error_surfaces() {
        let mut e = BoaEngine::new().unwrap();
        assert!(e.load("this is not valid js {{{").is_err());
    }

    #[test]
    fn gas_interrupts_runaway_recursion() {
        // Unbounded recursion hits the recursion limit and surfaces as an
        // error instead of blowing the stack / hanging the thread.
        let mut e = BoaEngine::new().unwrap();
        e.load(
            r#"globalThis.__outl_register({ activate(ctx) {
                ctx.commands.register('boom', () => { function f(){ return f(); } f(); });
            }});"#,
        )
        .unwrap();
        let out = e.run_command("boom", &ReadModel::default(), &Value::Null);
        assert!(out.is_err(), "runaway recursion should be interrupted");
    }
}
