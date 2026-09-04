//! JSON envelope and exit-code conventions shared by every machine-shaped
//! subcommand.
//!
//! See `docs/cli.md` for the contract:
//!
//! ```json
//! { "ok": true,  "data": { ... }, "error": null }
//! { "ok": false, "data": null,    "error": { "code": "X", "message": "..." } }
//! ```
//!
//! Exit codes: `0` success, `1` user error, `2` internal error.

use serde::Serialize;
use serde_json::{json, Value};

/// Suggested exit code for a successful command.
pub const EXIT_OK: i32 = 0;
/// Suggested exit code for a user-visible error (bad input, not found, conflict).
pub const EXIT_USER: i32 = 1;
/// Suggested exit code for an unexpected internal error.
pub const EXIT_INTERNAL: i32 = 2;

/// Stable string codes returned in the `error.code` field. New codes are
/// appended; existing codes are never renumbered.
pub mod codes {
    /// Workspace path does not contain an `.outl/` directory.
    pub const NO_WORKSPACE: &str = "NO_WORKSPACE";
    /// Page lookup by slug failed.
    pub const PAGE_NOT_FOUND: &str = "PAGE_NOT_FOUND";
    /// Block lookup by id failed.
    pub const BLOCK_NOT_FOUND: &str = "BLOCK_NOT_FOUND";
    /// Block id string is not a valid ULID.
    pub const INVALID_BLOCK_ID: &str = "INVALID_BLOCK_ID";
    /// Date string could not be parsed (ISO `YYYY-MM-DD` or natural).
    pub const INVALID_DATE: &str = "INVALID_DATE";
    /// `--confirm` (CLI) / `confirm: true` (MCP) missing on a destructive op.
    pub const CONFIRM_REQUIRED: &str = "CONFIRM_REQUIRED";
    /// `Op::Move` was rejected because it would create a cycle. The op
    /// still goes into the log; the materialized tree is unchanged.
    pub const CYCLE_REJECTED: &str = "CYCLE_REJECTED";
    /// Tried to rename a slug to one that already exists.
    pub const SLUG_CONFLICT: &str = "SLUG_CONFLICT";
    /// Property key not found on the page.
    pub const PROP_NOT_FOUND: &str = "PROP_NOT_FOUND";
    /// Underlying CRDT / storage / filesystem error.
    pub const INTERNAL: &str = "INTERNAL";
    /// Generic user-input validation failure.
    pub const INVALID_ARG: &str = "INVALID_ARG";
    /// A page's `.md` holds content that exists in no op (root
    /// `CLAUDE.md` invariant 8) — the write was refused rather than
    /// silently deleting it. Distinct from `INTERNAL` so a caller
    /// (the MCP's agent in particular) can tell "this page stopped
    /// syncing" from an opaque failure and act on it instead of just
    /// retrying. See RFC 0255.
    pub const PAGE_MARKDOWN_AHEAD_OF_LOG: &str = "PAGE_MARKDOWN_AHEAD_OF_LOG";
}

