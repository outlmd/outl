//! Regression net for the **producer** side of issue #210.
//!
//! A page reaches the "hash-faithful but unlogged" state when
//! `render → parse` is not a roundtrip: the renderer writes every line
//! of a block's text, the parser reads back only some of them, and the
//! next `reconcile_md` writes that truncation into the op log as an
//! `Op::Edit` while stamping `last_synced_hash` over the *whole* file.
//!
//! The consumer-side net (the guard that refuses to overwrite such a
//! page) lives in `outl-actions/src/journal/tests.rs` and
//! `outl-cli/src/cmd/doctor/tests/safety.rs`. This file pins the other
//! end: the state must not be *created* in the first place.
//!
//! Measured cost of the two defects below on a real 2,560-page
//! workspace: 41 pages holding 387 lines that existed in no op, 0 after.
//! See [RFC 0210](../../../docs/rfcs/0210-md-content-outside-op-log.md).

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_md::parse::parse;
use outl_md::render::render;
use outl_md::{OutlineNode, ParsedPage};

fn page_of(blocks: Vec<OutlineNode>) -> ParsedPage {
    ParsedPage {
        blocks,
        properties: Vec::new(),
        warnings: Vec::new(),
    }
}

fn block(text: &str) -> OutlineNode {
    OutlineNode {
        text: text.to_string(),
        properties: Vec::new(),
        children: Vec::new(),
    }
}

/// `text → render → parse` must return the same text.
fn assert_text_roundtrips(text: &str) {
    let rendered = render(&page_of(vec![block(text)]));
    let back = parse(&rendered);
    assert_eq!(
        back.blocks.len(),
        1,
        "expected one block back, got {} — rendered:\n{rendered}",
        back.blocks.len()
    );
    assert_eq!(
        back.blocks[0].text, text,
        "text did not survive render → parse — rendered:\n{rendered}"
    );
}

/// The exact shape that produced the loss: a briefing whose body is one
/// block carrying blank lines *and* its own indentation.
///
/// Before the fix this reparsed to a single line and the remaining 6
/// were dropped with `warnings: 0` — invisible to the user, to
/// `doctor`, and to the op log at once.
#[test]
fn a_block_whose_text_holds_blank_lines_and_indentation_survives_a_roundtrip() {
    assert_text_roundtrips(
        "🚨 Briefing Operacional — 2026-07-08\n\n  🔴 CRÍTICO — pricing parado\n    `L48h v1` — 25 falhas consecutivas\n\n  🚌 INCIDENTES — 5 ocorrências\n    08:19 UTC — Grupo 12805732",
    );
}

#[test]
fn a_blank_line_inside_a_blocks_text_is_not_read_as_a_separator() {
    assert_text_roundtrips("first paragraph\n\nsecond paragraph");
}

#[test]
fn a_continuation_line_keeps_its_own_indentation() {
    assert_text_roundtrips("head\n  detail\n    deeper detail");
}

#[test]
fn consecutive_blank_lines_inside_a_block_all_survive() {
    assert_text_roundtrips("one\n\n\ntwo");
}

/// A blank line *between* two blocks still separates them — the fix
/// distinguishes the two cases by indent, and must not have collapsed
/// this one into a continuation.
#[test]
fn a_blank_line_between_two_blocks_still_separates_them() {
    let back = parse("- first\n\n- second\n");
    assert_eq!(back.blocks.len(), 2);
    assert_eq!(back.blocks[0].text, "first");
    assert_eq!(back.blocks[1].text, "second");
}

/// Children still win over continuation: a bullet at `indent + 1` is a
/// child block, never text folded into its parent.
#[test]
fn an_indented_bullet_is_still_a_child_not_continuation() {
    let back = parse("- parent\n  - child\n");
    assert_eq!(back.blocks.len(), 1);
    assert_eq!(back.blocks[0].text, "parent");
    assert_eq!(back.blocks[0].children.len(), 1);
    assert_eq!(back.blocks[0].children[0].text, "child");
}

