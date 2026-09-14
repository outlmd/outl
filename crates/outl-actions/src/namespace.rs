//! Namespaced page names — `os/linux/debian`, `programming-language/rust`.
//!
//! **The hierarchy is derived from the page `title`, never from the
//! slug.** A page named `os/linux` keeps that name verbatim in
//! `title::` and slugifies to the single path component `os-linux`
//! (`outl_md::slug::slugify` folds `/` to `-`), because the slug is
//! joined into `pages/<slug>.md` and
//! [`crate::page::is_valid_slug`] rejects `/` for exactly that
//! reason. So the `/` a user typed survives in one place only — the
//! title — and that is where every namespace question is answered.
//!
//! That makes this module a **projection**: no new op, no new
//! on-disk field, no migration. `#os/linux/debian` already tokenizes
//! as one tag with a `/` in its name
//! (`outl_md::reference::try_tag`), already resolves through
//! [`crate::resolve::open_or_create_by_name`], and already keeps its
//! namespaced title (pinned by the importer's
//! `namespaced_page_keeps_title_and_flattens_slug`). What was missing
//! was only the *reading* side: which pages sit under `os`.
//!
//! ## What this depends on, measured
//!
//! The listing side is only as good as `title::`. A page ingested from
//! disk without one falls back to its slug via
//! [`crate::page::page_meta`], and a slug carries no `/` — so it reads
//! as one flat segment and nests under nothing.
//!
//! On a real 2,575-page workspace that was **14 pages with `title::`**.
//! The 65 `buser-*` pages that *are* the namespaced ones had none, so
//! `descendants` returned empty for every namespace while the
//! backlinks channel — which reads the *mention*, not the title —
//! collected 3,221 blocks under `buser` alone.
//!
//! Both halves are behaving as designed; the gap is upstream, in pages
//! that never got a `title::`. Deriving the hierarchy from the slug
//! instead would need `-` to mean "nest", which makes `meu-projeto` a
//! child of `meu`. The fix is to populate `title::`
//! ([`crate::page_repair_titles`] is the precedent), not to loosen
//! this comparison.
//!
//! ## Comparison is per-segment and slugified
//!
//! `OS/Linux` and `os/linux` are the same namespace, because they
//! resolve to the same page. So a name is compared as its vector of
//! **slugified segments**, never as a raw string prefix — a raw
//! prefix would also match `oscar/wilde` for the namespace `os`,
//! which is the bug `crate::backlinks_keys` pays a whole
//! `TargetKey` channel to avoid.

use outl_md::slug::slugify;
use serde::Serialize;

use crate::page::{PageKind, PageMeta};

/// Split a namespaced name into its segments, dropping empty ones.
///
/// `//os//linux/` → `["os", "linux"]`. Empty segments are dropped
/// rather than rejected because a user typing `#os//linux` means the
/// obvious thing, and the tokenizer happily accepts it.
pub fn segments(name: &str) -> Vec<&str> {
    name.split('/').filter(|s| !s.trim().is_empty()).collect()
}

/// The canonical comparison key for a namespaced name: one slugified
/// segment per level.
///
/// This is what makes `OS/Linux` and `os/linux` the same namespace —
/// the same fold that makes them resolve to the same page.
pub fn key(name: &str) -> Vec<String> {
    segments(name).into_iter().map(slugify).collect()
}

/// Every **proper** ancestor of `name`, outermost first, in the
/// caller's original spelling.
///
/// `os/linux/debian` → `["os", "os/linux"]`. A name with no `/` has
/// no ancestors. The name itself is never included: a page is not
/// nested under itself, and emitting it would make every namespaced
/// page its own backlink.
pub fn ancestors(name: &str) -> Vec<String> {
    let segs = segments(name);
    (1..segs.len()).map(|n| segs[..n].join("/")).collect()
}

/// One page nested under a namespace, ready for a client to render
/// as a tree row without re-deriving the hierarchy itself.
///
/// `depth` and `label` are computed **here** rather than in each
/// client, because three clients splitting a title on `/` is three
/// chances to disagree about what `os//linux` or `OS/Linux` means.
#[derive(Debug, Clone, Serialize)]
pub struct NamespaceChild {
    /// The nested page.
    pub page: PageMeta,
    /// Levels below the namespace root. A direct child is `1`.
    pub depth: usize,
    /// The trailing segment — what a client shows as the row label
    /// (`debian`, not `os/linux/debian`).
    pub label: String,
}

