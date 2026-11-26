//! The nine cross-page gestures that are not a text edit, and the one
//! thing they all have to get right: a stale row id is refused, in the
//! catalog's words, with nothing written.
//!
//! `tests/backlink_edit_strict.rs` owns the tenth gesture (the text
//! edit) and the `edit_block` / `edit_backlink_block` split. This file
//! is its sibling because the nine shipped on the *other* side of that
//! split: they routed through `commands/block.rs`, where every body may
//! be tolerant of a stale id because it acts on the page the user is
//! looking at.
//!
//! Two consequences reached the user, and both are pinned below.
//!
//! **`o` / `O` appended to the end of the other page.** `create_block`
//! resolves its anchor through `create_after_or_append`, whose append
//! tolerance is documented for the open page — so a backlink row with a
//! transient id created an empty block at the end of a page the user
//! was not looking at, and the toast then said "Added an empty block
//! after this reference". That is the exact recovery the command split
//! exists to refuse, reached through a different door.
//!
//! **The other eight printed a ULID.** `ActionError::NotInTree`'s
//! `Display` is `"block {0} is not in the tree"`, and
//! `outl_actions::refusal::stale_backlink_block_unchanged_refusal` —
//! written for precisely this event, with a test of its own asserting
//! its sentences never leak `"tree"`, `"NodeId"`, `"ULID"` or `"01"` —
//! was called only by the TUI. Root `CLAUDE.md` invariant 12: the nudge
//! wording belongs in the catalog, never in a client.
//!
//! **Three of the nine did not even refuse.** `move_up` / `move_down`
//! answer `Ok(())` when they find no sibling, and a node the tree never
//! held has no parent to look under; `indent` is the same shape through
//! `NoPreviousSibling`, which `block.rs` swallows on purpose. So a
//! stale id was a silent success. That is why the liveness gate belongs
//! to `backlink_row_op` rather than to each action's own error, and why
//! the loops below assert the **op count** as well as the sentence.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::refusal::ahead_of_log_notice;
use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, BacklinkIndex, PageKind,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::{
    backlink_row_op, create_backlink_sibling, BacklinkRowOp, BacklinkSide,
};
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::surface::SURFACE;
use parking_lot::Mutex;
use tempfile::TempDir;

/// Every `BacklinkRowOp`, written out rather than iterated from the
/// type: a variant added without a row here is a gesture whose refusal
/// nothing checks, and the `match` in `backlink_row_op` is what forces
/// the author to this file.
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

const EVERY_SIDE: &[BacklinkSide] = &[BacklinkSide::Below, BacklinkSide::Above];

/// Minimal [`AppHost`]: a workspace, an HLC, a storage root, and the
/// cached backlink-index slot a GUI client wires. No projection writer,
/// so the `.md` is written inline — the synchronous path.
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

/// A projected page holding two live blocks, plus the id of a third
/// that is **in the trash** — the state a peer's delete leaves behind
/// while a backlink row still carries the id it rendered.
///
/// Two live blocks, not one: "beside the citing block" and "at the end
/// of the page" are the same slot on a one-block page, so a single
/// block would make the append this file exists to refuse invisible
/// even on the live path.
fn host_with_a_trashed_block() -> (TempDir, TestHost, NodeId, NodeId, NodeId) {
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
    (tmp, host, page, cited, doomed)
}

/// Every block's text on `page`, in outline order, straight from the
/// tree.
///
/// `unwrap_or_default`, **never** `filter_map`: the block `o` appends is
/// *empty*, and an empty block carries no Yrs text, so `block_text`
/// answers `None` for it. A census that skipped those would have let
/// the append this file refuses land without changing its own answer —
/// the shape of a gate that cannot see the thing it guards.
fn page_texts(state: &TestHost, page: NodeId) -> Vec<String> {
    let workspace = state.workspace_arc();
    let guard = workspace.lock();
    let ws = guard.as_ref().unwrap();
    outl_actions::children_of(ws, page)
        .into_iter()
        .map(|(id, _)| ws.block_text(id).unwrap_or_default())
        .collect()
}

fn op_count(state: &TestHost) -> usize {
    let workspace = state.workspace_arc();
    let ops = workspace.lock().as_ref().unwrap().log().len();
    ops
}

