//! Generated coverage for the `render → parse` roundtrip on a block's
//! **text**, the producer half of issue #210.
//!
//! `tests/multiline_block_roundtrip.rs` pins the four shapes that were
//! found in the wild, one test each. This file asks the same question of
//! shapes nobody enumerated: leading newlines, interior blank runs, a
//! continuation line's own indentation, tabs, unicode, fence markers, and
//! text that merely looks like outline syntax.
//!
//! ## Three properties, deliberately of different strengths
//!
//! 1. **`parse(render(text)) == text`**, over text the renderer can
//!    encode losslessly. Some text it cannot, and pretending otherwise
//!    would make this file assert a contract the crate does not have —
//!    see the generator's doc comment for the exact three exclusions and
//!    why each is a *convergent* normalisation rather than a loss.
//! 2. **`render → parse` is a fixpoint**, over *arbitrary* text including
//!    all three exclusions. A normalisation is only acceptable if it
//!    happens once; a document that changes shape on every save mutates
//!    the user's file forever and emits an `Op::Edit` per reconcile.
//! 3. **The unlogged-content check never cries wolf**, over arbitrary
//!    text. This is the property with teeth: it is the exact condition
//!    that freezes a page in both directions (invariant 8), and it is the
//!    one the `"\na"` defect violated.

use outl_core::id::NodeId;
use outl_md::parse::parse;
use outl_md::render::render;
use outl_md::sidecar::SidecarBlock;
use outl_md::{OutlineNode, ParsedPage};
use proptest::prelude::*;

