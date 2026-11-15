//! What gets posted: the Discord payloads built from a GitHub discussion.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::text::{build_content, thread_name, truncate, CONTENT_LIMIT};

const USERNAME: &str = "GitHub Discussions";

#[derive(Debug, Deserialize)]
pub struct User {
    pub login: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Category {
    pub name: Option<String>,
}

/// The webhook payload's `discussion` object (GraphQL results are reshaped to it).
#[derive(Debug, Deserialize)]
pub struct Discussion {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub html_url: String,
    pub user: Option<User>,
    pub category: Option<Category>,
}

#[derive(Debug, Deserialize)]
pub struct Comment {
    pub html_url: String,
    pub body: Option<String>,
    pub user: Option<User>,
    pub parent_id: Option<u64>,
}

fn login(user: &Option<User>) -> String {
    let name = user
        .as_ref()
        .and_then(|user| user.login.as_deref())
        .filter(|name| !name.is_empty())
        .unwrap_or("ghost");
    truncate(name, 60)
}

fn payload(content: String, name: Option<String>) -> Value {
    let mut body = json!({
        "username": USERNAME,
        "content": content,
        // Empty `parse` means nothing in the text can ping: not @everyone,
        // @here, a role, or a user quoted from a discussion body.
        "allowed_mentions": { "parse": [] },
    });
    if let Some(name) = name {
        body["thread_name"] = Value::String(name);
    }
    body
}

pub fn plain_payload(content: &str) -> Value {
    payload(content.to_string(), None)
}

pub fn discussion_payload(discussion: &Discussion) -> Value {
    let category = discussion
        .category
        .as_ref()
        .and_then(|category| category.name.as_deref())
        .filter(|name| !name.is_empty())
        .unwrap_or("General");
    let head = format!(
        "**{}** opened a discussion in **{}**",
        login(&discussion.user),
        truncate(category, 100)
    );
    let tail = format!("Reply on GitHub: <{}>", discussion.html_url);
    let content = build_content(
        &head,
        discussion.body.as_deref().unwrap_or(""),
        &tail,
        CONTENT_LIMIT,
    );
    let name = thread_name(discussion.number, &discussion.title);
    payload(content, Some(name))
}

pub fn comment_payload(comment: &Comment) -> Value {
    let verb = if comment.parent_id.is_some() {
        "replied"
    } else {
        "commented"
    };
    let head = format!("**{}** {verb}", login(&comment.user));
    let tail = format!("Comment on GitHub: <{}>", comment.html_url);
    let content = build_content(
        &head,
        comment.body.as_deref().unwrap_or(""),
        &tail,
        CONTENT_LIMIT,
    );
    payload(content, None)
}
