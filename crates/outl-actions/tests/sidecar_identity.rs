//! A sidecar that describes a different page than the one being read.
//!
//! `read_page_outline` zips the parsed `.md` against `sidecar.blocks`
//! positionally and, until this check, never compared `sidecar.page_id`
//! with the page it was handed. On the maintainer's workspace that is
//! true of 1,775 sidecars: a Roam re-import on 2026-11-12 moved the
//! previous generation of the graph to the trash and nothing
//! re-projected the `.md` / `.outl` pairs, so those files describe a
//! page root the tree no longer holds under `NodeId::root()`.
//! Measurements and the staged repair this is pulled ahead of:
//! `docs/superpowers/specs/2026-11-20-stale-projection-repair-design.md`.
//!
//! **One of them is about the log rather than the verdict.** The check
//! shipped with a `warn!` inside the per-page read, documented as firing
//! "once per affected page **open**, never per frame" — and the same
//! function is the body of `build_backlink_index_from_disk`, which reads
//! every page in the workspace on a path an ordinary edit reaches. At
//! 1,775 of 3,390 that is 1,775 records per rescan, which is the same
//! failure as the silence the check was added to break. The volume is
//! pinned below as a **record count by level**, because on a nine-page
//! fixture every other assertion in this file passes either way.
//!
//! **Half of these tests exist for the direction nobody reported.** The
//! check is on the page-open path of every client, so the tempting fix
//! — refuse the read — trades a stale page for no page at all, and
//! `build_backlink_index_from_disk` skips a page whose read errors, so
//! it would also take 1,775 pages' `[[ref]]`s out of the backlinks
//! panel the mismatch was reported *from*. That is the deleting half of
//! root `CLAUDE.md` invariant 8's mirrored pair, arrived at by fixing
//! the reported half. The five states a page with nothing on disk
//! takes are asserted rather than reasoned about, because refusing on a
//! predicate narrower than the question warranted is what froze every
//! empty page a peer wrote into ([#332]).
//!
//! [#332]: https://github.com/outlmd/outl/issues/332

use std::path::{Path, PathBuf};

use outl_actions::journal::page_md_path;
use outl_actions::page::{open_or_create, page_meta, PageKind, PageMeta};
use outl_actions::{append_block, read_page_outline, SidecarIdentity};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn workspace() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    (ws, hlc)
}

/// A page with two blocks, one of them citing another page, projected to
/// `.md` + sidecar the ordinary way.
fn projected_page(tmp: &TempDir, slug: &str) -> (Workspace, PageMeta, PathBuf) {
    let (mut ws, hlc) = workspace();
    let page = open_or_create(&mut ws, &hlc, slug, slug, PageKind::Page).expect("create page");
    append_block(&mut ws, &hlc, Some(page), Some("cites [[elsewhere]]")).expect("first block");
    append_block(&mut ws, &hlc, Some(page), Some("and a second block")).expect("second block");
    let meta = page_meta(&ws, page).expect("page meta");
    outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).expect("project the page");
    let md = page_md_path(tmp.path(), &meta);
    (ws, meta, md)
}

/// Repoint the page's sidecar at a root the tree does not hold there,
/// leaving the block entries untouched. Returns the root it now claims.
fn repoint_sidecar_at_another_page(md: &Path) -> String {
    let path = outl_md::resolve_sidecar_path(md);
    let mut sc = outl_md::sidecar::read(&path).expect("read the projected sidecar");
    let other = NodeId::new();
    sc.page_id = other;
    outl_md::sidecar::write(&path, &sc).expect("write the repointed sidecar");
    other.to_string()
}

#[test]
fn a_healthy_page_reports_its_own_root() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, _md) = projected_page(&tmp, "notes");

    let outline = read_page_outline(tmp.path(), &meta).expect("a healthy page reads");
    assert_eq!(outline.sidecar_identity, SidecarIdentity::MatchesPage);
}

