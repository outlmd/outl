//! `sidecar_can_vouch_for` — see its doc for why the gate is this
//! question and not `sidecar_can_answer` ([issue #332]).
//!
//! Both directions are pinned, because the predicate decides whether
//! bytes get deleted: a page with nothing to lose vouches, a page
//! holding text the log never saw does not. Out here rather than in the
//! module's `mod tests` for the 600-line ratchet, and because
//! `mixed_version_sidecar.rs` already reaches its sibling the same way.
//!
//! [issue #332]: https://github.com/outlmd/outl/issues/332

use outl_core::id::NodeId;
use outl_md::sidecar::SidecarBlock;
use outl_md::unlogged;

fn blk(text: &str) -> SidecarBlock {
    SidecarBlock::from_text(NodeId::new(), 1, 0, text)
}

/// The reported shape (#332): a page whose blocks all carry an empty
/// `text` over a `.md` that holds nothing but bare bullets.
///
/// `sidecar_can_answer` says no, correctly — these blocks cannot say
/// what the log held. The vouching question is a different one and it
/// has an answer: the file holds no text, so there is nothing for the
/// sidecar to vouch for.
#[test]
fn an_all_empty_sidecar_can_vouch_for_a_md_holding_only_bare_bullets() {
    let disk = "title:: Notes\n\n-\n";
    let blocks = [blk("")];
    assert!(!unlogged::sidecar_can_answer(&blocks));
    assert!(unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// The case `sidecar_can_answer` was written for, and the reason this
/// predicate is not simply `true`: a pre-0.11 sidecar over a `.md`
/// that holds real text must still refuse.
#[test]
fn an_all_empty_sidecar_cannot_vouch_for_a_md_holding_real_text() {
    let disk = "- precious text only on disk\n";
    let blocks = [blk("")];
    assert!(!unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// One unlogged line among empties is still a refusal. The verdict is
/// "is anything at risk", not "is most of this empty".
#[test]
fn a_single_real_line_among_empty_ones_is_enough_to_refuse() {
    let disk = "-\n- real\n-\n";
    let blocks = [blk(""), blk(""), blk("")];
    assert!(!unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// A sidecar that *can* answer short-circuits: the content question
/// belongs to `content_lines_missing_from` at the call site, which
/// reports how many lines are at risk rather than just whether any
/// are. Vouching must not pre-empt that with a bare `false`.
#[test]
fn a_sidecar_that_can_answer_vouches_whatever_is_on_disk() {
    let disk = "- something the log never saw\n";
    let blocks = [blk("logged")];
    assert!(unlogged::sidecar_can_answer(&blocks));
    assert!(unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// Bare bullets a page accumulated beyond what its sidecar records
/// still vouch: an empty bullet carries no content, so a projection
/// that drops it deletes nothing. Pinned because the multiset makes
/// the surplus visible and the verdict must not turn on the count.
#[test]
fn surplus_bare_bullets_on_disk_still_vouch() {
    let disk = "-\n-\n-\n";
    let blocks = [blk("")];
    assert!(unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// Page and block properties travel the `Op::SetProp` channel, never a
/// block's `text`, so they are not content this predicate can lose.
/// `content_lines_missing_from` already skips them; pinned here so a
/// page with a `title::` is not read as holding unlogged text.
#[test]
fn property_lines_do_not_block_vouching() {
    let disk = "title:: Notes\nkind:: page\n\n-\n";
    let blocks = [blk("")];
    assert!(unlogged::sidecar_can_vouch_for(disk, &blocks));
}

/// An empty block list answers through the first arm, as
/// `sidecar_can_answer` already documents: a page with no blocks has
/// nothing to lose. Pinned so the added arm cannot change it.
#[test]
fn an_empty_block_list_vouches_for_anything() {
    assert!(unlogged::sidecar_can_vouch_for("- whatever\n", &[]));
}
