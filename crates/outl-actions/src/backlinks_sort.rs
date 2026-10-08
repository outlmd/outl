//! Chronological ordering for a backlinks list.
//!
//! Split out of [`crate::backlinks`] (which owns *what counts* as a
//! backlink) because *ordering* the resulting list is a distinct
//! concern — pure over `&mut [Backlink]`, no workspace, no matcher.

use std::collections::HashMap;

use crate::backlinks::Backlink;

/// Order backlinks by how recently each **source page** was referenced.
///
/// `block_id` is a ULID; its high bits are a millisecond timestamp, so
/// its lexicographic order tracks creation time — no separate timestamp
/// field is needed on [`Backlink`]. (ULIDs minted in the same
/// millisecond carry a random tail, so sub-millisecond order is not
/// meaningful; that's why the *within-page* order below is left as-is
/// rather than re-sorted per block.)
///
/// The sort is **group-stable**:
///
/// - Backlinks are ordered by their source page's **most recent**
///   referencing block, so the pages line up by recency.
/// - Within a page the blocks keep their incoming (DFS / document)
///   order — a page reads top-to-bottom regardless of the direction.
/// - Every page's blocks stay contiguous, so both renderers (the TUI
///   groups by consecutive run, the GUI clients by a `Map` keyed on
///   slug) emit one header per page and agree on page order.
///
/// `newest_first` (the product default) puts the most recently
/// referenced page at the top; `false` flips to oldest-first.
/// `slice::sort_by` is stable, so the within-page order is preserved.
pub fn sort_backlinks(links: &mut [Backlink], newest_first: bool) {
    // Group key: the source page slug, or "" for orphan blocks (no
    // enclosing page) so they cluster together.
    fn group_key(link: &Backlink) -> &str {
        link.source_page
            .as_ref()
            .map(|p| p.slug.as_str())
            .unwrap_or("")
    }

    // Sort key per group = its newest block id. Keying the sort on this
    // (and nothing finer) makes a whole page travel together and orders
    // the pages by recency, while equal keys inside a page let the
    // stable sort preserve DFS order. Owned keys so the sort below can
    // borrow `links` mutably.
    let mut group_rep: HashMap<String, String> = HashMap::new();
    for link in links.iter() {
        group_rep
            .entry(group_key(link).to_string())
            .and_modify(|rep| {
                if link.block_id > *rep {
                    rep.clone_from(&link.block_id);
                }
            })
            .or_insert_with(|| link.block_id.clone());
    }

    links.sort_by(|a, b| {
        let ord = group_rep[group_key(a)].cmp(&group_rep[group_key(b)]);
        if newest_first {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::OutlineNode;
    use crate::page::{PageKind, PageMeta};

    /// Minimal [`Backlink`] with a controlled `block_id` + source page,
    /// so the sort is exercised deterministically (no wall-clock ULIDs).
    /// `block_id` doubles as the block text so assertions read clearly.
    fn bl(block_id: &str, slug: &str) -> Backlink {
        Backlink {
            block_id: block_id.to_string(),
            block_text: block_id.to_string(),
            todo: None,
            source_page: Some(PageMeta {
                id: format!("page-{slug}"),
                slug: slug.to_string(),
                title: slug.to_string(),
                kind: PageKind::Journal,
                icon: None,
                pinned: false,
                page_type: None,
            }),
            source_block: OutlineNode {
                id: block_id.to_string(),
                text: block_id.to_string(),
                todo: None,
                collapsed: false,
                properties: Vec::new(),
                tokens: Vec::new(),
                table: None,
                children: Vec::new(),
            },
            source_block_path: Vec::new(),
            ancestors: Vec::new(),
            source_path: None,
        }
    }

    /// `(block_id, slug)` of each backlink in list order.
    fn shape(links: &[Backlink]) -> Vec<(&str, &str)> {
        links
            .iter()
            .map(|l| {
                (
                    l.block_id.as_str(),
                    l.source_page
                        .as_ref()
                        .map(|p| p.slug.as_str())
                        .unwrap_or(""),
                )
            })
            .collect()
    }

    /// Page `old` (blocks 01, 02) then page `new` (blocks 03, 04) in DFS
    /// order — `new` holds the freshest reference (04).
    fn sample() -> Vec<Backlink> {
        vec![
            bl("01", "old"),
            bl("02", "old"),
            bl("03", "new"),
            bl("04", "new"),
        ]
    }

    #[test]
    fn newest_first_puts_newest_page_on_top_dfs_within() {
        let mut links = sample();
        sort_backlinks(&mut links, true);
        // `new` (freshest block 04) leads; within each page the blocks
        // keep DFS order (03 before 04, 01 before 02).
        assert_eq!(
            shape(&links),
            vec![("03", "new"), ("04", "new"), ("01", "old"), ("02", "old")]
        );
    }

    #[test]
    fn oldest_first_is_the_group_reverse_dfs_within() {
        let mut links = sample();
        sort_backlinks(&mut links, false);
        // Pages flip (old first), but blocks inside a page stay DFS.
        assert_eq!(
            shape(&links),
            vec![("01", "old"), ("02", "old"), ("03", "new"), ("04", "new")]
        );
    }

    #[test]
    fn interleaved_sources_become_contiguous_runs() {
        // Input interleaves two pages; the sort must group each page
        // into a single run so both renderers emit one header per page.
        let mut links = vec![bl("01", "a"), bl("02", "b"), bl("03", "a"), bl("04", "b")];
        sort_backlinks(&mut links, true);
        // b's newest (04) beats a's newest (03) → b's run first; DFS
        // order within each run (02 before 04, 01 before 03).
        assert_eq!(
            shape(&links),
            vec![("02", "b"), ("04", "b"), ("01", "a"), ("03", "a")]
        );
    }

    #[test]
    fn orphan_blocks_cluster_under_the_empty_group() {
        let mut links = vec![bl("05", "page"), {
            let mut o = bl("02", "");
            o.source_page = None;
            o
        }];
        sort_backlinks(&mut links, true);
        // "page" (05) is newer than the orphan (02) → leads.
        assert_eq!(links[0].block_id, "05");
        assert_eq!(links[1].block_id, "02");
        assert!(links[1].source_page.is_none());
    }

    #[test]
    fn empty_list_is_a_noop() {
        let mut links: Vec<Backlink> = Vec::new();
        sort_backlinks(&mut links, true);
        assert!(links.is_empty());
    }
}
