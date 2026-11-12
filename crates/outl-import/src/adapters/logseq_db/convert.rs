//! Entity table → typed IR.
//!
//! Decides which entities are pages, which are blocks and which are
//! neither (property definitions, closed values, built-ins), rebuilds
//! each page's outline from `:block/parent` + `:block/order`, and
//! converts every block through the text and property passes.

use super::block::ms_to_datetime;
use super::graph::{Entity, Graph};
use super::props::Collected;
use super::text::{tag_value, RefTargets};
use crate::ir::{ImportBlock, ImportGraph, ImportPage, PageBody, PageName};
use crate::report::ImportReport;
use chrono::NaiveDate;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Warning scope for findings that belong to the whole import.
const GRAPH_SCOPE: &str = "(graph)";
/// How many example uuids a dangling-ref warning spells out.
const DANGLING_EXAMPLES: usize = 5;

/// Entity ids of the built-in classes the converter reads, resolved
/// once through `:db/ident`. `None` when the graph never created it.
#[derive(Debug, Default)]
pub(super) struct Classes {
    pub(super) page: Option<i64>,
    pub(super) journal: Option<i64>,
    pub(super) tag: Option<i64>,
    pub(super) property: Option<i64>,
    pub(super) whiteboard: Option<i64>,
    pub(super) code: Option<i64>,
    pub(super) quote: Option<i64>,
    pub(super) math: Option<i64>,
    pub(super) query: Option<i64>,
    pub(super) cards: Option<i64>,
    pub(super) asset: Option<i64>,
    pub(super) template: Option<i64>,
    pub(super) pdf: Option<i64>,
}

impl Classes {
    fn load(g: &Graph) -> Self {
        let id = |n: &str| g.ident_id(&format!("logseq.class/{n}"));
        Self {
            page: id("Page"),
            journal: id("Journal"),
            tag: id("Tag"),
            property: id("Property"),
            whiteboard: id("Whiteboard"),
            code: id("Code-block"),
            quote: id("Quote-block"),
            math: id("Math-block"),
            query: id("Query"),
            cards: id("Cards"),
            asset: id("Asset"),
            template: id("Template"),
            pdf: id("Pdf-annotation"),
        }
    }
}

pub(super) fn has(tags: &[i64], class: Option<i64>) -> bool {
    class.is_some_and(|c| tags.contains(&c))
}

/// Everything counted on the way through that the report has no field
/// for; turned into one aggregate warning at the end.
#[derive(Debug, Default)]
pub(super) struct Tally {
    built_in_classes: usize,
    property_defs: usize,
    closed_values: usize,
    value_blocks: usize,
    recycled_blocks: usize,
    hidden_blocks: usize,
    reparented_blocks: usize,
    cyclic_blocks: usize,
    pub(super) stripped: BTreeMap<String, usize>,
    pub(super) dangling: Vec<(String, String)>,
}

/// Shared state of one conversion.
pub(super) struct Ctx<'g> {
    pub(super) g: &'g Graph,
    pub(super) cls: Classes,
    pub(super) targets: RefTargets,
    pub(super) tally: Tally,
}

/// Why a node is left out of the import.
enum Exclusion {
    Recycled,
    Hidden,
}

/// What to do with one page-like entity.
enum Decision {
    Import,
    Skip(&'static str),
    /// Not a page in the user's mind (a property definition, a closed
    /// value, a built-in tag): counted in the aggregate warning.
    Structural,
}

/// Entry point: the whole graph → [`ImportGraph`].
pub(super) fn convert(g: &Graph, report: &mut ImportReport) -> ImportGraph {
    let mut ctx = Ctx {
        g,
        cls: Classes::load(g),
        targets: RefTargets::default(),
        tally: Tally::default(),
    };
    let by_page = ctx.group_blocks();
    let decisions = ctx.decide_pages(&by_page);
    ctx.register_targets(&by_page, &decisions);

    let mut out = ImportGraph::default();
    for (id, decision) in &decisions {
        let ent = g.entity(*id).expect("decided pages exist");
        let blocks = by_page.get(id).map_or(&[][..], Vec::as_slice);
        match decision {
            Decision::Import => {
                report.source_pages += 1;
                report.source_blocks += blocks.len();
                out.pages.push(ctx.convert_page(*id, ent, blocks, report));
            }
            Decision::Skip(reason) => {
                report.source_pages += 1;
                report.source_blocks += blocks.len();
                report.skip(page_label(ent, *id), *reason, blocks.len());
            }
            Decision::Structural => {}
        }
    }
    ctx.skip_orphan_pages(&by_page, &decisions, report);
    ctx.finish(report);
    out
}

fn page_label(ent: &Entity, id: i64) -> String {
    match ent.str("block/title") {
        Some(t) => format!("page \"{t}\""),
        None => format!("page entity #{id}"),
    }
}

impl Ctx<'_> {
    /// Blocks per page, minus the recycled and hidden ones and the
    /// value blocks properties keep their text in.
    fn group_blocks(&mut self) -> HashMap<i64, Vec<i64>> {
        let mut by_page: HashMap<i64, Vec<i64>> = HashMap::new();
        for (id, ent) in self.g.entities() {
            // Property values keep their text on an entity of their own;
            // it is folded into the owning node's `key:: value`.
            if ent.get("block/closed-value-property").is_some() {
                self.tally.closed_values += 1;
                continue;
            }
            let Some(page) = ent.int("block/page") else {
                continue;
            };
            if ent.get("logseq.property/created-from-property").is_some() {
                self.tally.value_blocks += 1;
                continue;
            }
            match self.exclusion(id) {
                Some(Exclusion::Recycled) => self.tally.recycled_blocks += 1,
                Some(Exclusion::Hidden) => self.tally.hidden_blocks += 1,
                None => by_page.entry(page).or_default().push(id),
            }
        }
        by_page
    }

