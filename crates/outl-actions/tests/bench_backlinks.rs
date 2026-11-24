//! Backlinks performance simulation — reproduces the "note with 760
//! backlinks is slow" report and isolates where the time goes.
//!
//! This is **not** a correctness test. It is a reproducible micro-bench
//! that separates the three cost layers a page's backlinks pass through
//! every time the page is opened:
//!
//!   1. **compute**  — `backlinks_for_page` walks *every block in the
//!      workspace* (O(total blocks), no inverted index for page refs).
//!   2. **transfer** — the resulting `Vec<Backlink>` serialized to JSON.
//!      This is the payload that crosses the Tauri IPC boundary on
//!      desktop/mobile. This bench builds through the workspace, where
//!      `source_block` is the full `OutlineNode` **subtree**, so it
//!      measures the *upper bound* of the payload. The shipped path is
//!      shallow plus the capped (4-level) subtree of each direct hit; the
//!      bench quantifies the difference by re-serializing a "shallow"
//!      copy (children dropped) and diffing the byte size.
//!   3. **render**   — real DOM/TUI cost lives in the client and can't be
//!      measured here; proxied by the total node + token counts the
//!      client has to reconcile.
//!
//! Run (release is mandatory — debug numbers are meaningless):
//!
//! ```sh
//! cargo test -p outl-actions --release --test bench_backlinks -- --ignored --nocapture
//! ```

use std::path::Path;
use std::time::Instant;

