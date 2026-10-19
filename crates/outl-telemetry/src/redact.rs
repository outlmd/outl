//! The one predicate deciding whether a string may leave the user's machine.
//!
//! A panic message is the only free-form field a crash report carries, and it
//! is the only real hazard in this crate. `expect("explicit reason")` is ours
//! and safe; `expect(&format!("page {slug}"))` is also ours and is a page
//! title. Everything that could be a path or a page identifier is replaced
//! here before anything is sent.
//!
//! The failure directions are not symmetric, which is why this file has more
//! tests than code. An over-cautious scrubber drops a stack frame the author
//! can *notice* missing. An over-permissive one publishes a page title, and
//! nobody can notice a leak that already happened. When the two trade off,
//! this file picks over-cautious.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// What replaces anything that looked like a filesystem path.
pub const PATH_MARKER: &str = "<path>";

/// What replaces a bare `.md` / `.outl` filename, which is a page title.
pub const FILE_MARKER: &str = "<file>";

/// What replaces a string that a `std` slicing panic quoted back at us, which
/// for outl is block text.
pub const TEXT_MARKER: &str = "<text>";

/// What replaces a write-actor id.
///
/// The actor ULID is the one identifier in outl that persists and is stable
/// per device, which is exactly what RFC 0377 §3 promises a report never
/// carries: two reports sharing one would be tie-able to one person.
pub const ID_MARKER: &str = "<id>";

/// Replaces every filesystem path and page identifier in `text` with a marker.
///
/// Six rules, applied in order:
///
/// 0. A string that one of `std`'s own slicing panics quoted back at us is cut
///    at the quote and becomes [`TEXT_MARKER`]. See below: this is the only
///    rule that exists because of code we did not write.
/// 1. A Windows path (`C:\Users\...`, `D:/notes/...`) or a UNC path
///    (`\\server\share\...`) becomes [`PATH_MARKER`].
/// 2. An absolute POSIX path (`/Users/...`, `/home/...`, `/data/user/0/...`,
///    `/var/mobile/Containers/...`) or a `~`-rooted one becomes [`PATH_MARKER`].
/// 3. Whatever continues a path *after* a space becomes [`PATH_MARKER`] too —
///    rules 1 and 2 stop at whitespace, and what follows is the rest of
///    somebody's home directory.
/// 4. A filename ending in `.md`, `.outl` or `.jsonl`, with any relative
///    directories in front of it, becomes [`FILE_MARKER`].
/// 5. A bare write-actor id becomes [`ID_MARKER`].
///
/// A *relative* source path survives, because the panic location we want
/// (`crates/outl-core/src/tree/mod.rs:42`) is one: it is fixed at compile time
/// and is repo-relative, so it describes our source tree and not the user's
/// disk. Rule 4 is what keeps that from being a loophole — `.rs` is not in its
/// extension set, so our location stays and a page goes.
///
/// # Rule 0, and why `std` is an author of free-form strings
///
/// This file used to assume the only author of a panic message was us. It is
/// not: `str` slicing panics are formatted by `core`, and under the toolchain
/// this repo pins they **quote the string being sliced**.
///
/// ```text
/// end byte index 16 is not a char boundary; it is inside 'ã' (bytes 15..17)
///   of `nota sobre o João: pressão 14/9`
/// ```
///
/// `outl-md` slices block text by byte offset in `cursor.rs`, `wikilink.rs`
/// and four other files, so an off-by-one on an accented line — the exact bug
/// crash reporting exists to catch — would have published up to 256 bytes of a
/// note. Rule 0 cuts the quote off and keeps the diagnosis.
///
/// # What still gets through, stated so it is not rediscovered from a report
///
/// A path component that contains a space and is followed by no separator
/// (`/Users/x/My Secret Notes` leaves `Secret`) survives as an isolated word.
/// Rules 3 and 4 close the cases that leak a username, a workspace folder or
/// an actor id; a single word of a user-chosen folder name is what is left,
/// and closing it would need to know where a space-containing path ends,
/// which no pattern can decide.
///
/// A bare slug with no path and no extension (`page "terapia" not found`)
/// cannot be caught at all. That is the hazard RFC 0377 §3 names.
pub fn redact(text: &str) -> String {
    // Rule 0 first, because it *truncates* rather than replaces: everything
    // after the quote is a string we never wrote, so no later rule should be
    // asked to make it safe one path at a time.
    let text = cut_quoted_string(text);
    // Windows first: a `D:/notes/x.md` handled by rule 2 would leave the drive
    // letter dangling in front of the marker.
    let text = WINDOWS_PATH.replace_all(&text, PATH_MARKER);
    let text = POSIX_PATH.replace_all(&text, |caps: &Captures<'_>| {
        // Group 1 is the character that proved the path starts here, and it is
        // put back. `regex` has no lookbehind, and without this boundary the
        // pattern would match the `/outl-core/src/lib.rs` inside a relative
        // path and eat the panic location.
        format!("{}{PATH_MARKER}", &caps[1])
    });
    // After the marker, not before it: rules 1 and 2 stop at the first space,
    // and `C:\Users\Joao Silva\AppData\...` leaves a surname and a home
    // directory standing. Anchoring on the marker is what keeps this from
    // eating our own relative panic location, which never follows one.
    let text = PATH_TAIL.replace_all(&text, PATH_MARKER);
    let text = PAGE_FILE.replace_all(&text, FILE_MARKER);
    ACTOR_ID.replace_all(&text, ID_MARKER).into_owned()
}

