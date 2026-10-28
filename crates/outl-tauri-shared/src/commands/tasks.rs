//! The workspace's open tasks, as one list — the first [`TASK_LIMIT`]
//! of them, with the uncapped `total` beside them.
//!
//! "What do I still have to do" was answerable only by writing a
//! ` ```query ` block with `status: open` on some page. Mobile now has a
//! Tasks tab that asks it directly, and this is the command behind it.
//!
//! The marker vocabulary is `outl_actions::split_todo`'s, so `TODO`,
//! `DOING` and the `[ ]` checkbox spelling all count, exactly as they
//! do for the checkbox a client draws. The list reads the projected
//! `.md` files the same way `search_blocks` does, which keeps trashed
//! blocks out by construction.

use serde::Serialize;

use outl_actions::{split_todo, TodoState};
use outl_md::block_index::BlockEntry;
use outl_md::index::WorkspaceIndex;

use crate::host::AppHost;

/// Rows sent to the client. The total is never capped, so a workspace
/// with more open tasks than this says so instead of looking complete.
pub const TASK_LIMIT: usize = 200;

/// One open task on the wire.
#[derive(Debug, Clone, Serialize)]
pub struct TaskHit {
    /// Node id, so the client can zoom into the block once its page is open.
    pub id: String,
    /// Slug of the page hosting the task.
    pub source_slug: String,
    /// `todo` or `doing`.
    pub status: String,
    /// Block text with the task marker stripped, on one line.
    pub text: String,
}

/// The open tasks plus the count they were cut from.
#[derive(Debug, Clone, Serialize)]
pub struct OpenTasks {
    /// Every open task in the workspace, before the cap.
    pub total: usize,
    /// At most [`TASK_LIMIT`] rows: started tasks first, then the most
    /// recent journal days, then pages, each page in outline order.
    pub tasks: Vec<TaskHit>,
}

/// List the workspace's open tasks.
///
/// Async: the index build reads and parses every `.md` + sidecar in the
/// workspace, and mobile fires this from the bottom bar on mount — the
/// IPC thread may not do that work ([`crate::offload`]).
pub async fn list_open_tasks<S: AppHost>(state: &S) -> Result<OpenTasks, String> {
    let root = state.storage_root()?;
    crate::offload::run("list_open_tasks", move || {
        let index = WorkspaceIndex::build(&root);
        Ok(collect_open_tasks(index.iter_blocks(), &index, TASK_LIMIT))
    })
    .await
}

