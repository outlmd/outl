//! An `alias::` that names ANOTHER live page must not pull that page's
//! backlinks onto the alias owner.
//!
//! Navigation already sends `[[Beta]]` to the page titled `Beta` even
//! when some other page claims `alias:: Beta` ("titles win over
//! aliases", `resolve::alias_does_not_steal_a_ref_from_a_page_titled_the_same`).
//! The backlink index used to key the alias owner under the alias
//! unconditionally, so the two disagreed: clicking `[[Beta]]` opened
//! beta while alpha's panel listed the mention.
//!
//! Every scenario runs against both index builders (workspace and
//! from-disk) and asserts they agree, so neither path is left with the
//! old behaviour.

use std::path::Path;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, build_backlink_index_from_disk,
    find_by_slug, list_pages, open_or_create_page, page_meta, set_property, PageKind, PageMeta,
    PERSON_TYPE, TYPE_KEY,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

struct Fixture {
    w: Workspace,
    hlc: HlcGenerator,
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let actor = ActorId::new();
        Self {
            w: Workspace::open_in_memory(actor).unwrap(),
            hlc: HlcGenerator::new(actor),
            dir: TempDir::new().unwrap(),
        }
    }

    fn page(&mut self, slug: &str, title: &str) -> NodeId {
        open_or_create_page(&mut self.w, &self.hlc, slug, title, PageKind::Page).unwrap()
    }

    fn alias(&mut self, page: NodeId, aliases: &str) {
        set_property(
            &mut self.w,
            &self.hlc,
            page,
            "alias",
            Some(PropValue::Text(aliases.into())),
        )
        .unwrap();
    }

    fn person(&mut self, page: NodeId) {
        set_property(
            &mut self.w,
            &self.hlc,
            page,
            TYPE_KEY,
            Some(PropValue::Text(PERSON_TYPE.into())),
        )
        .unwrap();
    }

    fn block(&mut self, page: NodeId, text: &str) {
        append_block(&mut self.w, &self.hlc, Some(page), Some(text)).unwrap();
    }

    fn meta(&self, id: NodeId) -> PageMeta {
        page_meta(&self.w, id).unwrap()
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Project every live page so the from-disk builder has `.md` to read.
    fn project(&self) {
        for meta in list_pages(&self.w) {
            let id = find_by_slug(&self.w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&self.w, self.root(), id).unwrap();
        }
    }

    /// The texts of `page`'s backlinks, sorted, from both builders.
    /// Panics when the two builders disagree.
    fn backlinks(&self, page: NodeId) -> Vec<String> {
        self.project();
        let meta = self.meta(page);
        let sorted = |links: Vec<outl_actions::Backlink>| {
            let mut v: Vec<String> = links.into_iter().map(|b| b.block_text).collect();
            v.sort();
            v
        };
        let from_ws = sorted(build_backlink_index(&self.w, self.root()).for_page(&self.w, &meta));
        let from_disk = sorted(
            build_backlink_index_from_disk(&list_pages(&self.w), self.root())
                .for_page(&self.w, &meta),
        );
        assert_eq!(
            from_ws, from_disk,
            "the workspace and from-disk builds must agree for `{}`",
            meta.slug
        );
        from_ws
    }
}

fn texts(items: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn an_alias_naming_another_page_does_not_steal_its_backlinks() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Beta");
    f.block(gamma, "ref to [[Beta]] here");
    f.block(gamma, "tagged #beta here");

    assert_eq!(
        f.backlinks(beta),
        texts(&["ref to [[Beta]] here", "tagged #beta here"])
    );
    assert!(
        f.backlinks(alpha).is_empty(),
        "alpha must not receive beta's mentions through its alias"
    );
}

#[test]
fn an_alias_matching_only_the_title_of_another_page_is_shadowed() {
    // Slug `beta-page`, title `Beta`: the alias collides by title only.
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta-page", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Beta");
    f.block(gamma, "ref to [[Beta]] here");
    f.block(gamma, "tagged #beta here");

    assert_eq!(
        f.backlinks(beta),
        texts(&["ref to [[Beta]] here", "tagged #beta here"])
    );
    assert!(f.backlinks(alpha).is_empty());
}