/// Review focus 1 — `o` / `O`.
///
/// Both halves are the guarantee, and the second is the one that cannot
/// be dropped: an error alone would be satisfied by a command that
/// errored *and* appended, which is what `create_after_or_append`
/// returns on a trashed page and what the previous wiring did on a
/// trashed block (it appended and answered `Ok`).
#[test]
fn creating_a_sibling_beside_a_stale_row_is_refused_rather_than_appended() {
    let (_tmp, host, page, _cited, doomed) = host_with_a_trashed_block();
    let before_texts = page_texts(&host, page);
    let before_ops = op_count(&host);

    for side in EVERY_SIDE {
        let err = create_backlink_sibling(&host, page.to_string(), doomed.to_string(), *side)
            .expect_err(
                "a stale backlink id must not be re-targeted — the append lands at the \
                 end of a page the user is not looking at",
            );
        assert_eq!(
            err, "That block was deleted, so nothing changed.",
            "{side:?}: the chord half of the catalog, because nothing was typed"
        );
    }

    assert_eq!(
        page_texts(&host, page),
        before_texts,
        "the source page must gain no block: the whole point of refusing"
    );
    assert_eq!(
        op_count(&host),
        before_ops,
        "and no op may be written — held down, `o` collected one junk block per repeat"
    );
}

/// Review focus 2 — the eight chords.
///
/// One sentence for every gesture, and it is the catalog's. Asserted
/// per op rather than once, because the defect was per op: each routed
/// through its own `block.rs` command, which knows nothing about having
/// been called from a backlink row.
#[test]
fn every_chord_on_a_deleted_row_refuses_in_the_catalogs_words() {
    let (_tmp, host, page, _cited, doomed) = host_with_a_trashed_block();
    let before_texts = page_texts(&host, page);
    let before_ops = op_count(&host);

    for op in EVERY_OP {
        let err = backlink_row_op(&host, page.to_string(), doomed.to_string(), *op)
            .expect_err("a block under the trash is not a block to act on");
        assert_eq!(
            err,
            outl_actions::refusal::stale_backlink_block_unchanged_text(true),
            "{op:?} must print the catalog's sentence, not its own"
        );
        assert!(
            !err.contains(&doomed.to_string()),
            "{op:?} leaked a raw id: {err:?}"
        );
        assert!(
            !err.contains("tree"),
            "{op:?} leaked an implementation noun: {err:?}"
        );
    }

    assert_eq!(
        page_texts(&host, page),
        before_texts,
        "a refused chord changes nothing on the source page"
    );
    assert_eq!(
        op_count(&host),
        before_ops,
        "`move_up` / `move_down` / `indent` used to answer Ok on a stale id, so \
         the op count is the half that catches a silent success"
    );
}

/// The other reason a row's block is gone, and the one "deleted" would
/// misdescribe: the id was transient and never named a node (sidecar
/// absent, short or misaligned, which is the ordinary state of an
/// imported graph). Nothing was deleted, so the sentence must not say
/// so.
#[test]
fn a_transient_row_id_is_not_called_deleted() {
    let (_tmp, host, page, _cited, _doomed) = host_with_a_trashed_block();
    let before_ops = op_count(&host);
    let transient = NodeId::new();

    for op in EVERY_OP {
        let err = backlink_row_op(&host, page.to_string(), transient.to_string(), *op)
            .expect_err("a transient id names no block to act on");
        assert_eq!(
            err,
            outl_actions::refusal::stale_backlink_block_unchanged_text(false),
            "{op:?}: a confidently wrong cause is worse than a vaguer true one"
        );
    }
    for side in EVERY_SIDE {
        let err = create_backlink_sibling(&host, page.to_string(), transient.to_string(), *side)
            .expect_err("a transient id names no block to create beside");
        assert_eq!(
            err,
            outl_actions::refusal::stale_backlink_block_unchanged_text(false),
            "{side:?}"
        );
    }

    assert_eq!(op_count(&host), before_ops, "and nothing is written");
}

/// The cost half of the catalog, which is why there are two pairs of
/// sentences and not one.
///
/// A chord typed nothing, so claiming "your text could not be saved" on
/// `Ctrl+T` names a loss that did not happen. That claim is
/// `edit_backlink_block`'s alone (`backlink_edit_strict.rs` pins it),
/// and this is the net that stops a later "unification" from handing
/// the nine gestures the text sentence because it was the one already
/// wired.
#[test]
fn no_chord_refusal_claims_the_users_text_was_lost() {
    let (_tmp, host, page, _cited, doomed) = host_with_a_trashed_block();

    for op in EVERY_OP {
        let err = backlink_row_op(&host, page.to_string(), doomed.to_string(), *op).unwrap_err();
        assert!(!err.contains("text"), "{op:?}: {err:?}");
        assert!(err.ends_with("nothing changed."), "{op:?}: {err:?}");
    }
    for side in EVERY_SIDE {
        let err = create_backlink_sibling(&host, page.to_string(), doomed.to_string(), *side)
            .unwrap_err();
        assert!(!err.contains("text"), "{side:?}: {err:?}");
    }
}

