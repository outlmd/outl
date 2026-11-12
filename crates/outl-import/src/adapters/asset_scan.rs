//! Shared asset-link scanner for the markdown-dialect adapters.
//!
//! Every adapter reaches a point where a block's text carries a
//! CommonMark link — `![alt](target)` or `[alt](target)` — that points
//! at a file the workspace should own: a local attachment (Logseq /
//! Obsidian) or a remote image (Roam's firebase-hosted uploads). This
//! module finds those links, records each as an [`AssetRef`] in the
//! graph's asset list, and swaps the link for an inert
//! [`asset_placeholder`] so the text stays tokenizable. It performs **no
//! asset IO** — it never opens, copies, downloads, or stat's a file;
//! resolving the placeholder (and counting copied / missing) is the
//! emit stage's job, which alone knows the destination root.
//!
//! After Obsidian's `convert_image_links` has already turned
//! `![[img.png]]` into `![alt](target)`, scanning the CommonMark forms
//! covers all three sources with one pass.

use crate::adapters::scan::alias_link;
use crate::ir::{asset_placeholder, AssetRef, AssetSource};
use std::path::{Component, Path, PathBuf};

/// Rewrite every asset link in `text` to an [`asset_placeholder`],
/// pushing an [`AssetRef`] into `assets` for each one. `base_dir` roots
/// relative local targets (the directory of the note / block's source
/// file); it's irrelevant to remote targets. Returns the rewritten
/// text; non-asset links (`#anchor`, `mailto:`, `[[wikilinks]]`, a plain
/// `[text](https://…)` hyperlink, already-`assets/<hash>` links) pass
/// through verbatim.
pub(crate) fn scan_assets(text: &str, base_dir: &Path, assets: &mut Vec<AssetRef>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        let rest = &text[i..];
        // An asset link is `[label](target)` or its image form
        // `![label](target)`. Skip `[[wikilinks]]` and `![[embeds]]`
        // entirely — those are page/block refs, never file links.
        let bracket_off = if rest.starts_with("![") && !rest.starts_with("![[") {
            1
        } else if rest.starts_with('[') && !rest.starts_with("[[") {
            0
        } else {
            let ch = rest.chars().next().expect("rest is non-empty");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        };

        let is_embed = bracket_off == 1;
        if let Some((alias, target, len)) = alias_link(&rest[bracket_off..]) {
            let whole_len = bracket_off + len;
            let whole = &rest[..whole_len];
            match classify(alias, target, whole, base_dir, is_embed) {
                Some(asset) => {
                    let idx = assets.len();
                    assets.push(asset);
                    out.push_str(&asset_placeholder(idx));
                }
                None => out.push_str(whole),
            }
            i += whole_len;
            continue;
        }

        // Not a well-formed link — emit the leading marker char and
        // keep scanning (a bare `!` re-examines the following `[`).
        let ch = rest.chars().next().expect("rest is non-empty");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Classify a `[alias](target)` link into an [`AssetRef`], or `None`
/// when the target isn't a file the workspace should pull in. `is_embed`
/// is `true` for the image form `![alias](target)`.
fn classify(
    alias: &str,
    target: &str,
    whole: &str,
    base_dir: &Path,
    is_embed: bool,
) -> Option<AssetRef> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let written_alt = alias;
    let alias = alias.trim();

    // Remote target. Only an **image embed** (`![](url)`) is an asset to
    // download — a plain `[text](url)` is a hyperlink (a LinkedIn profile,
    // a Google Doc, a GitHub PR), NOT a file, so it stays verbatim.
    if target.starts_with("http://") || target.starts_with("https://") {
        if !is_embed {
            return None;
        }
        // The alt as written, empty included: a downloaded copy keeps
        // it, never a label made up from the URL.
        return Some(AssetRef {
            source: AssetSource::Remote(target.to_string()),
            display_name: written_alt.to_string(),
            original: whole.to_string(),
        });
    }

    // Not a file link: a block ref (`[alias](((uid)))`), a page link
    // (`[alias]([[Page]])`), an anchor, any URI scheme, or a link this
    // importer already emitted (`assets/<64-hex>` — skip to stay
    // idempotent on re-import). A *source* path that merely lives in an
    // `assets/` folder (`assets/pic.png`) is still copied.
    if target.starts_with('#')
        || target.starts_with("((")
        || target.starts_with("[[")
        || is_content_addressed(target)
        || has_scheme(target)
    {
        return None;
    }

    // Relative local path — resolve it against the source file's dir.
    let abs = normalize(&base_dir.join(target));
    let display = if alias.is_empty() {
        path_leaf(target)
    } else {
        alias.to_string()
    };
    Some(AssetRef {
        source: AssetSource::Local(abs),
        display_name: display,
        original: whole.to_string(),
    })
}

