//! `outl serve` embeds the MCP server on a Unix socket, and `outl mcp serve`
//! bridges to it.
//!
//! Every test runs the real binary against a synthetic workspace with its own
//! device directory (`OUTL_DEVICE_DIR`), so the socket, the actor bindings and
//! the config never touch the developer's machine. Sync is switched off or has
//! no paired peers, so nothing here reaches the network. `serve` runs with
//! stdin closed throughout: the embedded server must never read it.
#![cfg(unix)]

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// One isolated machine: a workspace and a device directory.
struct Machine {
    _root: TempDir,
    ws: PathBuf,
    dev: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let root = TempDir::new().expect("tempdir");
        let ws = root.path().join("ws");
        let dev = root.path().join("dev");
        std::fs::create_dir_all(&dev).expect("device dir");
        let me = Self {
            _root: root,
            ws,
            dev,
        };
        let status = me
            .outl()
            .arg("init")
            .arg(&me.ws)
            .status()
            .expect("run init");
        assert!(status.success(), "init failed");
        me
    }

    fn outl(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_outl"));
        c.env("OUTL_DEVICE_DIR", &self.dev)
            .env("HOME", self._root.path())
            .env("XDG_CONFIG_HOME", self._root.path().join("xdg"))
            .arg("--workspace")
            .arg(&self.ws);
        c
    }

    fn log(&self, name: &str) -> PathBuf {
        self._root.path().join(format!("{name}.log"))
    }

    /// Spawn a long-running `outl` with stdin closed and stderr in a file.
    fn spawn(&self, name: &str, args: &[&str]) -> Proc {
        let log = std::fs::File::create(self.log(name)).expect("log file");
        let child = self
            .outl()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn outl");
        Proc {
            child,
            log: self.log(name),
        }
    }

    fn run_dir(&self) -> PathBuf {
        self.dev.join("run")
    }

    fn socket(&self) -> Option<PathBuf> {
        std::fs::read_dir(self.run_dir())
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "sock"))
    }

    /// Wait until a socket exists and accepts a connection.
    fn wait_for_socket(&self) -> PathBuf {
        wait(|| {
            let p = self.socket()?;
            UnixStream::connect(&p).ok().map(|_| p)
        })
    }

    /// File names in `ops/` that name an actor (op logs and lock files).
    fn actor_files(&self) -> BTreeSet<String> {
        std::fs::read_dir(self.ws.join("ops"))
            .expect("ops dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| (n.starts_with("ops-") && n.ends_with(".jsonl")) || n.contains("lock"))
            .collect()
    }
}

struct Proc {
    child: Child,
    log: PathBuf,
}

impl Proc {
    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn wait_for_log(&self, needle: &str) {
        wait(|| self.log_text().contains(needle).then_some(()));
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().expect("try_wait").is_none()
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One MCP session over the socket.
struct Session {
    out: UnixStream,
    inp: BufReader<UnixStream>,
    next: u64,
}

impl Session {
    fn connect(path: &Path) -> Self {
        let out = UnixStream::connect(path).expect("connect");
        out.set_read_timeout(Some(Duration::from_secs(60)))
            .expect("timeout");
        let inp = BufReader::new(out.try_clone().expect("clone"));
        Self { out, inp, next: 1 }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.out, "{req}").expect("write");
        let mut line = String::new();
        self.inp.read_line(&mut line).expect("read reply");
        let v: Value = serde_json::from_str(line.trim()).expect("reply is JSON");
        assert_eq!(v["id"], id);
        v
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        let v = self.call("tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(v["result"]["isError"], false, "{name} failed: {v}");
        v
    }

    fn write_page(&mut self, slug: &str) {
        self.tool("outl_page_create", json!({"slug": slug, "title": slug}));
        self.tool("outl_block_append", json!({"page": slug, "text": "hello"}));
    }
}

fn assert_answers(path: &Path) {
    let mut s = Session::connect(path);
    let init = s.call(
        "initialize",
        json!({"protocolVersion": "2024-11-05", "capabilities": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "outl");
    let tools = s.call("tools/list", json!({}));
    let list = tools["result"]["tools"].as_array().expect("tools array");
    assert!(list.iter().any(|t| t["name"] == "outl_workspace_info"));
}

#[test]
fn default_serve_with_the_watcher_answers_over_the_socket() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-sync"]);
    assert_answers(&b.wait_for_socket());
}

#[test]
fn default_serve_without_the_watcher_answers_over_the_socket() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    assert_answers(&b.wait_for_socket());
}

#[test]
fn no_mcp_creates_no_socket_and_serve_still_runs() {
    let b = Machine::new();
    let mut serve = b.spawn("serve", &["serve", "--no-sync", "--no-mcp"]);
    serve.wait_for_log("watching pages/");
    assert!(b.socket().is_none(), "--no-mcp must not bind a socket");
    assert!(serve.running());
}

#[test]
fn once_never_starts_mcp() {
    let b = Machine::new();
    let status = b.outl().args(["serve", "--once"]).status().expect("run");
    assert!(status.success());
    assert!(b.socket().is_none());
}

#[test]
fn the_socket_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-sync"]);
    let sock = b.wait_for_socket();
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode(&sock), 0o600);
    assert_eq!(mode(&b.run_dir()), 0o700);
}

