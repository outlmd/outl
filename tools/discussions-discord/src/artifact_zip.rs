//! A tiny reader for the one-file zip an artifact download returns.
//!
//! Not the `zip` crate: the artifact holds a single small `thread.json`, so
//! reading the central directory and inflating one entry with `flate2` (already
//! in the lockfile) is all this job needs, and keeps its build short.
//!
//! The central directory is read rather than the local header on purpose:
//! upload-artifact streams its zips, which sets the "data descriptor" flag and
//! leaves the sizes in the local header at zero.
//! Methods 0 (stored) and 8 (deflate) are supported, zip64 and encryption are not.
//! The CRC is not checked: the file came over TLS from the Actions API and its
//! content is validated as a mapping right after.
//!
//! A body that does not start with the zip signature is taken to be the JSON
//! itself, which is what a download of a non-zipped artifact looks like.

use std::io::Read;

use flate2::read::DeflateDecoder;

use crate::error::{Result, SyncError};

const LOCAL_SIG: &[u8] = b"PK\x03\x04";
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
const END_LEN: usize = 22;
/// A mapping is a few dozen bytes; this only bounds a hostile deflate stream.
const MAX_UNPACKED: u64 = 1 << 20;

fn bad(what: &str) -> SyncError {
    SyncError::new(format!("artifact zip is malformed: {what}"))
}

fn u16_at(data: &[u8], at: usize) -> Result<usize> {
    let bytes = data.get(at..at + 2).ok_or_else(|| bad("truncated"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
}

fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    let bytes = data.get(at..at + 4).ok_or_else(|| bad("truncated"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// The bytes of `entry` inside `data`, or `data` itself when it is not a zip.
pub fn read_entry(data: &[u8], entry: &str) -> Result<Vec<u8>> {
    if !data.starts_with(LOCAL_SIG) {
        return Ok(data.to_vec());
    }
    let end = find_end(data)?;
    let entries = u16_at(data, end + 10)?;
    let mut at = u32_at(data, end + 16)? as usize;
    for _ in 0..entries {
        if u32_at(data, at)? != CENTRAL_SIG {
            return Err(bad("bad central directory entry"));
        }
        let method = u16_at(data, at + 10)?;
        let packed = u32_at(data, at + 20)? as usize;
        let name_len = u16_at(data, at + 28)?;
        let extra_len = u16_at(data, at + 30)?;
        let comment_len = u16_at(data, at + 32)?;
        let local = u32_at(data, at + 42)? as usize;
        let name = data
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| bad("truncated"))?;
        if name == entry.as_bytes() {
            return unpack(data, local, method, packed);
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    Err(SyncError::new(format!("artifact has no {entry}")))
}

fn find_end(data: &[u8]) -> Result<usize> {
    if data.len() < END_LEN {
        return Err(bad("truncated"));
    }
    // The end record may be followed by a comment of up to 64 KiB.
    let lowest = data.len().saturating_sub(END_LEN + 0xFFFF);
    (lowest..=data.len() - END_LEN)
        .rev()
        .find(|&at| u32_at(data, at).is_ok_and(|sig| sig == END_SIG))
        .ok_or_else(|| bad("no end of central directory"))
}

fn unpack(data: &[u8], local: usize, method: usize, packed: usize) -> Result<Vec<u8>> {
    if data.get(local..local + 4) != Some(LOCAL_SIG) {
        return Err(bad("bad local header"));
    }
    let start = local + 30 + u16_at(data, local + 26)? + u16_at(data, local + 28)?;
    let raw = data
        .get(start..start + packed)
        .ok_or_else(|| bad("entry data is truncated"))?;
    match method {
        0 => Ok(raw.to_vec()),
        8 => {
            let mut out = Vec::new();
            let mut decoder = DeflateDecoder::new(raw).take(MAX_UNPACKED + 1);
            decoder
                .read_to_end(&mut out)
                .map_err(|err| bad(&format!("cannot inflate: {err}")))?;
            if out.len() as u64 > MAX_UNPACKED {
                return Err(bad("entry is too large"));
            }
            Ok(out)
        }
        other => Err(SyncError::new(format!(
            "artifact zip uses compression method {other}, only 0 and 8 are supported"
        ))),
    }
}
