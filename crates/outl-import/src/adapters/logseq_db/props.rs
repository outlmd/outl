//! Properties: built-in task properties and user properties.
//!
//! A DB graph keeps properties as plain attributes on the node. Which
//! attribute is a *user* property is read off the attribute's own
//! `:db/ident` namespace (`user.property/…`), and its display name is
//! the property entity's `:block/title`. Value shapes follow
//! `deps/db/src/logseq/db/frontend/property/type.cljs` at tag 2.0.2:
//!
//! - `:default` / `:url` / `:number` / `:node` / `:date` / `:asset`
//!   values are **refs** to another entity. A number lives on the
//!   referenced entity as `:logseq.property/value`; text lives in its
//!   `:block/title`; a node or date is a page, rendered as a link.
//! - `:checkbox` is a raw boolean and `:datetime` a raw epoch-ms int.

use super::convert::Ctx;
use super::edn::Edn;
use super::graph::Entity;
use chrono::{Local, NaiveDate, TimeZone};

/// Ident namespaces whose attributes the converter reads itself, so
/// they are neither user properties nor "stripped".
const HANDLED_ATTRS: &[&str] = &[
    "logseq.property/status",
    "logseq.property/priority",
    "logseq.property/deadline",
    "logseq.property/scheduled",
    "logseq.property/heading",
    "logseq.property/query",
    "logseq.property/page-tags",
    "logseq.property.code/lang",
];

/// Built-in attributes that carry no user content and are not worth a
/// count (they describe the node's own bookkeeping).
const SILENT_ATTRS: &[&str] = &[
    "logseq.property/built-in?",
    "logseq.property/hide?",
    "logseq.property/created-from-property",
    "logseq.property.node/display-type",
];

/// Task state read from `:logseq.property/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Backlog,
    Todo,
    Doing,
    InReview,
    Done,
    Canceled,
}

/// What [`Ctx::collect_props`] pulled off one node.
#[derive(Debug, Default)]
pub(super) struct Collected {
    pub(super) status: Option<Status>,
    pub(super) priority: Option<String>,
    pub(super) deadline: Option<NaiveDate>,
    pub(super) scheduled: Option<NaiveDate>,
    pub(super) heading: Option<u8>,
    pub(super) query: Option<String>,
    /// User properties as `key`, `value`.
    pub(super) props: Vec<(String, String)>,
    /// Page tags from the legacy `:logseq.property/page-tags`.
    pub(super) page_tags: Vec<String>,
}

