//! # outl-exec
//!
//! Engine that runs the code inside a fenced markdown block (` ```lisp `,
//! ` ```python `, ...) and writes the result back into the page as a
//! sibling subblock — idempotent on re-run.
//!
//! The crate is intentionally tiny and modular:
//!
//! - [`runtime::Runtime`] — the only trait you implement to add a new
//!   language. Everything else (registry, orchestration, result block
//!   upkeep) treats runtimes as opaque.
//! - [`registry::RuntimeRegistry`] — resolves a fence info-string
//!   (`"lisp"`, `"python"`, ...) to the concrete [`runtime::Runtime`].
//! - [`sandbox`] — cross-platform timeout helper. Runtimes that need
//!   stronger isolation (memory, syscalls) layer it on top — the
//!   wasmtime-based runtimes coming in M2 will, the in-process ones
//!   today don't.
//! - [`result_block`] — pure functions that find / create the result
//!   subblock under a code block. No I/O.
//! - [`orchestrate::run_block_at_index`] — single entry point for every
//!   UI (TUI, future Tauri GUI, future mobile via uniffi). Takes a
//!   workspace + page path + block flat-index, runs, persists,
//!   reconciles.
//!
//! ## Adding a new language in 10 lines
//!
//! ```ignore
//! struct RubyRuntime;
//! impl outl_exec::Runtime for RubyRuntime {
//!     fn language(&self) -> &'static str { "ruby" }
//!     fn execute(&self, source: &str, ctx: &outl_exec::ExecContext<'_>)
//!         -> Result<outl_exec::ExecOutput, outl_exec::ExecError>
//!     {
//!         // run the source however you like (wasm, subprocess, ...);
//!         // populate stdout/stderr/duration/exit and return.
//!         todo!()
//!     }
//! }
//!
//! let mut reg = outl_exec::RuntimeRegistry::default();
//! reg.register(RubyRuntime);
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod language;
pub mod orchestrate;
pub mod registry;
pub mod result_block;
pub mod runtime;
pub mod runtimes;
pub mod sandbox;

#[cfg(feature = "wasm")]
pub mod wasm;

pub use language::{extract_fence, FenceParts};
pub use orchestrate::{
    run_block_at_index, run_block_at_index_if_source_changed, RunError, RunReport,
};
pub use registry::RuntimeRegistry;
pub use result_block::{
    render_result_body, result_source_hash, source_hash, upsert_result_child,
    upsert_result_child_with_hash, upsert_result_embeds, RESULT_MARKER, SOURCE_HASH_KEY,
};
pub use runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

#[cfg(feature = "lang-query")]
pub use runtimes::query::{
    run_query_dsl, run_query_dsl_with_index, run_query_structured, QueryHit, QueryParams,
};