#[test]
fn a_sidecar_naming_another_root_is_reported_with_the_root_it_claims() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, md) = projected_page(&tmp, "notes");
    let other = repoint_sidecar_at_another_page(&md);

    let outline = read_page_outline(tmp.path(), &meta).expect("the read must still succeed");
    assert_eq!(
        outline.sidecar_identity,
        SidecarIdentity::NamesAnotherPage {
            page_id: other.clone()
        },
        "the verdict has to name the root the sidecar claims — a caller \
         cannot act on a bare boolean"
    );
    assert_ne!(other, meta.id, "the fixture repointed nothing");
}

/// **The mirror, stated as a test rather than as a sentence.**
///
/// The page opens, with its content and with the *sidecar's* block ids —
/// not the transient ones `outline_from_parsed` mints when there is no
/// sidecar to read. A refusal here would be the deleting direction; so
/// would quietly dropping the sidecar and re-minting every id, which
/// breaks every `((blk-…))` handle into the page.
#[test]
fn a_reported_mismatch_changes_nothing_about_the_outline() {
    let tmp = TempDir::new().unwrap();
    let (_ws, meta, md) = projected_page(&tmp, "notes");

    let before = read_page_outline(tmp.path(), &meta).expect("a healthy page reads");
    repoint_sidecar_at_another_page(&md);
    let after = read_page_outline(tmp.path(), &meta).expect("the read must not refuse");

    assert_eq!(
        after.nodes.len(),
        2,
        "the page lost blocks over a condition that is about the sidecar's \
         header, not its entries"
    );
    let texts: Vec<&str> = after.nodes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(texts, vec!["cites [[elsewhere]]", "and a second block"]);
    let ids_before: Vec<&str> = before.nodes.iter().map(|n| n.id.as_str()).collect();
    let ids_after: Vec<&str> = after.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids_after, ids_before,
        "the block ids must still come off the sidecar; re-minting them \
         breaks every ((blk-…)) handle into this page"
    );
}

/// The second half of the same mirror, on the surface that reported the
/// bug in the first place.
///
/// `build_backlink_index_from_disk` `warn!`s and `continue`s on an `Err`
/// from `read_page_outline`, so a refusing guard would take this page's
/// `[[elsewhere]]` out of every backlinks panel. Asserting on the index
/// rather than on the read is what makes that unfakeable.
#[test]
fn the_backlink_index_still_carries_the_refs_of_a_mismatched_page() {
    let tmp = TempDir::new().unwrap();
    let (ws, meta, md) = projected_page(&tmp, "notes");
    repoint_sidecar_at_another_page(&md);

    let metas = vec![meta.clone()];
    let index = outl_actions::build_backlink_index_from_disk(&metas, tmp.path());
    let target = PageMeta {
        slug: "elsewhere".to_string(),
        title: "elsewhere".to_string(),
        ..meta.clone()
    };
    let rows = index.for_page(&ws, &target);

    assert_eq!(
        rows.len(),
        1,
        "the citing page was skipped, so its [[ref]] went quiet — that is \
         the deleting direction of the same divergence"
    );
    assert_eq!(rows[0].source_block.text, "cites [[elsewhere]]");
}

/// One "nothing on disk to vouch for" shape: what it is called, how to
/// put a workspace into it, and the verdict it must produce.
type EmptyPageCase = (
    &'static str,
    fn(&TempDir) -> (Workspace, PageMeta),
    SidecarIdentity,
);

