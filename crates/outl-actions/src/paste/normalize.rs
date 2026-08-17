//! External markdown syntax → outl syntax.
//!
//! Pure string-level transforms applied to clipboard text before it
//! reaches `outl_md::parse::parse`. Reads no workspace, writes no
//! storage — split out from [`super`] so the conversion logic is
//! easy to test and easy to extend without dragging in the workspace
//! plumbing.
//!
//! See the table in `super::mod` for the full conversion catalog.

/// Apply every known external-syntax conversion and strip unknown
/// tokens. The pipeline runs in a fixed order — the more specific
/// conversions must happen before the generic `{{…}}` strip.
pub fn normalize_external_syntax(raw: &str) -> String {
    // 1. Newlines.
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");

    // 2. Indent 4 → 2 when every indented line is a multiple of 4 and
    //    the smallest non-zero leading-space count is 4.
    let lines: Vec<&str> = raw.lines().collect();
    let indent_unit = detect_indent_unit(&lines);
    let normalized_indent: Vec<String> = if indent_unit == 4 {
        lines.iter().map(|l| renormalize_indent_4_to_2(l)).collect()
    } else {
        lines.iter().map(|l| (*l).to_string()).collect()
    };

    // 3 + 4 + 5: per-line text-level conversions.
    let mut kept: Vec<String> = Vec::with_capacity(normalized_indent.len());
    for line in normalized_indent {
        if is_logseq_id_line(&line) {
            // Drop Logseq's inline `id::` lines — outl forbids ids in
            // markdown (root CLAUDE.md invariant #2).
            continue;
        }
        kept.push(convert_line(&line));
    }

    // 6. Strip remaining {{…}} and ^^…^^ tokens. After the more
    //    specific replacements had a chance to consume the tokens
    //    they understand, anything still wrapped is unknown to outl.
    let stripped: Vec<String> = kept.into_iter().map(strip_unknown_tokens).collect();

    // 7. Collapse double spaces left over by the strip, preserving
    //    the leading indent so structural meaning isn't lost.
    let collapsed: Vec<String> = stripped.into_iter().map(collapse_inner_spaces).collect();

    collapsed.join("\n")
}

fn detect_indent_unit(lines: &[&str]) -> usize {
    let mut min_nonzero: usize = usize::MAX;
    let mut all_multiples_of_4 = true;
    for line in lines {
        let spaces = line.chars().take_while(|c| *c == ' ').count();
        if spaces == 0 {
            continue;
        }
        if spaces % 4 != 0 {
            all_multiples_of_4 = false;
        }
        if spaces < min_nonzero {
            min_nonzero = spaces;
        }
    }
    if min_nonzero == 4 && all_multiples_of_4 {
        4
    } else {
        2
    }
}

fn renormalize_indent_4_to_2(line: &str) -> String {
    let spaces = line.chars().take_while(|c| *c == ' ').count();
    let levels = spaces / 4;
    let rest = &line[spaces..];
    let mut out = String::with_capacity(levels * 2 + rest.len());
    for _ in 0..levels {
        out.push_str("  ");
    }
    out.push_str(rest);
    out
}

fn convert_line(line: &str) -> String {
    let indent_len = line.chars().take_while(|c| *c == ' ').count();
    let indent: String = " ".repeat(indent_len);
    let rest = &line[indent_len..];

    // GitHub task list `[ ]` / `[x]` only at the start of a bullet
    // line. `[/]` is the in-progress box Logseq and Obsidian's tasks
    // plugin both write.
    let rest = if let Some(after) = rest.strip_prefix("- [ ] ") {
        format!("- TODO {after}")
    } else if let Some(after) = rest.strip_prefix("- [/] ") {
        format!("- DOING {after}")
    } else if let Some(after) = rest
        .strip_prefix("- [x] ")
        .or_else(|| rest.strip_prefix("- [X] "))
    {
        format!("- DONE {after}")
    } else {
        rest.to_string()
    };

    // Roam tokens, applied anywhere in the line.
    let rest = rest.replace("{{[[TODO]]}} ", "TODO ");
    let rest = rest.replace("{{[[TODO]]}}", "TODO");
    let rest = rest.replace("{{[[DOING]]}} ", "DOING ");
    let rest = rest.replace("{{[[DOING]]}}", "DOING");
    let rest = rest.replace("{{[[DONE]]}} ", "DONE ");
    let rest = rest.replace("{{[[DONE]]}}", "DONE");

    // Roam embed: {{embed: ((blk-XXXXXX))}} → !((blk-XXXXXX))
    let rest = rewrite_roam_embed(&rest);

    // Roam query: {{[[query]]: foo}} → {{query: foo}}
    let rest = rest.replace("{{[[query]]:", "{{query:");

    // Page refs `[[...]]` whose inner content is a non-ISO date
    // (Roam's "June 2nd, 2026") get normalised to outl's ISO journal
    // slug. Plain page refs (`[[Avelino]]`) stay verbatim.
    let rest = rewrite_date_refs(&rest);

    // Roam highlight `^^text^^` → outl `==text==` (a native inline
    // token now). Runs before the unknown-token strip so a balanced
    // pair survives as a highlight instead of being deleted.
    let rest = rewrite_highlight(&rest);

    format!("{indent}{rest}")
}

