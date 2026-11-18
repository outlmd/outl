//! Reading one op out of a `.jsonl` at a byte offset the index already gave
//! us.
//!
//! Its own module because it has **two** callers that reach it from opposite
//! directions, and it is the one thing they share:
//!
//! - [`JsonlStorage::read_op_at`](super::JsonlStorage) — the instance
//!   method, which looks the offset up in the storage's own loaded index.
//! - [`JsonlStorage::read_ops_at_offsets`](super::JsonlStorage) — the
//!   associated function behind the sync sender, which already *has* the
//!   offset (the census handed it over) and must not open a storage to use
//!   it.
//!
//! The second is why the seek-and-parse lives here rather than inside the
//! `impl` block: duplicating it would mean two answers to "what does this
//! offset point at", and the cheap path exists precisely to avoid the open
//! that would have made them agree by accident.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::hlc::Hlc;
use crate::op::LogOp;

use super::parse_log_line;

/// Read the op identified by `ts` from `path`, seeking to `offset`.
///
/// `None` on any failure — a missing file, a seek past EOF, an unparseable
/// record, or a record whose ops do not include `ts`. The `ts` check is what
/// makes a stale offset a miss rather than a wrong op.
///
/// Both callers turn `None` into an error rather than an omission; see
/// `JsonlStorage::read_op_or_missing` for why a short read must never look
/// like a healthy one.
pub(super) fn read_op_at_offset(path: &Path, ts: Hlc, offset: u64) -> Option<LogOp> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    if line.is_empty() {
        return None;
    }
    // A glued line yields multiple ops; pick the one whose HLC matches
    // (same recovery semantics as the boot path).
    let ops = parse_log_line(line.trim()).ok()?;
    ops.into_iter().find(|o| o.ts == ts)
}
