//! Harness shared by the MCP integration tests.
//!
//! Spawns the real `outl mcp serve` over stdio, so every test that uses
//! it exercises the same path Claude Desktop / Cursor take. Lives in its
//! own module because `mcp_smoke.rs` grew past the file-size ratchet and
//! a second MCP test file (`mcp_trash.rs`) needed the same client —
//! copying it would have been two harnesses drifting apart.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

pub fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success());
    dir
}

pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    pub fn spawn(workspace: &std::path::Path) -> Self {
        let mut child = outl()
            .args(["--workspace"])
            .arg(workspace)
            .args(["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    pub fn call(&mut self, payload: Value) -> Value {
        let line = payload.to_string();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut response = String::new();
        self.stdout.read_line(&mut response).expect("read response");
        serde_json::from_str(response.trim()).expect("response was JSON")
    }
}

/// Parse the data payload out of a **successful** `tools/call` result.
///
/// Success replies are content-only (no `structuredContent` at this
/// protocol version); the data lives as compact JSON in
/// `content[0].text`. Markdown-first tools put raw `.md` there instead,
/// so this is only for the JSON-shaped tools.
pub fn success_data(result: &Value) -> Value {
    assert_eq!(
        result["isError"], false,
        "expected a success reply: {result}"
    );
    let text = result["content"][0]["text"]
        .as_str()
        .expect("content[0].text is a string");
    serde_json::from_str(text).expect("success content is JSON")
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Closing stdin makes the MCP loop exit.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