/// Roam `^^highlight^^` → outl `==highlight==`. A balanced, single-line
/// pair whose inner has non-space content is rewritten, and the inner is
/// **trimmed** first: the tokenizer rejects a highlight with a space next
/// to either marker, so `^^ foo ^^` becomes `==foo==` (which renders),
/// not `== foo ==` (which wouldn't). A space-only or empty pair is left
/// for [`strip_unknown_tokens`]; a lone unbalanced `^^` has no pair, so
/// neither pass touches it and it survives verbatim.
fn rewrite_highlight(s: &str) -> String {
    if !s.contains("^^") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    while let Some(start) = s[cursor..].find("^^") {
        let abs = cursor + start;
        out.push_str(&s[cursor..abs]);
        let after_open = abs + 2;
        let Some(close_rel) = s[after_open..].find("^^") else {
            out.push_str(&s[abs..]);
            return out;
        };
        let close_pos = after_open + close_rel;
        let inner = &s[after_open..close_pos];
        let trimmed = inner.trim();
        if trimmed.is_empty() || inner.contains('\n') {
            out.push_str(&s[abs..close_pos + 2]);
        } else {
            out.push_str("==");
            out.push_str(trimmed);
            out.push_str("==");
        }
        cursor = close_pos + 2;
    }
    out.push_str(&s[cursor..]);
    out
}

/// Scan for `[[...]]` page refs and rewrite the inner text when it
/// parses as a date (via [`crate::dates::parse_date_label`], the
/// workspace-wide owner of flexible date parsing). Unrecognised forms
/// (plain page names, already ISO refs) pass through untouched.
fn rewrite_date_refs(s: &str) -> String {
    const OPEN: &str = "[[";
    const CLOSE: &str = "]]";
    if !s.contains(OPEN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    while let Some(start) = s[cursor..].find(OPEN) {
        let abs_open = cursor + start;
        out.push_str(&s[cursor..abs_open]);
        let after_open = abs_open + OPEN.len();
        if let Some(close_rel) = s[after_open..].find(CLOSE) {
            let close = after_open + close_rel;
            let inner = &s[after_open..close];
            let rewritten =
                crate::dates::parse_date_label(inner).unwrap_or_else(|| inner.to_string());
            out.push_str(OPEN);
            out.push_str(&rewritten);
            out.push_str(CLOSE);
            cursor = close + CLOSE.len();
        } else {
            // Unbalanced — keep the rest verbatim and stop.
            out.push_str(&s[abs_open..]);
            return out;
        }
    }
    out.push_str(&s[cursor..]);
    out
}

/// Replace every occurrence of `{{embed: ((blk-XXXXXX))}}` with
/// `!((blk-XXXXXX))`. Tolerates whitespace around the inner ref.
fn rewrite_roam_embed(s: &str) -> String {
    const NEEDLE: &str = "{{embed:";
    if !s.contains(NEEDLE) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    while let Some(start) = s[cursor..].find(NEEDLE) {
        let abs_start = cursor + start;
        out.push_str(&s[cursor..abs_start]);
        let after_open = abs_start + NEEDLE.len();
        // Locate the matching `}}` for this opener.
        if let Some(close_rel) = s[after_open..].find("}}") {
            let close = after_open + close_rel;
            let inner = s[after_open..close].trim();
            if let Some(inner_ref) = extract_block_ref(inner) {
                out.push('!');
                out.push_str(inner_ref);
                cursor = close + 2;
                continue;
            }
        }
        // Couldn't parse — leave the literal alone, advance past `{{`.
        out.push_str(&s[abs_start..abs_start + 2]);
        cursor = abs_start + 2;
    }
    out.push_str(&s[cursor..]);
    out
}

/// Extract `((blk-XXXXXX))` from a string that contains exactly that
/// token (possibly with surrounding whitespace). Returns the matched
/// slice so the caller can paste it back verbatim.
fn extract_block_ref(s: &str) -> Option<&str> {
    let s = s.trim();
    if !s.starts_with("((blk-") || !s.ends_with("))") {
        return None;
    }
    Some(s)
}

/// True for lines that look like `id:: 01HXY8KJZQ9T8M7VN3P2R6S4A1`
/// (Crockford-base32 ULID, 26 chars).
///
/// We validate the alphabet strictly so 26 random alphanumeric
/// characters don't get dropped by accident. Crockford base32 uses
/// `0-9` plus `A-Z` minus `I`, `L`, `O`, `U` (also accepts lowercase
/// in modern ULID libs, so we case-fold here).
fn is_logseq_id_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(after) = trimmed.strip_prefix("id:: ") else {
        return false;
    };
    after.len() == 26 && after.chars().all(is_crockford_base32_char)
}

