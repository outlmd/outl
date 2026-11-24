//! The slice of a scrolled body a frame actually draws.

use ratatui::text::Line;

/// `len` lines starting at `start` of the concatenation of `segments`,
/// without concatenating them: a frame needs the rows on screen, and
/// copying the whole body (a hub page's panel is thousands of lines)
/// to scroll past most of it was the other half of the per-keystroke
/// cost. Short of `len` when the body ends first.
pub(super) fn visible_window(
    segments: &[&[Line<'static>]],
    start: usize,
    len: usize,
) -> Vec<Line<'static>> {
    segments
        .iter()
        .flat_map(|s| s.iter())
        .skip(start)
        .take(len)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(range: std::ops::Range<usize>) -> Vec<Line<'static>> {
        range.map(|i| Line::from(i.to_string())).collect()
    }

    /// Equivalent to slicing the concatenation, which is what the frame
    /// did before: the window may start in any segment and span several.
    #[test]
    fn it_equals_slicing_the_concatenated_body() {
        let (a, b, c) = (lines(0..4), lines(4..4), lines(4..9));
        let all: Vec<Line<'static>> = [&a[..], &b[..], &c[..]].concat();
        for start in 0..=all.len() + 1 {
            for len in 0..=all.len() + 1 {
                let want: Vec<_> = all.iter().skip(start).take(len).cloned().collect();
                assert_eq!(visible_window(&[&a, &b, &c], start, len), want);
            }
        }
    }
}
