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

    // 3 - 7: per-line text-level conversions, on every line that is
    //    not inside a fenced code block. A fence is code the user
    //    copied: `{{ x }}` there is a template, `- [ ]` is a string
    //    literal, and an `id:: …` line is data. The indent pass above
    //    still runs on it, because it is structure (where the fence
    //    sits in the outline), not content.
    let mut fence = FenceState::default();
    let mut kept: Vec<String> = Vec::with_capacity(normalized_indent.len());
    for line in normalized_indent {
        if fence.verbatim(&line) {
            kept.push(line);
            continue;
        }
        if is_logseq_id_line(&line) {
            // Drop Logseq's inline `id::` lines — outl forbids ids in
            // markdown (root CLAUDE.md invariant #2).
            continue;
        }
        kept.push(normalize_line(&line));
    }

    kept.join("\n")
}

/// Steps 3 - 7 for one line outside a fence.
fn normalize_line(line: &str) -> String {
    // 3 + 4 + 5: known conversions.
    let converted = convert_line(line);
    // 6. Strip remaining {{…}} and ^^…^^ tokens. After the more
    //    specific replacements had a chance to consume the tokens
    //    they understand, anything still wrapped is unknown to outl.
    let stripped = strip_unknown_tokens(converted);
    // 7. Collapse double spaces left over by the strip, preserving
    //    the leading indent so structural meaning isn't lost.
    collapse_inner_spaces(stripped)
}

