//! `[[ref]]` extraction had **four** implementations and the one that
//! decided backlinks was the only one blind to the inline grammar.
//!
//! `outl_md::inline::tokenize` is what renders a block, resolves a
//! click (`ref_at_cursor`) and drives the Roam / Logseq importers.
//! `outl_actions::backlinks::extract_refs` was a raw byte scan for
//! `[[` … `]]`, so the backlink index (and, through
//! `reminders::scan::anchor_dates`, the reminder scheduler) disagreed
//! with every one of them on three shapes: a ref inside a `` `code` ``
//! span, a ref carrying a newline, and a nested `[[a [[b]] ]]`.
//!
//! The sharpest instance was inside one block: `mentions_of` read
//! `[[ref]]` through the byte scan and `#tag` through the tokenizer, so
//! a tag in a code span was correctly ignored while a ref in the *same*
//! code span was not.
//!
//! These tests pin the fix by keeping the old rule alive as
//! [`legacy_extract_refs`] and asserting, for each shape, both what the
//! old rule did and that the shipped one now agrees with the tokenizer.
//! Delete `legacy_extract_refs` and the record of what changed goes with
//! it.
//!
//! The corpus measurement at the bottom is `#[ignore]`d — it needs a
//! real workspace:
//!
//! ```sh
//! OUTL_REF_CORPUS=/path/to/workspace \
//!   cargo test -p outl-actions --release --test ref_extraction_divergence \
//!   -- --ignored --nocapture
//! ```

use outl_actions::extract_refs;

/// The byte scan `extract_refs` used to be, kept verbatim so every
/// assertion below can state *what changed*, not merely what is true.
fn legacy_extract_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if !(bytes[i] == b'[' && bytes[i + 1] == b'[') {
            i += 1;
            continue;
        }
        let start = i + 2;
        let mut j = start;
        let mut closed = false;
        while j + 1 < bytes.len() {
            if bytes[j] == b'[' && bytes[j + 1] == b'[' {
                break;
            }
            if bytes[j] == b']' && bytes[j + 1] == b']' {
                closed = true;
                break;
            }
            j += 1;
        }
        if closed {
            if let Ok(s) = std::str::from_utf8(&bytes[start..j]) {
                if !s.is_empty() {
                    refs.push(s.to_string());
                }
            }
            i = j + 2;
        } else {
            i += 2;
        }
    }
    refs
}