fn is_crockford_base32_char(c: char) -> bool {
    let c = c.to_ascii_uppercase();
    if c.is_ascii_digit() {
        return true;
    }
    if !c.is_ascii_uppercase() {
        return false;
    }
    !matches!(c, 'I' | 'L' | 'O' | 'U')
}

/// Remove every remaining `{{…}}` and `^^…^^` token. Runs after the
/// known conversions above, so anything still wearing these wrappers
/// is by definition unknown to outl.
///
/// The `{{...}}` pass keeps outl-native forms intact:
///
/// - `{{query: ...}}` — saved-query token (parsed by `outl_md`).
fn strip_unknown_tokens(line: String) -> String {
    let line = strip_pair(&line, "{{", "}}", is_unknown_braces_token);
    strip_pair(&line, "^^", "^^", |_| true)
}

/// True when the inner body of a `{{…}}` token is *not* a recognised
/// outl construct and should be deleted.
fn is_unknown_braces_token(inner: &str) -> bool {
    let trimmed = inner.trim_start();
    !trimmed.starts_with("query:") && !trimmed.starts_with("query ")
}

/// Strip every `open … close` pair from `s` for which
/// `should_strip(inner)` returns `true`. Pairs that the filter rejects
/// are copied through verbatim, opener and closer included.
fn strip_pair(s: &str, open: &str, close: &str, should_strip: impl Fn(&str) -> bool) -> String {
    if !s.contains(open) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    while let Some(start) = s[cursor..].find(open) {
        let abs = cursor + start;
        out.push_str(&s[cursor..abs]);
        let after_open = abs + open.len();
        if let Some(close_rel) = s[after_open..].find(close) {
            let close_pos = after_open + close_rel;
            let inner = &s[after_open..close_pos];
            if should_strip(inner) {
                cursor = close_pos + close.len();
            } else {
                // Keep the token verbatim, advance past its closer.
                out.push_str(&s[abs..close_pos + close.len()]);
                cursor = close_pos + close.len();
            }
        } else {
            // No closer — append the rest verbatim so we don't blow
            // up on unbalanced text.
            out.push_str(&s[abs..]);
            return out;
        }
    }
    out.push_str(&s[cursor..]);
    out
}

/// Collapse runs of two or more spaces in the body of `line` to a
/// single space. Preserves the leading indent so the parser's
/// indent-based hierarchy isn't garbled by a strip in the body.
///
/// Trailing spaces survive on purpose: the body of `normalize_external_syntax`
/// is also fed to plain-text paste (`splice into block at caret`),
/// and a user who pasted `"BRAVE "` (with a trailing space) expects
/// that space to land in the splice.
fn collapse_inner_spaces(line: String) -> String {
    let indent_len = line.chars().take_while(|c| *c == ' ').count();
    if line.len() == indent_len {
        return line;
    }
    let (indent, body) = line.split_at(indent_len);
    let mut out = String::with_capacity(line.len());
    out.push_str(indent);
    let mut prev_space = false;
    for c in body.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
}
