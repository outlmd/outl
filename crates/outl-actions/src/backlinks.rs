//! Backlinks: which blocks reference which pages.
//!
//! A reference is either a `[[target]]` **token** in a block's text or
//! a `#tag` token whose slug form resolves to the target page — the
//! same `slugify` rule a tag click goes through
//! (`open_or_create_by_name`), so "what opens the page" and "what
//! shows up in the page's backlinks" can't drift. `target` matches
//! either a page's slug or its title (the page root's text). Block
//! refs (`((blk-X))`) are handled by `outl-md::inline` — this module
//! is the workspace-level "which page mentions me" view.
//!
//! **Token, not substring.** Both channels go through the inline
//! tokenizer; [`crate::mentions`] owns that rule and explains what it
//! replaced.
//!
//! **This is the single source of truth for backlinks.** Both the
//! mobile client and the TUI consume [`backlinks_for_page`]; the
//! `outl-md::index` crate intentionally does NOT carry a parallel
//! backlinks cache — that earlier duplication was the bug that made
//! self-references invisible on one surface but not the other.

use std::path::{Path, PathBuf};

use outl_core::workspace::Workspace;
use serde::{Serialize, Serializer};

use crate::backlinks_index::build_backlink_index;
use crate::outline::OutlineNode;
use crate::page::PageMeta;
use crate::todo::TodoState;

/// One backlink reference from a source block to a target page.
///
/// Carries everything a UI needs to render the source block inline —
/// the block's own text and TODO state, the page it lives in, plus
/// the source block as an [`OutlineNode`] subtree so the renderer can
/// surface children + properties without a second workspace lookup.
///
/// `block_text` is the body **without** the `TODO `/`DONE ` prefix;
/// the prefix (if any) lives in [`Self::todo`]. Clients must surface
/// the TODO state with their own checkbox widget — there is no
/// marker left in `block_text` to fall back on.
#[derive(Debug, Clone, Serialize)]
pub struct Backlink {
    /// Block that contains the `[[target]]` mention.
    pub block_id: String,
    /// Body of the source block, with the TODO/DONE prefix stripped.
    ///
    /// **Consumed by the CLI/MCP JSON envelope** (`outl page rename`
    /// returns it inside `affected_refs`), not by the mobile renderer.
    /// Mobile reads `source_block.tokens` / `source_block.text` instead;
    /// the TS `Backlink` interface deliberately omits this field.
    /// Removing it from Rust would break the CLI contract.
    pub block_text: String,
    /// `None` for a plain bullet, `Some(Todo)` / `Some(Done)` otherwise.
    /// Serialised as `"TODO"` / `"DONE"` / `null` to match
    /// [`crate::outline::OutlineNode::todo`].
    #[serde(serialize_with = "serialize_todo_state")]
    pub todo: Option<TodoState>,
    /// Page that contains the source block, if any. `None` only when
    /// the block lives outside any page (legacy / migrated workspaces).
    pub source_page: Option<PageMeta>,
    /// Source block as a self-contained outline subtree (children +
    /// properties). Mirrors the shape `read_page_view_with_workspace`
    /// would return for the same block. UI clients that render
    /// backlinks as a mini-outline (TUI today, mobile in the future)
    /// consume this; light clients can ignore it and use only
    /// `block_text` + `todo`.
    pub source_block: OutlineNode,
    /// DFS path of the source block inside its `source_page`. Empty
    /// vector means the block is a direct child of the page root.
    /// Used by the TUI to track which block inside a backlink's
    /// subtree the cursor is on (`Focus::Backlink { sub_path }`).
    pub source_block_path: Vec<usize>,
    /// Ancestor blocks between the page root and the source block,
    /// **root-first**: `ancestors[0]` is the direct child of the page
    /// root, the last entry is the source block's immediate parent.
    /// Empty when the source block sits at page-root level.
    ///
    /// This is the breadcrumb every client renders as dimmed context
    /// above the citing block, so a reference buried inside a nested
    /// outline still reads with the branch it belongs to. The page
    /// root itself is **not** included — clients already show it as the
    /// group header (the page title).
    pub ancestors: Vec<BacklinkCrumb>,
    /// On-disk path of the source page's `.md`. Derived from the
    /// workspace root passed to [`backlinks_for_page`] / [`backlinks_for_target`].
    /// `None` when the block has no enclosing page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
}

