//! `apply_page_md_with_sidecar_if_stale` and the invariant-8 guard.
//!
//! Several of these are named in the root `CLAUDE.md` as the regression
//! net for issue #210. Do not rename or delete one.

use super::*;

/// The reported bug: a peer's op lands in the TREE, but the already-present
/// `.md` the view reads is never re-projected, so the page renders empty.
/// `_if_stale` must detect the tree ran ahead and re-project.
#[test]
fn if_stale_reprojects_when_tree_ran_ahead_of_the_md() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));

    // A synced-in block enters the tree; nothing re-projects the `.md`.
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();
    assert!(!std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(
        wrote.is_some(),
        "a tree ahead of its .md must be re-projected"
    );
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        md.contains("first") && md.contains("synced-in"),
        "re-projection must carry the synced-in block: {md:?}"
    );
}

/// An in-sync page must NOT be re-projected — otherwise every nav churns the
/// sidecar's `last_synced_at` and floods sync (the reason `_if_absent`
/// existed in the first place).
#[test]
fn if_stale_is_a_noop_when_the_md_matches_the_tree() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    let before = std::fs::read_to_string(&md_path).unwrap();
    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(wrote.is_none(), "an in-sync page must not be re-projected");
    assert_eq!(std::fs::read_to_string(&md_path).unwrap(), before);
}

/// Absent `.md` (a peer synced the page into the tree but it was never
/// projected here) → project it. Subsumes `_if_absent` (issue #120).
#[test]
fn if_stale_projects_a_page_whose_md_is_absent() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    assert!(!md_path.exists());

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(wrote.is_some());
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("first"));
}

/// A `.md` whose hash no longer matches its sidecar carries an unreconciled
/// external edit — `_if_stale` must leave it for the `.md → tree` reconcile,
/// never clobber it with a tree re-projection.
#[test]
fn if_stale_never_clobbers_an_external_edit() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, "- hand edited externally\n").unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();
    assert!(
        wrote.is_none(),
        "an externally-edited .md must not be clobbered"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- hand edited externally\n"
    );
}

/// The `.md` is a *faithful* projection by the hash gate — its sidecar
/// agrees with the bytes on disk — yet it carries content that exists in no
/// op. Measured on a real 2.5k-page workspace: 233 pages in that exact
/// state, 1,426 lines of content the log had never seen. (616 is what an
/// LCS diff reports, which is why the comparison is a multiset.)
///
/// The hash gate cannot tell this apart from a genuinely stale projection,
/// and the old code called both "stale" and re-projected, which deletes
/// the on-disk content and reports success. Every GUI open path
/// (`open_page_by_slug`, `open_journal_for`, …) runs through here, so the
/// loss fires on a plain page open, not just on `doctor --repair`.
///
/// Refuse, and say which lines would have been lost.
#[test]
fn if_stale_refuses_when_the_md_carries_content_the_log_lacks() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, "- first\n- only ever on disk\n").unwrap();
    restamp_sidecar_as_faithful(&md_path);

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);

    match result {
        Err(crate::ActionError::PageMarkdownAheadOfLog { sample, .. }) => assert!(
            sample.contains("only ever on disk"),
            "the error must name the content at risk, got {sample:?}"
        ),
        other => panic!("expected PageMarkdownAheadOfLog, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "- first\n- only ever on disk\n",
        "the bytes must survive untouched"
    );
}

/// A peer edited a block. The `.md` still holds the pre-edit text, the
/// tree holds the edit, and the sidecar is hash-faithful to the file.
///
/// The old line is on disk and absent from the render, which is exactly
/// what a disk-versus-render comparison measures — so the first version
/// of this guard refused, and the page froze showing the pre-edit text
/// with nothing surfaced to the user. That is issue #166 reintroduced
/// for the most ordinary sync case there is.
///
/// The question the guard has to ask is "does the op log know this
/// line", not "do disk and tree disagree". The sidecar answers the
/// first: its blocks are what the log held at the last agreement.
#[test]
fn if_stale_reprojects_a_page_a_peer_edited() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("original text");
    let block = crate::tree::children_of(&ws, page)[0].0;
    crate::block::edit_text(&mut ws, &hlc, block, "original text edited by peer").unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a remote edit must reach the .md, not freeze the page"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("edited by peer"));
}

