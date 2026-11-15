//! The GitHub side: find and read the stored thread id, fetch a discussion.
//!
//! The stored mapping is only as trustworthy as the artifact it sits in, and a
//! run triggered from a fork PR can upload an artifact with any name.
//! `trusted_artifact` is therefore the gate every candidate goes through, see
//! `docs/discussions-discord.md`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::artifact_zip;
use crate::ctx::Ctx;
use crate::error::{Result, SyncError};
use crate::http::Response;
use crate::payload::Discussion;

pub const ARTIFACT_PREFIX: &str = "discord-thread-";
pub const THREAD_FILE: &str = "thread.json";
/// The only workflow whose artifacts are believed.
pub const WORKFLOW_PATH: &str = ".github/workflows/discussions-discord.yml";
/// Run events that this workflow's triggers can produce.
pub const TRUSTED_EVENTS: [&str; 3] = ["discussion", "discussion_comment", "workflow_dispatch"];
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

const GRAPHQL: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    discussion(number: $number) {
      number title body url
      author { login }
      category { name }
    }
  }
}";

/// What a trusted artifact must have been produced by.
pub struct Trust {
    pub repository_id: u64,
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
pub struct RunRef {
    pub id: u64,
    pub repository_id: Option<u64>,
    pub head_repository_id: Option<u64>,
    pub head_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub created_at: Option<String>,
    #[serde(default)]
    pub expired: bool,
    pub archive_download_url: String,
    pub workflow_run: Option<RunRef>,
}

fn headers(ctx: &Ctx) -> Vec<(&'static str, String)> {
    let mut list = vec![
        ("Accept", "application/vnd.github+json".to_string()),
        ("X-GitHub-Api-Version", "2022-11-28".to_string()),
    ];
    if let Some(token) = ctx.var("GITHUB_TOKEN").filter(|token| !token.is_empty()) {
        list.push(("Authorization", format!("Bearer {token}")));
    }
    list
}

fn get(ctx: &Ctx, url: &str, authed: bool) -> Result<Response> {
    let owned = if authed { headers(ctx) } else { Vec::new() };
    let borrowed: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
    ctx.http.request("GET", url, &borrowed, None)
}

fn api<'a>(ctx: &Ctx<'a>) -> &'a str {
    ctx.var("GITHUB_API_URL")
        .unwrap_or("https://api.github.com")
}

fn repository<'a>(ctx: &Ctx<'a>) -> Result<&'a str> {
    ctx.var("GITHUB_REPOSITORY")
        .ok_or_else(|| SyncError::new("GITHUB_REPOSITORY is not set"))
}

fn reject(reason: impl Into<String>) -> Result<Option<String>> {
    Ok(Some(reason.into()))
}

/// Why `artifact` cannot be believed, or None when every check passes.
///
/// The cheap checks run first; the run lookup costs an API call.
fn untrusted_reason(ctx: &Ctx, trust: &Trust, artifact: &Artifact) -> Result<Option<String>> {
    if artifact.expired {
        return reject("it expired");
    }
    let Some(run) = &artifact.workflow_run else {
        return reject("it names no workflow run");
    };
    // A fork PR run uploads into this repo's artifact list too, but its head
    // repository is the fork.
    if run.repository_id != Some(trust.repository_id)
        || run.head_repository_id != Some(trust.repository_id)
    {
        return reject("it was not produced by a run of this repository");
    }
    if run.head_branch.as_deref() != Some(trust.default_branch.as_str()) {
        return reject(format!("its run was on branch {:?}", run.head_branch));
    }
    let url = format!(
        "{}/repos/{}/actions/runs/{}",
        api(ctx),
        repository(ctx)?,
        run.id
    );
    let resp = get(ctx, &url, true)?;
    if resp.status == 404 {
        return reject("its run no longer exists");
    }
    if resp.status != 200 {
        return Err(SyncError::new(format!(
            "looking up run {} failed with HTTP {}",
            run.id, resp.status
        )));
    }
    let info: Value = serde_json::from_slice(&resp.body)?;
    // `path` can carry an `@ref` suffix; only the file part identifies the workflow.
    let path = info["path"].as_str().unwrap_or("");
    let path = path.split('@').next().unwrap_or("");
    if path != WORKFLOW_PATH {
        return reject(format!("its run is workflow {path:?}"));
    }
    let event = info["event"].as_str().unwrap_or("");
    if !TRUSTED_EVENTS.contains(&event) {
        return reject(format!("its run was triggered by {event:?}"));
    }
    Ok(None)
}