/// The contract, stated as the property it actually is: **no non-blank
/// line of a `.md` may disappear across `parse → render`**, at any
/// depth, under any shape the grammar cannot place.
///
/// This is the check that fails on the pre-fix parser. It recovered an
/// unplaceable line only at `indent == 0` and skipped it in silence at
/// every deeper level — no AST entry, no warning, nothing for `doctor`
/// to find. That silent arm is where most of the measured 387 lines went.
///
/// The parser is free to decide *how* a line survives (continuation of
/// the open block, or a recovered verbatim block plus a warning). It is
/// not free to drop it.
#[test]
fn no_non_blank_line_is_ever_lost_between_parse_and_render() {
    let cases = [
        // Over-indented line under a child, at increasing depth.
        "- parent\n  - child\n      stray line\n",
        "- a\n  - b\n    - c\n              orphan at depth 3\n",
        // The shape a Roam import leaves behind: bullets the dialect
        // does not know, at depth.
        "- a\n  - b\n    * asterisk bullet\n    • unicode bullet\n",
        // Continuation reopened after a blank line, deep in the tree.
        "- a\n  - b\n\n    trailing prose\n",
        // A property line followed by prose the grammar cannot place.
        "- a\n  key:: value\n  - child\n        after the child\n",
    ];
    for md in cases {
        let rendered = render(&parse(md));
        for line in md.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let body = line.strip_prefix("- ").unwrap_or(line);
            assert!(
                rendered.contains(body),
                "line {body:?} vanished\n--- input ---\n{md}\n--- rendered ---\n{rendered}"
            );
        }
    }
}

/// An unplaceable line that cannot be folded into any open block still
/// has to be *reported*, not merely preserved — the user needs to know
/// their file holds something the dialect could not read.
#[test]
fn a_line_the_grammar_cannot_place_at_top_level_is_reported() {
    let back = parse("    over-indented before any bullet\n- a\n");
    assert!(
        !back.warnings.is_empty(),
        "an unplaceable line must be reported, never dropped"
    );
}

/// `render → parse` must be a **fixpoint**, not merely lossless.
///
/// Both regressions this pins were introduced by the fix above and
/// caught in review, not by the 237 tests that were green at the time.
/// They share a failure mode the original bug did not have: the original
/// dropped content and then *converged*, while these mutate the document
/// on every save.
///
/// - A bullet at an irregular indent (`    - child`, four spaces) was
///   recovered as verbatim **text**. The text carried a `- ` that the
///   renderer then wrote after its own marker, so each pass read one
///   more level of nesting: `- parent\n    - child` became
///   `- parent\n  -     - child`, then `- parent\n  - - child`.
/// - A whitespace-only line before a *sibling* appended a trailing `\n`
///   to the previous block's text. Invisible to the reader, a different
///   `content_hash` to the log — so every page carrying that shape
///   emitted an `Op::Edit` on the next reconcile, forever.
#[test]
fn render_then_parse_is_a_fixpoint_not_just_lossless() {
    let cases = [
        "- parent\n    - child\n",
        "- a\n    - b\n        - c\n",
        "- first\n  \n- second\n",
        "- head\n\n  body\n",
        "- a\n  - b\n    - c\n",
        "title:: Notes\n\n- a\n  key:: v\n  prose\n",
    ];
    for md in cases {
        let once = render(&parse(md));
        let twice = render(&parse(&once));
        assert_eq!(
            once, twice,
            "not a fixpoint — the document changes on every save\n\
             input:\n{md}\npass 1:\n{once}\npass 2:\n{twice}"
        );
    }
}

/// A blank line that turns out to be trailing must leave no mark.
///
/// The text is what the `content_hash` is taken over, so a `\n` nobody
/// typed is an `Op::Edit` nobody asked for.
#[test]
fn a_trailing_blank_line_does_not_end_up_in_the_blocks_text() {
    let back = parse("- first\n  \n- second\n");
    assert_eq!(back.blocks.len(), 2);
    assert_eq!(back.blocks[0].text, "first");
    assert_eq!(back.blocks[1].text, "second");
}