/// Stable error body used in both CLI (`--json`) and MCP responses.
#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    /// Stable string code; see [`codes`].
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Structured detail beyond `message`, for a caller that wants to
    /// act on the error rather than just display it (e.g.
    /// `PAGE_MARKDOWN_AHEAD_OF_LOG`'s `path` / `lines` / `sample` /
    /// `recovery_command`). Absent from the JSON body entirely for
    /// every other error, so the envelope shape most callers already
    /// parse doesn't change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ApiError {
    /// Build an error with the given code and message.
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            data: None,
        }
    }

    /// Build an error carrying structured detail alongside the message.
    pub fn with_data(code: &str, message: impl Into<String>, data: Value) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            data: Some(data),
        }
    }

    /// Convenience: wrap any `Display` error as an `INTERNAL` ApiError.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::new(codes::INTERNAL, err.to_string())
    }

    /// Map this error to its conventional process exit code.
    pub fn exit_code(&self) -> i32 {
        if self.code == codes::INTERNAL {
            EXIT_INTERNAL
        } else {
            EXIT_USER
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<outl_actions::ActionError> for ApiError {
    fn from(e: outl_actions::ActionError) -> Self {
        use outl_actions::ActionError::*;
        // Captured before the match moves `e` — `Display` only needs
        // `&self`, and this is the *one* variant below that reuses it
        // rather than writing its own `format!`. Reusing it here is
        // deliberate: `ActionError::PageMarkdownAheadOfLog`'s message is
        // already the single Rust-side sentence for this condition
        // (`outl doctor` and this crate's other call sites both let it
        // flow through as-is), and inventing a second phrasing for the
        // MCP's structured response is exactly the drift RFC 0255 exists
        // to prevent.
        let message = e.to_string();
        match e {
            NotInTree(s) => ApiError::new(codes::BLOCK_NOT_FOUND, format!("block {s} not in tree")),
            MissingPosition(s) => ApiError::new(
                codes::INTERNAL,
                format!("block {s} has no position in the tree"),
            ),
            NoPreviousSibling(s) => ApiError::new(
                codes::INVALID_ARG,
                format!("cannot indent {s}: no previous sibling"),
            ),
            AlreadyAtRoot(s) => ApiError::new(
                codes::INVALID_ARG,
                format!("cannot outdent {s}: already at root"),
            ),
            NoGrandparent(s) => ApiError::new(
                codes::INVALID_ARG,
                format!("cannot outdent {s}: parent has no grandparent"),
            ),
            InvalidSlug(s) => ApiError::new(codes::INVALID_ARG, format!("invalid page slug `{s}`")),
            PageMarkdownAheadOfLog {
                path,
                lines,
                sample,
            } => ApiError::with_data(
                codes::PAGE_MARKDOWN_AHEAD_OF_LOG,
                message,
                json!({
                    "path": path,
                    "lines": lines,
                    "sample": sample,
                    "recovery_command": outl_actions::error::AHEAD_OF_LOG_RECOVERY_COMMAND,
                }),
            ),
            other => ApiError::internal(other),
        }
    }
}

/// JSON envelope wrapping a command's payload or error.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope<T: Serialize> {
    /// Whether the command succeeded.
    pub ok: bool,
    /// Payload, present only when `ok` is true.
    pub data: Option<T>,
    /// Error body, present only when `ok` is false.
    pub error: Option<ApiError>,
}

impl<T: Serialize> Envelope<T> {
    /// Build a success envelope.
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
}

impl Envelope<Value> {
    /// Build a failure envelope (data is `null`).
    pub fn failure(error: ApiError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Print a success envelope as JSON to stdout. Convenience for CLI
/// handlers that always emit JSON when `--json` is set.
pub fn print_success_json<T: Serialize>(data: &T) {
    let env = Envelope {
        ok: true,
        data: Some(data),
        error: None::<ApiError>,
    };
    match serde_json::to_string(&env) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("internal: could not serialize success envelope: {e}"),
    }
}

/// Print a failure envelope as JSON to stdout. Exit code is the
/// caller's responsibility — use [`ApiError::exit_code`].
pub fn print_error_json(err: &ApiError) {
    let env = Envelope::<Value>::failure(err.clone());
    match serde_json::to_string(&env) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("internal: could not serialize error envelope: {e}"),
    }
}

/// Emit a result either as JSON (when `as_json` is true) or by running
/// the human formatter. On the JSON path, errors propagate via process
/// exit code matching [`ApiError::exit_code`].
pub fn emit<T, F>(as_json: bool, result: Result<T, ApiError>, human: F) -> i32
where
    T: Serialize,
    F: FnOnce(&T),
{
    match result {
        Ok(data) => {
            if as_json {
                print_success_json(&data);
            } else {
                human(&data);
            }
            EXIT_OK
        }
        Err(err) => {
            if as_json {
                print_error_json(&err);
            } else {
                eprintln!("error ({}): {}", err.code, err.message);
            }
            err.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_success_serializes_data_and_null_error() {
        let env = Envelope::success(json!({"x": 1}));
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(s.contains("\"data\""));
        assert!(s.contains("\"error\":null"));
    }

    #[test]
    fn envelope_failure_serializes_error_and_null_data() {
        let err = ApiError::new(codes::PAGE_NOT_FOUND, "missing");
        let env = Envelope::<Value>::failure(err);
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"ok\":false"));
        assert!(s.contains("\"data\":null"));
        assert!(s.contains("PAGE_NOT_FOUND"));
    }

    #[test]
    fn internal_errors_exit_with_2() {
        let err = ApiError::new(codes::INTERNAL, "boom");
        assert_eq!(err.exit_code(), EXIT_INTERNAL);
    }

    #[test]
    fn user_errors_exit_with_1() {
        let err = ApiError::new(codes::PAGE_NOT_FOUND, "missing");
        assert_eq!(err.exit_code(), EXIT_USER);
    }

    /// RFC 0255 Part 1, test 4: the refusal wording has one owner.
    ///
    /// `ActionError::PageMarkdownAheadOfLog`'s `Display` is already the
    /// single Rust-side sentence for this condition — `outl doctor`
    /// asks the same underlying `content_lines_missing_from` question
    /// and phrases its own listing, but nothing else in this crate
    /// wrote a competing full sentence for the refusal itself before
    /// this conversion existed. Pinning that the `ApiError`'s message
    /// is *exactly* that `Display` — not a paraphrase built fresh in
    /// this `From` impl — is what stops a future edit from quietly
    /// forking it into a second sentence for the same condition.
    ///
    /// A byte-identical sentence between this Rust message and
    /// `@outl/shared/warnings::aheadOfLogNotice` (the GUI clients' TS
    /// copy) isn't pinned here, or anywhere: there is no mechanism in
    /// this repo that shares a literal string across the Rust/MCP and
    /// TypeScript/GUI boundary, the way `outl_shortcuts::support`
    /// shares one Rust enum's text across TUI and desktop. Reusing the
    /// existing Rust sentence verbatim is the honest substitute — it
    /// guarantees no *second* Rust copy exists to drift, which is the
    /// half of "one owner" this codebase can actually enforce today.
    #[test]
    fn ahead_of_log_wording_is_the_actionerrors_display_not_a_second_sentence() {
        let action_err = outl_actions::ActionError::PageMarkdownAheadOfLog {
            path: "pages/frozen.md".to_string(),
            lines: 3,
            sample: "\"only ever on disk\"".to_string(),
        };
        let expected = action_err.to_string();

        let api_err: ApiError = outl_actions::ActionError::PageMarkdownAheadOfLog {
            path: "pages/frozen.md".to_string(),
            lines: 3,
            sample: "\"only ever on disk\"".to_string(),
        }
        .into();

        assert_eq!(api_err.code, codes::PAGE_MARKDOWN_AHEAD_OF_LOG);
        assert_eq!(
            api_err.message, expected,
            "the MCP/CLI message must forward ActionError's own Display verbatim, \
             never a second phrasing of the same refusal",
        );
    }
}
