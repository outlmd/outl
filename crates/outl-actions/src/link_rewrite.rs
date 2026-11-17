//! Rewriting every mention of one page so it names another: the single
//! owner of "what is a mention of page X, and how is it respelled".
//!
//! Page rename and page merge both end with the same sentence, "and every
//! link that pointed at the old page now points at the new one". Written
//! twice, that sentence drifts; written here, both ask one function.
//!
//! The rewrite is an ordinary action: one `Op::Edit` per changed block,
//! through [`crate::block::edit_text`], inside one batch. The op log stays
//! the source of truth (root invariant 1) and nothing here touches a `.md`.
//! The caller re-projects the pages named in
//! [`LinkRewriteReport::touched_pages`], ideally through
//! [`crate::commit::commit_page`].
//!
//! # What counts as a mention
//!
//! Everything is read off [`outl_md::inline::tokenize`], the tokenizer that
//! renders the block and decides backlinks ([`crate::mentions`]), so a form
//! the renderer does not draw as a link is never rewritten.
//!
//! - **`[[Name]]`** mentions `from` when it resolves to it by the ladder
//!   navigation uses (`resolve::resolve_or_create_by_name`): the literal
//!   slug equals `from.slug`, or `slugify(Name)` does, or `Name` equals
//!   `from.title` ignoring case. Navigation checks the slug rungs before
//!   the title, so a name that slug-resolves to a **different** existing
//!   page is left alone even when it also equals `from`'s title
//!   ([`rewrite_text_with`]'s `resolves_to_other`, answered by
//!   [`rewrite_mentions`] from the workspace). Respelled as
//!   `[[<to.title>]]`.
//! - **`![[Name]]`**, the page embed (issue #190), mentions `from` by the
//!   same ladder as `[[Name]]` and is respelled `![[<to.title>]]`. It is a
//!   page reference that happens to render expanded, so a rename that left
//!   it behind would point every transclusion at a page that no longer
//!   answers to that name. The only differences from the plain ref are
//!   mechanical and both live in `RefForm`: the opener is three bytes,
//!   and the replacement carries the `!` back.
//! - **`[[@Name]]`**, the person mention, is a `PageRef` whose name
//!   starts with `@` (there is no separate token). It mentions `from` only
//!   when `from.person`, which is the rule the backlinks side applies
//!   (`keys_for_page` indexes the `@` spellings of a person page only), and
//!   the bare name after the `@` goes through the same ladder. Respelled
//!   `[[@<to.title>]]` when `to.person`, `[[<to.title>]]` otherwise (the
//!   `@` is a person affordance, not part of the name).
//! - **`#tag`** mentions `from` when `slugify(tag) == from.slug`, which is
//!   how backlinks reduce tags. Respelled `#<to.slug>` when that
//!   tokenizes back as one tag named exactly the slug, otherwise as
//!   `[[<to.title>]]` (a tag cannot spell that name, a ref is the same
//!   mention). Punctuation right after a tag is outside the tag token and
//!   stays. A tag is not rewritten when the slug does not change, so a
//!   title-only rename leaves `#Alpha` alone.
//! - If neither title nor slug of `to` survives a `[[ ]]` round trip
//!   through the tokenizer (a title holding `]]`), the ref is spelled with
//!   the slug; if even that fails the mention is left as it was.
//!
//! # What is never touched
//!
//! - Inline `` `code` `` spans, and **fenced code** (`` ``` `` or `~~~`,
//!   opener to closer, or to the end of the text when unclosed). The
//!   tokenizer has no notion of a fence (it only knows single-backtick
//!   spans on one line), so a `[[x]]` in a fence is a `PageRef` to it and
//!   to [`crate::mentions`]. This module skips the fenced lines before
//!   tokenizing. That is the deliberate asymmetry: code is never edited,
//!   even where backlinks still count it.
//! - A `[[X]]` inside a link's text or url (`[label]([[X]])` is a `Link`
//!   token, opaque by construction). There is no `#[[Name]]` form and no
//!   aliased ref in this dialect, so nothing else to handle.
//! - Block refs `((blk-...))` and block embeds `!((blk-...))`: they name
//!   blocks, not pages. (A *page* embed, `![[Name]]`, is a page mention and
//!   is rewritten — see above.)
//! - Names that merely contain the slug (`[[alphabet]]`, `#alpha-two`
//!   against `alpha`) and names that only match a page **alias**
//!   (an alias is another page's property, not `from`'s names; `PageNames`
//!   carries none).
//! - **Namespace children**: `[[from/child]]` and `#from/child` (any
//!   mention whose [`crate::namespace::ancestors`], slugified, include
//!   `from.slug`) belong to a sub-page that is not being renamed. They are
//!   listed in [`LinkRewriteReport::skipped_namespace_refs`] (one entry per
//!   block and name) when the slug changes, and never listed otherwise,
//!   since nothing about them changes.
//! - Page root nodes (nodes carrying `page-slug`) and everything under
//!   `TRASH_ROOT`: the walk starts at `NodeId::root()`, whose subtree is
//!   the live tree, and deleting is a move out of it.
//!
//! # Known limit: properties
//!
//! `key:: value` properties, on a block or on a page, are `Op::SetProp`
//! values on the node, not block text. A `[[from]]` in a property value is
//! therefore not rewritten here, the same way backlinks never read it
//! (the one exception, `from-template::`, is a slug and is not a mention).
//!
//! # Guarantees
//!
//! - **Idempotent.** A second run over the result edits nothing: an edit
//!   is generated only when the new text differs from the old.
//! - **No-op** when `from` and `to` have the same slug and title.
//! - **Deterministic.** Blocks are visited in [`crate::tree::walk_subtree`]
//!   order (canonical sibling order), so the same workspace always gets
//!   the same ops in the same order.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::inline::{tokenize, InlineTok};
use outl_md::slug::slugify;
use serde::Serialize;

