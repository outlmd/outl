//! `outl query …` — structured filter over pages and blocks.
//!
//! v1 surface: `--tag=foo`, `--priority=p1`, `--since=Nd`,
//! `--kind=page|journal`, plus the negatives `--not-tag` and
//! `--not-prop`. Filters are AND-composed. `--raw` is reserved for the
//! query DSL (not yet implemented) and currently returns `INVALID_ARG`.
//!
//! **This filters pages; the ` ```query ` DSL filters blocks.** They
//! share a name and not a matcher: `--tag=ops` asks whether the page's
//! subtree mentions `#ops` exactly (`outl_md::text_contains_tag`),
//! while the DSL's `tag: ops` also answers for `#ops/deploy`. Each
//! negative is the exact complement of the positive *on its own
//! surface*, which is the property that matters — `--tag=x` together
//! with `--not-tag=x` returns nothing.

use std::path::Path;

use chrono::{Duration, NaiveDate};
use clap::Args;
use serde_json::{json, Value};

use outl_actions::{today, PageMeta};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// Args for `outl query`.
#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Page must mention `#<tag>` somewhere in its subtree. Matched
    /// exactly and case-sensitively: `#ops/deploy` is a different tag
    /// from `#ops`.
    #[arg(long)]
    pub tag: Option<String>,
    /// Page must NOT mention `#<tag>` anywhere in its subtree. Same
    /// exact, case-sensitive match as `--tag`. Repeatable; a page
    /// carrying any of them is dropped.
    #[arg(long = "not-tag", value_name = "TAG")]
    pub not_tags: Vec<String>,
    /// Page must carry property `priority::` matching this value.
    #[arg(long)]
    pub priority: Option<String>,
    /// Exclude pages whose `priority::` matches this value.
    #[arg(long = "not-priority", value_name = "PRIORITY")]
    pub not_priority: Option<String>,
    /// Generic property filter: `--prop key=value`, or `--prop key`
    /// for "carries this property at all". Reads the page's own
    /// `key::` property, not properties on blocks inside it. Key and
    /// value are matched case-sensitively. Repeatable.
    #[arg(long = "prop", value_name = "KEY[=VALUE]")]
    pub props: Vec<String>,
    /// Exclude pages matching a property: `--not-prop key=value`, or
    /// `--not-prop key` for "carries this property at all". Same
    /// page-level, case-sensitive match as `--prop`. Repeatable.
    #[arg(long = "not-prop", value_name = "KEY[=VALUE]")]
    pub not_props: Vec<String>,
    /// Only return journals whose date is within the last N days
    /// (`7d`, `30d`, …) or after an explicit ISO date.
    #[arg(long)]
    pub since: Option<String>,
    /// Exclude what `--since` would have kept — so dated pages on or
    /// after the cutoff go, and undated pages go with them. Read it
    /// as `!--since`, not as "older than".
    #[arg(long = "not-since", value_name = "SINCE")]
    pub not_since: Option<String>,
    /// Restrict to a single page kind: `page` | `journal`.
    #[arg(long)]
    pub kind: Option<String>,
    /// Exclude a single page kind: `page` | `journal`.
    #[arg(long = "not-kind", value_name = "KIND")]
    pub not_kind: Option<String>,
    /// Reserved for the query DSL (not yet implemented) — currently rejected.
    #[arg(long)]
    pub raw: Option<String>,
    /// Force JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Run a `outl query` invocation.
pub fn run(args: &QueryArgs, path: &Path) -> i32 {
    let result = ws::open(path).and_then(|ctx| handler(&ctx, args));
    emit(args.json, result, |v| {
        if let Some(items) = v.get("results").and_then(Value::as_array) {
            for item in items {
                let slug = item.get("slug").and_then(Value::as_str).unwrap_or("?");
                let kind = item.get("kind").and_then(Value::as_str).unwrap_or("page");
                let title = item.get("title").and_then(Value::as_str).unwrap_or("?");
                println!("{kind:8}  {slug:30}  {title}");
            }
        }
    })
}

/// Pure handler — used by both CLI and MCP shim.
pub fn handler(ctx: &WsCtx, args: &QueryArgs) -> Result<Value, ApiError> {
    if args.raw.is_some() {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "--raw is reserved for the query DSL and not yet implemented".to_string(),
        ));
    }

    check_tag_filters(args)?;
    let cutoff = args.since.as_deref().map(parse_since).transpose()?;
    let not_cutoff = args.not_since.as_deref().map(parse_since).transpose()?;
    let parsed_props = parse_prop_filters(&args.props, args.priority.as_deref())?;
    let parsed_not_props = parse_prop_filters(&args.not_props, args.not_priority.as_deref())?;

    let mut matches: Vec<Value> = Vec::new();
    for meta in outl_actions::list_pages(&ctx.workspace) {
        if let Some(kind) = &args.kind {
            if meta.kind.as_str() != kind {
                continue;
            }
        }
        // Each negative is `!` over the positive's own predicate, so
        // `--kind=x --not-kind=x` returns nothing and neither side can
        // drift into a second opinion about what `x` means.
        if let Some(kind) = &args.not_kind {
            if meta.kind.as_str() == kind {
                continue;
            }
        }
        if let Some(start) = cutoff {
            if !journal_after(&meta, start) {
                continue;
            }
        }
        if let Some(start) = not_cutoff {
            if journal_after(&meta, start) {
                continue;
            }
        }
        let id = match ulid::Ulid::from_string(&meta.id) {
            Ok(u) => NodeId(u),
            Err(_) => continue,
        };

        if let Some(tag) = &args.tag {
            if !super::page::page_has_tag(&ctx.workspace, id, tag) {
                continue;
            }
        }
        if args
            .not_tags
            .iter()
            .any(|tag| super::page::page_has_tag(&ctx.workspace, id, tag))
        {
            continue;
        }

        let mut props_ok = true;
        for f in &parsed_props {
            if !page_property_matches(&ctx.workspace, id, f) {
                props_ok = false;
                break;
            }
        }
        if !props_ok {
            continue;
        }
        if parsed_not_props
            .iter()
            .any(|f| page_property_matches(&ctx.workspace, id, f))
        {
            continue;
        }

        matches.push(json!({
            "id": meta.id,
            "slug": meta.slug,
            "title": meta.title,
            "kind": meta.kind,
        }));
    }
    Ok(json!({
        "count": matches.len(),
        "results": matches,
    }))
}

