//! `js` runtime — JavaScript via [Boa](https://boajs.dev).
//!
//! Boa is a JS engine in pure Rust, ES2015+ with ongoing work toward
//! full ECMAScript conformance. Good enough for snippets in notes
//! ("compute the slug of this title", "format this date"), nowhere
//! near production V8.
//!
//! We expose a single native `__outl_log` and prepend a tiny shim that
//! wires it into `console.log` / `.warn` / `.error`, so user code can
//! call `console.log(...)` naturally and the output lands in our
//! buffer.
//!
//! Gated behind the `lang-js` feature.

// Boa's only way to register a *capturing* native function is
// `NativeFunction::from_closure`, which is `unsafe` because the
// closure must not capture data that's `!Send` in a way that escapes
// `Context`'s lifetime. Our closure captures an `Rc<RefCell<String>>`
// we own throughout `execute`, so the invariant holds trivially.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use boa_engine::{js_string, Context, JsValue, NativeFunction, Source};

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

/// Boa-backed JavaScript runtime.
pub struct JsRuntime;

const CONSOLE_SHIM: &str = r#"
globalThis.console = {
    log:   (...a) => __outl_log(a.map(x => String(x)).join(' ') + '\n'),
    warn:  (...a) => __outl_log(a.map(x => String(x)).join(' ') + '\n'),
    error: (...a) => __outl_log(a.map(x => String(x)).join(' ') + '\n'),
    info:  (...a) => __outl_log(a.map(x => String(x)).join(' ') + '\n'),
};
"#;

impl Runtime for JsRuntime {
    fn language(&self) -> &'static str {
        "js"
    }

    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();
        let mut context = Context::default();
        // Prevent unused-variable warning when lang-query is off.
        let _ = ctx;
        let sink: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

        // Register `__outl_log(string)` as a native fn that pushes
        // into the shared buffer. The shim above turns `console.log`
        // calls into invocations of this.
        let log_sink = sink.clone();
        let log_fn = unsafe {
            NativeFunction::from_closure(move |_, args, ctx| {
                if let Some(arg) = args.first() {
                    let s = arg.to_string(ctx)?;
                    log_sink.borrow_mut().push_str(&s.to_std_string_escaped());
                }
                Ok(JsValue::undefined())
            })
        };
        context
            .register_global_callable(js_string!("__outl_log"), 1, log_fn)
            .map_err(|e| ExecError::Sandbox(format!("register __outl_log: {e}")))?;

        // Register `outl.query(params)` — structured workspace query
        // available to JS plugins and code blocks. Captures
        // `workspace_root` so it can build a WorkspaceIndex lazily.
        #[cfg(feature = "lang-query")]
        {
            let ws_root = Rc::new(ctx.workspace_root.clone());
            // Deliberately does NOT reuse `ctx.index`, unlike the
            // `query` runtime. Boa's only way to register a capturing
            // native fn stores the closure in the `Context`, which
            // outlives this call, so a borrowed index cannot travel
            // into it — and the alternative (owning it behind an `Arc`)
            // would make every resident client clone a 64k-block index
            // per fence to serve a path that is not the hot one. The
            // hot path is the ` ```query ` fence, which auto-runs on
            // every page load; `outl.query()` is a plugin call.
            let query_fn = unsafe {
                NativeFunction::from_closure(move |_, args, js_ctx| {
                    let root = ws_root.clone();
                    let arg0 = args.first().cloned().unwrap_or(JsValue::undefined());
                    let params = js_value_to_query_params(&arg0, js_ctx).map_err(|e| {
                        boa_engine::JsError::from(
                            boa_engine::error::JsNativeError::typ().with_message(e),
                        )
                    })?;
                    let hits = super::query::run_query_structured(&params, &root).map_err(|e| {
                        boa_engine::JsError::from(
                            boa_engine::error::JsNativeError::typ().with_message(e),
                        )
                    })?;
                    hits_to_js_array(&hits, js_ctx).map_err(|e| {
                        boa_engine::JsError::from(
                            boa_engine::error::JsNativeError::typ().with_message(e),
                        )
                    })
                })
            };
            let outl_obj = boa_engine::object::ObjectInitializer::new(&mut context)
                .function(query_fn, js_string!("query"), 1)
                .build();
            context
                .register_global_property(
                    js_string!("outl"),
                    JsValue::from(outl_obj),
                    boa_engine::property::Attribute::all(),
                )
                .map_err(|e| ExecError::Sandbox(format!("register outl: {e}")))?;
        }
        // Run the shim that wires console.log → __outl_log. Errors
        // here would mean a broken Boa install, so just panic-via-?.
        let _ = context
            .eval(Source::from_bytes(CONSOLE_SHIM))
            .map_err(|e| ExecError::Sandbox(format!("console shim: {e}")))?;

        // Don't carry the shim's `undefined` over as the auto-print
        // value — only the user script's last expression matters.
        let value = match context.eval(Source::from_bytes(source)) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ExecOutput {
                    stdout: sink.borrow().clone(),
                    stderr: e.to_string(),
                    duration: start.elapsed(),
                    exit: ExitStatus::Trap("js-error".into()),
                    format: OutputFormat::Text,
                });
            }
        };

        let mut stdout = sink.borrow().clone();
        if stdout.is_empty() && !value.is_undefined() {
            let s = value
                .to_string(&mut context)
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|_| format!("{value:?}"));
            stdout.push_str(&s);
        }
        Ok(ExecOutput {
            stdout,
            stderr: String::new(),
            duration: start.elapsed(),
            exit: ExitStatus::Ok,
            format: OutputFormat::Text,
        })
    }
}

