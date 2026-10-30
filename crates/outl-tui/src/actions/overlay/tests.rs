//! Tests for the overlay actions.
//!
//! Split out of `overlay.rs` to keep that file under the size ratchet
//! (`scripts/check-file-size.sh`); the layout follows
//! `view/outline.rs` + `view/outline/tests.rs`.

#[cfg(test)]
mod picker_tests {
    use crate::state::{App, Overlay, QuickSwitchState, SwitchCandidate, SwitchKind};
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use tempfile::TempDir;

    fn page(label: &str) -> SwitchCandidate {
        SwitchCandidate {
            label: label.to_string(),
            key: label.to_string(),
            kind: SwitchKind::Page,
            score: 0,
        }
    }

    fn test_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    /// Regression: deleting query characters must restore candidates an
    /// earlier, longer query had filtered out. The bug was that
    /// `refresh_quick_switch` narrowed `candidates` in place, destroying
    /// the source set, so backspacing could never bring matches back.
    #[test]
    fn quick_switch_widens_when_query_shrinks() {
        let (mut app, _dir) = test_app();
        let all = vec![page("foo"), page("food"), page("bar")];
        app.overlay = Some(Overlay::QuickSwitch(QuickSwitchState {
            query: String::new(),
            all_candidates: all.clone(),
            candidates: all,
            selected: 0,
            preview_cache: std::cell::RefCell::new(None),
        }));

        // Type "food": only the "food" page survives.
        if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
            qs.query = "food".into();
        }
        app.refresh_quick_switch();
        let after_food: Vec<String> = match &app.overlay {
            Some(Overlay::QuickSwitch(qs)) => {
                qs.candidates.iter().map(|c| c.label.clone()).collect()
            }
            _ => unreachable!(),
        };
        assert_eq!(after_food, vec!["food".to_string()]);

