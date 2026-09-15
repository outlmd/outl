//! What a code block may reach, and for how long.
//!
//! Two questions with one answer each, pinned here rather than per
//! runtime file so that adding a language cannot quietly answer them
//! differently:
//!
//! - **Host surface** (issue #278) — a fence body is not always written
//!   by the person running it. It arrives over iroh from a paired
//!   device, through `outl import` from someone else's graph, or under
//!   an LLM agent driving `outl_template_run` over MCP. So the set of
//!   host facilities an interpreter exposes is a security boundary, not
//!   a convenience.
//!
//! - **Termination** (issue #279) — execution is in-process and
//!   synchronous. A block that does not finish does not hang "the
//!   block", it hangs the TUI event loop, the desktop (which holds the
//!   workspace mutex while running) and `outl mcp serve`.
//!
//! These are integration tests on purpose. The unit tests inside each
//! `runtimes/*.rs` cover that the language works; these cover what it
//! is *not* allowed to do, which is a property of the crate rather than
//! of any one backend.

use std::time::Duration;

use outl_exec::runtime::{ExecContext, ExecError, ExitStatus, Runtime};

/// Deadline used by the termination tests.
///
/// Long enough that a healthy interpreter finishes its startup inside
/// it, short enough that a hung one does not make the suite unpleasant.
const DEADLINE: Duration = Duration::from_millis(500);

/// Ceiling for how long `execute` may take on a non-terminating
/// program. `DEADLINE` plus generous slack for interpreter teardown —
/// the assertion is "it came back", not "it came back to the
/// millisecond".
const PATIENCE: Duration = Duration::from_secs(5);

fn ctx_with_deadline() -> ExecContext<'static> {
    ExecContext {
        timeout: DEADLINE,
        ..ExecContext::default()
    }
}

/// Run `source` and return captured stdout, failing the test on an
/// infrastructure error.
fn stdout_of(rt: &dyn Runtime, source: &str) -> String {
    rt.execute(source, &ExecContext::default())
        .expect("runtime must not fail at the infrastructure level")
        .stdout
}

// ───────────────────────── host surface (#278) ─────────────────────────

/// The whole point: `os.execute` is a shell, and a fence body can
/// arrive from a peer.
#[cfg(feature = "lang-lua")]
#[test]
fn lua_cannot_reach_the_shell() {
    let out = stdout_of(&outl_exec::runtimes::lua::LuaRuntime, "print(type(os))");
    assert_eq!(
        out.trim(),
        "nil",
        "`os` must not exist inside a lua block — it carries `execute`, \
         and a fence body can arrive over sync or import"
    );
}

/// `io.open` is arbitrary file read and write, outside the workspace.
#[cfg(feature = "lang-lua")]
#[test]
fn lua_cannot_reach_the_filesystem() {
    let out = stdout_of(&outl_exec::runtimes::lua::LuaRuntime, "print(type(io))");
    assert_eq!(
        out.trim(),
        "nil",
        "`io` must not exist inside a lua block — it is arbitrary file access"
    );
}

/// The loaders are how a sandbox gets re-opened from inside: `require`
/// pulls a module off disk, `load` compiles a string built at runtime.
/// Removing `os` while leaving these is a lock with the key next to it.
#[cfg(feature = "lang-lua")]
#[test]
fn lua_cannot_load_more_code() {
    for name in ["require", "dofile", "loadfile", "load", "loadstring"] {
        let out = stdout_of(
            &outl_exec::runtimes::lua::LuaRuntime,
            &format!("print(type({name}))"),
        );
        assert_eq!(
            out.trim(),
            "nil",
            "`{name}` must not exist inside a lua block — a loader lets a \
             block re-open what the sandbox closed"
        );
    }
}