/// Every page nested under `name`, sorted by title, each carrying its
/// depth below the namespace root.
///
/// The namespace root itself is excluded (`depth >= 1` always), so
/// passing a page its own title yields its descendants and nothing
/// else.
///
/// **Journals are never nested, and that is enforced here rather than
/// left to chance.** An ISO date carries no `/`, so one could argue
/// the filter is unreachable — but "unreachable" was not good enough:
/// three surfaces had already written down three different answers
/// (a `📅` branch in the shared component, a hardcoded `"page"` kind
/// on mobile, no journal branch at all in the TUI). One `match` here
/// makes the other two provably right instead of accidentally right.
pub fn descendants(pages: &[PageMeta], name: &str) -> Vec<NamespaceChild> {
    let root = key(name);
    if root.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<NamespaceChild> = pages
        .iter()
        .filter_map(|page| {
            if page.kind == PageKind::Journal {
                return None;
            }
            // One split per page: the label is the last raw segment and
            // the comparison key is the same list, slugified. Asking
            // `key()` separately would re-split every title on a path
            // that runs per page, per backlinks fetch.
            let segs = segments(&page.title);
            let label = (*segs.last()?).to_string();
            let page_key: Vec<String> = segs.into_iter().map(slugify).collect();
            if page_key.len() <= root.len() || !page_key.starts_with(&root[..]) {
                return None;
            }
            Some(NamespaceChild {
                page: page.clone(),
                depth: page_key.len() - root.len(),
                label,
            })
        })
        .collect();
    out.sort_by(|a, b| a.page.title.cmp(&b.page.title));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str) -> PageMeta {
        PageMeta {
            id: format!("id-{title}"),
            slug: slugify(title),
            title: title.to_string(),
            kind: PageKind::Page,
            icon: None,
            pinned: false,
            page_type: None,
        }
    }

    #[test]
    fn segments_split_on_slash() {
        assert_eq!(segments("os/linux/debian"), vec!["os", "linux", "debian"]);
        assert_eq!(segments("os"), vec!["os"]);
    }

    #[test]
    fn empty_segments_are_dropped() {
        assert_eq!(segments("//os//linux/"), vec!["os", "linux"]);
        assert!(segments("///").is_empty());
    }

    #[test]
    fn ancestors_are_proper_prefixes_outermost_first() {
        assert_eq!(ancestors("os/linux/debian"), vec!["os", "os/linux"]);
    }

    #[test]
    fn a_flat_name_has_no_ancestors() {
        assert!(ancestors("os").is_empty());
    }

    #[test]
    fn a_name_is_never_its_own_ancestor() {
        // Emitting the name itself would make every namespaced page a
        // backlink of itself the moment `mentions_of` starts indexing
        // ancestors.
        assert_eq!(ancestors("os/linux"), vec!["os".to_string()]);
    }

    #[test]
    fn nesting_has_no_depth_limit() {
        let deep = "a/b/c/d/e/f/g/h";
        let ancs = ancestors(deep);
        assert_eq!(ancs.len(), 7);
        assert_eq!(ancs.last().unwrap(), "a/b/c/d/e/f/g");
    }

    #[test]
    fn descendants_include_every_level_with_its_depth() {
        let pages = vec![
            page("os"),
            page("os/linux"),
            page("os/linux/debian"),
            page("os/freebsd"),
        ];
        let found = descendants(&pages, "os");
        let rows: Vec<(&str, usize, &str)> = found
            .iter()
            .map(|c| (c.page.title.as_str(), c.depth, c.label.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("os/freebsd", 1, "freebsd"),
                ("os/linux", 1, "linux"),
                ("os/linux/debian", 2, "debian"),
            ]
        );
    }

    #[test]
    fn the_namespace_root_is_not_its_own_descendant() {
        let pages = vec![page("os"), page("os/linux")];
        let found = descendants(&pages, "os");
        assert!(found.iter().all(|c| c.page.title != "os"));
    }

    #[test]
    fn a_sibling_sharing_a_string_prefix_is_not_nested() {
        // The reason comparison is per-segment: `oscar` starts with
        // `os` as a string, and is not in the `os` namespace.
        let pages = vec![page("oscar"), page("oscar/wilde"), page("os/linux")];
        let found = descendants(&pages, "os");
        let titles: Vec<&str> = found.iter().map(|c| c.page.title.as_str()).collect();
        assert_eq!(titles, vec!["os/linux"]);
    }

    #[test]
    fn matching_folds_case_and_accents_like_the_slug_does() {
        // `OS/Linux` and `os/linux` resolve to the same page, so they
        // must land in the same namespace.
        let pages = vec![page("OS/Linux"), page("Sistemas/Ação")];
        assert_eq!(descendants(&pages, "os").len(), 1);
        assert_eq!(descendants(&pages, "sistemas").len(), 1);
        assert_eq!(descendants(&pages, "Sistemas").len(), 1);
    }

    #[test]
    fn an_empty_namespace_matches_nothing() {
        let pages = vec![page("os/linux")];
        assert!(descendants(&pages, "").is_empty());
        assert!(descendants(&pages, "///").is_empty());
    }

    #[test]
    fn descendants_of_an_intermediate_level_are_relative_to_it() {
        let pages = vec![
            page("os"),
            page("os/linux"),
            page("os/linux/debian"),
            page("os/linux/debian/sid"),
        ];
        let found = descendants(&pages, "os/linux");
        let rows: Vec<(&str, usize)> = found
            .iter()
            .map(|c| (c.page.title.as_str(), c.depth))
            .collect();
        assert_eq!(
            rows,
            vec![("os/linux/debian", 1), ("os/linux/debian/sid", 2)]
        );
    }

    #[test]
    fn a_journal_is_never_nested() {
        // A journal's title is an ISO date, so in practice it carries
        // no `/`. The filter exists so the clients' three different
        // journal answers become provably right rather than lucky.
        let mut journal = page("2026-09-13");
        journal.kind = PageKind::Journal;
        journal.title = "os/2026-09-13".to_string();
        let pages = vec![page("os"), journal, page("os/linux")];
        let found = descendants(&pages, "os");
        let titles: Vec<&str> = found.iter().map(|c| c.page.title.as_str()).collect();
        assert_eq!(titles, vec!["os/linux"]);
    }
}
