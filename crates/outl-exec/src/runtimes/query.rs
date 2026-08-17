//! `query` runtime — declarative workspace queries as code blocks.
//!
//! A ` ```query ` fence runs a line-by-line declarative DSL against the
//! workspace and returns matching blocks as **embed references**
//! (`!((blk-XXXXXX))`), not copies. This means toggling a TODO on the
//! original block is reflected everywhere the query result appears.
//!
//! Two entry points into the same engine:
//!
//! - **DSL string** (` ```query ` code block) — user-facing, renders embeds.
//! - **Structured API** (`run_query_structured`) — plugin-facing, returns
//!   typed `QueryHit` values. Exposed to JS as `outl.query({ … })`.
//!
//! Both converge on the same `Query` + `engine::run` pipeline.

use std::path::Path;
use std::time::Instant;

use outl_md::index::WorkspaceIndex;

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

// ── Public query API (used by both ```query and plugin SDK) ─────────────

/// Structured query parameters — the plugin-facing API.
///
/// Every field is optional; an empty struct matches every block.
/// This is the shape that `outl.query({ … })` deserialises from JS.
#[derive(Debug, Default, Clone)]
pub struct QueryParams {
    /// `"todo"`, `"done"`, or `"open"` (either).
    pub status: Option<String>,
    /// Partial tag match (without `#`).
    pub tag: Option<String>,
    /// `"journal"` or `"page"`.
    pub kind: Option<String>,
    /// Duration like `"7d"`, `"2w"`, `"3m"`.
    pub since: Option<String>,
    /// Substring search (case-insensitive).
    pub text: Option<String>,
    /// Sort keys in priority order.
    pub sort: Vec<String>,
    /// Maximum number of results.
    pub limit: Option<usize>,
}

/// One query result — structured, typed, no markdown.
#[derive(Debug, Clone)]
pub struct QueryHit {
    /// Block ref handle (`blk-XXXXXX`).
    pub handle: String,
    /// Slug of the hosting page.
    pub page: String,
    /// `"todo"`, `"done"`, or `None` when the block is not a task.
    pub status: Option<String>,
    /// Block text with TODO/DONE prefix stripped.
    pub text: String,
}

/// Run a query from structured parameters against the workspace at
/// `workspace_root`. Returns sorted, limited hits.
pub fn run_query_structured(
    params: &QueryParams,
    workspace_root: &Path,
) -> Result<Vec<QueryHit>, String> {
    let query = build_query_from_params(params)?;
    run_query_internal(&query, workspace_root)
}

/// Run a query from a DSL string against the workspace at
/// `workspace_root`. Returns sorted, limited hits.
pub fn run_query_dsl(dsl: &str, workspace_root: &Path) -> Result<Vec<QueryHit>, String> {
    let query = dsl::parse(dsl).map_err(|e| e.to_string())?;
    run_query_internal(&query, workspace_root)
}

fn run_query_internal(query: &dsl::Query, workspace_root: &Path) -> Result<Vec<QueryHit>, String> {
    let index = WorkspaceIndex::build(workspace_root);
    let mut hits = engine::run(&index, query);
    engine::sort_hits(&mut hits, &query.sort);
    if let Some(limit) = query.limit {
        hits.truncate(limit);
    }
    Ok(hits
        .into_iter()
        .map(|h| QueryHit {
            handle: h.handle,
            page: h.page_slug,
            status: h.status.map(|s| s.as_str().to_string()),
            text: h.text,
        })
        .collect())
}

