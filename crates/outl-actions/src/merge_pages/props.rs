//! The property half of a page merge: what lands on the target, and what
//! is reported instead. Pure planning over a `&Workspace`; the caller
//! writes the ops. The decision table lives in the parent module's docs.

use std::collections::HashSet;

use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::parse_alias_list;

use super::PropertyConflict;
use crate::page::{read_text_prop, ALIAS_KEY, TITLE_KEY};
use crate::template::{PARAMS_KEY, TEMPLATE_KEY};
use crate::tree::is_page_model_key;

/// The page-level tag list.
const TAGS_KEY: &str = "tags";

/// What the property merge decided.
#[derive(Debug, Default)]
pub(super) struct PropertyPlan {
    /// `SetProp`s to write on the target, in order.
    pub writes: Vec<(String, PropValue)>,
    /// Scalar keys where both pages disagreed and the target was kept.
    pub conflicts: Vec<PropertyConflict>,
    /// Aliases that were not on the target before and will be after.
    pub aliases_added: Vec<String>,
}

/// Keys the merge never copies, because they are the source's identity
/// rather than its content. `template` (and the `params` that describe
/// it) name how the page is invoked, so copying them would make two
/// pages answer to one template name or hand the target a name it did
/// not choose. `title` becomes an alias instead.
fn is_identity_key(key: &str) -> bool {
    is_page_model_key(key) || key == TITLE_KEY || key == TEMPLATE_KEY || key == PARAMS_KEY
}

/// Plan the merge of `source`'s properties onto `target`.
///
/// `own` are the target's own names (slug, title), never added as its
/// aliases; `redirects` are the source's slug and title, which become
/// aliases so the old name keeps resolving.
pub(super) fn plan(
    workspace: &Workspace,
    source: NodeId,
    target: NodeId,
    own: &[&str],
    redirects: &[&str],
) -> PropertyPlan {
    let mut plan = PropertyPlan::default();
    let mut keys: Vec<&str> = workspace
        .tree()
        .properties_of(source)
        .map(|(key, _)| key)
        .filter(|key| !is_identity_key(key))
        .collect();
    keys.sort_unstable();
    for key in keys {
        match key {
            ALIAS_KEY => {}
            TAGS_KEY => merge_tags(workspace, source, target, &mut plan),
            _ => merge_scalar(workspace, source, target, key, &mut plan),
        }
    }
    template_conflict(workspace, source, target, &mut plan);
    merge_aliases(workspace, source, target, own, redirects, &mut plan);
    plan
}

/// A value's text when it says something: blank counts as absent.
fn meaningful(workspace: &Workspace, node: NodeId, key: &str) -> Option<String> {
    read_text_prop(workspace, node, key)
        .filter(|text| !text.trim().is_empty())
}

fn merge_scalar(
    workspace: &Workspace,
    source: NodeId,
    target: NodeId,
    key: &str,
    plan: &mut PropertyPlan,
) {
    let Some(value) = workspace.tree().property(source, key) else {
        return;
    };
    let Some(dropped) = meaningful(workspace, source, key) else {
        return;
    };
    match meaningful(workspace, target, key) {
        None => plan.writes.push((key.to_string(), value.clone())),
        Some(kept) if kept.trim() == dropped.trim() => {}
        Some(kept) => plan.conflicts.push(PropertyConflict {
            key: key.to_string(),
            kept,
            dropped,
        }),
    }
}

/// A template name is the page's identity: report a difference, copy
/// nothing, whether or not the target has one of its own.
fn template_conflict(
    workspace: &Workspace,
    source: NodeId,
    target: NodeId,
    plan: &mut PropertyPlan,
) {
    let Some(dropped) = meaningful(workspace, source, TEMPLATE_KEY) else {
        return;
    };
    let kept = meaningful(workspace, target, TEMPLATE_KEY).unwrap_or_default();
    if kept.trim() != dropped.trim() {
        plan.conflicts.push(PropertyConflict {
            key: TEMPLATE_KEY.to_string(),
            kept,
            dropped,
        });
    }
}

// --- list-valued keys -----------------------------------------------------

/// Every string a value carries, one entry per list item.
fn strings(value: &PropValue) -> Vec<String> {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => vec![s.clone()],
        PropValue::List(items) => items.iter().flat_map(strings).collect(),
    }
}

fn non_blank(entries: Vec<String>) -> Vec<String> {
    entries
        .into_iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect()
}