/// An irregularly-indented bullet keeps its meaning: it is a block, not
/// a line of text that happens to start with a dash.
#[test]
fn a_four_space_indented_bullet_is_still_a_block() {
    let back = parse("- parent\n    - child\n");
    let rendered = render(&back);
    assert!(
        !rendered.contains("- -") && !rendered.contains("-     -"),
        "the marker was folded into the text — it will grow on each save:\n{rendered}"
    );
    assert!(rendered.contains("child"), "content lost:\n{rendered}");
}

/// The enforcement of invariant 8 in `reconcile_md` is defence in
/// depth: it withholds `last_synced_hash` whenever the pass read content
/// it could not emit an op for, whatever the reason.
///
/// Its own failure mode is the mirror of the bug it guards — a **false
/// positive** leaves the page permanently dirty, reconciling on every
/// boot and never converging, which on a 2,500-page workspace is the
/// whole graph. So the test that matters is not "does it fire", it is
/// "does it stay quiet on every shape a real workspace holds".
///
/// The shapes below are taken from the pages that actually carried
/// unlogged content: Roam's tab-indented ordinals and `*` bullets, a
/// `U+2029` paragraph separator pasted from a PDF, non-breaking spaces,
/// code fences, properties followed by prose.
#[test]
fn the_hash_is_still_advanced_for_every_shape_a_real_workspace_holds() {
    let shapes = [
        ("plain", "- one\n- two\n"),
        ("nested", "- one\n  - two\n    - three\n"),
        ("page props", "title:: Notes\n\n- one\n"),
        ("block props", "- one\n  key:: value\n  - child\n"),
        ("prose after a property", "- one\n  key:: value\n  prose\n"),
        ("fence", "- intro\n  ```rust\n  fn main() {}\n  ```\n"),
        ("unclosed fence", "- intro\n  ```rust\n  fn main() {}\n"),
        ("empty blocks", "- one\n-\n- two\n"),
        ("duplicate lines", "- x\n- x\n- x\n"),
        ("roam tab ordinal", "- a\n  - b\n    1.\tOs que têm medo\n"),
        ("roam asterisk", "- a\n  - b\n    * Nibo\n"),
        ("roam unicode bullet", "- a\n  •\tEntrevistas\n"),
        ("paragraph separator", "- a\n  x\u{2029}y\n"),
        ("non breaking space", "- a\n  \u{a0}weird\n"),
        ("multiline with blanks", "- head\n\n  body\n\n  tail\n"),
        ("quote continuation", "- > line one\n  > line two\n"),
        ("deep orphan", "- a\n  - b\n    - c\n          orphan\n"),
    ];

    for (name, md) in shapes {
        let dir = tempfile::tempdir().expect("tempdir");
        let md_path = dir.path().join("page.md");
        std::fs::write(&md_path, md).expect("write md");

        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).expect("workspace");
        let hlc = HlcGenerator::new(actor);

        let report = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");
        assert_eq!(
            report.unlogged_lines, 0,
            "{name}: reconcile read content it could not log, so the page \
             will stay dirty forever — input:\n{md}"
        );

        let sidecar =
            outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)).expect("sidecar");
        assert!(
            !sidecar.last_synced_hash.is_empty(),
            "{name}: the hash was withheld, so this page never reads as in sync"
        );
    }
}

/// The other half of the same contract: a page that reconciles cleanly
/// once must be a no-op the second time. A page that emits ops on every
/// pass grows the log without bound and never converges — the symptom a
/// user sees is the app writing to disk while they are not typing.
#[test]
fn a_second_reconcile_over_unchanged_bytes_is_always_a_no_op() {
    for md in [
        "- one\n  - two\n",
        "- head\n\n  body\n",
        "title:: Notes\n\n- one\n  key:: value\n  prose\n",
        "- intro\n  ```rust\n  fn main() {}\n  ```\n",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let md_path = dir.path().join("page.md");
        std::fs::write(&md_path, md).expect("write md");

        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).expect("workspace");
        let hlc = HlcGenerator::new(actor);

        outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("first");
        let second = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("second");
        assert_eq!(second.ops_applied, 0, "not idempotent for:\n{md}");
    }
}