fn build_query_from_params(p: &QueryParams) -> Result<dsl::Query, String> {
    let mut filters = Vec::new();
    if let Some(s) = &p.status {
        filters.push(dsl::Filter::Status(match s.as_str() {
            "todo" => dsl::StatusFilter::Todo,
            "doing" => dsl::StatusFilter::Doing,
            "done" => dsl::StatusFilter::Done,
            "open" => dsl::StatusFilter::Open,
            other => {
                return Err(format!(
                    "invalid status '{other}' (use todo|doing|done|open)"
                ))
            }
        }));
    }
    if let Some(t) = &p.tag {
        filters.push(dsl::Filter::Tag(t.clone()));
    }
    if let Some(k) = &p.kind {
        filters.push(dsl::Filter::Kind(match k.as_str() {
            "journal" => dsl::KindFilter::Journal,
            "page" => dsl::KindFilter::Page,
            other => return Err(format!("invalid kind '{other}' (use journal|page)")),
        }));
    }
    if let Some(s) = &p.since {
        filters.push(dsl::Filter::Since(parse_duration_pub(s)?));
    }
    if let Some(t) = &p.text {
        filters.push(dsl::Filter::Text(t.clone()));
    }
    let mut sort = Vec::new();
    for s in &p.sort {
        sort.push(match s.as_str() {
            "page" => dsl::SortKey::Page,
            "status" => dsl::SortKey::Status,
            "text" => dsl::SortKey::Text,
            other => return Err(format!("invalid sort key '{other}' (use page|status|text)")),
        });
    }
    Ok(dsl::Query {
        filters,
        sort,
        limit: p.limit,
    })
}

fn parse_duration_pub(v: &str) -> Result<u32, String> {
    if v.is_empty() {
        return Err("since requires a duration like '7d', '2w', '3m'".into());
    }
    let (num_str, unit) = v.split_at(v.len() - 1);
    let n: u32 = num_str
        .parse()
        .map_err(|_| format!("since: invalid number in '{v}'"))?;
    match unit {
        "d" => Ok(n),
        "w" => Ok(n * 7),
        "m" => Ok(n * 30),
        _ => Err(format!("since: unknown unit '{unit}' (use d, w, or m)")),
    }
}

/// Query runtime — runs the DSL against the workspace on disk.
pub struct QueryRuntime;

impl Runtime for QueryRuntime {
    fn language(&self) -> &'static str {
        "query"
    }

    fn auto_run(&self) -> bool {
        true
    }

    fn execute(&self, source: &str, ctx: &ExecContext) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();

        let hits = run_query_dsl(source, &ctx.workspace_root).map_err(ExecError::Language)?;

        let stdout = hits
            .iter()
            .map(|h| format!("!(({}))", h.handle))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ExecOutput {
            stdout,
            stderr: String::new(),
            duration: start.elapsed(),
            exit: ExitStatus::Ok,
            format: OutputFormat::Embeds,
        })
    }
}

/// DSL parser.
pub(crate) mod dsl {
    use std::fmt;

    /// Parsed query.
    #[derive(Debug, Default)]
    pub struct Query {
        pub filters: Vec<Filter>,
        pub sort: Vec<SortKey>,
        pub limit: Option<usize>,
    }

    #[derive(Debug, Clone)]
    pub enum Filter {
        Status(StatusFilter),
        Tag(String),
        Kind(KindFilter),
        Since(u32),
        Text(String),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum StatusFilter {
        Todo,
        Doing,
        Done,
        Open,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KindFilter {
        Journal,
        Page,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SortKey {
        Page,
        Status,
        Text,
    }

    #[derive(Debug)]
    pub struct ParseError {
        /// 1-based line number where the error occurred.
        pub line: usize,
        /// Human-readable description.
        pub msg: String,
    }

    impl fmt::Display for ParseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "line {}: {}", self.line, self.msg)
        }
    }

    /// Parse a query DSL source string into a [`Query`].
    pub fn parse(source: &str) -> Result<Query, ParseError> {
        let mut filters = Vec::new();
        let mut sort = Vec::new();
        let mut limit = None;

        for (i, raw) in source.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (key, value) = split_kv(line, i)?;
            let key = key.trim();
            let value = value.trim();

            match key {
                "status" => filters.push(Filter::Status(parse_status(value, i)?)),
                "tag" => filters.push(Filter::Tag(value.to_string())),
                "kind" => filters.push(Filter::Kind(parse_kind(value, i)?)),
                "since" => filters.push(Filter::Since(parse_duration(value, i)?)),
                "text" => filters.push(Filter::Text(value.to_string())),
                "sort" => {
                    for part in value.split(',') {
                        sort.push(parse_sort_key(part.trim(), i)?);
                    }
                }
                "limit" => {
                    limit = Some(parse_usize(value, i)?);
                }
                _ => {
                    return Err(ParseError {
                        line: i + 1,
                        msg: format!("unknown key: '{key}'"),
                    });
                }
            }
        }

