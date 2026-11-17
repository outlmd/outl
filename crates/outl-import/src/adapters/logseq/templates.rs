//! Logseq block templates → outl template pages (issue #563).
//!
//! A Logseq template is a **block** carrying `template:: Name`,
//! usually parked on a `Templates` page, and the thing it stamps is
//! that block's children (or the block itself, with
//! `template-including-parent:: true`).
//!
//! An outl template is a **page** carrying page-level `template::
//! name` ([`docs/templates.md`](../../../../../docs/templates.md)).
//! So a graph that lives in block templates used to import cleanly and
//! then show an empty template picker: the property survived as an
//! inert block property that `outl template apply` never looks at.
//!
//! This pass **copies**, it does not move. The original block stays
//! where the source put it, with its property, because a template is
//! also content someone wrote and deleting it to build a page would be
//! the one thing an importer must never do. The copy is what becomes
//! the template page.
//!
//! Copying means the emitted page and block counts now exceed the
//! source's, which would make the reconciliation false-alarm — so the
//! copies are counted (`pages_added_as_templates`,
//! `blocks_copied_into_templates`) and `ImportReport::finalize`
//! subtracts them by name, exactly as it does for `pages_merged`.

use super::outline::count_blocks;
use crate::ir::{ImportBlock, ImportGraph, ImportPage, PageBody, PageName};
use crate::report::ImportReport;
use outl_md::slug::slugify;
use std::collections::BTreeSet;

/// Logseq's block-template property.
const TEMPLATE_KEY: &str = "template";
/// Logseq's "stamp the carrier block too" switch.
const INCLUDING_PARENT_KEY: &str = "template-including-parent";
/// Where a lifted template lands, matching the `templates/<name>`
/// convention `outl init` uses for the journal template.
const TEMPLATE_PAGE_PREFIX: &str = "templates/";

/// Lift every block-level `template::` in the graph into its own
/// template page.
pub(crate) fn lift(graph: &mut ImportGraph, report: &mut ImportReport) {
    // A page that already carries page-level `template::` is already a
    // working outl template — the issue is explicit that it stays as
    // it is, and a second page with the same invocation name would
    // make `/template <name>` ambiguous.
    let mut taken: BTreeSet<String> = graph
        .pages
        .iter()
        .filter_map(|p| {
            p.props
                .iter()
                .find(|(k, _)| k == TEMPLATE_KEY)
                .map(|(_, v)| v.trim().to_lowercase())
        })
        .collect();

    let mut found: Vec<Found> = Vec::new();
    for page in &graph.pages {
        if let PageBody::Outline(blocks) = &page.body {
            collect(blocks, &page_label(page), &mut found);
        }
    }

    // Every stem a named page already occupies. The emitter merges named
    // pages that slugify alike, so `daily note` and `daily-note` would
    // otherwise land on one `templates-daily-note` page with both bodies,
    // and a lifted template could merge into a source page of the same
    // slug. Each lifted page gets its own pinned stem instead.
    let mut stems: BTreeSet<String> = graph
        .pages
        .iter()
        .filter_map(|p| match &p.name {
            PageName::Named(n) => Some(p.stem_override.clone().unwrap_or_else(|| slugify(n))),
            PageName::Journal(_) => None,
        })
        .collect();

    let mut lifted: Vec<ImportPage> = Vec::new();
    for f in found {
        let key = f.name.to_lowercase();
        if !taken.insert(key) {
            report.warn(
                &f.page,
                format!(
                    "two templates are named `{}` — the first one found is the template outl \
                     will apply, the other is still in the page it came from",
                    f.name
                ),
            );
            continue;
        }
        if f.body.is_empty() {
            report.warn(
                &f.page,
                format!(
                    "template `{}` has no body (no child blocks, and no \
                     `template-including-parent:: true`) — nothing to stamp, so no template \
                     page was created",
                    f.name
                ),
            );
            continue;
        }
        report.pages_added_as_templates += 1;
        report.blocks_copied_into_templates += f.body.iter().map(count_blocks).sum::<usize>();
        let title = format!("{TEMPLATE_PAGE_PREFIX}{}", f.name);
        let stem = free_stem(&slugify(&title), &mut stems);
        lifted.push(ImportPage {
            name: PageName::Named(title),
            props: vec![(TEMPLATE_KEY.to_string(), f.name.clone())],
            body: PageBody::Outline(f.body),
            stem_override: Some(stem),
            created: None,
            edited: None,
            source_uid: None,
        });
    }
    graph.pages.extend(lifted);
}