/// True when `target` is a link this importer itself emitted:
/// `assets/<64-lowercase-hex>` with an optional extension. Used to stay
/// idempotent across re-imports — a plain vault path like
/// `assets/pic.png` is *not* content-addressed and still gets copied.
fn is_content_addressed(target: &str) -> bool {
    let rel = target.strip_prefix("./").unwrap_or(target);
    let rel = rel.strip_prefix('/').unwrap_or(rel);
    let Some(name) = rel.strip_prefix("assets/") else {
        return false;
    };
    if name.contains('/') {
        return false;
    }
    let stem = name.split_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.len() == 64
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True when `s` begins with a URI scheme (`scheme:`), so it's an
/// external target rather than a relative path. `http(s)` is handled
/// before this, and a bare relative path never carries a `:` prefix.
fn has_scheme(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    for (i, &c) in bytes.iter().enumerate() {
        if c == b':' {
            return i > 0;
        }
        if !(c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.') {
            return false;
        }
    }
    false
}

/// Collapse `.`/`..` components so a `../assets/x.png` resolves to a
/// clean absolute path without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Last path segment of a relative target, minus any `#`/`?` suffix.
fn path_leaf(target: &str) -> String {
    let head = target.split(['#', '?']).next().unwrap_or(target);
    head.rsplit('/').next().unwrap_or(head).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(text: &str) -> (String, Vec<AssetRef>) {
        let mut assets = Vec::new();
        let out = scan_assets(text, Path::new("/vault/notes"), &mut assets);
        (out, assets)
    }

    #[test]
    fn local_relative_link_becomes_placeholder() {
        let (out, assets) = scan("see ![](../assets/pic.png) here");
        assert_eq!(out, "see ((outl-import-asset:0)) here");
        assert_eq!(assets.len(), 1);
        match &assets[0].source {
            AssetSource::Local(p) => {
                assert_eq!(p, Path::new("/vault/assets/pic.png"), "`..` resolved");
            }
            _ => panic!("expected local"),
        }
        assert_eq!(assets[0].display_name, "pic.png");
        assert_eq!(assets[0].original, "![](../assets/pic.png)");
    }

    #[test]
    fn remote_link_becomes_remote_ref() {
        let (out, assets) = scan("![diagram](https://firebasestorage.example/o/img.png?alt=media)");
        assert_eq!(out, "((outl-import-asset:0))");
        assert_eq!(assets.len(), 1);
        match &assets[0].source {
            AssetSource::Remote(u) => {
                assert_eq!(u, "https://firebasestorage.example/o/img.png?alt=media");
            }
            _ => panic!("expected remote"),
        }
        // Alias wins over the URL leaf as the display name.
        assert_eq!(assets[0].display_name, "diagram");
    }

    #[test]
    fn remote_plain_link_is_a_hyperlink_not_an_asset() {
        // A bare `[text](https://…)` is a hyperlink (LinkedIn, a Google
        // Doc, a GitHub PR), not a file — it must pass through untouched,
        // never queued for download. Only the `![](url)` image embed form
        // is a remote asset.
        let (out, assets) = scan("see [Vinícius](https://www.linkedin.com/in/foo/) profile");
        assert_eq!(
            out,
            "see [Vinícius](https://www.linkedin.com/in/foo/) profile"
        );
        assert!(assets.is_empty());
    }

    #[test]
    fn remote_without_alias_keeps_the_empty_alt() {
        let (_out, assets) = scan("![](https://host.example/path/photo.jpg)");
        assert_eq!(assets[0].display_name, "");
    }

    #[test]
    fn non_image_local_file_scans_like_an_image() {
        let (out, assets) = scan("the [spec](files/spec.pdf) doc");
        assert_eq!(out, "the ((outl-import-asset:0)) doc");
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].display_name, "spec");
    }

    #[test]
    fn mixed_local_and_remote_get_sequential_indices() {
        let (out, assets) = scan("![](a.png) and ![](https://x.example/b.gif)");
        assert_eq!(out, "((outl-import-asset:0)) and ((outl-import-asset:1))");
        assert!(matches!(assets[0].source, AssetSource::Local(_)));
        assert!(matches!(assets[1].source, AssetSource::Remote(_)));
    }

    #[test]
    fn text_without_assets_is_unchanged() {
        let (out, assets) = scan("just [[a page]] and #tag, nothing else");
        assert_eq!(out, "just [[a page]] and #tag, nothing else");
        assert!(assets.is_empty());
    }

    #[test]
    fn anchors_mail_and_wikilinks_are_ignored() {
        let (out, assets) = scan("[jump](#section) [[Wiki]] [mail](mailto:a@b.co)");
        assert_eq!(out, "[jump](#section) [[Wiki]] [mail](mailto:a@b.co)");
        assert!(assets.is_empty());
    }

    #[test]
    fn aliased_block_refs_and_page_links_pass_through() {
        // Roam's `[alias](((uid)))` and `[alias]([[Page]])` are not file
        // links — the dialect tokenizer owns them, scan must not touch them.
        let (out, assets) = scan("[the decision](((abc))) and [see]([[Some Page]])");
        assert_eq!(out, "[the decision](((abc))) and [see]([[Some Page]])");
        assert!(assets.is_empty());
    }

    #[test]
    fn our_own_content_addressed_links_are_left_alone() {
        // A 64-hex `assets/<hash>.png` is a link this importer emitted —
        // re-import must not re-process it.
        let hash = "a".repeat(64);
        let link = format!("[keep](assets/{hash}.png)");
        let (out, assets) = scan(&link);
        assert_eq!(out, link);
        assert!(assets.is_empty());
    }

    #[test]
    fn a_vault_assets_path_is_still_copied() {
        // A source path that merely lives in an `assets/` folder is NOT
        // content-addressed — it's a real file to pull in.
        let (out, assets) = scan("![](assets/pic.png)");
        assert_eq!(out, "((outl-import-asset:0))");
        assert_eq!(assets.len(), 1);
        assert!(matches!(assets[0].source, AssetSource::Local(_)));
    }
}