/// Rule 0: cut a `std` slicing panic at the string it quoted.
///
/// Keeps the diagnosis (`end byte index 16 is not a char boundary`) and drops
/// the quote, because the quote is the user's note. Returns the input
/// untouched when no marker is present, which is every message we write
/// ourselves.
fn cut_quoted_string(text: &str) -> Cow<'_, str> {
    let cut = QUOTING_PANICS
        .iter()
        .filter_map(|marker| text.find(marker).map(|at| at + marker.len()))
        .min();
    match cut {
        Some(at) => Cow::Owned(format!("{} {TEXT_MARKER}", text[..at].trim_end())),
        None => Cow::Borrowed(text),
    }
}

/// The `core` messages that quote the string they failed on, measured against
/// the pinned toolchain rather than taken from memory. Each is kept up to the
/// end of the marker, so the sentence still reads.
///
/// A later toolchain reworded the first one to `… of string` and stopped
/// quoting; this list is for the one we ship, and an extra marker that no
/// longer fires costs nothing.
const QUOTING_PANICS: [&str; 3] = [
    "is not a char boundary",
    "is out of bounds of ",
    "when slicing ",
];

/// A drive letter or a UNC prefix, a separator, then anything up to
/// whitespace.
///
/// UNC is not decoration: `transport = "file"` over a shared filesystem is a
/// documented setup, and on Windows that is `\\server\share\...`. Neither of
/// the other path rules can match it — one needs a drive letter and the other
/// needs a leading `/` — so without this branch a crash from that setup
/// published the file server's hostname, which the promise names directly.
static WINDOWS_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\b[A-Za-z]:[\\/]|\\\\)[^\s"'<>|]*"#)
        .expect("a literal pattern, checked by the tests")
});

/// The rest of a path that rule 1 or 2 abandoned at a space.
///
/// Anchored on the marker the earlier rule just wrote, and continuing only
/// through whitespace-separated runs that still carry a separator. That is
/// what distinguishes `<path> Silva\AppData\Local` (more path) from
/// `<path> while reading` (prose), and it is why this cannot swallow the
/// relative panic location: that location never appears directly after a
/// marker.
static PATH_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"{}(?:\s+[^\s"'<>|]*[\\/][^\s"'<>|]*)+"#,
        regex::escape(PATH_MARKER)
    ))
    .expect("a pattern built from a literal marker, checked by the tests")
});

/// A write-actor id: 26 Crockford base32 characters, which excludes `I`, `L`,
/// `O` and `U`.
///
/// Uppercase-only, so it cannot collide with a lowercase path segment.
static ACTOR_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b[0-9A-HJKMNP-TV-Z]{26}\b"#).expect("a literal pattern, checked by the tests")
});

/// An optional `~`, then one or more `/`-prefixed segments.
///
/// The leading separator is `/+` rather than `/` so that `file:///Users/...`
/// and `https://host/path` are caught too: a scheme is not a reason to let an
/// absolute path through.
static POSIX_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|[\s"'`(\[{=,;:])(~?(?:/+[^\s/"'`)\]}<>|,;]+)+/*)"#)
        .expect("a literal pattern, checked by the tests")
});