use crate::block::edit_text;
use crate::error::ActionError;
use crate::page::{find_by_slug, PageMeta, SLUG_KEY};
use crate::person::PERSON_TYPE;
use crate::tree::{page_slug_of, walk_subtree};

mod fence;

#[cfg(test)]
mod tests;

/// The names a page answers to in mentions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageNames {
    /// The page's slug (`page-slug`).
    pub slug: String,
    /// The page's title, as [`crate::page::page_meta`] resolves it.
    pub title: String,
    /// Whether the page is a person (`type:: person`), which decides how
    /// `[[@Name]]` mentions are read and respelled.
    pub person: bool,
}

impl PageNames {
    /// The names of the page `meta` describes.
    pub fn of(meta: &PageMeta) -> PageNames {
        PageNames {
            slug: meta.slug.clone(),
            title: meta.title.clone(),
            person: meta.page_type.as_deref() == Some(PERSON_TYPE),
        }
    }
}

/// A namespace-child mention that was left untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedRef {
    /// The block holding the mention (stringified `NodeId`).
    pub block_id: String,
    /// The mention's name as written (`from/child`).
    pub name: String,
}

/// What [`rewrite_mentions`] did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LinkRewriteReport {
    /// Blocks whose text was edited (stringified `NodeId`).
    pub rewritten_blocks: Vec<String>,
    /// Slugs of the pages that own those blocks (deduplicated, sorted) so
    /// callers re-project each.
    pub touched_pages: Vec<String>,
    /// Namespace-child mentions (`[[from/child]]`, `#from/child`) left
    /// untouched.
    pub skipped_namespace_refs: Vec<SkippedRef>,
}

/// Pure text rewrite. `None` when nothing in `text` mentions `from`.
///
/// Assumes no other page competes for the names `from` answers to; use
/// [`rewrite_text_with`] when the workspace can say otherwise.
pub fn rewrite_text(text: &str, from: &PageNames, to: &PageNames) -> Option<String> {
    rewrite_text_with(text, from, to, &|_| false)
}