/// `base`, or `base-2`, `base-3`… — the first stem no page holds yet,
/// claimed before it is returned.
fn free_stem(base: &str, taken: &mut BTreeSet<String>) -> String {
    let stem = (1..)
        .map(|n| {
            if n == 1 {
                base.to_string()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken.contains(s))
        .expect("an unbounded range always yields a free stem");
    taken.insert(stem.clone());
    stem
}

/// One block template found in the source.
struct Found {
    /// Invocation name, the `template::` value as written.
    name: String,
    /// Page it was found on, for warnings.
    page: String,
    /// The template body, already copied and uid-stripped.
    body: Vec<ImportBlock>,
}

/// Walk an outline, depth-first, recording every block template.
fn collect(blocks: &[ImportBlock], page: &str, out: &mut Vec<Found>) {
    for b in blocks {
        if let Some(name) = template_name(b) {
            let body = if including_parent(b) {
                vec![strip(b.clone())]
            } else {
                b.children.iter().cloned().map(strip).collect()
            };
            out.push(Found {
                name,
                page: page.to_string(),
                body,
            });
        }
        collect(&b.children, page, out);
    }
}

/// The `template::` value, when the block carries a usable one.
fn template_name(b: &ImportBlock) -> Option<String> {
    let raw = b
        .props
        .iter()
        .find(|(k, _)| k == TEMPLATE_KEY)
        .map(|(_, v)| v.trim())?;
    // A bare `template::` names nothing, so there is nothing to invoke
    // and nothing to lift — the property stays on the block as text.
    (!raw.is_empty()).then(|| raw.to_string())
}

/// `template-including-parent:: true` — Logseq stamps the carrier
/// block as part of the template. Logseq's own default when the
/// property is absent is to stamp children only.
fn including_parent(b: &ImportBlock) -> bool {
    b.props
        .iter()
        .find(|(k, _)| k == INCLUDING_PARENT_KEY)
        .is_some_and(|(_, v)| v.trim().eq_ignore_ascii_case("true"))
}

/// Prepare a copied block: drop the template bookkeeping properties
/// and, recursively, every source uid.
///
/// **The uid is the part that matters.** The resolve pass maps one
/// source uid to one `((blk-XXXXXX))` handle through the sidecar's
/// depth-first block list; two blocks claiming the same uid would make
/// that mapping ambiguous and could point an existing `((ref))` at the
/// template copy instead of the block the user wrote it against.
fn strip(mut b: ImportBlock) -> ImportBlock {
    b.uid = None;
    b.props
        .retain(|(k, _)| k != TEMPLATE_KEY && k != INCLUDING_PARENT_KEY);
    b.children = b.children.into_iter().map(strip).collect();
    b
}

/// Page label for a warning.
fn page_label(p: &ImportPage) -> String {
    match &p.name {
        PageName::Journal(d) => d.to_string(),
        PageName::Named(n) => n.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BlockContent, Inline};

    fn block(text: &str, props: &[(&str, &str)], children: Vec<ImportBlock>) -> ImportBlock {
        ImportBlock {
            content: BlockContent::Inline(vec![Inline::Text(text.to_string())]),
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            children,
            ..ImportBlock::default()
        }
    }

    fn page(name: &str, props: &[(&str, &str)], blocks: Vec<ImportBlock>) -> ImportPage {
        ImportPage {
            name: PageName::Named(name.to_string()),
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: PageBody::Outline(blocks),
            stem_override: None,
            created: None,
            edited: None,
            source_uid: None,
        }
    }

    fn graph_of(pages: Vec<ImportPage>) -> ImportGraph {
        ImportGraph {
            pages,
            ..ImportGraph::default()
        }
    }

    fn text_of(b: &ImportBlock) -> String {
        match &b.content {
            BlockContent::Inline(toks) => toks
                .iter()
                .map(|t| match t {
                    Inline::Text(s) => s.clone(),
                    other => format!("{other:?}"),
                })
                .collect(),
            other => panic!("expected inline, got {other:?}"),
        }
    }

    fn template_page<'a>(g: &'a ImportGraph, name: &str) -> &'a ImportPage {
        g.pages
            .iter()
            .find(|p| p.name == PageName::Named(format!("templates/{name}")))
            .unwrap_or_else(|| panic!("no template page for {name} in {:?}", g.pages.len()))
    }

    /// The default: the template body is the carrier block's children.
    #[test]
    fn a_block_template_becomes_a_template_page_of_its_children() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![block(
                "Interview",
                &[("template", "interview")],
                vec![
                    block("Candidate:", &[], vec![]),
                    block("Role:", &[], vec![]),
                ],
            )],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        let tpl = template_page(&g, "interview");
        assert_eq!(
            tpl.props,
            vec![("template".to_string(), "interview".to_string())]
        );
        let PageBody::Outline(body) = &tpl.body else {
            panic!("outline expected");
        };
        assert_eq!(body.len(), 2);
        assert_eq!(text_of(&body[0]), "Candidate:");
        assert_eq!(report.pages_added_as_templates, 1);
        assert_eq!(report.blocks_copied_into_templates, 2);
    }

    /// `template-including-parent:: true` keeps the carrier block.
    #[test]
    fn including_parent_keeps_the_carrier_block_in_the_body() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![block(
                "**Standup**",
                &[
                    ("template", "standup"),
                    ("template-including-parent", "true"),
                ],
                vec![block("Yesterday:", &[], vec![])],
            )],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        let PageBody::Outline(body) = &template_page(&g, "standup").body else {
            panic!("outline expected");
        };
        assert_eq!(body.len(), 1);
        assert_eq!(text_of(&body[0]), "**Standup**");
        assert_eq!(body[0].children.len(), 1);
        assert_eq!(report.blocks_copied_into_templates, 2);
    }

    /// The bookkeeping properties are Logseq's, not the template's.
    #[test]
    fn the_copy_drops_the_template_properties_and_every_uid() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![ImportBlock {
                uid: Some("6601-aaa".into()),
                props: vec![
                    ("template".into(), "weekly".into()),
                    ("template-including-parent".into(), "true".into()),
                ],
                children: vec![ImportBlock {
                    uid: Some("6601-bbb".into()),
                    ..block("child", &[], vec![])
                }],
                ..block("root", &[], vec![])
            }],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        let PageBody::Outline(body) = &template_page(&g, "weekly").body else {
            panic!("outline expected");
        };
        assert!(body[0].props.is_empty(), "{:?}", body[0].props);
        assert_eq!(body[0].uid, None, "a copied uid would dangle real refs");
        assert_eq!(body[0].children[0].uid, None);
    }

    /// The source keeps what it had — lifting is a copy, not a move.
    #[test]
    fn the_original_block_is_left_alone() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![block(
                "Interview",
                &[("template", "interview")],
                vec![block("Candidate:", &[], vec![])],
            )],
        )]);
        lift(&mut g, &mut ImportReport::new("logseq"));

        let PageBody::Outline(src) = &g.pages[0].body else {
            panic!("outline expected");
        };
        assert_eq!(src.len(), 1);
        assert_eq!(src[0].children.len(), 1);
        assert_eq!(
            src[0].props,
            vec![("template".to_string(), "interview".to_string())],
            "the source block keeps its property"
        );
    }

    /// A page that is already an outl template is left as it is, and
    /// does not get a second page competing for the same name.
    #[test]
    fn a_page_level_template_is_not_lifted_again() {
        let mut g = graph_of(vec![
            page(
                "templates/interview",
                &[("template", "interview")],
                vec![block("Candidate:", &[], vec![])],
            ),
            page(
                "Templates",
                &[],
                vec![block(
                    "Interview",
                    &[("template", "interview")],
                    vec![block("Role:", &[], vec![])],
                )],
            ),
        ]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        assert_eq!(g.pages.len(), 2, "nothing added");
        assert_eq!(report.pages_added_as_templates, 0);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.detail.contains("interview")),
            "{:?}",
            report.warnings
        );
    }

    /// Two block templates with one name: the first wins, out loud.
    #[test]
    fn a_duplicate_name_keeps_the_first_and_warns() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![
                block(
                    "one",
                    &[("template", "daily")],
                    vec![block("first", &[], vec![])],
                ),
                block(
                    "two",
                    &[("template", "daily")],
                    vec![block("second", &[], vec![])],
                ),
            ],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        assert_eq!(report.pages_added_as_templates, 1);
        let PageBody::Outline(body) = &template_page(&g, "daily").body else {
            panic!("outline expected");
        };
        assert_eq!(text_of(&body[0]), "first");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    }

    /// Two names that slugify alike stay two templates: the emitter
    /// merges named pages by slug, which would fold both bodies into one
    /// page and leave only one `template::` name to invoke. A source page
    /// already on that slug keeps it too.
    #[test]
    fn templates_whose_names_slugify_alike_get_their_own_stems() {
        let mut g = graph_of(vec![
            page(
                "templates/daily note",
                &[],
                vec![block("a page someone wrote", &[], vec![])],
            ),
            page(
                "Templates",
                &[],
                vec![
                    block(
                        "one",
                        &[("template", "daily note")],
                        vec![block("first", &[], vec![])],
                    ),
                    block(
                        "two",
                        &[("template", "daily-note")],
                        vec![block("second", &[], vec![])],
                    ),
                ],
            ),
        ]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        assert_eq!(report.pages_added_as_templates, 2);
        let a = template_page(&g, "daily note");
        let b = template_page(&g, "daily-note");
        let base = slugify("templates/daily note");
        assert_eq!(a.stem_override, Some(format!("{base}-2")));
        assert_eq!(b.stem_override, Some(format!("{base}-3")));
        assert_eq!(
            b.props,
            vec![("template".to_string(), "daily-note".to_string())]
        );
    }

    /// A template with no body stamps nothing; an empty template page
    /// would just be a name in the picker that does nothing.
    #[test]
    fn a_template_with_no_body_is_reported_instead_of_created() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![block("lonely", &[("template", "empty")], vec![])],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);

        assert_eq!(report.pages_added_as_templates, 0);
        assert!(
            report.warnings.iter().any(|w| w.detail.contains("no body")),
            "{:?}",
            report.warnings
        );
    }

    /// A bare `template::` names nothing to invoke.
    #[test]
    fn a_nameless_template_property_lifts_nothing() {
        let mut g = graph_of(vec![page(
            "Templates",
            &[],
            vec![block(
                "x",
                &[("template", "")],
                vec![block("child", &[], vec![])],
            )],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);
        assert_eq!(report.pages_added_as_templates, 0);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    /// Templates are found at any depth, not only at the page root.
    #[test]
    fn a_nested_block_template_is_found() {
        let mut g = graph_of(vec![page(
            "Notes",
            &[],
            vec![block(
                "section",
                &[],
                vec![block(
                    "deep",
                    &[("template", "deep")],
                    vec![block("body", &[], vec![])],
                )],
            )],
        )]);
        let mut report = ImportReport::new("logseq");
        lift(&mut g, &mut report);
        assert_eq!(report.pages_added_as_templates, 1);
        let PageBody::Outline(body) = &template_page(&g, "deep").body else {
            panic!("outline expected");
        };
        assert_eq!(text_of(&body[0]), "body");
    }
}