    /// Walk up the parent chain: a node inherits its ancestors'
    /// recycle-bin and hidden flags.
    fn exclusion(&self, start: i64) -> Option<Exclusion> {
        let mut id = start;
        for _ in 0..1024 {
            let e = self.g.entity(id)?;
            if e.get("logseq.property/deleted-at").is_some() {
                return Some(Exclusion::Recycled);
            }
            if e.flag("logseq.property/hide?") {
                return Some(Exclusion::Hidden);
            }
            id = e.int("block/parent")?;
        }
        None
    }

    fn decide_pages(&mut self, by_page: &HashMap<i64, Vec<i64>>) -> Vec<(i64, Decision)> {
        let mut out = Vec::new();
        for (id, ent) in self.g.entities() {
            let tags = ent.refs("block/tags");
            let has_blocks = by_page.contains_key(&id);
            let page_like = [
                self.cls.page,
                self.cls.journal,
                self.cls.tag,
                self.cls.property,
                self.cls.whiteboard,
            ]
            .iter()
            .any(|c| has(&tags, *c));
            if !(has_blocks || (page_like && ent.int("block/page").is_none())) {
                continue;
            }
            out.push((id, self.decide(ent, &tags, has_blocks)));
        }
        out
    }

    fn decide(&mut self, ent: &Entity, tags: &[i64], has_blocks: bool) -> Decision {
        let structural = has(tags, self.cls.property);
        if has(tags, self.cls.whiteboard) {
            return Decision::Skip("whiteboards are not imported yet");
        }
        if self.exclusion_of_entity(ent) {
            return Decision::Skip("hidden or in the recycle bin");
        }
        if ent.str("block/title").is_none() {
            return Decision::Skip("page has no title");
        }
        let built_in = ent.flag("logseq.property/built-in?");
        let page_kind = has(tags, self.cls.page) || has(tags, self.cls.journal);
        if has_blocks {
            return Decision::Import;
        }
        if structural {
            self.tally.property_defs += 1;
            return Decision::Structural;
        }
        if built_in && !page_kind {
            self.tally.built_in_classes += 1;
            return Decision::Structural;
        }
        if built_in {
            return Decision::Skip("built-in page");
        }
        Decision::Import
    }

    fn exclusion_of_entity(&self, ent: &Entity) -> bool {
        ent.get("logseq.property/deleted-at").is_some() || ent.flag("logseq.property/hide?")
    }

    /// uuid → title for every page, uuid set for every block, so the
    /// text pass can classify a reference without touching the graph.
    fn register_targets(
        &mut self,
        by_page: &HashMap<i64, Vec<i64>>,
        decisions: &[(i64, Decision)],
    ) {
        for (id, decision) in decisions {
            if matches!(decision, Decision::Structural) {
                continue;
            }
            let Some(ent) = self.g.entity(*id) else {
                continue;
            };
            if let (Some(uuid), Some(title)) = (ent.uuid(), self.page_title(ent)) {
                self.targets.pages.insert(uuid, title);
            }
        }
        // Classes and property definitions are referenced from text as
        // tags / links even when they are not imported as pages.
        for (_, ent) in self.g.entities() {
            let tags = ent.refs("block/tags");
            let titled = ent.str("block/title").is_some();
            if titled && (has(&tags, self.cls.tag) || has(&tags, self.cls.property)) {
                if let Some(uuid) = ent.uuid() {
                    let title = self.page_title(ent).unwrap_or_default();
                    self.targets.pages.entry(uuid).or_insert(title);
                }
            }
        }
        for ids in by_page.values() {
            for id in ids {
                if let Some(uuid) = self.g.entity(*id).and_then(Entity::uuid) {
                    self.targets.blocks.insert(uuid);
                }
            }
        }
    }

