//! What carrying a citing block's subtree costs on a real graph.
//!
//! Not a correctness test. Root `CLAUDE.md` invariant 15 asks for the
//! number measured at the size a graph really reaches, and the fixture's
//! nine blocks cannot say. It reads a **copy** of a workspace's `.md` and
//! `.outl` files (never the op log, never the original: the from-disk
//! reader can rename a legacy sidecar).
//!
//! ```sh
//! OUTL_BENCH_GRAPH=~/outl-p2p OUTL_BENCH_PAGE=buser \
//!   cargo test -p outl-actions --release --test bench_backlinks_subtree -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use outl_actions::{
    build_backlink_index_from_disk, page::page_id_from_slug, PageKind, PageMeta, SUBTREE_MAX_DEPTH,
};
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;

fn rss_kb() -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn copy_markdown(from: &Path, to: &Path, sub: &str) -> Vec<PathBuf> {
    let dst = to.join(sub);
    std::fs::create_dir_all(&dst).unwrap();
    let mut mds = Vec::new();
    for e in std::fs::read_dir(from.join(sub)).unwrap().flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if ext == "md" || ext == "outl" {
            std::fs::copy(&p, dst.join(e.file_name())).unwrap();
            if ext == "md" {
                mds.push(p);
            }
        }
    }
    mds
}

fn meta(slug: &str, kind: PageKind) -> PageMeta {
    PageMeta {
        id: page_id_from_slug(slug).to_string(),
        slug: slug.to_string(),
        title: slug.to_string(),
        kind,
        icon: None,
        pinned: false,
        page_type: None,
        aliases: Vec::new(),
    }
}

fn nodes(n: &[outl_actions::OutlineNode]) -> usize {
    n.iter().map(|x| 1 + nodes(&x.children)).sum()
}

#[test]
#[ignore = "needs OUTL_BENCH_GRAPH; run with --release --ignored --nocapture"]
fn subtree_cost_on_a_real_graph() {
    let graph = PathBuf::from(std::env::var("OUTL_BENCH_GRAPH").expect("OUTL_BENCH_GRAPH"));
    let page = std::env::var("OUTL_BENCH_PAGE").unwrap_or_else(|_| "buser".into());
    let tmp = tempfile::TempDir::new().unwrap();
    let mut metas = Vec::new();
    for p in copy_markdown(&graph, tmp.path(), "pages") {
        metas.push(meta(
            p.file_stem().unwrap().to_str().unwrap(),
            PageKind::Page,
        ));
    }
    for p in copy_markdown(&graph, tmp.path(), "journals") {
        metas.push(meta(
            p.file_stem().unwrap().to_str().unwrap(),
            PageKind::Journal,
        ));
    }
    println!("pages: {}", metas.len());

    let before = rss_kb();
    let t = Instant::now();
    let index = build_backlink_index_from_disk(&metas, tmp.path());
    let build = t.elapsed();
    let after = rss_kb();
    println!(
        "build: {build:?}  rss {} MB -> {} MB (+{} MB)",
        before / 1024,
        after / 1024,
        (after.saturating_sub(before)) / 1024
    );

    let ws = Workspace::open_in_memory(ActorId::new()).unwrap();
    let target = metas
        .iter()
        .find(|m| m.slug == page)
        .expect("OUTL_BENCH_PAGE not found");
    let t = Instant::now();
    let split = index.for_page_split(&ws, target);
    let lookup = t.elapsed();
    println!(
        "lookup (the part that runs under the workspace lock): {lookup:?}, direct {} namespaced {}",
        split.direct.len(),
        split.namespaced.len()
    );

    let t = Instant::now();
    let subs: Vec<_> = split
        .direct
        .iter()
        .map(|b| index.subtree(&b.block_id))
        .collect();
    println!(
        "subtree Arc fetch for {} hits: {:?}",
        subs.len(),
        t.elapsed()
    );

    let shallow: usize = split
        .direct
        .iter()
        .map(|b| serde_json::to_vec(b).unwrap().len())
        .sum();
    let t = Instant::now();
    let full: Vec<_> = split
        .direct
        .iter()
        .cloned()
        .zip(&subs)
        .map(|(b, s)| match s {
            Some(c) => b.with_children(c),
            None => b,
        })
        .collect();
    let copy = t.elapsed();
    let deep: usize = full
        .iter()
        .map(|b| serde_json::to_vec(b).unwrap().len())
        .sum();
    let kids: usize = subs.iter().flatten().map(|s| nodes(s)).sum();
    println!(
        "deep copy (after the locks drop): {copy:?}; hits with children {}/{}; child nodes {kids} (cap {SUBTREE_MAX_DEPTH})",
        subs.iter().flatten().count(),
        subs.len()
    );
    println!(
        "payload: shallow {} KB -> with children {} KB",
        shallow / 1024,
        deep / 1024
    );
}