        Ok(Query {
            filters,
            sort,
            limit,
        })
    }

    fn split_kv(line: &str, line_idx: usize) -> Result<(&str, &str), ParseError> {
        line.split_once(':').ok_or_else(|| ParseError {
            line: line_idx + 1,
            msg: "expected 'key: value'".into(),
        })
    }

    fn parse_status(v: &str, line_idx: usize) -> Result<StatusFilter, ParseError> {
        match v {
            "todo" => Ok(StatusFilter::Todo),
            "doing" => Ok(StatusFilter::Doing),
            "done" => Ok(StatusFilter::Done),
            "open" => Ok(StatusFilter::Open),
            _ => Err(ParseError {
                line: line_idx + 1,
                msg: format!("status must be 'todo', 'doing', 'done', or 'open', got '{v}'"),
            }),
        }
    }

    fn parse_kind(v: &str, line_idx: usize) -> Result<KindFilter, ParseError> {
        match v {
            "journal" => Ok(KindFilter::Journal),
            "page" => Ok(KindFilter::Page),
            _ => Err(ParseError {
                line: line_idx + 1,
                msg: format!("kind must be 'journal' or 'page', got '{v}'"),
            }),
        }
    }

    /// Parse `Nd` / `Nw` / `Nm` into a day count.
    fn parse_duration(v: &str, line_idx: usize) -> Result<u32, ParseError> {
        if v.is_empty() {
            return Err(ParseError {
                line: line_idx + 1,
                msg: "since requires a duration like '7d', '2w', '3m'".into(),
            });
        }
        let (num_str, unit) = v.split_at(v.len() - 1);
        let n: u32 = num_str.parse().map_err(|_| ParseError {
            line: line_idx + 1,
            msg: format!("since: invalid number in '{v}'"),
        })?;
        match unit {
            "d" => Ok(n),
            "w" => Ok(n * 7),
            "m" => Ok(n * 30),
            _ => Err(ParseError {
                line: line_idx + 1,
                msg: format!("since: unknown unit '{unit}' (use d, w, or m)"),
            }),
        }
    }

    fn parse_sort_key(v: &str, line_idx: usize) -> Result<SortKey, ParseError> {
        match v {
            "page" => Ok(SortKey::Page),
            "status" => Ok(SortKey::Status),
            "text" => Ok(SortKey::Text),
            _ => Err(ParseError {
                line: line_idx + 1,
                msg: format!("sort: must be 'page', 'status', or 'text', got '{v}'"),
            }),
        }
    }

    fn parse_usize(v: &str, line_idx: usize) -> Result<usize, ParseError> {
        v.parse::<usize>().map_err(|_| ParseError {
            line: line_idx + 1,
            msg: format!("expected a number, got '{v}'"),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_status_todo() {
            let q = parse("status: todo").unwrap();
            assert_eq!(q.filters.len(), 1);
            assert!(matches!(q.filters[0], Filter::Status(StatusFilter::Todo)));
        }

        #[test]
        fn parses_multiple_filters() {
            let q = parse("status: todo\ntag: ops\nlimit: 10").unwrap();
            assert_eq!(q.filters.len(), 2);
            assert_eq!(q.limit, Some(10));
        }

        #[test]
        fn ignores_comments() {
            let q = parse("# comment\nstatus: done").unwrap();
            assert_eq!(q.filters.len(), 1);
        }

        #[test]
        fn parses_sort() {
            let q = parse("sort: page, status").unwrap();
            assert_eq!(q.sort.len(), 2);
        }

        #[test]
        fn parses_since() {
            let q = parse("since: 2w").unwrap();
            assert!(matches!(q.filters[0], Filter::Since(14)));
        }

        #[test]
        fn rejects_unknown_key() {
            assert!(parse("bogus: value").is_err());
        }
    }
}

/// Execution engine — filter + collect matching blocks.
pub(crate) mod engine {
    use super::dsl::{Filter, KindFilter, Query, SortKey, StatusFilter};
    use chrono::{Duration, NaiveDate};
    use outl_md::block_index::BlockEntry;
    use outl_md::index::WorkspaceIndex;

    /// Task state of a block, as read off its text prefix.
    ///
    /// **This mirrors `outl_actions::TodoState`, which is the owner of
    /// the marker vocabulary.** It cannot be imported: `outl-actions`
    /// depends on this crate (for `run_code_block`), so the arrow only
    /// points one way. Adding a state there means adding it here in the
    /// same change — the pair is convention, not a compiler check.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Status {
        Todo,
        Doing,
        Done,
    }

    impl Status {
        /// Lowercase wire form used in `QueryHit.status`.
        pub fn as_str(self) -> &'static str {
            match self {
                Status::Todo => "todo",
                Status::Doing => "doing",
                Status::Done => "done",
            }
        }
    }

    /// One query hit — the data we need to render an embed.
    pub struct Hit {
        /// Block ref handle (`blk-XXXXXX`) for embed rendering.
        pub handle: String,
        /// Slug of the page hosting the block.
        pub page_slug: String,
        /// Task state, or `None` when the block is not a task.
        pub status: Option<Status>,
        /// Block text with the task prefix stripped.
        pub text: String,
    }

    /// Run `query` against `index`, returning all matching blocks.
    pub fn run(index: &WorkspaceIndex, query: &Query) -> Vec<Hit> {
        let today = chrono::Local::now().date_naive();

        index
            .iter_blocks()
            .filter_map(|entry| {
                let (status, body) = split_todo(&entry.text);
                let page = index.by_slug(&entry.source_slug);

                for f in &query.filters {
                    if !matches(f, entry, status, page.map(|p| p.is_journal), &today) {
                        return None;
                    }
                }

                Some(Hit {
                    handle: entry.ref_handle.clone(),
                    page_slug: entry.source_slug.clone(),
                    status,
                    text: body.to_string(),
                })
            })
            .collect()
    }

    /// Sort hits by the given criteria, in priority order (last key first
    /// so the first key dominates after stable sort).
    pub fn sort_hits(hits: &mut [Hit], keys: &[SortKey]) {
        for key in keys.iter().rev() {
            match key {
                SortKey::Page => hits.sort_by(|a, b| a.page_slug.cmp(&b.page_slug)),
                // Unfinished work first, in the order it moves through:
                // TODO, DOING, DONE. A non-task sorts with TODO, which
                // is where it sat before DOING existed.
                SortKey::Status => hits.sort_by(|a, b| {
                    let rank = |s: Option<Status>| s.unwrap_or(Status::Todo);
                    rank(a.status).cmp(&rank(b.status))
                }),
                SortKey::Text => hits.sort_by(|a, b| a.text.cmp(&b.text)),
            }
        }
    }

    fn matches(
        f: &Filter,
        entry: &BlockEntry,
        status: Option<Status>,
        is_journal: Option<bool>,
        today: &NaiveDate,
    ) -> bool {
        match f {
            Filter::Status(sf) => match sf {
                StatusFilter::Todo => status == Some(Status::Todo),
                StatusFilter::Doing => status == Some(Status::Doing),
                StatusFilter::Done => status == Some(Status::Done),
                // `open` has always meant "is a task", DONE included —
                // kept as-is so existing queries don't change meaning
                // under the user on an upgrade.
                StatusFilter::Open => status.is_some(),
            },
            Filter::Tag(tag) => {
                let needle = format!("#{}", tag.to_lowercase());
                entry.text_fold.contains(&needle)
            }
            Filter::Kind(kf) => match kf {
                KindFilter::Journal => is_journal == Some(true),
                KindFilter::Page => is_journal != Some(true),
            },
            Filter::Since(days) => {
                is_journal == Some(true)
                    && parse_journal_date(&entry.source_slug)
                        .map(|d| d >= *today - Duration::days(*days as i64))
                        .unwrap_or(false)
            }
            Filter::Text(needle) => entry.text_fold.contains(&needle.to_lowercase()),
        }
    }

    /// Split a block's text into `(status, body)`, accepting both the
    /// canonical word form (`"TODO body"`) and the CommonMark checkbox
    /// form (`"[ ] body"`). Mirrors `outl_actions::split_todo` — see
    /// [`Status`] for why it is a mirror and not a call.
    ///
    /// The checkbox spellings have to be here too, or a block the user
    /// wrote as `- [ ] ship it` renders a checkbox everywhere and then
    /// fails to match `status: todo`, which is the same "it's a task
    /// except where it isn't" split issue #230 was filed about.
    fn split_todo(raw: &str) -> (Option<Status>, &str) {
        const PREFIXES: [(&str, Status); 7] = [
            ("TODO ", Status::Todo),
            ("DOING ", Status::Doing),
            ("DONE ", Status::Done),
            ("[ ] ", Status::Todo),
            ("[/] ", Status::Doing),
            ("[x] ", Status::Done),
            ("[X] ", Status::Done),
        ];
        for (prefix, status) in PREFIXES {
            if let Some(rest) = raw.strip_prefix(prefix) {
                return (Some(status), rest);
            }
        }
        (None, raw)
    }

    fn parse_journal_date(slug: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(slug, "%Y-%m-%d").ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn split_todo_open() {
            assert_eq!(
                split_todo("TODO buy milk"),
                (Some(Status::Todo), "buy milk")
            );
        }

        #[test]
        fn split_todo_doing() {
            assert_eq!(
                split_todo("DOING buy milk"),
                (Some(Status::Doing), "buy milk")
            );
        }

        #[test]
        fn split_todo_done() {
            assert_eq!(
                split_todo("DONE buy milk"),
                (Some(Status::Done), "buy milk")
            );
        }

        #[test]
        fn split_todo_none() {
            assert_eq!(split_todo("just text"), (None, "just text"));
        }

        #[test]
        fn split_todo_reads_the_checkbox_spelling() {
            // A block the user typed as `- [ ] ship it` has to match
            // `status: todo`, or it renders a checkbox everywhere and
            // then goes missing from the query (issue #230).
            assert_eq!(split_todo("[ ] buy milk"), (Some(Status::Todo), "buy milk"));
            assert_eq!(
                split_todo("[/] buy milk"),
                (Some(Status::Doing), "buy milk")
            );
            assert_eq!(split_todo("[x] buy milk"), (Some(Status::Done), "buy milk"));
            // A link is not a checkbox.
            assert_eq!(
                split_todo("[x](https://example.com)"),
                (None, "[x](https://example.com)")
            );
        }

        #[test]
        fn doing_is_neither_todo_nor_done_to_a_filter() {
            // The whole point of the state: a query for open work must
            // not sweep up started work, and `status: done` must not
            // count something nobody finished.
            let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
            let text = "DOING ship the parser";
            let entry = BlockEntry {
                id: outl_core::NodeId::new(),
                ref_handle: "blk-aaaaaa".into(),
                source_slug: "notes".into(),
                source_path: std::path::PathBuf::from("pages/notes.md"),
                source_block_path: vec![0],
                text: text.into(),
                text_fold: text.to_lowercase(),
                children: Vec::new(),
            };
            let (status, _) = split_todo(&entry.text);
            for (filter, expected) in [
                (StatusFilter::Todo, false),
                (StatusFilter::Doing, true),
                (StatusFilter::Done, false),
                (StatusFilter::Open, true),
            ] {
                assert_eq!(
                    matches(&Filter::Status(filter), &entry, status, None, &today),
                    expected,
                    "{filter:?} against a DOING block"
                );
            }
        }
    }
}
