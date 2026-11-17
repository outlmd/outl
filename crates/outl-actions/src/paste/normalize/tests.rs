//! The normaliser's battery: one test per row of the conversion table
//! in [`super::super`], plus the order-of-operations assertions that
//! table's rule 5 requires.
//!
//! Split out of `normalize.rs` to keep the scan itself one piece — the
//! same seam `outl_md::parse` / `parse/tests.rs` uses. The pipeline is a
//! fixed order of passes and a seam inside it would be a lie about how
//! it runs; the tests are not part of that order.

use super::*;

#[test]
fn roam_todo_becomes_prefix() {
    let out = normalize_external_syntax("- {{[[TODO]]}} foo");
    assert_eq!(out, "- TODO foo");
}

#[test]
fn roam_done_becomes_prefix() {
    let out = normalize_external_syntax("- {{[[DONE]]}} bar");
    assert_eq!(out, "- DONE bar");
    let out = normalize_external_syntax("- {{[[DOING]]}} baz");
    assert_eq!(out, "- DOING baz");
}

#[test]
fn github_checkbox_becomes_todo() {
    assert_eq!(normalize_external_syntax("- [ ] foo"), "- TODO foo");
    assert_eq!(normalize_external_syntax("- [/] foo"), "- DOING foo");
    assert_eq!(normalize_external_syntax("- [x] foo"), "- DONE foo");
    assert_eq!(normalize_external_syntax("- [X] foo"), "- DONE foo");
}

#[test]
fn roam_embed_becomes_outl_embed() {
    let out = normalize_external_syntax("see {{embed: ((blk-r6s4a1))}} here");
    assert_eq!(out, "see !((blk-r6s4a1)) here");
}

/// A pasted page embed used to be **deleted**, not left literal: the
/// token reached `strip_unknown_tokens` and the page name went with
/// it. `![[Page]]` is a target for it now (issue #190).
#[test]
fn a_pasted_page_embed_becomes_an_outl_page_embed() {
    // Roam, bracketed (what the app actually writes).
    assert_eq!(
        normalize_external_syntax("- see {{[[embed]]: [[Other Page]]}} here"),
        "- see ![[Other Page]] here"
    );
    // Roam, bare.
    assert_eq!(
        normalize_external_syntax("- {{embed: [[Other Page]]}}"),
        "- ![[Other Page]]"
    );
    // Logseq, space-shaped.
    assert_eq!(
        normalize_external_syntax("- {{embed [[Other Page]]}}"),
        "- ![[Other Page]]"
    );
}

/// The conversion has to beat the generic `{{…}}` strip, which is
/// what deleted it before. Same assertion shape as
/// `known_token_wins_over_generic_strip`.
#[test]
fn the_page_embed_conversion_wins_over_the_generic_strip() {
    let out = normalize_external_syntax("- {{embed [[Roadmap]]}}");
    assert!(out.contains("Roadmap"), "page name must survive: {out}");
    assert_eq!(out, "- ![[Roadmap]]");
}

/// Two embeds on one line, and one of each kind: a wrong opener
/// length on the first corrupts the second.
#[test]
fn several_embeds_on_one_line_all_convert() {
    assert_eq!(
        normalize_external_syntax("a {{embed [[One]]}} b {{embed: ((blk-r6s4a1))}} c"),
        "a ![[One]] b !((blk-r6s4a1)) c"
    );
}

/// `rewrite_embed` runs before `rewrite_date_refs`, so a page embed
/// of a Roam-dated journal gets the same ISO normalisation every
/// other `[[…]]` gets. The `!` sits outside the brackets that pass
/// scans, so it survives.
#[test]
fn a_page_embed_of_a_roam_date_is_normalised_to_iso() {
    assert_eq!(
        normalize_external_syntax("- {{embed [[June 2nd, 2026]]}}"),
        "- ![[2026-06-02]]"
    );
}

/// A target outl cannot name keeps this pass's standing behaviour —
/// the literal is left for the strip. A source uuid is not an outl
/// handle and the paste path has no graph to resolve it against.
#[test]
fn an_embed_of_a_foreign_uuid_is_still_not_translatable() {
    assert_eq!(normalize_external_syntax("- {{embed ((CLGp3LXxM))}}"), "- ");
}

/// `![[photo.png]]` is the image-embed spelling, which the tokenizer
/// refuses as a page embed — so the name is kept as a plain ref
/// rather than written in a form that reads back as a `!` in front of
/// a link.
#[test]
fn an_image_named_target_falls_back_to_a_plain_ref() {
    assert_eq!(
        normalize_external_syntax("- {{embed [[photo.png]]}}"),
        "- [[photo.png]]"
    );
}

#[test]
fn a_malformed_embed_does_not_panic_or_eat_the_line() {
    // No closer.
    assert_eq!(
        normalize_external_syntax("- {{embed [[One]]"),
        "- {{embed [[One]]"
    );
    // Empty target.
    assert_eq!(normalize_external_syntax("- {{embed [[]]}}"), "- ");
}

#[test]
fn roam_query_becomes_outl_query() {
    let out = normalize_external_syntax("- {{[[query]]: TODO}}");
    assert_eq!(out, "- {{query: TODO}}");
}