/// The newest artifact that passes every provenance check, or None.
///
/// A candidate that fails is skipped with a notice, so a forged one never
/// shadows a genuine older mapping.
pub fn trusted_artifact(
    ctx: &Ctx,
    trust: &Trust,
    mut artifacts: Vec<Artifact>,
) -> Result<Option<Artifact>> {
    artifacts.sort_by(|a, b| (&b.created_at, b.id).cmp(&(&a.created_at, a.id)));
    for artifact in artifacts {
        match untrusted_reason(ctx, trust, &artifact)? {
            None => return Ok(Some(artifact)),
            Some(reason) => ctx.log(&format!(
                "::notice::ignoring artifact {}: {reason}",
                artifact.id
            )),
        }
    }
    Ok(None)
}

fn trust_from_env(ctx: &Ctx, event: &Value) -> Result<Trust> {
    let repository_id = ctx
        .var("GITHUB_REPOSITORY_ID")
        .and_then(|id| id.trim().parse().ok())
        .ok_or_else(|| SyncError::new("GITHUB_REPOSITORY_ID is not set"))?;
    let default_branch = event["repository"]["default_branch"]
        .as_str()
        .filter(|branch| !branch.is_empty())
        .unwrap_or("main")
        .to_string();
    Ok(Trust {
        repository_id,
        default_branch,
    })
}

fn read_mapping(zip: &[u8], number: u64) -> Result<String> {
    let raw = artifact_zip::read_entry(zip, THREAD_FILE)?;
    let data: Value = serde_json::from_slice(&raw)?;
    let thread_id = match &data["thread_id"] {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => String::new(),
    };
    let numeric = !thread_id.is_empty() && thread_id.bytes().all(|b| b.is_ascii_digit());
    if data["discussion"].as_u64() != Some(number) || !numeric {
        return Err(SyncError::new(format!(
            "artifact for discussion {number} holds an unexpected mapping"
        )));
    }
    Ok(thread_id)
}

/// Thread id for `number`, or None when no trusted live artifact maps it.
pub fn lookup_thread(ctx: &Ctx, event: &Value, number: u64) -> Result<Option<String>> {
    let trust = trust_from_env(ctx, event)?;
    let url = format!(
        "{}/repos/{}/actions/artifacts?name={ARTIFACT_PREFIX}{number}&per_page=100",
        api(ctx),
        repository(ctx)?
    );
    let resp = get(ctx, &url, true)?;
    if resp.status != 200 {
        return Err(SyncError::new(format!(
            "listing artifacts failed with HTTP {}",
            resp.status
        )));
    }
    let listing: Value = serde_json::from_slice(&resp.body)?;
    let artifacts: Vec<Artifact> =
        serde_json::from_value(listing.get("artifacts").cloned().unwrap_or(json!([])))?;
    let Some(artifact) = trusted_artifact(ctx, &trust, artifacts)? else {
        return Ok(None);
    };
    let mut resp = get(ctx, &artifact.archive_download_url, true)?;
    if REDIRECTS.contains(&resp.status) {
        // The blob host rejects a request that still carries our token.
        let location = resp
            .header("location")
            .ok_or_else(|| SyncError::new("artifact redirect has no Location"))?
            .to_string();
        resp = get(ctx, &location, false)?;
    }
    if resp.status != 200 {
        return Err(SyncError::new(format!(
            "downloading the artifact failed with HTTP {}",
            resp.status
        )));
    }
    read_mapping(&resp.body, number).map(Some)
}

/// The discussion through GraphQL, shaped like the webhook payload's.
pub fn fetch_discussion(ctx: &Ctx, number: u64) -> Result<Discussion> {
    let (owner, name) = repository(ctx)?
        .split_once('/')
        .ok_or_else(|| SyncError::new("GITHUB_REPOSITORY is not owner/name"))?;
    let url = ctx
        .var("GITHUB_GRAPHQL_URL")
        .unwrap_or("https://api.github.com/graphql");
    let body = serde_json::to_vec(&json!({
        "query": GRAPHQL,
        "variables": { "owner": owner, "name": name, "number": number },
    }))?;
    let owned = headers(ctx);
    let mut list: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
    list.push(("Content-Type", "application/json"));
    let resp = ctx.http.request("POST", url, &list, Some(&body))?;
    if resp.status != 200 {
        return Err(SyncError::new(format!(
            "GraphQL request failed with HTTP {}",
            resp.status
        )));
    }
    let data: Value = serde_json::from_slice(&resp.body)?;
    let found = &data["data"]["repository"]["discussion"];
    if found.is_null() {
        return Err(SyncError::new(format!("discussion #{number} not found")));
    }
    serde_json::from_value(json!({
        "number": found["number"],
        "title": found["title"],
        "body": found["body"],
        "html_url": found["url"],
        "user": found["author"],
        "category": found["category"],
    }))
    .map_err(Into::into)
}