/// Same shape, remote delete: the deleted block's text is on disk and
/// gone from the render. It is not unlogged content, it is content the
/// log deliberately removed, and refusing here resurrects it on the next
/// forced reconcile.
#[test]
fn if_stale_reprojects_a_page_a_peer_deleted_from() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("kept");
    let doomed = append_block(&mut ws, &hlc, Some(page), Some("to be deleted")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    crate::block::delete(&mut ws, &hlc, doomed).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(wrote.is_some(), "a remote delete must reach the .md");
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert!(md.contains("kept") && !md.contains("to be deleted"));
}

/// Indent with no text change moved the line but changed nothing about
/// what the log knows. `trim_end` alone left the indent in the
/// comparison, so `- child` and `  - child` read as different lines and
/// a pure indent counted as unlogged content.
#[test]
fn if_stale_reprojects_after_a_pure_indent() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("parent");
    let child = append_block(&mut ws, &hlc, Some(page), Some("child")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    crate::block::indent(&mut ws, &hlc, child).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(wrote.is_some(), "a pure indent is not unlogged content");
    assert!(std::fs::read_to_string(&md_path).unwrap().contains("child"));
}

/// A sidecar written before `SidecarBlock::text` existed (v1, and every
/// v2 written by a pre-0.11 binary) carries `text: ""` on every block.
///
/// Measured on a real workspace: 7,400 blocks, **zero** with text. With
/// no text to compare against, every line on disk reads as unknown, so
/// `content_lines_missing_from` stands down and returns an empty verdict
/// rather than flagging 615 pages / 35,261 lines against the 233 / 1,426
/// genuinely unlogged.
///
/// An empty verdict from a reference that cannot answer is **not** the
/// same as "nothing is at risk", and reading it as permission to write is
/// how a peer still on an older binary re-arms the exact loss this guard
/// exists to stop: it rewrites the sidecar without `text` and the next
/// page open here overwrites the file. So the write is declined instead —
/// quietly, because there is nothing to tell the user and nothing for
/// them to do: a `text`-less sidecar over real text carries a stale
/// `pipeline_version`, so `scan_for_orphans` has the page queued, and
/// its reconcile rewrites the sidecar **with** that text, which arms the
/// real check for the next open.
///
/// **That self-healing reaches this page and not every unanswerable
/// one.** A page whose blocks are genuinely empty gets the same
/// `text: ""` written back by each reconcile, so being queued never
/// clears it; the gate tells the two apart up front instead
/// (`sidecar_can_vouch_for`, issue #332).
#[test]
fn if_stale_declines_when_the_sidecar_cannot_answer() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    // Strip `text` from every block, as a pre-0.11 sidecar has it.
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    for b in &mut sc.blocks {
        b.text = String::new();
    }
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    let before = std::fs::read_to_string(&md_path).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_none(),
        "a sidecar that cannot answer must not authorise the write"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        before,
        "the file must be left exactly as it was"
    );
}