#[test]
fn concurrent_sessions_write_under_one_actor() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-sync"]);
    let sock = b.wait_for_socket();
    // The first write creates the device actor's own files; the baseline is
    // taken after it.
    Session::connect(&sock).write_page("warm");
    let before = b.actor_files();

    let handles: Vec<_> = (0..4)
        .map(|i| {
            let sock = sock.clone();
            std::thread::spawn(move || {
                let mut s = Session::connect(&sock);
                for n in 0..3 {
                    s.write_page(&format!("page-{i}-{n}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("session thread");
    }

    assert_eq!(
        b.actor_files(),
        before,
        "no new actor log or lock file may appear"
    );
    let mut s = Session::connect(&sock);
    let listed = s.tool("outl_page_list", json!({})).to_string();
    for i in 0..4 {
        for n in 0..3 {
            assert!(listed.contains(&format!("page-{i}-{n}")), "missing page");
        }
    }
}

#[test]
fn concurrent_sessions_write_under_one_actor_without_the_watcher() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    let sock = b.wait_for_socket();
    Session::connect(&sock).write_page("warm");
    let before = b.actor_files();
    let handles: Vec<_> = (0..3)
        .map(|i| {
            let sock = sock.clone();
            std::thread::spawn(move || Session::connect(&sock).write_page(&format!("p{i}")))
        })
        .collect();
    for h in handles {
        h.join().expect("session thread");
    }
    assert_eq!(b.actor_files(), before);
}

/// Create a page through a separate CLI process; returns success.
fn cli_create(b: &Machine, slug: &str) -> bool {
    b.outl()
        .args(["page", "create", slug])
        .output()
        .expect("run outl page create")
        .status
        .success()
}

/// Wait until a CLI writer gets the device actor again: no new actor file.
fn assert_cli_gets_device_actor(b: &Machine, slug: &str) {
    let before = b.actor_files();
    assert!(cli_create(b, slug), "CLI write failed");
    assert_eq!(
        b.actor_files(),
        before,
        "the CLI writer must use the device actor, not mint one"
    );
}

#[test]
fn no_watch_idle_mcp_leaves_the_device_actor_to_a_cli_writer() {
    let b = Machine::new();
    // Create the device actor's own files first so the baseline includes them.
    assert!(cli_create(&b, "seed"));
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    b.wait_for_socket();
    // Let the warm-up finish.
    std::thread::sleep(Duration::from_secs(2));
    assert_cli_gets_device_actor(&b, "while-idle");
}

#[test]
fn no_watch_releases_the_device_actor_when_the_last_session_closes() {
    let b = Machine::new();
    assert!(cli_create(&b, "seed"));
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    let sock = b.wait_for_socket();
    {
        let mut s = Session::connect(&sock);
        s.write_page("from-mcp");
    } // disconnected
    // The server notices the close on its session thread; poll the CLI.
    let before = b.actor_files();
    wait(|| {
        let ok = cli_create(&b, &format!("after-{}", b.actor_files().len()));
        (b.actor_files() == before && ok).then_some(())
    });
}

#[test]
fn no_watch_mutating_tool_errors_while_another_process_holds_the_actor() {
    let b = Machine::new();
    assert!(cli_create(&b, "seed"));
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    let sock = b.wait_for_socket();
    // A long-lived writer in another process: `outl serve` in watch mode takes
    // the device actor's write lock (the socket stays with the first serve).
    let mut holder = b.spawn("holder", &["serve", "--no-sync", "--no-mcp"]);
    holder.wait_for_log("watching pages/");
    assert!(holder.running(), "the holder must keep running");
    let before = b.actor_files();
    let mut s = Session::connect(&sock);
    let v = s.call(
        "tools/call",
        json!({"name": "outl_page_create", "arguments": {"slug": "nope", "title": "x"}}),
    );
    assert_eq!(v["result"]["isError"], true, "{v}");
    assert!(v.to_string().contains("holds the device actor"), "{v}");
    s.tool("outl_page_list", json!({}));
    assert_eq!(b.actor_files(), before, "no ephemeral actor may appear");
    drop(holder);
}

/// Drive `outl mcp serve` over its stdio with one write.
fn stdio_write(b: &Machine, name: &str, extra: &[&str]) -> Proc {
    let log = std::fs::File::create(b.log(name)).expect("log");
    let mut child = b
        .outl()
        .args(["mcp", "serve"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .expect("spawn mcp serve");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let req = |id: u64, name: &str, args: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": name, "arguments": args}})
    };
    for (id, r) in [
        (
            1,
            req(
                1,
                "outl_page_create",
                json!({"slug": "via-stdio", "title": "x"}),
            ),
        ),
        (
            2,
            req(
                2,
                "outl_block_append",
                json!({"page": "via-stdio", "text": "t"}),
            ),
        ),
    ] {
        writeln!(stdin, "{r}").expect("write");
        let mut line = String::new();
        stdout.read_line(&mut line).expect("reply");
        let v: Value = serde_json::from_str(line.trim()).expect("JSON");
        assert_eq!(v["id"], id);
        assert_eq!(v["result"]["isError"], false, "{v}");
    }
    Proc {
        child,
        log: b.log(name),
    }
}

#[test]
fn mcp_serve_bridges_to_a_running_serve() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-sync"]);
    let sock = b.wait_for_socket();
    Session::connect(&sock).write_page("warm");
    let before = b.actor_files();

    let bridge = stdio_write(&b, "bridge", &[]);
    assert!(bridge.log_text().contains("bridging to the running"));
    assert_eq!(b.actor_files(), before, "a bridge mints no actor");
    // The write landed in serve's workspace.
    let listed = Session::connect(&sock)
        .tool("outl_page_list", json!({}))
        .to_string();
    assert!(listed.contains("via-stdio"));
}

#[test]
fn mcp_serve_falls_back_to_standalone_without_a_server() {
    let b = Machine::new();
    let p = stdio_write(&b, "alone", &[]);
    assert!(p.log_text().contains("running standalone"));
}

#[test]
fn standalone_flag_ignores_a_running_serve() {
    let b = Machine::new();
    let _serve = b.spawn("serve", &["serve", "--no-watch"]);
    b.wait_for_socket();
    let p = stdio_write(&b, "forced", &["--standalone"]);
    let log = p.log_text();
    assert!(log.contains("--standalone"), "{log}");
    assert!(!log.contains("bridging"));
}

#[test]
fn a_stale_socket_is_replaced() {
    let b = Machine::new();
    let mut first = b.spawn("first", &["serve", "--no-sync"]);
    let stale = b.wait_for_socket();
    // SIGKILL: the process cannot clean up, so the file stays behind.
    first.child.kill().expect("kill");
    first.child.wait().expect("wait");
    assert!(stale.exists(), "a killed server leaves its socket behind");
    assert!(UnixStream::connect(&stale).is_err());

    let _second = b.spawn("second", &["serve", "--no-sync"]);
    assert_answers(&b.wait_for_socket());
}

#[test]
fn a_second_serve_warns_and_keeps_running() {
    let b = Machine::new();
    let _first = b.spawn("first", &["serve", "--no-sync"]);
    let sock = b.wait_for_socket();
    // `--no-watch`: no write lock, so this one cannot disturb the first's actor.
    let mut second = b.spawn("second", &["serve", "--no-watch"]);
    second.wait_for_log("already serves MCP");
    assert!(second.running(), "serve must not fail over MCP");
    assert_answers(&sock);
}

#[test]
fn sigterm_removes_the_socket() {
    let b = Machine::new();
    let mut serve = b.spawn("serve", &["serve", "--no-sync"]);
    let sock = b.wait_for_socket();
    let pid = serve.child.id().to_string();
    let status = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("kill");
    assert!(status.success());
    let exit = wait(|| serve.child.try_wait().expect("try_wait"));
    assert!(exit.success(), "clean shutdown exits 0");
    assert!(!sock.exists(), "the socket is removed on clean shutdown");
}
