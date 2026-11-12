//! Asset resolution — the emit side of the asset pipeline.
//!
//! The adapters' [`scan_assets`](crate::adapters::asset_scan::scan_assets)
//! pass recorded every referenced file in [`ImportGraph::assets`] and
//! dropped an [`asset_placeholder`] where each link used to sit. This
//! module runs once, right after render and **before** any file is
//! written: for every [`AssetRef`] it copies the local file (or
//! downloads the remote URL) into `<root>/assets/<hash>.<ext>` and maps
//! the placeholder to the final `[name](assets/<hash>.<ext>)` link. A
//! remote image is downloaded only on `download_remote_assets`; by
//! default it keeps the link the source wrote.
//!
//! The substitution touches both the page text *and* every
//! [`PlaceholderBlock`](super::render::PlaceholderBlock) text, so the
//! block-ref resolve pass (which hash-checks its blocks against the
//! reconciled sidecar) sees the same asset link that landed on disk.
//! No asset placeholder ever reaches `write_atomic` — a file that can't
//! be pulled keeps its original link verbatim and is counted in
//! `assets_missing`, never fatal.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::ir::{AssetRef, AssetSource, ImportGraph, ASSET_PLACEHOLDER_PREFIX};
use crate::report::ImportReport;
use crate::ImportOptions;

use super::render::RenderedPage;

/// Resolve every asset and substitute its placeholder across the
/// rendered pages, in place. A no-op when the graph referenced none.
pub(super) fn apply(
    pages: &mut [RenderedPage],
    graph: &ImportGraph,
    root: &Path,
    opts: &ImportOptions,
    report: &mut ImportReport,
) {
    if graph.assets.is_empty() {
        return;
    }
    let links = resolve_all(graph, root, opts, report);
    for page in pages.iter_mut() {
        page.text = substitute(&page.text, &links, graph);
        for pb in page.placeholder_blocks.iter_mut() {
            pb.text = substitute(&pb.text, &links, graph);
        }
    }
}

/// idx → final link text for every asset. When `import_assets` is off,
/// or a pull fails, the entry is the asset's original link verbatim.
fn resolve_all(
    graph: &ImportGraph,
    root: &Path,
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> HashMap<usize, String> {
    let mut links = HashMap::with_capacity(graph.assets.len());
    let mut missing_local: Vec<&Path> = Vec::new();
    for (idx, asset) in graph.assets.iter().enumerate() {
        if !opts.import_assets {
            links.insert(idx, asset.original.clone());
            continue;
        }
        if !pulls(asset, opts) {
            report.assets_remote_kept += 1;
            links.insert(idx, asset.original.clone());
            continue;
        }
        let pulled = match &asset.source {
            AssetSource::Local(path) => pull_local(path, root, opts, report),
            AssetSource::Remote(url) => pull_remote(url, &asset.display_name, root, opts, report),
        };
        match pulled {
            Some(link) => {
                links.insert(idx, link);
            }
            None => {
                report.assets_missing += 1;
                if let AssetSource::Local(path) = &asset.source {
                    missing_local.push(path);
                }
                links.insert(idx, asset.original.clone());
            }
        }
    }
    warn_missing_local(&missing_local, report);
    links
}

/// Local files the source links to and the import could not read, said
/// once instead of once per file: an export that left its attachments
/// behind misses hundreds, and a line each buried every other warning.
/// Each path is still logged at `debug`.
fn warn_missing_local(missing: &[&Path], report: &mut ImportReport) {
    const NAMED: usize = 5;
    if missing.is_empty() {
        return;
    }
    let named: Vec<String> = missing
        .iter()
        .take(NAMED)
        .map(|p| p.display().to_string())
        .collect();
    let more = missing.len().saturating_sub(NAMED);
    let tail = if more > 0 {
        format!(" and {more} more (RUST_LOG=debug lists every path)")
    } else {
        String::new()
    };
    let detail = format!(
        "{} local file(s) linked from the source could not be read, links kept as written: {}{tail}",
        missing.len(),
        named.join(", ")
    );
    tracing::warn!("import: {detail}");
    report.warn("(graph)", detail);
}

/// Whether `asset` is pulled into `assets/` (given `import_assets` is
/// on). A local file always is. A remote image only on
/// `download_remote_assets`, and never an encrypted upload: Roam keeps
/// an encrypted graph's images as `….enc` objects whose bytes are
/// ciphertext, so the download would store noise under an image link.
pub(super) fn pulls(asset: &AssetRef, opts: &ImportOptions) -> bool {
    match &asset.source {
        AssetSource::Local(_) => true,
        AssetSource::Remote(url) => opts.download_remote_assets && !is_encrypted_upload(url),
    }
}

/// `true` for a URL whose path names an encrypted object (`….enc`,
/// percent-encoded or not), like the files of an encrypted Roam graph.
fn is_encrypted_upload(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".enc")
}

