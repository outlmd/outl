//! `lua` runtime — Lua 5.4 via [mlua](https://github.com/mlua-rs/mlua).
//!
//! mlua statically links the Lua C library through the `vendored`
//! feature, so the host doesn't need Lua installed. We override
//! `print` to funnel into a captured buffer.
//!
//! Gated behind the `lang-lua` feature.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use mlua::{Lua, MultiValue, Value, Variadic};

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

/// mlua-backed Lua 5.4 runtime.
pub struct LuaRuntime;

impl Runtime for LuaRuntime {
    fn language(&self) -> &'static str {
        "lua"
    }

    fn execute(&self, source: &str, _ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();
        let lua = Lua::new();
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
