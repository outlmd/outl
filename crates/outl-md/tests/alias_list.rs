//! `parse_alias_list`: comma-separated names, with `[[…]]` for a name
//! that has a comma of its own.

use outl_md::index::WorkspaceIndex;
use outl_md::parse_alias_list;
use std::fs;
use tempfile::TempDir;

#[test]
fn plain_names_split_on_commas() {
    assert_eq!(parse_alias_list("Big Apple, NYC"), ["Big Apple", "NYC"]);
    assert_eq!(parse_alias_list(" a ,, b ,"), ["a", "b"]);
    assert!(parse_alias_list("").is_empty());
}

#[test]
fn a_bracketed_name_keeps_its_comma_and_drops_the_brackets() {
    assert_eq!(
        parse_alias_list("[[Speak, then listen]], NYC, [[Plain]]"),
        ["Speak, then listen", "NYC", "Plain"]
    );
    assert_eq!(parse_alias_list("[[only, one]]"), ["only, one"]);
}

#[test]
fn brackets_that_do_not_wrap_the_whole_entry_are_text() {
    // Not a bracketed entry: text follows the closing brackets.
    assert_eq!(parse_alias_list("[[a]] b, c"), ["[[a]] b", "c"]);
    // Never closed: the comma splits as usual.
    assert_eq!(parse_alias_list("[[a, b"), ["[[a", "b"]);
}

#[test]
fn a_bracketed_alias_with_a_comma_resolves_to_its_page() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::write(
        dir.path().join("pages/speak.md"),
        "title:: Speak\nalias:: [[Speak, then listen]], Talk\n\n- body\n",
    )
    .unwrap();
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(
        idx.by_slug("speak").unwrap().aliases,
        ["Speak, then listen", "Talk"]
    );
    assert_eq!(idx.by_title("Speak, then listen").unwrap().slug, "speak");
    assert_eq!(idx.by_title("Talk").unwrap().slug, "speak");
}
