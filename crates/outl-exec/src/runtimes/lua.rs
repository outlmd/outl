//! `lua` runtime — Lua 5.4 via [mlua](https://github.com/mlua-rs/mlua).
//!
//! mlua statically links the Lua C library through the `vendored`
//! feature, so the host doesn't need Lua installed. We override
//! `print` to funnel into a captured buffer.
//!
//! Gated behind the `lang-lua` feature.
//!
//! # What a block may reach
//!
//! `stdlib` is the whole answer, and it is an allowlist because a
//! denylist here fails open: every mlua release that adds a library
//! would add it to a `lua` block without anyone deciding to.
//!
//! `Lua::new()` is **not** what this runtime uses. It loads
//! [`mlua::StdLib::ALL_SAFE`], and "safe" in mlua's vocabulary means
//! *memory-safe* — it excludes `debug` and `ffi` and includes `os`
//! (which carries `execute`), `io` (arbitrary file access) and
//! `package` (which carries `require`). See issue #278.
//!
//! # For how long
//!
//! Three layers, because no single one covers every way a block can
//! refuse to finish:
//!
//! 1. mlua's instruction hook raises past `ctx.timeout`, so the VM
//!    unwinds itself and the thread ends. This is the path that fires
//!    for ordinary Lua code, and it stops the work.
//! 2. `ctx.mem_limit`, through `Lua::set_memory_limit`. The hook counts
//!    VM instructions and cannot fire inside one long C call, and
//!    `string.rep('x', 2e9)` is a single instruction that allocates two
//!    gigabytes. The allocator refusing is what stops that one.
//! 3. [`with_timeout`] around the whole run. A C call that is long
//!    without allocating (`string.rep` under a generous cap, a
//!    pathological `string.find` pattern) is caught by neither hook
//!    nor allocator, so the interpreter lives on a worker thread and
//!    the caller is released shortly after the deadline no matter what
//!    (`C_CALL_GRACE` past it, so the hook gets to answer first). When
//!    the C call eventually returns, the hook fires and the thread
//!    exits on its own, so unlike `python` and `js` this leak is
//!    bounded by the C call, not by the script.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlua::{Lua, LuaOptions, MultiValue, StdLib, Value, Variadic};

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};
use crate::sandbox::with_timeout;

/// The Lua standard libraries a code block is allowed to see.
///
/// The language, and nothing that reaches the host: string handling,
/// tables, arithmetic, unicode and coroutines. Deliberately absent:
///
/// - `os` — `execute` is a shell, `getenv` reads the environment.
/// - `io` — arbitrary file read and write, outside the workspace.
/// - `package` — `require` pulls a module off disk.
/// - `debug` — reaches into the interpreter itself (also not in
///   `ALL_SAFE`).
///
/// A function rather than a `const` because `StdLib` wraps a private
/// `u32` and its `BitOr` is not `const`.
fn stdlib() -> StdLib {
    StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE
}

/// Base-library globals removed after the interpreter is built.
///
/// These are not part of any [`StdLib`] flag — Lua's base library is
/// always loaded — and each one re-opens what `stdlib` closed: a
/// loader turns a string into code, so leaving them is a lock with the
/// key next to it.
const LOADERS: &[&str] = &["load", "loadfile", "dofile", "require"];

/// How often the deadline hook runs, in VM instructions.
///
/// A wall-clock check costs an `Instant::now`, so this trades deadline
/// precision for interpreter throughput. 10k instructions is well under
/// a millisecond of Lua on any machine outl runs on, which keeps
/// overshoot far below the smallest timeout a caller would set.
const DEADLINE_CHECK_INTERVAL: u32 = 10_000;

/// How long past `ctx.timeout` the worker gets before the caller is
/// released without it.
///
/// The hook is the primary path: it fires at the deadline for any Lua
/// code and the worker returns `Err(Timeout)` through the channel like
/// a normal result. Releasing the caller at the exact same instant
/// would race it, and a worker abandoned a millisecond before it
/// finished on its own would be counted against
/// `sandbox::MAX_RUNAWAY_WORKERS` for that millisecond. The grace makes
/// the wrapper fire only for the case it exists for: a single C call
/// that outlasts the deadline by more than this.
const C_CALL_GRACE: Duration = Duration::from_millis(100);

/// mlua-backed Lua 5.4 runtime.
pub struct LuaRuntime;

impl Runtime for LuaRuntime {
    fn language(&self) -> &'static str {
        "lua"
    }

    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let source = source.to_string();
        let timeout = ctx.timeout;
        let mem_limit = ctx.mem_limit;
        // The interpreter is built *inside* the worker: `Lua` is not
        // `Send`, and a worker that outlives the caller must own it.
        with_timeout(timeout + C_CALL_GRACE, move || {
            run_isolated(&source, timeout, mem_limit)
        })
    }
}