        // Backspace down to "foo": both "foo" and "food" must come back.
        if let Some(Overlay::QuickSwitch(ref mut qs)) = app.overlay {
            qs.query = "foo".into();
        }
        app.refresh_quick_switch();
        let mut after_foo: Vec<String> = match &app.overlay {
            Some(Overlay::QuickSwitch(qs)) => {
                qs.candidates.iter().map(|c| c.label.clone()).collect()
            }
            _ => unreachable!(),
        };
        after_foo.sort();
        assert_eq!(
            after_foo,
            vec!["foo".to_string(), "food".to_string()],
            "shrinking the query must restore candidates the longer query dropped"
        );
    }

    // `detect_trigger_mention_*` tests live in
    // `actions::autocomplete::tests` (same module as the function).

    // ---------------------------------------------------------------
    // Slash palette navigation — cross-category regression
    // ---------------------------------------------------------------

    /// Helper: open the slash palette, set the query, refresh, and
    /// return the list of command names in visual (paint) order.
    fn slash_visual_names(app: &mut App, query: &str) -> Vec<String> {
        app.open_slash();
        if !query.is_empty() {
            if let Some(Overlay::Slash(ref mut s)) = app.overlay {
                s.query = query.to_string();
            }
            app.refresh_slash();
        }
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        order
            .into_iter()
            .map(|i| candidates[i].name.clone())
            .collect()
    }

    /// `slash_select_next` must walk items in the same order that the
    /// renderer paints them (category-first), not in raw `s.candidates`
    /// order. Repro: query `t` matches commands from at least three
    /// categories (Actions, Settings, Dates & time); without the fix
    /// the highlighted row jumped instead of moving to the next
    /// visible item.
    #[test]
    fn slash_nav_follows_visual_order_under_filter() {
        let (mut app, _dir) = test_app();
        let visual = slash_visual_names(&mut app, "t");
        // Must have matched something across categories — sanity check.
        assert!(
            visual.len() >= 2,
            "expected multiple matches for 't', got {visual:?}"
        );

        // Re-open with the same query, then walk with slash_select_next
        // and verify each step lands on the next item in visual order.
        app.open_slash();
        if let Some(Overlay::Slash(ref mut s)) = app.overlay {
            s.query = "t".to_string();
        }
        app.refresh_slash();

        // The initial selection must be the first item in visual order.
        let initial_selected = match &app.overlay {
            Some(Overlay::Slash(s)) => s.selected,
            _ => unreachable!(),
        };
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        assert_eq!(
            initial_selected, order[0],
            "initial selection must be first in visual order"
        );

        // Walk forward through the whole list and confirm every step
        // matches the next position in `visual_order`, never jumping.
        let mut walked: Vec<String> = vec![candidates[initial_selected].name.clone()];
        for _ in 1..order.len() {
            app.slash_select_next();
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            let cands = match &app.overlay {
                Some(Overlay::Slash(s)) => s.candidates.clone(),
                _ => unreachable!(),
            };
            let cur_order = crate::view::overlays::visual_order(&cands);
            let pos = cur_order
                .iter()
                .position(|&i| i == sel)
                .expect("selected must be in visual_order");
            assert_eq!(
                walked.len(),
                pos,
                "after {} next-presses, selection should be at visual position {}, got {pos}",
                walked.len(),
                walked.len()
            );
            walked.push(cands[sel].name.clone());
        }
        // Walked names must equal the visual order computed upfront.
        assert_eq!(walked, visual, "walked sequence must match visual order");
    }

    /// `slash_select_prev` mirrors `slash_select_next` and must never
    /// jump past the first item in visual order.
    #[test]
    fn slash_nav_prev_clamps_at_first_visual_item() {
        let (mut app, _dir) = test_app();
        app.open_slash();
        // No filter — full list.
        let first_visual_idx = {
            let candidates = match &app.overlay {
                Some(Overlay::Slash(s)) => s.candidates.clone(),
                _ => unreachable!(),
            };
            crate::view::overlays::visual_order(&candidates)[0]
        };
        // Start is already at first; pressing prev must stay there.
        app.slash_select_prev();
        let sel = match &app.overlay {
            Some(Overlay::Slash(s)) => s.selected,
            _ => unreachable!(),
        };
        assert_eq!(
            sel, first_visual_idx,
            "prev at the first item must not move"
        );
    }

    /// Query-empty palette: `open_slash` + `slash_select_next` walking
    /// the whole list must cover every registered command exactly once,
    /// in canonical category order (Actions → Navigation → Search →
    /// Settings → Dates & time).
    #[test]
    fn slash_nav_empty_query_covers_all_commands_in_category_order() {
        let (mut app, _dir) = test_app();
        app.open_slash();
        let candidates = match &app.overlay {
            Some(Overlay::Slash(s)) => s.candidates.clone(),
            _ => unreachable!(),
        };
        let order = crate::view::overlays::visual_order(&candidates);
        // Walk every item.
        let mut walked_names: Vec<String> = {
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            vec![candidates[sel].name.clone()]
        };
        for _ in 1..order.len() {
            app.slash_select_next();
            let sel = match &app.overlay {
                Some(Overlay::Slash(s)) => s.selected,
                _ => unreachable!(),
            };
            walked_names.push(candidates[sel].name.clone());
        }
        // Deduplicate to check all commands were visited.
        let mut sorted_walked = walked_names.clone();
        sorted_walked.sort();
        sorted_walked.dedup();
        let mut sorted_expected: Vec<String> = candidates.iter().map(|c| c.name.clone()).collect();
        sorted_expected.sort();
        assert_eq!(
            sorted_walked, sorted_expected,
            "walking the full list must visit every command exactly once"
        );
        // Category order must be non-decreasing.
        let cat_orders: Vec<u8> = walked_names
            .iter()
            .map(|n| crate::view::overlays::category_order_for(n))
            .collect();
        let is_non_decreasing = cat_orders.windows(2).all(|w| w[0] <= w[1]);
        assert!(
            is_non_decreasing,
            "category order must be non-decreasing while walking; got {cat_orders:?}"
        );
    }
}