/// The narrowing above must not reach a page that simply has no blocks
/// yet: its sidecar carries an empty list, which is "nothing on disk to
/// lose", not "cannot answer". Refusing there would freeze every page
/// between its creation and its first block.
#[test]
fn if_stale_still_projects_a_page_whose_sidecar_has_no_blocks() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    // Project the empty page, then let a peer's block land in the tree.
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(wrote.is_some(), "an empty sidecar is not a veto");
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// Four defects in the comparison itself, all found by AI review on the
/// PR and reproduced with a probe before fixing.
///
/// An **empty block** is a first-class state (every Enter in the TUI makes
/// one) and its sidecar entry carries `text: ""`, so a sentinel that stood
/// down whenever *any* block had empty text disarmed the guard for the
/// whole page. Standing down only when *every* block is empty is the
/// legacy case it was meant for — but that alone reintroduces #166,
/// because the renderer emits a bare `-` for an empty block and a bare
/// `-` matched nothing on the sidecar side.
///
/// The other two are false negatives, where unlogged content slips
/// through: stripping the bullet marker repeatedly turned `- - - x` into
/// `x`, and filtering `key:: value` on the disk side hid content the
/// parser stores as a block's own text.
#[test]
fn content_comparison_handles_empty_blocks_and_marker_edge_cases() {
    fn blk(n: u128, text: &str) -> outl_md::sidecar::SidecarBlock {
        outl_md::sidecar::SidecarBlock::from_text(NodeId(ulid::Ulid(n)), 1, 0, text)
    }

    // An empty block among logged ones: the page is fully logged.
    let logged = vec![blk(1, "first"), blk(2, ""), blk(3, "third")];
    assert!(
        content_lines_missing_from("- first\n-\n- third\n", &logged).is_empty(),
        "an empty block is logged content, not a reason to flag or to stand down"
    );

    // And the guard still fires on the same page when a line really is
    // absent from the log.
    assert_eq!(
        content_lines_missing_from("- first\n-\n- unlogged\n", &logged),
        vec!["unlogged".to_string()],
        "an empty block must not disarm the guard for the rest of the page"
    );

    // The marker is stripped once, so a block whose text itself starts
    // with `- ` cannot be laundered into matching a different block.
    assert_eq!(
        content_lines_missing_from("- - - x\n", &[blk(9, "x")]),
        vec!["- - x".to_string()],
        "stripping the marker repeatedly let unlogged content pass"
    );

    // A bullet whose text looks like a property is content: that is how
    // the parser stores it.
    assert_eq!(
        content_lines_missing_from("- note:: remember\n", &[blk(8, "other")]),
        vec!["note:: remember".to_string()],
        "property-shaped block text must not be invisible to the guard"
    );

    // A property line the renderer emits for page or block props has no
    // bullet and never lives in a block's text, so it stays skipped.
    assert!(
        content_lines_missing_from("title:: Notes\n- first\n", &[blk(1, "first")]).is_empty(),
        "a rendered page property is not unlogged content"
    );
}

/// The counterpart that must keep working: the tree genuinely ran ahead
/// (a peer's ops landed), the `.md` holds a strict subset of what the log
/// renders, so nothing on disk is at risk. This is issue #166's case and
/// the guard above must not regress it.
#[test]
fn if_stale_still_reprojects_when_the_md_holds_no_unlogged_content() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a .md that lost nothing must still be re-projected"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// Whitespace-only drift must not trip the guard. The renderer's trailing
/// newline changed between releases, so on a real workspace a large share
/// of "stale" pages differ from the log by exactly that — refusing those
/// would strand the genuine re-projections behind noise.
#[test]
fn if_stale_ignores_whitespace_only_differences_when_deciding() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    // Trailing spaces + a missing final newline: no content is unique to
    // disk once trimmed, so the tree's new block still wins.
    std::fs::write(&md_path, "- first   ").unwrap();
    restamp_sidecar_as_faithful(&md_path);
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "whitespace-only drift is not unlogged content"
    );
    assert!(std::fs::read_to_string(&md_path)
        .unwrap()
        .contains("synced-in"));
}

/// A present-but-unreadable `.md` (non-UTF8 bytes here — `read_to_string`
/// fails with `InvalidData`, not `NotFound`) must NOT be treated as absent
/// and re-projected; that would clobber a file that may hold real content.
/// It surfaces the I/O error and leaves the bytes untouched.
#[test]
fn if_stale_does_not_clobber_an_unreadable_md() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    std::fs::write(&md_path, [0xff, 0xfe, 0x00]).unwrap();

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);
    assert!(
        result.is_err(),
        "an unreadable .md must surface an error, not be clobbered"
    );
    assert_eq!(std::fs::read(&md_path).unwrap(), vec![0xff, 0xfe, 0x00]);
}

/// An undownloaded iCloud file reads as `NotFound` — the real name does
/// **not** exist, only `.notes.md.icloud` does — and `_if_stale` mapped
/// `NotFound` straight to "project it".
///
/// `mutate_page_md` has refused this since the placeholder guard landed;
/// this door was never checked. It was survivable while `_if_stale` only
/// ran on page-open with a user present. `outl serve`'s projection sweep
/// runs it over **every** page, unattended, every 30 seconds — so the
/// write that collides with the file iCloud is still fetching now happens
/// on its own.
#[test]
fn if_stale_refuses_a_page_icloud_has_not_downloaded() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    // Only the placeholder is left, so nothing but it can carry the
    // verdict.
    std::fs::remove_file(outl_md::sidecar::sidecar_path_for(&md_path)).unwrap();
    std::fs::remove_file(&md_path).unwrap();
    std::fs::write(md_path.with_file_name(".notes.md.icloud"), "").unwrap();

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);

    assert!(
        matches!(
            result,
            Err(crate::error::ActionError::PageMarkdownNotDownloaded(_))
        ),
        "a file iCloud has not fetched yet is not an absent page — got {result:?}"
    );
    assert!(
        !md_path.exists(),
        "the refused projection must not create the file it declined to touch"
    );
}