/// [`rewrite_text`] with the workspace's say on ambiguous names.
///
/// `resolves_to_other(name)` answers whether navigation, given `name`,
/// would land on a page other than `from`. A mention for which it says yes
/// is not a mention of `from` and is left alone.
pub fn rewrite_text_with(
    text: &str,
    from: &PageNames,
    to: &PageNames,
    resolves_to_other: &dyn Fn(&str) -> bool,
) -> Option<String> {
    scan(text, from, to, resolves_to_other).rewritten
}

/// Rewrite every block in the workspace so its mentions of `from` name
/// `to`, one `Op::Edit` per changed block, in one batch.
///
/// See the module docs for what is and is not a mention. The caller owns
/// re-projecting [`LinkRewriteReport::touched_pages`].
pub fn rewrite_mentions(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    from: &PageNames,
    to: &PageNames,
) -> Result<LinkRewriteReport, ActionError> {
    let mut report = LinkRewriteReport::default();
    if from.slug == to.slug && from.title == to.title {
        return Ok(report);
    }

    let edits = plan_edits(workspace, from, to, &mut report);

    let mut batch = workspace.begin_batch();
    let mut pages = BTreeSet::new();
    for (node, new_text) in &edits {
        edit_text(&mut batch, hlc, *node, new_text)?;
        report.rewritten_blocks.push(node.to_string());
        if let Some(slug) = page_slug_of(&batch, *node) {
            pages.insert(slug);
        }
    }
    batch.commit()?;
    report.touched_pages = pages.into_iter().collect();
    Ok(report)
}

/// Every block that needs an edit, in walk order, with its new text.
/// Namespace skips go straight into `report`.
fn plan_edits(
    workspace: &Workspace,
    from: &PageNames,
    to: &PageNames,
    report: &mut LinkRewriteReport,
) -> Vec<(NodeId, String)> {
    // The page `from` names, when it still exists: a name that slug-resolves
    // to any other page is not a mention of it.
    let from_page = find_by_slug(workspace, &from.slug);
    let memo: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
    let resolves_to_other = |name: &str| -> bool {
        let Some(from_page) = from_page else {
            return false;
        };
        if let Some(known) = memo.borrow().get(name) {
            return *known;
        }
        let hit = slug_ladder_hit(workspace, name);
        let answer = hit.is_some_and(|id| id != from_page);
        memo.borrow_mut().insert(name.to_string(), answer);
        answer
    };

    let mut edits = Vec::new();
    walk_subtree(workspace, NodeId::root(), |node| {
        // A page root's own text is its title, not prose.
        if workspace.tree().property(node, SLUG_KEY).is_some() {
            return true;
        }
        let Some(text) = workspace.block_text(node) else {
            return true;
        };
        // Cheap exit for the overwhelming majority of blocks: no `[[`
        // and no `#` means no ref and no tag.
        if !text.contains("[[") && !text.contains('#') {
            return true;
        }
        let found = scan(&text, from, to, &resolves_to_other);
        for name in found.skipped {
            report.skipped_namespace_refs.push(SkippedRef {
                block_id: node.to_string(),
                name,
            });
        }
        if let Some(new_text) = found.rewritten {
            edits.push((node, new_text));
        }
        true
    });
    edits
}

/// The page the navigation ladder's slug rungs land on for `name`:
/// literal slug first, then the slugified form.
fn slug_ladder_hit(workspace: &Workspace, name: &str) -> Option<NodeId> {
    find_by_slug(workspace, name).or_else(|| {
        let normalised = slugify(name);
        (normalised != name)
            .then(|| find_by_slug(workspace, &normalised))
            .flatten()
    })
}

// --- the scan -------------------------------------------------------------

/// One text's verdict: the new text (when something changed) and the
/// namespace-child names it left alone.
struct Scan {
    rewritten: Option<String>,
    skipped: Vec<String>,
}

/// A byte range of the source and what replaces it.
struct Splice {
    start: usize,
    end: usize,
    replacement: String,
}

