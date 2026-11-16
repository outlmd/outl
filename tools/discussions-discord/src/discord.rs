//! The Discord side: post through the webhook, retry once on a 429.

use std::time::Duration;

use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::{Result, SyncError};
use crate::http::Response;
use crate::payload::{discussion_payload, Discussion};

const MAX_RETRY_SLEEP: f64 = 60.0;
const ERROR_DETAIL_BYTES: usize = 300;

/// Replace the webhook URL in text that may be printed.
pub fn redact(text: &str, webhook: &str) -> String {
    if webhook.is_empty() {
        return text.to_string();
    }
    text.replace(webhook, "***")
}

fn retry_after(resp: &Response) -> Duration {
    let from_body = serde_json::from_slice::<Value>(&resp.body)
        .ok()
        .and_then(|body| match body.get("retry_after")? {
            Value::Number(num) => num.as_f64(),
            Value::String(text) => text.parse().ok(),
            _ => None,
        });
    let seconds = from_body.or_else(|| resp.header("retry-after")?.trim().parse().ok());
    match seconds {
        Some(secs) if secs.is_finite() => Duration::from_secs_f64(secs.clamp(0.0, MAX_RETRY_SLEEP)),
        _ => Duration::from_secs(1),
    }
}

fn with_query(webhook: &str, thread_id: Option<&str>) -> String {
    let sep = if webhook.contains('?') { '&' } else { '?' };
    let mut url = format!("{webhook}{sep}wait=true");
    if let Some(id) = thread_id {
        url.push_str("&thread_id=");
        url.push_str(id);
    }
    url
}

/// POST to the webhook and return the created message.
///
/// One retry on 429, waiting what Discord asks for.
/// Any other failure carries the status and Discord's reply, never the URL.
pub fn post(ctx: &Ctx, webhook: &str, body: &Value, thread_id: Option<&str>) -> Result<Value> {
    let url = with_query(webhook, thread_id);
    let data = serde_json::to_vec(body)?;
    let headers = [("Content-Type", "application/json")];
    let mut attempt = 0;
    let resp = loop {
        let resp = ctx
            .http
            .request("POST", &url, &headers, Some(&data))
            .map_err(|err| SyncError::new(format!("Discord {}", redact(&err.0, webhook))))?;
        if resp.status == 429 && attempt == 0 {
            let wait = retry_after(&resp);
            ctx.log(&format!(
                "Discord rate limited the request, retrying in {:.1}s",
                wait.as_secs_f64()
            ));
            (ctx.sleep)(wait);
            attempt += 1;
            continue;
        }
        break resp;
    };
    if !(200..300).contains(&resp.status) {
        let cut = resp.body.len().min(ERROR_DETAIL_BYTES);
        let detail = redact(&String::from_utf8_lossy(&resp.body[..cut]), webhook);
        return Err(SyncError::new(format!(
            "Discord returned HTTP {}: {detail}",
            resp.status
        )));
    }
    if resp.body.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(&resp.body)
        .map_err(|_| SyncError::new("Discord returned a body that is not JSON"))
}

/// Open the forum post and return its thread id.
///
/// On a forum webhook with `wait=true` the message's `channel_id` is the id of
/// the thread it created.
pub fn create_thread(ctx: &Ctx, webhook: &str, discussion: &Discussion) -> Result<String> {
    let message = post(ctx, webhook, &discussion_payload(discussion), None)?;
    let id = message
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SyncError::new(
            "Discord did not return a thread id; is the webhook on a forum channel?",
        ));
    }
    Ok(id.to_string())
}
