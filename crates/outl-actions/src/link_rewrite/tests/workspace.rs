//! The workspace half of the link rewriter's battery: `rewrite_mentions`
//! driving real `Op::Edit`s through the op log, rather than the pure text
//! transform its siblings in [`super`] cover.
//!
//! Split from `mod.rs` at the seam the file already had. The two halves
//! answer different questions — "is this spelling a mention, and how is
//! it respelled" against "does the op log end up holding that, and which
//! pages does the caller have to re-project" — and only this one needs a
//! `Workspace`.

use super::*;

// --- workspace: rewrite_mentions ------------------------------------------

struct World {
    w: Workspace,
    hlc: HlcGenerator,
    from: PageNames,
    to: PageNames,
    notes: NodeId,
}

/// Pages `alpha` (the one being replaced), `beta` (its replacement) and a
/// bystander `notes` holding the mentions.
fn world() -> World {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    World {
        w,
        hlc,
        from,
        to,
        notes,
    }
}

impl World {
    fn run(&mut self) -> LinkRewriteReport {
        rewrite_mentions(&mut self.w, &self.hlc, &self.from, &self.to).unwrap()
    }
}

#[test]
fn a_name_that_slug_resolves_to_another_page_is_not_a_mention() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Shared Name");
    page(&mut w, &hlc, "shared-name", "Something Else");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    // Navigation resolves this by slugify to `shared-name`, never reaching
    // the case-insensitive title rung where `alpha` would have matched.
    let elsewhere = block(&mut w, &hlc, notes, "[[Shared Name]]");
    let mine = block(&mut w, &hlc, notes, "[[alpha]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(text_of(&w, elsewhere), "[[Shared Name]]");
    assert_eq!(text_of(&w, mine), "[[Beta Project]]");
    assert_eq!(report.rewritten_blocks, vec![mine.to_string()]);
}

#[test]
fn a_ref_by_title_still_rewrites_when_nothing_else_claims_the_name() {
    let mut s = world();
    let b = block(&mut s.w, &s.hlc, s.notes, "[[alpha project]]");
    let report = s.run();
    assert_eq!(text_of(&s.w, b), "[[Beta Project]]");
    assert_eq!(report.rewritten_blocks, vec![b.to_string()]);
}

/// Through the op log, not only the pure text path: a rename has to reach
/// `![[old]]` or every transclusion aimed at the page keeps naming a page
/// that no longer answers to that name (issue #190).
#[test]
fn a_rename_respells_a_page_embed_through_the_op_log() {
    let mut s = world();
    let b = block(&mut s.w, &s.hlc, s.notes, "before ![[alpha]] after");
    let report = s.run();
    assert_eq!(text_of(&s.w, b), "before ![[Beta Project]] after");
    assert_eq!(report.rewritten_blocks, vec![b.to_string()]);
    assert_eq!(report.touched_pages, vec!["notes".to_string()]);
}

/// Same guarantee the ref has: running twice edits nothing the second
/// time. A splice whose width was wrong would keep finding work to do.
#[test]
fn rewriting_a_page_embed_twice_is_idempotent() {
    let mut s = world();
    let b = block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "![[alpha]] and ![[Alpha Project]]",
    );
    s.run();
    let once = text_of(&s.w, b);
    assert_eq!(once, "![[Beta Project]] and ![[Beta Project]]");
    let again = s.run();
    assert_eq!(text_of(&s.w, b), once);
    assert!(again.rewritten_blocks.is_empty(), "{again:?}");
}

#[test]
fn rewrites_every_form_across_the_workspace() {
    let mut s = world();
    let r = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let t = block(&mut s.w, &s.hlc, s.notes, "do it #alpha.");
    let nested = block(&mut s.w, &s.hlc, r, "**[[Alpha Project]]**");
    s.run();
    assert_eq!(text_of(&s.w, r), "[[Beta Project]]");
    assert_eq!(text_of(&s.w, t), "do it #beta.");
    assert_eq!(text_of(&s.w, nested), "**[[Beta Project]]**");
}

#[test]
fn a_person_page_is_rewritten_through_the_page_names() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "ana", "Ana Souza");
    let b = page(&mut w, &hlc, "ana-lima", "Ana Lima");
    for id in [a, b] {
        let kind = Some(PropValue::Text("person".into()));
        set_property(&mut w, &hlc, id, "type", kind).unwrap();
    }
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let m = block(&mut w, &hlc, notes, "ask [[@Ana Souza]] / [[ana]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    assert!(from.person && to.person);

    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(text_of(&w, m), "ask [[@Ana Lima]] / [[Ana Lima]]");
}

#[test]
fn code_and_fences_in_the_workspace_are_untouched() {
    let mut s = world();
    let text = "`[[alpha]]`\n```\n[[alpha]]\n#alpha\n```";
    let b = block(&mut s.w, &s.hlc, s.notes, text);
    let ops = s.w.log().len();
    let report = s.run();
    assert_eq!(text_of(&s.w, b), text);
    assert!(report.rewritten_blocks.is_empty());
    assert_eq!(s.w.log().len(), ops);
}

#[test]
fn a_namespace_child_is_skipped_and_reported() {
    let mut s = world();
    let b = block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "[[alpha/child]] #alpha/child/deep and [[alpha]]",
    );
    let other = block(&mut s.w, &s.hlc, s.notes, "[[alpha/child]] [[alpha/child]]");
    let report = s.run();

    assert_eq!(
        text_of(&s.w, b),
        "[[alpha/child]] #alpha/child/deep and [[Beta Project]]"
    );
    assert_eq!(text_of(&s.w, other), "[[alpha/child]] [[alpha/child]]");
    let skipped: Vec<(String, String)> = report
        .skipped_namespace_refs
        .iter()
        .map(|s| (s.block_id.clone(), s.name.clone()))
        .collect();
    assert_eq!(
        skipped,
        vec![
            (b.to_string(), "alpha/child".to_string()),
            (b.to_string(), "alpha/child/deep".to_string()),
            (other.to_string(), "alpha/child".to_string()),
        ]
    );
}

