//! The mapping file the workflow uploads, and the step outputs that say so.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde_json::json;

use crate::ctx::Ctx;
use crate::error::Result;
use crate::github::{ARTIFACT_PREFIX, THREAD_FILE};

/// Write the artifact file and tell the workflow to upload it.
///
/// Called as soon as a thread exists, before the comment is posted, so a failed
/// comment cannot orphan a thread the next run would then duplicate.
pub fn save_mapping(ctx: &Ctx, number: u64, thread_id: &str) -> Result<()> {
    let base = ctx
        .var("RUNNER_TEMP")
        .filter(|dir| !dir.is_empty())
        .unwrap_or("/tmp");
    let dir = PathBuf::from(base).join("discord-thread");
    fs::create_dir_all(&dir)?;
    let path = dir.join(THREAD_FILE);
    fs::write(
        &path,
        serde_json::to_vec(&json!({ "discussion": number, "thread_id": thread_id }))?,
    )?;
    set_outputs(
        ctx,
        &[
            ("save", "true".to_string()),
            ("artifact_name", format!("{ARTIFACT_PREFIX}{number}")),
            ("thread_file", path.display().to_string()),
        ],
    )
}

fn set_outputs(ctx: &Ctx, outputs: &[(&str, String)]) -> Result<()> {
    let Some(path) = ctx.var("GITHUB_OUTPUT").filter(|path| !path.is_empty()) else {
        return Ok(());
    };
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for (key, value) in outputs {
        writeln!(file, "{key}={value}")?;
    }
    Ok(())
}