/// Removing the host surface must not cost the language. These are the
/// facilities a code block in a note actually uses.
#[cfg(feature = "lang-lua")]
#[test]
fn lua_keeps_the_language_itself() {
    let rt = &outl_exec::runtimes::lua::LuaRuntime;
    assert_eq!(stdout_of(rt, "print(1 + 2)"), "3\n");
    assert_eq!(stdout_of(rt, "print(('x'):rep(3))"), "xxx\n");
    assert_eq!(stdout_of(rt, "print(math.floor(3.7))"), "3\n");
    assert_eq!(stdout_of(rt, "print(#{1,2,3})"), "3\n");
    assert_eq!(
        stdout_of(rt, "print(table.concat({'a','b'}, '-'))"),
        "a-b\n"
    );
    assert_eq!(stdout_of(rt, "print(tostring(nil))"), "nil\n");
}

/// Same boundary, stated for python. RustPython runs
/// `without_stdlib`, so this is expected to hold already — it is
/// pinned so that enabling a stdlib feature later has to come past a
/// failing test rather than past a reviewer.
#[cfg(feature = "lang-python")]
#[test]
fn python_cannot_reach_the_host() {
    let rt = &outl_exec::runtimes::python::PythonRuntime;
    for probe in ["import os", "import subprocess", "open('/etc/passwd')"] {
        let out = rt
            .execute(probe, &ExecContext::default())
            .expect("infrastructure must not fail");
        assert!(
            matches!(out.exit, ExitStatus::Trap(_) | ExitStatus::NonZero(_)),
            "`{probe}` must not succeed inside a python block, got {:?}",
            out.exit
        );
    }
}

// ───────────────────────── termination (#279) ─────────────────────────

/// `runtime.rs` says implementations **must** honour `ctx.timeout`.
/// This is that sentence, as a test, for every runtime that can loop.
///
/// Asserted per language rather than in a loop so a failure names the
/// backend that hung.
#[cfg(feature = "lang-lua")]
#[test]
fn lua_honours_the_deadline() {
    assert_times_out("lua", "while true do end", || {
        outl_exec::runtimes::lua::LuaRuntime.execute("while true do end", &ctx_with_deadline())
    });
}

#[cfg(feature = "lang-python")]
#[test]
fn python_honours_the_deadline() {
    const SRC: &str = "while True:\n    pass\n";
    assert_times_out("python", SRC, || {
        outl_exec::runtimes::python::PythonRuntime.execute(SRC, &ctx_with_deadline())
    });
}

#[cfg(feature = "lang-lisp")]
#[test]
fn lisp_honours_the_deadline() {
    const SRC: &str = "(define (spin) (spin)) (spin)";
    assert_times_out("lisp", SRC, || {
        outl_exec::runtimes::lisp::LispRuntime.execute(SRC, &ctx_with_deadline())
    });
}

#[cfg(feature = "lang-js")]
#[test]
fn js_honours_the_deadline() {
    const SRC: &str = "while (true) {}";
    assert_times_out("js", SRC, || {
        outl_exec::runtimes::js::JsRuntime.execute(SRC, &ctx_with_deadline())
    });
}

/// A program that finishes well inside the deadline is not disturbed by
/// the mechanism that stops one that does not.
#[cfg(feature = "lang-lua")]
#[test]
fn a_fast_block_is_not_cut_short() {
    let out = outl_exec::runtimes::lua::LuaRuntime
        .execute(
            "local s=0 for i=1,100000 do s=s+i end print(s)",
            &ctx_with_deadline(),
        )
        .expect("a loop that finishes must not be reported as a timeout");
    assert_eq!(out.stdout.trim(), "5000050000");
}

