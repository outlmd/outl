use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::artifact_zip::read_entry;

fn le16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// A one-entry zip. `descriptor` mimics a streaming writer: flag bit 3 set and
/// the sizes in the local header left at zero.
fn build(name: &str, method: u16, packed: &[u8], unpacked: usize, descriptor: bool) -> Vec<u8> {
    let flags: u16 = if descriptor { 8 } else { 0 };
    let (local_packed, local_unpacked) = if descriptor {
        (0, 0)
    } else {
        (packed.len() as u32, unpacked as u32)
    };
    let mut out = Vec::new();
    out.extend_from_slice(b"PK\x03\x04");
    le16(&mut out, 20);
    le16(&mut out, flags);
    le16(&mut out, method);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le32(&mut out, 0);
    le32(&mut out, local_packed);
    le32(&mut out, local_unpacked);
    le16(&mut out, name.len() as u16);
    le16(&mut out, 0);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(packed);
    if descriptor {
        out.extend_from_slice(b"PK\x07\x08");
        le32(&mut out, 0);
        le32(&mut out, packed.len() as u32);
        le32(&mut out, unpacked as u32);
    }
    let cd_start = out.len();
    out.extend_from_slice(b"PK\x01\x02");
    le16(&mut out, 20);
    le16(&mut out, 20);
    le16(&mut out, flags);
    le16(&mut out, method);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le32(&mut out, 0);
    le32(&mut out, packed.len() as u32);
    le32(&mut out, unpacked as u32);
    le16(&mut out, name.len() as u16);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le32(&mut out, 0);
    le32(&mut out, 0);
    out.extend_from_slice(name.as_bytes());
    let cd_len = out.len() - cd_start;
    out.extend_from_slice(b"PK\x05\x06");
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, 1);
    le16(&mut out, 1);
    le32(&mut out, cd_len as u32);
    le32(&mut out, cd_start as u32);
    le16(&mut out, 0);
    out
}

pub fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
    build(name, 0, data, data.len(), false)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).expect("write");
    enc.finish().expect("finish")
}

const JSON: &[u8] = br#"{"discussion": 42, "thread_id": "555000111222333444"}"#;

#[test]
fn reads_a_stored_entry() {
    let zip = stored_zip("thread.json", JSON);
    assert_eq!(read_entry(&zip, "thread.json").expect("read"), JSON);
}

#[test]
fn reads_a_deflated_entry() {
    let zip = build("thread.json", 8, &deflate(JSON), JSON.len(), false);
    assert_eq!(read_entry(&zip, "thread.json").expect("read"), JSON);
}

#[test]
fn reads_a_streamed_entry_whose_local_header_has_no_sizes() {
    let zip = build("thread.json", 8, &deflate(JSON), JSON.len(), true);
    assert_eq!(read_entry(&zip, "thread.json").expect("read"), JSON);
}

#[test]
fn raw_json_is_returned_as_is() {
    assert_eq!(read_entry(JSON, "thread.json").expect("read"), JSON);
}

#[test]
fn a_missing_entry_is_an_error() {
    let zip = stored_zip("other.json", JSON);
    let err = read_entry(&zip, "thread.json").expect_err("no entry");
    assert!(err.to_string().contains("no thread.json"), "{err}");
}

#[test]
fn an_unsupported_method_is_an_error() {
    let zip = build("thread.json", 12, JSON, JSON.len(), false);
    let err = read_entry(&zip, "thread.json").expect_err("bzip2");
    assert!(err.to_string().contains("method 12"), "{err}");
}

#[test]
fn truncated_and_garbled_zips_are_errors_not_panics() {
    let zip = stored_zip("thread.json", JSON);
    for cut in [4, 10, 40, zip.len() - 1, zip.len() - 22] {
        assert!(read_entry(&zip[..cut], "thread.json").is_err(), "cut {cut}");
    }
    let mut broken = zip.clone();
    let at = broken.len() - 6;
    broken[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_entry(&broken, "thread.json").is_err());
}

#[test]
fn a_deflate_bomb_is_refused() {
    let big = vec![b'a'; 2 << 20];
    let zip = build("thread.json", 8, &deflate(&big), big.len(), false);
    let err = read_entry(&zip, "thread.json").expect_err("too large");
    assert!(err.to_string().contains("too large"), "{err}");
}
