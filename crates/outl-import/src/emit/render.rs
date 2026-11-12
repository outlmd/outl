//! IR → outl markdown. The single owner of output syntax.
//!
//! Block refs and embeds are emitted as inert placeholders — refs as
//! `((outl-import:<uid>))`, embeds as `!((outl-import-embed:<uid>))` —
//! deliberately not `blk-`-prefixed, so outl's tokenizer treats them
//! as plain text until the resolve pass rewrites them into real
//! handles. The two prefixes are distinct so resolve knows ref-vs-embed
//! from the marker itself.

use crate::ir::{
    BlockContent, EmbedTarget, ImportBlock, ImportGraph, ImportPage, Inline, PageBody, PageName,
};
use crate::report::ImportReport;
use crate::ImportOptions;
use chrono::NaiveDate;
use outl_md::slug::slugify;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// UID marker prefix of the interim block-ref form.
pub(crate) const PLACEHOLDER_PREFIX: &str = "outl-import:";

/// UID marker prefix of the interim embed form. Distinct from
/// [`PLACEHOLDER_PREFIX`] so the resolve pass knows ref-vs-embed from
/// the marker itself — sniffing the preceding `!` would misclassify a
/// user's text that happens to end in `!` glued to a ref
/// (`Done!((uid))`).
pub(crate) const PLACEHOLDER_EMBED_PREFIX: &str = "outl-import-embed:";

/// Interim block-ref form.
pub(crate) fn placeholder_ref(uid: &str) -> String {
    format!("(({PLACEHOLDER_PREFIX}{uid}))")
}

/// Interim embed form. The leading `!` is already the final outl
/// syntax — resolve only rewrites the `((…))` part.
pub(crate) fn placeholder_embed(uid: &str) -> String {
    format!("!(({PLACEHOLDER_EMBED_PREFIX}{uid}))")
}

/// One page ready to hit the disk.
pub struct RenderedPage {
    /// Destination `.md` path.
    pub path: PathBuf,
    /// On-disk file stem.
    pub stem: String,
    /// Human title (Named) or ISO date (Journal).
    pub title: String,
    /// Journal or named page.
    pub is_journal: bool,
    /// Full markdown text.
    pub text: String,
    /// Outline blocks emitted (depth-first preorder count) — must
    /// match the sidecar's block list length after reconcile.
    pub block_count: usize,
    /// Blocks whose text contains at least one placeholder.
    pub placeholder_blocks: Vec<PlaceholderBlock>,
    /// Whether the raw page text carries any placeholder marker at
    /// all — including ones the parser lifts into *property values*,
    /// which never appear in a block's text and therefore not in
    /// [`Self::placeholder_blocks`]. The resolve pass's file-fallback
    /// sweep triggers on this, so no marker can hide inside a prop.
    pub has_markers: bool,
    /// Depth-first indices of blocks folded in the source.
    pub collapsed: Vec<usize>,
}

/// A block the resolve pass must rewrite.
pub struct PlaceholderBlock {
    /// Depth-first preorder index into the page's block list.
    pub dfs_index: usize,
    /// The block's logical text exactly as the parser will store it
    /// (multi-line joined with `\n`, property lines excluded).
    pub text: String,
}

/// Where a source UID landed.
#[derive(Clone)]
pub struct UidLoc {
    /// Index into the rendered-pages vec.
    pub page_idx: usize,
    /// Depth-first preorder index inside that page.
    pub dfs_index: usize,
    /// Human page title, for the `[[Title]]` fallback path.
    pub page_title: String,
}

