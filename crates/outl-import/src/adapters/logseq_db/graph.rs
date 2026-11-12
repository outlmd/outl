//! Datoms → entity table.
//!
//! The export (`export-type :graph`) is a flat vector of `[e a v]`
//! triples sorted by entity id. This module validates the envelope and
//! groups the triples per entity so the converter can ask "what are
//! this entity's tags / parent / title" without re-scanning the vector.
//!
//! Cardinality-many attributes simply repeat (`[5 :block/tags 12]`,
//! `[5 :block/tags 13]`), so an entity keeps every value in order.

use super::edn::Edn;
use crate::adapter::ImportError;
use std::collections::{BTreeMap, HashMap};

/// Namespaced keys of the export envelope.
const FORMAT_KEY: &str = "logseq.db.sqlite.export/graph-format";
const EXPORT_TYPE_KEY: &str = "logseq.db.sqlite.export/export-type";
const SCHEMA_KEY: &str = "logseq.db.sqlite.export/schema-version";

/// One entity: every datom that shares an entity id.
#[derive(Debug, Default)]
pub(super) struct Entity {
    attrs: Vec<(String, Edn)>,
}

impl Entity {
    /// First value of `attr`.
    pub(super) fn get(&self, attr: &str) -> Option<&Edn> {
        self.attrs.iter().find(|(a, _)| a == attr).map(|(_, v)| v)
    }

    /// Every value of `attr` (card-many attributes repeat).
    pub(super) fn all<'a>(&'a self, attr: &'a str) -> impl Iterator<Item = &'a Edn> + 'a {
        self.attrs
            .iter()
            .filter(move |(a, _)| a == attr)
            .map(|(_, v)| v)
    }

    /// Every `(attribute, value)` pair, in datom order.
    pub(super) fn attrs(&self) -> &[(String, Edn)] {
        &self.attrs
    }

    /// String value of `attr`.
    pub(super) fn str(&self, attr: &str) -> Option<&str> {
        self.get(attr).and_then(Edn::as_str)
    }

    /// Integer value of `attr` (entity ref, timestamp, order, …).
    pub(super) fn int(&self, attr: &str) -> Option<i64> {
        self.get(attr).and_then(Edn::as_int)
    }

    /// Entity ids referenced by every value of `attr`.
    pub(super) fn refs(&self, attr: &str) -> Vec<i64> {
        self.all(attr).filter_map(Edn::as_int).collect()
    }

    /// `true` only when `attr` is the boolean `true`.
    pub(super) fn flag(&self, attr: &str) -> bool {
        matches!(self.get(attr), Some(Edn::Bool(true)))
    }

    /// `:db/ident`, without the leading colon.
    pub(super) fn ident(&self) -> Option<&str> {
        self.get("db/ident").and_then(Edn::as_keyword)
    }

    /// `:block/uuid` as a lowercase string.
    pub(super) fn uuid(&self) -> Option<String> {
        self.get("block/uuid").and_then(uuid_string)
    }
}

/// `#uuid "…"` (or a bare string) → lowercase uuid text.
fn uuid_string(v: &Edn) -> Option<String> {
    match v {
        Edn::Tagged(tag, inner) if tag == "uuid" => inner.as_str().map(str::to_ascii_lowercase),
        Edn::Str(s) => Some(s.to_ascii_lowercase()),
        _ => None,
    }
}

/// The whole export, indexed.
#[derive(Debug, Default)]
pub(super) struct Graph {
    entities: BTreeMap<i64, Entity>,
    idents: HashMap<String, i64>,
    by_uuid: HashMap<String, i64>,
    /// Number of `[e a v]` triples read.
    pub(super) datom_count: usize,
}

impl Graph {
    /// Validate the envelope and group the datoms by entity.
    pub(super) fn from_edn(root: &Edn) -> Result<Self, ImportError> {
        check_envelope(root)?;
        let datoms = root
            .map_get("datoms")
            .and_then(Edn::as_seq)
            .ok_or_else(|| bad("the export has no `:datoms` vector"))?;
        let mut graph = Graph::default();
        for (i, d) in datoms.iter().enumerate() {
            let triple = d.as_seq().filter(|t| t.len() == 3).ok_or_else(|| {
                bad(&format!("datom #{i} is not an `[entity attribute value]` triple"))
            })?;
            let e = triple[0]
                .as_int()
                .ok_or_else(|| bad(&format!("datom #{i} has a non-integer entity id")))?;
            let a = triple[1]
                .as_keyword()
                .ok_or_else(|| bad(&format!("datom #{i} has a non-keyword attribute")))?;
            graph
                .entities
                .entry(e)
                .or_default()
                .attrs
                .push((a.to_string(), triple[2].clone()));
        }
        graph.datom_count = datoms.len();
        graph.index();
        Ok(graph)
    }

    fn index(&mut self) {
        for (id, ent) in &self.entities {
            if let Some(ident) = ent.ident() {
                self.idents.insert(ident.to_string(), *id);
            }
            if let Some(uuid) = ent.uuid() {
                self.by_uuid.insert(uuid, *id);
            }
        }
    }

    /// Entity by id.
    pub(super) fn entity(&self, id: i64) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// Every entity, ordered by id.
    pub(super) fn entities(&self) -> impl Iterator<Item = (i64, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
    }

    /// Entity id of a `:db/ident` (`logseq.class/Page`).
    pub(super) fn ident_id(&self, ident: &str) -> Option<i64> {
        self.idents.get(ident).copied()
    }
}

fn bad(msg: &str) -> ImportError {
    ImportError::Parse(format!("logseq-db export: {msg}"))
}

/// Refuse anything that is not the `:graph` datoms export, naming the
/// shape that was found so the user can re-export.
fn check_envelope(root: &Edn) -> Result<(), ImportError> {
    if !matches!(root, Edn::Map(_)) {
        return Err(bad(
            "the file is not an EDN map — expected the graph export written by Logseq's \
             \"Export graph > Export EDN file\"",
        ));
    }
    let format = root.map_get(FORMAT_KEY).and_then(Edn::as_keyword);
    if format == Some("datoms") {
        return Ok(());
    }
    let kind = root
        .map_get(EXPORT_TYPE_KEY)
        .and_then(Edn::as_keyword)
        .map(|k| format!("export-type :{k}"));
    let found = match (format, kind) {
        (Some(f), _) => format!("graph-format :{f}"),
        (None, Some(k)) => k,
        (None, None) => "no `graph-format` key".to_string(),
    };
    Err(bad(&format!(
        "unsupported export shape ({found}). Only the whole-graph datoms export \
         (`:logseq.db.sqlite.export/graph-format :datoms`, from \"Export graph > Export \
         EDN file\") is supported; page/block exports and the human-readable graph \
         export are not"
    )))
}

/// Schema version the export was written with, e.g. `65.34`, for the
/// report. `None` when absent or in an unexpected shape.
pub(super) fn schema_version(root: &Edn) -> Option<String> {
    let v = root.map_get(SCHEMA_KEY)?;
    let major = v.map_get("major")?.as_int()?;
    Some(match v.map_get("minor").and_then(Edn::as_int) {
        Some(minor) => format!("{major}.{minor}"),
        None => major.to_string(),
    })
}