/// Run a non-terminating program under `DEADLINE` and require that
/// `execute` comes back with `ExecError::Timeout`.
///
/// `run` executes on a worker thread and the assertion waits on a
/// channel, because the failure being tested for is *not returning*.
/// Calling `execute` directly here would hang the whole suite on the
/// very defect the test exists to catch — a red test has to fail, not
/// wedge CI. The worker is deliberately not joined: if the runtime
/// ignores the deadline, that thread spins until the process exits,
/// which is exactly the leak this issue is about and is survivable for
/// the length of one test binary.
fn assert_times_out<F>(language: &str, source: &str, run: F)
where
    F: FnOnce() -> Result<outl_exec::runtime::ExecOutput, ExecError> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let started = std::time::Instant::now();
    std::thread::Builder::new()
        .name(format!("deadline-{language}"))
        .spawn(move || {
            let _ = tx.send(run());
        })
        .expect("spawn the worker that runs the block");

    let result = rx.recv_timeout(PATIENCE).unwrap_or_else(|_| {
        panic!(
            "`{language}` did not return within {PATIENCE:?} running `{source}` \
             under a {DEADLINE:?} deadline — ctx.timeout is not honoured, and \
             in a client this is the TUI event loop, the desktop holding the \
             workspace mutex, or `outl mcp serve` frozen with no way out"
        )
    });

    match result {
        Err(ExecError::Timeout(_)) => {}
        Err(other) => panic!(
            "`{language}` returned {other:?} for a non-terminating program; expected Timeout"
        ),
        Ok(out) => panic!(
            "`{language}` returned Ok({:?}) for a non-terminating program — it did \
             not honour ctx.timeout",
            out.exit
        ),
    }

    let elapsed = started.elapsed();
    assert!(
        elapsed < PATIENCE,
        "`{language}` took {elapsed:?} to honour a {DEADLINE:?} deadline"
    );
}

/// A script must not be able to make its own failure look like ours.
///
/// The deadline unwinds through the same channel a failing script does,
/// so whatever distinguishes them has to be something the script cannot
/// produce. A string sentinel is not: `error("<sentinel>")` forged it,
/// and a forged `Timeout` is worse than a wrong label — `Timeout` is an
/// infrastructure error, so the orchestrator writes no result block, and
/// the script suppresses its own failure from the page.
#[cfg(feature = "lang-lua")]
#[test]
fn a_script_cannot_forge_a_timeout() {
    let out = outl_exec::runtimes::lua::LuaRuntime.execute(
        r#"error("outl-exec: deadline exceeded")"#,
        &ExecContext::default(),
    );
    match out {
        Ok(o) => assert!(
            matches!(o.exit, ExitStatus::Trap(_)),
            "a script calling error() must be a Trap, got {:?}",
            o.exit
        ),
        Err(e) => panic!("a failing script is a user-level Trap, not {e:?}"),
    }
}

/// `pcall` must not be able to catch the deadline and keep going.
///
/// The hook signals by returning `mlua::Error`, and `pcall` catches
/// ordinary Lua errors — including that one. So the escape is to put
/// the loop *inside* the `pcall`: the deadline fires, `pcall` swallows
/// it, and the script carries on. It did, returning `ExitStatus::Ok`
/// with whatever it printed next.
///
/// An earlier version of this test used `pcall(function() end)` as the
/// body and passed while the escape was wide open — an empty body never
/// runs long enough for the hook to fire inside it. The body has to be
/// the thing that overruns.
#[cfg(feature = "lang-lua")]
#[test]
fn pcall_cannot_swallow_the_deadline() {
    const SRC: &str = "pcall(function() while true do end end) print('escaped')";
    assert_times_out("lua", SRC, || {
        outl_exec::runtimes::lua::LuaRuntime.execute(SRC, &ctx_with_deadline())
    });
}

/// A coroutine the script creates must inherit the deadline.
///
/// `Lua::set_hook` installs per Lua thread, and mlua's per-thread
/// `hook_proc` resolves the callback through a registry table keyed by
/// the running thread. A coroutine made by `coroutine.create` goes
/// through `lua_newthread` directly, is absent from that table, and
/// mlua's response is to **turn the hook off**. So this ran with no
/// deadline at all — and `lua` is the runtime that does not fall back
/// to `sandbox::with_timeout`, so nothing else would have stopped it.
#[cfg(feature = "lang-lua")]
#[test]
fn a_coroutine_inherits_the_deadline() {
    const SRC: &str = "coroutine.wrap(function() while true do end end)()";
    assert_times_out("lua", SRC, || {
        outl_exec::runtimes::lua::LuaRuntime.execute(SRC, &ctx_with_deadline())
    });
}