/// End-to-end on the pipeline the bug actually ran through: project a
/// page from the tree, read it back, and check the op log still holds
/// every line.
///
/// This is the test that would have failed on the commit that shipped
/// the defect. The unit roundtrips above pin the mechanism; this one
/// pins the consequence.
#[test]
fn reconciling_a_projected_multiline_block_does_not_truncate_the_op_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let md_path = dir.path().join("2026-07-08.md");

    let body = "🚨 Briefing — 2026-07-08\n\n  🔴 CRÍTICO — pricing parado\n  🚌 INCIDENTES — 5 ocorrências";
    let projected = render(&page_of(vec![block(body), block("later note")]));
    std::fs::write(&md_path, &projected).expect("write md");

    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");
    let hlc = HlcGenerator::new(actor);

    // First pass: the `.md` enters the log.
    outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("first reconcile");

    // Second pass over the same bytes must be a no-op. Before the fix
    // this emitted an `Op::Edit` that replaced the block's text with
    // its first line — the truncation, made permanent in the log.
    let again = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("second reconcile");
    assert_eq!(
        again.ops_applied, 0,
        "a reconcile over unchanged bytes must emit no ops"
    );

    let sidecar =
        outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)).expect("sidecar");
    let logged: Vec<&str> = sidecar.blocks.iter().map(|b| b.text.as_str()).collect();
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            logged.iter().any(|t| t.contains(line.trim())),
            "line {line:?} reached the .md but no block the log knows — logged: {logged:?}"
        );
    }
}

// ---------------------------------------------------------------------
// A block whose text *starts* with a newline — the fourth shape of the
// same defect, and the only one that freezes a page in both directions.
//
// `outl_actions::block::edit_text` writes a block's text verbatim, so
// `"\na"` is reachable from an ordinary paste. `render::write_block_text`
// emits it as `- \n  a\n`: a bullet declaring an empty first line, then
// one continuation. The parser used to read that back as the single-line
// block `"a"`, dropping the empty first line.
//
// That drop is worse than the three the module doc lists, because the
// line it loses is the empty string and the empty string is exactly what
// `unlogged::disk_line` maps the bare marker to. The page therefore
// reports one unlogged line — `""` — on every pass, `reconcile_md`
// withholds `last_synced_hash`, and the invariant-8 guard refuses to
// re-project. The documented recovery (`outl reconcile --ahead-of-log`)
// re-runs the same reconcile, emits zero ops, recomputes the same `[""]`
// and withholds again, so the page never thaws and the error message
// quotes the empty string at the user.
//
// The inverse the renderer defines is exact: `- ` (marker plus a space)
// declares a first line, a bare `-` does not.

#[test]
fn a_block_whose_text_starts_with_a_newline_survives_a_roundtrip() {
    assert_text_roundtrips("\na");
}

#[test]
fn a_block_text_that_is_all_leading_newlines_then_prose_survives() {
    assert_text_roundtrips("\n\na");
    assert_text_roundtrips("\n\n\nlate start");
}

#[test]
fn a_leading_newline_survives_alongside_the_shapes_that_follow_it() {
    assert_text_roundtrips("\nline one\nline two");
    assert_text_roundtrips("\nhead\n  indented detail");
    assert_text_roundtrips("\nfirst paragraph\n\nsecond paragraph");
}

/// An empty block is still an empty block: a bare `-` declares no first
/// line, so nothing may be prepended to whatever follows it.
#[test]
fn a_bare_dash_marker_still_means_an_empty_first_line_is_absent() {
    let back = parse("-\n  a\n");
    assert_eq!(back.blocks.len(), 1);
    assert_eq!(
        back.blocks[0].text, "a",
        "a bare `-` does not declare a first line"
    );
    assert_eq!(parse("-\n").blocks[0].text, "");
}