    /// Journals are spelled as their ISO date, everything else by title.
    fn page_title(&self, ent: &Entity) -> Option<String> {
        if let Some(d) = journal_date(ent) {
            return Some(d.to_string());
        }
        ent.str("block/title").map(str::to_string)
    }

    /// A block pointing at a page nobody decided on (a missing entity)
    /// would otherwise vanish from both sides of the reconciliation.
    fn skip_orphan_pages(
        &self,
        by_page: &HashMap<i64, Vec<i64>>,
        decisions: &[(i64, Decision)],
        report: &mut ImportReport,
    ) {
        let decided: HashSet<i64> = decisions.iter().map(|(id, _)| *id).collect();
        let mut orphans: Vec<(&i64, &Vec<i64>)> = by_page
            .iter()
            .filter(|(p, _)| !decided.contains(p))
            .collect();
        orphans.sort();
        for (page, blocks) in orphans {
            report.source_pages += 1;
            report.source_blocks += blocks.len();
            report.skip(
                format!("page entity #{page}"),
                "blocks point at a page that is not in the export",
                blocks.len(),
            );
        }
    }

    fn convert_page(
        &mut self,
        id: i64,
        ent: &Entity,
        block_ids: &[i64],
        report: &mut ImportReport,
    ) -> ImportPage {
        let title = ent.str("block/title").unwrap_or_default().to_string();
        let label = journal_date(ent).map_or(title.clone(), |d| d.to_string());
        let name = match journal_date(ent) {
            Some(d) => PageName::Journal(d),
            None => PageName::Named(title),
        };
        let mut c = self.collect_props(ent);
        let mut props = std::mem::take(&mut c.props);
        self.page_header_props(ent, &c, &mut props);
        let blocks = self.build_tree(id, block_ids, &label, report);
        ImportPage {
            name,
            props,
            body: PageBody::Outline(blocks),
            stem_override: None,
            created: ent.int("block/created-at").and_then(ms_to_datetime),
            edited: ent.int("block/updated-at").and_then(ms_to_datetime),
            source_uid: None,
        }
    }

    /// `alias::` and `tags::` page properties.
    fn page_header_props(
        &mut self,
        ent: &Entity,
        c: &Collected,
        props: &mut Vec<(String, String)>,
    ) {
        let aliases: Vec<String> = ent
            .refs("block/alias")
            .into_iter()
            .filter_map(|a| self.title_of(a))
            .map(|t| format!("[[{t}]]"))
            .collect();
        if !aliases.is_empty() {
            props.insert(0, ("alias".to_string(), aliases.join(", ")));
        }
        let mut tags = self.user_tag_titles(ent);
        for t in &c.page_tags {
            if !tags.contains(t) {
                tags.push(t.clone());
            }
        }
        if !tags.is_empty() {
            let joined: Vec<String> = tags.iter().map(|t| tag_value(t)).collect();
            props.push(("tags".to_string(), joined.join(" ")));
        }
    }

    /// Titles of the non-built-in classes a node is tagged with.
    fn user_tag_titles(&self, ent: &Entity) -> Vec<String> {
        ent.refs("block/tags")
            .into_iter()
            .filter_map(|t| self.g.entity(t))
            .filter(|t| !t.flag("logseq.property/built-in?"))
            .filter_map(|t| t.str("block/title").map(str::to_string))
            .collect()
    }

    // --- outline ---------------------------------------------------------

