//! Source adapters. One module per dialect.

pub(crate) mod asset_scan;
pub mod logseq;
pub mod logseq_db;
pub mod obsidian;
pub mod roam;
pub(crate) mod scan;

pub use logseq::LogseqAdapter;
pub use logseq_db::LogseqDbAdapter;
pub use obsidian::ObsidianAdapter;
pub use roam::RoamAdapter;