/// The shapes a page with no content of its own takes, each of which is
/// ordinary rather than a defect — and each of which a tightened guard
/// would freeze.
fn empty_page_cases() -> Vec<EmptyPageCase> {
    vec![
        // A live page whose `.md` this device has never written: a peer
        // shipped only the ops.
        (
            "a live page with no .md yet",
            |_tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page =
                    open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // A `.md` with no sidecar beside it: a peer shipped the
        // projection, or an orphan scan has not run yet.
        (
            "a .md with no sidecar beside it",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page =
                    open_or_create(&mut ws, &hlc, "shipped", "Shipped", PageKind::Page).unwrap();
                append_block(&mut ws, &hlc, Some(page), Some("from a peer")).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                let md = page_md_path(tmp.path(), &meta);
                std::fs::create_dir_all(md.parent().unwrap()).unwrap();
                std::fs::write(&md, "- from a peer\n").unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // A sidecar with the right `page_id` and no entries. It names
        // this page, so it is not a mismatch — the ids just come out
        // transient, which `read_page_outline` already documents.
        (
            "an empty sidecar for this page",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = open_or_create(&mut ws, &hlc, "fresh", "Fresh", PageKind::Page).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                let md = page_md_path(tmp.path(), &meta);
                std::fs::create_dir_all(md.parent().unwrap()).unwrap();
                let text = "- \n";
                std::fs::write(&md, text).unwrap();
                let sc = outl_md::sidecar::Sidecar::new_for_page(
                    page,
                    &outl_md::sidecar::file_hash(text),
                );
                outl_md::sidecar::write(&outl_md::resolve_sidecar_path(&md), &sc).unwrap();
                (ws, meta)
            },
            SidecarIdentity::MatchesPage,
        ),
        // Journal-first means this is the product's first screen, every
        // day, before anything has written it.
        (
            "a brand-new journal day, before its first projection",
            |_tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = outl_actions::open_today(&mut ws, &hlc).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::Absent,
        ),
        // …and the same day once outl itself has written it.
        (
            "a brand-new journal day, freshly projected",
            |tmp: &TempDir| {
                let (mut ws, hlc) = workspace();
                let page = outl_actions::open_today(&mut ws, &hlc).unwrap();
                let meta = page_meta(&ws, page).unwrap();
                outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
                (ws, meta)
            },
            SidecarIdentity::MatchesPage,
        ),
    ]
}

// ---------------------------------------------------------------------
// How loud the check is, which is a different question from what it says.
// ---------------------------------------------------------------------

/// Records the calling thread emitted, by level.
#[derive(Default)]
struct Counts {
    warns: std::sync::atomic::AtomicUsize,
    debugs: std::sync::atomic::AtomicUsize,
}

thread_local! {
    static SINK: std::cell::RefCell<Option<std::sync::Arc<Counts>>> =
        const { std::cell::RefCell::new(None) };
}

/// Routes every event to the calling thread's `SINK`, if it has one.
///
/// Installed **globally, once**, and reporting `TRACE` unconditionally.
/// `with_default` alone does not work: `tracing`'s level filter is
/// process-wide, so a thread leaving its `with_default` drops the filter
/// back to off and a sibling test's event is discarded before any
/// subscriber sees it — the capture then comes back empty and reads as
/// "nothing was logged", which is the assertion passing for the wrong
/// reason. `outl_plugins::net::tests` carries the same harness and the
/// same note; this binary cannot import it across the crate edge.
struct Router;

impl tracing::Subscriber for Router {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        SINK.with(|s| {
            if let Some(c) = s.borrow().as_ref() {
                let bucket = match *event.metadata().level() {
                    tracing::Level::WARN => &c.warns,
                    tracing::Level::DEBUG => &c.debugs,
                    _ => return,
                };
                bucket.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn counting<T>(f: impl FnOnce() -> T) -> (T, std::sync::Arc<Counts>) {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        tracing::subscriber::set_global_default(Router)
            .expect("no other global tracing subscriber may be installed in this test binary");
    });
    let counts = std::sync::Arc::new(Counts::default());
    SINK.with(|s| *s.borrow_mut() = Some(counts.clone()));
    let out = f();
    SINK.with(|s| *s.borrow_mut() = None);
    (out, counts)
}

fn warns(c: &Counts) -> usize {
    c.warns.load(std::sync::atomic::Ordering::Relaxed)
}

fn debugs(c: &Counts) -> usize {
    c.debugs.load(std::sync::atomic::Ordering::Relaxed)
}

/// `pages` pages in one workspace, every one projected and then
/// repointed at a root the tree does not hold there.
///
/// One workspace, not one per page, because
/// `build_backlink_index_from_disk` takes a `&[PageMeta]` and a root and
/// reads only disk — the walk has to be the real one to count what it
/// logs.
fn mismatched_pages(tmp: &TempDir, pages: usize) -> Vec<PageMeta> {
    let (mut ws, hlc) = workspace();
    let mut metas = Vec::with_capacity(pages);
    for i in 0..pages {
        let slug = format!("notes-{i}");
        let page = open_or_create(&mut ws, &hlc, &slug, &slug, PageKind::Page).expect("page");
        append_block(&mut ws, &hlc, Some(page), Some("cites [[elsewhere]]")).expect("block");
        let meta = page_meta(&ws, page).expect("meta");
        outl_actions::apply_page_md_with_sidecar(&ws, tmp.path(), page).expect("project");
        repoint_sidecar_at_another_page(&page_md_path(tmp.path(), &meta));
        metas.push(meta);
    }
    metas
}

/// **The volume, not the wording.**
///
/// `identify` cannot tell one page open from a whole-workspace rescan,
/// and the rescan is on an ordinary edit path (`BacklinkRefresh::DropAll`
/// is the GUI commit default; the TUI re-spawns the build after a
/// cross-page save). So a `warn!` there scales with the graph: 1,775
/// records per rescan on the workspace this check was written for, which
/// is the silence it exists to break, restated at volume.
///
/// Asserted as a count **by level**, because every other test in this
/// file passes with the `warn!` restored. The aggregate is `outl doctor`'s
/// — it reports these pages as stale projections with the lines at risk
/// per page — so there is deliberately no counted `warn!` to assert
/// anywhere either.
#[test]
fn a_mismatch_is_logged_per_read_at_debug_and_never_at_warn() {
    const PAGES: usize = 3;
    let tmp = TempDir::new().unwrap();
    let metas = mismatched_pages(&tmp, PAGES);

    // One page open.
    let (one, counts) = counting(|| {
        read_page_outline(tmp.path(), &metas[0]).expect("a mismatched page still reads")
    });
    assert!(
        matches!(
            one.sidecar_identity,
            SidecarIdentity::NamesAnotherPage { .. }
        ),
        "the fixture repointed nothing, so this test could not fail"
    );
    assert_eq!(
        warns(&counts),
        0,
        "one page open must not warn: the same frame is the body of a \
         whole-workspace rescan, so a level that is right for one read is \
         the only level that is right at all"
    );
    assert!(
        debugs(&counts) >= 1,
        "the path is still reported somewhere — a silent check is the \
         defect this was added to close, and `debug!` is what keeps it \
         collectable with RUST_LOG=outl_actions=debug"
    );

    // The same reads, as the rescan an edit triggers.
    let (index, counts) =
        counting(|| outl_actions::build_backlink_index_from_disk(&metas, tmp.path()));
    assert_eq!(
        index.len(),
        PAGES,
        "the walk must really have read all {PAGES} pages, or the record \
         count below is counting nothing"
    );
    assert_eq!(
        warns(&counts),
        0,
        "the rescan emitted {} warning(s) for {PAGES} mismatched pages. This \
         is the one assertion that scales: at 1,775 of 3,390 it is 1,775 \
         records per edit. `outl doctor` owns the aggregate.",
        warns(&counts)
    );
    assert!(
        debugs(&counts) >= PAGES,
        "each read still names its own path at debug; got {}",
        debugs(&counts)
    );
}

#[test]
fn a_page_with_nothing_to_vouch_for_is_never_reported_as_a_mismatch() {
    for (name, prepare, expected) in empty_page_cases() {
        let tmp = TempDir::new().unwrap();
        let (_ws, meta) = prepare(&tmp);

        let outline = read_page_outline(tmp.path(), &meta)
            .unwrap_or_else(|e| panic!("{name}: the read must not refuse: {e}"));

        assert_eq!(
            outline.sidecar_identity, expected,
            "{name}: reporting this as a mismatch is how a later \"tighten \
             the check\" freezes an ordinary page"
        );
    }
}
