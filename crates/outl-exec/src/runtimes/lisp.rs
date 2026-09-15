//! `lisp` runtime — Scheme via [Steel](https://github.com/mattwparas/steel).
//!
//! Steel is a mature, embeddable Scheme dialect in pure Rust. We
//! register `display` / `displayln` / `print` as native functions that
//! funnel into our own buffer, run the source, and (if nothing was
//! printed) auto-display the value of the last expression.
//!
//! Gated behind the `lang-lisp` feature.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use steel::steel_vm::engine::Engine;
use steel::steel_vm::register_fn::RegisterFn;
use steel::SteelVal;

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};
use crate::sandbox::with_timeout;

/// Steel-backed Scheme runtime.
pub struct LispRuntime;

impl Runtime for LispRuntime {
    fn language(&self) -> &'static str {
        "lisp"
    }

    /// Runs on a worker thread, so the caller is released on time and
    /// the VM is not stopped.
    ///
    /// `Engine::with_interrupted` looks like the better answer and is
    /// not: it takes an `Arc<AtomicBool>` that the VM **never reads** —
    /// every occurrence of `interrupted` in the crate is a write.
    ///
    /// Steel does have a real cancellation point, through a different
    /// door: `Engine::get_thread_state_controller()` hands back a
    /// `ThreadStateController` whose `interrupt()` makes the VM stop at
    /// the next instruction (`VmCore::safepoint_or_interrupt`). Moving
    /// to it would upgrade `lisp` to a real abort, like `lua`. Not done
    /// here because this change is already closing two holes, and
    /// swapping the termination mechanism deserves its own test pass.
    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let owned = source.to_string();
        with_timeout(ctx.timeout, move || run_isolated(&owned))
    }
}

fn run_isolated(source: &str) -> Result<ExecOutput, ExecError> {
    let start = Instant::now();

    let sink: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    // `new_sandboxed`, not `new`: `Engine::new` calls
    // `register_builtin_modules(sandbox = false)`, which registers
    // `steel/filesystem`, `steel/process`, `steel/tcp` and `steel/http`
    // — and `ALL_MODULES` requires filesystem, ports and process into
    // the global scope. A ```lisp fence had `command` (a shell),
    // `open-output-file` and `tcp-connect`: the same hole issue #278
    // closed in `lua`, in the runtime next door.
    let mut engine = Engine::new_sandboxed();

    // Override the printing builtins so output lands in our buffer
    // instead of the host's real stdout. Steel still has the
    // originals available under different names if user code asks,
    // but `(display ...)` / `(displayln ...)` / `(print ...)` —
    // the muscle-memory forms — go through us.
    install_printers(&mut engine, sink.clone());
    revoke_host_bindings(&mut engine);

    match engine.run(source.to_string()) {
        Ok(values) => {
            let mut stdout = sink.lock().unwrap().clone();
            if stdout.is_empty() {
                if let Some(last) = values.last() {
                    stdout.push_str(&steel_value_to_string(last));
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
            stdout: sink.lock().unwrap().clone(),
            stderr: format!("{e}"),
            duration: start.elapsed(),
            exit: ExitStatus::Trap("steel-error".into()),
            format: OutputFormat::Text,
        }),
    }
}

/// Host bindings that survive `Engine::new_sandboxed()` and are shadowed
/// by hand.
///
/// **This half is a denylist, and a denylist fails open** — the opposite
/// of what `runtimes::lua::stdlib` does. It is here because Steel offers
/// no allowlist to build from: `sandboxed_prelude()` exists in the crate
/// and nothing calls it, and `new_sandboxed()` still leaves `command`
/// resolvable. Shadowing the known names is strictly better than
/// leaving them, and strictly worse than a real allowlist.
///
/// `lisp_cannot_reach_the_host` pins each name. That test is the only
/// thing standing between this list and a Steel bump that adds a symbol
/// nobody here thought of, which is why closing this properly —
/// upstream allowlist, or dropping the runtime — has its own issue.
const HOST_BINDINGS: &[&str] = &[
    "command",
    "spawn-process",
    "wait",
    "which",
    "open-input-file",
    "open-output-file",
    "delete-file",
    "create-directory!",
    "read-dir",
    "copy-directory-recursively!",
    "tcp-connect",
    "tcp-listen",
    "with-env-var",
];

fn revoke_host_bindings(engine: &mut Engine) {
    for name in HOST_BINDINGS {
        engine.register_value(name, SteelVal::Void);
    }
}

fn install_printers(engine: &mut Engine, sink: Arc<Mutex<String>>) {
    let s1 = sink.clone();
    engine.register_fn("display", move |v: SteelVal| {
        s1.lock().unwrap().push_str(&steel_value_to_string(&v));
    });
    let s2 = sink.clone();
    engine.register_fn("displayln", move |v: SteelVal| {
        let mut s = s2.lock().unwrap();
        s.push_str(&steel_value_to_string(&v));
        s.push('\n');
    });
    let s3 = sink.clone();
    engine.register_fn("print", move |v: SteelVal| {
        s3.lock().unwrap().push_str(&steel_value_to_string(&v));
    });
    let s4 = sink.clone();
    engine.register_fn("println", move |v: SteelVal| {
        let mut s = s4.lock().unwrap();
        s.push_str(&steel_value_to_string(&v));
        s.push('\n');
    });
    let s5 = sink;
    engine.register_fn("newline", move || {
        s5.lock().unwrap().push('\n');
    });
}

/// Render a Steel value the way you'd see it at a REPL.
fn steel_value_to_string(v: &SteelVal) -> String {
    match v {
        SteelVal::StringV(s) => s.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        LispRuntime
            .execute(src, &ExecContext::default())
            .unwrap()
            .stdout
    }

    #[test]
    fn simple_addition() {
        assert_eq!(run("(+ 1 2)"), "3");
    }

    #[test]
    fn nested_arithmetic() {
        assert_eq!(run("(* (+ 1 2) (- 10 4))"), "18");
    }

    #[test]
    fn explicit_display() {
        assert_eq!(run("(display \"hello\")"), "hello");
    }

    #[test]
    fn displayln_appends_newline() {
        assert_eq!(run("(displayln \"hi\")"), "hi\n");
    }

    #[test]
    fn list_operations() {
        // Real Scheme — `map`, lambda all work out of the box because
        // it's Steel under the hood, not our own toy.
        let out = run("(map (lambda (x) (* x x)) (list 1 2 3))");
        assert!(out.contains("1") && out.contains("4") && out.contains("9"));
    }

    #[test]
    fn syntax_error_returns_trap() {
        let out = LispRuntime
            .execute("(+ 1", &ExecContext::default())
            .unwrap();
        assert!(matches!(out.exit, ExitStatus::Trap(_)));
        assert!(!out.stderr.is_empty());
    }
}
