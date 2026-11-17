//! Logseq file name → page title.
//!
//! A Logseq page name is not its filename: `/`, `:`, `?` and friends
//! are illegal or awkward on some filesystem, so Logseq encodes them.
//! It has done so in two ways, and which one a graph uses is written
//! in `logseq/config.edn` under `:file/name-format`.
//!
//! Decoding is ported from Logseq's own
//! `deps/graph-parser/src/logseq/graph_parser/extract.cljc`
//! (`tri-lb-title-parsing` / `legacy-title-parsing`), which is the
//! function Logseq itself uses to turn a file back into a page:
//!
//! - **`:triple-lowbar`** (every graph Logseq has created since 0.6.7,
//!   and what it writes into a new `config.edn`): `___` → `/`, then
//!   percent-decode, then drop empty namespace segments.
//! - **legacy** (pre-0.6.7): `.` → `/`, then percent-decode the whole
//!   stem.
//!
//! **A missing key means `:triple-lowbar` here, where Logseq's own
//! file→DB importer reads it as legacy** (`exporter.cljs`: `(or
//! (:file/name-format config) :legacy)`). Logseq can afford that
//! default because it wrote the config itself and knows an old graph
//! when it sees one. An importer reading someone else's directory
//! cannot, and the two guesses are not symmetric: legacy turns every
//! `.` into a namespace separator, so one wrong guess renames
//! `node.js` to `node/js` and `v1.2` to `v1/2` on a modern graph,
//! while guessing triple-lowbar on an old graph only leaves a literal
//! `___` in a title nobody wrote one into. Percent-decoding runs in
//! both formats, so `%2F` from an old graph decodes either way.

use std::path::Path;

/// Which of Logseq's two file-name encodings a graph uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameFormat {
    /// `:triple-lowbar` — `/` written as `___`.
    TripleLowbar,
    /// Pre-0.6.7 — `/` written as `.`.
    Legacy,
}

/// Read `:file/name-format` out of `<graph>/logseq/config.edn`.
///
/// The shipped `config.edn` carries most keys commented out, so a
/// `;;`-prefixed mention must not count as a setting — that is the one
/// subtlety here, and reading it wrong would silently flip the
/// encoding for every page in the graph.
pub(crate) fn read_name_format(src: &Path) -> NameFormat {
    let Ok(text) = std::fs::read_to_string(src.join("logseq").join("config.edn")) else {
        return NameFormat::TripleLowbar;
    };
    name_format_in(&text).unwrap_or(NameFormat::TripleLowbar)
}

/// `:file/name-format <keyword>` from EDN text, comments stripped.
fn name_format_in(text: &str) -> Option<NameFormat> {
    const KEY: &str = ":file/name-format";
    let code: String = text
        .lines()
        .map(|l| match l.find(';') {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let after = code.find(KEY).map(|i| &code[i + KEY.len()..])?;
    let value = after.trim_start();
    if value.starts_with(":triple-lowbar") {
        return Some(NameFormat::TripleLowbar);
    }
    if value.starts_with(":legacy") {
        return Some(NameFormat::Legacy);
    }
    None
}

/// Decode one file stem into the page name Logseq would show.
pub(crate) fn decode_page_name(stem: &str, format: NameFormat) -> String {
    let decoded = match format {
        NameFormat::TripleLowbar => valid_namespaces(&percent_decode(&stem.replace("___", "/"))),
        NameFormat::Legacy => percent_decode(&stem.replace('.', "/")),
    };
    // A stem that decodes to nothing (`___`, `%2F`) would reach the
    // emitter as an empty title and slugify to `untitled`, losing the
    // one clue to what the file was. Keep what the filesystem had.
    if decoded.trim().is_empty() {
        return stem.to_string();
    }
    decoded
}

/// Drop empty namespace segments, as Logseq's `make-valid-namespaces`
/// does: `a//b` → `a/b`, `/a` → `a`.
fn valid_namespaces(name: &str) -> String {
    name.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Percent-decode, keeping the input verbatim when it is not valid
/// UTF-8 — a title is never worth mangling over an encoding guess.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The namespace separator is `___`, not a space. Reading it as a
    /// space is what flattened every `[[parent/child]]` page in a
    /// modern graph into one unrelated title.
    #[test]
    fn triple_lowbar_is_a_namespace_separator() {
        assert_eq!(
            decode_page_name("meu___projeto", NameFormat::TripleLowbar),
            "meu/projeto"
        );
        assert_eq!(
            decode_page_name("a___b___c", NameFormat::TripleLowbar),
            "a/b/c"
        );
    }

    #[test]
    fn percent_escapes_decode_in_both_formats() {
        assert_eq!(
            decode_page_name("foo%2Fbar", NameFormat::TripleLowbar),
            "foo/bar"
        );
        assert_eq!(
            decode_page_name("what%3F", NameFormat::TripleLowbar),
            "what?"
        );
        assert_eq!(
            decode_page_name("caf%C3%A9", NameFormat::TripleLowbar),
            "café"
        );
        assert_eq!(decode_page_name("a%2Fb", NameFormat::Legacy), "a/b");
    }

    /// Triple-lowbar leaves `.` alone; legacy is the format where a dot
    /// is the separator.
    #[test]
    fn a_dot_is_only_a_separator_in_the_legacy_format() {
        assert_eq!(
            decode_page_name("node.js", NameFormat::TripleLowbar),
            "node.js"
        );
        assert_eq!(decode_page_name("node.js", NameFormat::Legacy), "node/js");
    }

    #[test]
    fn empty_namespace_segments_are_dropped() {
        assert_eq!(
            decode_page_name("a______b", NameFormat::TripleLowbar),
            "a/b"
        );
    }

    /// Nothing left after decoding means the decode lost the file, so
    /// the stem stands in rather than a page called `untitled`.
    #[test]
    fn a_stem_that_decodes_to_nothing_keeps_its_stem() {
        assert_eq!(decode_page_name("___", NameFormat::TripleLowbar), "___");
    }

    #[test]
    fn the_format_comes_from_config_edn() {
        assert_eq!(
            name_format_in("{:file/name-format :triple-lowbar}"),
            Some(NameFormat::TripleLowbar)
        );
        assert_eq!(
            name_format_in("{:file/name-format :legacy}"),
            Some(NameFormat::Legacy)
        );
        assert_eq!(
            name_format_in("{:file/name-format\n :legacy}"),
            Some(NameFormat::Legacy)
        );
    }

    /// Logseq ships `config.edn` with most keys commented out. A
    /// `;;`-prefixed mention is documentation, not a setting.
    #[test]
    fn a_commented_out_key_is_not_a_setting() {
        assert_eq!(
            name_format_in(";; :file/name-format :legacy\n{:foo 1}"),
            None
        );
    }

    /// No config (or no key) reads as the modern format — see the
    /// module doc for why this does not follow Logseq's own default.
    #[test]
    fn a_graph_without_the_key_reads_as_triple_lowbar() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_name_format(dir.path()), NameFormat::TripleLowbar);
        std::fs::create_dir_all(dir.path().join("logseq")).expect("mkdir");
        std::fs::write(dir.path().join("logseq/config.edn"), "{:foo 1}").expect("write");
        assert_eq!(read_name_format(dir.path()), NameFormat::TripleLowbar);
    }

    #[test]
    fn a_legacy_graph_is_read_as_legacy() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logseq")).expect("mkdir");
        std::fs::write(
            dir.path().join("logseq/config.edn"),
            ";; comment\n{:file/name-format :legacy}\n",
        )
        .expect("write");
        assert_eq!(read_name_format(dir.path()), NameFormat::Legacy);
    }
}
