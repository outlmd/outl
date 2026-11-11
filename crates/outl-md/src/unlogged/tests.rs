use super::*;
use outl_core::id::NodeId;

fn blk(text: &str) -> SidecarBlock {
    SidecarBlock::from_text(NodeId::new(), 1, 0, text)
}

/// A bullet inside a code fence lives in the block's text **with its
/// marker**, because the renderer does not add one there.
///
/// Asking only the marker-stripped form made every such page read as
/// carrying unlogged content — which withholds its hash and refuses
/// its re-projection, freezing a page that has nothing wrong with it.
/// Measured on a real workspace: 8 pages, 49 lines, all of this shape,
/// every one verified to survive `parse → render` intact.
#[test]
fn a_bullet_inside_a_code_fence_is_not_reported_as_unlogged() {
    let disk = "- intro\n  ```yaml\n  - endpoint:\n    method: POST\n  ```\n";
    let logged = [blk("intro\n```yaml\n- endpoint:\n  method: POST\n```")];
    assert!(
        content_lines_missing_from(disk, &logged).is_empty(),
        "a fenced bullet the log holds verbatim must not read as unlogged"
    );
}

/// The same shape one level down, which is how it actually appeared:
/// a fenced list pasted from Roam under a nested block.
#[test]
fn a_fenced_list_under_a_nested_block_is_not_reported_as_unlogged() {
    let disk = "- parent\n  - ```md\n    ## Objetivos\n    - Aumentar resistência\n    - Desenvolver força\n    ```\n";
    let logged = [
        blk("parent"),
        blk("```md\n## Objetivos\n- Aumentar resistência\n- Desenvolver força\n```"),
    ];
    assert!(
        content_lines_missing_from(disk, &logged).is_empty(),
        "got: {:?}",
        content_lines_missing_from(disk, &logged)
    );
}

/// **The guard must not have gone blind.** Accepting a second shape
/// per line widens how a known line may match; it must not widen
/// *which* lines count as known.
#[test]
fn content_the_log_does_not_have_is_still_reported() {
    let disk = "- intro\n  ```yaml\n  - endpoint:\n  ```\n- a line the log never saw\n";
    let logged = [blk("intro\n```yaml\n- endpoint:\n```")];
    let missing = content_lines_missing_from(disk, &logged);
    assert_eq!(missing, vec!["a line the log never saw".to_string()]);
}

/// A bullet on disk whose text the log does not hold in *either* form
/// fails both lookups — the widening is per-shape, not per-line.
#[test]
fn an_unlogged_bullet_is_reported_whichever_shape_it_takes() {
    let disk = "- known\n  - unknown child\n";
    let logged = [blk("known")];
    let missing = content_lines_missing_from(disk, &logged);
    assert_eq!(missing, vec!["unknown child".to_string()]);
}

/// Multiset semantics survive the second lookup: a line the log holds
/// once and disk holds twice still reports one.
#[test]
fn a_line_duplicated_on_disk_still_counts_the_surplus() {
    let disk = "- dup\n- dup\n";
    let logged = [blk("dup")];
    assert_eq!(content_lines_missing_from(disk, &logged).len(), 1);
}

/// The stand-down moved to the callers, so this function must now
/// answer even when every block is empty.
///
/// `outl doctor` builds its reference from a **render**, and a render
/// of empty blocks says "the tree holds no text" — an answer. While
/// the rule lived in here it silenced that caller too, so the doctor
/// reported `0` lines removed for a repair that would empty the page
/// and the volume guard never fired. Measured: 4 content lines on
/// disk, tree rendering `-\n-\n`, reported 0.
#[test]
fn empty_reference_blocks_still_report_the_disk_content() {
    let disk = "- line one\n- line two\n- line three\n- line four\n";
    let empty_render = [blk(""), blk("")];
    assert_eq!(
        content_lines_missing_from(disk, &empty_render).len(),
        4,
        "a reference of empty blocks is an answer, not an absence of one"
    );
}