#[test]
fn a_second_run_rewrites_nothing() {
    let mut s = world();
    block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "[[alpha]] #alpha **[[Alpha Project]]**",
    );
    block(&mut s.w, &s.hlc, s.notes, "[[alpha/child]]");
    let first = s.run();
    assert_eq!(first.rewritten_blocks.len(), 1);

    let ops = s.w.log().len();
    let second = s.run();
    assert!(second.rewritten_blocks.is_empty());
    assert!(second.touched_pages.is_empty());
    assert_eq!(s.w.log().len(), ops, "nothing left to edit");
}

#[test]
fn a_case_only_rename_is_stable_on_a_second_run() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let notes = page(&mut w, &hlc, "notes", "Notes");
    let m = block(&mut w, &hlc, notes, "[[Alpha Project]]");
    let from = names_of(&w, a);
    let to = names("alpha", "ALPHA PROJECT");

    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();
    assert_eq!(text_of(&w, m), "[[ALPHA PROJECT]]");
    let ops = w.log().len();
    rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();
    assert_eq!(w.log().len(), ops);
}

#[test]
fn identical_names_are_a_no_op() {
    let mut s = world();
    block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "[[alpha]] #alpha [[alpha/child]]",
    );
    let ops = s.w.log().len();
    s.to = s.from.clone();
    let report = s.run();
    assert!(report.rewritten_blocks.is_empty());
    assert!(report.touched_pages.is_empty());
    assert!(report.skipped_namespace_refs.is_empty());
    assert_eq!(s.w.log().len(), ops);
}

#[test]
fn one_edit_op_per_changed_block() {
    let mut s = world();
    block(
        &mut s.w,
        &s.hlc,
        s.notes,
        "[[alpha]] twice [[alpha]] and #alpha",
    );
    block(&mut s.w, &s.hlc, s.notes, "untouched [[beta]]");
    block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let before = s.w.log().len();
    let report = s.run();
    assert_eq!(report.rewritten_blocks.len(), 2);
    assert_eq!(s.w.log().len() - before, 2);
}

#[test]
fn blocks_are_edited_in_document_order() {
    let mut s = world();
    let first = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let second = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let child = block(&mut s.w, &s.hlc, first, "[[alpha]]");
    let report = s.run();
    assert_eq!(
        report.rewritten_blocks,
        vec![first.to_string(), child.to_string(), second.to_string()]
    );
}

#[test]
fn page_roots_are_never_rewritten() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    // A page whose own title is prose that mentions `alpha`.
    let titled = page(&mut w, &hlc, "see-alpha", "see [[alpha]]");
    let (from, to) = (names_of(&w, a), names_of(&w, b));
    let title_before = page_meta(&w, titled).unwrap().title;
    let text_before = w.block_text(titled);
    let ops = w.log().len();

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert!(report.rewritten_blocks.is_empty());
    assert_eq!(page_meta(&w, titled).unwrap().title, title_before);
    assert_eq!(w.block_text(titled), text_before);
    assert_eq!(w.log().len(), ops);
}

#[test]
fn trashed_blocks_are_never_rewritten() {
    let mut s = world();
    let live = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let doomed = block(&mut s.w, &s.hlc, s.notes, "[[alpha]]");
    let under_doomed = block(&mut s.w, &s.hlc, doomed, "#alpha");
    delete(&mut s.w, &s.hlc, doomed).unwrap();

    let report = s.run();

    assert_eq!(report.rewritten_blocks, vec![live.to_string()]);
    assert_eq!(text_of(&s.w, doomed), "[[alpha]]");
    assert_eq!(text_of(&s.w, under_doomed), "#alpha");
}

#[test]
fn rewritten_blocks_name_their_pages_once_and_sorted() {
    let (mut w, hlc) = ws();
    let a = page(&mut w, &hlc, "alpha", "Alpha Project");
    let b = page(&mut w, &hlc, "beta", "Beta Project");
    let zed = page(&mut w, &hlc, "zed", "Zed");
    let ann = page(&mut w, &hlc, "ann", "Ann");
    let quiet = page(&mut w, &hlc, "quiet", "Quiet");
    let z1 = block(&mut w, &hlc, zed, "[[alpha]]");
    let z2 = block(&mut w, &hlc, zed, "#alpha");
    let n1 = block(&mut w, &hlc, ann, "x");
    let n2 = block(&mut w, &hlc, n1, "[[alpha]]");
    block(&mut w, &hlc, quiet, "nothing here");
    let (from, to) = (names_of(&w, a), names_of(&w, b));

    let report = rewrite_mentions(&mut w, &hlc, &from, &to).unwrap();

    assert_eq!(
        report.touched_pages,
        vec!["ann".to_string(), "zed".to_string()]
    );
    let mut ids = report.rewritten_blocks.clone();
    ids.sort();
    let mut expected = vec![z1.to_string(), z2.to_string(), n2.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn a_title_only_rename_leaves_tags_alone() {
    let mut s = world();
    s.to = names("alpha", "Alpha Renamed");
    let b = block(&mut s.w, &s.hlc, s.notes, "#alpha and [[alpha project]]");
    s.run();
    assert_eq!(text_of(&s.w, b), "#alpha and [[Alpha Renamed]]");
}
