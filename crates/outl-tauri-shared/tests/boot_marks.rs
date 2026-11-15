//! The launch milestone names, pinned across the Rust/TS boundary.
//!
//! `BOOT_MARKS` is a **wire contract** and an unusually quiet one. The
//! frontend sends a string; the backend logs it, matches two of them to
//! release the interactive phase, and a CI budget checker greps for
//! them. Nothing in any of those steps is typed, so a client that
//! spells `firstContent` compiles, ships, and simply never releases the
//! gate — which looks like "sync is slow on this machine", four seconds
//! at a time.
//!
//! This is deliberately **not** in `wire_mirrors.rs`. That file pins
//! serialized *shapes* through `assert_wire_shape`, and a `&[&str]` has
//! no shape to serialize; adding `boot/index.ts` to
//! `ts_parser::MIRROR_FILES` would also hand the declaration-coverage
//! gate a `type BootMark` it cannot pin. The honest form is to read the
//! array.

use std::path::PathBuf;

use outl_tauri_shared::commands::boot::BOOT_MARKS;

/// The TypeScript mirror.
const MIRROR: &str = "../outl-frontend-shared/src/boot/index.ts";

/// The string literals of `export const BOOT_MARKS = [...] as const`.
///
/// A deliberately small parser: it takes the text between the
/// declaration and its closing bracket and collects every
/// double-quoted run. A mark name with a quote in it would break this,
/// and a mark name with a quote in it should break something.
fn mirror_marks() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(MIRROR);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let start = src
        .find("export const BOOT_MARKS")
        .expect("@outl/shared/boot no longer exports BOOT_MARKS");
    let open = src[start..]
        .find('[')
        .expect("BOOT_MARKS is not an array literal");
    let rest = &src[start + open..];
    let close = rest.find(']').expect("BOOT_MARKS array is not closed");
    rest[..close]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn the_shared_frontend_knows_the_same_marks_in_the_same_order() {
    let mirror = mirror_marks();
    let rust: Vec<String> = BOOT_MARKS.iter().map(|m| (*m).to_string()).collect();
    assert_eq!(
        mirror, rust,
        "@outl/shared/boot's BOOT_MARKS drifted from outl_tauri_shared's"
    );
}