impl Backlink {
    /// Drop the source block's subtree (children), keeping the leaf
    /// itself (text, tokens, todo, properties) plus `ancestors`.
    ///
    /// The GUI clients render only `source_block.tokens` for a backlink
    /// row, so shipping the whole descendant subtree over the IPC wire
    /// is pure waste. The TUI, which renders the subtree as a
    /// mini-outline, keeps the full form (it reads the index in-process,
    /// no serialization). Apply this on the GUI `page_backlinks` path
    /// only.
    pub fn into_shallow(mut self) -> Self {
        self.source_block.children.clear();
        self
    }
}

/// One ancestor step in a backlink's breadcrumb.
///
/// Plain text only (no inline tokens): the breadcrumb is dimmed
/// context, not an interactive surface, so clients render it as a
/// muted trail rather than re-rendering links/bold the way they do for
/// the citing block itself. `text` already has the `TODO `/`DONE `
/// prefix stripped, mirroring [`Backlink::block_text`]. `id` lets a
/// client make a crumb tappable (jump to that ancestor) later without
/// a shape change.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinkCrumb {
    /// Node id of the ancestor block.
    pub id: String,
    /// Ancestor block's text, TODO/DONE prefix stripped, single line.
    pub text: String,
}

fn serialize_todo_state<S>(state: &Option<TodoState>, ser: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match state {
        None => ser.serialize_none(),
        Some(s) => ser.serialize_str(s.as_str()),
    }
}

/// Every block in the workspace that mentions `target` — either as a
/// literal `[[target]]` substring or as a `#tag` token whose slug
/// form equals `target`'s slug form.
///
/// `[[target]]` is matched literally — pass the page's slug AND title
/// separately if you want to catch both forms. Tags go through
/// `outl_md::slug::slugify` on both sides, mirroring how a tag click
/// resolves its page (`open_or_create_by_name`), so `#Avelino` counts
/// as a mention of the page whose slug is `avelino`. `root` is the
/// workspace root directory; it's needed so each backlink can carry
/// its `source_path` (the `.md` of the page the source block lives
/// in).
///
/// This builds a one-shot [`BacklinkIndex`][crate::backlinks_index::BacklinkIndex]
/// and looks the target up in it — fine for a one-off caller (CLI,
/// tests). A long-lived client that reads backlinks repeatedly should
/// keep an index around ([`build_backlink_index`]) and call
/// [`BacklinkIndex::for_target`][crate::backlinks_index::BacklinkIndex::for_target]
/// so it pays the `O(blocks)` build once, off the input path, instead
/// of per lookup.
pub fn backlinks_for_target(workspace: &Workspace, root: &Path, target: &str) -> Vec<Backlink> {
    build_backlink_index(workspace, root).for_target(target)
}

/// Convenience: backlinks against either the page's slug or its title
/// (plus the `@`-alias forms for a person page and the template
/// channels for a template page).
///
/// Like [`backlinks_for_target`], this builds a one-shot index and
/// looks the page up. The single source of truth for *what counts as a
/// mention* lives in [`crate::backlinks_index`]; this is just the
/// build-once-then-lookup convenience. A repeated reader should hold a
/// [`BacklinkIndex`][crate::backlinks_index::BacklinkIndex] and call
/// [`for_page`][crate::backlinks_index::BacklinkIndex::for_page].
pub fn backlinks_for_page(workspace: &Workspace, root: &Path, meta: &PageMeta) -> Vec<Backlink> {
    build_backlink_index(workspace, root).for_page(workspace, meta)
}