/// The same absence with the other piece of evidence, and the one that
/// actually survives iCloud: the placeholder is a **dotfile**, which
/// iCloud Documents drops during cross-device sync (root `CLAUDE.md`
/// invariant 2), while the `.outl` sidecar is not and does ride along.
///
/// A sidecar is only ever written beside a `.md` this device projected,
/// so it is proof the page existed here. Projecting over that absence
/// writes a file the real bytes then collide with.
#[test]
fn if_stale_refuses_a_missing_md_beside_a_live_sidecar() {
    let (tmp, ws, _hlc, page, md_path) = projected_page("first");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let sidecar_before = std::fs::read_to_string(&sidecar_path).unwrap();
    std::fs::remove_file(&md_path).unwrap();

    let result = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page);

    assert!(
        matches!(
            result,
            Err(crate::error::ActionError::PageMarkdownVanished(_))
        ),
        "a missing .md beside a live sidecar is a lost file, not a new page — got {result:?}"
    );
    assert!(!md_path.exists(), "nothing may be written over the page");
    assert_eq!(
        std::fs::read_to_string(&sidecar_path).unwrap(),
        sidecar_before,
        "the sidecar is the evidence; the refusal must not rewrite it"
    );
}

/// Issue #332: a page whose only block is empty — a journal day the first
/// time it is opened, a `[[link]]` page nobody filled, either page
/// `outl init` leaves behind — is never re-projected once a peer's blocks
/// reach the tree.
///
/// Its sidecar carries one entry with `text: ""`, which is byte-identical
/// to a pre-0.11 sidecar, so [`sidecar_can_answer`] said no and the write
/// declined. Nothing surfaced: `Ok(None)` is what `reproject_stale_md`
/// reads as success, so the view was built from the stale `.md` and the
/// page showed one empty bullet indefinitely. Reported from iOS against a
/// Linux `outl serve`, on a workspace where the op log and tree were both
/// correct.
///
/// This is the mirror of #166 and it survived the fix for it.
#[test]
fn if_stale_reprojects_a_page_whose_only_block_is_empty() {
    let (tmp, mut ws, _hlc, page, md_path) = projected_page("");
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        "title:: Notes\n\n-\n"
    );

    let peer = HlcGenerator::new(ActorId::new());
    append_block(&mut ws, &peer, Some(page), Some("peer wrote this")).unwrap();
    append_block(&mut ws, &peer, Some(page), Some("and this")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_some(),
        "a page holding one bare bullet has nothing to lose — it must be re-projected"
    );
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        md.contains("peer wrote this") && md.contains("and this"),
        "the re-projection must carry the peer's blocks: {md:?}"
    );
}

/// The narrowing above must not reach the case the gate exists for. A
/// pre-0.11 sidecar (every `text` empty) over a `.md` holding **real**
/// text still declines, and the bytes still survive untouched.
///
/// This is the other half of `if_stale_declines_when_the_sidecar_cannot_answer`:
/// that one pins the decline, this one pins that the decline is decided by
/// what is on disk rather than by the sidecar's shape alone. Deleting
/// either re-arms the #210 loss.
#[test]
fn if_stale_still_declines_when_an_unanswerable_sidecar_covers_real_text() {
    let (tmp, mut ws, hlc, page, md_path) = projected_page("first");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    for b in &mut sc.blocks {
        b.text = String::new();
    }
    // Content the stripped sidecar cannot vouch for, declared faithful —
    // the state a pre-0.11 `reconcile_md` leaves behind.
    let disk = "title:: Notes\n\n- precious text only on disk\n";
    std::fs::write(&md_path, disk).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("synced-in")).unwrap();

    let wrote = apply_page_md_with_sidecar_if_stale(&ws, tmp.path(), page).unwrap();

    assert!(
        wrote.is_none(),
        "a sidecar that cannot vouch for real text on disk must not authorise the write"
    );
    assert_eq!(
        std::fs::read_to_string(&md_path).unwrap(),
        disk,
        "the file must be left exactly as it was"
    );
}
