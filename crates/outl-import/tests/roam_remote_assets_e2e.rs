//! End-to-end: remote images keep the link the source wrote unless the
//! download is asked for, an encrypted upload is never downloaded, and a
//! downloaded copy keeps the alt text as written.

mod common;

use common::{import_with_opts, read};
use outl_import::adapters::RoamAdapter;
use outl_import::ImportOptions;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;

fn import_json(json: &str, opts: &ImportOptions) -> (common::TestWs, outl_import::ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with_opts(&RoamAdapter, &src, opts)
}

/// Serve `body` as `image/png` to every request on a local port.
fn serve_png(body: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body);
        }
    });
    format!("http://{addr}")
}

#[test]
fn remote_images_keep_their_link_by_default() {
    let json = r#"[
        {"title": "Gallery", "children": [
            {"string": "![](https://img.invalid/a/photo.png)", "uid": "g1", "children": []},
            {"string": "logo ![The Logo](https://img.invalid/logo.png?alt=media) here", "uid": "g2", "children": []},
            {"string": "![](https://img.invalid/o/imgs%2Fapp%2Fgraph%2Fx.png.enc?alt=media)", "uid": "g3", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_json(json, &ImportOptions::default());
    let page = read(&ws.root.join("pages/gallery.md"));
    assert!(
        page.contains("- ![](https://img.invalid/a/photo.png)\n"),
        "page:\n{page}"
    );
    assert!(
        page.contains("- logo ![The Logo](https://img.invalid/logo.png?alt=media) here\n"),
        "page:\n{page}"
    );
    assert!(
        page.contains("- ![](https://img.invalid/o/imgs%2Fapp%2Fgraph%2Fx.png.enc?alt=media)\n"),
        "page:\n{page}"
    );
    assert_eq!(report.assets_remote_kept, 3);
    assert_eq!(report.assets_copied, 0);
    assert_eq!(report.assets_missing, 0);
    assert!(!ws.root.join("assets").exists(), "nothing downloaded");
}

#[test]
fn a_downloaded_image_keeps_its_alt_and_an_encrypted_one_is_never_fetched() {
    let base = serve_png(b"\x89PNG\r\n\x1a\nnot-really-a-png");
    let json = format!(
        r#"[
        {{"title": "Gallery", "children": [
            {{"string": "![]({base}/plain.png)", "uid": "g1", "children": []}},
            {{"string": "![Chart]({base}/chart.png)", "uid": "g2", "children": []}},
            {{"string": "![]({base}/imgs%2Fsecret.png.enc)", "uid": "g3", "children": []}}
        ]}}
    ]"#
    );
    let opts = ImportOptions {
        download_remote_assets: true,
        ..Default::default()
    };
    let (ws, report) = import_json(&json, &opts);
    let page = read(&ws.root.join("pages/gallery.md"));
    assert!(
        page.contains("- ![](assets/"),
        "empty alt stays empty:\n{page}"
    );
    assert!(page.contains("- ![Chart](assets/"), "page:\n{page}");
    assert!(
        page.contains(&format!("- ![]({base}/imgs%2Fsecret.png.enc)\n")),
        "an encrypted upload keeps its link:\n{page}"
    );
    assert_eq!(report.assets_copied, 2);
    assert_eq!(report.assets_remote_kept, 1);
}