/// Re-exported so `outl_actions::backlinks::extract_refs` keeps
/// resolving. The rule itself lives in [`crate::mentions`], which owns
/// "what does this block text mention" for the backlink index and the
/// reminder scanner alike.
pub use crate::mentions::extract_refs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{append_block, edit_text};
    use crate::page::page_meta;
    use crate::page::{find_by_slug, open_journal, open_or_create, PageKind};
    use chrono::NaiveDate;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    /// Tests don't need a real filesystem — `source_path` is just
    /// `root + journals/<slug>.md` / `pages/<slug>.md`. Using a
    /// constant root keeps assertions readable.
    fn root() -> &'static Path {
        Path::new("/tmp/outl-test")
    }

    #[test]
    fn extract_refs_finds_multiple_tokens() {
        let refs = extract_refs("see [[avelino]] and [[2026-05-27]] please");
        assert_eq!(refs, vec!["avelino".to_string(), "2026-05-27".to_string()]);
    }

    /// **Changed deliberately**: the byte scan skipped an unterminated
    /// `[[` and recovered the inner `[[ok]]`; the tokenizer is greedy to
    /// the first `]]`, which is what the renderer draws. The property
    /// this test protects is unchanged — an unbalanced opener never
    /// swallows the rest of the text.
    #[test]
    fn extract_refs_ignores_unbalanced() {
        let refs = extract_refs("[[unterminated and [[ok]] mixed");
        assert_eq!(refs, vec!["unterminated and [[ok".to_string()]);
        assert!(!refs.iter().any(|r| r.contains("mixed")));

        let refs = extract_refs("[[no close at all and then [[ok]]");
        assert_eq!(refs, vec!["no close at all and then [[ok".to_string()]);
    }

    /// Build a template page named `name` with a single code block, so
    /// it can be both instantiated (structural) and called (callable).
    fn make_template(workspace: &mut Workspace, hlc: &HlcGenerator, slug: &str, name: &str) {
        use crate::page::set_property;
        use crate::template::TEMPLATE_KEY;
        use outl_core::property::PropValue;

        let id = open_or_create(workspace, hlc, slug, slug, PageKind::Page).unwrap();
        set_property(
            workspace,
            hlc,
            id,
            TEMPLATE_KEY,
            Some(PropValue::Text(name.into())),
        )
        .unwrap();
        append_block(workspace, hlc, Some(id), Some("- **Item:**")).unwrap();
    }

    #[test]
    fn structural_instance_shows_in_template_backlinks() {
        // Instantiate a template into a journal, then the template
        // page's backlinks must list where it was stamped — the
        // `from-template::` property is not a `[[ref]]`, so this only
        // works because the matcher reads the property directly.
        let (mut w, hlc) = ws();
        make_template(&mut w, &hlc, "template-1on1", "1on1");
        let j = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 9).unwrap()).unwrap();
        let target = append_block(&mut w, &hlc, Some(j), Some("host")).unwrap();
        crate::template::instantiate_template(&mut w, &hlc, "1on1", target, "2026-07-09", None)
            .unwrap();

        let meta = page_meta(&w, find_by_slug(&w, "template-1on1").unwrap()).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(
            !bl.is_empty(),
            "structural instance should appear in the template's backlinks"
        );
        assert!(bl.iter().any(|b| b
            .source_page
            .as_ref()
            .is_some_and(|p| p.slug == "2026-07-09")));
    }

    #[test]
    fn callable_site_shows_in_template_backlinks() {
        // A `call:<name>` fence must surface in the template page's
        // backlinks without a hand-written `[[link]]`.
        let (mut w, hlc) = ws();
        make_template(&mut w, &hlc, "template-calc", "calc");
        let j = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 9).unwrap()).unwrap();
        append_block(&mut w, &hlc, Some(j), Some("```call:calc\nx: 1\n```")).unwrap();

        let meta = page_meta(&w, find_by_slug(&w, "template-calc").unwrap()).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(
            bl.iter().any(|b| b
                .source_page
                .as_ref()
                .is_some_and(|p| p.slug == "2026-07-09")),
            "callable site should appear in the template's backlinks"
        );
    }

    #[test]
    fn non_template_page_ignores_call_and_provenance() {
        // A regular page must not accidentally pull in call/provenance
        // matches — the template channel only fires for template pages.
        let (mut w, hlc) = ws();
        let p = open_or_create(&mut w, &hlc, "regular", "regular", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(p), Some("```call:regular\n```")).unwrap();

        let meta = page_meta(&w, p).unwrap();
        let bl = backlinks_for_page(&w, root(), &meta);
        assert!(bl.is_empty(), "regular page has no template channel");
    }

    #[test]
    fn tag_mentions_count_as_backlinks() {
        // `#avelino` must surface in the backlinks of the `avelino`
        // page exactly like `[[avelino]]` would — a tag click and a
        // ref click open the same page, so the "Linked from" panel
        // has to agree.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("pairing with #avelino today")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
    }

    #[test]
    fn tag_mentions_match_through_slugify() {
        // A tag click resolves its page via `slugify` (see
        // `open_or_create_by_name`), so `#Avelino` is a mention of
        // the page whose slug is `avelino`.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention = append_block(&mut w, &hlc, Some(day), Some("ship it #Avelino")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
    }

    #[test]
    fn longer_tag_does_not_false_match_a_prefix_target() {
        // `#avelino-foo` is a different page; it must NOT appear in
        // `avelino`'s backlinks. A substring probe on `#avelino`
        // would get this wrong — the tokenizer-based match doesn't.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("see #avelino-foo instead")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "prefix tag leaked in: {links:#?}");
    }

    #[test]
    fn tag_inside_inline_code_is_not_a_mention() {
        // `` `#avelino` `` is code, not a tag — the inline tokenizer
        // already knows that; the backlink matcher must respect it.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("escape it as `#avelino`")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "code-span tag leaked in: {links:#?}");
    }

    #[test]
    fn block_with_ref_and_tag_emits_one_backlink() {
        // Mentioning the same page via both forms in one block still
        // produces a single backlink (matcher is a yes/no probe and
        // `backlinks_for_page` dedupes by block id).
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] aka #avelino")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
    }

    #[test]
    fn backlinks_find_blocks_pointing_at_slug() {
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let mention =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] shipped it")).unwrap();
        let _ = target;

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, mention.to_string());
        assert_eq!(
            links[0].source_page.as_ref().map(|p| p.slug.clone()),
            Some("2026-05-27".to_string())
        );
        assert_eq!(
            links[0].source_path.as_deref(),
            Some(root().join("journals/2026-05-27.md").as_path()),
            "journal source page resolves to journals/<slug>.md"
        );
    }

    #[test]
    fn backlinks_for_a_future_journal_finds_blocks_in_past_journals() {
        // Scenario: I open today's journal and write a task that
        // tags tomorrow (`[[2026-05-30]]`). Tomorrow, when I open
        // that journal, the section "Linked from" should list my
        // task. This is the workflow the user described as their
        // primary use of journals.
        let (mut w, hlc) = ws();

        let today = NaiveDate::from_ymd_opt(2026, 5, 29).unwrap();
        let tomorrow = NaiveDate::from_ymd_opt(2026, 5, 30).unwrap();

        // Today's journal carries a block tagging tomorrow.
        let today_id = open_journal(&mut w, &hlc, today).unwrap();
        let task = append_block(
            &mut w,
            &hlc,
            Some(today_id),
            Some("call avelino back [[2026-05-30]]"),
        )
        .unwrap();

        // Open tomorrow's journal and pull its backlinks.
        let tomorrow_id = open_journal(&mut w, &hlc, tomorrow).unwrap();
        let meta = page_meta(&w, tomorrow_id).unwrap();
        let links = backlinks_for_page(&w, root(), &meta);

        assert_eq!(
            links.len(),
            1,
            "tomorrow's journal should see today's mention"
        );
        assert_eq!(links[0].block_id, task.to_string());
        assert_eq!(
            links[0].source_page.as_ref().map(|p| p.slug.clone()),
            Some("2026-05-29".to_string())
        );
    }

    #[test]
    fn backlinks_split_todo_prefix_from_body() {
        // Regression: the mobile client renders `block_text` through a
        // plain markdown tokenizer, so the `TODO `/`DONE ` prefix can't
        // leak into `block_text` — it has to live in `todo` so the
        // frontend can paint a checkbox instead of literal text.
        let (mut w, hlc) = ws();
        let target = open_or_create(&mut w, &hlc, "derick", "Derick", PageKind::Page).unwrap();
        let _ = target;

        let day = open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 2).unwrap()).unwrap();
        let _todo = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("TODO [[derick]] agendar papo"),
        )
        .unwrap();
        let _done = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("DONE [[derick]] enviou contrato"),
        )
        .unwrap();
        let _plain = append_block(&mut w, &hlc, Some(day), Some("[[derick]] note solta")).unwrap();

        let mut links = backlinks_for_target(&w, root(), "derick");
        // Order is DFS-stable; sort by text just so assertions don't
        // depend on insertion order.
        links.sort_by(|a, b| a.block_text.cmp(&b.block_text));

        assert_eq!(links.len(), 3);

        assert_eq!(links[0].block_text, "[[derick]] agendar papo");
        assert_eq!(links[0].todo, Some(TodoState::Todo));

        assert_eq!(links[1].block_text, "[[derick]] enviou contrato");
        assert_eq!(links[1].todo, Some(TodoState::Done));

        assert_eq!(links[2].block_text, "[[derick]] note solta");
        assert_eq!(links[2].todo, None);
    }

    #[test]
    fn backlinks_include_self_references_inside_the_same_page() {
        // Reproduce user-reported bug: in today's journal there is a
        // block whose text contains `[[2026-06-02]]` (a link back to
        // the page the block lives in). The "Linked from" panel
        // should still list it — the user typed the ref expecting it
        // to show up in their cross-references view.
        let (mut w, hlc) = ws();
        let today_date = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let today = open_journal(&mut w, &hlc, today_date).unwrap();
        let block = append_block(
            &mut w,
            &hlc,
            Some(today),
            Some("@Derick agendar papo [[2026-06-02]]"),
        )
        .unwrap();

        let meta = page_meta(&w, today).unwrap();
        let links = backlinks_for_page(&w, root(), &meta);

        assert!(
            links.iter().any(|l| l.block_id == block.to_string()),
            "self-ref backlink missing: links = {links:#?}"
        );
    }

    #[test]
    fn backlinks_for_page_dedup_slug_and_title() {
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, avelino).unwrap();

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        // One block mentions both forms.
        let n = append_block(&mut w, &hlc, Some(day), Some("[[avelino]] aka [[Avelino]]")).unwrap();
        let _ = edit_text;

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, n.to_string());
    }

    #[test]
    fn block_with_repeated_reference_only_emits_one_backlink() {
        // A block whose text contains the same `[[target]]` twice
        // should still appear once. The previous `outl-md` index used
        // a per-block `HashSet` to dedupe; we get the same behaviour
        // here because `text.contains(needle)` is a yes/no probe, not
        // a counter.
        let (mut w, hlc) = ws();
        let _avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let _ = append_block(
            &mut w,
            &hlc,
            Some(day),
            Some("[[avelino]] and again [[avelino]]"),
        )
        .unwrap();

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn backlink_carries_shallow_source_block_and_path() {
        // A backlink carries the referencing block as a SHALLOW leaf
        // (text + tokens + path), NOT its subtree. Materializing every
        // referencing block's children across the workspace — under the
        // workspace lock — is what froze input, so the index stops at
        // the leaf. Clients render the row from `source_block.tokens`.
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let _ = avelino;

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        let _first = append_block(&mut w, &hlc, Some(day), Some("warmup block")).unwrap();
        let parent =
            append_block(&mut w, &hlc, Some(day), Some("[[avelino]] led the project")).unwrap();
        let _child_a = append_block(&mut w, &hlc, Some(parent), Some("milestone A")).unwrap();
        let _child_b = append_block(&mut w, &hlc, Some(parent), Some("milestone B")).unwrap();

        let links = backlinks_for_target(&w, root(), "avelino");
        assert_eq!(links.len(), 1);
        let bl = &links[0];

        // Path = [1] because the matching block is the second direct
        // child of `day` (index 1, after `warmup block` at index 0).
        assert_eq!(bl.source_block_path, vec![1]);

        // The leaf itself, without its subtree.
        assert_eq!(bl.source_block.text, "[[avelino]] led the project");
        assert!(
            bl.source_block.children.is_empty(),
            "backlink leaf must be shallow (no subtree), got {:?}",
            bl.source_block.children
        );
    }

    #[test]
    fn person_page_picks_up_at_alias_mentions() {
        use crate::page::{page_meta, set_property};
        use crate::person::{PERSON_TYPE, TYPE_KEY};
        use outl_core::property::PropValue;
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        set_property(
            &mut w,
            &hlc,
            avelino,
            TYPE_KEY,
            Some(PropValue::Text(PERSON_TYPE.into())),
        )
        .unwrap();
        let meta = page_meta(&w, avelino).unwrap();

        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();
        // The `@` autocomplete inserts `[[@avelino]]` — a normal
        // wikilink whose target carries the `@`. The person's backlinks
        // panel must surface it even though the page slug is `avelino`.
        let at_mention =
            append_block(&mut w, &hlc, Some(day), Some("blocked on [[@avelino]]")).unwrap();
        let plain_mention =
            append_block(&mut w, &hlc, Some(day), Some("talked to [[avelino]] today")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        let block_ids: Vec<String> = links.iter().map(|l| l.block_id.clone()).collect();
        assert!(
            block_ids.contains(&at_mention.to_string()),
            "@-mention not surfaced in backlinks"
        );
        assert!(
            block_ids.contains(&plain_mention.to_string()),
            "plain [[avelino]] mention not surfaced"
        );
        // Plain pages (non-person) must NOT scan the `@` alias.
        let other = open_or_create(&mut w, &hlc, "projeto", "Projeto", PageKind::Page).unwrap();
        let projeto_meta = page_meta(&w, other).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("worked on [[@projeto]]")).unwrap();
        let projeto_links = backlinks_for_page(&w, root(), &projeto_meta);
        assert!(
            projeto_links.is_empty(),
            "non-person page must not match `@`-aliased refs"
        );
    }
}
