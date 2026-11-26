//! **In what order the two cross-page gates run, and what the disk says
//! about it.**
//!
//! Every cross-page gesture asks two questions before it writes: *is the
//! row's block still live*, and *can this page's `.md` take a write at
//! all* (root `CLAUDE.md` invariant 8). The two clients asked them in
//! opposite orders.
//!
//! The TUI asks liveness **first**, and documents why in place:
//! *"a chord aimed at a block that is gone should not touch the disk at
//! all"* — because `reproject_stale_md` is not a read. On a page whose
//! `.md` is **merely stale** it refreshes the projection, which is a
//! write. The GUI commands ran it first and asked about the row's block
//! only inside the commit closure, so a gesture aimed at a block a peer
//! had already deleted rewrote a file on a page the user is not looking
//! at and *then* refused. Same gesture, same workspace, two outcomes.
//!
//! It also decided the **sentence**: on a page ahead of its log the old
//! order reported the ahead-of-log notice for a row whose block was
//! deleted, where the TUI said the block was gone. The block is the
//! nearer fact and the one the user acted on.
//!
//! **Its own test binary** because it is its own question, like the four
//! files beside it: `backlink_row_ops.rs` owns *what a stale row is told*,
//! `backlink_edit_strict.rs` the `edit_block` split,
//! `backlink_edit_concurrent.rs` the window between render and commit,
//! `backlink_commit_cost.rs` how often a commit walks its page. This one
//! owns the order, for **both** commands — the defect was in each of
//! them, and invariant 8's own lesson is that these come in mirrored
//! pairs.
//!
//! **Everything here asserts the disk, never the message.** In the
//! merely-stale case `reproject_stale_md` *succeeds*, so the refusal is
//! the same `stale_row_block` sentence in both orders — a message test
//! passes in the broken state, which is the trap this file exists to
//! avoid.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::refusal::{
    ahead_of_log_notice, stale_backlink_block_text, stale_backlink_block_unchanged_text,
};
use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, BacklinkIndex, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::{
    backlink_row_op, edit_backlink_block, BacklinkRowOp,
};
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::surface::SURFACE;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Every row op, written out rather than iterated from the type: the
/// guarantee is uniform across the seven, and a variant added without a
/// row here is a gesture whose ordering nothing checks.
const EVERY_OP: &[BacklinkRowOp] = &[
    BacklinkRowOp::ToggleTodo,
    BacklinkRowOp::MarkDone,
    BacklinkRowOp::Delete,
    BacklinkRowOp::Indent,
    BacklinkRowOp::Outdent,
    BacklinkRowOp::MoveUp,
    BacklinkRowOp::MoveDown,
    BacklinkRowOp::Collapse,
    BacklinkRowOp::Expand,
];

/// Minimal [`AppHost`]: no projection writer, so every projection this
/// command performs happens **inline** and is visible on disk the
/// moment the call returns. A writer host would queue it onto a thread
/// and make the assertion a race.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<BacklinkIndex>>>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        Some(self.backlinks.clone())
    }
}

struct Fixture {
    tmp: TempDir,
    host: TestHost,
    page: NodeId,
    /// A live block on the source page, citing `[[target]]`.
    cited: NodeId,
    /// A block a peer deleted — the id a backlink row keeps carrying.
    doomed: NodeId,
}

/// A projected source page with two live blocks plus a trashed third.
///
/// Two live blocks, not one, so "beside the citing block" and "the end
/// of the page" are different slots.
fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let page = open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let cited = append_block(&mut ws, &hlc, Some(page), Some("cites [[target]]")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("tail")).unwrap();
    let doomed = append_block(&mut ws, &hlc, Some(page), Some("cited from somewhere")).unwrap();
    outl_actions::delete(&mut ws, &hlc, doomed).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(None)),
    };
    Fixture {
        tmp,
        host,
        page,
        cited,
        doomed,
    }
}

fn notes_md(f: &Fixture) -> PathBuf {
    f.tmp.path().join("pages").join("notes.md")
}

fn op_count(host: &TestHost) -> usize {
    host.workspace.lock().as_ref().unwrap().log().len()
}

/// What the `.md` looked like, in the two ways a write shows up.
fn disk_state(path: &PathBuf) -> (String, std::time::SystemTime) {
    let md = std::fs::read_to_string(path).expect("the fixture projected the page");
    let mtime = std::fs::metadata(path).unwrap().modified().unwrap();
    (md, mtime)
}