/// Render the whole graph. Returns the pages plus the UID → location
/// map the resolve pass consumes.
pub fn render_graph(
    graph: &ImportGraph,
    pages_dir: &Path,
    journals_dir: &Path,
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> (Vec<RenderedPage>, HashMap<String, UidLoc>) {
    // Group by destination file: journals merge on the date (Roam can
    // carry both `May 25th, 2026` and `2026-05-25`); named pages
    // disambiguate slug collisions with a numeric suffix.
    let mut journal_groups: Vec<(NaiveDate, Vec<&ImportPage>)> = Vec::new();
    // Named pages grouped by their outl identity. outl derives a page's
    // id from its slug and resolves `[[Name]]` by slug before anything
    // else, so two source pages on one slug (`Cloudflare` and
    // `cloudflare`) are one page in outl whatever the files say: with a
    // `-2` suffix the second one was unreachable, every link to it landed
    // on the first. They merge like same-date journals do, and the other
    // spellings become `alias::` names. Adapters with their own stem
    // policy (Obsidian paths) pin the stem and are left alone.
    let mut named: Vec<(&str, Vec<&ImportPage>)> = Vec::new();
    let mut named_by_slug: HashMap<String, usize> = HashMap::new();
    for page in &graph.pages {
        match &page.name {
            PageName::Journal(d) => match journal_groups.iter_mut().find(|(k, _)| k == d) {
                Some((_, v)) => {
                    // A legitimate reducer of the emitted page count —
                    // counted so the reconciliation doesn't read the
                    // merge as a lost page.
                    report.pages_merged += 1;
                    report.warn(
                        &d.to_string(),
                        "two source pages map to the same journal date — merged",
                    );
                    v.push(page);
                }
                None => journal_groups.push((*d, vec![page])),
            },
            PageName::Named(t) if page.stem_override.is_none() => {
                let slug = slugify(t);
                match named_by_slug.get(&slug) {
                    Some(&i) => {
                        report.pages_merged += 1;
                        report.slug_collisions += 1;
                        report.warn(
                            t,
                            format!(
                                "shares the outl page `{slug}` with [[{}]], merged and kept as an alias",
                                named[i].0
                            ),
                        );
                        named[i].1.push(page);
                    }
                    None => {
                        named_by_slug.insert(slug, named.len());
                        named.push((t.as_str(), vec![page]));
                    }
                }
            }
            PageName::Named(t) => named.push((t.as_str(), vec![page])),
        }
    }

    let mut used_stems: HashMap<String, usize> = HashMap::new();
    let mut pages_out: Vec<RenderedPage> = Vec::new();
    let mut uid_map: HashMap<String, UidLoc> = HashMap::new();

    for (date, sources) in &journal_groups {
        let title = date.to_string();
        let stem = title.clone();
        let path = journals_dir.join(format!("{stem}.md"));
        render_page(
            sources,
            &title,
            &[],
            stem,
            path,
            true,
            opts,
            report,
            &mut pages_out,
            &mut uid_map,
        );
        report.journals += 1;
    }
    for (title, sources) in &mut named {
        // The page with the most content keeps the title; the other
        // spellings of a merged group become aliases.
        if sources.len() > 1 {
            let primary = (0..sources.len())
                .max_by_key(|&i| (outline_len(sources[i]), std::cmp::Reverse(i)))
                .unwrap_or(0);
            sources.swap(0, primary);
            sources[1..].sort_by_key(|p| graph_position(graph, p));
            *title = page_title(sources[0]);
        }
        let title: &str = title;
        let sources: &[&ImportPage] = sources;
        // Adapters with their own collision policy (Obsidian) pin the
        // stem; otherwise the title slugifies.
        let base = sources[0]
            .stem_override
            .clone()
            .unwrap_or_else(|| slugify(title));
        let n = used_stems.entry(base.clone()).or_insert(0);
        *n += 1;
        let stem = if *n == 1 {
            base
        } else {
            report.slug_collisions += 1;
            report.warn(title, format!("slug collision on `{base}` — suffixed"));
            format!("{base}-{n}")
        };
        let path = pages_dir.join(format!("{stem}.md"));
        let aliases: Vec<&str> = sources[1..]
            .iter()
            .map(|p| page_title(p))
            .filter(|t| *t != title && !t.contains(','))
            .collect();
        render_page(
            sources,
            title,
            &aliases,
            stem,
            path,
            false,
            opts,
            report,
            &mut pages_out,
            &mut uid_map,
        );
        report.pages += 1;
    }

    (pages_out, uid_map)
}

/// A named page's title (journals never reach the callers).
fn page_title(page: &ImportPage) -> &str {
    match &page.name {
        PageName::Named(t) => t,
        PageName::Journal(_) => "",
    }
}

/// Blocks in a page's outline, for picking a merged group's title.
fn outline_len(page: &ImportPage) -> usize {
    fn count(blocks: &[ImportBlock]) -> usize {
        blocks.iter().map(|b| 1 + count(&b.children)).sum()
    }
    match &page.body {
        PageBody::Outline(blocks) => count(blocks),
        PageBody::Raw(raw) => raw.len(),
    }
}

/// Source order of a page, so a merged group keeps it after its primary.
fn graph_position(graph: &ImportGraph, page: &ImportPage) -> usize {
    graph
        .pages
        .iter()
        .position(|p| std::ptr::eq(p, page))
        .unwrap_or(usize::MAX)
}

/// Per-page render accumulator.
struct Ctx<'a> {
    report: &'a mut ImportReport,
    opts: &'a ImportOptions,
    page_title: String,
    out: String,
    dfs: usize,
    collapsed: Vec<usize>,
    uids: Vec<(String, usize)>,
}