/// A file ending in `.md`, `.outl` or `.jsonl`, together with any relative
/// directories in front of it.
///
/// The separators are **inside** the match, not excluded from it, because an
/// outl slug keeps `/` for hierarchy: `pages/buser/tech/data-platform.md`
/// without them became `pages/buser/tech/<file>`, and those two middle
/// segments are the page's namespace — page data under a different name.
///
/// `.jsonl` joins the set for `ops-<actor>.jsonl`, whose name carries the id
/// rule 5 also catches. `.rs` stays out, which is what lets the panic location
/// survive all of this.
static PAGE_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)[^\s"'`)\]}<>|,;]+\.(?:md|outl|jsonl)\b"#)
        .expect("a literal pattern, checked by the tests")
});

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Uppercase on purpose: every generated segment below is lowercase, so a
    /// generated name can never collide with the template and make an
    /// assertion pass or fail for the wrong reason.
    const PREFIX: &str = "PANIC AT ";
    const SUFFIX: &str = " -- NO SUCH FILE";

    #[test]
    fn a_posix_home_path_does_not_survive() {
        assert_eq!(
            redact("failed to open /Users/avelino/notes/terapia.md"),
            "failed to open <path>"
        );
        assert_eq!(redact("stat /home/avelino/outl-p2p"), "stat <path>");
    }

    #[test]
    fn a_windows_path_does_not_survive() {
        assert_eq!(
            redact(r"failed to open C:\Users\avelino\notes\terapia.md"),
            "failed to open <path>"
        );
        assert_eq!(redact("open D:/notes/terapia.md"), "open <path>");
    }

    /// iOS and Android hand the app an absolute container path, and both are
    /// ordinary POSIX paths — so rule 2 covers them. Pinned by name anyway,
    /// because "the mobile paths are covered" is the kind of claim that is
    /// true until somebody anchors the rule to `/Users|/home`.
    #[test]
    fn a_mobile_container_path_does_not_survive() {
        assert_eq!(
            redact("open /var/mobile/Containers/Data/Application/ABC-123/Documents/pages/x.md"),
            "open <path>"
        );
        assert_eq!(
            redact("open /data/user/0/app.outl/files/pages/terapia.md"),
            "open <path>"
        );
        assert_eq!(
            redact("open /storage/emulated/0/outl/pages/terapia.md"),
            "open <path>"
        );
    }

    #[test]
    fn a_tilde_path_does_not_survive() {
        assert_eq!(redact("reading ~/outl-p2p/pages/x.md"), "reading <path>");
    }

    /// The whole reason the panic location is worth sending: it is a
    /// compile-time, repo-relative path to *our* source, not the user's disk.
    #[test]
    fn the_relative_panic_location_survives() {
        assert_eq!(
            redact("crates/outl-core/src/tree/mod.rs:42:17"),
            "crates/outl-core/src/tree/mod.rs:42:17"
        );
        assert_eq!(
            redact("panicked at crates/outl-md/src/parse.rs:118"),
            "panicked at crates/outl-md/src/parse.rs:118"
        );
    }

    /// A relative page path goes **whole**, directories included.
    ///
    /// This used to assert `pages/<file>`, which looked like enough because
    /// the fixture was a one-level page. An outl slug keeps `/` for
    /// hierarchy, so for `buser/tech/data-platform` those surviving segments
    /// were the page's namespace.
    #[test]
    fn a_relative_page_path_does_not_survive() {
        assert_eq!(redact("pages/terapia.md not found"), "<file> not found");
        assert_eq!(redact("pages/terapia.outl is stale"), "<file> is stale");
        assert_eq!(
            redact("render pages/buser/tech/data-platform.md failed"),
            "render <file> failed"
        );
    }

    /// Rule 0. The three shapes were measured on the pinned toolchain, not
    /// recalled: `core` quotes the string it failed to slice, and for outl
    /// that string is a line of somebody's journal.
    #[test]
    fn a_quoted_string_does_not_survive_a_slicing_panic() {
        assert_eq!(
            redact(
                "end byte index 16 is not a char boundary; it is inside 'x' \
                 (bytes 15..17) of `nota sobre o Joao: pressao 14/9`"
            ),
            "end byte index 16 is not a char boundary <text>"
        );
        assert_eq!(
            redact("end byte index 900 is out of bounds of `pressao da Dona Maria`"),
            "end byte index 900 is out of bounds of <text>"
        );
        assert_eq!(
            redact("begin > end (20 > 3) when slicing `reuniao com o paciente`"),
            "begin > end (20 > 3) when slicing <text>"
        );
    }

    /// A Vec index panic reads similarly and quotes nothing, so rule 0 must
    /// leave it alone — otherwise the most common panic in the codebase
    /// arrives truncated.
    #[test]
    fn an_index_panic_is_not_mistaken_for_a_quoted_string() {
        let msg = "index out of bounds: the len is 3 but the index is 5";
        assert_eq!(redact(msg), msg);
    }

    /// Rule 1's UNC branch. `transport = "file"` over a shared filesystem is
    /// documented, and on Windows that is a hostname in the path.
    #[test]
    fn a_unc_path_does_not_survive() {
        assert_eq!(
            redact(r"failed to open \\avelino-macbook\share\notes\terapia.md"),
            "failed to open <path>"
        );
        assert_eq!(redact(r"stat \\AVELINO-PC\c$\outl"), "stat <path>");
    }

    /// Rule 3. Every one of these stopped at the first space before, leaving
    /// a surname, a workspace folder or an actor id in the report.
    #[test]
    fn a_path_with_a_space_does_not_leave_a_tail() {
        assert_eq!(
            redact(r"could not open C:\Users\Joao Silva\AppData\Local\outl\outl.exe"),
            "could not open <path>"
        );
        assert_eq!(
            redact(
                "failed to write ~/Library/Application Support/outl/actors/\
                 01JQZX8K9M4TRV2N7B3D5E6F7G.json"
            ),
            "failed to write <path>"
        );
        assert_eq!(
            redact("sync /Users/avelino/Library/Mobile Documents/com~apple~CloudDocs/outl/x.md"),
            "sync <path>"
        );
        assert_eq!(
            redact("lock held by /Users/avelino/Google Drive/Meu Diario/.outl/lock"),
            "lock held by <path>"
        );
    }

    /// Rule 3 must stop at prose, or it eats the half of the message that
    /// says where we were.
    ///
    /// The orphaned `b.md` becomes `<file>` rather than surviving: rule 3
    /// declines it (no separator, so it reads as prose) and rule 4 takes it
    /// for the page name it is. Both halves of the point are here — the
    /// user's leaf goes, our source location stays.
    #[test]
    fn a_path_tail_stops_at_prose() {
        assert_eq!(
            redact("could not read /Users/x/a b.md while in crates/outl-md/src/parse.rs:118"),
            "could not read <path> <file> while in crates/outl-md/src/parse.rs:118"
        );
        assert_eq!(
            redact("copied /tmp/notes to the trash"),
            "copied <path> to the trash"
        );
    }

    /// Rule 5. The actor id is the one identifier in outl that is stable per
    /// device, so it is the one that could tie two reports together.
    #[test]
    fn an_actor_id_does_not_survive() {
        assert_eq!(
            redact("unknown actor 01JQZX8K9M4TRV2N7B3D5E6F7G"),
            "unknown actor <id>"
        );
        assert_eq!(
            redact("corrupt op log at ops/ops-01JQZX8K9M4TRV2N7B3D5E6F7G.jsonl"),
            "corrupt op log at <file>"
        );
    }

    #[test]
    fn a_bare_page_filename_does_not_survive() {
        assert_eq!(redact("terapia.md is missing"), "<file> is missing");
        assert_eq!(redact("TERAPIA.MD is missing"), "<file> is missing");
    }

    #[test]
    fn an_ordinary_panic_message_is_left_alone() {
        let msg = "called `Option::unwrap()` on a `None` value";
        assert_eq!(redact(msg), msg);
        assert_eq!(
            redact("index out of bounds: the len is 3"),
            "index out of bounds: the len is 3"
        );
    }

    proptest! {
        /// Over generated paths rather than the ones we happened to think of.
        /// The assertion is an exact string match, not `!contains(segment)`:
        /// a one-character segment like `f` is a substring of `<file>`, so a
        /// containment check would fail for a reason that is not a leak.
        #[test]
        fn no_absolute_path_survives_the_scrubber(
            segments in prop::collection::vec("[a-z0-9_-]{1,12}", 1..6),
            root in prop::sample::select(vec!["", "~"]),
        ) {
            let path = format!("{root}/{}", segments.join("/"));
            let out = redact(&format!("{PREFIX}{path}{SUFFIX}"));
            prop_assert_eq!(out, format!("{PREFIX}{PATH_MARKER}{SUFFIX}"));
        }

        /// Same, for a Windows path: a different rule, so a separate case.
        #[test]
        fn no_windows_path_survives_the_scrubber(
            drive in "[A-Z]",
            segments in prop::collection::vec("[a-z0-9_-]{1,12}", 1..6),
            sep in prop::sample::select(vec!['\\', '/']),
        ) {
            let joined = segments.join(&sep.to_string());
            let path = format!("{drive}:{sep}{joined}");
            let out = redact(&format!("{PREFIX}{path}{SUFFIX}"));
            prop_assert_eq!(out, format!("{PREFIX}{PATH_MARKER}{SUFFIX}"));
        }

        /// A `.md` / `.outl` filename is a page title, so it goes whether or
        /// not it arrived as part of a path.
        #[test]
        fn no_page_slug_survives_the_scrubber(
            name in "[a-z0-9_-]{1,20}",
            ext in prop::sample::select(vec!["md", "outl", "MD", "Outl"]),
        ) {
            let out = redact(&format!("{PREFIX}{name}.{ext}{SUFFIX}"));
            prop_assert_eq!(out, format!("{PREFIX}{FILE_MARKER}{SUFFIX}"));
        }

        /// The other direction: the scrubber must not eat the panic location,
        /// which is the only thing making a report actionable.
        #[test]
        fn a_relative_source_path_survives_the_scrubber(
            segments in prop::collection::vec("[a-z0-9_-]{1,12}", 2..6),
            line in 1u32..9999,
        ) {
            let location = format!("{}.rs:{line}", segments.join("/"));
            let out = redact(&format!("{PREFIX}{location}{SUFFIX}"));
            prop_assert_eq!(out, format!("{PREFIX}{location}{SUFFIX}"));
        }
    }
}