impl Ctx<'_> {
    /// Walk the node's attributes once and sort them into the buckets
    /// the converter understands. Anything it cannot render is counted
    /// in the tally, never dropped silently.
    pub(super) fn collect_props(&mut self, ent: &Entity) -> Collected {
        let mut out = Collected::default();
        let mut seen: Vec<&str> = Vec::new();
        for (attr, _) in ent.attrs() {
            if seen.contains(&attr.as_str()) {
                continue;
            }
            seen.push(attr);
            self.collect_attr(ent, attr, &mut out);
        }
        out
    }

    fn collect_attr(&mut self, ent: &Entity, attr: &str, out: &mut Collected) {
        let ns = attr.split('/').next().unwrap_or("");
        if ns == "block" || ns == "db" || ns == "kv" || SILENT_ATTRS.contains(&attr) {
            return;
        }
        if HANDLED_ATTRS.contains(&attr) {
            self.collect_builtin(ent, attr, out);
            return;
        }
        if ns.starts_with("logseq.") {
            *self.tally.stripped.entry(attr.to_string()).or_insert(0) += 1;
            return;
        }
        self.collect_user_prop(ent, attr, out);
    }

    fn collect_builtin(&mut self, ent: &Entity, attr: &str, out: &mut Collected) {
        match attr {
            "logseq.property/status" => out.status = self.read_status(ent),
            "logseq.property/priority" => out.priority = self.read_closed_name(ent, attr),
            "logseq.property/deadline" => out.deadline = ent.int(attr).and_then(local_date),
            "logseq.property/scheduled" => out.scheduled = ent.int(attr).and_then(local_date),
            "logseq.property/heading" => {
                out.heading = ent.int(attr).map(|n| n.clamp(1, 3) as u8);
            }
            "logseq.property/query" => {
                out.query = ent
                    .refs(attr)
                    .first()
                    .and_then(|id| self.g.entity(*id))
                    .and_then(|e| e.str("block/title"))
                    .map(str::to_string);
            }
            "logseq.property/page-tags" => {
                for id in ent.refs(attr) {
                    if let Some(t) = self.title_of(id) {
                        out.page_tags.push(t);
                    }
                }
            }
            _ => {}
        }
    }

    /// `:logseq.property/status.<name>` closed value → [`Status`].
    fn read_status(&mut self, ent: &Entity) -> Option<Status> {
        let name = self.read_closed_name(ent, "logseq.property/status")?;
        let s = match name.as_str() {
            "backlog" => Status::Backlog,
            "todo" => Status::Todo,
            "doing" => Status::Doing,
            "in-review" => Status::InReview,
            "done" => Status::Done,
            "canceled" => Status::Canceled,
            other => {
                *self
                    .tally
                    .stripped
                    .entry(format!("logseq.property/status.{other}"))
                    .or_insert(0) += 1;
                return None;
            }
        };
        Some(s)
    }

    /// Name of the closed value a built-in property points at: the
    /// ident suffix (`…/priority.high` → `high`), else its title.
    pub(super) fn read_closed_name(&self, ent: &Entity, attr: &str) -> Option<String> {
        let target = self.g.entity(ent.refs(attr).into_iter().next()?)?;
        if let Some(ident) = target.ident() {
            if let Some((_, name)) = ident.rsplit_once('.') {
                return Some(name.to_string());
            }
        }
        target.str("block/title").map(|t| t.to_ascii_lowercase())
    }

    fn collect_user_prop(&mut self, ent: &Entity, attr: &str, out: &mut Collected) {
        let def = self.g.ident_id(attr).and_then(|id| self.g.entity(id));
        let title = def
            .and_then(|d| d.str("block/title"))
            .unwrap_or_else(|| attr.split_once('/').map_or(attr, |(_, n)| n));
        let ptype = def.and_then(|d| d.get("logseq.property/type").and_then(Edn::as_keyword));
        let values: Vec<String> = ent
            .all(attr)
            .filter_map(|v| self.render_value(ptype, v))
            .filter(|s| !s.trim().is_empty())
            .collect();
        if values.is_empty() {
            *self
                .tally
                .stripped
                .entry(format!("{title} (value not renderable)"))
                .or_insert(0) += 1;
            return;
        }
        out.props.push((prop_key(title), values.join(", ")));
    }

    /// One property value → text. `None` when the shape is unknown.
    fn render_value(&mut self, ptype: Option<&str>, v: &Edn) -> Option<String> {
        match v {
            Edn::Bool(b) => Some(b.to_string()),
            Edn::Str(s) => Some(s.clone()),
            Edn::Float(f) => Some(f.to_string()),
            Edn::BigInt(s) => Some(s.clone()),
            Edn::Keyword(k) => Some(k.clone()),
            Edn::Int(n) => match ptype {
                Some("datetime") => local_date(*n).map(|d| format!("[[{d}]]")),
                Some("checkbox" | "raw-number") => Some(n.to_string()),
                _ => match self.g.entity(*n) {
                    Some(target) => self.render_entity(*n, target),
                    None => Some(n.to_string()),
                },
            },
            _ => None,
        }
    }

    /// A ref value: page → link, number holder → number, text → title.
    fn render_entity(&mut self, id: i64, target: &Entity) -> Option<String> {
        if let Some(page) = self.page_link(id) {
            return Some(page);
        }
        if let Some(value) = target.get("logseq.property/value") {
            return match value {
                Edn::Int(n) => Some(n.to_string()),
                Edn::Float(f) => Some(f.to_string()),
                Edn::Str(s) => Some(s.clone()),
                Edn::Bool(b) => Some(b.to_string()),
                _ => None,
            };
        }
        let title = target.str("block/title")?;
        let rewritten = self.targets.rewrite_plain(title);
        self.note_dangling("(property value)", &rewritten.dangling);
        Some(rewritten.text)
    }

    /// `[[Title]]` (or `[[YYYY-MM-DD]]` for a journal) when `id` is a
    /// page.
    fn page_link(&self, id: i64) -> Option<String> {
        let uuid = self.g.entity(id)?.uuid()?;
        self.targets.pages.get(&uuid).map(|t| format!("[[{t}]]"))
    }

    /// Display title of any titled entity.
    pub(super) fn title_of(&self, id: i64) -> Option<String> {
        let ent = self.g.entity(id)?;
        if let Some(uuid) = ent.uuid() {
            if let Some(t) = self.targets.pages.get(&uuid) {
                return Some(t.clone());
            }
        }
        ent.str("block/title").map(str::to_string)
    }
}

/// Epoch-ms → the calendar date in the importing machine's zone
/// (Logseq shows datetimes in local time, so a UTC date would be a
/// day off for part of the day).
pub(super) fn local_date(ms: i64) -> Option<NaiveDate> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.date_naive())
}

/// A property title → an outl property key (`Reading status` →
/// `reading-status`). Keys that would collide with the emitter's own
/// lines get a `-prop` suffix.
pub(super) fn prop_key(title: &str) -> String {
    let mut key = String::new();
    for c in title.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            key.extend(c.to_lowercase());
        } else if !key.ends_with('-') {
            key.push('-');
        }
    }
    let key = key.trim_matches('-').to_string();
    match key.as_str() {
        "" => "property".to_string(),
        "title" | "id" | "collapsed" => format!("{key}-prop"),
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_keys() {
        assert_eq!(prop_key("Reading status"), "reading-status");
        assert_eq!(prop_key("  Due / Soon!  "), "due-soon");
        assert_eq!(prop_key("title"), "title-prop");
        assert_eq!(prop_key("!!!"), "property");
        assert_eq!(prop_key("Café"), "café");
    }
}