/// Tag entries: text splits on commas and whitespace, the other shapes
/// carry one entry per item.
fn tag_entries(value: &PropValue) -> Vec<String> {
    match value {
        PropValue::Text(s) => s
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::to_string)
            .filter(|e| !e.is_empty())
            .collect(),
        other => non_blank(strings(other)),
    }
}

fn tag_key(entry: &str) -> String {
    entry.trim_start_matches('#').to_lowercase()
}

/// Alias entries as the alias readers see them: `parse_alias_list` over
/// text, one entry per item for the other shapes.
fn alias_entries(value: &PropValue) -> Vec<String> {
    match value {
        PropValue::Text(s) => parse_alias_list(s),
        other => non_blank(strings(other)),
    }
}

/// `entries` whose key is not in `seen`, first occurrence only. `seen`
/// grows, so a later call over a second list stays deduplicated.
fn unseen(
    entries: Vec<String>,
    seen: &mut HashSet<String>,
    key: fn(&str) -> String,
) -> Vec<String> {
    entries
        .into_iter()
        .filter(|entry| seen.insert(key(entry)))
        .collect()
}

fn merge_tags(workspace: &Workspace, source: NodeId, target: NodeId, plan: &mut PropertyPlan) {
    let Some(incoming) = workspace.tree().property(source, TAGS_KEY) else {
        return;
    };
    let incoming_entries = tag_entries(incoming);
    if incoming_entries.is_empty() {
        return;
    }
    let existing = workspace.tree().property(target, TAGS_KEY);
    let have = existing.map(tag_entries).unwrap_or_default();
    let Some(existing) = existing.filter(|_| !have.is_empty()) else {
        plan.writes.push((TAGS_KEY.to_string(), incoming.clone()));
        return;
    };
    let mut seen: HashSet<String> = have.iter().map(|e| tag_key(e)).collect();
    let fresh = unseen(incoming_entries, &mut seen, tag_key);
    if !fresh.is_empty() {
        plan.writes
            .push((TAGS_KEY.to_string(), extend(existing, &fresh)));
    }
}

/// `existing` with `fresh` appended, in the shape `existing` already has.
fn extend(existing: &PropValue, fresh: &[String]) -> PropValue {
    match existing {
        PropValue::List(items) => {
            let mut items = items.clone();
            items.extend(fresh.iter().cloned().map(PropValue::Text));
            PropValue::List(items)
        }
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => {
            let sep = if s.contains(',') { ", " } else { " " };
            PropValue::Text(format!("{}{sep}{}", s.trim_end(), fresh.join(sep)))
        }
    }
}

/// One alias as the parser reads it back: a name holding a comma is
/// written `[[...]]`.
fn render_alias(entry: &str) -> String {
    if entry.contains(',') {
        format!("[[{entry}]]")
    } else {
        entry.to_string()
    }
}

/// Whether `entry` survives a write and a re-parse unchanged. A name the
/// alias syntax cannot spell is left out rather than stored as another
/// name.
fn alias_round_trips(entry: &str) -> bool {
    parse_alias_list(&render_alias(entry)) == [entry]
}

fn join_aliases(entries: &[String]) -> String {
    let rendered: Vec<String> = entries.iter().map(|e| render_alias(e)).collect();
    rendered.join(", ")
}

fn alias_key(entry: &str) -> String {
    entry.to_lowercase()
}

fn merge_aliases(
    workspace: &Workspace,
    source: NodeId,
    target: NodeId,
    own: &[&str],
    redirects: &[&str],
    plan: &mut PropertyPlan,
) {
    let existing = workspace.tree().property(target, ALIAS_KEY);
    let have = existing.map(alias_entries).unwrap_or_default();
    let mut incoming = workspace
        .tree()
        .property(source, ALIAS_KEY)
        .map(alias_entries)
        .unwrap_or_default();
    incoming.extend(redirects.iter().map(|name| name.trim().to_string()));
    incoming.retain(|entry| !entry.is_empty() && alias_round_trips(entry));

    let mut seen: HashSet<String> = have.iter().map(|e| alias_key(e)).collect();
    seen.extend(own.iter().map(|name| alias_key(name.trim())));
    let fresh = unseen(incoming, &mut seen, alias_key);
    if fresh.is_empty() {
        return;
    }

    let base = match existing {
        Some(PropValue::Text(text)) => text.trim_end().to_string(),
        Some(_) => join_aliases(&have),
        None => String::new(),
    };
    let added = join_aliases(&fresh);
    let text = if base.is_empty() {
        added
    } else {
        format!("{base}, {added}")
    };
    plan.writes
        .push((ALIAS_KEY.to_string(), PropValue::Text(text)));
    plan.aliases_added = fresh;
}
