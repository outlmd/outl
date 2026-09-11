//! Parse outl markdown (`.md`, no IDs) into an outline AST.
//!
//! Grammar (informal):
//!
//! ```text
//! page          = page_props blank? block_list
//! page_props    = (prop_line newline)*
//! prop_line     = key "::" SPACE value
//! block_list    = (block_item)*
//! block_item    = indent? "- " content newline
//!                 (prop_line | block_item)*    // children at indent+1
//! indent        = (SPACE SPACE)*               // two spaces per level
//! ```
//!
//! See `docs/markdown-format.md` for the user-facing spec.
//!
//! ## Nothing is dropped in silence
//!
//! Every line of the file must end up somewhere: a block, a property, a
//! block's text, or — when the grammar cannot place it — a recovered
//! verbatim block plus a [`ParseWarning`]. Never advancing past a line
//! without a record.
//!
//! That is not a style preference, it is the producer half of issue
//! #210. `render → parse` used to lose content three ways, all of them
//! reachable from one ordinary shape (a block whose text carries a blank
//! line, which the renderer writes as indented whitespace):
//!
//! 1. an over-indented line was recovered only at `indent == 0` and
//!    skipped mutely below it;
//! 2. a blank line inside a block's text was read as a separator, which
//!    closed continuation and stranded everything after it;
//! 3. a continuation line's own indentation pushed it past the level
//!    that could have claimed it.
//!
//! The reconcile that followed wrote the truncation into the op log as
//! an `Op::Edit`, so the loss reached the source of truth. Measured on a
//! real workspace: 41 pages, 387 lines, down to 0. Each arm below names which of
//! the three it is; the story is here so it is told once.
//!
//! Pinned by `tests/multiline_block_roundtrip.rs`, in particular
//! `no_non_blank_line_is_ever_lost_between_parse_and_render`.
//!
//! Sibling modules own one piece of that grammar each: `ast` (the
//! types this module produces), `property` (the `key:: value` line)
//! and `fence` (fenced code, where the outline grammar is suspended).
//! Their public items are re-exported here, so every
//! `outl_md::parse::*` path stays stable.

use crate::fence::{consume_fence, consume_fence_until_close, fence_marker};
use crate::property::read_page_header;

pub use crate::ast::{OutlineNode, ParseWarning, ParseWarningKind, ParsedPage};
pub use crate::property::parse_property_line;

/// Indent width in spaces. Two-space convention.
pub(crate) const INDENT_WIDTH: usize = 2;

/// Parse a `.md` string into a [`ParsedPage`].
pub fn parse(md: &str) -> ParsedPage {
    // A UTF-8 BOM is an encoding artifact, not content. It is not
    // whitespace (`char::is_whitespace` is false for U+FEFF), so `trim`
    // leaves it glued to the first `- ` and the first line stops being a
    // bullet: the whole first block was recovered as verbatim text with
    // the marker inside it, warning and all. Any `.md` written by a
    // Windows editor lost its first block's identity on import, and a
    // leading `title::` stopped being a page property the same way.
    //
    // Dropped rather than preserved: no renderer re-emits it, so keeping
    // it would leave the file changing shape on every save.
    let md = md.strip_prefix('\u{feff}').unwrap_or(md);
    let lines: Vec<&str> = md.lines().collect();
    let mut cursor = 0usize;

    // `page = page_props blank? block_list` — the header run first, then
    // the outline picks up wherever it stopped.
    let page_props = read_page_header(&lines, &mut cursor);

    let mut warnings: Vec<ParseWarning> = Vec::new();
    let blocks = parse_block_list(&lines, &mut cursor, 0, &mut warnings);

    ParsedPage {
        properties: page_props,
        blocks,
        warnings,
    }
}