/// Move the tree past the `.md` while leaving the sidecar vouching for
/// the bytes on disk — the **merely stale** state, which is the one
/// `reproject_stale_md` writes in.
///
/// That is what makes the gate order observable: a page ahead of its log
/// is refused and never written, and a page in sync renders identically,
/// so neither tells the two orders apart.
fn put_the_tree_ahead_of_the_page(f: &Fixture) {
    let workspace = f.host.workspace_arc();
    let mut guard = workspace.lock();
    let ws = guard.as_mut().unwrap();
    append_block(
        ws,
        &f.host.hlc,
        Some(f.page),
        Some("written after the projection"),
    )
    .expect("move the tree past the .md");
    drop(guard);

    let (md, _) = disk_state(&notes_md(f));
    assert!(
        !md.contains("written after the projection"),
        "the fixture has to leave the `.md` behind the tree, or nothing \
         re-projects and these tests cannot fail"
    );
}

/// Put the source page in invariant 8's state: a line on disk that
/// exists in no op, with a sidecar hash that agrees with the file.
fn put_the_page_ahead_of_its_log(f: &Fixture) {
    let md_path = notes_md(f);
    let mut md = std::fs::read_to_string(&md_path).expect("the fixture projected the page");
    md.push_str("- only ever on disk\n");
    std::fs::write(&md_path, &md).expect("append a line no op produced");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).expect("the fixture wrote a sidecar");
    sc.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sc).expect("restamp as hash-faithful");
}

// ---------------------------------------------------------------------
// 1. A refused gesture touches no disk — the assertion that matters.
// ---------------------------------------------------------------------

/// **The seven chords.** Each one aimed at a block a peer deleted, on a
/// page whose `.md` is merely stale.
///
/// Before the pre-check this re-projected `notes.md` for all seven and
/// then refused, because `reproject_stale_md` ran first. The sentence
/// was identical, which is why this is on the bytes, the mtime and the
/// op log.
#[test]
fn a_chord_on_a_deleted_row_writes_nothing_even_when_the_page_is_stale() {
    for op in EVERY_OP {
        let f = fixture();
        put_the_tree_ahead_of_the_page(&f);
        let md_path = notes_md(&f);
        let (before_md, before_mtime) = disk_state(&md_path);
        let before_ops = op_count(&f.host);

        let err = backlink_row_op(&f.host, f.page.to_string(), f.doomed.to_string(), *op)
            .expect_err("a block under the trash is not a block to act on");
        assert_eq!(
            err,
            stale_backlink_block_unchanged_text(true),
            "{op:?}: the sentence stays the chord half of the catalog"
        );

        let (after_md, after_mtime) = disk_state(&md_path);
        assert_eq!(
            after_md, before_md,
            "{op:?}: the refused gesture re-projected a page the user is not \
             looking at. The liveness verdict has to come before the \
             stale-`.md` refresh, because that refresh writes."
        );
        assert_eq!(
            after_mtime, before_mtime,
            "{op:?}: the `.md` was rewritten. Identical bytes are still a \
             write on a page nothing asked to touch."
        );
        assert_eq!(
            op_count(&f.host),
            before_ops,
            "{op:?}: a refused gesture must write no op"
        );
    }
}

/// The same question for the tenth gesture, which had the same order.
///
/// `edit_backlink_block` ran `reproject_stale_md` at its first statement
/// and asked about the block only inside the commit closure. Fixing one
/// of the two and leaving the other is exactly the mirrored pair
/// invariant 8 warns about.
#[test]
fn a_text_edit_on_a_deleted_row_writes_nothing_even_when_the_page_is_stale() {
    let f = fixture();
    put_the_tree_ahead_of_the_page(&f);
    let md_path = notes_md(&f);
    let (before_md, before_mtime) = disk_state(&md_path);
    let before_ops = op_count(&f.host);

    let err = edit_backlink_block(
        &f.host,
        f.page.to_string(),
        f.doomed.to_string(),
        "typed into a row whose block is gone".into(),
    )
    .expect_err("a block under the trash cannot take an edit");
    assert_eq!(
        err,
        stale_backlink_block_text(true),
        "a text edit owes the cost half of the catalog, not the chord half"
    );

    let (after_md, after_mtime) = disk_state(&md_path);
    assert_eq!(
        after_md, before_md,
        "the refused edit re-projected the source page"
    );
    assert_eq!(after_mtime, before_mtime, "the `.md` was rewritten");
    assert_eq!(op_count(&f.host), before_ops, "a refusal writes no op");
    assert!(
        !after_md.contains("typed into a row"),
        "the user's text reached a page they are not looking at: {after_md:?}"
    );
}