/// Convert a JS value (expected: plain object) into [`QueryParams`].
#[cfg(feature = "lang-query")]
fn js_value_to_query_params(
    val: &JsValue,
    ctx: &mut Context,
) -> Result<super::query::QueryParams, String> {
    let obj = val.as_object().ok_or("outl.query expects an object")?;
    let mut params = super::query::QueryParams::default();
    if let Some(v) = obj
        .get(js_string!("status"), ctx)
        .map_err(|e| e.to_string())?
        .as_string()
    {
        params.status = Some(v.to_std_string_escaped());
    }
    if let Some(v) = obj
        .get(js_string!("tag"), ctx)
        .map_err(|e| e.to_string())?
        .as_string()
    {
        params.tag = Some(v.to_std_string_escaped());
    }
    if let Some(v) = obj
        .get(js_string!("kind"), ctx)
        .map_err(|e| e.to_string())?
        .as_string()
    {
        params.kind = Some(v.to_std_string_escaped());
    }
    if let Some(v) = obj
        .get(js_string!("since"), ctx)
        .map_err(|e| e.to_string())?
        .as_string()
    {
        params.since = Some(v.to_std_string_escaped());
    }
    if let Some(v) = obj
        .get(js_string!("text"), ctx)
        .map_err(|e| e.to_string())?
        .as_string()
    {
        params.text = Some(v.to_std_string_escaped());
    }
    if let Some(v) = obj
        .get(js_string!("limit"), ctx)
        .map_err(|e| e.to_string())?
        .as_number()
    {
        if v.is_finite() && v >= 0.0 {
            params.limit = Some(v as usize);
        }
    }
    let sort_val = obj
        .get(js_string!("sort"), ctx)
        .map_err(|e| e.to_string())?;
    if let Some(s) = sort_val.as_string() {
        let raw = s.to_std_string_escaped();
        for part in raw.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                params.sort.push(trimmed.to_string());
            }
        }
    }
    Ok(params)
}

/// Convert query hits into a JS array of objects.
#[cfg(feature = "lang-query")]
fn hits_to_js_array(hits: &[super::query::QueryHit], ctx: &mut Context) -> Result<JsValue, String> {
    let arr = boa_engine::object::ObjectInitializer::new(ctx).build();
    for (i, hit) in hits.iter().enumerate() {
        let obj = boa_engine::object::ObjectInitializer::new(ctx)
            .property(
                js_string!("handle"),
                js_string!(hit.handle.as_str()),
                boa_engine::property::Attribute::all(),
            )
            .property(
                js_string!("text"),
                js_string!(hit.text.as_str()),
                boa_engine::property::Attribute::all(),
            )
            .property(
                js_string!("page"),
                js_string!(hit.page.as_str()),
                boa_engine::property::Attribute::all(),
            )
            .property(
                js_string!("status"),
                match hit.status.as_deref() {
                    Some(s) => js_string!(s).into(),
                    None => JsValue::null(),
                },
                boa_engine::property::Attribute::all(),
            )
            .build();
        arr.set(i, obj, true, ctx).map_err(|e| e.to_string())?;
    }
    Ok(arr.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        JsRuntime
            .execute(src, &ExecContext::default())
            .unwrap()
            .stdout
    }

    #[test]
    fn arithmetic_last_value_auto_printed() {
        assert_eq!(run("1 + 2"), "3");
    }

    #[test]
    fn console_log_writes_stdout() {
        assert_eq!(run("console.log('hello')"), "hello\n");
    }

    #[test]
    fn template_literals() {
        assert_eq!(run("`x=${2+3}`"), "x=5");
    }

    #[test]
    fn arrow_fn_and_map() {
        assert_eq!(run("[1,2,3].map(n => n * n).join(',')"), "1,4,9");
    }

    #[test]
    fn parse_error_returns_trap() {
        let out = JsRuntime
            .execute("function (", &ExecContext::default())
            .unwrap();
        assert!(matches!(out.exit, ExitStatus::Trap(_)));
        assert!(!out.stderr.is_empty());
    }
}