/// The texts-only entry point must reach the **same** verdict as the
/// block one, on every shape the block one is pinned against.
///
/// It exists for speed (the survey's reference is a render, so
/// wrapping each line in a `SidecarBlock` minted a ULID and a
/// SHA-256 it never reads). Speed is not a reason to take
/// equivalence on faith here: this decides whether a background pass
/// deletes bytes, so the two forms are compared rather than assumed.
#[test]
fn the_texts_entry_point_agrees_with_the_block_one() {
    let cases: [(&str, &[&str]); 7] = [
        (
            "- intro\n  ```yaml\n  - endpoint:\n    method: POST\n  ```\n",
            &["intro\n```yaml\n- endpoint:\n  method: POST\n```"],
        ),
        (
            "- intro\n  ```yaml\n  - endpoint:\n  ```\n- a line the log never saw\n",
            &["intro\n```yaml\n- endpoint:\n```"],
        ),
        ("- known\n  - unknown child\n", &["known"]),
        ("- dup\n- dup\n", &["dup"]),
        ("- line one\n- line two\n", &["", ""]),
        ("- secret\n", &["```\n- secret\n```"]),
        ("- a\n  ```\n  - j\n  ```\n  j\n", &["a\n```\n- j\n```\nj"]),
    ];
    for (disk, texts) in cases {
        let blocks: Vec<SidecarBlock> = texts.iter().map(|t| blk(t)).collect();
        assert_eq!(
            content_lines_missing_from(disk, &blocks),
            content_lines_missing_from_texts(disk, texts.iter().copied()),
            "the two entry points disagree on {disk:?}"
        );
    }
}

/// A root-level bullet does not get the verbatim second chance.
///
/// The renderer never writes a continuation at column 0, so a bullet
/// there is a block's own marker. Letting it match a fenced
/// `- secret` the log holds elsewhere on the page would hide real
/// unlogged content — narrow, but this decides whether bytes are
/// deleted.
#[test]
fn a_root_level_bullet_does_not_match_a_fenced_copy_of_itself() {
    let disk = "- secret\n";
    let logged = [blk("```\n- secret\n```")];
    assert_eq!(
        content_lines_missing_from(disk, &logged),
        vec!["secret".to_string()],
        "a root bullet must not borrow a fenced line's identity"
    );
}

/// An indented bullet must not spend a later line's match.
///
/// `known` is a **multiset** that decrements, so trying the wrong
/// form first does not merely fail — it consumes the entry another
/// line needed. Here the continuation `- j` strips to `j` and takes
/// the single `j` the log holds for the final line, which then
/// matches nothing: every line is logged and one is reported.
///
/// A false positive is the expensive direction. It withholds
/// `last_synced_hash`, refuses re-projection, and `outl reconcile
/// --ahead-of-log` — the recovery the error names — re-runs this
/// same computation and refuses again. The page is frozen in both
/// directions with nothing wrong with it.
///
/// Found by `a_rendered_page_never_reports_content_its_own_log_holds`
/// on a later proptest seed, which is why this deterministic twin
/// exists: a seed that finds a defect once is not a test.
#[test]
fn a_fenced_bullet_does_not_steal_a_later_lines_match() {
    // Exactly what `render` emits for a single block whose text is
    // "a\n```\n- j\n```\nj".
    let disk = "- a\n  ```\n  - j\n  ```\n  j\n";
    let logged = [blk("a\n```\n- j\n```\nj")];
    assert!(
        content_lines_missing_from(disk, &logged).is_empty(),
        "every line here is in the block's own text"
    );
}

/// An empty child block (a bare `-`, indented) next to a block whose
/// text is literally `-` (written `- -`). Reading the indented bare
/// `-` verbatim first spent the `-` the log held for the later block,
/// so a page whose every line the log holds reported one unlogged
/// line and stayed unsynced. Seen on a real Roam import.
#[test]
fn an_empty_child_does_not_steal_a_dash_blocks_match() {
    let disk = "- head
  -
  - x
    - -
";
    let logged = [blk("head"), blk(""), blk("x"), blk("-")];
    assert!(
        content_lines_missing_from(disk, &logged).is_empty(),
        "every line here is a block the log holds: {:?}",
        content_lines_missing_from(disk, &logged)
    );
}

/// Outside a fence the stripped reading wins, but the verbatim one is
/// still a fallback, and a line absent in both forms is reported.
#[test]
fn outside_a_fence_an_unlogged_nested_bullet_is_still_reported() {
    let disk = "- head
  - gone
";
    let logged = [blk("head")];
    assert_eq!(
        content_lines_missing_from(disk, &logged),
        vec!["gone".to_string()]
    );
}

/// The same shape, with the bullet's stripped form genuinely absent.
///
/// Guards the fix from being "always prefer verbatim": the indented
/// `- k` is not in the log in any form, so it must still be
/// reported. Without this, a fix that stopped stripping altogether
/// would pass the test above and let unlogged content through —
/// which is the direction that deletes bytes.
#[test]
fn an_indented_bullet_the_log_lacks_is_still_reported() {
    let disk = "- a\n  ```\n  - k\n  ```\n";
    let logged = [blk("a\n```\n```")];
    assert_eq!(
        content_lines_missing_from(disk, &logged),
        vec!["k".to_string()],
        "an indented line absent from the log must fail both forms"
    );
}
