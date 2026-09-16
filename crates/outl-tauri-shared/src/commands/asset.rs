//! Asset command bodies: open an uploaded file outside outl, and import
//! a file from the OS into the workspace as a new block.
//!
//! Both delegate to `outl-actions` (`import_asset` / `resolve_asset_path`)
//! and follow the same `AppHost`-generic shape as the block commands.
//! outl deliberately does **not** render the asset — it copies the bytes
//! into `<root>/assets/<hash>.<ext>` and hands the OS the file so the
//! user's default viewer (Preview, an image viewer) opens it.

use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use outl_actions::{
    append_block, create_after_or_append, import_asset, resolve_asset_path, ActionError,
    ImportedAsset,
};

use crate::helpers::{finish_in_page, normalize_picker_path, parse_node_id, storage_root_or_err};
use crate::host::AppHost;
use crate::state::PageView;

/// Open an `assets/<hash>.<ext>` link in the OS default app.
///
/// The URL is a workspace-relative asset link the user clicked. We
/// resolve it to an absolute path under `<root>/assets/` (rejecting
/// traversal / external schemes upstream in `resolve_asset_path`) and
/// hand it to `open::that`, which launches the OS default handler.
///
/// Read-only: it touches no workspace lock, only a path resolution plus
/// the OS open. `Ok(None)` from the resolver means the asset hasn't
/// synced to this device yet — surfaced as a distinct error so the
/// client can tell "not here yet" from "bad link".
pub fn open_asset<S: AppHost>(state: &S, url: String) -> Result<(), String> {
    let root = storage_root_or_err(state)?;
    match resolve_asset_path(&root, &url) {
        Ok(Some(path)) => open::that(&path).map_err(|e| format!("failed to open asset: {e}")),
        Ok(None) => Err("asset not found on this device yet".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Ceiling on the bytes we're willing to inline into a `data:` URL. A
/// base64 payload is ~4/3 the file size and lives entirely in the
/// webview's memory, so an unbounded read would let one giant asset wedge
/// the renderer. Images / small PDFs (the only kinds the frontend fetches)
/// sit well under this.
const MAX_DATA_URL_BYTES: u64 = 25 * 1024 * 1024;

/// Guess a MIME type from a file extension, covering the kinds the
/// frontend inlines (images + pdf). Anything else falls back to the
/// generic binary type — the webview still renders images/pdf correctly,
/// and the frontend only asks for kinds it knows how to show.
fn mime_from_ext(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Resolve an `assets/<hash>.<ext>` link to a `data:<mime>;base64,<…>`
/// URL the webview loads directly (an `<img src>` or a PDF viewer).
///
/// The Tauri asset protocol is deliberately **not** used: it needs a
/// static `assetProtocol.scope`, but outl's workspace root is picked at
/// runtime, so there's nothing to scope ahead of time. Encoding the bytes
/// into a `data:` URL sidesteps the protocol entirely — same code path on
/// desktop and mobile, zero Tauri config.
///
/// The path resolves through the shared [`resolve_asset_path`] guard
/// (rejects traversal / external schemes). Only a regular file is served,
/// and the read itself is bounded to `MAX_DATA_URL_BYTES`, so a huge file
/// can never be base64'd into the webview. `Ok(None)` from the resolver
/// means the asset hasn't synced to this device yet — surfaced as a
/// distinct error, mirroring [`open_asset`].
pub fn read_asset_data_url<S: AppHost>(state: &S, url: String) -> Result<String, String> {
    let root = storage_root_or_err(state)?;
    let path = match resolve_asset_path(&root, &url) {
        Ok(Some(path)) => path,
        Ok(None) => return Err("asset not found on this device yet".to_string()),
        Err(e) => return Err(e.to_string()),
    };

    // Reject anything that isn't a plain file. `metadata().len()` is
    // meaningless for a FIFO / device node (reports 0), and reading one
    // would block forever (FIFO) or never reach EOF (`/dev/zero`) — the
    // one gap `resolve_asset_path` leaves, since such a node is still a
    // `Component::Normal` entry under `assets/`.
    let meta = std::fs::metadata(&path).map_err(|e| format!("failed to read asset: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("asset is not a regular file".to_string());
    }

    // Bound the read structurally (not just a pre-check on the reported
    // size): read one byte past the cap, and if we got it the file is over
    // the limit. Mirrors the capped read in `outl_actions::import_asset`.
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| format!("failed to read asset: {e}"))?
        .take(MAX_DATA_URL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read asset: {e}"))?;
    if bytes.len() as u64 > MAX_DATA_URL_BYTES {
        return Err(format!(
            "asset too large to display inline (over {MAX_DATA_URL_BYTES} bytes)"
        ));
    }

    let mime = mime_from_ext(&path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Import `source_path` into the workspace and return the ready-to-insert
/// link **without** touching the outline.
///
/// This is the drag-and-drop path: the file is dropped onto the *active
/// line*, and the client splices [`ImportedAsset::markdown`] straight into
/// that block's in-flight editor text at the caret — so the insertion
/// respects an uncommitted edit instead of racing it through a backend
/// `edit_text`. The copy is content-addressed / idempotent / size-capped,
/// exactly like [`attach_asset`], but no `Op` is emitted here; the client's
/// own commit (on blur / Enter) writes the block text through the normal
/// path. Pure filesystem side effect + a returned DTO, no workspace lock.
pub fn import_asset_file<S: AppHost>(
    state: &S,
    source_path: String,
) -> Result<ImportedAsset, String> {
    let root = storage_root_or_err(state)?;
    let max_bytes = outl_config::load().assets.max_bytes;
    let source = normalize_picker_path(&source_path);
    import_asset(&root, Path::new(&source), max_bytes).map_err(|e| e.to_string())
}

/// Import `source_path` into the workspace and attach its link as a new
/// block.
///
/// `import_asset` copies the file into `<root>/assets/<hash>.<ext>`
/// (content-addressed, idempotent, atomic, size-capped by
/// `[assets] max_bytes`) and returns the ready-to-insert markdown
/// (`[name](assets/hash.pdf)`). The copy touches only the filesystem, not
/// the workspace, so it runs before the lock; the block insert then goes
/// through the same `finish_in_page` commit path every mutation uses.
///
/// `after_block_id` places the new block right after that block
/// (tolerating a stale anchor exactly like `create_block`); `None`
/// appends it at the end of the page.
pub fn attach_asset<S: AppHost>(
    state: &S,
    source_path: String,
    page_id: String,
    after_block_id: Option<String>,
) -> Result<PageView, String> {
    let root = storage_root_or_err(state)?;
    let page = parse_node_id(&page_id)?;
    let max_bytes = outl_config::load().assets.max_bytes;
    let source = normalize_picker_path(&source_path);
    let imported = import_asset(&root, Path::new(&source), max_bytes).map_err(|e| e.to_string())?;
    let markdown = imported.markdown;
    finish_in_page(state, page, |ws| {
        if let Some(id) = &after_block_id {
            let after = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_after_or_append(ws, state.hlc(), page, after, Some(&markdown)).map(|_| ())
        } else {
            append_block(ws, state.hlc(), Some(page), Some(&markdown)).map(|_| ())
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use base64::Engine as _;
    use outl_actions::SyncTransport;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use outl_exec::RuntimeRegistry;
    use parking_lot::Mutex;
    use tempfile::TempDir;

    use super::{mime_from_ext, read_asset_data_url, MAX_DATA_URL_BYTES};
    use crate::helpers::normalize_picker_path;
    use crate::host::AppHost;

    /// Minimal [`AppHost`] for the read-only asset commands: they only touch
    /// `storage_root()`, never the workspace lock, so the workspace slot
    /// stays `None` and the rest is trait boilerplate. Mirrors the shape in
    /// `tests/resolve_embeds.rs` and both real clients.
    struct TestHost {
        workspace: Arc<Mutex<Option<Workspace>>>,
        hlc: HlcGenerator,
        root: PathBuf,
        registry: Arc<RuntimeRegistry>,
    }

    impl AppHost for TestHost {
        fn workspace(&self) -> &Mutex<Option<Workspace>> {
            &self.workspace
        }
        fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
            self.workspace.clone()
        }
        fn hlc(&self) -> &HlcGenerator {
            &self.hlc
        }
        fn storage_root(&self) -> Result<PathBuf, String> {
            Ok(self.root.clone())
        }
        fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
            None
        }
        fn exec_registry(&self) -> Arc<RuntimeRegistry> {
            self.registry.clone()
        }
    }

    /// A temp workspace root with an `assets/` dir, plus a host pointed at
    /// it. `read_asset_data_url` never locks the workspace, so the slot is
    /// left empty (`None`).
    fn setup() -> (TempDir, TestHost) {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("assets")).expect("create assets dir");
        let host = TestHost {
            workspace: Arc::new(Mutex::new(None)),
            hlc: HlcGenerator::new(ActorId::new()),
            root,
            registry: Arc::new(RuntimeRegistry::default()),
        };
        (dir, host)
    }

    fn write_asset(host: &TestHost, name: &str, bytes: &[u8]) {
        std::fs::write(host.root.join("assets").join(name), bytes).expect("write asset");
    }

    #[test]
    fn desktop_plain_path_is_untouched() {
        assert_eq!(
            normalize_picker_path("/Users/me/report.pdf"),
            "/Users/me/report.pdf"
        );
    }

    #[test]
    fn ios_file_url_is_stripped_and_decoded() {
        assert_eq!(
            normalize_picker_path("file:///private/var/My%20File.pdf"),
            "/private/var/My File.pdf"
        );
        assert_eq!(
            normalize_picker_path("file://localhost/tmp/a.pdf"),
            "/tmp/a.pdf"
        );
    }

    // ---- mime_from_ext ---------------------------------------------------

    #[test]
    fn mime_covers_known_and_unknown_extensions() {
        assert_eq!(mime_from_ext(Path::new("assets/a.png")), "image/png");
        assert_eq!(mime_from_ext(Path::new("assets/a.jpg")), "image/jpeg");
        assert_eq!(mime_from_ext(Path::new("assets/a.jpeg")), "image/jpeg");
        assert_eq!(mime_from_ext(Path::new("assets/a.svg")), "image/svg+xml");
        assert_eq!(mime_from_ext(Path::new("assets/a.pdf")), "application/pdf");
        // Unknown / extensionless falls back to the generic binary type.
        assert_eq!(
            mime_from_ext(Path::new("assets/a.xyz")),
            "application/octet-stream"
        );
        assert_eq!(
            mime_from_ext(Path::new("assets/noext")),
            "application/octet-stream"
        );
    }

    #[test]
    fn mime_lowercases_the_extension() {
        // An uppercase extension must still map — the fn lowercases first.
        assert_eq!(mime_from_ext(Path::new("assets/a.PNG")), "image/png");
        assert_eq!(mime_from_ext(Path::new("assets/a.Jpeg")), "image/jpeg");
    }

    // ---- read_asset_data_url: happy path ---------------------------------

    #[test]
    fn happy_path_returns_data_url_with_original_bytes() {
        let (_dir, host) = setup();
        let bytes = b"\x89PNG\r\n\x1a\n not a real png but real bytes";
        write_asset(&host, "pic.png", bytes);

        let url = read_asset_data_url(&host, "assets/pic.png".to_string())
            .expect("a regular file under assets/ resolves");

        let b64 = url
            .strip_prefix("data:image/png;base64,")
            .expect("data URL carries the png mime + base64 marker");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("tail is valid base64");
        assert_eq!(decoded, bytes, "round-trips the original bytes");
    }

    // ---- read_asset_data_url: traversal / bad links ----------------------

    #[test]
    fn traversal_link_is_rejected() {
        let (_dir, host) = setup();
        // A crafted `..` escape must never reach the filesystem — the error
        // is the traversal rejection, not the "not found" sentinel.
        for bad in ["assets/../../etc/passwd", "../secret"] {
            let err = read_asset_data_url(&host, bad.to_string())
                .expect_err("traversal / non-asset link must be rejected");
            assert!(
                err.contains("invalid asset path"),
                "expected traversal rejection for {bad:?}, got: {err}"
            );
            assert_ne!(
                err, "asset not found on this device yet",
                "traversal must not be reported as a plain missing file"
            );
        }
    }

    // ---- read_asset_data_url: not synced yet -----------------------------

    #[test]
    fn well_formed_but_absent_asset_is_not_found() {
        let (_dir, host) = setup();
        // Well-formed link, no traversal, but the file was never written —
        // `resolve_asset_path` returns `Ok(None)`, the distinct sentinel.
        let err = read_asset_data_url(&host, "assets/deadbeef.png".to_string())
            .expect_err("a missing asset resolves to an error");
        assert_eq!(err, "asset not found on this device yet");
    }

    // ---- read_asset_data_url: oversize cap -------------------------------

    #[test]
    fn oversize_asset_is_rejected() {
        let (_dir, host) = setup();
        // Exactly one byte past the cap: the structural `Take` bound reads
        // `MAX + 1`, sees it exceeded the limit, and refuses to inline it.
        let over = vec![0u8; (MAX_DATA_URL_BYTES + 1) as usize];
        write_asset(&host, "huge.png", &over);

        let err = read_asset_data_url(&host, "assets/huge.png".to_string())
            .expect_err("a file over the cap must be rejected");
        assert!(
            err.contains("too large"),
            "expected the oversize error, got: {err}"
        );
    }

    #[test]
    fn asset_exactly_at_cap_is_served() {
        let (_dir, host) = setup();
        // The boundary itself is allowed: `len > MAX` is the reject test, so
        // a file of exactly `MAX` bytes still inlines.
        let at_cap = vec![0u8; MAX_DATA_URL_BYTES as usize];
        write_asset(&host, "atcap.png", &at_cap);

        let url = read_asset_data_url(&host, "assets/atcap.png".to_string())
            .expect("a file at exactly the cap must still be served");
        assert!(url.starts_with("data:image/png;base64,"));
    }

    // ---- read_asset_data_url: non-regular file ---------------------------

    #[test]
    fn directory_under_assets_is_not_a_regular_file() {
        let (_dir, host) = setup();
        // A directory resolves (it exists) but is not a regular file, so the
        // `is_file()` guard rejects it before any read.
        std::fs::create_dir_all(host.root.join("assets").join("subdir"))
            .expect("create asset subdir");
        let err = read_asset_data_url(&host, "assets/subdir".to_string())
            .expect_err("a directory is not a regular file");
        assert_eq!(err, "asset is not a regular file");
    }

    #[cfg(unix)]
    #[test]
    fn fifo_under_assets_is_rejected_without_hanging() {
        let (_dir, host) = setup();
        let fifo = host.root.join("assets").join("pipe.png");
        // No `libc`/`nix` dependency in this crate, so the FIFO is created
        // via `mkfifo(1)` (present on macOS + Linux). `metadata()` stats the
        // node without opening it, so the `is_file()` guard rejects it
        // immediately instead of blocking on an `open()` that never returns.
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("spawn mkfifo");
        assert!(status.success(), "mkfifo should create the FIFO");

        let err = read_asset_data_url(&host, "assets/pipe.png".to_string())
            .expect_err("a FIFO is not a regular file");
        assert_eq!(err, "asset is not a regular file");
    }
}