/// "Does it still do the right thing": the gate refuses a dead row and
/// nothing else.
///
/// A refusal-only suite passes against a command that refuses
/// everything, which is the shape invariant 15 names for a fast path —
/// so each op is also driven against a **live** row, on a page where
/// "beside the citing block" and "the end of the page" are different
/// slots.
#[test]
fn a_live_row_still_performs_every_gesture() {
    let (_tmp, host, page, cited, _doomed) = host_with_a_trashed_block();

    let view = backlink_row_op(
        &host,
        page.to_string(),
        cited.to_string(),
        BacklinkRowOp::ToggleTodo,
    )
    .expect("a live row takes its chord");
    assert!(
        view.outline.iter().any(|n| n.todo.is_some()),
        "the task marker must have landed on the source page: {:?}",
        view.outline.iter().map(|n| &n.text).collect::<Vec<_>>()
    );

    let reply = create_backlink_sibling(
        &host,
        page.to_string(),
        cited.to_string(),
        BacklinkSide::Below,
    )
    .expect("a live row takes a sibling");
    let texts = page_texts(&host, page);
    assert_eq!(
        texts,
        vec![
            "TODO cites [[target]]".to_string(),
            String::new(),
            "tail".to_string()
        ],
        "the new block lands immediately after the citing block, not at the page end"
    );
    assert!(
        !reply.new_id.is_empty(),
        "the reply carries the new block's id so a self-citing page can focus it"
    );
}

/// Parity with `block.rs`: a block with nowhere to indent into is a
/// no-op, not a refusal.
///
/// `indent_block` swallows `NoPreviousSibling` and `outdent_block`
/// swallows `AlreadyAtRoot` on purpose — a first child has no previous
/// sibling and a page's own child is already at its root, and neither
/// is news. Dropping the swallow here would make `Tab` on the first row
/// of a page print a refusal the outline never prints for the same
/// block.
#[test]
fn a_gesture_with_nowhere_to_go_is_a_no_op_not_a_refusal() {
    let (_tmp, host, page, cited, _doomed) = host_with_a_trashed_block();
    let before_texts = page_texts(&host, page);

    for op in [BacklinkRowOp::Indent, BacklinkRowOp::Outdent] {
        backlink_row_op(&host, page.to_string(), cited.to_string(), op)
            .unwrap_or_else(|e| panic!("{op:?} on the first block of a page must be Ok: {e}"));
    }
    // `move_up` on a first child and `move_down` on a last one are the
    // same shape, answered inside `outl-actions` rather than swallowed
    // here.
    backlink_row_op(
        &host,
        page.to_string(),
        cited.to_string(),
        BacklinkRowOp::MoveUp,
    )
    .expect("moving the first block up is a no-op");

    assert_eq!(
        page_texts(&host, page),
        before_texts,
        "a no-op must leave the page as it was"
    );
}

/// The `.md` of the fixture's page, and the one unlogged line put into
/// it.
fn notes_md(tmp: &TempDir) -> PathBuf {
    tmp.path().join("pages").join("notes.md")
}

const ONLY_ON_DISK: &str = "- only ever on disk\n";

/// Put the source page in the state root `CLAUDE.md` invariant 8 exists
/// for: one line on disk that exists in no op, and a sidecar whose hash
/// agrees with the file, so nothing but
/// `outl_md::unlogged::sidecar_can_vouch_for` can tell this from a
/// merely stale projection.
fn put_the_page_ahead_of_its_log(tmp: &TempDir) -> PathBuf {
    let md_path = notes_md(tmp);
    let mut md = std::fs::read_to_string(&md_path).expect("the fixture projected the page");
    md.push_str(ONLY_ON_DISK);
    std::fs::write(&md_path, &md).expect("append a line no op produced");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).expect("the fixture wrote a sidecar");
    sc.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sc).expect("restamp as hash-faithful");
    md_path
}