#[allow(clippy::too_many_arguments)]
fn render_page(
    sources: &[&ImportPage],
    title: &str,
    aliases: &[&str],
    stem: String,
    path: PathBuf,
    is_journal: bool,
    opts: &ImportOptions,
    report: &mut ImportReport,
    pages_out: &mut Vec<RenderedPage>,
    uid_map: &mut HashMap<String, UidLoc>,
) {
    let mut ctx = Ctx {
        report,
        opts,
        page_title: title.to_string(),
        out: String::new(),
        dfs: 0,
        collapsed: Vec::new(),
        uids: Vec::new(),
    };

    // Page properties. `title::` is the emitter's own on named pages;
    // journals carry no title (the filename is the date).
    let mut header = String::new();
    if !is_journal {
        header.push_str(&format!("title:: {title}\n"));
    }
    let mut aliases_written = aliases.is_empty();
    for page in sources {
        for (k, v) in &page.props {
            if k == "alias" && !aliases_written {
                // One `alias::` line: the index reads only the first.
                header.push_str(&format!("{k}:: {v}, {}\n", aliases.join(", ")));
                aliases_written = true;
            } else {
                header.push_str(&format!("{k}:: {v}\n"));
            }
            ctx.report.props_pages += 1;
        }
    }
    if !aliases_written {
        header.push_str(&format!("alias:: {}\n", aliases.join(", ")));
    }
    // Page timestamps, same policy as block ones. A merged journal
    // (two source pages, one date) keeps the earliest creation and the
    // latest edit. A source attribute of the same name wins: it is what
    // the user wrote.
    let created = sources.iter().filter_map(|p| p.created).min();
    let edited = sources.iter().filter_map(|p| p.edited).max();
    if created.is_some() || edited.is_some() {
        if ctx.opts.preserve_timestamps {
            let has_key = |key: &str| {
                sources
                    .iter()
                    .any(|p| p.props.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)))
            };
            for (key, ts) in [("created", created), ("edited", edited)] {
                if let Some(ts) = ts.filter(|_| !has_key(key)) {
                    header.push_str(&format!("{key}:: {}\n", ts.to_rfc3339()));
                    ctx.report.props_pages += 1;
                }
            }
        } else {
            ctx.report.timestamps_dropped += 1;
        }
    }
    if !header.is_empty() {
        header.push('\n');
    }
    ctx.out.push_str(&header);

    let mut any_body = false;
    for page in sources {
        match &page.body {
            PageBody::Outline(blocks) => {
                for block in blocks {
                    render_block(block, 0, &mut ctx);
                    any_body = true;
                }
            }
            PageBody::Raw(raw) => {
                // Free markdown — emitted verbatim; outl's permissive
                // parser owns the structure downstream. No DFS
                // bookkeeping: raw pages carry no UIDs, so the
                // resolve pass never touches them.
                let t = raw.trim();
                if !t.is_empty() {
                    ctx.out.push_str(t);
                    ctx.out.push('\n');
                    any_body = true;
                }
            }
        }
    }
    if !any_body {
        ctx.out.push_str("- \n");
        ctx.dfs = 1;
    }

    // The resolve pass hash-checks each placeholder block against the
    // sidecar, whose hashes come from outl's parser — so the texts we
    // hand it must be what THAT parser stores, not what we assembled.
    // The two can diverge: a source block whose text carries an
    // embedded `key:: value` line (Logseq residue pasted into Roam is
    // the real-world case) renders as a continuation line that the
    // parser lifts into a block *property*, removing it from the text.
    // So placeholder blocks come from re-parsing the emitted page —
    // and only when the parser agrees with our block count (the 1:1
    // DFS mapping the whole resolve pass rests on). On a disagreement
    // the list stays empty: the count guard already sidelines the page
    // from handle wiring, and the file-fallback sweep (`has_markers`)
    // degrades whatever markers it carries.
    let parsed_texts = preorder_texts(&outl_md::parse(&ctx.out).blocks);
    let placeholder_blocks = if parsed_texts.len() == ctx.dfs {
        parsed_texts
            .into_iter()
            .enumerate()
            .filter(|(_, t)| t.contains(PLACEHOLDER_PREFIX) || t.contains(PLACEHOLDER_EMBED_PREFIX))
            .map(|(dfs_index, text)| PlaceholderBlock { dfs_index, text })
            .collect()
    } else {
        Vec::new()
    };

    let page_idx = pages_out.len();
    for (uid, dfs_index) in ctx.uids {
        uid_map.insert(
            uid,
            UidLoc {
                page_idx,
                dfs_index,
                page_title: title.to_string(),
            },
        );
    }
    let has_markers =
        ctx.out.contains(PLACEHOLDER_PREFIX) || ctx.out.contains(PLACEHOLDER_EMBED_PREFIX);
    pages_out.push(RenderedPage {
        path,
        stem,
        title: title.to_string(),
        is_journal,
        text: ctx.out,
        block_count: ctx.dfs,
        placeholder_blocks,
        has_markers,
        collapsed: ctx.collapsed,
    });
}

