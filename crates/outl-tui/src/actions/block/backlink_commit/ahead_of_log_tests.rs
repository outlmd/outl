//! A cross-page gesture aimed at a source page whose `.md` holds lines
//! the op log never recorded.
//!
//! Its own module rather than arms in `tests.rs`, which sits one edit
//! away from the 600-line ratchet — and the question is its own: every
//! test there asks *what a cross-page commit does*, these ask *whether
//! it happens at all*.
//!
//! The state under test is root `CLAUDE.md` invariant 8's: those lines
//! exist in no op, so a projection of the tree over them is withheld.
//! **What this module pins is which gestures that costs**, because the
//! answer is per gesture and was once per client:
//!
//! - `Ctrl+T` / `g D` read `block_text`, move a marker and write the
//!   result back, so on a frozen page they compute an op from text the
//!   row never showed, the projection is then withheld, and the row
//!   comes back unchanged while the hidden state walks on. Refused up
//!   front, with nothing minted.
//! - The **text edit** is not refused here, and that is the whole of
//!   C1. `enter_insert_backlink` seeds the buffer from the op log, so
//!   the logged text is the text the user edited; the gate had nothing
//!   to protect and `commit_insert` consumes the buffer before it
//!   commits, so refusing dropped the keystrokes and said `outl
//!   reconcile` — a file condition that named none of it.
//! - A **structural** op names a node and converges through the log.
//!
//! So the assertions are on the **op log** and the block's text, not on
//! the message: a test that only reads `status` passes just as well
//! against a post-commit report, and the test this replaced asserted
//! the loss as the contract.

use super::tests::edit_the_focused_backlink;
use crate::actions::block::test_support::workspace_with_a_backlink;
use crate::state::App;
use std::path::Path;

const EDITED: &str = "mentions [[target]] and is edited";
const ONLY_ON_DISK: &str = "- only ever on disk\n";

/// The two gestures whose op is computed from the block's **logged**
/// text: what to call it, and the chord that runs it.
///
/// A table rather than two copies of each test, because the defect is
/// per gesture and these two were classified as "structural" together.
type MarkerChord = (&'static str, fn(&mut App));
fn marker_chords() -> Vec<MarkerChord> {
    vec![
        ("Ctrl+T", App::toggle_todo as fn(&mut App)),
        ("g D", App::mark_done),
    ]
}

/// Put `source.md` in the state the hash gate cannot tell from a
/// merely stale projection: the sidecar agrees with the bytes on disk,
/// and one of those lines exists in no op.
///
/// Restamping is how the producer-side defect leaves a page (RFC 0210),
/// and it is what `outl-actions`' own `if_stale` fixtures do — the
/// alternative is a `reconcile_md` that has to be talked into
/// mis-stamping, which tests the producer rather than this guard.
fn put_the_source_page_ahead_of_its_log(source: &Path) {
    let mut md = std::fs::read_to_string(source).expect("the fixture wrote the source page");
    md.push_str(ONLY_ON_DISK);
    std::fs::write(source, &md).expect("append a line no op produced");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(source);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("the fixture reconciled it");
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sidecar).expect("restamp as hash-faithful");
}

/// **C1, and the assertion is the op log and the text — never the
/// status line.**
///
/// The buffer is already consumed when `commit_backlink_mutation` runs
/// (`commit_insert` replaces the mode on its first line), so a refusal
/// here drops the user's keystrokes: they existed in no op, no `.md`
/// and on no peer, only in that buffer. The gate bought nothing in
/// exchange — the buffer was seeded from `workspace.block_text`, so the
/// text it applies to is the text the user edited.
///
/// Durability is the whole claim, so it is asserted twice over: the log
/// grew, and the block carries what was typed.
#[test]
fn an_edit_into_a_page_ahead_of_its_log_keeps_the_users_text() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    put_the_source_page_ahead_of_its_log(&source);
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let ops_before = app.workspace.log().len();

    edit_the_focused_backlink(&mut app, EDITED);

    assert!(
        app.workspace.log().len() > ops_before,
        "the typed text reached no op: it is now in no `.md`, no log and \
         on no peer, and the status line named a file condition instead"
    );
    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(EDITED),
        "the op landed but not the user's text"
    );
}

/// The other half of invariant 8, and it is what makes committing the
/// edit safe: the op goes in the log, and
/// `apply_page_md_with_sidecar_guarded` inside
/// [`BacklinkCommit::project`](super::BacklinkCommit) still withholds
/// the write, so the lines that exist only on disk stay there.
///
/// **This test used to claim it covered the gate and did not** — it was
/// green with the pre-check forced either way, because the guarded
/// writer refuses on its own account. It is aimed at that writer now,
/// and the op-log assertion is what makes it bite in the direction this
/// change moved: withhold the op instead of the projection and it fails
/// on the first assertion rather than passing by luck.
#[test]
fn the_edit_lands_in_the_log_while_the_lines_only_on_disk_stay_on_disk() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    put_the_source_page_ahead_of_its_log(&source);
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");

    edit_the_focused_backlink(&mut app, EDITED);

    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(EDITED),
        "the op is the durable half and it has to land"
    );
    let md = std::fs::read_to_string(&source).expect("the source page's `.md`");
    assert!(
        md.contains("only ever on disk"),
        "the content the log never saw was overwritten:\n{md}"
    );
    assert!(
        !md.contains("and is edited"),
        "the projection was not withheld, so the write that deletes ran:\n{md}"
    );
}

