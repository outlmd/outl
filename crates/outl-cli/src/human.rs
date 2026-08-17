//! Human-readable formatters shared by every CLI subcommand.
//!
//! The JSON envelope is for machines; this module is the
//! terminal-friendly view that runs when `--json` is *not* set. Each
//! formatter consumes the same `serde_json::Value` shape the handler
//! returns, so adding a new printer never forces a change in the
//! business path.

use outl_actions::TodoState;
use serde_json::Value;

/// Prefix used in front of a block body to surface its task state.
///
/// Mirrors what the user sees in the TUI today: `[ ]` for open,
/// `[/]` for started, `[x]` for done, empty for a plain bullet. Lives
/// in one place so every CLI surface (page, block, daily, backlinks,
/// embed) renders the same.
///
/// The value arrives as a `&str` (it crosses the wire as one), so it
/// is resolved back to a [`TodoState`] and rendered through
/// [`state_marker`]'s exhaustive match. A string that matches no
/// state renders as a plain bullet — that can only be an unknown
/// producer, never a variant this binary knows and forgot to draw.
pub fn todo_prefix(todo: Option<&str>) -> &'static str {
    let Some(value) = todo else { return "" };
    ALL_STATES
        .iter()
        .find(|state| state.as_str() == value)
        .map(|state| state_marker(*state))
        .unwrap_or("")
}

/// Marker drawn for one task state.
///
/// No `_` arm on purpose: rendering an unrecognised state as a plain
/// bullet is indistinguishable from a block that is not a task at
/// all, and that is exactly how `DOING` went silently missing from
/// every non-`--json` command. A variant added upstream stops this
/// crate from **compiling** — in every build, not just under
/// `cargo test` — until it gets a marker here.
fn state_marker(state: TodoState) -> &'static str {
    match state {
        TodoState::Todo => "[ ] ",
        TodoState::Doing => "[/] ",
        TodoState::Done => "[x] ",
    }
}

/// Every `TodoState` this crate knows about, as one list.
///
/// The `match` is what makes it exhaustive: it has no `_` arm, so
/// adding a variant upstream stops this file from compiling until
/// someone extends the array below — otherwise the wire-string lookup
/// in [`todo_prefix`] would skip the new variant silently, even with
/// [`state_marker`] already covering it.
const ALL_STATES: [TodoState; 3] = {
    const fn assert_exhaustive(state: TodoState) {
        match state {
            TodoState::Todo | TodoState::Doing | TodoState::Done => (),
        }
    }
    assert_exhaustive(TodoState::Todo);
    [TodoState::Todo, TodoState::Doing, TodoState::Done]
};

/// Print an outline tree starting at depth `depth`. Each node is the
/// shape produced by `outl_actions::project_outline` (after JSON
/// serialization): `{ "text": "...", "todo": "Todo|Done|null",
/// "children": [...] }`.
///
/// `depth = 0` puts the first level flush left. Children render with
/// 2-space indent per level.
pub fn print_outline_tree(nodes: &[Value], depth: usize) {
    for node in nodes {
        print_outline_node(node, depth);
    }
}

/// Print one outline node and recurse into its children.
pub fn print_outline_node(node: &Value, depth: usize) {
    let text = node.get("text").and_then(Value::as_str).unwrap_or("");
    let todo = node.get("todo").and_then(Value::as_str);
    let prefix = todo_prefix(todo);
    println!("{:indent$}- {}{}", "", prefix, text, indent = depth * 2);
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        print_outline_tree(children, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every task state must render as something, and no two may share
    /// a marker.
    #[test]
    fn every_task_state_has_a_visible_marker() {
        let mut seen: Vec<&str> = Vec::new();
        for state in ALL_STATES {
            let marker = todo_prefix(Some(state.as_str()));
            assert!(
                !marker.is_empty(),
                "{} renders as a plain bullet, indistinguishable from a non-task block",
                state.as_str()
            );
            assert!(
                !seen.contains(&marker),
                "{} reuses the marker {marker:?} of another state",
                state.as_str()
            );
            seen.push(marker);
        }
    }

    #[test]
    fn a_block_with_no_task_state_gets_no_marker() {
        assert_eq!(todo_prefix(None), "");
    }
}
