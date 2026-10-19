//! Human-readable lines for `outl block`.
//!
//! Split out of `block.rs`, which sits on the file-size ratchet, so a
//! new subcommand can land there without the file growing.

use serde_json::Value;

use crate::human::{print_outline_node, todo_prefix};

pub(crate) fn print_block(v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    let todo = v.get("todo").and_then(Value::as_str);
    println!("{id}");
    println!("  {}{}", todo_prefix(todo), text);
    if let Some(children) = v.get("children").and_then(Value::as_array) {
        for child in children {
            print_outline_node(child, 1);
        }
    }
}

pub(crate) fn print_block_created(verb: &str, v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    println!("{verb}: {id}  {text}");
}

pub(crate) fn print_block_simple(verb: &str, v: &Value) {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("?");
    println!("{verb}: {id}");
}

pub(crate) fn print_append_tree(v: &Value) {
    let parent = v.get("parent").and_then(Value::as_str).unwrap_or("?");
    println!("appended tree under {parent}");
    if let Some(tree) = v.get("tree") {
        print_tree_outcome(tree, 1);
    }
}

fn print_tree_outcome(node: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    let id = node.get("id").and_then(Value::as_str).unwrap_or("?");
    println!("{pad}{id}");
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            print_tree_outcome(child, depth + 1);
        }
    }
}