fn run_isolated(
    source: &str,
    timeout: Duration,
    mem_limit: Option<usize>,
) -> Result<ExecOutput, ExecError> {
    let start = Instant::now();
    let lua = Lua::new_with(stdlib(), LuaOptions::default())
        .map_err(|e| ExecError::Sandbox(format!("build interpreter: {e}")))?;

    // Lua's base library is always loaded, so the loaders are not
    // covered by `stdlib()` and have to go by hand.
    let globals = lua.globals();
    for name in LOADERS {
        globals
            .set(*name, Value::Nil)
            .map_err(|e| ExecError::Sandbox(format!("remove global `{name}`: {e}")))?;
    }

    // A deadline counts VM instructions, so it cannot fire inside a
    // single long C call: `string.rep('x', 2e9)` is one instruction
    // and two gigabytes. Honouring `mem_limit` is the other half of
    // bounding a block, and lua is the one runtime that can.
    // `ExecContext::default()` and `orchestrate` both set it, so a
    // caller has to opt out of the cap on purpose.
    if let Some(limit) = mem_limit {
        lua.set_memory_limit(limit)
            .map_err(|e| ExecError::Sandbox(format!("set memory limit: {e}")))?;
    }

    // Cooperative cancellation: the hook runs every
    // `DEADLINE_CHECK_INTERVAL` VM instructions and returns an error
    // past the deadline, which unwinds the interpreter the same way
    // a script calling `error()` does. Real abort for Lua code; the
    // `with_timeout` wrapper in `execute` only matters for the C
    // call this hook cannot interrupt, and even then this hook is
    // what ends the worker once that call returns.
    let deadline = start + timeout;
    // The hook raises the flag *before* returning the error, and the
    // flag — not the error's message — is what `execute` reads. A
    // string sentinel would be forgeable: a script calling
    // `error("<sentinel>")` would be reported as a timeout, and
    // since `ExecError::Timeout` is an infrastructure error the
    // orchestrator writes no result block, so the script would
    // suppress its own failure from the page. The script cannot
    // reach this `AtomicBool`.
    let expired = Arc::new(AtomicBool::new(false));
    let hook_flag = expired.clone();
    // `set_global_hook`, not `set_hook`: the latter installs per
    // Lua thread, and mlua's per-thread `hook_proc` looks the
    // callback up in a registry table keyed by the running thread —
    // a coroutine the *script* creates goes through `lua_newthread`
    // directly, is not in that table, and mlua responds by turning
    // the hook off. So `coroutine.wrap(function() while true do end
    // end)()` ran with no deadline at all. The global hook is
    // inherited by every thread.
    lua.set_global_hook(
        mlua::HookTriggers::new().every_nth_instruction(DEADLINE_CHECK_INTERVAL),
        move |_lua, _debug| {
            if Instant::now() >= deadline {
                hook_flag.store(true, Ordering::Relaxed);
                Err(mlua::Error::runtime("outl-exec: deadline exceeded"))
            } else {
                Ok(mlua::VmState::Continue)
            }
        },
    )
    .map_err(|e| ExecError::Sandbox(format!("install deadline hook: {e}")))?;

    let buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // Override `print` to write to our buffer instead of stdout.
    let sink = buffer.clone();
    let print_fn = lua
        .create_function(move |_, args: Variadic<Value>| {
            let mut s = sink.lock().unwrap();
            for (i, v) in args.iter().enumerate() {
                if i > 0 {
                    s.push('\t');
                }
                s.push_str(&lua_value_tostring(v));
            }
            s.push('\n');
            Ok(())
        })
        .map_err(|e| ExecError::Sandbox(format!("install print: {e}")))?;
    lua.globals()
        .set("print", print_fn)
        .map_err(|e| ExecError::Sandbox(format!("set global print: {e}")))?;

    match lua.load(source).eval::<MultiValue>() {
        // The deadline is checked here too, not only on the error
        // arm: the hook raises an ordinary Lua error, and `pcall`
        // catches ordinary Lua errors. `pcall(function() while true
        // do end end)` therefore swallowed the deadline and the
        // block reported success with whatever it printed next.
        _ if expired.load(Ordering::Relaxed) => Err(ExecError::Timeout(timeout)),
        Ok(values) => {
            let mut stdout = buffer.lock().unwrap().clone();
            if stdout.is_empty() && !values.is_empty() {
                // Auto-display the last value of the chunk: `1 + 2`
                // returns 3 from `eval`, we show it.
                if let Some(last) = values.iter().last() {
                    stdout.push_str(&lua_value_tostring(last));
                }
            }
            Ok(ExecOutput {
                stdout,
                stderr: String::new(),
                duration: start.elapsed(),
                exit: ExitStatus::Ok,
                format: OutputFormat::Text,
            })
        }
        Err(e) => Ok(ExecOutput {
            stdout: buffer.lock().unwrap().clone(),
            stderr: e.to_string(),
            duration: start.elapsed(),
            exit: ExitStatus::Trap("lua-error".into()),
            format: OutputFormat::Text,
        }),
    }
}

fn lua_value_tostring(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        LuaRuntime
            .execute(src, &ExecContext::default())
            .unwrap()
            .stdout
    }

    #[test]
    fn print_writes_stdout() {
        assert_eq!(run("print(1 + 2)"), "3\n");
    }

    #[test]
    fn return_value_auto_printed() {
        assert_eq!(run("return 6 * 7"), "42");
    }

    #[test]
    fn string_concat() {
        assert_eq!(run("print('hello ' .. 'world')"), "hello world\n");
    }

    #[test]
    fn tables_and_loops() {
        let out = run("local s=0; for i=1,5 do s=s+i end; print(s)");
        assert_eq!(out, "15\n");
    }

    #[test]
    fn syntax_error_returns_trap() {
        let out = LuaRuntime
            .execute("function (", &ExecContext::default())
            .unwrap();
        assert!(matches!(out.exit, ExitStatus::Trap(_)));
    }
}
