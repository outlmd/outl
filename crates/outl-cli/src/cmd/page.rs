//! `outl page …` — page-level operations.
//!
//! Each handler returns a `serde_json::Value` so the same body powers
//! both the CLI (`emit` → human or `--json`) and the MCP shim (raw
//! payload returned as the tool result).

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{
    find_by_slug, list_pages, open_or_create_page, page_meta, project_outline, read_text_prop,
    render_page_md, set_property, walk_subtree, BlockTreeSpec, PageKind,
};
use outl_core::id::NodeId;
use outl_core::property::PropValue;

use super::page_merge;
pub use super::page_rename::rename;
pub use super::page_update::update;
use crate::human::print_outline_tree;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl page …` subcommands.
#[derive(Subcommand, Debug)]
pub enum PageCommand {
    /// Get a page (meta + outline).
    Get {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Create a new page.
    ///
    /// Pass `--content` with a JSON forest (`[{text, children?}, ...]`)
    /// — or `--content -` to read it from stdin — to populate the
    /// page's outline in a single call. Skipping `--content` keeps
    /// the historical "empty page" behavior.
    Create {
        /// Page slug (filename-safe, e.g. `ideas`). With `--slugify`,
        /// pass a human name instead (`"My Ideas"`) and it is converted.
        slug: String,
        /// Page title (defaults to slug when omitted).
        #[arg(long)]
        title: Option<String>,
        /// Page icon (single emoji or short string).
        #[arg(long)]
        icon: Option<String>,
        /// Initial page content as JSON forest. Pass `-` to read from stdin.
        #[arg(long)]
        content: Option<String>,
        /// Treat the positional argument as a human name and derive the
        /// slug from it via the shared `outl_md::slugify` rule (lowercase,
        /// fold accents, non-alnum → `-`). Idempotent on an already-clean
        /// slug. Opt-in so the default path stays literal — the MCP layer
        /// and hierarchical slugs (`ai-agent/learning`) keep `/` verbatim.
        #[arg(long)]
        slugify: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Update a page's metadata (title and/or icon).
    Update {
        /// Page slug to update.
        slug: String,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New icon (use `--icon=""` to clear).
        #[arg(long)]
        icon: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Delete a page (moves it to the trash root; op stays in the log).
    Delete {
        /// Page slug.
        slug: String,
        /// Required to actually delete.
        #[arg(long)]
        confirm: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List every page in the workspace.
    List {
        /// Optional filter expression: `tag:foo` or `kind:journal|page`.
        #[arg(long)]
        filter: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Rename a page slug and rewrite `[[old_slug]]` / `#old_slug`
    /// mentions in every block (`rewritten_refs` lists the blocks).
    /// Fails with `SLUG_CONFLICT` when the new slug is another page,
    /// unless `--merge` folds the page into that one.
    Rename {
        /// Current slug.
        old_slug: String,
        /// New slug.
        new_slug: String,
        /// When `new_slug` is already a page, merge into it instead of
        /// failing. The existing page keeps its name.
        #[arg(long)]
        merge: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Merge `source` into `target`: move its blocks, merge its
    /// properties, rewrite every `[[source]]` / `#source` mention to the
    /// target, keep the source's names as aliases of the target and move
    /// the source to the trash. Safe to run again.
    Merge {
        /// Page to fold away (slug).
        source: String,
        /// Page that keeps the content (slug).
        target: String,
        /// Required to actually merge.
        #[arg(long, visible_alias = "yes")]
        confirm: bool,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Render a page to clean `.md`.
    Render {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Page-level property reads/writes.
    Prop {
        #[command(subcommand)]
        sub: super::prop::PropCommand,
    },
    /// Show what changed on a page, newest first, from the op log.
    ///
    /// Read-only. Includes blocks that were deleted out of the page —
    /// that is usually what someone opens a history to find.
    History {
        /// Page slug.
        slug: String,
        /// How many events to show. The total is always reported.
        #[arg(long, default_value_t = super::history::DEFAULT_LIMIT)]
        limit: usize,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl page …` invocation.
pub fn run(cmd: &PageCommand, path: &Path) -> i32 {
    match cmd {
        PageCommand::Get { slug, json } => {
            let result = ws::open_read(path).and_then(|mut ctx| get(&mut ctx, slug));
            emit(*json, result, print_page_get)
        }
        PageCommand::Create {
            slug,
            title,
            icon,
            content,
            slugify,
            json,
        } => {
            // `--slugify` turns a human name into a filename-safe slug via
            // the shared rule (one owner: `outl_md::slugify`). Default
            // leaves the positional verbatim so the MCP path and
            // hierarchical slugs are untouched.
            let slug = if *slugify {
                outl_md::slug::slugify(slug)
            } else {
                slug.clone()
            };
            let result = read_content_arg(content.as_deref()).and_then(|specs| {
                ws::open(path).and_then(|mut ctx| {
                    create(
                        &mut ctx,
                        &slug,
                        title.as_deref(),
                        icon.as_deref(),
                        specs.as_deref(),
                    )
                })
            });
            emit(*json, result, |v| print_page_meta("created", v))
        }
        PageCommand::Update {
            slug,
            title,
            icon,
            json,
        } => {
            let result = ws::open(path)
                .and_then(|mut ctx| update(&mut ctx, slug, title.as_deref(), icon.as_deref()));
            emit(*json, result, |v| print_page_meta("updated", v))
        }
        PageCommand::Delete {
            slug,
            confirm,
            json,
        } => {
            if !*confirm {
                let err = ApiError::new(
                    codes::CONFIRM_REQUIRED,
                    format!("refusing to delete page `{slug}` without --confirm"),
                );
                return emit::<Value, _>(*json, Err(err), |_| {});
            }
            let result = ws::open(path).and_then(|mut ctx| delete(&mut ctx, slug));
            emit(*json, result, |v| {
                println!(
                    "deleted: {}",
                    v.get("slug").and_then(Value::as_str).unwrap_or("?")
                );
            })
        }
        PageCommand::List { filter, json } => {
            let result = ws::open_read(path).and_then(|ctx| list(&ctx, filter.as_deref()));
            emit(*json, result, print_page_list)
        }
        PageCommand::Rename {
            old_slug,
            new_slug,
            merge,
            json,
        } => {
            let result =
                ws::open(path).and_then(|mut ctx| rename(&mut ctx, old_slug, new_slug, *merge));
            emit(*json, result, |v| {
                if v.get("outcome").and_then(Value::as_str) == Some("merged") {
                    return page_merge::print_human(v);
                }
                print_page_meta("renamed", v);
                let rewritten = v
                    .get("rewritten_refs")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                if rewritten > 0 {
                    println!("rewrote {rewritten} reference(s) to the old name");
                }
            })
        }
        PageCommand::Merge {
            source,
            target,
            confirm,
            json,
        } => page_merge::run(path, source, target, *confirm, *json),
        PageCommand::Render { slug, json } => {
            let result = ws::open_read(path).and_then(|ctx| render(&ctx, slug));
            emit(*json, result, |v| {
                if let Some(md) = v.get("md").and_then(Value::as_str) {
                    println!("{md}");
                }
            })
        }
        PageCommand::Prop { sub } => super::prop::run(sub, path),
        PageCommand::History { slug, limit, json } => {
            super::history::run_page(path, slug, *limit, *json)
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Resolve a slug to a page node id or return [`codes::PAGE_NOT_FOUND`].
pub(super) fn resolve(ctx: &WsCtx, slug: &str) -> Result<NodeId, ApiError> {
    find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))
}

/// Page meta + outline tree.
pub fn get(ctx: &mut WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, slug)?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    let outline = project_outline(&ctx.workspace, id);
    let icon = page_property(&ctx.workspace, id, "icon");
    Ok(json!({
        "meta": serde_json::to_value(&meta).map_err(ApiError::internal)?,
        "icon": icon,
        "outline": serde_json::to_value(&outline).map_err(ApiError::internal)?,
    }))
}

/// Read `--content` arg: `None` (skip), a JSON literal, or `-` (stdin).
/// Returns the parsed forest so the caller can pass it to `create`.
pub(crate) fn read_content_arg(arg: Option<&str>) -> Result<Option<Vec<BlockTreeSpec>>, ApiError> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    let raw = if arg == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("read stdin: {e}")))?;
        buf
    } else {
        arg.to_string()
    };
    parse_content_payload(&raw).map(Some)
}

/// Parse the page content payload — either a forest array
/// (`[{...}, {...}]`) or a single root spec (`{...}`) for ergonomics.
pub(crate) fn parse_content_payload(raw: &str) -> Result<Vec<BlockTreeSpec>, ApiError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| ApiError::new(codes::INVALID_ARG, format!("invalid content JSON: {e}")))?;
    let specs: Vec<BlockTreeSpec> = if value.is_array() {
        serde_json::from_value(value).map_err(|e| {
            ApiError::new(codes::INVALID_ARG, format!("invalid content forest: {e}"))
        })?
    } else {
        let spec: BlockTreeSpec = serde_json::from_value(value).map_err(|e| {
            ApiError::new(codes::INVALID_ARG, format!("invalid content shape: {e}"))
        })?;
        vec![spec]
    };
    Ok(specs)
}

/// Create a new page (idempotent on slug — re-creating returns the same
/// meta).
///
/// When `content` is provided, every top-level spec is appended in
/// order as a child of the page root. The ids land in the response
/// under `content` so the caller can address them in follow-ups.
pub fn create(
    ctx: &mut WsCtx,
    slug: &str,
    title: Option<&str>,
    icon: Option<&str>,
    content: Option<&[BlockTreeSpec]>,
) -> Result<Value, ApiError> {
    let display_title = title.unwrap_or(slug);
    let id = open_or_create_page(
        &mut ctx.workspace,
        &ctx.hlc,
        slug,
        display_title,
        PageKind::Page,
    )?;

    if let Some(value) = icon {
        if !value.is_empty() {
            write_page_property(ctx, id, "icon", Some(value))?;
        }
    }

    let content_outcome = match content {
        Some(specs) if !specs.is_empty() => {
            Some(super::block::append_forest_under(ctx, id, specs)?)
        }
        _ => None,
    };

    write_projection(ctx, id)?;
    let mut payload = page_meta_value(ctx, id)?;
    if let Some(outcomes) = content_outcome {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("content".to_string(), serde_json::Value::Array(outcomes));
        }
    }
    Ok(payload)
}

/// Delete a page by moving its root to the trash. The op stays in the
/// log so peers converge; the on-disk `.md` + sidecar projection is
/// dropped so the page disappears from this device's listings right
/// away.
///
/// The CRDT half lives in [`outl_actions::page::delete`] (shared with
/// the TUI sidebar and the desktop / mobile clients); this wrapper
/// owns only the projection cleanup and the JSON envelope.
pub fn delete(ctx: &mut WsCtx, slug: &str) -> Result<Value, ApiError> {
    let meta =
        outl_actions::delete_page(&mut ctx.workspace, &ctx.hlc, slug).map_err(|e| match e {
            outl_actions::ActionError::PageNotFound(_) => {
                ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found"))
            }
            other => ApiError::internal(other),
        })?;

    // Remove the on-disk projection so peers don't see a stale page.
    // We log failures so a broken FS doesn't silently leave orphans —
    // `outl doctor` would flag them, but the operator deserves to know
    // immediately.
    if let Err(e) = outl_actions::remove_page_projection(&ctx.root, &meta) {
        tracing::warn!(
            target: "outl::cmd::page",
            "could not remove page projection for {}: {e}",
            meta.slug
        );
    }

    Ok(json!({
        "slug": meta.slug,
        "id": meta.id,
    }))
}

/// List every page, optionally filtered.
pub fn list(ctx: &WsCtx, filter: Option<&str>) -> Result<Value, ApiError> {
    let metas = list_pages(&ctx.workspace);
    let kept: Vec<_> = match filter {
        None => metas,
        Some(expr) => {
            let (key, value) = expr.split_once(':').ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARG,
                    format!("filter must be `key:value`, got `{expr}`"),
                )
            })?;
            let key = key.trim();
            let value = value.trim();
            metas
                .into_iter()
                .filter(|m| match key {
                    "kind" => m.kind.as_str() == value,
                    "tag" => {
                        let node = ulid::Ulid::from_string(&m.id).ok().map(NodeId);
                        node.map(|n| page_has_tag(&ctx.workspace, n, value))
                            .unwrap_or(false)
                    }
                    _ => true,
                })
                .collect()
        }
    };
    Ok(json!({ "pages": kept }))
}

/// Render a page's outline to clean markdown.
pub fn render(ctx: &WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, slug)?;
    let md = render_page_md(&ctx.workspace, id);
    Ok(json!({ "slug": slug, "md": md }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn page_meta_value(ctx: &WsCtx, id: NodeId) -> Result<Value, ApiError> {
    Ok(json!({ "meta": page_meta_inner(ctx, id)? }))
}

pub(super) fn page_meta_inner(ctx: &WsCtx, id: NodeId) -> Result<Value, ApiError> {
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    serde_json::to_value(&meta).map_err(ApiError::internal)
}

/// Read a page property into a JSON value (string when text; structured
/// form for `List`/`Tag`/`PageRef`).
fn page_property(workspace: &outl_core::workspace::Workspace, id: NodeId, key: &str) -> Value {
    match read_text_prop(workspace, id, key) {
        Some(s) => Value::String(s),
        None => Value::Null,
    }
}

/// Wrap `outl_actions::set_property` with `ApiError` mapping. `None`
/// clears the property.
pub(super) fn write_page_property(
    ctx: &mut WsCtx,
    id: NodeId,
    key: &str,
    value: Option<&str>,
) -> Result<(), ApiError> {
    let val = value.map(|v| PropValue::Text(v.to_string()));
    set_property(&mut ctx.workspace, &ctx.hlc, id, key, val).map_err(ApiError::internal)
}

/// Re-render `id`'s `.md` + sidecar after a mutation.
///
/// Routes through the **guarded** projection — the same one every GUI
/// write path uses (`outl-tauri-shared/CLAUDE.md` → "A page that
/// stopped syncing") — rather than the unconditional one: a page whose
/// `.md` holds content the op log never recorded must refuse the
/// write and surface `ActionError::PageMarkdownAheadOfLog`, not
/// silently delete it. Before this, `outl page update` / `outl block
/// append` (and every other CLI mutation) used the unconditional
/// writer, so the CLI/MCP surface was the one place invariant 8's
/// guard was never wired in (RFC 0255).
pub(super) fn write_projection(ctx: &mut WsCtx, id: NodeId) -> Result<(), ApiError> {
    // The mutation already ran at the call site, so this commits an
    // empty one purely for the projection and the steps around it. The
    // shape is worth keeping: when this context grows a transport, the
    // announce lands here for every caller at once.
    ctx.commit(id)?;
    Ok(())
}

/// True when any block in `page`'s subtree mentions `#tag` as a whole
/// tag token (boundary-correct: `#tag-longer` does not match `tag`).
/// Shared by `page list --filter tag:` and `outl query --tag`.
pub(crate) fn page_has_tag(
    workspace: &outl_core::workspace::Workspace,
    page: NodeId,
    tag: &str,
) -> bool {
    let mut found = false;
    walk_subtree(workspace, page, |id| {
        if let Some(text) = workspace.block_text(id) {
            if outl_md::text_contains_tag(&text, tag) {
                found = true;
                return false; // early-stop
            }
        }
        true
    });
    found
}

// ---------------------------------------------------------------------------
// Human-readable formatters
// ---------------------------------------------------------------------------

fn print_page_get(v: &Value) {
    let title = v
        .pointer("/meta/title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)");
    let slug = v
        .pointer("/meta/slug")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = v
        .pointer("/meta/kind")
        .and_then(Value::as_str)
        .unwrap_or("page");
    println!("{title}  ({slug}, {kind})");
    if let Some(outline) = v.get("outline").and_then(Value::as_array) {
        print_outline_tree(outline, 0);
    }
}

fn print_page_meta(verb: &str, v: &Value) {
    let slug = v
        .pointer("/meta/slug")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let title = v
        .pointer("/meta/title")
        .and_then(Value::as_str)
        .unwrap_or("?");
    println!("{verb}: {slug} ({title})");
}

fn print_page_list(v: &Value) {
    let pages = v.get("pages").and_then(Value::as_array);
    let Some(pages) = pages else {
        return;
    };
    if pages.is_empty() {
        println!("no pages");
        return;
    }
    for page in pages {
        let slug = page.get("slug").and_then(Value::as_str).unwrap_or("?");
        let title = page.get("title").and_then(Value::as_str).unwrap_or("?");
        let kind = page.get("kind").and_then(Value::as_str).unwrap_or("page");
        println!("{kind:8}  {slug:30}  {title}");
    }
}
