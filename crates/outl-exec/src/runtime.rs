//! The `Runtime` trait and its surrounding value types.
//!
//! Every backend (toy Lisp today, wasmtime-hosted interpreters
//! tomorrow) implements [`Runtime`]. The trait is *deliberately tiny* —
//! a single `execute(source, ctx) -> result` — so that we can swap
//! implementations later without dragging UI code along.

use std::path::PathBuf;
use std::time::Duration;

use outl_md::index::WorkspaceIndex;
use thiserror::Error;

/// A language backend.
///
/// The contract is intentionally Unix-y: take a `source` string, return
/// stdout / stderr / exit-status / duration. Errors bubble up through
/// [`ExecError`].
///
/// Implementations **must** honour `ctx.timeout`. If your runtime can't
/// be cancelled cooperatively, wrap the work in
/// [`crate::sandbox::with_timeout`] which spawns the work on a separate
/// thread and drops the channel on overrun.
pub trait Runtime: Send + Sync {
    /// The fence info-string this runtime claims. Matched
    /// case-insensitively against ` ```<lang> `.
    fn language(&self) -> &'static str;

    /// Run `source` and return what happened. Returning `Ok` with a
    /// non-zero [`ExitStatus`] signals a *user-level* error (the script
    /// ran but crashed); returning `Err` signals an infrastructure
    /// error (timeout, OOM, missing toolchain).
    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError>;

    /// Whether blocks using this runtime should auto-run on every
    /// page load **without** requiring the `auto-run::` block
    /// property.
    ///
    /// Auto-run runtimes are also **excluded from manual `gx`
    /// execution** — their results depend on external state (the
    /// workspace, not just the fence body), so a manual re-run
    /// provides no additional value over the automatic one.
    ///
    /// Default: `false`. The `query` runtime returns `true`.
    fn auto_run(&self) -> bool {
        false
    }

    /// Whether this runtime reads [`ExecContext::index`].
    ///
    /// Lets a caller skip building one for a fence that will not look
    /// at it: deriving an index over a large workspace is not free, and
    /// running a `python` block should not pay for a facility only
    /// `query` uses. A runtime that answers `true` still has to cope
    /// with `None` — the caller may have no index to give.
    ///
    /// Default: `false`. The `query` runtime returns `true`.
    fn needs_workspace_index(&self) -> bool {
        false
    }
}

/// Context passed to every execution.
///
/// We deliberately keep this small. Anything runtime-specific (env vars,
/// preopened directories, sandbox tweaks) lives inside the runtime
/// itself.
#[derive(Debug, Clone)]
pub struct ExecContext<'a> {
    /// Workspace root — runtimes that resolve relative file references
    /// (`include "./helper.lisp"`) start here.
    pub workspace_root: PathBuf,
    /// Optional content piped to the script as stdin. Future: chain
    /// blocks via `((ref))`.
    pub stdin: Option<String>,
    /// Hard wall-clock limit. Past this we kill the run.
    pub timeout: Duration,
    /// Optional heap cap. Honoured only by runtimes that can enforce
    /// it (wasmtime can; in-process toy interpreters can't yet).
    pub mem_limit: Option<usize>,
    /// A workspace index the caller already holds, if any.
    ///
    /// The `query` runtime cannot answer without one. Left `None` it
    /// falls back to building an index from [`Self::workspace_root`]
    /// — walkdir + comrak + sidecar JSON over **every** page, on
    /// **every** query block executed. A caller that holds a
    /// `Workspace` should derive the index once
    /// (`outl_actions::index::derive`) and hand it in here instead.
    ///
    /// This crate cannot derive one itself: deriving needs the tree
    /// walk owned by `outl-actions`, which depends on this crate, so
    /// the arrow only points one way. Injection is how the dependency
    /// stays acyclic while the work still happens once.
    ///
    /// Borrowed, not owned: a resident client already holds an index
    /// (the TUI rebuilds one on a background thread), and cloning a
    /// 64k-block index per fence would cost more than the rebuild this
    /// field exists to avoid.
    pub index: Option<&'a WorkspaceIndex>,
}

impl Default for ExecContext<'_> {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            stdin: None,
            timeout: Duration::from_secs(5),
            mem_limit: None,
            index: None,
        }
    }
}

/// What an execution produced.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Wall-clock duration of the call to `execute`.
    pub duration: Duration,
    /// How it ended.
    pub exit: ExitStatus,
    /// How the orchestrator should render this output into the
    /// result subblock. See [`OutputFormat`].
    pub format: OutputFormat,
}

/// How a run terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    /// Normal completion.
    Ok,
    /// Script returned non-zero — user-level error, not an
    /// infrastructure failure.
    NonZero(i32),
    /// Runtime trapped (panic, division by zero, etc). Message is
    /// runtime-specific.
    Trap(String),
}

/// How the orchestrator should render [`ExecOutput`] into a result
/// subblock.
///
/// `Text` (the default) feeds `stdout` through
/// [`render_result_body`](crate::result_block::render_result_body),
/// producing the classic `> **result:** …` single child.
///
/// `Embeds` tells the orchestrator to split `stdout` into one embed
/// reference per line and render each as a child bullet, so the result
/// block becomes a **live view** of the referenced blocks. Used by
/// the `query` runtime: results are `!((blk-XXXXXX))` references, not
/// copies, so toggling a TODO on the original block is reflected
/// everywhere it appears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Single result child with stdout as inline code or fenced block.
    #[default]
    Text,
    /// One child per non-empty stdout line, each rendered as a bullet.
    /// Lines are expected to be embed references (`!((blk-…))`).
    Embeds,
}

/// Infrastructure-level errors.
///
/// User-script errors (a Lisp `(error ...)` form, a Python exception)
/// surface as `Ok(ExecOutput { exit: NonZero | Trap, .. })`. This enum
/// is reserved for "your sandbox didn't even get to run the code".
#[derive(Debug, Error)]
pub enum ExecError {
    /// `ctx.timeout` elapsed before the script finished.
    #[error("execution timed out after {0:?}")]
    Timeout(Duration),
    /// Out of memory — currently only fired by wasmtime-backed runtimes.
    #[error("out of memory")]
    OutOfMemory,
    /// Language-specific parse / compile failure (e.g. malformed Lisp).
    #[error("{0}")]
    Language(String),
    /// Sandbox setup failed (toolchain missing, wasm load error, ...).
    #[error("sandbox: {0}")]
    Sandbox(String),
    /// I/O failure reading source or writing artifacts.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