/// Where the scan stands relative to a fenced code block.
///
/// The same rules `outl_md`'s parser and its `unlogged::track_fence`
/// apply, over `outl_md::fence_marker` and `parse::leading_indent` (the
/// tracker itself is crate-private, and this pass runs on text that has
/// not been parsed yet): an opener is ```` ``` ```` or `~~~` at the start
/// of a line or right after its `- `; the fence closes on a line of its
/// **own** marker character at **exactly** the body's level (a deeper
/// one is content, e.g. a code sample quoting a nested fence), or on a
/// non-blank line outdented below the body. Levels are counted the
/// parser's way: two spaces or one tab each. An opener with no bullet at
/// level 0 starts a block, so its body is one level in, like a bullet's;
/// otherwise a stray closer at column 0 would hold every later line
/// verbatim. A fence never closed runs until something outdents below
/// its body, which is what the parser does with it too.
#[derive(Default)]
struct FenceState {
    /// The opener's marker and the level its body sits at.
    open: Option<(&'static str, usize)>,
}

impl FenceState {
    /// Advance over `line` and say whether it passes through untouched:
    /// an opener, a body line or a closer.
    fn verbatim(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        let level = outl_md::parse::leading_indent(line);
        if let Some((marker, body_level)) = self.open {
            if trimmed.is_empty() {
                return true;
            }
            if level >= body_level {
                if level == body_level && closes(trimmed, marker) {
                    self.open = None;
                }
                return true;
            }
            self.open = None;
        }
        let (content, body_indent) = match trimmed.strip_prefix("- ") {
            Some(rest) => (rest.trim_start(), level + 1),
            None => (trimmed, level.max(1)),
        };
        let Some(marker) = outl_md::fence_marker(content) else {
            return false;
        };
        self.open = Some((marker, body_indent));
        true
    }
}

/// Whether `trimmed` is a closing fence for `marker`: the marker, then
/// nothing but more of its own character.
fn closes(trimmed: &str, marker: &str) -> bool {
    let fence_char = marker.chars().next().unwrap_or('`');
    trimmed.starts_with(marker) && trimmed.chars().all(|c| c == fence_char)
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

    // Roam / Logseq embed: `{{embed: ((blk-XXXXXX))}}` → `!((blk-XXXXXX))`,
    // `{{embed [[Page]]}}` → `![[Page]]`. Runs before `rewrite_date_refs`
    // on purpose, so a page embed of a Roam-dated journal
    // (`{{embed [[June 2nd, 2026]]}}`) gets its ISO rewrite from that pass
    // like any other `[[…]]` — the `!` is outside the brackets it scans.
    let rest = rewrite_embed(&rest);

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

/// Every spelling Roam and Logseq write a `{{embed}}` with.
///
/// Roam's is colon-shaped and comes in a bare and a bracketed form;
/// Logseq's is space-shaped. None of the three shares a prefix with
/// another, so the scan simply takes whichever appears first.
const EMBED_OPENERS: [&str; 3] = ["{{[[embed]]:", "{{embed:", "{{embed "];

/// Translate every `{{embed …}}` whose target outl can name:
/// `((blk-XXXXXX))` becomes `!((blk-XXXXXX))` and `[[Page]]` becomes
/// `![[Page]]` (issue #190). Whitespace around the target is tolerated.
///
/// **An untranslatable target is left as the literal `{{…}}`, which
/// [`strip_unknown_tokens`] then deletes.** That is this pass's standing
/// rule for a token outl has no syntax for, and the page target is no
/// longer one of them — until `![[page]]` existed, the page name a user
/// pasted was deleted outright, which is the gap this closes.
///
/// A source uuid that is not an outl handle (`{{embed ((CLGp3LXxM))}}`,
/// what a real Roam or Logseq block embed actually carries) is still
/// untranslatable here: the handle is derived from a `NodeId` that only
/// exists for a block already in *this* workspace, and the paste path has
/// no graph to resolve it against. That is `outl-import`'s two-pass job,
/// not a text transform's.
fn rewrite_embed(s: &str) -> String {
    if !s.contains("embed") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;
    while let Some((abs_start, opener)) = next_embed_opener(s, cursor) {
        out.push_str(&s[cursor..abs_start]);
        let after_open = abs_start + opener.len();
        // Locate the matching `}}` for this opener.
        if let Some(close_rel) = s[after_open..].find("}}") {
            let close = after_open + close_rel;
            if let Some(expanded) = expand_embed_target(s[after_open..close].trim()) {
                out.push_str(&expanded);
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

/// The earliest [`EMBED_OPENERS`] match at or after `from`, with the
/// spelling that matched so the caller knows how many bytes to skip.
fn next_embed_opener(s: &str, from: usize) -> Option<(usize, &'static str)> {
    EMBED_OPENERS
        .iter()
        .filter_map(|opener| s[from..].find(opener).map(|rel| (from + rel, *opener)))
        .min_by_key(|(pos, _)| *pos)
}

/// The outl spelling of one embed target, or `None` when outl cannot
/// name it (see [`rewrite_embed`]).
fn expand_embed_target(inner: &str) -> Option<String> {
    if let Some(handle) = extract_block_ref(inner) {
        return Some(format!("!{handle}"));
    }
    extract_page_ref(inner).map(spell_page_embed)
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

/// Extract `Name` from a string that is exactly `[[Name]]`.
///
/// The name rules match the tokenizer's (`reference::try_page_ref`): a
/// non-empty name on one line. A nested `]]` is refused rather than
/// guessed at, so `[[a]] and [[b]]` is not read as one target.
fn extract_page_ref(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    if inner.is_empty() || inner.contains('\n') || inner.contains("]]") {
        return None;
    }
    Some(inner)
}

/// `![[Name]]` when the tokenizer reads that back as the page embed it is
/// meant to be, `[[Name]]` when it does not.
///
/// One name comes apart: an image target. `![[photo.png]]` is the
/// image-embed spelling, owned by `outl_md::wikilink::convert_image_links`,
/// so `try_page_embed` refuses it and the `!` would sit in the block as
/// literal text in front of a link. Falling back to the plain ref loses
/// the expansion and keeps the name, which is the right way round — and
/// still strictly better than what this pass did before, which was to
/// delete the whole token.
fn spell_page_embed(name: &str) -> String {
    let embed = format!("![[{name}]]");
    let reads_back = matches!(
        outl_md::inline::tokenize(&embed).as_slice(),
        [outl_md::inline::InlineTok::PageEmbed { name: back }] if *back == name
    );
    if reads_back {
        embed
    } else {
        format!("[[{name}]]")
    }
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
/// Trailing spaces survive on purpose: a line that ends in a space
/// (`"BRAVE "`) keeps it, so normalisation never trims the user's text
/// beyond what the conversion itself requires.
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
mod tests;