/// Depth-first preorder block texts of a parsed page.
fn preorder_texts(blocks: &[outl_md::OutlineNode]) -> Vec<String> {
    fn walk(blocks: &[outl_md::OutlineNode], out: &mut Vec<String>) {
        for b in blocks {
            out.push(b.text.clone());
            walk(&b.children, out);
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

fn render_block(b: &ImportBlock, depth: usize, ctx: &mut Ctx<'_>) {
    let dfs_index = ctx.dfs;
    ctx.dfs += 1;
    ctx.report.blocks += 1;

    if let Some(uid) = &b.uid {
        ctx.uids.push((uid.clone(), dfs_index));
    }
    if b.collapsed {
        ctx.collapsed.push(dfs_index);
    }

    // Assemble the block's logical text — exactly what the parser
    // will store as the node's text (prefixes included, property
    // lines excluded).
    let mut logical = String::new();
    if let Some(task) = b.task {
        logical.push_str(task.outl_prefix());
        logical.push(' ');
    }
    if let Some(h) = b.heading {
        logical.push_str(&"#".repeat(h as usize));
        logical.push(' ');
    }
    logical.push_str(&content_to_string(&b.content, ctx));

    // Emit: first line as the bullet, the rest as continuation lines.
    let pad = "  ".repeat(depth);
    let cont_pad = "  ".repeat(depth + 1);
    // Same line split + escaping as outl-md's own renderer, so a
    // continuation line shaped like `- item` stays text instead of
    // parsing back as a child bullet (which broke the 1:1 DFS mapping).
    let mut lines = outl_md::render::block_text_lines(&logical).into_iter();
    let first = lines.next().unwrap_or_default();
    ctx.out.push_str(&pad);
    ctx.out.push_str("- ");
    ctx.out.push_str(&first);
    ctx.out.push('\n');
    for line in lines {
        ctx.out.push_str(&cont_pad);
        ctx.out.push_str(&line);
        ctx.out.push('\n');
    }

    // Block properties as continuation property lines.
    let mut props: Vec<(String, String)> = Vec::new();
    if let Some(state) = b.task.and_then(|t| t.state_prop()) {
        props.push(("state".to_string(), state.to_string()));
    }
    props.extend(b.props.iter().cloned());
    if b.created.is_some() || b.edited.is_some() {
        if ctx.opts.preserve_timestamps {
            if let Some(c) = b.created {
                props.push(("created".to_string(), c.to_rfc3339()));
            }
            if let Some(e) = b.edited {
                props.push(("edited".to_string(), e.to_rfc3339()));
            }
        } else {
            ctx.report.timestamps_dropped += 1;
        }
    }
    for (k, v) in &props {
        ctx.out.push_str(&cont_pad);
        ctx.out.push_str(&format!("{k}:: {v}\n"));
        ctx.report.props_blocks += 1;
    }

    for child in &b.children {
        render_block(child, depth + 1, ctx);
    }
}

fn content_to_string(content: &BlockContent, ctx: &mut Ctx<'_>) -> String {
    match content {
        BlockContent::Inline(toks) => {
            let mut s = String::new();
            for tok in toks {
                match tok {
                    Inline::Text(t) => s.push_str(t),
                    Inline::CodeSpan(c) => {
                        s.push('`');
                        s.push_str(c);
                        s.push('`');
                    }
                    Inline::BlockRef { uid, alias: None } => s.push_str(&placeholder_ref(uid)),
                    Inline::BlockRef {
                        uid,
                        alias: Some(a),
                    } => s.push_str(&format!("[{a}]({})", placeholder_ref(uid))),
                    Inline::Embed(EmbedTarget::Block(uid)) => {
                        s.push_str(&placeholder_embed(uid));
                    }
                    Inline::Embed(EmbedTarget::Page(p)) => {
                        ctx.report.embeds_page_fallback += 1;
                        ctx.report.warn(
                            &ctx.page_title.clone(),
                            format!("page embed of [[{p}]] degraded to a link (issue #190)"),
                        );
                        s.push_str(&format!("[[{p}]]"));
                    }
                    Inline::Component { raw, .. } => s.push_str(raw),
                }
            }
            s
        }
        BlockContent::Code { lang, body } => {
            let l = lang.as_deref().unwrap_or("");
            if body.is_empty() {
                format!("```{l}\n```")
            } else {
                format!("```{l}\n{body}\n```")
            }
        }
        BlockContent::Verbatim(s) => s.clone(),
    }
}
