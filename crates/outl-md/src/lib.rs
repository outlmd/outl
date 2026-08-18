//! # outl-md
//!
//! Markdown parsing, sidecar handling, and the 3-level matching algorithm.
//!
//! This crate is the boundary between the user-visible `.md` files and the
//! op log managed by [`outl_core`]. See `crates/outl-md/CLAUDE.md` and
//! `docs/markdown-format.md` for the dialect specification and the matching
//! algorithm.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod asset;
mod ast;
pub mod atomic;
pub mod block_index;
pub mod cursor;
pub mod diff;
pub mod emoji;
mod emphasis;
mod fence;
pub mod frontmatter;
pub mod index;
pub mod inline;
pub mod lang;
mod link;
pub mod matching;
pub mod outline_ops;
pub mod parse;
mod plain;
mod property;
pub mod reconcile;
mod reference;
pub mod remind;
pub mod render;
mod shortcode;
pub mod sidecar;
mod similarity;
pub mod slug;
pub mod tag;
mod token;
pub mod unlogged;
pub mod view;
pub mod wikilink;

pub use asset::{asset_rel_path, hash_bytes, is_asset_link, is_safe_asset_name, ASSETS_DIR};
pub use atomic::{read_for_rewrite, write_atomic};
pub use block_index::{BlockEntry, BlockIndex, BlockReference, IdentifiedNode};
pub use diff::{diff_to_ops, DiffPlan};
pub use emoji::{is_valid_shortcode, search as search_emoji, shortcode_to_unicode, EmojiHit};
pub use index::{PageEntry, WorkspaceIndex};
pub use inline::{
    byte_index_for_char, link_at_cursor, plain_text, ref_at_cursor, tokenize, tokenize_owned,
    InlineTok, InlineToken, RefTarget,
};
pub use matching::{match_blocks, Match, MatchLevel};
pub use parse::{parse, OutlineNode, ParseWarning, ParseWarningKind, ParsedPage};
pub use reconcile::{
    reconcile_dir, reconcile_md, reconcile_md_with_guard, ReconcileError, ReconcileReport,
};
pub use remind::{
    parse_remind, rule_from_properties, RemindAnchor, RemindParse, RemindRule, RemindStop,
    MAX_FIRES_CAP, MIN_INTERVAL_MINUTES, REMIND_KEY,
};
pub use render::render;
pub use sidecar::{
    content_hash, file_hash, resolve_sidecar_path, sidecar_path_for, Sidecar, SidecarBlock,
};
pub use slug::{slugify, UNTITLED_SLUG};
pub use tag::text_contains_tag;
pub use unlogged::content_lines_missing_from;
pub use view::{block_to_rows, char_to_line_col, BlockRow, BlockRowKind};