#[test]
fn an_alias_matching_the_slugified_form_of_another_page_is_shadowed() {
    // Alias `Beta Two` slugifies to the slug `beta-two` of another page.
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta-two", "Something Else");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Beta Two");
    f.block(gamma, "tagged #beta-two here");
    f.block(gamma, "ref to [[Beta Two]] here");

    assert!(f
        .backlinks(beta)
        .contains(&"tagged #beta-two here".to_string()));
    assert!(
        f.backlinks(alpha).is_empty(),
        "navigation sends `Beta Two` to the `beta-two` page, so alpha keeps nothing"
    );
}

#[test]
fn the_comparison_ignores_case() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "BETA");
    f.block(gamma, "tagged #beta here");

    assert_eq!(f.backlinks(beta), texts(&["tagged #beta here"]));
    assert!(f.backlinks(alpha).is_empty());
}

#[test]
fn a_person_alias_naming_another_page_does_not_steal_the_at_forms() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.person(alpha);
    f.person(beta);
    f.alias(alpha, "Beta");
    f.block(gamma, "ping [[@Beta]] now");
    f.block(gamma, "ping [[Beta]] now");

    assert_eq!(
        f.backlinks(beta),
        texts(&["ping [[@Beta]] now", "ping [[Beta]] now"])
    );
    assert!(f.backlinks(alpha).is_empty());
}

#[test]
fn a_person_alias_naming_a_non_person_page_does_not_keep_the_at_form() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.person(alpha);
    f.alias(alpha, "Beta");
    f.block(gamma, "ping [[@Beta]] now");
    f.block(gamma, "ping [[Beta]] now");

    assert_eq!(f.backlinks(beta), texts(&["ping [[Beta]] now"]));
    assert!(
        f.backlinks(alpha).is_empty(),
        "a shadowed alias is dropped in every spelling, `@` forms included"
    );
}

#[test]
fn an_alias_naming_no_other_page_still_lands_on_its_owner() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Nickname");
    f.block(gamma, "ref to [[Nickname]] here");
    f.block(gamma, "tagged #nickname here");

    assert_eq!(
        f.backlinks(alpha),
        texts(&["ref to [[Nickname]] here", "tagged #nickname here"])
    );
}

#[test]
fn two_pages_sharing_an_unclaimed_alias_both_keep_it() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Shared");
    f.alias(beta, "Shared");
    f.block(gamma, "ref to [[Shared]] here");

    assert_eq!(f.backlinks(alpha), texts(&["ref to [[Shared]] here"]));
    assert_eq!(f.backlinks(beta), texts(&["ref to [[Shared]] here"]));
}

#[test]
fn an_alias_matching_a_trashed_page_does_not_count_as_another_page() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let _beta = f.page("beta", "Beta");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Beta");
    outl_actions::delete_page(&mut f.w, &f.hlc, "beta").unwrap();
    f.block(gamma, "ref to [[Beta]] here");

    assert_eq!(f.backlinks(alpha), texts(&["ref to [[Beta]] here"]));
}

#[test]
fn a_page_may_alias_its_own_title() {
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "Alpha Page");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Alpha Page");
    f.block(gamma, "ref to [[Alpha Page]] here");

    assert_eq!(f.backlinks(alpha), texts(&["ref to [[Alpha Page]] here"]));
}

#[test]
fn reindexing_one_page_keeps_the_rule_in_force() {
    // The incremental path: the index is built before `beta` exists, so
    // alpha owns the alias. Once beta appears and is re-indexed, the alias
    // is shadowed without a full rebuild.
    let mut f = Fixture::new();
    let alpha = f.page("alpha", "alpha");
    let gamma = f.page("gamma", "gamma");
    f.alias(alpha, "Beta");
    f.block(gamma, "tagged #beta here");
    f.project();
    let mut index = build_backlink_index_from_disk(&list_pages(&f.w), f.root());
    let alpha_meta = f.meta(alpha);
    assert_eq!(index.for_page(&f.w, &alpha_meta).len(), 1);

    let beta = f.page("beta", "Beta");
    f.project();
    let beta_meta = f.meta(beta);
    index.reindex_page_from_disk(&beta_meta, f.root());

    assert!(index.for_page(&f.w, &alpha_meta).is_empty());
    assert_eq!(index.for_page(&f.w, &beta_meta).len(), 1);
}
