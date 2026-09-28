//! Regression net for [issue #281]: a `.md` that opens with a YAML
//! frontmatter fence must not be read as outline blocks.
//!
//! Same family as `multiline_block_roundtrip.rs` — the producer side of
//! invariant 8 — and the same failure shape one layer out. There the
//! `render → parse` roundtrip lost *some* of a block's text; here it
//! reads four lines of page metadata as five blocks, one of them `- ---`,
//! and the next write projects that reading back over the user's file.
//!
//! The reported sequence was four commands: `serve --once` left the file
//! alone (invariant 8's guard held, correctly, because the log had not
//! seen those lines), `doctor` warned, and then one `block append` made
//! the log hold them *as bullets* — at which point the guard's answer
//! flipped to yes and the fence was rewritten as an outline. The warning
//! was a countdown, not a wall, and it disappeared afterwards because the
//! file had become valid dialect.
//!
//! What this file pins:
//!
//! 1. the fence never becomes block text (`parse`);
//! 2. it comes back byte-for-byte (`render`);
//! 3. it reaches the **op log** as a page property, so a projection from
//!    the tree re-emits it instead of dropping it (`reconcile_md`);
//! 4. it is not reported as unlogged content, so the page is not frozen.
//!
//! Point 3 is the one that makes the other three safe. Preserving the
//! fence only in the parser would leave it outside the op log, which
//! either freezes the page forever (invariant 8 refuses every write) or
//! deletes it on the first projection — the two failures the issue asked
//! us to choose between and neither of which we accept.
//!
//! [issue #281]: https://github.com/outlmd/outl/issues/281

use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::parse::parse;
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;
use outl_md::sidecar::SidecarBlock;

/// The op-log page-property key that carries the verbatim fence.
///
/// Spelled literally here on purpose: it is a wire key that outlives any
/// one binary, so the test pins the string rather than the constant.
const FRONTMATTER_KEY: &str = "page-frontmatter";

/// The file from the issue report, verbatim.
const REPORTED: &str = "---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n";

/// An Obsidian vault page: the keys the issue named as destroyed on the
/// first block append (`aliases`, `tags`, `cssclass`, `publish`).
const OBSIDIAN: &str = "---\naliases: [Note One, note-one]\ntags:\n  - project/alpha\n  - status/wip\ncssclass: wide\npublish: false\n---\n\n- first bullet\n  - nested\n";

fn workspace() -> (tempfile::TempDir, Workspace, HlcGenerator) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).expect("workspace");
    let hlc = HlcGenerator::new(actor);
    (dir, ws, hlc)
}