/// The selection and ordering behind [`list_open_tasks`], split out so
/// it can be tested without a workspace on disk.
///
/// `pages` is only read for `PageEntry::is_journal` — the one existing
/// owner of "is this slug a journal", so the ranking below cannot
/// disagree with the rest of the app about what a journal is.
pub fn collect_open_tasks<'a>(
    blocks: impl Iterator<Item = &'a BlockEntry>,
    pages: &WorkspaceIndex,
    limit: usize,
) -> OpenTasks {
    let mut open: Vec<(TodoState, &BlockEntry, &str)> = blocks
        .filter_map(|b| match split_todo(&b.text) {
            (Some(s @ (TodoState::Todo | TodoState::Doing)), body) => Some((s, b, body)),
            _ => None,
        })
        .collect();
    // Started tasks first, then journals, then named pages. Journal
    // slugs are ISO dates, so a *descending* slug puts the latest day
    // first; named pages are alphabetical, because nobody scans those by
    // recency and reverse alphabetical is not an order anyone asked for.
    //
    // The two need separate arms: one descending comparison for both
    // puts every letter-initial slug above every ISO date (`i` is 0x69,
    // `2` is 0x32), so journals landed last and named pages came back
    // Z-to-A.
    let journal = |slug: &str| pages.by_slug(slug).is_some_and(|p| p.is_journal);
    open.sort_by(|(sa, a, _), (sb, b, _)| {
        let started = |s: &TodoState| u8::from(*s != TodoState::Doing);
        let (ja, jb) = (journal(&a.source_slug), journal(&b.source_slug));
        started(sa)
            .cmp(&started(sb))
            .then_with(|| u8::from(!ja).cmp(&u8::from(!jb)))
            .then_with(|| {
                if ja && jb {
                    b.source_slug.cmp(&a.source_slug)
                } else {
                    a.source_slug.cmp(&b.source_slug)
                }
            })
            .then_with(|| a.source_block_path.cmp(&b.source_block_path))
    });
    let total = open.len();
    let tasks = open
        .into_iter()
        .take(limit)
        .map(|(s, b, body)| TaskHit {
            id: b.id.to_string(),
            source_slug: b.source_slug.clone(),
            status: if s == TodoState::Doing {
                "doing"
            } else {
                "todo"
            }
            .into(),
            text: body.replace('\n', " ").trim().to_string(),
        })
        .collect();
    OpenTasks { total, tasks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::NodeId;
    use outl_md::index::PageEntry;

    /// The page-kind signal the ordering reads, built by hand so the
    /// test needs no workspace on disk. `is_journal` is the only field
    /// `collect_open_tasks` looks at.
    fn pages(entries: &[(&str, bool)]) -> WorkspaceIndex {
        let mut idx = WorkspaceIndex::default();
        for (slug, is_journal) in entries {
            idx.insert_page(PageEntry {
                path: format!("{slug}.md").into(),
                slug: (*slug).to_string(),
                title: (*slug).to_string(),
                icon: None,
                is_journal: *is_journal,
                pinned: false,
                page_type: None,
                aliases: Vec::new(),
            });
        }
        idx
    }

    fn block(slug: &str, path: &[usize], text: &str) -> BlockEntry {
        BlockEntry {
            id: NodeId::new(),
            ref_handle: "blk-000000".into(),
            source_slug: slug.into(),
            source_path: format!("{slug}.md").into(),
            source_block_path: path.to_vec(),
            text: text.into(),
            text_fold: text.to_lowercase(),
            properties: Vec::new(),
            children: Vec::new(),
        }
    }

    /// **`1on1-with-mario` is load-bearing, not decoration.** `slugify`
    /// keeps digits, so a page called "1on1 with Mario" really slugs to
    /// that — and `'1'` (0x31) sorts *below* `'2'` (0x32), which every
    /// ISO journal slug starts with. A named page whose slug begins with
    /// a letter would be ordered correctly by the ascending tiebreak
    /// alone, so the journal-first arm could be deleted with every test
    /// still green. Replacing this slug with `archive` re-opens that
    /// hole: on a journal-first product, every `1on1`, `2026-review` and
    /// `0-inbox` page would outrank today.
    #[test]
    fn keeps_open_tasks_and_orders_started_then_latest_day_then_pages() {
        let blocks = [
            block("2026-10-10", &[0], "TODO older day"),
            block("2026-10-12", &[1], "TODO second on today"),
            block("2026-10-12", &[0], "[ ] first on today"),
            block("2026-10-11", &[0], "DONE finished"),
            block("2026-10-09", &[2], "DOING started a while ago"),
            block("inbox", &[0], "plain text, not a task"),
            block("inbox", &[1], "TODO on a named page"),
            block("1on1-with-mario", &[0], "TODO on 1on1"),
        ];
        let index = pages(&[
            ("2026-10-09", true),
            ("2026-10-10", true),
            ("2026-10-11", true),
            ("2026-10-12", true),
            ("inbox", false),
            ("1on1-with-mario", false),
        ]);
        let out = collect_open_tasks(blocks.iter(), &index, 10);
        let texts: Vec<_> = out.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "started a while ago",
                "first on today",
                "second on today",
                "older day",
                "on 1on1",
                "on a named page",
            ]
        );
        assert_eq!(out.total, 6);
        assert_eq!(out.tasks[0].status, "doing");
        assert_eq!(out.tasks[1].status, "todo");
    }

    #[test]
    fn the_total_survives_the_cap() {
        let blocks: Vec<_> = (0..5).map(|i| block("inbox", &[i], "TODO x")).collect();
        let index = pages(&[("inbox", false)]);
        let out = collect_open_tasks(blocks.iter(), &index, 2);
        assert_eq!(out.tasks.len(), 2);
        assert_eq!(out.total, 5);
    }

    /// Being started crosses the journal / named-page split: a `DOING`
    /// on a named page is still the thing the user is in the middle of.
    #[test]
    fn started_outranks_the_journal_first_rule() {
        let blocks = [
            block("2026-10-12", &[0], "TODO today"),
            block("inbox", &[0], "DOING started on a named page"),
        ];
        let index = pages(&[("2026-10-12", true), ("inbox", false)]);
        let out = collect_open_tasks(blocks.iter(), &index, 10);
        let texts: Vec<_> = out.tasks.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["started on a named page", "today"]);
    }
}