/// **Which refusal the user reads**, which is the other half of the same
/// reordering.
///
/// On a page ahead of its log, a deleted row used to be told about the
/// page (`outl reconcile --ahead-of-log`) because that gate ran first.
/// The TUI told them about the block. The block is what they acted on,
/// and it is the fact that explains why nothing happened.
#[test]
fn a_deleted_row_is_told_about_the_block_not_about_the_page() {
    for op in [BacklinkRowOp::ToggleTodo, BacklinkRowOp::MarkDone] {
        let f = fixture();
        put_the_page_ahead_of_its_log(&f);

        let err = backlink_row_op(&f.host, f.page.to_string(), f.doomed.to_string(), op)
            .expect_err("a deleted row is refused whatever the page's state");

        assert_eq!(
            err,
            stale_backlink_block_unchanged_text(true),
            "{op:?}: the nearer fact is the block the chord was aimed at"
        );
        let page_notice = ahead_of_log_notice(SURFACE, 1, "- only ever on disk");
        assert!(
            !err.contains(page_notice.command),
            "{op:?}: a user whose block is gone was handed a command for a \
             file condition instead: {err:?}"
        );
    }
}

// ---------------------------------------------------------------------
// 2. The pre-check refuses a dead row and nothing else.
// ---------------------------------------------------------------------

/// A gate asserted only through refusals passes against one that refuses
/// everything — root `CLAUDE.md` invariant 15's shape for a fast path.
///
/// So: a **live** row on a page ahead of its log still has to reach the
/// invariant-8 notice (the pre-check must not shadow the other gate),
/// and a live row on a healthy page still has to perform its gesture.
#[test]
fn the_pre_check_leaves_a_live_row_to_the_other_gates() {
    for op in [BacklinkRowOp::ToggleTodo, BacklinkRowOp::MarkDone] {
        let f = fixture();
        put_the_page_ahead_of_its_log(&f);

        let err = backlink_row_op(&f.host, f.page.to_string(), f.cited.to_string(), op)
            .expect_err("a live row on a frozen page is still refused, by the other gate");

        let expected = ahead_of_log_notice(SURFACE, 1, "- only ever on disk");
        assert!(
            err.contains(expected.title) && err.contains(expected.command),
            "{op:?}: the pre-check swallowed the ahead-of-log refusal: {err:?}"
        );
    }

    for op in EVERY_OP {
        let f = fixture();
        let before_ops = op_count(&f.host);
        backlink_row_op(&f.host, f.page.to_string(), f.cited.to_string(), *op)
            .unwrap_or_else(|e| panic!("{op:?} on a live row must not be refused: {e}"));
        // `Indent` / `Outdent` / `MoveUp` have nowhere to go for the
        // first block of a page and are swallowed as no-ops, so only the
        // refusal is asserted for every op; the op log is asserted where
        // the gesture has somewhere to land.
        if matches!(op, BacklinkRowOp::ToggleTodo | BacklinkRowOp::MarkDone) {
            assert!(
                op_count(&f.host) > before_ops,
                "{op:?} reported success on a live row and wrote nothing"
            );
        }
    }

    let f = fixture();
    let before_ops = op_count(&f.host);
    edit_backlink_block(
        &f.host,
        f.page.to_string(),
        f.cited.to_string(),
        "cites [[target]], edited".into(),
    )
    .expect("a live row takes a text edit");
    assert!(
        op_count(&f.host) > before_ops,
        "the pre-check refused a live text edit"
    );
}

/// **The in-closure verdict is still there, and this is what needs it.**
///
/// The pre-check runs outside the commit's lock, so a peer can delete
/// the block in the window between the two reads. Dropping the second
/// ask because the first one covers the common case would turn that into
/// a write — `create_after` on a trashed anchor builds under the trash,
/// and the moves reorder blocks inside it.
///
/// The peer is not mocked: its change goes through `Workspace::apply`
/// under its own actor, which is where sync ingest ends up.
#[test]
fn a_delete_landing_after_the_pre_check_is_still_refused() {
    let f = fixture();
    let peer = HlcGenerator::new(ActorId::new());

    // The pre-check's read, with the block still live.
    assert!(
        outl_actions::is_live(f.host.workspace.lock().as_ref().unwrap(), f.cited),
        "the fixture's citing block starts live, or this test proves nothing"
    );

    // The peer's delete lands in the window.
    {
        let workspace = f.host.workspace_arc();
        let mut guard = workspace.lock();
        outl_actions::delete(guard.as_mut().unwrap(), &peer, f.cited)
            .expect("the peer deletes the block");
    }
    let before_ops = op_count(&f.host);

    let err = backlink_row_op(
        &f.host,
        f.page.to_string(),
        f.cited.to_string(),
        BacklinkRowOp::Indent,
    )
    .expect_err("the second ask is what catches a delete inside the window");
    assert_eq!(err, stale_backlink_block_unchanged_text(true));
    assert_eq!(
        op_count(&f.host),
        before_ops,
        "an op landed on a block that left the tree: `indent` answers Ok when \
         it finds no previous sibling, so without the in-closure ask this is \
         a silent reparent inside the trash"
    );
}
