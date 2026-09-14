//! Recover the namespaced `title::` an ingested page never got.
//!
//! [`crate::namespace`] reads a page's hierarchy off its `title`,
//! because that is the only place the `/` a user typed survives (a
//! slug is one path component, so `slugify` folds `/` to `-`). That
//! works for a page created in-app, where `open_or_create_by_name`
//! parks the typed name in `title::`.
//!
//! It does not work for a page that arrived as a `.md` on disk. Its
//! root text is empty and it has no `title::`, so
//! [`crate::page::page_meta`] falls back to the slug — `os-linux`,
//! which has no `/` and therefore nests under nothing.
//!
//! **Measured, not hypothesised.** On a real 2,575-page workspace, 14
//! pages had a `title::`. The 65 `buser-*` pages that *are* the
//! namespaced ones had none, so the nested-pages section rendered
//! empty for every namespace while the backlinks channel — which reads
//! the *mention*, not the title — credited 3,221 blocks under `buser`
//! alone. The feature looked broken because half its input was missing.
//!
//! ## Where the lost name is still written down
//!
//! In the mentions. A block saying `[[buser/tech/data]]` spells the
//! namespaced name in full, and `slugify` maps it to exactly the slug
//! the ingested page carries. So the repair is a join: for every
//! namespaced mention in the workspace, if a page exists at its
//! slugified form and has no title of its own, that mention *is* the
//! title.
//!
//! This is recovery, not invention. The pass never derives a hierarchy
//! from a slug — `meu-projeto` stays one segment forever, because no
//! mention ever spelled it `meu/projeto`.
//!
//! ## What it refuses
//!
//! - A page that already has a non-empty `title::`, or whose root text
//!   is non-empty. Both mean somebody already said what this page is
//!   called, and a repair that overwrites a human is not a repair.
//! - A slug two different spellings claim (`buser/tech` and
//!   `Buser/Tech` both slugify to `buser-tech`). Picking one would be a
//!   coin flip written into the op log, so the pass leaves it alone and
//!   reports it. Silence would be worse: the page stays un-nested and
//!   nobody knows why.
//! - A mention with no `/`. It would set the title to the slug, which
//!   is what `page_meta` already falls back to — an op that changes
//!   nothing.

use std::collections::{HashMap, HashSet};

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::mentions::extract_refs_and_tags;
use crate::page::{PageKind, TITLE_KEY};
use crate::tree::walk_subtree;

/// What one pass did, and what it declined to touch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NamespaceTitleRepair {
    /// Pages that gained a namespaced `title::`, as `(slug, title)`.
    pub repaired: Vec<(String, String)>,
    /// Slugs where two or more spellings disagreed, with the
    /// candidates. Reported rather than guessed — see the module doc.
    pub ambiguous: Vec<(String, Vec<String>)>,
}

impl NamespaceTitleRepair {
    /// Nothing to do and nothing to report.
    pub fn is_clean(&self) -> bool {
        self.repaired.is_empty() && self.ambiguous.is_empty()
    }
}

/// Scan the workspace's mentions and give every titleless page that a
/// namespaced mention names its `title::` back.
///
/// Idempotent: a second run finds every repaired page already titled
/// and emits nothing. Scales with blocks (one walk) plus pages, so
/// clients run it off the synchronous boot path, like
/// [`crate::page_repair_titles`].
pub fn repair_namespaced_titles(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
) -> Result<NamespaceTitleRepair, ActionError> {
    let candidates = collect_namespaced_mentions(workspace);
    let mut out = NamespaceTitleRepair::default();

    for meta in crate::page::list_all(workspace) {
        if meta.kind == PageKind::Journal {
            continue;
        }
        let Some(spellings) = candidates.get(&meta.slug) else {
            continue;
        };
        // A page whose title is already something other than its slug
        // has been named by a human or by in-app creation. `page_meta`
        // resolves `title::` first and the root text second, and it
        // falls back to the slug only when both are empty — so this one
        // comparison covers both cases without re-reading the tree.
        if meta.title != meta.slug {
            continue;
        }
        let mut names: Vec<&String> = spellings.iter().collect();
        names.sort();
        if names.len() > 1 {
            out.ambiguous.push((
                meta.slug.clone(),
                names.into_iter().cloned().collect::<Vec<_>>(),
            ));
            continue;
        }
        let title = names[0].clone();
        let Some(id) = crate::page::find_by_slug(workspace, &meta.slug) else {
            continue;
        };
        crate::page::set_property(
            workspace,
            hlc,
            id,
            TITLE_KEY,
            Some(PropValue::Text(title.clone())),
        )?;
        out.repaired.push((meta.slug, title));
    }
    Ok(out)
}

