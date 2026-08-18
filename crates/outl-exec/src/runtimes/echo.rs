//! `echo` runtime — echoes the block's source back on stdout.
//!
//! Purpose: smoke-test the pipeline (UI keybind → orchestrate → result
//! subblock) without needing a real interpreter. Also useful in
//! integration tests as a fast, deterministic stand-in.
//!
//! Yes, it would let users run their text as-is. But the fence info
//! string would have to be `echo` deliberately — nobody types
//! ` ```echo ` by accident.

use std::time::Instant;

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

/// See module docs.
pub struct EchoRuntime;

impl Runtime for EchoRuntime {
    fn language(&self) -> &'static str {
        "echo"
    }

    fn execute(&self, source: &str, _ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();
        Ok(ExecOutput {
            stdout: source.to_string(),
            stderr: String::new(),
            duration: start.elapsed(),
            exit: ExitStatus::Ok,
            format: OutputFormat::Text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echoes_single_line() {
        let out = EchoRuntime
            .execute("hello", &ExecContext::default())
            .unwrap();
        assert_eq!(out.stdout, "hello");
        assert!(matches!(out.exit, ExitStatus::Ok));
    }

    #[test]
    fn echoes_multi_line() {
        let out = EchoRuntime
            .execute("one\ntwo\nthree", &ExecContext::default())
            .unwrap();
        assert_eq!(out.stdout, "one\ntwo\nthree");
    }
}