/// Sidecar blocks as the log holds them for a freshly parsed page.
fn blocks_of(md: &str) -> Vec<SidecarBlock> {
    fn walk(nodes: &[outl_md::OutlineNode], out: &mut Vec<SidecarBlock>) {
        for n in nodes {
            out.push(SidecarBlock::from_text(
                NodeId::new(),
                out.len() + 1,
                0,
                &n.text,
            ));
            walk(&n.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&parse(md).blocks, &mut out);
    out
}

/// The defect itself: five blocks where the page has one.
#[test]
fn a_yaml_frontmatter_fence_does_not_become_outline_blocks() {
    let page = parse(REPORTED);
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["body"],
        "the fence was read as block text; the op log would hold it as bullets"
    );
    assert!(
        !texts.contains(&"---"),
        "a fence delimiter became a block: {texts:?}"
    );
}

/// And it comes back exactly as written.
#[test]
fn a_page_with_frontmatter_renders_back_byte_for_byte() {
    assert_eq!(render(&parse(REPORTED)), REPORTED);
    assert_eq!(render(&parse(OBSIDIAN)), OBSIDIAN);
}

/// `render → parse` is a fixpoint, so the file does not keep changing
/// shape on every save (corpus-gate property 2).
#[test]
fn a_frontmatter_page_is_a_render_parse_fixpoint() {
    for src in [REPORTED, OBSIDIAN] {
        let once = render(&parse(src));
        let twice = render(&parse(&once));
        assert_eq!(
            once, twice,
            "not a fixpoint:\n--- 1 ---\n{once}--- 2 ---\n{twice}"
        );
    }
}

/// The Obsidian fixture the issue asked for: a block list and a nested
/// mapping survive, keys and all, because the fence is preserved
/// verbatim rather than flattened into `key:: value`.
#[test]
fn an_obsidian_vault_page_keeps_every_frontmatter_key() {
    let rendered = render(&parse(OBSIDIAN));
    for key in [
        "aliases: [Note One, note-one]",
        "  - project/alpha",
        "  - status/wip",
        "cssclass: wide",
        "publish: false",
    ] {
        assert!(rendered.contains(key), "{key:?} was dropped:\n{rendered}");
    }
}

/// Page properties written in the outl dialect keep working next to a
/// fence — the header run must still see them.
#[test]
fn outl_page_properties_survive_alongside_a_frontmatter_fence() {
    let src = "---\ntitle: External\n---\ntype:: person\n\n- bio\n";
    let page = parse(src);
    assert_eq!(
        page.properties,
        vec![("type".to_string(), "person".to_string())],
        "the outl page-property header was lost behind the fence"
    );
    assert_eq!(render(&page), src);
}

/// A fence with no closing delimiter is not frontmatter, and its lines
/// stay content — the pre-existing permissive-recovery behaviour.
#[test]
fn an_unclosed_fence_is_still_recovered_as_content() {
    let src = "---\ntitle: half\n- bullet\n";
    let page = parse(src);
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert!(
        texts.contains(&"---") && texts.contains(&"title: half"),
        "an unterminated fence must stay content: {texts:?}"
    );
}

/// Warning line numbers stay **file**-relative once the fence is split
/// off, so `doctor` and the TUI banner point at the right row.
#[test]
fn parse_warning_lines_stay_file_relative_past_the_frontmatter() {
    // Lines 1-4 are the fence; line 5 blank; line 6 is the stray prose.
    let src = "---\ntitle: X\ntags: [a]\n---\n\nnot a bullet\n";
    let page = parse(src);
    assert_eq!(page.warnings.len(), 1, "{:?}", page.warnings);
    assert_eq!(
        page.warnings[0].line, 6,
        "warning points at the wrong source line: {:?}",
        page.warnings[0]
    );
}

/// The fence must not read as content the op log lacks.
///
/// A false positive here withholds `last_synced_hash` and refuses every
/// re-projection, freezing the page in both directions — the expensive
/// direction, and the one that would make every Obsidian page read-only.
#[test]
fn a_frontmatter_page_reports_no_unlogged_content() {
    for src in [REPORTED, OBSIDIAN] {
        let missing = outl_md::unlogged::content_lines_missing_from(src, &blocks_of(src));
        assert!(
            missing.is_empty(),
            "the fence read as unlogged content, which freezes the page: {missing:?}"
        );
    }
}

/// The fence reaches the op log as a page property on the page root.
///
/// This is what lets a projection from the tree re-emit it. Without it
/// the frontmatter lives only on disk, and invariant 8 has to choose
/// between freezing the page and deleting the fence.
#[test]
fn reconcile_puts_the_frontmatter_in_the_op_log() {
    let (dir, mut ws, hlc) = workspace();
    let md_path = dir.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&md_path, REPORTED).expect("write");

    let report = reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");
    assert_eq!(
        report.unlogged_lines, 0,
        "the page was left unsynced, so every later write is refused"
    );

    let page_id = NodeId::from_slug("fm");
    assert_eq!(
        ws.tree().property(page_id, FRONTMATTER_KEY),
        Some(&PropValue::Text("title: My Note\ntags: [a, b]".to_string())),
        "the frontmatter never reached the op log"
    );

    // And no block in the tree carries a fence delimiter as its text.
    let fence_blocks: Vec<String> = ws
        .tree()
        .iter_nodes()
        .filter_map(|(id, _, _)| ws.block_text(id))
        .filter(|t| t.trim() == "---" || t.starts_with("title: "))
        .collect();
    assert!(
        fence_blocks.is_empty(),
        "frontmatter lines were logged as blocks: {fence_blocks:?}"
    );
}

/// A render that lost the fence is a write that would delete it.
///
/// This is the window between upgrading to a binary that reads the fence
/// and a page's first reconcile: an older parser put the fence in the log
/// as bullets, so the render carries none. Every write gate asks this, and
/// the answer has to be "refuse" — otherwise the reported rewrite happens
/// one more time, unattended, on `outl serve`'s sweep.
#[test]
fn a_render_without_the_fence_is_reported_as_a_frontmatter_loss() {
    let bullets = "- ---\n- title: My Note\n- tags: [a, b]\n- ---\n- body\n";
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(REPORTED, bullets),
        4,
        "a render that dropped the fence must be reported"
    );
}

/// And a render that keeps it is not.
///
/// A false positive here refuses every projection of every Obsidian page,
/// which freezes the page in both directions — the expensive direction.
#[test]
fn a_render_that_keeps_the_fence_reports_no_loss() {
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(REPORTED, &render(&parse(REPORTED))),
        0
    );
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(OBSIDIAN, &render(&parse(OBSIDIAN))),
        0
    );
}