fn parse_since(s: &str) -> Result<NaiveDate, ApiError> {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_suffix('d') {
        let days: i64 = rest.parse().map_err(|_| {
            ApiError::new(
                codes::INVALID_ARG,
                format!("--since `{s}` is not a valid `Nd` form"),
            )
        })?;
        return Ok(today() - Duration::days(days));
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d").map_err(|_| {
        ApiError::new(
            codes::INVALID_ARG,
            format!("--since `{s}` is neither `Nd` nor ISO YYYY-MM-DD"),
        )
    })
}

/// One parsed `--prop` / `--not-prop` target.
///
/// `value: None` means "carries this property at all", which is what
/// makes `--not-prop status` expressible — the common case of parking
/// a page under a property whose value you do not want to enumerate.
struct PropFilter {
    key: String,
    value: Option<String>,
}

/// Reject a tag filter that would silently do nothing.
///
/// `--not-tag ""` reaches `text_contains_tag` with an empty name, the
/// tokenizer never emits one, so the filter never fires and the user
/// gets back every page they asked to hide. The prop filters already
/// refuse their version of this typo; tags have the same duty, and the
/// negative is the direction where staying silent hands back more than
/// was asked for.
fn check_tag_filters(args: &QueryArgs) -> Result<(), ApiError> {
    let empty = args
        .tag
        .iter()
        .chain(args.not_tags.iter())
        .any(|t| t.trim().trim_start_matches('#').trim().is_empty());
    if empty {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "tag filter is empty — pass a tag name, e.g. `--not-tag someday`".to_string(),
        ));
    }
    Ok(())
}

fn parse_prop_filters(
    props: &[String],
    priority: Option<&str>,
) -> Result<Vec<PropFilter>, ApiError> {
    let mut out: Vec<PropFilter> = Vec::new();
    if let Some(p) = priority {
        out.push(PropFilter {
            key: "priority".to_string(),
            value: Some(p.to_string()),
        });
    }
    for raw in props {
        let (key, value) = match raw.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v.trim())),
            None => (raw.trim(), None),
        };
        if key.is_empty() {
            return Err(ApiError::new(
                codes::INVALID_ARG,
                format!("property filter must be `KEY` or `KEY=VALUE`, got `{raw}`"),
            ));
        }
        // An empty value is a typo, not a wildcard: reading
        // `--not-prop status=` as "any status" drops far more pages
        // than the user asked for, and says nothing about it.
        if value == Some("") {
            return Err(ApiError::new(
                codes::INVALID_ARG,
                format!("property filter `{raw}` has no value — drop the `=` to match any value"),
            ));
        }
        // `--not-prop "status: done"` is the ` ```query ` fence's
        // spelling. Taken literally it becomes a key named
        // `status: done`, which no page carries, so the exclusion
        // never fires and the command hands back exactly the pages it
        // was told to drop. Fail loudly on the separator instead.
        if key.contains(':') {
            return Err(ApiError::new(
                codes::INVALID_ARG,
                format!(
                    "property filter `{raw}` uses the query-fence separator — \
                     on the CLI it is `KEY=VALUE`, not `KEY: VALUE`"
                ),
            ));
        }
        out.push(PropFilter {
            key: key.to_string(),
            value: value.map(str::to_string),
        });
    }
    Ok(out)
}

fn journal_after(meta: &PageMeta, cutoff: NaiveDate) -> bool {
    if let Some(d) = outl_actions::date_from_slug(&meta.slug) {
        return d >= cutoff;
    }
    // Non-journals are kept; date filter only narrows down dated pages.
    !matches!(meta.kind, outl_actions::PageKind::Journal)
}

fn page_property_matches(
    workspace: &outl_core::workspace::Workspace,
    page: NodeId,
    filter: &PropFilter,
) -> bool {
    let Some(prop) = workspace.tree().property(page, &filter.key) else {
        return false;
    };
    let Some(expected) = filter.value.as_deref() else {
        // Key-only filter: presence is the whole question.
        return true;
    };
    match prop {
        PropValue::Text(s) | PropValue::Tag(s) | PropValue::PageRef(s) => s == expected,
        PropValue::List(items) => items.iter().any(|v| match v {
            PropValue::Text(s) | PropValue::Tag(s) | PropValue::PageRef(s) => s == expected,
            _ => false,
        }),
    }
}
