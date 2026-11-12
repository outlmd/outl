//! One block: content, task state, tags, special node kinds.

use super::convert::{has, Ctx};
use super::graph::Entity;
use super::props::{Collected, Status};
use super::text::{tag_text, Dangling};
use crate::adapters::logseq::inline;
use crate::adapters::scan::parse_whole_fence;
use crate::ir::{BlockContent, ImportBlock, TaskState};
use crate::report::ImportReport;
use chrono::{DateTime, Utc};

impl Ctx<'_> {
    pub(super) fn convert_block(
        &mut self,
        ent: &Entity,
        label: &str,
        report: &mut ImportReport,
    ) -> ImportBlock {
        let tags = ent.refs("block/tags");
        let mut c = self.collect_props(ent);
        let title = ent.str("block/title").unwrap_or("");
        let content = if has(&tags, self.cls.code) {
            let lang = ent
                .str("logseq.property.code/lang")
                .filter(|l| !l.is_empty())
                .map(str::to_string);
            BlockContent::Code {
                lang,
                body: title.to_string(),
            }
        } else {
            self.text_content(ent, title, &tags, &c, label, report)
        };
        let (task, mut props) = self.task_props(&c, report);
        if let Some(q) = c.query.take() {
            props.push(("query".to_string(), q));
        }
        props.append(&mut c.props);
        ImportBlock {
            uid: ent.uuid(),
            content,
            children: Vec::new(),
            task,
            heading: c.heading,
            collapsed: ent.flag("block/collapsed?"),
            props,
            created: ent.int("block/created-at").and_then(ms_to_datetime),
            edited: ent.int("block/updated-at").and_then(ms_to_datetime),
        }
    }

    fn text_content(
        &mut self,
        ent: &Entity,
        title: &str,
        tags: &[i64],
        c: &Collected,
        label: &str,
        report: &mut ImportReport,
    ) -> BlockContent {
        let rw = self.targets.rewrite(title);
        self.note_dangling(label, &rw.dangling);
        let mut text = rw.text;
        self.mark_special_kind(ent, tags, &mut text, report);
        for date in [c.deadline, c.scheduled].into_iter().flatten() {
            text.push_str(&format!(" [[{date}]]"));
            report.org_dates_converted += 1;
        }
        for t in self.unmentioned_tags(ent, title) {
            text.push(' ');
            text.push_str(&tag_text(&t));
        }
        if let Some((lang, body)) = parse_whole_fence(&text) {
            BlockContent::Code { lang, body }
        } else if text.contains("```") {
            BlockContent::Verbatim(text)
        } else {
            BlockContent::Inline(inline::tokenize(&text, report))
        }
    }

    /// Quote blocks keep their quote; the other node kinds keep their
    /// text and are counted by kind so the loss is visible.
    fn mark_special_kind(
        &self,
        ent: &Entity,
        tags: &[i64],
        text: &mut String,
        report: &mut ImportReport,
    ) {
        if has(tags, self.cls.quote) {
            *text = text
                .split('\n')
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            report.count_component("quote");
        }
        let kinds = [
            (self.cls.math, "math"),
            (self.cls.query, "query"),
            (self.cls.cards, "cards"),
            (self.cls.template, "template"),
            (self.cls.pdf, "pdf-annotation"),
        ];
        for (class, kind) in kinds {
            if has(tags, class) {
                report.count_component(kind);
            }
        }
        if has(tags, self.cls.asset) || ent.get("logseq.property.asset/type").is_some() {
            report.count_component("asset");
        }
    }

    /// User classes on the node that the title does not already spell
    /// as `#[[<uuid>]]` — tags added through the tags property are not
    /// in the text at all.
    fn unmentioned_tags(&self, ent: &Entity, title: &str) -> Vec<String> {
        ent.refs("block/tags")
            .into_iter()
            .filter_map(|t| self.g.entity(t))
            .filter(|t| !t.flag("logseq.property/built-in?"))
            .filter(|t| t.uuid().is_none_or(|u| !title.to_ascii_lowercase().contains(&u)))
            .filter_map(|t| t.str("block/title").map(str::to_string))
            .collect()
    }

    /// `:logseq.property/status` → task state, with the two states outl
    /// lacks (`backlog`, `in-review`) kept in a `state::` property.
    fn task_props(
        &self,
        c: &Collected,
        report: &mut ImportReport,
    ) -> (Option<TaskState>, Vec<(String, String)>) {
        let mut props = Vec::new();
        let task = c.status.map(|s| {
            let (state, key, extra) = match s {
                Status::Todo => (TaskState::Todo, "TODO", None),
                Status::Doing => (TaskState::Doing, "DOING", None),
                Status::Done => (TaskState::Done, "DONE", None),
                Status::Canceled => (TaskState::Canceled, "CANCELED", None),
                Status::Backlog => (TaskState::Todo, "BACKLOG", Some("backlog")),
                Status::InReview => (TaskState::Doing, "IN-REVIEW", Some("in-review")),
            };
            report.count_task(key);
            if let Some(extra) = extra {
                props.push(("state".to_string(), extra.to_string()));
            }
            state
        });
        if let Some(p) = &c.priority {
            props.push(("priority".to_string(), p.clone()));
        }
        (task, props)
    }

    pub(super) fn note_dangling(&mut self, label: &str, dangling: &[Dangling]) {
        for d in dangling {
            self.tally.dangling.push((label.to_string(), d.uuid.clone()));
        }
    }
}

pub(super) fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}