/// Copy a local file into `assets/`. `None` (→ missing) on a read error
/// or the size cap; never aborts the import.
fn pull_local(
    path: &Path,
    root: &Path,
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> Option<String> {
    match outl_actions::import_asset(root, path, opts.max_bytes) {
        Ok(asset) => {
            report.assets_copied += 1;
            report.assets_bytes += std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            Some(asset.markdown)
        }
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "import: local asset skipped");
            None
        }
    }
}

/// Download a remote URL and import the bytes. `None` (→ missing) on any
/// network / status / import failure; never aborts the import.
fn pull_remote(
    url: &str,
    alt: &str,
    root: &Path,
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> Option<String> {
    let (bytes, ext) = match download(url, opts.max_bytes) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(url, error = %e, "import: remote asset download failed");
            return None;
        }
    };
    match outl_actions::import_asset_bytes(root, &bytes, &ext, alt, opts.max_bytes) {
        Ok(asset) => {
            report.assets_copied += 1;
            report.assets_bytes += bytes.len() as u64;
            // The alt text stays as the source wrote it, empty included:
            // `import_asset_bytes` would label an empty one with the file
            // name, which the source never said.
            let prefix = if asset.is_image { "!" } else { "" };
            Some(format!("{prefix}[{alt}]({})", asset.rel_path))
        }
        Err(e) => {
            tracing::warn!(url, error = %e, "import: remote asset import failed");
            None
        }
    }
}

/// GET `url` with a bounded body read. Returns the bytes plus a derived
/// extension (URL path first, then `Content-Type`). `Err` on any
/// network error, a non-2xx status, or a body over `max_bytes`.
fn download(url: &str, max_bytes: u64) -> Result<(Vec<u8>, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // Read one byte past the cap so "exactly at the cap" and "over" are
    // distinguishable without buffering the whole (possibly huge) body.
    let read_limit = if max_bytes == 0 {
        u64::MAX
    } else {
        max_bytes.saturating_add(1)
    };
    let mut bytes = Vec::new();
    resp.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if max_bytes > 0 && bytes.len() as u64 > max_bytes {
        return Err(format!("body exceeds cap of {max_bytes} bytes"));
    }

    let ext = ext_from_url(url)
        .or_else(|| ext_from_content_type(content_type.as_deref()))
        .unwrap_or_default();
    Ok((bytes, ext))
}

/// Extension from a URL's path leaf, when it's a short alnum run.
fn ext_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let leaf = path.rsplit('/').next()?;
    let ext = leaf.rsplit_once('.')?.1;
    if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(ext.to_ascii_lowercase())
    } else {
        None
    }
}

/// Extension from a `Content-Type` header, for the common web media
/// types. `None` leaves the asset extensionless (still content-addressed).
fn ext_from_content_type(content_type: Option<&str>) -> Option<String> {
    let ct = content_type?;
    let main = ct
        .split(';')
        .next()
        .unwrap_or(ct)
        .trim()
        .to_ascii_lowercase();
    let ext = match main.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        _ => return None,
    };
    Some(ext.to_string())
}

