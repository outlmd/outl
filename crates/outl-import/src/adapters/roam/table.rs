//! Roam `{{[[table]]}}` blocks → one outl pipe-table block.
//!
//! Roam draws a table from the block's subtree: each child is a row,
//! and a row's cells are the chain below it (the row block is the first
//! cell, its only child the second, and so on). outl models a table as
//! one block whose text carries the rows, so a table whose subtree has
//! that plain shape is rewritten, before conversion, into one block
//! whose text is the pipe table. The cells keep their Roam text, so
//! refs, embeds and links in them go through the normal inline pass.
//!
//! Anything the fold could not carry over keeps the subtree as it was:
//! a cell with more than one child (Roam's merged cells), a multi-line
//! cell, a cell with an attribute, task, heading or code, a cell another
//! block refers to (its uid would stop existing), or a table with fewer
//! than two rows (the first row is the header outl's delimiter row goes
//! under).

use super::{RoamBlock, RoamPage};
use crate::adapters::scan::{is_uid, parse_prop_line};
use crate::report::ImportReport;
use std::collections::HashSet;

/// Fold every foldable table in `pages` in place, counting the tables
/// and the cell blocks that became rows.
pub(super) fn fold_tables(pages: &mut [RoamPage], report: &mut ImportReport) {
    if !pages.iter().any(|p| has_table(&p.children)) {
        return;
    }
    let mut referenced = HashSet::new();
    for page in pages.iter() {
        referenced_uids(&page.children, &mut referenced);
    }
    for page in pages.iter_mut() {
        fold_in(&mut page.children, &referenced, report);
    }
}

fn has_table(blocks: &[RoamBlock]) -> bool {
    blocks
        .iter()
        .any(|b| is_table_marker(&b.string) || has_table(&b.children))
}

fn fold_in(blocks: &mut [RoamBlock], referenced: &HashSet<String>, report: &mut ImportReport) {
    for b in blocks.iter_mut() {
        if is_table_marker(&b.string) {
            if let Some((text, cells)) = pipe_table(b, referenced) {
                b.string = text;
                b.children.clear();
                report.tables_converted += 1;
                report.blocks_folded_into_tables += cells;
                continue;
            }
        }
        fold_in(&mut b.children, referenced, report);
    }
}

/// Whether `raw` is a bare table component.
fn is_table_marker(raw: &str) -> bool {
    let t = raw.trim().to_ascii_lowercase();
    t == "{{[[table]]}}" || t == "{{table}}"
}

/// The pipe-table text of `table` and how many cell blocks it folds,
/// or `None` when the subtree has to stay as it is.
fn pipe_table(table: &RoamBlock, referenced: &HashSet<String>) -> Option<(String, usize)> {
    if table.children.len() < 2 {
        return None;
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(table.children.len());
    for first in &table.children {
        let mut row = Vec::new();
        let mut cell = first;
        loop {
            if !plain_cell(cell, referenced) {
                return None;
            }
            row.push(cell.string.trim().replace('|', "\\|"));
            match cell.children.as_slice() {
                [] => break,
                [next] => cell = next,
                _ => return None,
            }
        }
        rows.push(row);
    }
    let cells = rows.iter().map(Vec::len).sum();
    let width = rows.iter().map(Vec::len).max().unwrap_or(1);
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| format!("| {} |", row.join(" | ")))
        .collect();
    lines.insert(1, format!("|{}", " --- |".repeat(width)));
    Some((lines.join("\n"), cells))
}

/// A cell whose text can sit between two pipes as written.
fn plain_cell(cell: &RoamBlock, referenced: &HashSet<String>) -> bool {
    let s = cell.string.trim();
    let lower = s.to_ascii_lowercase();
    !s.contains('\n')
        && !s.contains("```")
        // A pipe inside a code span would need an escape the span can't carry.
        && !(s.contains('`') && s.contains('|'))
        && !(lower.contains("{{") && lower.contains("query"))
        && !["{{[[todo]]}}", "{{[[done]]}}", "{{todo}}", "{{done}}"]
            .iter()
            .any(|m| lower.contains(m))
        && parse_prop_line(s).is_none()
        && cell.heading.is_none()
        && !referenced.contains(&cell.uid)
        && self_contained(s)
}

