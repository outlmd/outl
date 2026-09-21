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
//! Both converge on the same `Query` + `engine::run` pipeline, and both
//! spell a filter the same way — [`QueryParams::not_prop`] takes the
//! `"key: value"` string the DSL takes, rather than inventing a second
//! shape for the same fact.

use std::path::Path;
use std::time::Instant;

use outl_md::index::WorkspaceIndex;

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};

pub(crate) mod dsl;
pub(crate) mod engine;

// ── Public query API (used by both ```query and plugin SDK) ─────────────

/// Structured query parameters — the plugin-facing API.
///
/// Every field is optional; an empty struct matches every block.
/// This is the shape that `outl.query({ … })` deserialises from JS.
#[derive(Debug, Default, Clone)]
pub struct QueryParams {
    /// `"todo"`, `"doing"`, `"done"`, or `"open"` (any task, DONE
    /// included).
    pub status: Option<String>,
    /// Tag match (without `#`, though one is tolerated).
    /// `#tag/child` matches its parent name.
    pub tag: Option<String>,
    /// Block properties to require, each `"key"` or `"key: value"`.
    pub prop: Vec<String>,
    /// `"journal"` or `"page"`.
    pub kind: Option<String>,
    /// Duration like `"7d"`, `"2w"`, `"3m"`.
    pub since: Option<String>,
    /// Substring search (case-insensitive).
    pub text: Option<String>,

