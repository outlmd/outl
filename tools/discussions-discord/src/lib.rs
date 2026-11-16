//! Mirror GitHub Discussions into a Discord forum channel, one way.
//!
//! Run by `.github/workflows/discussions-discord.yml`.
//! The event comes from `GITHUB_EVENT_PATH` / `GITHUB_EVENT_NAME`, never from
//! the workflow's `run:` line, so a discussion title cannot inject shell.
//!
//! Nothing is written back to GitHub.
//! The only state is the discussion number -> Discord thread id map, kept as one
//! workflow artifact per discussion (`discord-thread-<number>`): this tool reads
//! it through the Actions API (only from runs of that workflow, see
//! `github::trusted_artifact`) and writes the file the workflow then uploads.
//! See `docs/discussions-discord.md` for why an artifact and not something else.

pub mod artifact_zip;
pub mod ctx;
pub mod discord;
pub mod error;
pub mod github;
pub mod http;
pub mod payload;
pub mod state;
pub mod text;
pub mod ureq_http;

#[cfg(test)]
mod tests;

use std::fs;

use serde_json::Value;

use ctx::Ctx;
use error::{Result, SyncError};
use payload::{Comment, Discussion};

fn webhook(ctx: &Ctx) -> String {
    ctx.var("DISCORD_DISCUSSIONS_WEBHOOK")
        .unwrap_or("")
        .trim()
        .to_string()
}

fn number_of(value: &Value) -> Result<u64> {
    match value {
        Value::Number(num) => num.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .ok_or_else(|| SyncError::new("the discussion number is missing or not a number"))
}

fn handle_created(ctx: &Ctx, hook: &str, event: &Value, discussion: &Discussion) -> Result<()> {
    let number = discussion.number;
    if github::lookup_thread(ctx, event, number)?.is_some() {
        ctx.log(&format!(
            "::notice::discussion #{number} already has a Discord thread, nothing to do"
        ));
        return Ok(());
    }
    let thread_id = discord::create_thread(ctx, hook, discussion)?;
    state::save_mapping(ctx, number, &thread_id)?;
    ctx.log(&format!("Opened a Discord thread for discussion #{number}"));
    Ok(())
}

fn handle_comment(
    ctx: &Ctx,
    hook: &str,
    event: &Value,
    discussion: &Discussion,
    comment: &Comment,
) -> Result<()> {
    let number = discussion.number;
    let thread_id = match github::lookup_thread(ctx, event, number)? {
        Some(id) => id,
        None => {
            let id = discord::create_thread(ctx, hook, discussion)?;
            state::save_mapping(ctx, number, &id)?;
            id
        }
    };
    let body = payload::comment_payload(comment);
    discord::post(ctx, hook, &body, Some(&thread_id))?;
    // Re-saved on every event so the 90 day retention restarts with activity.
    state::save_mapping(ctx, number, &thread_id)?;
    ctx.log(&format!("Posted a comment on discussion #{number}"));
    Ok(())
}

fn run(ctx: &Ctx) -> Result<()> {
    let hook = webhook(ctx);
    if hook.is_empty() {
        ctx.log("::warning::DISCORD_DISCUSSIONS_WEBHOOK is not set, skipping the Discord mirror");
        return Ok(());
    }
    ctx.log(&format!("::add-mask::{hook}"));

    let event_path = ctx
        .var("GITHUB_EVENT_PATH")
        .ok_or_else(|| SyncError::new("GITHUB_EVENT_PATH is not set"))?;
    let event: Value = serde_json::from_slice(&fs::read(event_path)?)?;
    match ctx.var("GITHUB_EVENT_NAME").unwrap_or("") {
        "workflow_dispatch" => {
            let number = number_of(&event["inputs"]["discussion"])?;
            let discussion = github::fetch_discussion(ctx, number)?;
            handle_created(ctx, &hook, &event, &discussion)
        }
        "discussion" => {
            let discussion: Discussion = serde_json::from_value(event["discussion"].clone())?;
            handle_created(ctx, &hook, &event, &discussion)
        }
        "discussion_comment" => {
            let discussion: Discussion = serde_json::from_value(event["discussion"].clone())?;
            let comment: Comment = serde_json::from_value(event["comment"].clone())?;
            handle_comment(ctx, &hook, &event, &discussion, &comment)
        }
        other => Err(SyncError::new(format!("unsupported event {other:?}"))),
    }
}

/// Run the mirror and return the process exit code.
///
/// A failure is printed as an `::error::` annotation with the webhook URL
/// replaced, because the message can quote a transport error or Discord's reply.
pub fn execute(ctx: &Ctx) -> i32 {
    match run(ctx) {
        Ok(()) => 0,
        Err(err) => {
            ctx.log(&format!(
                "::error::{}",
                discord::redact(&err.to_string(), &webhook(ctx))
            ));
            1
        }
    }
}
