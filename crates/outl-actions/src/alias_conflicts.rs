//! Which `alias::` names are shadowed by another live page.
//!
//! Navigation sends `[[Beta]]` to the page *titled* `Beta` even when
//! some other page claims `alias:: Beta` ("titles win over aliases",
//! [`crate::resolve`] and `outl_md::index`). An alias that names another
//! live page is therefore never reached through the alias owner: the
//! name already has a home. The backlink index follows the same rule, so
//! clicking a `[[Beta]]` and reading the backlinks panel cannot disagree
//! about which page owns the mention.
//!
//! **One owner of the verdict.** `PageClaims::shadowed_by` answers "is
//! this alias taken by another page". The backlink keys
//! (`crate::backlinks_keys::keys_for_page`) and the public
//! [`alias_conflicts`] listing both ask it, so what the user is warned
//! about is exactly what the index ignores.
//!
//! The claims are built once per index build (`O(pages)`), never per
//! page: asking `list_all` for every page would make the build `O(n^2)`.

use std::collections::{HashMap, HashSet};

use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::page::{list_all, PageMeta};

/// One spelling a page answers to, tagged by how navigation compares it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Name {
    /// A slug-shaped name: the page's own slug and the slugified form of
    /// its title. Compared against the alias verbatim and slugified.
    Slug(String),
    /// A title, lower-cased. Compared against the lower-cased alias.
    Title(String),
}

/// The names every live page answers to, built once and consulted per
/// alias.
///
/// It also carries a reverse map, so one page can be withdrawn and
/// registered again (`BacklinkIndex::reindex_page_from_disk`) without a
/// rescan of the others.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageClaims {
    /// Name -> slugs of the live pages answering to it.
    by_name: HashMap<Name, HashSet<String>>,
    /// Slug -> the names it registered, so it can be withdrawn.
    by_page: HashMap<String, Vec<Name>>,
}

impl PageClaims {
    /// Claims for a whole page list. Pass live pages only (what
    /// `list_all` returns); a trashed page answers to nothing.
    pub(crate) fn from_metas<'a>(metas: impl IntoIterator<Item = &'a PageMeta>) -> Self {
        let mut claims = Self::default();
        for meta in metas {
            claims.add_page(meta);
        }
        claims
    }

    /// Register `meta`'s names, replacing whatever the same slug
    /// registered before (a renamed title must not keep its old claim).
    pub(crate) fn add_page(&mut self, meta: &PageMeta) {
        self.remove_page(&meta.slug);
        let names = claimed_names(meta);
        for name in &names {
            self.by_name
                .entry(name.clone())
                .or_default()
                .insert(meta.slug.clone());
        }
        self.by_page.insert(meta.slug.clone(), names);
    }

    /// Forget every name registered by the page with `slug`.
    fn remove_page(&mut self, slug: &str) {
        let Some(names) = self.by_page.remove(slug) else {
            return;
        };
        for name in names {
            if let Some(slugs) = self.by_name.get_mut(&name) {
                slugs.remove(slug);
                if slugs.is_empty() {
                    self.by_name.remove(&name);
                }
            }
        }
    }

    /// The slug of **another** live page that `alias` already names, or
    /// `None` when the alias is free for `owner_slug` to use.
    ///
    /// This is the single predicate behind "this alias is shadowed".
    /// The comparison mirrors the navigation ladder: the alias against a
    /// slug verbatim, its slugified form against a slug, and its
    /// lower-cased form against a title. A page never shadows its own
    /// alias, and two pages sharing a name nobody else holds both keep
    /// it. When several pages hold the name the smallest slug is
    /// reported, so the answer does not depend on hash order.
    pub(crate) fn shadowed_by(&self, owner_slug: &str, alias: &str) -> Option<&str> {
        let probes = [
            Name::Slug(alias.to_string()),
            Name::Slug(outl_md::slug::slugify(alias)),
            Name::Title(alias.to_lowercase()),
        ];
        probes
            .iter()
            .filter_map(|probe| self.by_name.get(probe))
            .flatten()
            .filter(|slug| slug.as_str() != owner_slug)
            .map(String::as_str)
            .min()
    }

    /// The aliases of `meta` that are really its own: the ones no other
    /// live page already names.
    pub(crate) fn live_aliases<'a>(
        &'a self,
        meta: &'a PageMeta,
    ) -> impl Iterator<Item = &'a String> + 'a {
        meta.aliases
            .iter()
            .filter(|alias| self.shadowed_by(&meta.slug, alias).is_none())
    }
}

/// Every name `meta` answers to: its slug, the slugified form of its
/// title (the `#tag` channel) and its lower-cased title.
fn claimed_names(meta: &PageMeta) -> Vec<Name> {
    vec![
        Name::Slug(meta.slug.clone()),
        Name::Slug(outl_md::slug::slugify(&meta.title)),
        Name::Title(meta.title.to_lowercase()),
    ]
}