/// Every namespaced name mentioned anywhere in the workspace, indexed
/// by the slug it resolves to.
///
/// Both channels count: `[[a/b]]` and `#a/b` resolve through the same
/// `slugify`, so a page referenced one way and tagged the other is one
/// candidate, not two.
fn collect_namespaced_mentions(workspace: &Workspace) -> HashMap<String, HashSet<String>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    walk_subtree(workspace, NodeId::root(), |id| {
        if let Some(text) = workspace.block_text(id) {
            let (refs, tags) = extract_refs_and_tags(&text);
            for name in refs.into_iter().chain(tags) {
                // No `/`, nothing to recover: the title would equal the
                // slug, which is already the fallback.
                if !name.contains('/') {
                    continue;
                }
                out.entry(outl_md::slug::slugify(&name))
                    .or_default()
                    .insert(name);
            }
        }
        true
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::namespace::descendants;
    use crate::page::{list_all, open_or_create, PageKind};
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    /// The shape a Roam/Logseq import leaves behind: the page exists at
    /// its flattened slug with no `title::`, and only the mentions
    /// still spell the namespace.
    fn ingested_page(w: &mut Workspace, hlc: &HlcGenerator, slug: &str) {
        open_or_create(w, hlc, slug, slug, PageKind::Page).unwrap();
    }

    #[test]
    fn a_mention_gives_an_ingested_page_its_namespace_back() {
        let (mut w, hlc) = ws();
        ingested_page(&mut w, &hlc, "os");
        ingested_page(&mut w, &hlc, "os-linux");
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(notes), Some("see [[os/linux]] today")).unwrap();

        // Before: the page is titled by its slug, so it nests nowhere.
        assert!(descendants(&list_all(&w), "os").is_empty());

        let report = repair_namespaced_titles(&mut w, &hlc).unwrap();
        assert_eq!(
            report.repaired,
            vec![("os-linux".to_string(), "os/linux".to_string())]
        );

        let found = descendants(&list_all(&w), "os");
        let rows: Vec<(&str, usize, &str)> = found
            .iter()
            .map(|c| (c.page.title.as_str(), c.depth, c.label.as_str()))
            .collect();
        assert_eq!(rows, vec![("os/linux", 1, "linux")]);
    }

    #[test]
    fn it_is_idempotent() {
        let (mut w, hlc) = ws();
        ingested_page(&mut w, &hlc, "os-linux");
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(notes), Some("#os/linux")).unwrap();

        assert_eq!(
            repair_namespaced_titles(&mut w, &hlc)
                .unwrap()
                .repaired
                .len(),
            1
        );
        assert!(repair_namespaced_titles(&mut w, &hlc).unwrap().is_clean());
    }

    #[test]
    fn a_page_that_already_has_a_title_is_left_alone() {
        // In-app creation writes the title, so this page is already
        // right. Overwriting a human is not a repair.
        let (mut w, hlc) = ws();
        crate::resolve::open_or_create_by_name(&mut w, &hlc, "os/Linux Desktop", PageKind::Page)
            .unwrap();
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(notes), Some("see [[os/linux desktop]]")).unwrap();

        assert!(repair_namespaced_titles(&mut w, &hlc).unwrap().is_clean());
        let titles: Vec<String> = list_all(&w).into_iter().map(|p| p.title).collect();
        assert!(titles.contains(&"os/Linux Desktop".to_string()));
    }

    #[test]
    fn two_spellings_of_one_slug_are_reported_not_guessed() {
        // `buser/tech` and `Buser/Tech` both slugify to `buser-tech`.
        // Writing a coin flip into the op log is worse than saying so.
        let (mut w, hlc) = ws();
        ingested_page(&mut w, &hlc, "buser-tech");
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(
            &mut w,
            &hlc,
            Some(notes),
            Some("[[buser/tech]] and [[Buser/Tech]]"),
        )
        .unwrap();

        let report = repair_namespaced_titles(&mut w, &hlc).unwrap();
        assert!(report.repaired.is_empty());
        assert_eq!(
            report.ambiguous,
            vec![(
                "buser-tech".to_string(),
                vec!["Buser/Tech".to_string(), "buser/tech".to_string()]
            )]
        );
    }

    #[test]
    fn a_flat_mention_never_writes_a_title() {
        // `[[notes]]` would set `title:: notes`, which is what
        // `page_meta` already falls back to — an op that changes
        // nothing, on every page in the workspace.
        let (mut w, hlc) = ws();
        ingested_page(&mut w, &hlc, "notes");
        let other = open_or_create(&mut w, &hlc, "other", "other", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(other), Some("see [[notes]]")).unwrap();

        assert!(repair_namespaced_titles(&mut w, &hlc).unwrap().is_clean());
    }

    #[test]
    fn a_mention_of_a_page_that_does_not_exist_is_ignored() {
        // The namespace `os/linux` is mentioned but no page carries its
        // slug, so there is nothing to title.
        let (mut w, hlc) = ws();
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(notes), Some("see [[os/linux]]")).unwrap();

        assert!(repair_namespaced_titles(&mut w, &hlc).unwrap().is_clean());
    }

    #[test]
    fn a_journal_is_never_retitled() {
        // A journal's title is its date. Nothing nests under it, and a
        // `[[2026-09-13/x]]` mention must not rename the day.
        let (mut w, hlc) = ws();
        crate::page::open_journal(
            &mut w,
            &hlc,
            chrono::NaiveDate::from_ymd_opt(2026, 9, 13).unwrap(),
        )
        .unwrap();
        let notes = open_or_create(&mut w, &hlc, "notes", "notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(notes), Some("[[2026-09-13]] fine")).unwrap();

        assert!(repair_namespaced_titles(&mut w, &hlc).unwrap().is_clean());
    }
}