use outl_actions::{
    append_block, backlinks_for_page, open_or_create_page, page_meta, set_property, Backlink,
    OutlineNode, PageKind, PageMeta, FROM_TEMPLATE_KEY, TEMPLATE_KEY,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

const ROOT: &str = "/tmp/outl-bench";
/// Samples per timed operation; the median is reported.
const SAMPLES: usize = 7;

#[derive(Clone, Copy)]
enum Mechanism {
    /// A block whose text literally contains `[[hub]]` — the cheapest
    /// matching channel (substring scan).
    Ref,
    /// The report's actual case: `hub` is a `template::` page and each
    /// source block carries a `from-template:: hub` property, so the
    /// template channel (property lookups) fires instead of a substring.
    Template,
}

fn new_ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

/// Build a workspace with `n_backlinks` blocks that reference `hub`, each
/// carrying `subtree` children, plus `noise_per_note` non-matching blocks
/// per note so the O(total-blocks) walk has realistic weight.
fn build(
    n_backlinks: usize,
    subtree: usize,
    noise_per_note: usize,
    mech: Mechanism,
) -> (Workspace, PageMeta) {
    let (mut w, hlc) = new_ws();
    let hub = open_or_create_page(&mut w, &hlc, "hub", "hub", PageKind::Page).unwrap();
    if matches!(mech, Mechanism::Template) {
        set_property(
            &mut w,
            &hlc,
            hub,
            TEMPLATE_KEY,
            Some(PropValue::Text("mytpl".into())),
        )
        .unwrap();
    }

    for i in 0..n_backlinks {
        let slug = format!("note-{i}");
        let note = open_or_create_page(&mut w, &hlc, &slug, &slug, PageKind::Page).unwrap();

        let src = match mech {
            Mechanism::Ref => append_block(
                &mut w,
                &hlc,
                Some(note),
                Some("see [[hub]] for the weekly context"),
            )
            .unwrap(),
            Mechanism::Template => {
                let b = append_block(
                    &mut w,
                    &hlc,
                    Some(note),
                    Some("**Item:** follow-up from the 1:1"),
                )
                .unwrap();
                set_property(
                    &mut w,
                    &hlc,
                    b,
                    FROM_TEMPLATE_KEY,
                    Some(PropValue::Text("hub".into())),
                )
                .unwrap();
                b
            }
        };

        for c in 0..subtree {
            append_block(
                &mut w,
                &hlc,
                Some(src),
                Some(&format!(
                    "child {c}: some body text to make the subtree non-trivial"
                )),
            )
            .unwrap();
        }
        for n in 0..noise_per_note {
            append_block(
                &mut w,
                &hlc,
                Some(note),
                Some(&format!("filler line {n} with no reference at all")),
            )
            .unwrap();
        }
    }

    let meta = page_meta(&w, hub).unwrap();
    (w, meta)
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn count_nodes(n: &OutlineNode) -> usize {
    1 + n.children.iter().map(count_nodes).sum::<usize>()
}

/// Drop every backlink's subtree children — the shape desktop/mobile
/// actually consume (they render `source_block.tokens`, never children).
fn shallow(links: &[Backlink]) -> Vec<Backlink> {
    links
        .iter()
        .cloned()
        .map(|mut b| {
            b.source_block.children = Vec::new();
            b
        })
        .collect()
}

fn bench_one(label: &str, n_backlinks: usize, subtree: usize, noise: usize, mech: Mechanism) {
    let (w, meta) = build(n_backlinks, subtree, noise, mech);
    let root = Path::new(ROOT);

    // Warm up caches / branch predictors before timing.
    let bl = backlinks_for_page(&w, root, &meta);
    let n_results = bl.len();
    let total_nodes: usize = bl.iter().map(|b| count_nodes(&b.source_block)).sum();

    // Layer 1 — compute (full workspace walk + match + materialize).
    let walk_us = median(
        (0..SAMPLES)
            .map(|_| {
                let t = Instant::now();
                let r = backlinks_for_page(&w, root, &meta);
                std::hint::black_box(&r);
                t.elapsed().as_micros()
            })
            .collect(),
    );

    // Layer 2 — transfer (JSON payload over the IPC boundary).
    let full = &bl;
    let shal = shallow(&bl);
    let bytes_full = serde_json::to_vec(full).unwrap().len();
    let bytes_shallow = serde_json::to_vec(&shal).unwrap().len();

    let ser_full_us = median(
        (0..SAMPLES)
            .map(|_| {
                let t = Instant::now();
                let v = serde_json::to_vec(full).unwrap();
                std::hint::black_box(&v);
                t.elapsed().as_micros()
            })
            .collect(),
    );
    let ser_shallow_us = median(
        (0..SAMPLES)
            .map(|_| {
                let t = Instant::now();
                let v = serde_json::to_vec(&shal).unwrap();
                std::hint::black_box(&v);
                t.elapsed().as_micros()
            })
            .collect(),
    );

    let ws_blocks = n_backlinks * (1 + subtree + noise);
    let saved_pct = 100.0 * (bytes_full as f64 - bytes_shallow as f64) / bytes_full as f64;

    println!(
        "{label:<34} | {n_results:>4} | {ws_blocks:>7} | {total_nodes:>7} | \
         {walk_ms:>8.2} | {ser_full_ms:>8.2} | {ser_shallow_ms:>8.2} | \
         {kb_full:>8.1} | {kb_shallow:>8.1} | {saved_pct:>5.0}%",
        walk_ms = walk_us as f64 / 1000.0,
        ser_full_ms = ser_full_us as f64 / 1000.0,
        ser_shallow_ms = ser_shallow_us as f64 / 1000.0,
        kb_full = bytes_full as f64 / 1024.0,
        kb_shallow = bytes_shallow as f64 / 1024.0,
    );
}

#[test]
#[ignore = "performance simulation; run manually with --release --nocapture"]
fn simulate_760_backlinks() {
    println!();
    println!("outl backlinks performance simulation ({SAMPLES} samples, median)");
    println!(
        "{:<34} | {:>4} | {:>7} | {:>7} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | saved",
        "scenario", "n", "ws_blk", "nodes", "walk ms", "serF ms", "serS ms", "fullKB", "shalKB"
    );
    println!("{}", "-".repeat(130));

    // Pure baseline: 760 refs, no subtree, no noise — isolates the walk
    // floor for a tiny workspace.
    bench_one("ref  760  st=0  noise=0", 760, 0, 0, Mechanism::Ref);

    // Add workspace noise: same 760 backlinks, but the walk now crosses a
    // realistic block count. Shows how much the missing inverted index costs.
    bench_one("ref  760  st=0  noise=10", 760, 0, 10, Mechanism::Ref);
    bench_one("ref  760  st=0  noise=30", 760, 0, 30, Mechanism::Ref);

    // Add subtree weight per backlink: the children desktop/mobile ship
    // but never render. Watch fullKB vs shalKB diverge.
    bench_one("ref  760  st=5  noise=10", 760, 5, 10, Mechanism::Ref);
    bench_one("ref  760  st=15 noise=10", 760, 15, 10, Mechanism::Ref);

    // The report's real shape: a template used 760 times, each instance a
    // non-trivial subtree, in a busy workspace.
    bench_one("tmpl 760  st=5  noise=10", 760, 5, 10, Mechanism::Template);
    bench_one("tmpl 760  st=15 noise=30", 760, 15, 30, Mechanism::Template);
    println!();
}