fn parse_block_list(
    lines: &[&str],
    i: &mut usize,
    indent: usize,
    warnings: &mut Vec<ParseWarning>,
) -> Vec<OutlineNode> {
    let mut blocks: Vec<OutlineNode> = Vec::new();

    while *i < lines.len() {
        let raw = lines[*i];
        let stripped = raw.trim();
        if stripped.is_empty() {
            *i += 1;
            continue;
        }
        let line_indent = leading_indent(raw);
        if line_indent < indent {
            // Outdent — back to the caller's scope.
            return blocks;
        }
        if line_indent > indent {
            // Over-indented line. The grammar reserves child blocks
            // under a bullet — that case is handled by the inner
            // child-loop inside the bullet branch below. Reaching here
            // means a deeper-indented line landed BEFORE its parent
            // bullet (typical of imported markdown: an indented
            // snippet at the top of the file, or a code-fence the
            // dialect cannot recognise).
            //
            // **Loss #1 in the module doc.** Recovery runs at every
            // depth; this arm used to skip silently whenever
            // `indent != 0`, on the reasoning that the recovery
            // upstream had already caught the parent line. It had not.
            //
            // **A bullet stays a bullet, whatever its indent.** Recovering
            // `    - child` as verbatim *text* preserves the bytes and
            // corrupts the document: the text carries a `- ` the renderer
            // then writes after its own marker, so the next parse reads
            // one more level of nesting than the last, and the block's
            // text grows a marker per save. Measured: `- parent\n    - child`
            // rendered to `- parent\n  -     - child`, then to
            // `- parent\n  - - child`. Content that mutates on every save
            // is worse than content that is merely misplaced, and worse
            // than the silent drop this arm replaced — that at least
            // converged.
            //
            // So an over-indented *marker* line is claimed as a block at
            // this level (its indentation was irregular, its meaning was
            // not), and only a genuinely unplaceable line becomes verbatim
            // text. `raw` there, for the same reason as the marker-less
            // arm below: no byte of the user's file is this parser's to
            // discard.
            warnings.push(ParseWarning {
                line: *i + 1,
                raw: raw.to_string(),
                kind: ParseWarningKind::UnrecognizedBlockMarker,
            });
            if is_block_marker(stripped) {
                // Re-read the line at *our* indent so the marker is
                // consumed rather than folded into the text. Recursing
                // would loop: the callee sees the same over-indent.
                let content = strip_block_marker(stripped);
                blocks.push(OutlineNode {
                    text: content.to_string(),
                    properties: Vec::new(),
                    children: Vec::new(),
                });
            } else {
                // `trim_start`, not `raw`: the leading indent of a line
                // the grammar could not place is the renderer's layout,
                // not the user's content, and keeping it inside the text
                // means the renderer writes it *after* its own marker —
                // so the file settles on the second save instead of the
                // first. The warning above carries the untouched line.
                blocks.push(OutlineNode {
                    text: raw.trim_start().to_string(),
                    properties: Vec::new(),
                    children: Vec::new(),
                });
            }
            *i += 1;
            continue;
        }
        if !is_block_marker(stripped) {
            // Non-outline line at our indent. At depth 0 we recover by
            // turning it into a verbatim block (and emit a warning) so
            // a hand-written `.md` with a leading `# title`, a stray
            // paragraph, or imported markdown doesn't silently lose
            // content on the next save. At deeper levels we bail back
            // to the caller (it knows the context). Store `raw`, not
            // the trimmed form, so trailing whitespace and any other
            // significant bytes round-trip on the next save.
            if indent != 0 {
                return blocks;
            }
            warnings.push(ParseWarning {
                line: *i + 1,
                raw: raw.to_string(),
                kind: ParseWarningKind::UnrecognizedBlockMarker,
            });
            blocks.push(OutlineNode {
                text: raw.to_string(),
                properties: Vec::new(),
                children: Vec::new(),
            });
            *i += 1;
            continue;
        }

        // Consume a block marker line.
        let content = strip_block_marker(stripped);
        // Did this marker line declare a first line of text at all?
        //
        // **Loss #4 in the module doc.** `render::write_block_text`
        // distinguishes the two states it can be handed: empty text emits a
        // bare `-`, and text whose first line is empty (`"\na"`) emits
        // `- ` — marker, space, nothing — followed by the rest as
        // continuation. Reading both back as "no first line" collapses the
        // second onto the first and drops an empty line that was really
        // there, so this is the exact inverse of that decision and has to
        // stay one.
        //
        // `stripped` cannot answer it (`"- "` and `"-"` both trim to
        // `"-"`), hence the second look at the raw line.
        let declares_first_line = declares_first_line(raw);
        *i += 1;

        let mut node = OutlineNode {
            text: content.to_string(),
            properties: Vec::new(),
            children: Vec::new(),
        };

        // Continuation lines are only valid before any property or
        // child. Once one of those appears, plain indented text becomes
        // "unrecognized" again (skipped) — keeps the grammar
        // unambiguous.
        let mut accepting_continuation = true;

        // Blank lines seen inside this block's text, not yet written.
        // Held rather than appended so a blank line that turns out to be
        // trailing (the next line is a sibling, not a continuation)
        // leaves no mark on the text — see the blank-line arm below.
        let mut pending_blanks = 0usize;

        // If the block's initial content is itself a fence opener (the
        // user wrote `- ```lisp` on one line), the opener already lives
        // in `node.text`. Pull the body and closing fence in *now*,
        // before we go looking at child/continuation lines — otherwise
        // we'd misread the closing `` ``` `` on a later line as a new
        // opener and swallow everything down to EOF.
        if let Some(marker) = fence_marker(node.text.trim_start()) {
            consume_fence_until_close(lines, i, indent + 1, marker, &mut node.text);
            // Once a code fence has closed, the block is done — any
            // further indented text is no longer "continuation of a
            // single bullet" but a fresh thing the grammar can't see
            // safely. Properties and children still work via the loop
            // below if they appear.
            accepting_continuation = false;
        }

        // Read this block's continuation, properties and children at
        // indent + 1.
        loop {
            if *i >= lines.len() {
                break;
            }
            let next_raw = lines[*i];
            if next_raw.trim().is_empty() {
                // A blank line inside the block's own text, or a
                // separator between siblings? The indent answers it.
                //
                // **Loss #2 in the module doc.**
                // `render::write_block_text` emits every line of
                // `text` after the first at `indent + 1`, so an empty
                // line *within* the text comes back as whitespace
                // indented to that level, while a separator between
                // blocks is a genuinely empty line.
                //
                // Only while still accepting continuation — once a
                // child or a fence has claimed the slot, a blank line
                // is a separator whatever its indent.
                //
                // **Held, not appended.** A trailing `\n` on the text is
                // not what the user wrote and not what the renderer will
                // write back, but it *is* a different `content_hash`, so
                // appending eagerly made every page carrying this shape
                // emit an `Op::Edit` on the next reconcile — churn in the
                // log for a byte nobody typed. The newline is only
                // materialised when a continuation line actually follows
                // it (see `pending_blanks` below).
                if accepting_continuation
                    && !next_raw.is_empty()
                    && leading_indent(next_raw) > indent
                {
                    pending_blanks += 1;
                    *i += 1;
                    continue;
                }
                // Blank line terminates continuation but not children
                // (children can have blank gaps between them in the
                // user's source).
                accepting_continuation = false;
                *i += 1;
                continue;
            }
            let next_indent = leading_indent(next_raw);
            if next_indent <= indent {
                break;
            }
            if next_indent == indent + 1 {
                let next_stripped = next_raw.trim();
                if is_block_marker(next_stripped) {
                    // Child block — recurse for the full sub-list.
                    accepting_continuation = false;
                    let children = parse_block_list(lines, i, indent + 1, warnings);
                    node.children.extend(children);
                } else if accepting_continuation && fence_marker(next_stripped).is_some() {
                    // Fenced code block — consume literally until the
                    // matching closing fence at the same indent.
                    // Separate and flush held blank lines exactly as the
                    // two prose arms below do. Doing neither dropped a
                    // blank line that sat between the text and the fence
                    // (`"a\n\n```…"` came back as `"a\n```…"`) — loss #2
                    // again, in the one arm that did not share the code.
                    if needs_newline_separator(&node.text, declares_first_line) {
                        node.text.push('\n');
                    }
                    for _ in 0..std::mem::take(&mut pending_blanks) {
                        node.text.push('\n');
                    }
                    consume_fence(lines, i, indent + 1, false, &mut node.text);
                } else if let Some(kv) = parse_property_line(next_stripped) {
                    // A property does NOT close continuation.
                    //
                    // Properties are contiguous in the grammar at the top
                    // of this file, so the first non-property line after
                    // them is still this block's prose. Closing here made
                    // every following text line fall into the "skip to
                    // avoid hang" arm below — dropped with no AST entry
                    // and no warning, which is how a page ends up
                    // hash-faithful while its content exists in no op
                    // (issue #210, RFC 0210).
                    //
                    // The trigger was ordinary: outl writes
                    // `collapsed:: true` itself when a block is folded,
                    // so folding a multi-line block put its body at risk.
                    // A `remind::` whose value the grammar can't schedule
                    // stays on disk verbatim (never rewritten, never
                    // dropped) — it only loses its scheduling and shows
                    // up in the parse banner so the user can fix the typo.
                    if kv.0.eq_ignore_ascii_case(crate::remind::REMIND_KEY) {
                        for kind in crate::remind::parse_remind(&kv.1).warnings {
                            warnings.push(ParseWarning {
                                line: *i + 1,
                                raw: next_raw.to_string(),
                                kind,
                            });
                        }
                    }
                    node.properties.push(kv);
                    *i += 1;
                } else if accepting_continuation {
                    // Continuation of the block's text — append with a
                    // newline separator. Preserves the user's wrap
                    // intent without baking the indent into the text.
                    if needs_newline_separator(&node.text, declares_first_line) {
                        node.text.push('\n');
                    }
                    for _ in 0..std::mem::take(&mut pending_blanks) {
                        node.text.push('\n');
                    }
                    node.text.push_str(next_stripped);
                    *i += 1;
                } else {
                    // A line the grammar cannot place, and the arm the
                    // module doc's rule exists for.
                    //
                    // Not folded into `node.text`: continuation is closed
                    // at this point (a blank line, or a child block
                    // already claimed the slot), so appending there would
                    // silently re-flow the document. It becomes a
                    // recovered **child** block instead, at the depth the
                    // line was written, so the reader sees it where they
                    // put it.
                    //
                    // Warning *and* a block, not one or the other. An
                    // earlier version emitted only the warning, trusting
                    // `reconcile` to refuse to advance the sidecar hash
                    // so the `.md` would keep the text. That leaves the
                    // page permanently dirty with its content permanently
                    // outside the log, and makes not losing bytes depend
                    // on a guard in another crate.
                    warnings.push(ParseWarning {
                        line: *i + 1,
                        raw: next_raw.to_string(),
                        kind: ParseWarningKind::UnrecognizedBlockMarker,
                    });
                    // Strip the indentation the renderer will put back.
                    // Storing `raw` here keeps a leading indent inside the
                    // *text*, which the renderer then writes after its own
                    // — `  body` became `  -   body`, and the next parse
                    // read a different text than the one it wrote. The
                    // warning above carries the untouched line, so nothing
                    // about the original is lost for the reader.
                    node.children.push(OutlineNode {
                        // `trim_start` on top of the strip: an indent
                        // that is not a whole number of levels (three
                        // spaces where a level is two) leaves a residue
                        // the renderer writes after its marker, and the
                        // next parse trims it — so the file settled on
                        // pass two instead of pass one. Measured on the
                        // real workspace: 3 pages of 2,827, all of them
                        // whitespace-only differences with no line lost.
                        text: strip_indent_levels(next_raw, indent + 1)
                            .trim_start()
                            .to_string(),
                        properties: Vec::new(),
                        children: Vec::new(),
                    });
                    *i += 1;
                }
            } else if accepting_continuation && !is_block_marker(next_raw.trim()) {
                // Over-indented, but this block's text is still open and
                // the line carries no bullet: it is a continuation line
                // whose *own* text was indented.
                //
                // **Loss #3 in the module doc.**
                // `render::write_block_text` writes each continuation at
                // `indent + 1` and then the line verbatim, so a text like
                // "head\n  detail" comes back at `indent + 2`. Recursing
                // here (the old behaviour) handed it to a child list that
                // could not place it either. Strip exactly the levels the
                // renderer added so the internal indentation survives.
                let body = strip_indent_levels(next_raw, indent + 1);
                if needs_newline_separator(&node.text, declares_first_line) {
                    node.text.push('\n');
                }
                for _ in 0..std::mem::take(&mut pending_blanks) {
                    node.text.push('\n');
                }
                node.text.push_str(body.trim_end());
                *i += 1;
            } else {
                // Over-indented; recurse so the deeper level can claim it.
                accepting_continuation = false;
                let extra = parse_block_list(lines, i, indent + 1, warnings);
                node.children.extend(extra);
            }
        }

        blocks.push(node);
    }

    blocks
}

