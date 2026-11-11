//! `outl sync` does not mint a write actor.
//!
//! [RFC 0298](../../../docs/rfcs/0298-read-opens-do-not-mint-actors.md) stopped
//! reads from minting, and listed `outl sync` among the writers it did not
//! reach. It is the sharpest one left: an explicit flush is exactly what a
//! script runs on a loop, and on a machine with a GUI or `outl serve` open the
//! device actor is held for the whole session — so every run took the
//! `AlreadyHeld` branch and left a permanent `ops-<ulid>.jsonl` behind.
//!
//! It has nothing to append. `run_sync` reads `wc.root` and `wc.actor` and
//! never commits; the transport buckets incoming ops by `op.actor` through
//! `OpsDirAppendLock`, never the `JsonlStorage::append` path `ActorWriteLock`
//! guards. So the write open bought an identity and used none of it.
//!
//! The actor it passed to the transport was the *minted* one, which is worse
//! than wasteful: the handshake announced a clock entry for an actor holding no
//! ops at all.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use outl_core::lock::ActorWriteLock;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

/// The names that are actor identities, or the locks that gate them.
/// Index sidecars are a boot cache and are not part of this set — the
/// same split `outl-ws/tests/read_only_open.rs` draws, for the same
/// reason: a sidecar write must not be mistakable for a minted actor.
fn actor_names(ops: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Ok(read) = std::fs::read_dir(ops) else {
        return names;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lock = name.starts_with(".lock-");
        let log = name.starts_with("ops-") && name.ends_with(".jsonl");
        if lock || log {
            names.insert(name);
        }
    }
    names
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success(), "outl init must succeed");
    dir
}

/// A config home holding `transport = "file"`, so `outl sync` stands down
/// at `TransportOutcome::Disabled` without binding an endpoint or
/// touching the network.
///
/// It isolates the run from the developer's own `~/.config/outl`, which on
/// a real machine says `iroh` — the workspace is resolved from config, not
/// from the cwd, so a test that skips this reaches for the network and
/// reads whatever transport the developer happens to have set.
fn p2p_off() -> TempDir {
    let home = TempDir::new().expect("config home");
    let dir = home.path().join("outl");
    std::fs::create_dir_all(&dir).expect("config dir");
    std::fs::write(dir.join("config.toml"), "[sync]\ntransport = \"file\"\n")
        .expect("write config");
    home
}

/// The resolved device actor, as the CLI itself reports it. Taken from
/// `workspace info` rather than the device store directly, so the lock
/// below is the one a writing open would actually contend for.
fn device_actor(ws: &Path, config_home: &Path) -> String {
    let out = outl()
        .args(["workspace", "info", "--json", "--workspace"])
        .arg(ws)
        .env("XDG_CONFIG_HOME", config_home)
        .output()
        .expect("workspace info");
    assert!(out.status.success(), "workspace info must succeed");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("workspace info returns JSON");
    v["data"]["actor"]
        .as_str()
        .expect("actor in the info payload")
        .to_string()
}

/// **Do not delete.** The regression is `outl sync` going back to
/// `ws::open`: with the device actor held, that mints, and the name set
/// grows by a lock plus a log on every run.
#[test]
fn sync_does_not_mint_an_actor_while_the_device_lock_is_held() {
    let ws = init_workspace();
    let config_home = p2p_off();
    let ops = ws.path().join("ops");

    let actor = outl_core::id::ActorId(
        device_actor(ws.path(), config_home.path())
            .parse::<ulid::Ulid>()
            .expect("the reported actor is a ULID"),
    );

    // Hold the device actor the way a GUI or `outl serve` holds it for a
    // whole session. A separate process, so the subprocess below takes
    // the `AlreadyHeld` branch if it opens for writing.
    let _held = ActorWriteLock::try_acquire(&ops, actor).expect("hold the device actor");
    let before = actor_names(&ops);

    for run in 1..=3 {
        let out = outl()
            .args(["sync", "--workspace"])
            .arg(ws.path())
            .env("XDG_CONFIG_HOME", config_home.path())
            .output()
            .expect("outl sync");
        // 3 = nothing was flushed, which is the whole point of `transport
        // = "file"`: it stands down rather than failing.
        assert_eq!(
            out.status.code(),
            Some(3),
            "run {run} should stand down, not fail: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            actor_names(&ops),
            before,
            "run {run} minted an actor: before {before:?}, after {:?}",
            actor_names(&ops)
        );
    }
}
