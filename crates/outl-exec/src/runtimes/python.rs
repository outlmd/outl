//! `python` runtime — Python via [RustPython](https://rustpython.github.io).
//!
//! RustPython is a pure-Rust Python 3 interpreter. It supports a
//! substantial subset of CPython 3 — enough for arithmetic, list/dict
//! comprehensions, string methods, f-strings, regex, json. It is not
//! CPython: numpy / pandas / native extensions don't run here.
//!
//! Output capture strategy: instead of redirecting `sys.stdout` (which
//! requires the full stdlib), we prepend a tiny Python prelude that
//! shadows the global `print` with a function that appends to a list.
//! After running, we read that list out of the scope and join it.
//! Works without `stdio` / `freeze-stdlib` features, keeping the
//! binary small.
//!
//! Gated behind the `lang-python` feature.

use std::time::Instant;

use rustpython_vm::{compiler, scope::Scope, Interpreter, PyObjectRef, Settings, VirtualMachine};

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

/// RustPython-backed runtime.
pub struct PythonRuntime;

const PRELUDE: &str = r#"
__outl_out = []
def print(*args, sep=' ', end='\n', **_kw):
    __outl_out.append(sep.join(str(a) for a in args) + end)
"#;

impl Runtime for PythonRuntime {
    fn language(&self) -> &'static str {
        "python"
    }

    fn execute(&self, source: &str, _ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();
        let interpreter = Interpreter::without_stdlib(Settings::default());

        let outcome = interpreter.enter(|vm| -> Result<String, PyErrString> {
            let scope = vm.new_scope_with_builtins();

            run_code(vm, &scope, PRELUDE, "<prelude>")?;
            run_code(vm, &scope, source, "<block>")?;

            // Pull `__outl_out` out of the scope and join.
            let key = vm.ctx.new_str("__outl_out");
            let captured: PyObjectRef = scope
                .globals
                .get_item(&*key, vm)
                .map_err(|e| pyerr_string(vm, e))?;

            // It's a list of strings — len + getitem.
            let len = vm
                .call_method(&captured, "__len__", ())
                .and_then(|v| v.try_int(vm).map(|i| i.as_bigint().clone()));
            let len = match len {
                Ok(n) => n.to_string().parse::<usize>().unwrap_or(0),
                Err(_) => 0,
            };

            let mut buf = String::new();
            for i in 0..len {
                let idx: PyObjectRef = vm.ctx.new_int(i).into();
                let item = vm
                    .call_method(&captured, "__getitem__", (idx,))
                    .map_err(|e| pyerr_string(vm, e))?;
                let s = item.str(vm).map_err(|e| pyerr_string(vm, e))?;
                // Surface non-UTF-8 conversion failures instead of
                // silently dropping bytes from stdout. Python 3 strs
                // are notionally UTF-8, but a buggy/native extension
                // could feed in something we can't decode — better to
                // fail loudly than to ship truncated output.
                let chunk = s
                    .to_str()
                    .ok_or_else(|| PyErrString("python stdout has non-UTF-8 bytes".into()))?;
                buf.push_str(chunk);
            }
            Ok(buf)
        });

        match outcome {
            Ok(stdout) => Ok(ExecOutput {
                stdout,
                stderr: String::new(),
                duration: start.elapsed(),
                exit: ExitStatus::Ok,
                format: OutputFormat::Text,
            }),
            Err(stderr) => Ok(ExecOutput {
                stdout: String::new(),
                stderr: stderr.0,
                duration: start.elapsed(),
                exit: ExitStatus::Trap("python-error".into()),
                format: OutputFormat::Text,
            }),
        }
    }
}

struct PyErrString(String);

fn run_code(
    vm: &VirtualMachine,
    scope: &Scope,
    source: &str,
    label: &str,
) -> Result<(), PyErrString> {
    let code = vm
        .compile(source, compiler::Mode::Exec, label.to_string())
        .map_err(|e| PyErrString(format!("{e}")))?;
    vm.run_code_obj(code, scope.clone())
        .map_err(|e| pyerr_string(vm, e))?;
    Ok(())
}

fn pyerr_string(
    vm: &VirtualMachine,
    e: rustpython_vm::PyRef<rustpython_vm::builtins::PyBaseException>,
) -> PyErrString {
    // `write_exception` requires a writer that implements RustPython's
    // own `py_io::Write` — `String` does, `Vec<u8>` doesn't.
    let mut buf = String::new();
    let _ = vm.write_exception(&mut buf, &e);
    PyErrString(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> String {
        PythonRuntime
            .execute(src, &ExecContext::default())
            .unwrap()
            .stdout
    }

    #[test]
    fn print_writes_stdout() {
        assert_eq!(run("print(1 + 2)"), "3\n");
    }

    #[test]
    fn list_comprehension() {
        assert_eq!(run("print([x*x for x in range(4)])"), "[0, 1, 4, 9]\n");
    }

    #[test]
    fn fstrings_and_dicts() {
        let out = run(r#"d={'a':1}; print(f"d={d}")"#);
        assert!(out.contains("d={'a': 1}"));
    }

    #[test]
    fn print_sep_end_kwargs() {
        assert_eq!(run("print('a','b',sep='-',end='!')"), "a-b!");
    }

    #[test]
    fn syntax_error_returns_trap() {
        let out = PythonRuntime
            .execute("def (", &ExecContext::default())
            .unwrap();
        assert!(matches!(out.exit, ExitStatus::Trap(_)));
    }

    #[test]
    fn runtime_exception_returns_trap_with_traceback() {
        let out = PythonRuntime
            .execute("raise ValueError('boom')", &ExecContext::default())
            .unwrap();
        assert!(matches!(out.exit, ExitStatus::Trap(_)));
        assert!(out.stderr.contains("ValueError"));
    }
}
