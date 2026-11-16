//! Shared test harness: a fake network and a temp workspace for one run.

mod artifacts;
mod discord;
mod flow;
mod provenance;
mod text;
mod zip;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::ctx::{Ctx, Env};
use crate::error::Result;
use crate::github::WORKFLOW_PATH;
use crate::http::{Http, Response};
use crate::payload::{Comment, Discussion};

pub const WEBHOOK: &str = "https://discord.com/api/webhooks/123/secret-token";
pub const THREAD: &str = "555000111222333444";
pub const NEW_THREAD: &str = "999888777666555444";
pub const REPO_ID: u64 = 4242;

pub fn discussion_json() -> Value {
    json!({
        "number": 42,
        "title": "How do I sync two devices?",
        "body": "Hello @everyone and @here, ping <@&123> please.",
        "html_url": "https://github.com/outlmd/outl/discussions/42",
        "user": {"login": "alice"},
        "category": {"name": "Q&A"},
    })
}

pub fn comment_json() -> Value {
    json!({
        "html_url": "https://github.com/outlmd/outl/discussions/42#discussioncomment-7",
        "body": "Try `outl peer`.",
        "user": {"login": "bob"},
    })
}

pub fn content(payload: &Value) -> &str {
    payload["content"].as_str().expect("content")
}

pub fn discussion() -> Discussion {
    serde_json::from_value(discussion_json()).expect("fixture parses")
}

pub fn comment() -> Comment {
    serde_json::from_value(comment_json()).expect("fixture parses")
}

pub fn json_reply(status: u16, body: Value) -> Result<Response> {
    let raw = serde_json::to_vec(&body).expect("json");
    Ok(Response::new(status, raw))
}

pub const DAY1: &str = "2026-01-01T00:00:00Z";
pub const DAY2: &str = "2026-02-01T00:00:00Z";

/// An artifact listing entry produced by a trusted run of the default branch.
pub fn artifact(id: u64, created_at: &str) -> Value {
    json!({
        "id": id,
        "created_at": created_at,
        "expired": false,
        "archive_download_url": format!("https://api.github.com/artifact-zip/{id}"),
        "workflow_run": {
            "id": id * 100,
            "repository_id": REPO_ID,
            "head_repository_id": REPO_ID,
            "head_branch": "main",
        },
    })
}

pub fn run_info(path: &str, event: &str) -> Value {
    json!({"path": path, "event": event})
}

pub fn mapping_zip(number: u64, thread_id: &str) -> Vec<u8> {
    zip::stored_zip(
        "thread.json",
        json!({"discussion": number, "thread_id": thread_id})
            .to_string()
            .as_bytes(),
    )
}

pub type Call = (String, String, Vec<(String, String)>, Option<Vec<u8>>);

/// Routes the calls the tool makes and records them.
#[derive(Default)]
pub struct Net {
    pub calls: RefCell<Vec<Call>>,
    pub artifacts: Vec<Value>,
    /// Artifact id -> the bytes its download returns.
    pub zips: HashMap<u64, Vec<u8>>,
    /// Run id -> what `GET /actions/runs/{id}` returns; anything else is trusted.
    pub runs: HashMap<u64, Value>,
    pub discord: RefCell<VecDeque<Result<Response>>>,
    pub graphql: Option<Value>,
}

impl Net {
    pub fn with_discord(replies: Vec<Result<Response>>) -> Self {
        Net {
            discord: RefCell::new(replies.into()),
            ..Net::default()
        }
    }

    /// A net holding one genuine artifact that maps discussion 42 to `thread`.
    pub fn one_mapping(thread: &str) -> Self {
        Net::mapped(vec![(artifact(1, DAY1), mapping_zip(42, thread))])
    }

    pub fn mapped(artifacts: Vec<(Value, Vec<u8>)>) -> Self {
        let mut net = Net::default();
        for (art, zip) in artifacts {
            net.zips.insert(art["id"].as_u64().expect("id"), zip);
            net.artifacts.push(art);
        }
        net
    }

    pub fn discord_calls(&self) -> Vec<Call> {
        self.calls
            .borrow()
            .iter()
            .filter(|call| call.1.starts_with(WEBHOOK))
            .cloned()
            .collect()
    }

    pub fn discord_bodies(&self) -> Vec<Value> {
        self.discord_calls()
            .iter()
            .map(|call| serde_json::from_slice(call.3.as_ref().expect("body")).expect("json"))
            .collect()
    }
}

fn last_id(url: &str) -> u64 {
    let last = url.rsplit('/').next().expect("a path segment");
    last.parse().expect("numeric id")
}

