//! Randomised agreement check between the backlinks index and a naive scan
//! (issue [#213] item 4, [RFC 0169]).
//!
//! ## What this is for
//!
//! The inverted index exists because the naive answer — walk every block in
//! the workspace looking for mentions — cost 3.8 s on a page with 760
//! backlinks. Replacing an O(blocks) scan with a lookup is exactly the kind of
//! optimisation that is correct on the cases its author thought of and wrong on
//! the ones they did not: a ref that is also a tag, a page whose title differs
//! from its slug, a block that mentions the same page twice, a ref to a page
//! that does not exist.
//!
//! RFC 0169 proposed a property test for this and it was never written, so the
//! index's agreement with the thing it replaced has been resting on hand-picked
//! examples.
//!
//! ## Why it generates rather than enumerates
//!
//! Hand-written cases test what the author already suspected. These workspaces
//! are generated from a fixed seed so a failure reproduces exactly, while the
//! shapes themselves — how many pages, which of them are referenced, whether a
//! mention is a `[[ref]]` or a `#tag`, whether a block mentions nothing —
//! are not ones anyone chose.
//!
//! Deliberately no `proptest` dependency: a 64-bit LCG with a fixed seed gives
//! reproducibility without adding a crate to `outl-actions`, which is published
//! and linked into the mobile app.
//!
//! [#213]: https://github.com/outlmd/outl/issues/213
//! [RFC 0169]: ../../docs/rfcs/0169-backlinks.md

use std::collections::BTreeSet;
use std::path::Path;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, extract_refs, find_by_slug,
    list_pages, open_or_create_page, PageKind, PageMeta,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// Deterministic PRNG. Fixed seed in, same workspace out — a failure here is
/// reproducible by re-running, not by getting lucky.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        // Numerical Recipes constants; quality is irrelevant here, only
        // reproducibility and spread.
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 33) as usize % n.max(1)
    }
}

/// One generated workspace: a handful of pages, each with blocks that mention
/// zero or more of the others.
fn generate(root: &Path, rng: &mut Lcg) -> (Workspace, Vec<PageMeta>) {
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let page_count = 3 + rng.below(6);
    let slugs: Vec<String> = (0..page_count).map(|i| format!("page-{i}")).collect();

    {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();

        for slug in &slugs {
            open_or_create_page(&mut w, &hlc, slug, slug, PageKind::Page).unwrap();
        }

        for slug in &slugs {
            let page = find_by_slug(&w, slug).unwrap();
            let blocks = 1 + rng.below(6);
            for b in 0..blocks {
                // Four shapes, so the index is asked about more than the happy
                // path: a plain ref, a ref to a page that does not exist, two
                // refs in one block, and a block with nothing to find.
                // Every block gets text unique across the whole workspace.
                // The sets below are keyed on that text, so two source pages
                // emitting the same string would hide exactly the bugs this
                // test is for: the index returning a backlink twice, or
                // dropping one of two identical-looking blocks.
                let tag = format!("{slug}-b{b}");
                let text = match rng.below(4) {
                    0 => {
                        let target = &slugs[rng.below(slugs.len())];
                        format!("{tag} mentions [[{target}]]")
                    }
                    1 => format!("{tag} mentions [[ghost-{}]]", rng.below(99)),
                    2 => {
                        let a = &slugs[rng.below(slugs.len())];
                        let c = &slugs[rng.below(slugs.len())];
                        format!("{tag} mentions [[{a}]] and again [[{c}]]")
                    }
                    _ => format!("{tag} has nothing to find"),
                };
                append_block(&mut w, &hlc, Some(page), Some(&text)).unwrap();
            }
        }

        for meta in list_pages(&w) {
            let id = find_by_slug(&w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&w, root, id).unwrap();
        }
    }

    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let metas = list_pages(&w);
    (w, metas)
}

/// The thing the index replaced: read every projected `.md`, extract each
/// line's refs, keep the lines that name `slug`.
///
/// Written out longhand against the files on disk on purpose. Calling any
/// indexed helper here would make the test compare the index with itself,
/// which is the usual way an agreement test ends up proving nothing.
///
/// Compares **block text** rather than ids: the ids are exactly what the index
/// is responsible for resolving, so using them on this side would import the
/// thing under test into the reference.
fn naive_backlink_texts(root: &Path, slug: &str) -> BTreeSet<String> {
    let mut hits = BTreeSet::new();
    let pages_dir = root.join("pages");
    let Ok(entries) = std::fs::read_dir(&pages_dir) else {
        return hits;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let body = line.trim_start().trim_start_matches("- ").trim();
            if body.is_empty() {
                continue;
            }
            if extract_refs(body).iter().any(|r| r == slug) {
                hits.insert(body.to_string());
            }
        }
    }
    hits
}

#[test]
fn the_index_agrees_with_a_naive_scan_on_generated_workspaces() {
    // Enough shapes to cross the interesting cases many times over, and fast
    // enough to stay in the normal test run rather than behind `--ignored`,
    // which is how the existing benchmark stopped guarding anything.
    let mut total_backlinks = 0usize;
    for seed in 0..12u64 {
        let dir = TempDir::new().unwrap();
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
        let (w, metas) = generate(dir.path(), &mut rng);
        let index = build_backlink_index(&w, dir.path());

        for meta in &metas {
            let raw: Vec<String> = index
                .for_page(&w, meta)
                .into_iter()
                .map(|b| b.block_text.trim().to_string())
                .collect();
            let indexed: BTreeSet<String> = raw.iter().cloned().collect();
            // A set would swallow a duplicate. One block mentioning a page
            // twice must still be one backlink (`block_with_repeated_reference_
            // only_emits_one_backlink` pins the unit case; this pins it across
            // generated workspaces).
            assert_eq!(
                raw.len(),
                indexed.len(),
                "seed {seed}: the index returned the same block twice for {:?}",
                meta.slug,
            );
            let naive = naive_backlink_texts(dir.path(), &meta.slug);

            assert_eq!(
                indexed,
                naive,
                "seed {seed}: the index and a naive scan disagree about who \
                 links to {:?}\n  index only: {:?}\n  scan only:  {:?}",
                meta.slug,
                indexed.difference(&naive).collect::<Vec<_>>(),
                naive.difference(&indexed).collect::<Vec<_>>(),
            );
            total_backlinks += indexed.len();
        }
    }

    // Not vacuous: two empty sets are equal, so a generator that produced no
    // mentions at all — or an index that returned nothing — would satisfy every
    // assertion above without comparing anything.
    assert!(
        total_backlinks > 50,
        "the generated workspaces produced only {total_backlinks} backlinks; \
         the agreement assertions were comparing empty sets",
    );
}

#[test]
fn a_page_nobody_links_to_has_no_backlinks() {
    // Guards the guard. If `for_page` ever returned everything, or the naive
    // scan ever returned nothing, the agreement test above would still pass
    // while both halves were broken in the same direction.
    let dir = TempDir::new().unwrap();
    let mut rng = Lcg(7);
    let (w, _metas) = generate(dir.path(), &mut rng);
    let index = build_backlink_index(&w, dir.path());

    let orphan = "page-nobody-mentions";
    assert!(
        index.for_target(orphan).is_empty(),
        "a page no block mentions must have no backlinks",
    );
    assert!(
        naive_backlink_texts(dir.path(), orphan).is_empty(),
        "the reference scan must agree that nobody mentions it",
    );
}
