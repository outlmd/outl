use serde_json::{json, Value};

use super::{artifact, mapping_zip, Call, Fixed, Harness, Net, DAY1, DAY2, THREAD};
use crate::error::Result;
use crate::github::lookup_thread;

pub fn lookup(net: &Net, event: &Value) -> (Result<Option<String>>, Harness) {
    let harness = Harness::new();
    let env = harness.env("discussion", event, Some("https://hook.invalid/x"));
    let result = harness.with_ctx(&env, net, |ctx| lookup_thread(ctx, event, 42));
    (result, harness)
}

fn has_auth(call: &Call) -> bool {
    call.2.iter().any(|(name, _)| name == "Authorization")
}

#[test]
fn lookup_reads_the_newest_zip_and_drops_auth_on_the_redirect() {
    let net = Net::mapped(vec![
        (artifact(1, DAY1), mapping_zip(42, "111")),
        (artifact(2, DAY2), mapping_zip(42, THREAD)),
    ]);
    let (result, _) = lookup(&net, &json!({}));
    assert_eq!(result.expect("lookup").as_deref(), Some(THREAD));

    let calls = net.calls.borrow();
    let listing = &calls[0];
    let url = &listing.1;
    assert!(url.contains("name=discord-thread-42"), "{url}");
    assert!(listing.1.contains("per_page=100"));
    assert!(has_auth(listing));
    let download = calls
        .iter()
        .find(|call| call.1.contains("/artifact-zip/"))
        .expect("download");
    assert!(download.1.ends_with("/artifact-zip/2"));
    assert!(has_auth(download));
    let blob = calls
        .iter()
        .find(|call| call.1.starts_with("https://blob.example/"))
        .expect("blob");
    assert_eq!(blob.1, "https://blob.example/2");
    assert!(blob.2.is_empty(), "no headers on the blob: {:?}", blob.2);
}

#[test]
fn nothing_stored_means_no_mapping() {
    let (result, _) = lookup(&Net::default(), &json!({}));
    assert_eq!(result.expect("lookup"), None);
}

#[test]
fn a_mapping_for_another_discussion_is_rejected() {
    let net = Net::mapped(vec![(artifact(1, DAY1), mapping_zip(41, THREAD))]);
    let (result, _) = lookup(&net, &json!({}));
    let err = result.expect_err("wrong discussion");
    assert!(err.to_string().contains("unexpected mapping"));
}

#[test]
fn a_non_numeric_thread_id_is_rejected() {
    let (result, _) = lookup(&Net::one_mapping("../x"), &json!({}));
    assert!(result.is_err());
}

#[test]
fn a_raw_json_download_is_accepted() {
    let raw = json!({"discussion": 42, "thread_id": THREAD}).to_string();
    let net = Net::mapped(vec![(artifact(1, DAY1), raw.into_bytes())]);
    let (result, _) = lookup(&net, &json!({}));
    assert_eq!(result.expect("lookup").as_deref(), Some(THREAD));
}

#[test]
fn a_failed_listing_is_an_error() {
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some("x"));
    let http = Fixed {
        artifacts: Vec::new(),
        status: 500,
    };
    let result = harness.with_ctx(&env, &http, |ctx| lookup_thread(ctx, &json!({}), 42));
    let err = result.expect_err("500");
    assert!(err.to_string().contains("listing artifacts failed"));
}