/// Whether every paired marker in a cell closes inside it. The rows
/// become one block's text, so an unpaired `__` or `` ` `` would pair
/// with one in a later cell and turn the text between them into one
/// italic or code span across the pipes.
fn self_contained(s: &str) -> bool {
    ["__", "^^", "$$", "`"]
        .iter()
        .all(|m| s.matches(m).count() % 2 == 0)
        && s.matches("{{").count() == s.matches("}}").count()
        && s.matches("[[").count() == s.matches("]]").count()
        && s.matches("((").count() == s.matches("))").count()
}

/// Every uid a `((uid))` ref or embed anywhere in the backup points at.
fn referenced_uids(blocks: &[RoamBlock], out: &mut HashSet<String>) {
    for b in blocks {
        let mut rest = b.string.as_str();
        while let Some(open) = rest.find("((") {
            let after = &rest[open + 2..];
            let Some(close) = after.find("))") else {
                break;
            };
            let uid = after[..close].trim();
            if is_uid(uid) {
                out.insert(uid.to_string());
            }
            rest = &after[close..];
        }
        referenced_uids(&b.children, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(uid: &str, s: &str, children: Vec<RoamBlock>) -> RoamBlock {
        RoamBlock {
            string: s.to_string(),
            uid: uid.to_string(),
            children,
            heading: None,
            open: None,
            create_time: None,
            edit_time: None,
        }
    }

    fn row(uid: &str, cells: &[&str]) -> RoamBlock {
        let mut chain: Option<RoamBlock> = None;
        for (i, c) in cells.iter().enumerate().rev() {
            let kids = chain.take().into_iter().collect();
            chain = Some(block(&format!("{uid}{i}"), c, kids));
        }
        chain.expect("a row has cells")
    }

    #[test]
    fn a_plain_table_folds_into_pipe_rows() {
        let t = block(
            "t",
            "{{[[table]]}}",
            vec![
                row("a", &["Name", "Qty"]),
                row("b", &["apples", "a|b"]),
                row("c", &["pears"]),
            ],
        );
        let (text, cells) = pipe_table(&t, &HashSet::new()).expect("foldable");
        assert_eq!(
            text,
            "| Name | Qty |\n| --- | --- |\n| apples | a\\|b |\n| pears |"
        );
        assert_eq!(cells, 5);
    }

    #[test]
    fn shapes_the_fold_cannot_carry_keep_the_subtree() {
        let one_row = block("t", "{{[[table]]}}", vec![row("a", &["x", "y"])]);
        assert!(pipe_table(&one_row, &HashSet::new()).is_none());

        let mut merged = row("a", &["x"]);
        merged.children = vec![block("m1", "y", vec![]), block("m2", "z", vec![])];
        let t = block("t", "{{[[table]]}}", vec![merged, row("b", &["p", "q"])]);
        assert!(pipe_table(&t, &HashSet::new()).is_none());

        for odd in [
            "two\nlines",
            "key:: value",
            "{{[[TODO]]}} task",
            "```code```",
            "half __italic",
            "a ` tick",
            "open [[link",
        ] {
            let t = block(
                "t",
                "{{table}}",
                vec![row("a", &["x", odd]), row("b", &["p", "q"])],
            );
            assert!(pipe_table(&t, &HashSet::new()).is_none(), "{odd:?}");
        }

        let t = block(
            "t",
            "{{[[table]]}}",
            vec![row("a", &["x", "y"]), row("b", &["p", "q"])],
        );
        let referenced: HashSet<String> = ["b1".to_string()].into();
        assert!(pipe_table(&t, &referenced).is_none());
    }
}