/// Replace every `((outl-import-asset:<idx>))` in `text` with its final
/// link. An idx missing from `links` falls back to the asset's original
/// link (defensive — `resolve_all` always fills every idx). No asset
/// placeholder is allowed to survive onto disk.
fn substitute(text: &str, links: &HashMap<usize, String>, graph: &ImportGraph) -> String {
    let open = format!("(({ASSET_PLACEHOLDER_PREFIX}");
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find(&open) {
        let start = cursor + rel;
        let after = start + open.len();
        let Some(close_rel) = text[after..].find("))") else {
            break; // Unbalanced — keep the rest verbatim.
        };
        out.push_str(&text[cursor..start]);
        let num = &text[after..after + close_rel];
        let replacement = num
            .parse::<usize>()
            .ok()
            .and_then(|idx| {
                links
                    .get(&idx)
                    .cloned()
                    .or_else(|| graph.assets.get(idx).map(|a| a.original.clone()))
            })
            .unwrap_or_else(|| text[start..after + close_rel + 2].to_string());
        out.push_str(&replacement);
        cursor = after + close_rel + 2;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{asset_placeholder, AssetRef};

    fn graph_with(originals: &[&str]) -> ImportGraph {
        let mut g = ImportGraph::default();
        for o in originals {
            g.assets.push(AssetRef {
                source: AssetSource::Remote((*o).to_string()),
                display_name: "x".to_string(),
                original: (*o).to_string(),
            });
        }
        g
    }

    #[test]
    fn substitute_swaps_each_placeholder() {
        let g = graph_with(&["![](a)", "![](b)"]);
        let mut links = HashMap::new();
        links.insert(0, "[a](assets/1.png)".to_string());
        links.insert(1, "[b](assets/2.png)".to_string());
        let text = format!("x {} y {} z", asset_placeholder(0), asset_placeholder(1));
        assert_eq!(
            substitute(&text, &links, &g),
            "x [a](assets/1.png) y [b](assets/2.png) z"
        );
    }

    #[test]
    fn substitute_falls_back_to_original_when_missing() {
        let g = graph_with(&["![](orig)"]);
        let links = HashMap::new(); // nothing resolved
        let text = asset_placeholder(0);
        assert_eq!(substitute(&text, &links, &g), "![](orig)");
    }

    #[test]
    fn content_type_extension_mapping() {
        assert_eq!(
            ext_from_content_type(Some("image/png")).as_deref(),
            Some("png")
        );
        assert_eq!(
            ext_from_content_type(Some("image/jpeg; charset=binary")).as_deref(),
            Some("jpg")
        );
        assert_eq!(
            ext_from_content_type(Some("application/pdf")).as_deref(),
            Some("pdf")
        );
        assert_eq!(
            ext_from_content_type(Some("image/svg+xml")).as_deref(),
            Some("svg")
        );
        assert_eq!(ext_from_content_type(Some("text/html")), None);
        assert_eq!(ext_from_content_type(None), None);
    }

    #[test]
    fn url_extension_derivation() {
        assert_eq!(
            ext_from_url("https://x.example/a/b.PNG?alt=media").as_deref(),
            Some("png")
        );
        // The URL path's own extension is kept verbatim (lowercased) —
        // no image/jpeg → jpg remapping here; that's Content-Type's job.
        assert_eq!(
            ext_from_url("https://x.example/a/b.jpeg").as_deref(),
            Some("jpeg")
        );
        // No usable extension in the path.
        assert_eq!(ext_from_url("https://x.example/download"), None);
    }

    #[test]
    fn encrypted_uploads_are_told_apart_by_the_path() {
        assert!(is_encrypted_upload(
            "https://h.invalid/o/a%2Fb.png.enc?alt=media&token=t"
        ));
        assert!(is_encrypted_upload("https://h.invalid/x.PNG.ENC#frag"));
        assert!(!is_encrypted_upload("https://h.invalid/x.png?name=y.enc"));
        assert!(!is_encrypted_upload("https://h.invalid/encoder.png"));
    }
}