/// The page that holds a shadowed alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictingPage {
    /// Slug of the page that already answers to the alias.
    pub slug: String,
    /// Its title.
    pub title: String,
}

/// An `alias::` name of a page that another live page already answers to.
///
/// The alias is inert: navigation and backlinks both send the name to
/// `other_page`, so it never reaches the page that declared it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AliasConflict {
    /// The alias as the user wrote it.
    pub alias: String,
    /// The other live page that wins the name.
    pub other_page: ConflictingPage,
}

/// The aliases of `page` shadowed by another live page, in the order
/// they were written.
///
/// Asks the same predicate as the backlink index
/// (`PageClaims::shadowed_by`), so what this lists is exactly what the
/// index ignores. Trashed pages do not count: they are not in
/// `list_all`.
pub fn alias_conflicts(workspace: &Workspace, page: &PageMeta) -> Vec<AliasConflict> {
    if page.aliases.is_empty() {
        return Vec::new();
    }
    let pages = list_all(workspace);
    let claims = PageClaims::from_metas(&pages);
    page.aliases
        .iter()
        .filter_map(|alias| {
            let slug = claims.shadowed_by(&page.slug, alias)?;
            let other = pages.iter().find(|p| p.slug == slug)?;
            Some(AliasConflict {
                alias: alias.clone(),
                other_page: ConflictingPage {
                    slug: other.slug.clone(),
                    title: other.title.clone(),
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{delete, open_or_create, page_meta, set_property, PageKind, ALIAS_KEY};
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::property::PropValue;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn page(w: &mut Workspace, hlc: &HlcGenerator, slug: &str, title: &str) -> NodeId {
        open_or_create(w, hlc, slug, title, PageKind::Page).unwrap()
    }

    fn alias(w: &mut Workspace, hlc: &HlcGenerator, id: NodeId, aliases: &str) {
        let value = Some(PropValue::Text(aliases.into()));
        set_property(w, hlc, id, ALIAS_KEY, value).unwrap();
    }

    #[test]
    fn lists_an_alias_that_names_another_pages_title() {
        let (mut w, hlc) = ws();
        let alpha = page(&mut w, &hlc, "alpha", "alpha");
        page(&mut w, &hlc, "beta-page", "Beta");
        alias(&mut w, &hlc, alpha, "Beta, Free Name");

        let found = alias_conflicts(&w, &page_meta(&w, alpha).unwrap());
        assert_eq!(
            found,
            vec![AliasConflict {
                alias: "Beta".into(),
                other_page: ConflictingPage {
                    slug: "beta-page".into(),
                    title: "Beta".into(),
                },
            }]
        );
    }

    #[test]
    fn lists_an_alias_that_slugifies_to_another_pages_slug() {
        let (mut w, hlc) = ws();
        let alpha = page(&mut w, &hlc, "alpha", "alpha");
        page(&mut w, &hlc, "beta-two", "Something Else");
        alias(&mut w, &hlc, alpha, "Beta Two");

        let found = alias_conflicts(&w, &page_meta(&w, alpha).unwrap());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].other_page.slug, "beta-two");
    }

    #[test]
    fn an_alias_nobody_else_holds_is_not_a_conflict() {
        let (mut w, hlc) = ws();
        let alpha = page(&mut w, &hlc, "alpha", "Alpha Page");
        let beta = page(&mut w, &hlc, "beta", "beta");
        alias(&mut w, &hlc, alpha, "Alpha Page, Shared");
        alias(&mut w, &hlc, beta, "Shared");

        assert!(alias_conflicts(&w, &page_meta(&w, alpha).unwrap()).is_empty());
        assert!(alias_conflicts(&w, &page_meta(&w, beta).unwrap()).is_empty());
    }

    #[test]
    fn a_trashed_page_does_not_hold_the_name() {
        let (mut w, hlc) = ws();
        let alpha = page(&mut w, &hlc, "alpha", "alpha");
        page(&mut w, &hlc, "beta", "beta");
        alias(&mut w, &hlc, alpha, "Beta");
        delete(&mut w, &hlc, "beta").unwrap();

        assert!(alias_conflicts(&w, &page_meta(&w, alpha).unwrap()).is_empty());
    }

    #[test]
    fn re_registering_a_page_withdraws_its_old_claim() {
        let (mut w, hlc) = ws();
        let beta = page(&mut w, &hlc, "beta", "Old Title");
        let mut meta = page_meta(&w, beta).unwrap();
        let mut claims = PageClaims::from_metas(std::slice::from_ref(&meta));
        assert_eq!(claims.shadowed_by("alpha", "Old Title"), Some("beta"));

        meta.title = "New Title".into();
        claims.add_page(&meta);
        assert_eq!(claims.shadowed_by("alpha", "Old Title"), None);
        assert_eq!(claims.shadowed_by("alpha", "New Title"), Some("beta"));
    }
}