    /// Rebuild the page's tree: siblings ordered by `:block/order`
    /// (fractional-index strings, compared byte-wise), a block whose
    /// parent is unknown hangs off the page root.
    fn build_tree(
        &mut self,
        page: i64,
        block_ids: &[i64],
        label: &str,
        report: &mut ImportReport,
    ) -> Vec<ImportBlock> {
        let members: HashSet<i64> = block_ids.iter().copied().collect();
        let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
        for id in block_ids {
            let parent = self.g.entity(*id).and_then(|e| e.int("block/parent"));
            let key = match parent {
                Some(p) if p == page || members.contains(&p) => p,
                _ => {
                    self.tally.reparented_blocks += 1;
                    page
                }
            };
            kids.entry(key).or_default().push(*id);
        }
        for list in kids.values_mut() {
            list.sort_by(|a, b| self.order_key(*a).cmp(self.order_key(*b)).then(a.cmp(b)));
        }
        let mut visited: HashSet<i64> = HashSet::new();
        let mut roots: Vec<ImportBlock> = kids
            .get(&page)
            .map(|ids| self.convert_siblings(ids, &kids, &mut visited, label, report))
            .unwrap_or_default();
        // A parent cycle never reaches the root; keep those blocks
        // rather than lose them.
        for id in block_ids {
            if !visited.contains(id) {
                self.tally.cyclic_blocks += 1;
                roots.push(self.convert_node(*id, &kids, &mut visited, label, report));
            }
        }
        roots
    }

    fn order_key(&self, id: i64) -> &str {
        self.g
            .entity(id)
            .and_then(|e| e.str("block/order"))
            .unwrap_or("")
    }

    fn convert_siblings(
        &mut self,
        ids: &[i64],
        kids: &HashMap<i64, Vec<i64>>,
        visited: &mut HashSet<i64>,
        label: &str,
        report: &mut ImportReport,
    ) -> Vec<ImportBlock> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if !visited.contains(id) {
                out.push(self.convert_node(*id, kids, visited, label, report));
            }
        }
        out
    }

    fn convert_node(
        &mut self,
        id: i64,
        kids: &HashMap<i64, Vec<i64>>,
        visited: &mut HashSet<i64>,
        label: &str,
        report: &mut ImportReport,
    ) -> ImportBlock {
        visited.insert(id);
        let ent = self.g.entity(id).expect("block ids come from the graph");
        let mut block = self.convert_block(ent, label, report);
        if let Some(ids) = kids.get(&id) {
            block.children = self.convert_siblings(ids, kids, visited, label, report);
        }
        block
    }

    // --- aggregate warnings ----------------------------------------------

    fn finish(&mut self, report: &mut ImportReport) {
        let t = &self.tally;
        let mut skipped: Vec<String> = Vec::new();
        for (n, what) in [
            (t.built_in_classes, "built-in tags/classes"),
            (t.property_defs, "property definitions"),
            (t.closed_values, "closed-value definitions"),
            (
                t.value_blocks,
                "property value blocks (folded into their properties)",
            ),
            (t.recycled_blocks, "blocks in the recycle bin"),
            (t.hidden_blocks, "hidden blocks"),
        ] {
            if n > 0 {
                skipped.push(format!("{n} {what}"));
            }
        }
        if !skipped.is_empty() {
            report.warn(
                GRAPH_SCOPE,
                format!(
                    "not imported (not page or block content): {}",
                    skipped.join(", ")
                ),
            );
        }
        if t.reparented_blocks > 0 || t.cyclic_blocks > 0 {
            report.warn(
                GRAPH_SCOPE,
                format!(
                    "{} block(s) had a parent outside their page and {} sat in a parent \
                     cycle; they were attached to the page root",
                    t.reparented_blocks, t.cyclic_blocks
                ),
            );
        }
        if !t.stripped.is_empty() {
            let total: usize = t.stripped.values().sum();
            report.artifacts_stripped += total;
            let list: Vec<String> = t
                .stripped
                .iter()
                .map(|(k, v)| format!("{k} ×{v}"))
                .collect();
            report.warn(
                GRAPH_SCOPE,
                format!(
                    "{total} attribute value(s) have no outl equivalent and were dropped: {}",
                    list.join(", ")
                ),
            );
        }
        if !t.dangling.is_empty() {
            report.refs_unresolved += t.dangling.len();
            let examples: Vec<String> = t
                .dangling
                .iter()
                .take(DANGLING_EXAMPLES)
                .map(|(page, uuid)| format!("{uuid} (on {page})"))
                .collect();
            report.warn(
                GRAPH_SCOPE,
                format!(
                    "{} reference(s) point at a uuid that is not in the import (a deleted, \
                     hidden or skipped node); the text is kept verbatim, e.g. {}",
                    t.dangling.len(),
                    examples.join(", ")
                ),
            );
        }
    }
}

/// `:block/journal-day` (`yyyymmdd`) → date.
fn journal_date(ent: &Entity) -> Option<NaiveDate> {
    let day = ent.int("block/journal-day")?;
    NaiveDate::from_ymd_opt(
        i32::try_from(day / 10_000).ok()?,
        u32::try_from(day / 100 % 100).ok()?,
        u32::try_from(day % 100).ok()?,
    )
}