impl Http for Net {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Response> {
        self.calls.borrow_mut().push((
            method.to_string(),
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body.map(<[u8]>::to_vec),
        ));
        if url.starts_with(WEBHOOK) {
            return self
                .discord
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| json_reply(200, json!({"channel_id": NEW_THREAD})));
        }
        if url.contains("/actions/artifacts?") {
            return json_reply(200, json!({"artifacts": self.artifacts}));
        }
        if url.contains("/actions/runs/") {
            let info = self
                .runs
                .get(&last_id(url))
                .cloned()
                .unwrap_or_else(|| run_info(WORKFLOW_PATH, "discussion"));
            return json_reply(200, info);
        }
        if url.starts_with("https://api.github.com/artifact-zip/") {
            let id = last_id(url);
            return Ok(Response::new(302, "")
                .with_header("Location", &format!("https://blob.example/{id}")));
        }
        if url.starts_with("https://blob.example/") {
            return Ok(Response::new(200, self.zips[&last_id(url)].clone()));
        }
        if url == "https://api.github.com/graphql" {
            return json_reply(200, self.graphql.clone().expect("graphql reply"));
        }
        panic!("unexpected call {method} {url}");
    }
}

/// Answers every call with one status, except an artifact listing that has entries.
pub struct Fixed {
    pub artifacts: Vec<Value>,
    pub status: u16,
}

impl Http for Fixed {
    fn request(
        &self,
        _method: &str,
        url: &str,
        _headers: &[(&str, &str)],
        _body: Option<&[u8]>,
    ) -> Result<Response> {
        if url.contains("/actions/artifacts?") && !self.artifacts.is_empty() {
            return json_reply(200, json!({"artifacts": self.artifacts}));
        }
        Ok(Response::new(self.status, ""))
    }
}

/// A temp dir holding the event file and the step outputs for one run.
pub struct Harness {
    pub dir: TempDir,
    pub sleeps: RefCell<Vec<Duration>>,
    pub log: RefCell<Vec<String>>,
}

impl Harness {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let output = dir.path().join("output");
        std::fs::write(&output, "").expect("output file");
        Harness {
            dir,
            sleeps: RefCell::new(Vec::new()),
            log: RefCell::new(Vec::new()),
        }
    }

    pub fn env(&self, event_name: &str, event: &Value, webhook: Option<&str>) -> Env {
        let event_path = self.dir.path().join("event.json");
        let raw = event.to_string();
        std::fs::write(&event_path, raw).expect("event file");
        let path = |name: &str| self.dir.path().join(name).display().to_string();
        let mut env = Env::new();
        for (key, value) in [
            ("GITHUB_EVENT_NAME", event_name.to_string()),
            ("GITHUB_EVENT_PATH", event_path.display().to_string()),
            ("GITHUB_OUTPUT", path("output")),
            ("GITHUB_REPOSITORY", "outlmd/outl".to_string()),
            ("GITHUB_REPOSITORY_ID", REPO_ID.to_string()),
            ("GITHUB_TOKEN", "ghs_token".to_string()),
            ("RUNNER_TEMP", self.dir.path().display().to_string()),
        ] {
            env.insert(key.to_string(), value);
        }
        if let Some(hook) = webhook {
            env.insert("DISCORD_DISCUSSIONS_WEBHOOK".into(), hook.to_string());
        }
        env
    }

    pub fn with_ctx<T>(&self, env: &Env, net: &dyn Http, f: impl FnOnce(&Ctx) -> T) -> T {
        let sleep = |wait: Duration| self.sleeps.borrow_mut().push(wait);
        let log = |line: &str| self.log.borrow_mut().push(line.to_string());
        f(&Ctx {
            env,
            http: net,
            sleep: &sleep,
            log: &log,
        })
    }

    /// Run the whole tool; returns the exit code.
    pub fn execute(&self, event_name: &str, event: &Value, net: &Net) -> i32 {
        let env = self.env(event_name, event, Some(WEBHOOK));
        self.with_ctx(&env, net, crate::execute)
    }

    pub fn output(&self) -> String {
        self.log.borrow().join("\n")
    }

    pub fn outputs(&self) -> HashMap<String, String> {
        std::fs::read_to_string(self.dir.path().join("output"))
            .expect("outputs")
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub fn saved(&self) -> Value {
        let path = self.dir.path().join("discord-thread").join("thread.json");
        let raw = std::fs::read(path).expect("saved mapping");
        serde_json::from_slice(&raw).expect("json")
    }
}