/// What the renderer, `ref_at_cursor` and both importers see.
///
/// Written independently of the shipped implementation so the
/// assertions below are a real comparison and not a tautology.
/// **It descends through emphasis wrappers**, because `Bold` /
/// `Italic` / `Strike` / `Highlight` carry recursively tokenized
/// contents and every client renders the ref inside them as a link.
/// A flat `filter(PageRef)` looks correct in a unit test and drops 403
/// references on a real workspace — see the corpus module.
fn tokenizer_refs(text: &str) -> Vec<String> {
    fn walk(toks: &[outl_md::inline::InlineTok<'_>], out: &mut Vec<String>) {
        use outl_md::inline::InlineTok as T;
        for t in toks {
            match t {
                T::PageRef { name } => out.push((*name).to_string()),
                T::Bold { inner }
                | T::Italic { inner, .. }
                | T::Strike { inner }
                | T::Highlight { inner } => walk(inner, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&outl_md::inline::tokenize(text), &mut out);
    out
}

// --- divergence 1: code span ---------------------------------------------

#[test]
fn a_ref_inside_a_code_span_is_not_a_mention() {
    let text = "escape it as `[[avelino]]`";

    // What the old rule did: a code span was invisible to it.
    assert_eq!(legacy_extract_refs(text), vec!["avelino".to_string()]);
    // What every other surface does: the span is `Code`, not a ref.
    assert!(tokenizer_refs(text).is_empty());

    assert_eq!(extract_refs(text), tokenizer_refs(text));
    assert!(
        extract_refs(text).is_empty(),
        "a `[[ref]]` inside a code span must not create a backlink"
    );
}

#[test]
fn a_ref_and_a_tag_in_one_code_span_are_now_judged_by_one_rule() {
    // The inconsistency that made this a bug rather than a wart:
    // `mentions_of` asked the tokenizer about `#tag` and the byte scan
    // about `[[ref]]`, so one code span produced two different verdicts.
    let text = "both of these are literal: `[[avelino]] #avelino`";

    let tags: Vec<String> = outl_md::inline::tokenize(text)
        .into_iter()
        .filter_map(|t| match t {
            outl_md::inline::InlineTok::Tag { name } => Some(name.to_string()),
            _ => None,
        })
        .collect();

    assert!(tags.is_empty(), "the tag side was always right");
    assert_eq!(
        legacy_extract_refs(text),
        vec!["avelino".to_string()],
        "…and the ref side was always wrong, in the same span"
    );
    assert!(extract_refs(text).is_empty());
}

#[test]
fn a_ref_outside_the_code_span_still_counts() {
    // The fix must not over-swing: only the span itself is inert.
    let text = "[[avelino]] wrote `[[not-a-ref]]` today";
    assert_eq!(extract_refs(text), vec!["avelino".to_string()]);
}

#[test]
fn an_unterminated_backtick_leaves_the_ref_alone() {
    // A stray backtick opens no span, so the ref behind it is real.
    let text = "a ` stray tick then [[avelino]]";
    assert_eq!(extract_refs(text), vec!["avelino".to_string()]);
    assert_eq!(extract_refs(text), tokenizer_refs(text));
}

// --- divergence 2: newline -----------------------------------------------

#[test]
fn a_ref_spanning_a_newline_is_not_a_mention() {
    // `try_page_ref` rejects `name.contains('\n')`; the byte scan did
    // not. Block text is multi-line whenever a block has continuation
    // lines, so this is reachable from ordinary typing.
    let text = "open bracket here [[avelino\nand the close over here]] done";

    assert_eq!(
        legacy_extract_refs(text),
        vec!["avelino\nand the close over here".to_string()]
    );
    assert!(tokenizer_refs(text).is_empty());
    assert!(extract_refs(text).is_empty());
}

#[test]
fn a_later_single_line_ref_survives_an_earlier_multi_line_one() {
    let text = "[[broken\nacross lines]] but [[avelino]] is fine";
    assert_eq!(extract_refs(text), vec!["avelino".to_string()]);
    assert_eq!(extract_refs(text), tokenizer_refs(text));
}

// --- divergence 3: nesting -----------------------------------------------

#[test]
fn a_nested_ref_resolves_to_the_page_the_renderer_shows() {
    // The two rules did not merely disagree about *whether* this is a
    // ref — they named **different pages**. The renderer draws a link
    // labelled `a [[b`, and the backlink landed on page `b`.
    let text = "[[a [[b]] ]]";

    assert_eq!(legacy_extract_refs(text), vec!["b".to_string()]);
    assert_eq!(tokenizer_refs(text), vec!["a [[b".to_string()]);
    assert_eq!(extract_refs(text), vec!["a [[b".to_string()]);
}

#[test]
fn an_unterminated_opener_is_absorbed_by_the_next_close() {
    // Behaviour change, stated out loud: the byte scan restarted after
    // an unterminated `[[` and recovered the inner `[[ok]]`. The
    // tokenizer is greedy to the first `]]`, so it yields one ref
    // spanning both — which is exactly what the user sees rendered.
    let text = "[[unterminated and [[ok]] mixed";

    assert_eq!(legacy_extract_refs(text), vec!["ok".to_string()]);
    assert_eq!(
        extract_refs(text),
        vec!["unterminated and [[ok".to_string()],
        "the backlink now names the page the rendered link names"
    );
}

// --- shapes that must NOT change ----------------------------------------

#[test]
fn the_ordinary_shapes_are_untouched() {
    for text in [
        "see [[avelino]] and [[2026-05-27]] please",
        "[[avelino]] and again [[avelino]]",
        "blocked on [[@avelino]]",
        "no refs here at all",
        "no refs here at all",
        "",
        "[[]]",
        "[[",
        "]]",
    ] {
        assert_eq!(
            extract_refs(text),
            tokenizer_refs(text),
            "extract_refs must be the tokenizer, for {text:?}"
        );
    }
    assert_eq!(
        extract_refs("see [[avelino]] and [[2026-05-27]] please"),
        vec!["avelino".to_string(), "2026-05-27".to_string()]
    );
}

#[test]
fn emphasis_wrappers_stay_transparent_to_a_reference() {
    // RFC 0008 promised this explicitly ("`==important [[topic]]==`
    // still yields the `[[topic]]` backlink because the wrapper is
    // transparent"), and it was true only by accident of the byte scan.
    // Routing through the tokenizer keeps the promise on purpose:
    // `collect_page_refs` descends into every wrapper whose `inner` is
    // itself tokenized, which is the same set the clients render
    // through.
    for (text, want) in [
        ("**[[bold ref]]** stays", "bold ref"),
        ("==important [[topic]]== stays", "topic"),
        ("*[[italic]]*", "italic"),
        ("_[[under]]_", "under"),
        ("~~[[struck]]~~", "struck"),
        ("**bold with [[nested]]:** trailing", "nested"),
        ("***[[deep]]***", "deep"),
    ] {
        assert_eq!(
            extract_refs(text),
            vec![want.to_string()],
            "emphasis must stay transparent for {text:?}"
        );
        assert_eq!(legacy_extract_refs(text), extract_refs(text));
    }
}

#[test]
fn a_tag_inside_emphasis_is_now_a_mention_too() {
    // The same descent the refs needed. Before, `mentions_of` read tags
    // off the TOP-LEVEL tokens only, so `**#avelino**` produced no tag
    // key — the tokenizer had already folded it inside `Bold`.
    let tags: Vec<String> = outl_md::inline::tokenize("**#avelino**")
        .into_iter()
        .filter_map(|t| match t {
            outl_md::inline::InlineTok::Tag { name } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    assert!(
        tags.is_empty(),
        "top-level scan sees nothing — that was the bug"
    );
    // Proven end-to-end through the index in
    // `backlinks::tests::tag_inside_emphasis_is_a_mention`.
}

// --- end-to-end through the index ---------------------------------------
//
// The unit assertions above pin the rule; these pin what a user sees in
// the "Linked from" panel, which is the only surface that matters.

mod end_to_end {
    use chrono::NaiveDate;
    use outl_actions::{
        append_block, backlinks_for_page, open_journal, open_or_create_page, page_meta, PageKind,
    };
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use std::path::Path;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn root() -> &'static Path {
        Path::new("/tmp/outl-test")
    }

    #[test]
    fn ref_inside_inline_code_is_not_a_mention() {
        // The mirror of `tag_inside_inline_code_is_not_a_mention`, which
        // has passed since the tag channel was written. The ref channel
        // read a raw byte scan instead, so one code span in one block
        // produced two opposite verdicts.
        let (mut w, hlc) = ws();
        let target =
            open_or_create_page(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("escape it as `[[avelino]]`")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "code-span ref leaked in: {links:#?}");
    }

    #[test]
    fn ref_inside_emphasis_is_still_a_mention() {
        // The fix must not over-swing. `**[[avelino]]:**` is how the
        // user writes a labelled bullet, and RFC 0008 promised the
        // wrapper stays transparent. 403 references on a real workspace
        // sit inside one.
        let (mut w, hlc) = ws();
        let target =
            open_or_create_page(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let bold = append_block(&mut w, &hlc, Some(day), Some("**[[avelino]]:** shipped")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1, "emphasis must stay transparent");
        assert_eq!(links[0].block_id, bold.to_string());
    }

    #[test]
    fn tag_inside_emphasis_is_a_mention() {
        // Same descent, the tag channel. `mentions_of` used to read tags
        // off the TOP-LEVEL tokens only, so the tokenizer folding
        // `#avelino` inside `Bold` hid it. One walk now serves both.
        let (mut w, hlc) = ws();
        let target =
            open_or_create_page(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let bold = append_block(&mut w, &hlc, Some(day), Some("**#avelino** ships")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert_eq!(links.len(), 1, "a tag inside bold is still a tag");
        assert_eq!(links[0].block_id, bold.to_string());
    }

    #[test]
    fn a_ref_that_straddles_a_newline_is_not_a_mention() {
        // Block text is multi-line whenever a block has continuation
        // lines, so a `[[` on one line and a `]]` on another is reachable
        // from ordinary typing. `try_page_ref` has always rejected it;
        // the byte scan accepted it and invented a page named across two
        // lines.
        let (mut w, hlc) = ws();
        let target =
            open_or_create_page(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, target).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()).unwrap();
        let _ = append_block(&mut w, &hlc, Some(day), Some("[[avelino\nand more]] here")).unwrap();

        let links = backlinks_for_page(&w, root(), &meta);
        assert!(links.is_empty(), "multi-line ref leaked in: {links:#?}");
    }
}

// --- real-workspace measurement -----------------------------------------

mod corpus {
    use super::*;
    use std::collections::{BTreeMap, HashSet};
    use std::path::{Path, PathBuf};

    use outl_actions::{
        build_backlink_index_from_disk, read_page_outline, OutlineNode, PageKind, PageMeta,
    };

    fn corpus_root() -> Option<PathBuf> {
        std::env::var_os("OUTL_REF_CORPUS").map(PathBuf::from)
    }

    /// Page list straight off disk — no op log, no `Workspace`. Title
    /// comes from the `title::` header when present (that is what
    /// `page_meta` resolves), else the slug.
    fn metas_from_disk(root: &Path) -> Vec<PageMeta> {
        let mut out = Vec::new();
        for (dir, kind) in [("pages", PageKind::Page), ("journals", PageKind::Journal)] {
            let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let body = std::fs::read_to_string(&path).unwrap_or_default();
                let title = body
                    .lines()
                    .take_while(|l| !l.trim_start().starts_with("- "))
                    .find_map(|l| l.trim().strip_prefix("title:: "))
                    .map(|t| t.trim().to_string())
                    .unwrap_or_else(|| slug.to_string());
                out.push(PageMeta {
                    id: String::new(),
                    slug: slug.to_string(),
                    title,
                    kind,
                    icon: None,
                    pinned: false,
                    page_type: None,
                });
            }
        }
        out.sort_by(|a, b| a.slug.cmp(&b.slug));
        out
    }

    fn walk<'a>(nodes: &'a [OutlineNode], out: &mut Vec<&'a OutlineNode>) {
        for n in nodes {
            out.push(n);
            walk(&n.children, out);
        }
    }

    /// Multiset difference, `a` minus `b`.
    fn minus(a: &[String], b: &[String]) -> Vec<String> {
        let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
        for s in b {
            *counts.entry(s.as_str()).or_default() += 1;
        }
        let mut out = Vec::new();
        for s in a {
            let c = counts.entry(s.as_str()).or_default();
            if *c > 0 {
                *c -= 1;
            } else {
                out.push(s.clone());
            }
        }
        out
    }

    #[test]
    #[ignore = "needs OUTL_REF_CORPUS pointing at a real workspace"]
    fn measure_the_backlink_delta_on_a_real_workspace() {
        let Some(root) = corpus_root() else {
            eprintln!("OUTL_REF_CORPUS unset — skipping");
            return;
        };
        let metas = metas_from_disk(&root);
        // A `Ref(x)` key only becomes a visible backlink when some page
        // answers to `x` (slug, title, or the `@`-alias form).
        let mut resolvable: HashSet<String> = HashSet::new();
        for m in &metas {
            resolvable.insert(m.slug.clone());
            resolvable.insert(m.title.clone());
            resolvable.insert(format!("@{}", m.slug));
            resolvable.insert(format!("@{}", m.title));
        }

        let (mut blocks, mut old_total, mut new_total) = (0usize, 0usize, 0usize);
        let (mut lost, mut gained) = (Vec::new(), Vec::new());
        let mut pages_touched: HashSet<String> = HashSet::new();
        // How far the SHIPPED function is from the tokenizer right now.
        // Before the fix this equals the delta below; after it, zero.
        let mut shipped_disagrees = 0usize;

        for meta in &metas {
            let Ok(outline) = read_page_outline(&root, meta) else {
                continue;
            };
            let mut nodes = Vec::new();
            walk(&outline.nodes, &mut nodes);
            for n in nodes {
                blocks += 1;
                let old = legacy_extract_refs(&n.text);
                let new = tokenizer_refs(&n.text);
                if extract_refs(&n.text) != new {
                    shipped_disagrees += 1;
                }
                old_total += old.len();
                new_total += new.len();
                if old == new {
                    continue;
                }
                pages_touched.insert(meta.slug.clone());
                for r in minus(&old, &new) {
                    lost.push((meta.slug.clone(), r, n.text.clone()));
                }
                for r in minus(&new, &old) {
                    gained.push((meta.slug.clone(), r, n.text.clone()));
                }
            }
        }

        let resolving = |v: &Vec<(String, String, String)>| {
            v.iter().filter(|(_, r, _)| resolvable.contains(r)).count()
        };

        // --- the number that actually matters -------------------------
        // A lost *ref* is only a lost *backlink* when no other mention
        // in the same block still points at the same page. Build the
        // (target page, source block) pair set under both rules and
        // diff those.
        let by_key: HashSet<String> = resolvable.iter().cloned().collect();
        let mut old_pairs: HashSet<(String, String, usize)> = HashSet::new();
        let mut new_pairs: HashSet<(String, String, usize)> = HashSet::new();
        for meta in &metas {
            let Ok(outline) = read_page_outline(&root, meta) else {
                continue;
            };
            let mut nodes = Vec::new();
            walk(&outline.nodes, &mut nodes);
            for (i, n) in nodes.iter().enumerate() {
                for r in legacy_extract_refs(&n.text) {
                    if by_key.contains(&r) {
                        old_pairs.insert((r, meta.slug.clone(), i));
                    }
                }
                for r in extract_refs(&n.text) {
                    if by_key.contains(&r) {
                        new_pairs.insert((r, meta.slug.clone(), i));
                    }
                }
            }
        }
        let lost_bl: Vec<_> = old_pairs.difference(&new_pairs).cloned().collect();
        let gained_bl: Vec<_> = new_pairs.difference(&old_pairs).cloned().collect();

        println!("\n=== [[ref]] extraction delta — {} ===", root.display());
        println!("pages: {}  blocks: {blocks}", metas.len());
        println!("refs extracted: old {old_total} -> new {new_total}");
        println!("pages with any change: {}", pages_touched.len());
        println!("blocks where the shipped extract_refs != tokenizer: {shipped_disagrees}");
        println!(
            "LOST  refs: {} (of which resolve to a real page: {})",
            lost.len(),
            resolving(&lost)
        );
        println!(
            "GAINED refs: {} (of which resolve to a real page: {})",
            gained.len(),
            resolving(&gained)
        );
        let show = |label: &str, v: &Vec<(String, String, String)>| {
            println!("--- {label} (up to 60, real-page ones first) ---");
            let mut sorted: Vec<_> = v.iter().collect();
            sorted.sort_by_key(|(_, r, _)| !resolvable.contains(r));
            for (slug, r, text) in sorted.into_iter().take(60) {
                let t: String = text.chars().take(120).collect();
                println!(
                    "  [{}] {:?}{}  <- {:?}",
                    slug,
                    r,
                    if resolvable.contains(r) {
                        " (REAL PAGE)"
                    } else {
                        ""
                    },
                    t.replace('\n', "\\n")
                );
            }
        };
        show("LOST", &lost);
        show("GAINED", &gained);

        println!("--- backlink-level delta ((target page, source block) pairs) ---");
        println!(
            "backlinks: old {} -> new {}",
            old_pairs.len(),
            new_pairs.len()
        );
        println!("LOST backlinks: {}", lost_bl.len());
        for (target, page, _) in lost_bl.iter().take(25) {
            println!("  [[{target}]] cited from {page}");
        }
        println!("GAINED backlinks: {}", gained_bl.len());
        for (target, page, _) in gained_bl.iter().take(25) {
            println!("  [[{target}]] cited from {page}");
        }

        // --- cost ------------------------------------------------------
        // `tokenize` is structural and a byte scan is not, so the swap
        // has to be priced. Isolate the part that changed by running
        // both rules over every block text, then show the whole-index
        // build it sits inside.
        let texts: Vec<String> = metas
            .iter()
            .filter_map(|m| read_page_outline(&root, m).ok())
            .flat_map(|o| {
                let mut nodes = Vec::new();
                walk(&o.nodes, &mut nodes);
                nodes
                    .into_iter()
                    .map(|n| n.text.clone())
                    .collect::<Vec<_>>()
            })
            .collect();

        let median = |mut v: Vec<std::time::Duration>| {
            v.sort();
            v[v.len() / 2].as_secs_f64() * 1000.0
        };
        let (mut old_t, mut new_t) = (Vec::new(), Vec::new());
        let mut sink = 0usize;
        for _ in 0..7 {
            // What `mentions_of` used to do: byte scan for refs, plus a
            // second pass through the tokenizer for tags behind a
            // `contains('#')` probe.
            let t = std::time::Instant::now();
            for x in &texts {
                sink += legacy_extract_refs(x).len();
                if x.contains('#') {
                    sink += outl_md::inline::tokenize(x).len();
                }
            }
            old_t.push(t.elapsed());

            // What it does now: one tokenization, one walk, both
            // channels out of it.
            let t = std::time::Instant::now();
            for x in &texts {
                sink += extract_refs(x).len();
            }
            new_t.push(t.elapsed());
        }
        assert!(sink > 0);
        println!(
            "--- cost over {} block texts (median of 7) ---",
            texts.len()
        );
        println!(
            "old rule (byte scan + conditional tokenize): {:.1} ms",
            median(old_t)
        );
        println!(
            "new rule (one tokenize + one walk):          {:.1} ms",
            median(new_t)
        );

        // Whole-workspace index build — what a client pays on boot.
        let mut samples = Vec::new();
        let mut refs_indexed = 0;
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let idx = build_backlink_index_from_disk(&metas, &root);
            refs_indexed = idx.len();
            samples.push(t.elapsed());
        }
        println!(
            "index build (median of 7): {:.0} ms, {refs_indexed} referencing blocks",
            median(samples)
        );
    }
}