    // ── Negatives ────────────────────────────────────────────────
    //
    // One per positive above, taking the same values and parsed by
    // the same code. They are not a separate matcher: each becomes a
    // `Filter::Not` around the filter its positive would have built.
    /// Exclude blocks in this state.
    pub not_status: Option<String>,
    /// Exclude blocks carrying **any** of these tags.
    pub not_tag: Vec<String>,
    /// Exclude blocks matching **any** of these properties.
    pub not_prop: Vec<String>,
    /// Exclude blocks hosted by a page of this kind.
    pub not_kind: Option<String>,
    /// Exclude blocks [`QueryParams::since`] would have matched — so
    /// everything that is *not* a journal inside the window, which
    /// includes ordinary pages. It is the exact complement, and that
    /// is the surprising part: read it as `!since`, not as "older
    /// than".
    pub not_since: Option<String>,
    /// Exclude blocks whose text contains this substring.
    pub not_text: Option<String>,

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
    /// `"todo"`, `"doing"`, `"done"`, or `None` when the block is not
    /// a task.
    pub status: Option<String>,
    /// Block text with the task prefix stripped, in either spelling
    /// (`TODO ` / `[ ] `) and behind an optional `"> "` quote marker.
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

/// Run `query` against an index the caller already holds.
///
/// This is the path every caller should be on: building a
/// `WorkspaceIndex` costs a walk of every `.md` plus every sidecar, and
/// a page with several ` ```query ` fences would otherwise pay it once
/// per fence.
pub fn run_query_dsl_with_index(
    dsl: &str,
    index: &WorkspaceIndex,
) -> Result<Vec<QueryHit>, String> {
    let query = dsl::parse(dsl).map_err(|e| e.to_string())?;
    Ok(run_against(&query, index))
}

fn run_query_internal(query: &dsl::Query, workspace_root: &Path) -> Result<Vec<QueryHit>, String> {
    let index = WorkspaceIndex::build(workspace_root);
    Ok(run_against(query, &index))
}

fn run_against(query: &dsl::Query, index: &WorkspaceIndex) -> Vec<QueryHit> {
    let mut hits = engine::run(index, query);
    engine::sort_hits(&mut hits, &query.sort);
    if let Some(limit) = query.limit {
        hits.truncate(limit);
    }
    hits.into_iter()
        .map(|h| QueryHit {
            handle: h.handle,
            page: h.page_slug,
            status: h.status.map(|s| s.as_str().to_string()),
            text: h.text,
        })
        .collect()
}

fn build_query_from_params(p: &QueryParams) -> Result<dsl::Query, String> {
    let mut filters = Vec::new();

    // Each row is (positive value, negative value, builder). Pairing
    // them here is what keeps `not_x` from acquiring its own parser:
    // both sides go through one builder and only the wrapping differs.
    let scalars: [ScalarPair<'_>; 4] = [
        (&p.status, &p.not_status, status_filter),
        (&p.kind, &p.not_kind, kind_filter),
        (&p.since, &p.not_since, since_filter),
        (&p.text, &p.not_text, text_filter),
    ];
    for (yes, no, build) in scalars {
        if let Some(v) = yes {
            filters.push(build(v)?);
        }
        if let Some(v) = no {
            filters.push(negate(build(v)?));
        }
    }

    if let Some(t) = &p.tag {
        filters.push(tag_filter(t)?);
    }
    for t in &p.not_tag {
        filters.push(negate(tag_filter(t)?));
    }
    for raw in &p.prop {
        filters.push(prop_filter(raw)?);
    }
    for raw in &p.not_prop {
        filters.push(negate(prop_filter(raw)?));
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

/// A positive field, its negative twin, and the one builder both use.
type ScalarPair<'a> = (
    &'a Option<String>,
    &'a Option<String>,
    fn(&str) -> Result<dsl::Filter, String>,
);

fn negate(f: dsl::Filter) -> dsl::Filter {
    dsl::Filter::Not(Box::new(f))
}

fn status_filter(s: &str) -> Result<dsl::Filter, String> {
    Ok(dsl::Filter::Status(match s {
        "todo" => dsl::StatusFilter::Todo,
        "doing" => dsl::StatusFilter::Doing,
        "done" => dsl::StatusFilter::Done,
        "open" => dsl::StatusFilter::Open,
        other => {
            return Err(format!(
                "invalid status '{other}' (use todo|doing|done|open)"
            ))
        }
    }))
}

fn kind_filter(s: &str) -> Result<dsl::Filter, String> {
    Ok(dsl::Filter::Kind(match s {
        "journal" => dsl::KindFilter::Journal,
        "page" => dsl::KindFilter::Page,
        other => return Err(format!("invalid kind '{other}' (use journal|page)")),
    }))
}

fn since_filter(s: &str) -> Result<dsl::Filter, String> {
    Ok(dsl::Filter::Since(dsl::parse_duration_str(s)?))
}

fn text_filter(s: &str) -> Result<dsl::Filter, String> {
    if s.is_empty() {
        return Err("text requires something to search for".into());
    }
    Ok(dsl::Filter::Text(s.to_string()))
}

fn tag_filter(s: &str) -> Result<dsl::Filter, String> {
    Ok(dsl::Filter::Tag(dsl::TagFilter::new(s)?))
}

fn prop_filter(s: &str) -> Result<dsl::Filter, String> {
    Ok(dsl::Filter::Prop(dsl::PropFilter::new(s)?))
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

    fn needs_workspace_index(&self) -> bool {
        true
    }

    fn execute(&self, source: &str, ctx: &ExecContext<'_>) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();

        // Prefer the caller's index. Falling back to a disk build keeps
        // every existing caller working, but it re-reads the whole
        // workspace per fence — see `ExecContext::index`.
        let hits = match &ctx.index {
            Some(index) => run_query_dsl_with_index(source, index),
            None => run_query_dsl(source, &ctx.workspace_root),
        }
        .map_err(ExecError::Language)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_params_reject_a_malformed_negative_filter() {
        // The structured API has no line numbers to report, so the
        // message has to stand on its own.
        let err = build_query_from_params(&QueryParams {
            not_tag: vec![String::new()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.contains("tag requires a name"), "got {err:?}");

        let err = build_query_from_params(&QueryParams {
            not_prop: vec!["status:".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.contains("no value"), "got {err:?}");
    }

    #[test]
    fn every_negative_entry_becomes_its_own_filter() {
        let q = build_query_from_params(&QueryParams {
            not_tag: vec!["research".into(), "future".into(), "someday".into()],
            not_prop: vec!["status".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(q.filters.len(), 4);
    }
}