/// **The gate's remaining job.** `toggle_todo` / `mark_done` read the
/// block's logged text, move a marker and write the result back, while
/// the row the chord was aimed at was rendered from the `.md`. On a
/// frozen page that computes an op from text the user never saw, the
/// projection is then withheld, and the row comes back unchanged — so
/// repeated chords walk `TODO → DOING → DONE` invisibly in the log and
/// `outl reconcile --ahead-of-log` later drops all of it.
///
/// Asserted on the log and the text: nothing was typed, so there is no
/// cost to weigh against refusing.
#[test]
fn a_marker_chord_into_a_page_ahead_of_its_log_writes_no_op() {
    for (name, chord) in marker_chords() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        put_the_source_page_ahead_of_its_log(&source);
        let backlink = app
            .focused_backlink_target()
            .unwrap_or_else(|| panic!("{name}: a healthy backlink row resolves"));
        let text_before = app.workspace.block_text(backlink.block);
        let ops_before = app.workspace.log().len();

        chord(&mut app);

        assert_eq!(
            app.workspace.log().len(),
            ops_before,
            "{name}: a marker was computed from the logged text and written \
             into a page that cannot project it — invisible on the row, and \
             dropped by the next reconcile"
        );
        assert_eq!(
            app.workspace.block_text(backlink.block),
            text_before,
            "{name}: the citing block's text changed, so the op applied to \
             text the row never rendered"
        );
    }
}

/// **A structural op is not refused, and that follows from the
/// criterion rather than from "structural".** `delete` is an
/// `Op::Move` naming a node: it reads no text, so a stale local `.md`
/// cannot change what it does, and it converges through the op log
/// whatever that file says. The frozen `.md` is the pre-existing
/// condition, not something the gesture caused (invariant 11).
#[test]
fn a_structural_chord_into_a_page_ahead_of_its_log_still_lands_its_op() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    put_the_source_page_ahead_of_its_log(&source);
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let ops_before = app.workspace.log().len();

    app.delete_current();

    assert!(
        app.workspace.log().len() > ops_before,
        "a gesture that names a node was withheld over a condition it \
         cannot be affected by"
    );
    assert!(
        outl_actions::is_trashed(&app.workspace, backlink.block),
        "the delete reported nothing and did nothing"
    );
}

/// Every word is `outl_actions::refusal::ahead_of_log_notice`'s, at
/// `Surface::Tui` — the same call `report_backlink_projection_failure`
/// makes, so the pre-check and the post-commit report cannot describe
/// one event two ways.
///
/// Compared against the catalog's own output rather than a literal:
/// a literal here is the second copy the catalog exists to prevent,
/// and it would keep passing after the catalog was reworded.
///
/// It is the *marker chord's* refusal now. Nothing was typed, so the
/// notice may describe the file and nothing else; the half that says
/// "your text could not be saved" belongs to a gesture that consumed a
/// buffer, and no gesture reaches this notice having done so.
#[test]
fn the_marker_refusal_is_the_catalogs_sentence_and_names_no_block_id() {
    for (name, chord) in marker_chords() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        put_the_source_page_ahead_of_its_log(&source);
        let backlink = app
            .focused_backlink_target()
            .unwrap_or_else(|| panic!("{name}: a healthy backlink row resolves"));

        chord(&mut app);

        let notice = outl_actions::refusal::ahead_of_log_notice(
            outl_actions::refusal::Surface::Tui,
            1,
            "- only ever on disk",
        );
        assert_eq!(
            app.status,
            format!("`source`: {}", notice.one_line()),
            "{name}: the refusal has to be the catalog's sentence, not one \
             this crate wrote"
        );
        assert!(
            !app.status.contains(&backlink.block.to_string()),
            "{name}: a block id reached the status line: {}",
            app.status
        );
    }
}