/// A peer that **edited** the frontmatter is a remote change, not a loss.
///
/// Refusing it would freeze any page whose fence another device touched,
/// which is issue #166 with the blame moved — the exact mistake the
/// block-level guard already paid for once.
#[test]
fn a_peer_edit_to_the_fence_is_not_reported_as_a_loss() {
    let peer = "---\ntitle: Renamed By Peer\n---\n\n- body\n";
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(REPORTED, peer),
        0,
        "a differing fence is a remote edit; refusing it freezes the page"
    );
}

/// Deleting the fence in the editor must stick.
///
/// The renderer materialises the property into file syntax, so a stale one
/// grows the fence back on the next projection — the user would be unable
/// to remove their own frontmatter.
#[test]
fn deleting_the_fence_clears_it_from_the_op_log() {
    let (dir, mut ws, hlc) = workspace();
    let md_path = dir.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&md_path, REPORTED).expect("write");
    reconcile_md(&mut ws, &hlc, &md_path, None).expect("first reconcile");

    std::fs::write(&md_path, "- body\n").expect("rewrite");
    reconcile_md(&mut ws, &hlc, &md_path, None).expect("second reconcile");

    assert_eq!(
        ws.tree().property(NodeId::from_slug("fm"), FRONTMATTER_KEY),
        None,
        "the fence was deleted on disk but survives in the log, so the next \
         projection writes it back"
    );
}

/// The whole reported sequence, end to end: reconcile the file, append a
/// block through the log, then project the tree back over the `.md`. The
/// fence has to survive the round trip through the op log.
#[test]
fn a_block_append_does_not_rewrite_the_frontmatter_as_bullets() {
    let (dir, mut ws, hlc) = workspace();
    let md_path = dir.path().join("pages").join("fm.md");
    std::fs::create_dir_all(md_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&md_path, REPORTED).expect("write");
    reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");

    // The projection a client writes back after a mutation is
    // `render(ParsedPage { frontmatter, properties, blocks })` built from
    // the tree. Model it with the two page-level facts the tree holds.
    let page_id = NodeId::from_slug("fm");
    let frontmatter = match ws.tree().property(page_id, FRONTMATTER_KEY) {
        Some(PropValue::Text(s)) => Some(s.clone()),
        _ => None,
    };
    assert!(
        frontmatter.is_some(),
        "the tree cannot re-emit a fence it does not hold"
    );

    let projected = format!(
        "---\n{}\n---\n\n- body\n- novo bloco\n",
        frontmatter.expect("checked above")
    );
    assert!(
        projected.starts_with("---\ntitle: My Note\ntags: [a, b]\n---\n"),
        "the projected .md lost the fence:\n{projected}"
    );
    // The projection must parse back to one frontmatter block and two
    // bullets — never to seven bullets.
    let back = parse(&projected);
    let texts: Vec<&str> = back.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, vec!["body", "novo bloco"], "got {texts:?}");
}

// --- the fence behind a UTF-8 byte order mark --------------------------
//
// `REPORTED` as a Windows editor writes it. An Obsidian vault on Windows
// is the shape this whole file exists for, so it is the shape the guard
// was blind to.

/// [`REPORTED`] with a UTF-8 BOM in front of the opening delimiter.
const BOM_REPORTED: &str = "\u{feff}---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n";

/// The parser reads it as frontmatter, exactly as it reads [`REPORTED`].
///
/// This much already held: `parse` drops the BOM before splitting. It is
/// the reason the scan disagreeing was a data bug and not merely an
/// inconsistency — the op log holds one block, and the fence's four lines
/// belong to a channel the block list cannot answer for.
#[test]
fn a_byte_order_mark_before_the_fence_is_still_frontmatter() {
    let page = parse(BOM_REPORTED);
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, vec!["body"], "got {texts:?}");
    assert_eq!(
        page.frontmatter.as_deref(),
        Some("title: My Note\ntags: [a, b]")
    );
}

/// **The one thing the first save normalizes away: the BOM itself.**
///
/// Deliberate, and the only reachable alternative is worse. No renderer
/// re-emits U+FEFF, so preserving it in the parser would leave the file
/// changing shape on every save — the corpus gate's property 2. The fence
/// comes back byte for byte; only the encoding artifact in front of it is
/// dropped, once, and the file is a fixpoint afterwards.
#[test]
fn the_first_save_drops_the_bom_and_keeps_the_fence_byte_for_byte() {
    let once = render(&parse(BOM_REPORTED));
    assert_eq!(once, REPORTED, "the fence must survive; the BOM must not");
    assert_eq!(
        render(&parse(&once)),
        once,
        "the page must settle on the second read, not keep changing shape"
    );
}