#[test]
fn logseq_id_line_is_dropped() {
    let input = "- header\n  id:: 01HXY8KJZQ9T8M7VN3P2R6S4A1\n  - child";
    let out = normalize_external_syntax(input);
    assert_eq!(out, "- header\n  - child");
}

#[test]
fn id_line_with_non_crockford_chars_survives() {
    // Looks 26-char and alphanumeric but uses banned letters
    // (`I`, `L`) — not a real ULID, must NOT be dropped.
    let input = "- header\n  id:: IIIILLLLOOOO0000000000000A\n  - child";
    let out = normalize_external_syntax(input);
    assert!(out.contains("IIIILLLLOOOO"), "non-ULID id:: stays: {out}");
}

#[test]
fn id_line_with_wrong_length_survives() {
    // 27 chars — wrong length, not a ULID.
    let input = "- header\n  id:: 01HXY8KJZQ9T8M7VN3P2R6S4A1X\n  - child";
    let out = normalize_external_syntax(input);
    assert!(
        out.contains("01HXY8KJZQ9T8M7VN3P2R6S4A1X"),
        "long id:: stays: {out}"
    );
}

#[test]
fn unknown_tokens_are_stripped() {
    // {{video: ...}} not in the known set → stripped.
    let out = normalize_external_syntax("- check {{video: https://x.test}} this");
    assert_eq!(out, "- check this");
    // A balanced `^^highlight^^` now converts to native `==…==`
    // instead of being stripped (see roam_highlight_becomes_double_equals).
    let out = normalize_external_syntax("- ^^hot take^^ here");
    assert_eq!(out, "- ==hot take== here");
}

#[test]
fn indent_4_normalizes_to_2() {
    let input = "- parent\n    - child\n        - grand";
    let out = normalize_external_syntax(input);
    assert_eq!(out, "- parent\n  - child\n    - grand");
}

#[test]
fn indent_2_stays_unchanged() {
    let input = "- parent\n  - child";
    let out = normalize_external_syntax(input);
    assert_eq!(out, "- parent\n  - child");
}

#[test]
fn known_token_wins_over_generic_strip() {
    // The {{[[TODO]]}} conversion must run before the generic
    // {{...}} strip, otherwise TODO becomes empty.
    let out = normalize_external_syntax("- {{[[TODO]]}} review");
    assert_eq!(out, "- TODO review");
}

#[test]
fn roam_highlight_becomes_double_equals() {
    // Balanced `^^…^^` converts to the native `==…==`.
    assert_eq!(
        normalize_external_syntax("- mark ^^this^^ well"),
        "- mark ==this== well"
    );
    // A single unpaired `^^` has no closing marker, so it stays
    // verbatim (the strip pass only removes balanced pairs).
    assert_eq!(
        normalize_external_syntax("- a ^^ dangling"),
        "- a ^^ dangling"
    );
    // Spaces adjacent to the markers are trimmed so the result is a
    // real highlight the tokenizer accepts, not inert `== foo ==`.
    assert_eq!(
        normalize_external_syntax("- mark ^^ this ^^ well"),
        "- mark ==this== well"
    );
}

#[test]
fn roam_long_date_becomes_iso() {
    assert_eq!(
        normalize_external_syntax("- check [[June 2nd, 2026]]"),
        "- check [[2026-06-02]]",
    );
    assert_eq!(
        normalize_external_syntax("- [[January 1st, 2025]]"),
        "- [[2025-01-01]]",
    );
    assert_eq!(
        normalize_external_syntax("- [[December 31st, 2025]]"),
        "- [[2025-12-31]]",
    );
    assert_eq!(
        normalize_external_syntax("- meet [[April 22nd, 2026]] again"),
        "- meet [[2026-04-22]] again",
    );
}

#[test]
fn short_month_long_date_becomes_iso() {
    assert_eq!(
        normalize_external_syntax("- [[Apr 22nd, 2026]]"),
        "- [[2026-04-22]]",
    );
    assert_eq!(
        normalize_external_syntax("- [[Sep 3rd, 2025]]"),
        "- [[2025-09-03]]",
    );
}

#[test]
fn slashed_date_becomes_iso() {
    assert_eq!(
        normalize_external_syntax("- [[2026/04/22]]"),
        "- [[2026-04-22]]",
    );
}

#[test]
fn iso_date_is_left_alone() {
    assert_eq!(
        normalize_external_syntax("- [[2026-06-02]]"),
        "- [[2026-06-02]]",
    );
}

#[test]
fn plain_page_ref_is_left_alone() {
    assert_eq!(
        normalize_external_syntax("- ping [[Avelino]] today"),
        "- ping [[Avelino]] today",
    );
    // Looks date-ish but isn't: missing year, leave verbatim.
    assert_eq!(
        normalize_external_syntax("- [[June 2nd]]"),
        "- [[June 2nd]]",
    );
}

#[test]
fn unbalanced_brackets_dont_panic() {
    // No closer — passthrough.
    assert_eq!(
        normalize_external_syntax("- before [[unclosed"),
        "- before [[unclosed",
    );
}