/// **Invariant 8's mirror, which is the half that never gets reported.**
///
/// Gating on the wrong predicate froze every fresh journal day and every
/// unfilled `[[link]]` page, silently (issue #332): a page holding only
/// bare bullets is unanswerable to `sidecar_can_answer` and has nothing
/// to lose. The pre-check inherits `sidecar_can_vouch_for` through
/// `reproject_stale_md` rather than asking its own question, so these
/// three states have to keep committing — and asserting that here is
/// what stops a later "tighten the gate" from passing.
///
/// Driven by a **marker chord**, because that is the only gesture left
/// asking the question: driving it with the text edit would be green
/// whatever the gate answered, which is the shape of the vacuous test
/// this module shipped with.
#[test]
fn a_source_page_with_nothing_to_lose_still_takes_a_marker_chord() {
    for (name, prepare) in cases() {
        let (mut app, _d, source) = workspace_with_a_backlink();
        prepare(&source);
        let backlink = app
            .focused_backlink_target()
            .unwrap_or_else(|| panic!("{name}: a healthy backlink row resolves"));
        let before = app.workspace.block_text(backlink.block).unwrap_or_default();

        app.toggle_todo();

        assert_eq!(
            app.workspace.block_text(backlink.block).as_deref(),
            Some(outl_actions::cycle_todo(&before).as_str()),
            "{name}: the pre-check froze a page with nothing to lose, \
             which is the failure nobody reports"
        );
    }
}

/// One "nothing to lose" shape: what it is called, and how to put the
/// source page into it.
type NothingToLoseCase = (&'static str, fn(&Path));

/// The three shapes `sidecar_can_vouch_for` was narrowed for, each
/// applied to the **source** page — the one the chord is aimed at.
fn cases() -> Vec<NothingToLoseCase> {
    vec![
        // Bare bullets and nothing else: `outl init`'s two pages, and
        // every page created for a `[[link]]` nobody filled in.
        ("an empty page", |source: &Path| {
            std::fs::write(source, "- \n").expect("empty the source page");
            let sidecar_path = outl_md::sidecar::sidecar_path_for(source);
            let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("reconciled");
            for block in &mut sidecar.blocks {
                block.text = String::new();
            }
            sidecar.last_synced_hash = outl_md::sidecar::file_hash("- \n");
            outl_md::sidecar::write(&sidecar_path, &sidecar).expect("write it back");
        }),
        // No sidecar at all: a peer shipped the `.md` alone, or an
        // import landed before the first reconcile. Nothing
        // establishes that outl wrote these bytes, so the gate
        // declines the *write* — it must not decline the *op*.
        ("a page with no sidecar", |source: &Path| {
            std::fs::remove_file(outl_md::sidecar::sidecar_path_for(source))
                .expect("drop the sidecar");
        }),
        // No `.md` on disk: the shape of a journal day the moment it
        // is created, before anything has been projected.
        ("a page with no `.md` yet", |source: &Path| {
            std::fs::remove_file(source).expect("drop the projection");
            std::fs::remove_file(outl_md::sidecar::sidecar_path_for(source))
                .expect("and its sidecar, as a brand-new day has neither");
        }),
    ]
}

/// **A retryable condition is not a refusal**, which is the other way
/// this pre-check could have gone wrong.
///
/// An unreadable `.md` (here: bytes that are not UTF-8, the shape an
/// iCloud placeholder or a truncated transfer has) is local and
/// transient. The op log is still the right place for the write, so the
/// commit goes ahead and the condition is a `tracing::warn!` — the
/// same policy `load_current_no_autorun` and the trash restore follow.
/// Turning it into a refusal would make a permissions blip look like a
/// page that stopped syncing, and then the notice that means the latter
/// goes unread.
///
/// On the marker chord for `a_source_page_with_nothing_to_lose_…`'s
/// reason: it is the gesture that asks.
#[test]
fn an_unreadable_source_page_is_logged_and_the_marker_still_lands() {
    let (mut app, _d, source) = workspace_with_a_backlink();
    std::fs::write(&source, [0xff, 0xfe, 0x00, 0x9f]).expect("bytes that are not UTF-8");
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let before = app.workspace.block_text(backlink.block).unwrap_or_default();

    app.toggle_todo();

    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(outl_actions::cycle_todo(&before).as_str()),
        "a transient I/O condition must not withhold the op"
    );
    assert!(
        !app.status.contains("isn't syncing"),
        "an I/O condition was dressed up as invariant 8's refusal: {}",
        app.status
    );
}

/// A sanity net under the fixture: without the restamp, the same chord
/// commits. Otherwise the refusal tests above could be passing because
/// the fixture broke the backlink rather than because the guard fired.
#[test]
fn the_same_marker_chord_commits_when_the_page_is_not_ahead_of_its_log() {
    let (mut app, _d, _source) = workspace_with_a_backlink();
    let backlink = app
        .focused_backlink_target()
        .expect("a healthy backlink row resolves");
    let before = app.workspace.block_text(backlink.block).unwrap_or_default();
    let ops_before = app.workspace.log().len();

    app.toggle_todo();

    assert!(
        app.workspace.log().len() > ops_before,
        "the fixture itself must not be what refuses the chord"
    );
    assert_eq!(
        app.workspace.block_text(backlink.block).as_deref(),
        Some(outl_actions::cycle_todo(&before).as_str()),
        "and the marker has to land on the block the row names"
    );
}
