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

/// Steel-backed Scheme runtime.
pub struct LispRuntime;

impl Runtime for LispRuntime {
    fn language(&self) -> &'static str {
        "lisp"
    }

    fn execute(&self, source: &str, _ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();

        let sink: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let mut engine = Engine::new();

        // Override the printing builtins so output lands in our buffer
        // instead of the host's real stdout. Steel still has the
        // originals available under different names if user code asks,
        // but `(display ...)` / `(displayln ...)` / `(print ...)` —
        // the muscle-memory forms — go through us.
        install_printers(&mut engine, sink.clone());

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