/// The same boundary as `lua`, for `lisp`.
///
/// `Engine::new()` registers `steel/filesystem`, `steel/process`,
/// `steel/tcp` and `steel/http`, and requires filesystem, ports and
/// process into the global scope — so a fence had `command` (a shell),
/// `open-output-file` and `tcp-connect`. Issue #278 in the runtime next
/// door, found while reviewing the fix for the first one.
#[cfg(feature = "lang-lisp")]
#[test]
fn lisp_cannot_reach_the_host() {
    for call in [
        r#"(command "/bin/echo" (list "x"))"#,
        r#"(spawn-process "/bin/echo" (list "x"))"#,
        r#"(open-output-file "/tmp/outl-lisp-should-not-exist.txt")"#,
        r#"(open-input-file "/etc/passwd")"#,
        r#"(delete-file "/tmp/outl-lisp-should-not-exist.txt")"#,
        r#"(create-directory! "/tmp/outl-lisp-should-not-exist")"#,
        r#"(tcp-connect "127.0.0.1:9")"#,
    ] {
        let out = outl_exec::runtimes::lisp::LispRuntime
            .execute(call, &ExecContext::default())
            .expect("infrastructure must not fail");
        assert!(
            matches!(out.exit, ExitStatus::Trap(_)),
            "`{call}` must not run inside a lisp block, got {:?} / {:?}",
            out.exit,
            out.stdout
        );
    }
}

/// The revocation has to stop the *effect*, not just the name.
#[cfg(feature = "lang-lisp")]
#[test]
fn a_lisp_block_cannot_write_a_file() {
    let marker = std::env::temp_dir().join("outl-lisp-sandbox-probe.txt");
    let _ = std::fs::remove_file(&marker);

    let src = format!(
        r#"(let ([p (open-output-file "{}")]) (display "pwned" p))"#,
        marker.display()
    );
    let _ = outl_exec::runtimes::lisp::LispRuntime.execute(&src, &ExecContext::default());

    assert!(
        !marker.exists(),
        "a lisp block wrote {} — the host surface is still reachable",
        marker.display()
    );
}

/// Sandboxing lisp must not cost the language.
#[cfg(feature = "lang-lisp")]
#[test]
fn lisp_keeps_the_language_itself() {
    let rt = &outl_exec::runtimes::lisp::LispRuntime;
    assert_eq!(stdout_of(rt, "(displayln (+ 1 2))"), "3\n");
    assert_eq!(
        stdout_of(rt, "(displayln (map (lambda (x) (* x x)) (list 1 2 3)))"),
        "(1 4 9)\n"
    );
    assert_eq!(
        stdout_of(rt, "(displayln (string-append \"a\" \"b\"))"),
        "ab\n"
    );
}

/// A deadline counts VM instructions, so it cannot fire inside one long
/// C call: `string.rep` is a single instruction and an arbitrary number
/// of bytes. `ctx.mem_limit` is the other half of bounding a block, and
/// `lua` is the only runtime that can enforce it.
///
/// No caller sets one today, which is exactly why this is pinned: the
/// field is inert, and a test is what keeps "inert" from drifting into
/// "ignored".
#[cfg(feature = "lang-lua")]
#[test]
fn lua_honours_a_memory_limit() {
    let ctx = ExecContext {
        mem_limit: Some(8 * 1024 * 1024),
        ..ExecContext::default()
    };
    let out = outl_exec::runtimes::lua::LuaRuntime
        .execute("local s = string.rep('x', 200000000) print(#s)", &ctx)
        .expect("a memory-limited run is a user-level failure, not infrastructure");
    assert!(
        matches!(out.exit, ExitStatus::Trap(_)),
        "allocating 200MB under an 8MB cap must trap, got {:?}",
        out.exit
    );
}
