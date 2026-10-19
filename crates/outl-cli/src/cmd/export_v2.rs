//! `outl export …` — render a page in a target format.
//!
//! Three formats today:
//!
//! - `hugo` — writes a Hugo-compatible Markdown file with TOML
//!   frontmatter built from the page's `title:: / icon:: / tags::`
//!   properties. The body is the clean projection — same string the
//!   user already sees in `pages/<slug>.md`.
//! - `md` — emits the clean projection to stdout.
//! - `json` — emits the parsed AST plus sidecar entries.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{find_by_slug, is_valid_slug, page_meta, read_text_prop, render_page_md};
use outl_md::parse::parse;
use outl_md::sidecar::{self, sidecar_path_for};

use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl export …` subcommands.
#[derive(Subcommand, Debug)]
pub enum ExportCommand {
    /// Render a page as a Hugo-compatible Markdown file.
    Hugo {
        /// Page slug.
        slug: String,
        /// Output directory (file is `<slug>.md` inside it).
        #[arg(long)]
        out: PathBuf,
        /// Force JSON output (envelope on stdout; the file is still
        /// written to disk).
        #[arg(long)]
        json: bool,
    },
    /// Emit the clean Markdown projection of a page to stdout.
    Md {
        /// Page slug.
        slug: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Emit the page's AST plus sidecar entries as JSON.
    Json {
        /// Page slug.
        slug: String,
        /// Force JSON output (this command is always JSON-shaped; the
        /// flag is accepted for consistency).
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl export …` invocation.
pub fn run(cmd: &ExportCommand, path: &Path) -> i32 {
    match cmd {
        ExportCommand::Hugo { slug, out, json } => {
            let result = ws::open_read(path).and_then(|ctx| hugo(&ctx, slug, out));
            emit(*json, result, |v| {
                if let Some(p) = v.get("path").and_then(Value::as_str) {
                    println!("wrote {p}");
                }
            })
        }
        ExportCommand::Md { slug, json } => {
            let result = ws::open_read(path).and_then(|ctx| md(&ctx, slug));
            emit(*json, result, |v| {
                if let Some(s) = v.get("md").and_then(Value::as_str) {
                    println!("{s}");
                }
            })
        }
        ExportCommand::Json { slug, json } => {
            let result = ws::open_read(path).and_then(|ctx| json_ast(&ctx, slug));
            emit(*json, result, |v| {
                let pretty = serde_json::to_string_pretty(v).unwrap_or_default();
                println!("{pretty}");
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Render the page as a Hugo Markdown file and write it under `out`.
pub fn hugo(ctx: &WsCtx, slug: &str, out_dir: &Path) -> Result<Value, ApiError> {
    let id = find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;

    let body = render_page_md(&ctx.workspace, id);
    let parsed = parse(&body);
    let mut frontmatter = String::from("+++\n");
    frontmatter.push_str(&format!("title = {}\n", toml_string(&meta.title)));
    frontmatter.push_str(&format!("slug = {}\n", toml_string(&meta.slug)));
    if let Some(icon) = read_text_prop(&ctx.workspace, id, "icon") {
        frontmatter.push_str(&format!("icon = {}\n", toml_string(&icon)));
    }
    if let Some(tags) = read_text_prop(&ctx.workspace, id, "tags") {
        let split: Vec<String> = tags
            .split_whitespace()
            .map(|s| s.trim_start_matches('#').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !split.is_empty() {
            let serialized = split
                .iter()
                .map(|s| toml_string(s))
                .collect::<Vec<_>>()
                .join(", ");
            frontmatter.push_str(&format!("tags = [{serialized}]\n"));
        }
    }
    // Carry over any other top-level `key:: value` properties the
    // markdown projection already surfaces.
    for (key, value) in &parsed.properties {
        if matches!(
            key.as_str(),
            "title" | "icon" | "tags" | "page-slug" | "page-kind"
        ) {
            continue;
        }
        frontmatter.push_str(&format!("{key} = {}\n", toml_string(value)));
    }
    frontmatter.push_str("+++\n\n");

    // Strip outl's property lines from the body — Hugo gets them from
    // frontmatter. The clean body keeps just the bullet outline.
    let body_no_props = strip_property_lines(&body);

    let mut out = String::with_capacity(frontmatter.len() + body_no_props.len());
    out.push_str(&frontmatter);
    out.push_str(&body_no_props);

    // Belt-and-suspenders even though `open_or_create_page` already
    // rejects bad slugs: anything that escapes a single path component
    // here would write outside the user-supplied `out_dir`. We re-check
    // because the slug could have entered the workspace through a
    // legacy `.md` file the user imported.
    if !is_valid_slug(&meta.slug) {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            format!(
                "page slug `{}` is not safe for filesystem export",
                meta.slug
            ),
        ));
    }
    fs::create_dir_all(out_dir).map_err(ApiError::internal)?;
    let target = out_dir.join(format!("{}.md", meta.slug));
    // Final check: after the join, the target must still sit inside
    // `out_dir`. Catches anything `is_valid_slug` missed.
    if !target_within(out_dir, &target) {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            format!(
                "refusing to write outside out_dir: target {} escapes {}",
                target.display(),
                out_dir.display()
            ),
        ));
    }
    fs::write(&target, &out).map_err(ApiError::internal)?;

    Ok(json!({
        "slug": meta.slug,
        "path": target.display().to_string(),
        "bytes": out.len(),
    }))
}

/// Render to clean markdown (the same string the on-disk `.md` holds).
pub fn md(ctx: &WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))?;
    let md = render_page_md(&ctx.workspace, id);
    Ok(json!({ "slug": slug, "md": md }))
}

/// Emit AST + sidecar entries.
pub fn json_ast(ctx: &WsCtx, slug: &str) -> Result<Value, ApiError> {
    let id = find_by_slug(&ctx.workspace, slug)
        .ok_or_else(|| ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found")))?;
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "page meta missing".to_string()))?;
    let md = render_page_md(&ctx.workspace, id);
    let parsed = parse(&md);

    let md_path = outl_actions::page_md_path(&ctx.root, &meta);
    let sidecar_path = sidecar_path_for(&md_path);
    let sidecar_value = sidecar::read(&sidecar_path)
        .ok()
        .and_then(|sc| serde_json::to_value(&sc).ok());

    Ok(json!({
        "meta": meta,
        "properties": parsed.properties,
        "blocks": serde_json::to_value(&parsed.blocks).map_err(ApiError::internal)?,
        "sidecar": sidecar_value,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn toml_string(s: &str) -> String {
    // Naive but safe enough for property values that come from the
    // outl workspace — we escape backslashes and double quotes only.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// True iff `target` sits inside `root`, after canonicalisation
/// when possible. We canonicalise to defeat symlink tricks; if
/// canonicalisation fails (the path doesn't exist yet) we fall back
/// to a literal component prefix check, which is safe because
/// `is_valid_slug` already rejected `..`.
fn target_within(root: &std::path::Path, target: &std::path::Path) -> bool {
    let root_canonical = root.canonicalize();
    let target_canonical = target.canonicalize();
    match (root_canonical, target_canonical) {
        (Ok(r), Ok(t)) => t.starts_with(&r),
        _ => target.starts_with(root),
    }
}

fn strip_property_lines(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut in_props = true;
    for line in md.lines() {
        if in_props {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((_, _)) = trimmed.split_once("::") {
                // Skip — it's a property line at the top of the page.
                continue;
            }
            in_props = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
