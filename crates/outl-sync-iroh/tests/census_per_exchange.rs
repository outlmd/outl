//! How many times one exchange reads the op-log census, pinned at the
//! call sites.
//!
//! The census is `O(total ops)` — 38 ms and ~40 MB on a 748,684-op
//! workspace — and an exchange used to ask for it **six times** with two
//! full `JsonlStorage::open`s on top, which is the defect root
//! `CLAUDE.md` invariant 15 exists for. The type system carries half the
//! fix: `ops_missing_for` takes `&Census`, so it *cannot* read one.
//!
//! What a type cannot say is how many a **caller** builds. Nothing stops
//! someone writing `ops_missing_for(dir, &Census::read(dir)?, clock)` and
//! putting the cost straight back, so this reads the source, the way
//! `outl-desktop`'s `BlockRow.recursion.test.ts` reads JSX for a prop
//! that must reach a child.
//!
//! **Verified against both directions of the mistake, and they are
//! caught by different things.**
//!
//! *Adding* a read costs CPU, compiles fine, and is what these tests
//! fail on (checked by injecting a second `Census::read` into
//! `engine_serve.rs`: `the_responder_reads_one_census_per_exchange`
//! fails).
//!
//! *Removing* one is the dangerous direction — sharing the initiator's
//! step-1 census with its step-4 selection, which looks like exactly the
//! tidy-up that was right for the responder. It drops ops: the ingest
//! between those steps appended the peer's ops to our log, so the step-1
//! census describes a log that no longer exists. That one **does not
//! compile**, because step 1 holds a `HashMap<ActorId, ActorClock>` and
//! the selection needs a `&Census`. The type is the guard there; these
//! tests cover the half it cannot see, and
//! `the_initiator_reads_two_because_its_ingest_sits_between_them`
//! records *why* the count is two so nobody "fixes" it by changing the
//! types to make the collapse expressible.

use std::path::Path;

/// Non-comment occurrences of `needle` in `path`.
///
/// Line comments are stripped because this file's own explanations name
/// the symbols it counts, and so do the ones in the modules under test.
fn count_in_code(path: &Path, needle: &str) -> usize {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    src.lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//"))
        .map(|line| line.matches(needle).count())
        .sum()
}

fn src(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)
}

/// The responder's clock and its selection come out of **one** census.
/// Nothing touches the op log between them — the initiator's ops are
/// ingested in step 4 — so a second read would re-derive the same answer
/// at `O(total ops)`.
#[test]
fn the_responder_reads_one_census_per_exchange() {
    let path = src("engine_serve.rs");
    assert_eq!(
        count_in_code(&path, "Census::read"),
        1,
        "engine_serve.rs must build exactly one Census per exchange: its clock \
         and its selection are two questions about the same op log, asked with \
         nothing mutating it in between"
    );
    assert_eq!(
        count_in_code(&path, "local_vector_clock"),
        0,
        "the responder derives its clock from the Census it already has \
         (`Census::vector_clock`), never from a second read"
    );
}

/// The initiator reads **two**, and that is not an oversight to clean up.
///
/// Step 1 sends our clock; step 3 ingests the peer's ops, which appends
/// to our log; step 4 selects what the peer still lacks. A census from
/// before step 3 describes a log that no longer exists.
#[test]
fn the_initiator_reads_two_because_its_ingest_sits_between_them() {
    let path = src("engine_sync.rs");
    let census = count_in_code(&path, "Census::read");
    let clock = count_in_code(&path, "local_vector_clock(");

    assert_eq!(
        census + clock,
        2,
        "engine_sync.rs reads the census twice on purpose — once for the clock \
         it advertises, once after the ingest for the selection. Collapsing \
         them drops every op that arrived during the exchange; adding a third \
         is the cost invariant 15 is about"
    );
    assert_eq!(
        census, 1,
        "the post-ingest read is the one that needs the offsets, so it is a \
         Census; the pre-ingest one only needs a clock"
    );
}

/// The selection never reads a census and never opens storage: it ranges
/// over the one it was handed and spends its offsets.
///
/// `oplog_select/tests.rs`'s `a_selection_with_ops_to_send_still_costs_no_open`
/// proves the behaviour; this pins the shape, so the behaviour cannot be
/// restored by a call that happens to be cheap on a nine-op fixture.
#[test]
fn the_selection_neither_reads_a_census_nor_opens_storage() {
    let path = src("oplog_select.rs");
    assert_eq!(
        count_in_code(&path, "Census::read"),
        0,
        "ops_missing_for takes the census its caller already read"
    );
    assert_eq!(
        count_in_code(&path, "JsonlStorage::open"),
        0,
        "a non-empty selection used to open the whole workspace — both index \
         sidecars for every actor, 160 MB resident — to re-derive offsets the \
         census was already holding"
    );
    assert_eq!(
        count_in_code(&path, "read_ops_at_offsets"),
        1,
        "the op bodies are read by seeking to the census's offsets, in one place"
    );
}

/// The ingest's dedup needs the census, reads it **once**, and reads it
/// under the append flock.
///
/// Under the lock is mandatory: taken before it, a census could miss a
/// concurrent writer's op and let this batch append it twice. That is
/// also what makes its cost every other op-log writer's problem on the
/// device, which is a reason to keep it cheap and not to move it.
#[test]
fn the_ingest_reads_one_census_and_keeps_it_under_the_lock() {
    let path = src("oplog.rs");
    let src_text = std::fs::read_to_string(&path).expect("read oplog.rs");

    assert_eq!(
        count_in_code(&path, "Census::read"),
        1,
        "the ingest dedups against one census per batch"
    );

    let lock_at = src_text
        .find("OpsDirAppendLock::acquire")
        .expect("the ingest takes the cross-process append flock");
    let census_at = src_text
        .find("Census::read")
        .expect("the ingest reads a census");
    assert!(
        lock_at < census_at,
        "the census must be read AFTER the flock is held: before it, a \
         concurrent writer's op can land between the read and the append, and \
         the batch appends it a second time"
    );
}