/// The fence must not read as content the op log lacks — BOM or no BOM.
///
/// The sibling of `a_frontmatter_page_reports_no_unlogged_content`, and
/// the half that was broken: `frontmatter_line_count` could not see a
/// fence behind a BOM, so `skip` was `0` and all four fence lines were
/// reported. `reconcile_md` then wrote an empty `last_synced_hash`, the
/// page reconciled on every boot, and every re-projection was refused —
/// permanently, because the write that would have dropped the BOM is
/// exactly the write being refused.
#[test]
fn a_bom_before_the_fence_reports_no_unlogged_content() {
    let missing =
        outl_md::unlogged::content_lines_missing_from(BOM_REPORTED, &blocks_of(BOM_REPORTED));
    assert!(
        missing.is_empty(),
        "the fence behind a BOM read as unlogged content, which freezes the \
         page in both directions: {missing:?}"
    );
}

/// The same question without a fence: a BOM glued to the first bullet.
///
/// `parse` drops it, so the log holds `body` and the disk line is
/// `\u{feff}- body`. U+FEFF is not whitespace, so nothing trims it and the
/// line matches no block — the same freeze, reached without any
/// frontmatter at all.
#[test]
fn a_bom_without_a_fence_reports_no_unlogged_content() {
    let src = "\u{feff}- body\n  - nested\n";
    let missing = outl_md::unlogged::content_lines_missing_from(src, &blocks_of(src));
    assert!(
        missing.is_empty(),
        "a BOM glued to the first bullet read as unlogged content: {missing:?}"
    );
}

/// And the guard has not gone blind behind a BOM: real unlogged content is
/// still reported.
#[test]
fn a_bom_does_not_hide_content_the_log_lacks() {
    let disk = "\u{feff}---\ntitle: My Note\ntags: [a, b]\n---\n\n- body\n- typed in vim\n";
    let logged = blocks_of(BOM_REPORTED);
    assert_eq!(
        outl_md::unlogged::content_lines_missing_from(disk, &logged),
        vec!["typed in vim".to_string()],
        "stripping the BOM must not widen which lines count as logged"
    );
}

/// The #281 write guard has to see the fence behind a BOM too.
///
/// `frontmatter_lines_missing_from` is the only channel that can answer
/// for the fence, and it read `0` for every BOM'd file — blind in exactly
/// the class of file where the rewrite happens (a vault written on
/// Windows). The log holds the fence as bullets, the render therefore
/// carries none, and the write deletes it.
#[test]
fn a_render_without_the_fence_is_a_loss_behind_a_bom_too() {
    let bullets = "- ---\n- title: My Note\n- tags: [a, b]\n- ---\n- body\n";
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(BOM_REPORTED, bullets),
        4,
        "a render that dropped the fence must be reported, BOM or not"
    );
}

/// And a render that keeps it is not reported — the false-positive
/// direction, which freezes the page.
///
/// Note the render has no BOM (nothing re-emits one) while the disk file
/// does: the two sides are asked with different leading bytes on purpose,
/// which is the asymmetry that must not read as a loss.
#[test]
fn a_render_that_keeps_the_fence_reports_no_loss_behind_a_bom() {
    assert_eq!(
        outl_md::unlogged::frontmatter_lines_missing_from(
            BOM_REPORTED,
            &render(&parse(BOM_REPORTED))
        ),
        0
    );
}

/// End to end: a BOM'd vault page reconciles clean instead of freezing.
///
/// `unlogged_lines == 0` is the whole point — a non-zero count makes
/// `reconcile_md` write an empty `last_synced_hash`, which is what turns
/// "reconciles again next pass" into "never syncs again".
#[test]
fn reconcile_does_not_freeze_a_page_whose_fence_sits_behind_a_bom() {
    let (dir, mut ws, hlc) = workspace();
    let md_path = dir.path().join("pages").join("bom.md");
    std::fs::create_dir_all(md_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&md_path, BOM_REPORTED).expect("write");

    let report = reconcile_md(&mut ws, &hlc, &md_path, None).expect("reconcile");
    assert_eq!(
        report.unlogged_lines, 0,
        "the page was left unsynced, so every later write is refused"
    );

    let sidecar =
        outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&md_path)).expect("sidecar");
    assert!(
        !sidecar.last_synced_hash.is_empty(),
        "an empty hash never matches the file, so the page reconciles forever"
    );

    assert_eq!(
        ws.tree()
            .property(NodeId::from_slug("bom"), FRONTMATTER_KEY),
        Some(&PropValue::Text("title: My Note\ntags: [a, b]".to_string())),
        "the frontmatter never reached the op log"
    );
}
