//! Logseq DB-version adapter (EDN graph export).
//!
//! Logseq 2.x keeps a graph in SQLite (`db.sqlite`, one `kvs` table of
//! DataScript storage nodes). Reading that directly means decoding
//! transit and DataScript's B-tree storage, which is internal and
//! unversioned. Logseq documents its EDN export as the only export that
//! fully captures a graph ("Export graph > Export EDN file"), so this
//! adapter reads that file instead.
//!
//! # The format (Logseq 2.0.2)
//!
//! - `src/main/frontend/handler/db_based/export.cljs`
//!   (`export-repo-as-db-edn!`) calls the db worker with
//!   `{:export-type :graph :graph-options {:include-timestamps? true}}`
//!   and pretty-prints the result.
//! - `deps/db/src/logseq/db/sqlite/export.cljs` (`build-export`):
//!   `:graph` maps to `build-graph-datoms-export`, a map with
//!   `:logseq.db.sqlite.export/schema-version`,
//!   `:logseq.db.sqlite.export/graph-format :datoms`, `:datoms` (a
//!   vector of `[entity-id attribute value]` triples sorted by entity
//!   id, lookup-ref values already turned into entity ids) and
//!   `:logseq.db.sqlite.export/export-type :graph`. Some kv entities
//!   and attributes are excluded by the exporter.
//! - `deps/db/src/logseq/db/frontend/schema.cljs`: `:block/uuid`,
//!   `:block/parent`, `:block/page`, `:block/order`,
//!   `:block/collapsed?`, `:block/tags`, `:block/alias`,
//!   `:block/created-at`, `:block/updated-at`, `:block/title`,
//!   `:block/journal-day`.
//! - `deps/db/src/logseq/db/frontend/content.cljs`: references inside
//!   `:block/title` are stored as `[[<uuid>]]`, tags as `#[[<uuid>]]`.
//! - `property.cljs`, `class.cljs`, `property/type.cljs`,
//!   `entity_util.cljs` (same directory): built-in properties and
//!   classes, and how a property value is stored per type.
//!
//! # What is read
//!
//! Pages are entities tagged `:logseq.class/Page`, `Journal` or `Tag`
//! (user tags become named pages) plus anything blocks point at through
//! `:block/page`; blocks are rebuilt from `:block/parent`, siblings
//! ordered by `:block/order` (fractional-index strings, compared
//! byte-wise). Task status/priority/deadline/scheduled, code blocks,
//! user properties and references are translated; whatever cannot be is
//! counted in the report. See `docs/import.md` for the user-facing
//! mapping.

mod block;
mod convert;
mod edn;
mod graph;
mod props;
mod text;

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod reject_tests;
#[cfg(test)]
mod tests;

use crate::adapter::{ImportError, SourceAdapter};
use crate::ir::ImportGraph;
use crate::report::ImportReport;
use std::path::Path;

/// Schema major the importer was checked against (`65.34` at tag 2.0.2).
const KNOWN_SCHEMA_MAJOR: i64 = 65;

/// The Logseq DB-graph adapter, reading the EDN graph export.
pub struct LogseqDbAdapter;

impl SourceAdapter for LogseqDbAdapter {
    fn id(&self) -> &'static str {
        "logseq-db"
    }

    /// An `.edn` file whose envelope says `graph-format :datoms`.
    fn detect(src: &Path) -> bool {
        src.is_file()
            && src.extension().and_then(|e| e.to_str()) == Some("edn")
            && std::fs::read_to_string(src).is_ok_and(|t| looks_like_datoms_export(&t))
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        if src.is_dir() {
            let msg = if is_db_graph_dir(src) {
                db_graph_dir_message(src)
            } else {
                format!(
                    "logseq-db source must be the EDN file from \"Export graph > Export EDN \
                     file\" (got the directory {})",
                    src.display()
                )
            };
            return Err(ImportError::Parse(msg));
        }
        let text = std::fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;
        let root =
            edn::parse(&text).map_err(|e| ImportError::Parse(format!("{}: {e}", src.display())))?;
        let graph = graph::Graph::from_edn(&root)?;
        if let Some(v) = graph::schema_version(&root) {
            let major = v.split('.').next().and_then(|m| m.parse::<i64>().ok());
            if major != Some(KNOWN_SCHEMA_MAJOR) {
                report.warn(
                    "(graph)",
                    format!(
                        "export schema version {v}; this importer was checked against \
                         {KNOWN_SCHEMA_MAJOR}.x, so some attributes may be misread"
                    ),
                );
            }
        }
        Ok(convert::convert(&graph, report))
    }
}

/// Cheap check used by `auto`: the envelope key followed by `:datoms`.
/// The export is one pretty-printed map, so a scan for the key is
/// enough; [`graph::Graph::from_edn`] does the strict validation.
fn looks_like_datoms_export(text: &str) -> bool {
    const KEY: &str = ":logseq.db.sqlite.export/graph-format";
    text.find(KEY).is_some_and(|i| {
        text[i + KEY.len()..]
            .trim_start_matches(|c: char| c.is_whitespace() || c == ',')
            .starts_with(":datoms")
    })
}

/// A Logseq DB graph directory: `db.sqlite` and none of the file-graph
/// layout (`pages/` + `journals/`).
pub fn is_db_graph_dir(src: &Path) -> bool {
    src.is_dir()
        && src.join("db.sqlite").is_file()
        && !(src.join("pages").is_dir() && src.join("journals").is_dir())
}

/// The error for a DB graph directory, shared by the `logseq`,
/// `logseq-db` and `auto` entry points so the guidance never drifts.
pub fn db_graph_dir_message(src: &Path) -> String {
    format!(
        "{} is a Logseq DB graph (it holds db.sqlite and no pages/ or journals/), which cannot \
         be read directly yet. In Logseq open the graph and use \"Export graph > Export EDN \
         file\", then run `outl import logseq-db <export.edn> <dst>` on that file",
        src.display()
    )
}