struct Ctx<'a> {
    text: &'a str,
    from: &'a PageNames,
    to: &'a PageNames,
    resolves_to_other: &'a dyn Fn(&str) -> bool,
    splices: Vec<Splice>,
    skipped: Vec<String>,
}

fn scan(
    text: &str,
    from: &PageNames,
    to: &PageNames,
    resolves_to_other: &dyn Fn(&str) -> bool,
) -> Scan {
    let mut ctx = Ctx {
        text,
        from,
        to,
        resolves_to_other,
        splices: Vec::new(),
        skipped: Vec::new(),
    };
    for segment in fence::prose_segments(text) {
        collect(&tokenize(segment), &mut ctx);
    }
    Scan {
        rewritten: splice(text, &ctx.splices),
        skipped: ctx.skipped,
    }
}

/// Apply `splices` (ascending, disjoint) from the end so earlier offsets
/// stay valid. `None` when no splice changes anything.
fn splice(text: &str, splices: &[Splice]) -> Option<String> {
    let mut out = text.to_string();
    let mut changed = false;
    for s in splices.iter().rev() {
        if out[s.start..s.end] != s.replacement {
            out.replace_range(s.start..s.end, &s.replacement);
            changed = true;
        }
    }
    changed.then_some(out)
}

/// DFS the token tree through the wrappers a client renders through. The
/// `match` is exhaustive on purpose, like `mentions::collect`: a new
/// `InlineTok` variant must not compile until someone says whether it can
/// hold a mention.
fn collect(toks: &[InlineTok<'_>], ctx: &mut Ctx<'_>) {
    for tok in toks {
        match tok {
            InlineTok::PageRef { name } => visit_ref(name, RefForm::Link, ctx),
            InlineTok::PageEmbed { name } => visit_ref(name, RefForm::Embed, ctx),
            InlineTok::Tag { name } => visit_tag(name, ctx),
            InlineTok::Bold { inner }
            | InlineTok::Italic { inner, .. }
            | InlineTok::Strike { inner }
            | InlineTok::Highlight { inner } => collect(inner, ctx),
            InlineTok::Plain(_)
            | InlineTok::Code { .. }
            | InlineTok::Link { .. }
            | InlineTok::Image { .. }
            | InlineTok::BlockRef { .. }
            | InlineTok::Embed { .. }
            | InlineTok::Emoji { .. } => {}
        }
    }
}

/// Byte offset of `slice` inside `ctx.text`. Every token borrows from the
/// text it was cut out of, so the pointers are comparable.
fn offset_in(text: &str, slice: &str) -> usize {
    slice.as_ptr() as usize - text.as_ptr() as usize
}

/// How a page mention is spelled on disk: `[[Name]]` or `![[Name]]`.
///
/// Both are mentions of the same page and both get respelled; the two
/// things that differ are mechanical, and keeping them here is what stops
/// a splice from being computed with the wrong opener width. A token's
/// `name` borrows from the middle of the source, so the splice's start is
/// derived by *subtracting* the opener — three bytes for an embed, not two
/// — and an off-by-one there eats the character in front of the mention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefForm {
    /// `[[Name]]` — a page reference.
    Link,
    /// `![[Name]]` — a page embed (issue #190). Still a page reference, so
    /// a rename respells it; the `!` is the only extra byte.
    Embed,
}

impl RefForm {
    /// Bytes between the start of the token and the start of `name`.
    fn opener_len(self) -> usize {
        match self {
            RefForm::Link => "[[".len(),
            RefForm::Embed => "![[".len(),
        }
    }

    /// What the replacement has to put back in front of the `[[`.
    fn prefix(self) -> &'static str {
        match self {
            RefForm::Link => "",
            RefForm::Embed => "!",
        }
    }
}