/// Drop exactly `levels` indent levels from the front of `line`,
/// keeping whatever indentation the line carried beyond them.
///
/// The inverse of what [`crate::render::write_block_text`] adds when it
/// emits a continuation line, so a block whose text is itself indented
/// ("head\n  detail") survives `render → parse` unchanged. Trimming the
/// whole prefix instead is what flattened that indentation and made the
/// two directions disagree.
///
/// A line shallower than `levels` is returned with its leading
/// whitespace removed and nothing else — there is no negative indent to
/// preserve.
fn strip_indent_levels(line: &str, levels: usize) -> &str {
    let mut budget = levels * INDENT_WIDTH;
    let mut cut = 0usize;
    for b in line.bytes() {
        if budget == 0 {
            break;
        }
        match b {
            b' ' => budget -= 1,
            // A tab is one whole level (see `leading_indent`); it cannot
            // be split, so stop rather than eat more than was asked for.
            b'\t' if budget >= INDENT_WIDTH => budget -= INDENT_WIDTH,
            _ => break,
        }
        cut += 1;
    }
    &line[cut..]
}

/// Outline depth of a line: leading whitespace divided by the
/// two-space indent unit. One tab counts as one full level.
pub(crate) fn leading_indent(line: &str) -> usize {
    let mut spaces = 0usize;
    for b in line.bytes() {
        if b == b' ' {
            spaces += 1;
        } else if b == b'\t' {
            // Treat one tab as one full indent level.
            spaces += INDENT_WIDTH;
        } else {
            break;
        }
    }
    spaces / INDENT_WIDTH
}