/// **The two ops the "a structural op names an identity" reason does
/// not cover.**
///
/// `toggle_todo` reads `workspace.block_text`, runs `cycle_todo` on it
/// and writes the result back; `mark_done` does the same through
/// `set_todo` + `edit_text`. So both compute their op from the
/// **logged** text while the row the user tapped was rendered from the
/// `.md` — the property the text edit has a gate for. Left ungated, the
/// op lands on stale text, the commit's projection is withheld, the
/// frontend refetches from disk and the row comes back unchanged: the
/// tap looks like it did nothing while the state walks on invisibly.
///
/// Asserted on the op log and the page, not on the message: a wording
/// assertion alone passes against a command that refuses *after*
/// writing.
#[test]
fn a_text_rewriting_chord_on_a_page_ahead_of_its_log_is_refused_before_it_writes() {
    for op in [BacklinkRowOp::ToggleTodo, BacklinkRowOp::MarkDone] {
        let (tmp, host, page, cited, _doomed) = host_with_a_trashed_block();
        let md_path = put_the_page_ahead_of_its_log(&tmp);
        let before_texts = page_texts(&host, page);
        let before_ops = op_count(&host);
        let before_md = std::fs::read_to_string(&md_path).unwrap();

        let err = backlink_row_op(&host, page.to_string(), cited.to_string(), op)
            .expect_err("a page that stopped syncing cannot take a text read-modify-write");

        let expected = ahead_of_log_notice(SURFACE, 1, "- only ever on disk");
        assert!(
            err.contains(expected.title) && err.contains(expected.command),
            "{op:?}: every word is the catalog's, got {err:?}"
        );
        assert_eq!(
            op_count(&host),
            before_ops,
            "{op:?}: an op computed from text the row never showed was written"
        );
        assert_eq!(
            page_texts(&host, page),
            before_texts,
            "{op:?}: the source page changed in the log and not on screen"
        );
        assert_eq!(
            std::fs::read_to_string(&md_path).unwrap(),
            before_md,
            "{op:?}: the bytes the guard protects were touched"
        );
    }
}

/// **The five the criterion leaves alone, and the reason is the op,
/// not the word "structural".**
///
/// Each is an `Op::Move` naming a node: it reads no text, so a stale
/// local `.md` cannot change what it does, and it converges through the
/// op log whatever that file says. Withholding it would refuse a
/// gesture over a condition it cannot be affected by and that it did
/// not cause (invariant 11).
///
/// `lands_an_op` is false for the two with nowhere to go on this
/// fixture (`tail` is already at its page's root and is the last
/// sibling), which the command swallows as a no-op rather than a
/// refusal — so the positive half is asserted only where it applies.
#[test]
fn a_structural_chord_on_a_page_ahead_of_its_log_still_lands_its_op() {
    let cases: &[(BacklinkRowOp, bool, bool)] = &[
        (BacklinkRowOp::Delete, false, true),
        (BacklinkRowOp::Indent, true, true),
        (BacklinkRowOp::MoveUp, true, true),
        (BacklinkRowOp::Outdent, true, false),
        (BacklinkRowOp::MoveDown, true, false),
        // `Op::SetCollapsed` names a node too, and always lands its op.
        (BacklinkRowOp::Collapse, false, true),
        (BacklinkRowOp::Expand, false, true),
    ];
    for (op, aim_at_tail, lands_an_op) in cases {
        let (tmp, host, page, cited, _doomed) = host_with_a_trashed_block();
        let md_path = put_the_page_ahead_of_its_log(&tmp);
        let target = if *aim_at_tail {
            let workspace = host.workspace_arc();
            let guard = workspace.lock();
            outl_actions::children_of(guard.as_ref().unwrap(), page)[1].0
        } else {
            cited
        };
        let before_ops = op_count(&host);

        backlink_row_op(&host, page.to_string(), target.to_string(), *op).unwrap_or_else(|e| {
            panic!("{op:?} names a node, so a frozen `.md` cannot refuse it: {e}")
        });

        if *lands_an_op {
            assert!(
                op_count(&host) > before_ops,
                "{op:?} reported success and wrote nothing"
            );
        }
        assert!(
            std::fs::read_to_string(&md_path)
                .unwrap()
                .contains("only ever on disk"),
            "{op:?}: the projection was not withheld, so the write that deletes ran"
        );
    }
}

/// **Invariant 8's mirror, which is the half nobody reports.**
///
/// The gate inherits `sidecar_can_vouch_for` through
/// `reproject_stale_md` rather than asking `sidecar_can_answer`, so a
/// page with nothing to lose — no sidecar here, which is a peer-shipped
/// `.md` or an import before its first reconcile — still takes the
/// chord. Gating on the narrower question froze 233 pages once
/// (issue #332).
#[test]
fn a_text_rewriting_chord_still_commits_on_a_page_with_nothing_to_lose() {
    let (tmp, host, page, cited, _doomed) = host_with_a_trashed_block();
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&notes_md(&tmp)))
        .expect("drop the sidecar");
    let before_ops = op_count(&host);

    backlink_row_op(
        &host,
        page.to_string(),
        cited.to_string(),
        BacklinkRowOp::ToggleTodo,
    )
    .expect("a page with nothing to lose is not frozen");

    assert!(
        op_count(&host) > before_ops,
        "the gate refused a page that has nothing to lose"
    );
}
