//! Splitting a block's text into the runs outside fenced code.
//!
//! The inline tokenizer knows single-backtick spans on one line and
//! nothing about fences, so `[[x]]` inside a ```` ``` ```` block comes out
//! of it as a `PageRef`. The rewriter must never edit code, so it hands
//! the tokenizer only what this returns.
//!
//! The fence grammar mirrors the outline parser's: a line whose trimmed
//! text starts with ```` ``` ```` or `~~~` opens a fence, and the fence
//! closes only on its own character, alone on the line. An unclosed fence
//! runs to the end of the text, which is the parser's "graceful close".

/// The fence character a line opens with, if it opens one.
fn opener(line: &str) -> Option<char> {
    let trimmed = line.trim();
    ['`', '~']
        .into_iter()
        .find(|c| trimmed.starts_with(&c.to_string().repeat(3)))
}

/// Whether `line` closes a fence opened with `fence_char`.
fn closes(line: &str, fence_char: char) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with(&fence_char.to_string().repeat(3))
        && trimmed.chars().all(|c| c == fence_char)
}

/// The slices of `text` that sit outside every fenced block, in order.
/// The fence lines themselves are excluded. Each slice borrows from
/// `text`, so byte offsets into it stay computable by pointer.
pub(super) fn prose_segments(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    let mut run_start: Option<usize> = Some(0);
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let in_fence = match fence {
            Some(c) => {
                if closes(line, c) {
                    fence = None;
                }
                true
            }
            None => match opener(line) {
                Some(c) => {
                    fence = Some(c);
                    true
                }
                None => false,
            },
        };
        if in_fence {
            if let Some(start) = run_start.take() {
                if start < offset {
                    out.push(&text[start..offset]);
                }
            }
        } else if run_start.is_none() {
            run_start = Some(offset);
        }
        offset += line.len();
    }
    if let Some(start) = run_start {
        if start < text.len() {
            out.push(&text[start..]);
        }
    }
    out
}