fn is_block_marker(stripped: &str) -> bool {
    stripped == "-" || stripped.starts_with("- ")
}

/// Whether appending to `text` needs a `\n` in front of what comes next.
///
/// Three sites ask it — prose continuation, an over-indented
/// continuation, and a fence opener — and they have to agree. When they
/// did not, the fence site read an empty `text` as "nothing written yet"
/// and swallowed the empty first line that `- ` had just declared, so
/// `"\n```"` came back as `"```"`: the same defect as the prose arms, in
/// the one place that did not share their guard. A property test found
/// it; none of the hand-written cases covered a fence opening on a block
/// whose text starts with a newline.
fn needs_newline_separator(text: &str, declares_first_line: bool) -> bool {
    !text.is_empty() || declares_first_line
}

/// Whether a bullet line declares a first line of text, even an empty one.
///
/// The inverse of [`crate::render::write_block_text`]'s only branch: it
/// writes a bare `-` for empty text and `- ` plus the first line for
/// everything else, so `- ` with nothing after it means "the text starts
/// with a newline". Takes the raw line because the trimmed form has
/// already thrown the distinction away.
fn declares_first_line(raw: &str) -> bool {
    raw.trim_start().starts_with("- ")
}

fn strip_block_marker(stripped: &str) -> &str {
    if stripped == "-" {
        return "";
    }
    stripped.strip_prefix("- ").unwrap_or(stripped).trim_start()
}

#[cfg(test)]
mod tests;