/// The consequence, on the pipeline that freezes: a page carrying a
/// leading-newline block must advance its hash like any other.
#[test]
fn a_leading_newline_block_does_not_freeze_its_page() {
    let body = "\na";
    let projected = render(&page_of(vec![block(body)]));
    assert_eq!(projected, "- \n  a\n", "the shape under test changed");

    let dir = tempfile::tempdir().expect("tempdir");
    let md_path = dir.path().join("p.md");
    std::fs::write(&md_path, &projected).expect("write md");

    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("workspace");
    let hlc = HlcGenerator::new(actor);

    let report = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");
    assert_eq!(
        report.unlogged_lines, 0,
        "the empty first line was reported as content the log lacks"
    );
    let sidecar =
        outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)).expect("sidecar");
    assert!(
        !sidecar.last_synced_hash.is_empty(),
        "the hash was withheld — the page is frozen in both directions"
    );

    let again = outl_md::reconcile_md(&mut ws, &hlc, &md_path, None).expect("second reconcile");
    assert_eq!(again.ops_applied, 0, "a second pass must be a no-op");
}

// ---------------------------------------------------------------------
// Two producer-side gaps of the same family: an input the dialect claims
// to accept, which `parse` cannot place, so the content is recovered as
// literal text and the block loses its identity.

/// CommonMark has **two** fence characters, and this dialect advertises
/// "fenced code", not "backtick-fenced code". `~~~` used to fall through
/// to the outline grammar, so a bullet inside the fence body became a
/// real block — content promoted into the tree, plus an
/// `UnrecognizedBlockMarker` on a line the user wrote correctly.
#[test]
fn a_tilde_fence_suspends_the_outline_grammar_like_a_backtick_fence() {
    let back = parse("- x\n  ~~~\n  - a\n  ~~~\n");
    assert_eq!(
        back.blocks.len(),
        1,
        "the fence body must not become a block"
    );
    assert!(
        back.blocks[0].children.is_empty(),
        "a bullet inside a fence is literal text, not a child: {:?}",
        back.blocks[0]
    );
    assert_eq!(back.blocks[0].text, "x\n~~~\n- a\n~~~");
    assert!(back.warnings.is_empty(), "no line here is unrecognised");
}

#[test]
fn a_tilde_fence_carries_an_info_string_and_roundtrips() {
    assert_text_roundtrips("intro\n~~~rust\nfn main() {}\n~~~");
    let once = render(&parse("- intro\n  ~~~rust\n  fn main() {}\n  ~~~\n"));
    assert_eq!(once, render(&parse(&once)), "not a fixpoint:\n{once}");
}

/// A fence closes only on its **own** character. The other one inside the
/// body is content, and reading it as a closer would end the fence early
/// and hand the rest of the body back to the outline grammar.
#[test]
fn a_fence_does_not_close_on_the_other_fence_character() {
    let back = parse("- x\n  ~~~\n  ```\n  - a\n  ~~~\n");
    assert_eq!(back.blocks.len(), 1);
    assert!(back.blocks[0].children.is_empty());
    assert_eq!(back.blocks[0].text, "x\n~~~\n```\n- a\n~~~");
}

/// A UTF-8 BOM is what a Windows editor leaves at the head of a file. It
/// is not whitespace, so it used to glue itself to the first `- ` and
/// make the first line unparseable as a bullet: the block was recovered
/// as verbatim text with the marker *inside* it, and the page's first
/// block lost its identity on import.
#[test]
fn a_utf8_bom_does_not_destroy_the_first_block() {
    let back = parse("\u{feff}- a\n- b\n");
    assert_eq!(back.blocks.len(), 2);
    assert_eq!(back.blocks[0].text, "a", "the BOM was folded into the text");
    assert_eq!(back.blocks[1].text, "b");
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(render(&back), "- a\n- b\n");
}

/// The BOM must not survive into a page property key either — the header
/// run reads the same first line.
#[test]
fn a_utf8_bom_does_not_destroy_a_leading_page_property() {
    let back = parse("\u{feff}title:: Notes\n\n- a\n");
    assert_eq!(
        back.properties,
        vec![("title".to_string(), "Notes".to_string())]
    );
    assert_eq!(back.blocks.len(), 1);
}
