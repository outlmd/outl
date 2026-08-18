//! `rust` runtime — compile the snippet to `wasm32-wasip1` via
//! `rustc`, cache the resulting `.wasm`, run via wasmtime.
//!
//! There is no in-process Rust interpreter (the language is statically
//! typed and ahead-of-time compiled). The pragmatic path is:
//!
//! 1. Wrap the snippet in `fn main()` if the user didn't.
//! 2. Hash the wrapped source. The hash names the cached `.wasm`.
//! 3. On cache miss, invoke `rustc --target wasm32-wasip1 -O`. On
//!    cache hit, read the bytes back from disk and skip the compile
//!    (Rust compiles are slow; 50–500ms typical).
//! 4. Hand the bytes to [`WasmModule`] and run.
//!
//! Requires the host toolchain to have the `wasm32-wasip1` target:
//!
//! ```text
//! rustup target add wasm32-wasip1
//! ```
//!
//! When the target is missing the runtime returns an
//! [`ExecError::Sandbox`] with the exact `rustup` command the user
//! needs — surfaces as a friendly status-line message.
//!
//! Gated behind the `lang-rust` (which requires `wasm`) feature.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use crate::runtime::{ExecContext, ExecError, ExecOutput, Runtime};
use crate::wasm::{cache_path_for_source, engine::make_engine, WasmModule};

/// Rust → WASM runtime.
///
/// One engine per instance is cheap (sub-ms construction). Each
/// `execute` builds (or reads from cache) a fresh `WasmModule`.
pub struct RustRuntime {
    engine: wasmtime::Engine,
}

impl Default for RustRuntime {
    fn default() -> Self {
        Self {
            engine: make_engine(),
        }
    }
}

impl RustRuntime {
    /// Construct with the shared engine. The engine is cloned, so
    /// callers can build many runtimes from one engine.
    pub fn new(engine: wasmtime::Engine) -> Self {
        Self { engine }
    }
}

impl Runtime for RustRuntime {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();
        let wrapped = wrap_in_main(source);

        // Try the cache first.
        let cache_target = cache_path_for_source("rust", &wrapped);
        let wasm_bytes = match &cache_target {
            Some(path)
                if path.exists()
                    && std::fs::metadata(path)
                        .map(|m| m.len() > 0)
                        .unwrap_or(false) =>
            {
                std::fs::read(path).map_err(ExecError::Io)?
            }
            Some(path) => {
                let bytes = compile_rust_to_wasm(&wrapped, ctx)?;
                // Best-effort write — if the cache dir vanished mid-run
                // we still got the bytes, no need to fail the user's
                // execution over it.
                let _ = std::fs::write(path, &bytes);
                bytes
            }
            None => compile_rust_to_wasm(&wrapped, ctx)?,
        };

        // Hand off to the generic WASM adapter. The source we pipe in
        // doesn't matter — the compiled program already has its logic;
        // we keep stdin empty so `std::io::stdin()` returns EOF
        // immediately.
        let module = WasmModule::from_bytes("rust", &self.engine, &wasm_bytes)?;
        let mut out = module.execute("", ctx)?;
        out.duration = start.elapsed();
        Ok(out)
    }
}

/// If the snippet doesn't already declare `fn main`, wrap it so it
/// becomes a valid program. Lets users write `(+ 1 2)`-style one-liners
/// in Rust too: `println!("{}", 1 + 2);` works on its own.
fn wrap_in_main(source: &str) -> String {
    if source.contains("fn main") {
        source.to_string()
    } else {
        format!("fn main() {{\n{source}\n}}\n")
    }
}

/// Invoke `rustc` to produce a `wasm32-wasip1` binary from `source`.
///
/// Uses a temp directory under the runtime cache so the .rs file is
/// reachable for diagnostics ("`error[E0425]: ... at /var/.../snippet.rs`")
/// and gets cleaned up by the OS in due course.
fn compile_rust_to_wasm(source: &str, ctx: &ExecContext<'_>) -> Result<Vec<u8>, ExecError> {
    let tmp_root = std::env::temp_dir().join("outl-rustc");
    std::fs::create_dir_all(&tmp_root).map_err(ExecError::Io)?;
    let src_path = tmp_root.join(format!("snippet-{}.rs", std::process::id()));
    std::fs::write(&src_path, source).map_err(ExecError::Io)?;

    let wasm_out = src_path.with_extension("wasm");
    let output = Command::new("rustc")
        .arg("--target")
        .arg("wasm32-wasip1")
        .arg("-O")
        .arg("-o")
        .arg(&wasm_out)
        .arg(&src_path)
        .current_dir(&ctx.workspace_root)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ExecError::Sandbox(
                    "`rustc` not found on PATH. Install via `rustup` and \
                     add the wasm32-wasip1 target: `rustup target add wasm32-wasip1`."
                        .into(),
                )
            } else {
                ExecError::Sandbox(format!("spawn rustc: {e}"))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // Common case: target not installed.
        if stderr.contains("the `wasm32-wasip1` target") || stderr.contains("toolchain") {
            return Err(ExecError::Sandbox(format!(
                "{stderr}\n\nhint: `rustup target add wasm32-wasip1`"
            )));
        }
        // Treat compile errors as language errors so the result subblock
        // shows them inline instead of crashing the run.
        return Err(ExecError::Language(stderr));
    }

    let bytes = std::fs::read(&wasm_out).map_err(ExecError::Io)?;
    // Clean up the .rs / .wasm scratch files. Failures are silent.
    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&wasm_out);
    Ok(bytes)
}

/// Re-export helpers for hosts that want to pre-warm the cache (e.g.
/// CI). Not part of the public surface of the trait.
pub fn cache_dir_for_rust() -> Option<PathBuf> {
    let mut p = crate::wasm::cache_dir()?;
    p.push("rust");
    std::fs::create_dir_all(&p).ok()?;
    Some(p)
}