#[cfg(test)]
mod search_source_tests {
    //! Workspace search reads the block index, not the filesystem.
    //!
    //! `refresh_search` used to walk `pages/` and `journals/`, read
    //! every `.md` and re-parse it, **on every keystroke**
    //! ([#449](https://github.com/outlmd/outl/issues/449)): thousands
    //! of reads and full parses per character on a real graph, on the
    //! event-loop thread.
    //!
    //! The source is now `App::index`, which a background worker
    //! builds once over the whole workspace. Deleting the `.md` files
    //! after indexing is how these tests tell the two apart: a
    //! filesystem-backed search finds nothing, an index-backed one
    //! still answers.

    use crate::state::{App, Overlay};
    use outl_core::hlc::HlcGenerator;
    use outl_core::{ActorId, Workspace};
    use outl_md::index::WorkspaceIndex;
    use outl_md::reconcile::reconcile_md;
    use std::fs;
    use tempfile::TempDir;

    /// Two pages, indexed, with the index built the way the running
    /// app builds it.
    ///
    /// Pages live flat under `pages/`: `slugify` folds `/` away, so
    /// `[[os/linux]]` is `pages/os-linux.md`, not a subdirectory. The
    /// namespace is a slug convention, not a filesystem one.
    fn app_with_an_index() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let pages = dir.path().join("pages");
        fs::create_dir_all(&pages).unwrap();
        let flat = pages.join("flat.md");
        let namespaced = pages.join("os-linux.md");
        fs::write(&flat, "- findme at the top level\n").unwrap();
        fs::write(&namespaced, "- findme under a namespaced slug\n").unwrap();

        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        reconcile_md(&mut ws, &hlc, &flat, None).unwrap();
        reconcile_md(&mut ws, &hlc, &namespaced, None).unwrap();

        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.index = WorkspaceIndex::build(dir.path());
        (app, dir)
    }

    fn search_for(app: &mut App, query: &str) -> Vec<String> {
        app.open_search();
        if let Some(Overlay::Search(ref mut s)) = app.overlay {
            s.query = query.to_string();
        }
        app.refresh_search();
        match &app.overlay {
            Some(Overlay::Search(s)) => s.hits.iter().map(|h| h.snippet.clone()).collect(),
            other => panic!("expected the search overlay, got {other:?}"),
        }
    }

    #[test]
    fn the_search_reads_the_index_not_the_filesystem() {
        let (mut app, dir) = app_with_an_index();
        // Every `.md` is gone. A walk-and-parse search returns
        // nothing from here; the index still holds the blocks.
        fs::remove_dir_all(dir.path().join("pages")).unwrap();

        let snippets = search_for(&mut app, "findme");

        assert_eq!(
            snippets.len(),
            2,
            "the search went back to the filesystem; got {snippets:?}"
        );
    }

    #[test]
    fn every_indexed_page_is_searched() {
        let (mut app, _d) = app_with_an_index();
        let snippets = search_for(&mut app, "findme");
        assert!(
            snippets.iter().any(|s| s.contains("top level")),
            "missing the flat page; got {snippets:?}"
        );
        assert!(
            snippets.iter().any(|s| s.contains("namespaced slug")),
            "missing the namespaced page; got {snippets:?}"
        );
    }

    #[test]
    fn an_empty_query_returns_nothing() {
        let (mut app, _d) = app_with_an_index();
        assert!(search_for(&mut app, "   ").is_empty());
    }

    #[test]
    fn fuzzy_matching_is_preserved() {
        // `BlockIndex::search_text` matches by substring and would
        // miss this. Both this box and the `((` autocomplete
        // fuzzy-match, so reusing it would have traded match quality
        // for the speed.
        let (mut app, _d) = app_with_an_index();
        let snippets = search_for(&mut app, "fnme");
        assert!(
            !snippets.is_empty(),
            "fuzzy matching was lost: 'fnme' should still reach 'findme'"
        );
    }

    #[test]
    fn the_hit_order_is_stable_across_rebuilds() {
        // `iter_blocks` is explicitly unordered, so the sort has to
        // settle ties itself or the list reshuffles between rebuilds.
        let (mut app, dir) = app_with_an_index();
        let first = search_for(&mut app, "findme");
        app.index = WorkspaceIndex::build(dir.path());
        let second = search_for(&mut app, "findme");
        assert_eq!(first, second, "hit order changed after a rebuild");
    }
}