fn page_of(text: &str) -> ParsedPage {
    ParsedPage {
        blocks: vec![OutlineNode {
            text: text.to_string(),
            properties: Vec::new(),
            children: Vec::new(),
        }],
        properties: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The sidecar a reconcile would write for `md`: one entry per parsed
/// block, text verbatim. Mirrors `tests/corpus_gate.rs`.
fn blocks_of(md: &str) -> Vec<SidecarBlock> {
    fn walk(nodes: &[OutlineNode], out: &mut Vec<SidecarBlock>) {
        for n in nodes {
            out.push(SidecarBlock::from_text(
                NodeId::new(),
                out.len() + 1,
                0,
                &n.text,
            ));
            walk(&n.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&parse(md).blocks, &mut out);
    out
}

/// A line of block text: prose, outline-looking syntax, unicode, or a
/// tab-bearing body.
///
/// Three kinds of line are held back, each because it makes the text
/// *change shape* rather than fail to survive, so asserting a roundtrip
/// over it would assert a contract the crate does not have. They are
/// exercised by [`arb_any_text`] against properties 2 and 3 instead:
///
/// - a bare fence marker, which opens a fence the text never closes; the
///   parser graceful-closes it (`consume_fence_until_close` synthesizes a
///   closer), so balanced fences are generated as a unit by
///   [`arb_fence_block`];
/// - a `- ` line, which splits the block in two — see the agent report;
/// - a `key:: value` line, which becomes a block *property*.
fn arb_line() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => "[a-z]{1,8}( [a-z]{1,8}){0,3}".prop_map(String::from),
        1 => Just("> quoted".to_string()),
        1 => Just("café — naïve ✅ 🚌".to_string()),
        1 => Just("x\u{2029}y".to_string()),
        1 => Just("mid\u{a0}nbsp".to_string()),
        1 => Just("end with\ttab".to_string()),
    ]
}

/// A closed fence, both CommonMark fence characters, with a body that
/// looks like outline syntax — the thing a fence exists to protect.
fn arb_fence_block() -> impl Strategy<Value = String> {
    (
        prop_oneof![Just("```"), Just("~~~")],
        prop_oneof![Just(""), Just("rust"), Just("yaml")],
        proptest::collection::vec(
            prop_oneof![
                2 => "[a-z0-9 ()+=*/-]{0,20}".prop_map(String::from),
                1 => "- [a-z]{1,6}".prop_map(String::from),
                1 => "[a-z]{1,6}:: [a-z]{1,6}".prop_map(String::from),
            ],
            0..4,
        ),
    )
        .prop_map(|(marker, info, body)| {
            let mut out = format!("{marker}{info}");
            for line in body {
                out.push('\n');
                out.push_str(line.trim_end());
            }
            out.push('\n');
            out.push_str(marker);
            out
        })
}

/// A continuation line, which unlike the first line may carry its own
/// leading indentation — `render::write_block_text` writes the levels it
/// added and `parse::strip_indent_levels` takes back exactly those, so
/// whatever is left is the user's.
fn arb_continuation() -> impl Strategy<Value = String> {
    (
        prop_oneof![
            4 => Just(""),
            2 => Just("  "),
            1 => Just("    "),
            1 => Just("\t"),
        ],
        arb_line(),
    )
        .prop_map(|(pad, body)| format!("{pad}{body}"))
}

/// Block text the renderer can encode without normalising anything.
///
/// Three shapes are deliberately excluded, because the crate normalises
/// them rather than preserving them. Each is a **convergent** rewrite —
/// it happens once, costs one `Op::Edit`, and cannot freeze a page
/// (property 3 below covers all three, and holds) — so excluding them
/// here states the contract honestly instead of asserting one the crate
/// does not have:
///
/// - **Leading whitespace on a line, where the indent machinery cannot
///   account for it as a whole level.** On the first line
///   `strip_block_marker` trims after the `- `, so `"\tfoo"` reads back
///   as `"foo"`. On a continuation line, two or more spaces and a tab all
///   survive (`leading_indent` sees another level, and
///   `strip_indent_levels` gives back exactly what the renderer added),
///   but a single space or a Unicode space that `leading_indent` does not
///   count — U+00A0, say — leaves the line at exactly `indent + 1`, where
///   the branch that claims it trims instead of stripping levels. Hence
///   `arb_continuation` pads only with whole levels.
/// - **A carriage return.** `str::lines` and `trim` both treat it as
///   whitespace, so it never reaches the AST.
/// - **Trailing blank lines.** Held in `pending_blanks` and dropped if no
///   continuation follows — deliberate, and pinned by
///   `a_trailing_blank_line_does_not_end_up_in_the_blocks_text`.
///
/// Everything else round-trips exactly, *including* a leading blank run:
/// `- ` (marker, space, nothing) is how the renderer writes "this text
/// starts with a newline", and the parser reads it back as exactly that.
fn arb_block_text() -> impl Strategy<Value = String> {
    (
        // Leading blank lines — the shape the `"\na"` defect lost.
        0usize..3,
        arb_line(),
        proptest::collection::vec(
            prop_oneof![
                6 => arb_continuation(),
                2 => Just(String::new()),
                3 => arb_fence_block(),
            ],
            0..5,
        ),
    )
        .prop_map(|(leading, first, rest)| {
            let mut lines: Vec<String> = vec![String::new(); leading];
            lines.push(first);
            lines.extend(rest);
            // No trailing blank run: it is dropped by design.
            while lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
            lines.join("\n")
        })
        // A first line that is only whitespace is normalised away, and an
        // all-blank text has nothing left to assert.
        .prop_filter("no unaccountable leading whitespace", |t| {
            let mut lines = t.split('\n').skip_while(|l| l.is_empty());
            let first_is_clean = lines
                .next()
                .is_some_and(|l| !l.starts_with(char::is_whitespace));
            // A continuation line may be indented, but only by whole
            // levels the renderer can give back — never by a space the
            // indent machinery does not count as one.
            let rest_is_clean = lines.all(|l| {
                let body = l.trim_start_matches([' ', '\t']);
                !body.starts_with(char::is_whitespace)
            });
            first_is_clean && rest_is_clean && !t.trim().is_empty()
        })
}

/// Arbitrary text, every exclusion above included, for the two properties
/// that must hold for anything a user can type.
///
/// The normalising shapes are named literals rather than generated,
/// because one combination of them is a **known, unfixed** non-fixpoint
/// and generating it would mean asserting it passes. A fence marker
/// stranded after a child block (`- a\n  - a\n  ```\n  ```\n`) is
/// recovered as two verbatim child blocks whose text is a bare fence
/// marker; each re-opens a fence on the next parse, so the document
/// settles on the **third** save rather than the first. It is bounded
/// (pass 3 is stable), it predates the `"\na"` fix, and it occurs in 0
/// of the 2,856 `.md` files of the reporting workspace — so it is
/// reported rather than fixed here, since deciding what a stranded fence
/// after a child block *means* is a grammar change, not a bug fix.
fn arb_any_text() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => arb_block_text(),
        1 => Just("\tleading tab".to_string()),
        1 => Just("has\rcarriage".to_string()),
        1 => Just("trailing blanks\n\n".to_string()),
        1 => Just("\n".to_string()),
        1 => Just(String::new()),
        1 => Just("  spaced first line".to_string()),
        1 => Just("one space\n indented".to_string()),
        1 => Just("\u{a0}nbsp".to_string()),
        1 => Just("a\n\u{a0}nbsp".to_string()),
        1 => Just("a\n- a".to_string()),
        1 => Just("a\n- a\nb".to_string()),
        1 => Just("\n- a".to_string()),
        1 => Just("a\nkey:: v".to_string()),
        1 => Just("a\n```\nunclosed".to_string()),
        1 => Just("~~~\n- a\n~~~".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Property 1. The renderer and the parser are inverses.
    #[test]
    fn block_text_survives_render_then_parse(text in arb_block_text()) {
        let rendered = render(&page_of(&text));
        let back = parse(&rendered);
        prop_assert_eq!(
            back.blocks.len(),
            1,
            "expected one block back, got {} — rendered:\n{}",
            back.blocks.len(),
            rendered
        );
        prop_assert_eq!(
            &back.blocks[0].text,
            &text,
            "text did not survive render → parse — rendered:\n{}",
            rendered
        );
    }

    /// Property 2. Whatever the pipeline normalises, it normalises once.
    ///
    /// Stated from the *document*, the way `tests/corpus_gate.rs` states
    /// it, so the pipeline keeps its right to normalise on the first pass
    /// (an unclosed fence gets a synthetic close; a carriage return and a
    /// trailing blank run are dropped) while giving up the right to keep
    /// doing it. A document that changes on every save mutates the user's
    /// file forever and emits an `Op::Edit` per reconcile — worse than
    /// the bug that shape replaced, which at least converged.
    #[test]
    fn render_then_parse_is_a_fixpoint_for_any_block_text(text in arb_any_text()) {
        let src = render(&page_of(&text));
        let once = render(&parse(&src));
        let twice = render(&parse(&once));
        prop_assert_eq!(
            &once,
            &twice,
            "the document changes shape on every save\n--- source ---\n{}",
            src
        );
    }

    /// Property 3. The one with teeth: a page whose content the log fully
    /// accounts for must never be reported as holding unlogged lines.
    ///
    /// That verdict withholds `last_synced_hash` and refuses
    /// re-projection, so a false positive freezes the page in both
    /// directions — and `outl reconcile --ahead-of-log`, the documented
    /// recovery, re-runs this same computation and re-withholds.
    #[test]
    fn a_rendered_page_never_reports_content_its_own_log_holds(text in arb_any_text()) {
        let rendered = render(&page_of(&text));
        let missing = outl_md::unlogged::content_lines_missing_from(
            &rendered,
            &blocks_of(&rendered),
        );
        prop_assert!(
            missing.is_empty(),
            "reported {} line(s) as unlogged that the log does hold, \
             freezing the page: {:?}\n--- rendered ---\n{}",
            missing.len(),
            missing,
            rendered
        );
    }
}

/// The deterministic pin for the shape proptest found on a later seed,
/// so it survives seed rotation and cannot be lost with the regressions
/// file.
///
/// A `- ` line **inside a fenced block** is content, held in the block's
/// text with its marker. `unlogged::disk_line` strips the marker from
/// every indented line, so this line offers two readings — `"- j"` and
/// `"j"` — and `known` is a **multiset that decrements**. Trying the
/// stripped reading first did not merely fail to match: it spent the
/// single `j` the log held for the *last* line, which then matched
/// nothing. A page whose every line came out of the op log reported one
/// unlogged line, and that withholds `last_synced_hash` and refuses
/// re-projection — the `"\na"` freeze reached from the opposite side.
///
/// Two things make this worth its own test rather than a property seed.
/// The producer is **exonerated by construction** — the roundtrip
/// assertion below is exact, so no parse/render/fence change could have
/// caused or fixed it — and the bug is *positional*: changing the last
/// line to anything that does not collide (`"…\n```\nk"`) makes it pass,
/// which is why a single hand-written fence case never caught it.
///
/// Fixed in `unlogged.rs` by trying the verbatim reading first for an
/// indented line (its `a_fenced_bullet_does_not_steal_a_later_lines_match`
/// pins the same defect at the unit level). This test is the end-to-end
/// half: a rendered page, through the real guard.
#[test]
fn a_fenced_bullet_does_not_steal_a_later_plain_lines_match() {
    let text = "a\n```\n- j\n```\nj";
    let rendered = render(&page_of(text));
    assert_eq!(rendered, "- a\n  ```\n  - j\n  ```\n  j\n");

    // The producer is clean: this is an exact roundtrip, so the `.md`
    // holds precisely what the log holds and nothing else.
    let back = parse(&rendered);
    assert_eq!(back.blocks.len(), 1);
    assert_eq!(back.blocks[0].text, text, "producer is not at fault");

    let missing = outl_md::unlogged::content_lines_missing_from(&rendered, &blocks_of(&rendered));
    assert!(
        missing.is_empty(),
        "a faithful .md reported {missing:?} as unlogged — the page is frozen \
         in both directions and `reconcile --ahead-of-log` re-withholds"
    );

    // The near-miss that made the bug positional: one different
    // character on the last line and it always passed.
    let near = "a\n```\n- j\n```\nk";
    let r2 = render(&page_of(near));
    assert!(outl_md::unlogged::content_lines_missing_from(&r2, &blocks_of(&r2)).is_empty());
}
