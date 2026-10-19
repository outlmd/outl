//! A read does not mint a write actor.
//!
//! `resolve_write_actor` answers a held device lock by minting a fresh
//! `ActorId`. That is correct for a process that is about to append, and
//! it is how `outl workspace info` — a poll, every thirty seconds, beside
//! a running `outl serve` — used to leave a permanent actor behind on
//! every tick (issue #298).
//!
//! A read has nothing to append. It must not take the per-actor lock and
//! it must not create a `.lock-<id>` or an `ops-<id>.jsonl`.

use std::collections::BTreeSet;
use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::lock::ActorWriteLock;
use outl_core::op::{LogOp, Op};
use outl_ws::layout::{init, Paths};
use tempfile::TempDir;

fn workspace() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    init(&Paths::at(dir.path().to_path_buf())).expect("init");
    dir
}

/// The names that are actor identities or the locks that gate them.
/// Index sidecars are a boot cache and are not part of this set.
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

/// **Do not delete.** The regression is a read that calls
/// `resolve_write_actor` again: the second open mints, the directory
/// grows, and the assertion on the name set is what catches it.
#[test]
fn a_read_only_open_does_not_mint_an_actor_while_the_device_lock_is_held() {
    let dir = workspace();
    let ops = dir.path().join("ops");

    // The first process owns the device actor, the way `outl serve` or a
    // GUI does for the whole time it is open.
    let holder = outl_ws::open(dir.path()).expect("writer open");
    let device_actor = holder.actor;
    let held = ActorWriteLock::try_acquire(&ops, device_actor);
    // The writer open already holds this actor. A second acquire in the
    // same process fails, which is the lock working. Drop the writer so
    // the test can hold the lock the way a *different* process would,
    // then prove the file set does not grow.
    drop(held);
    drop(holder);
    let _held = ActorWriteLock::try_acquire(&ops, device_actor).expect("re-acquire device actor");
    let before = actor_names(&ops);

    let first = outl_ws::open_read(dir.path()).expect("read open");
    assert_eq!(
        first.actor, device_actor,
        "a read reports the device actor; it does not invent one"
    );
    assert!(!first.ephemeral_actor, "a read is not an ephemeral writer");
    assert_eq!(
        actor_names(&ops),
        before,
        "the first read added an actor identity: before {before:?}, after {:?}",
        actor_names(&ops)
    );

    // A second reader, while the first is still open and the writer
    // still holds the device actor. Readers do not contend with each
    // other and do not contend with the writer.
    let second = outl_ws::open_read(dir.path()).expect("second read open");
    assert_eq!(second.actor, device_actor);
    assert!(!second.ephemeral_actor);
    drop(first);
    drop(second);

    let third = outl_ws::open_read(dir.path()).expect("read after the others closed");
    assert_eq!(third.actor, device_actor);
    drop(third);
    assert_eq!(
        actor_names(&ops),
        before,
        "repeated reads must not accumulate actors"
    );
}

/// The backstop. `open_read` holds no write lock, so an append that got
/// through would land on the device actor's file while another process
/// may be appending to it. The refusal has to happen before any byte is
/// created.
#[test]
fn a_read_only_open_cannot_append_to_the_device_actors_log() {
    let dir = workspace();
    let ops = dir.path().join("ops");
    let mut ctx = outl_ws::open_read(dir.path()).expect("read open");
    let before = actor_names(&ops);
    let ts = ctx.hlc.next();
    let err = ctx.workspace.apply(LogOp {
        ts,
        actor: ctx.actor,
        op: Op::Create {
            node: NodeId::new(),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    });
    assert!(
        err.is_err(),
        "a read-only workspace must refuse to append, got {err:?}"
    );
    let commit = ctx.commit(NodeId::root());
    assert!(
        commit.is_err(),
        "commit on a read must refuse before it mutates, got {commit:?}"
    );
    assert_eq!(
        actor_names(&ops),
        before,
        "the refused append must not create an ops file"
    );
}