fn visit_ref(name: &str, form: RefForm, ctx: &mut Ctx<'_>) {
    let (bare, at) = match name.strip_prefix('@') {
        Some(rest) if !rest.is_empty() => (rest, true),
        _ => (name, false),
    };
    // `[[@x]]` is a mention of a person page, never of an ordinary one.
    if at && !ctx.from.person {
        return;
    }
    if names_page(bare, ctx.from) {
        if (ctx.resolves_to_other)(bare) {
            return;
        }
        let Some(replacement) = spell_ref(ctx.to, at, form) else {
            return;
        };
        let start = offset_in(ctx.text, name) - form.opener_len();
        ctx.splices.push(Splice {
            start,
            end: start + form.opener_len() + name.len() + "]]".len(),
            replacement,
        });
    } else {
        note_namespace_child(bare, name, ctx);
    }
}

fn visit_tag(name: &str, ctx: &mut Ctx<'_>) {
    if slugify(name) == ctx.from.slug {
        if ctx.from.slug == ctx.to.slug || (ctx.resolves_to_other)(name) {
            return;
        }
        let Some(replacement) = spell_tag(ctx.to) else {
            return;
        };
        let start = offset_in(ctx.text, name) - 1;
        ctx.splices.push(Splice {
            start,
            end: start + 1 + name.len(),
            replacement,
        });
    } else {
        note_namespace_child(name, name, ctx);
    }
}

/// Whether `name` is a spelling of `from` by the navigation ladder's own
/// rungs (the workspace-dependent part is `resolves_to_other`).
fn names_page(name: &str, from: &PageNames) -> bool {
    name == from.slug
        || slugify(name) == from.slug
        || name.to_lowercase() == from.title.to_lowercase()
}

/// Record `written` when `bare` sits under `from`'s namespace. Only when
/// the slug changes: otherwise nothing about the child changes either.
fn note_namespace_child(bare: &str, written: &str, ctx: &mut Ctx<'_>) {
    if ctx.from.slug == ctx.to.slug || !bare.contains('/') {
        return;
    }
    let under = crate::namespace::ancestors(bare)
        .iter()
        .any(|ancestor| slugify(ancestor) == ctx.from.slug);
    if under && !ctx.skipped.iter().any(|seen| seen == written) {
        ctx.skipped.push(written.to_string());
    }
}

/// `[[<to.title>]]` (or `[[@<to.title>]]`, or `![[<to.title>]]` for an
/// embed), falling back to the slug when the title cannot be written
/// inside brackets. `None` when neither can.
///
/// The round trip is checked against the spelling this will actually
/// write, `form` included, so a replacement the tokenizer would read back
/// as something else is never emitted. The forms are not interchangeable
/// here: `![[photo.png]]` is refused by `try_page_embed` (the image-embed
/// spelling has another owner), so a title the plain ref could carry
/// verbatim falls through to the slug — where `.` folds to `-` and
/// `![[photo-png]]` reads back as the embed it is.
fn spell_ref(to: &PageNames, at: bool, form: RefForm) -> Option<String> {
    let person = at && to.person;
    [&to.title, &to.slug].into_iter().find_map(|name| {
        let inner = if person {
            format!("@{name}")
        } else {
            name.clone()
        };
        let spelled = format!("{}[[{inner}]]", form.prefix());
        let round_trips = match form {
            RefForm::Link => matches!(
                tokenize(&spelled).as_slice(),
                [InlineTok::PageRef { name: back }] if *back == inner
            ),
            RefForm::Embed => matches!(
                tokenize(&spelled).as_slice(),
                [InlineTok::PageEmbed { name: back }] if *back == inner
            ),
        };
        round_trips.then_some(spelled)
    })
}

/// `#<to.slug>` when the tokenizer reads it back as exactly that tag,
/// otherwise the ref spelling of the same mention.
fn spell_tag(to: &PageNames) -> Option<String> {
    let spelled = format!("#{}", to.slug);
    let round_trips = matches!(
        tokenize(&spelled).as_slice(),
        [InlineTok::Tag { name }] if *name == to.slug
    );
    if round_trips {
        Some(spelled)
    } else {
        spell_ref(to, false, RefForm::Link)
    }
}
