//! `outl page merge`: the wrapper around [`outl_actions::merge_pages()`].
//!
//! Glue only. What a merge does (blocks, properties, aliases, link
//! rewrite, trashing the source) is owned by `outl-actions`
//! (RFC 0557); this file owns the projection of every page the merge
//! touched, the removal of the source's `.md` + sidecar, the JSON reply
//! and the human listing. `outl page rename --merge` shares [`finish`].
//!
//! There is no dry run: `merge_pages` applies its ops to the workspace
//! inside one batch and has no preview path, so reporting "what would
//! happen" would mean a second implementation of the merge.

use std::path::Path;

use serde_json::{json, Value};

use outl_actions::{find_by_slug, ActionError, MergeReport};

use super::page::write_projection;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl page merge`: require `--confirm` (like `page delete`), then merge.
pub fn run(path: &Path, source: &str, target: &str, confirm: bool, json: bool) -> i32 {
    if !confirm {
        let err = ApiError::new(
            codes::CONFIRM_REQUIRED,
            format!("refusing to merge page `{source}` into `{target}` without --confirm"),
        );
        return emit::<Value, _>(json, Err(err), |_| {});
    }
    let result = ws::open(path).and_then(|mut ctx| merge(&mut ctx, source, target));
    emit(json, result, print_human)
}

/// Merge page `source` into page `target`.
///
/// `source` may already be in the trash (a re-run that moves blocks that
/// arrived late), which is why it is not resolved with `page::resolve`.
pub fn merge(ctx: &mut WsCtx, source: &str, target: &str) -> Result<Value, ApiError> {
    let report =
        outl_actions::merge_pages(&mut ctx.workspace, &ctx.hlc, source, target).map_err(refusal)?;
    finish(ctx, &report)
}

/// Project everything a merge touched and build the reply.
///
/// The source's projection goes last, and only once the target itself
/// projected, so a refused write never costs the user the only `.md`
/// that still holds the blocks.
pub(super) fn finish(ctx: &mut WsCtx, report: &MergeReport) -> Result<Value, ApiError> {
    let mut first_failure: Option<ApiError> = None;
    let mut target_projected = false;
    for slug in &report.touched_pages {
        let Some(page) = find_by_slug(&ctx.workspace, slug) else {
            continue;
        };
        match write_projection(ctx, page) {
            Ok(()) => target_projected |= *slug == report.target.slug,
            Err(e) => {
                first_failure.get_or_insert(e);
            }
        }
    }
    if target_projected {
        // Logged on failure for the same reason as in `page delete`.
        if let Err(e) = outl_actions::remove_page_projection(&ctx.root, &report.source) {
            tracing::warn!(
                target: "outl::cmd::page",
                "could not remove merged page projection for {}: {e}",
                report.source.slug
            );
        }
    }
    if let Some(e) = first_failure {
        return Err(e);
    }
    reply(report)
}

/// The JSON payload: the report, with `rewritten_blocks` spelled
/// `rewritten_refs` like `page rename` does.
fn reply(report: &MergeReport) -> Result<Value, ApiError> {
    let mut value = serde_json::to_value(report).map_err(ApiError::internal)?;
    if let Some(map) = value.as_object_mut() {
        if let Some(blocks) = map.remove("rewritten_blocks") {
            map.insert("rewritten_refs".to_string(), blocks);
        }
        map.insert("outcome".to_string(), json!("merged"));
        map.insert(
            "nothing_changed".to_string(),
            json!(report.nothing_changed()),
        );
    }
    Ok(value)
}

/// Map an action refusal onto the CLI's stable codes.
///
/// `PageNotFound` is spelled out here because the merge source may be
/// trashed, so the caller cannot pre-resolve it; `PageExists` gets the
/// way out for this surface appended to the diagnosis.
pub(super) fn refusal(e: ActionError) -> ApiError {
    match e {
        ActionError::PageNotFound(name) => {
            ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{name}` not found"))
        }
        ActionError::PageExists { existing } => ApiError::new(
            codes::SLUG_CONFLICT,
            format!(
                "page `{existing}` already exists, so it cannot be a rename target; \
                 pass --merge (CLI) or merge:true (MCP, batch) to fold the page into it, \
                 or run `outl page merge <source> {existing} --confirm`"
            ),
        ),
        other => ApiError::from(other),
    }
}

/// Human listing for `outl page merge` and `outl page rename --merge`.
pub fn print_human(v: &Value) {
    let slug_of = |ptr: &str| v.pointer(ptr).and_then(Value::as_str).unwrap_or("?");
    let (source, target) = (slug_of("/source/slug"), slug_of("/target/slug"));
    if v.get("nothing_changed").and_then(Value::as_bool) == Some(true) {
        println!("already merged: {source} into {target}, nothing to do");
    } else {
        println!("merged: {source} into {target}");
    }
    let count = |key: &str| v.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    let moved = v.get("moved_blocks").and_then(Value::as_u64).unwrap_or(0);
    if moved > 0 {
        println!("moved {moved} block(s)");
    }
    if count("rewritten_refs") > 0 {
        println!("rewrote {} reference(s)", count("rewritten_refs"));
    }
    if let Some(aliases) = v.get("aliases_added").and_then(Value::as_array) {
        let names: Vec<&str> = aliases.iter().filter_map(Value::as_str).collect();
        if !names.is_empty() {
            println!("aliases added: {}", names.join(", "));
        }
    }
    if count("skipped_namespace_refs") > 0 {
        println!(
            "left {} namespace reference(s) alone (they name a sub-page)",
            count("skipped_namespace_refs")
        );
    }
    print_conflicts(v);
}

fn print_conflicts(v: &Value) {
    let Some(conflicts) = v.get("property_conflicts").and_then(Value::as_array) else {
        return;
    };
    if conflicts.is_empty() {
        return;
    }
    println!("property conflicts (the target's value was kept):");
    for c in conflicts {
        let field = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("");
        println!(
            "  {}: kept `{}`, dropped `{}`",
            field("key"),
            field("kept"),
            field("dropped")
        );
    }
}

/// Parse tests for the `merge` surface of `outl page`. They live here
/// rather than in `cli.rs`, which sits right under the file-size ceiling.
#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::{Cli, Command};
    use crate::cmd::page::PageCommand;

    fn page_command(args: &[&str]) -> PageCommand {
        let mut argv = vec!["outl", "page"];
        argv.extend_from_slice(args);
        match Cli::try_parse_from(argv).expect("must parse").command {
            Some(Command::Page { sub }) => sub,
            _ => panic!("expected a `page` subcommand"),
        }
    }

    #[test]
    fn merge_takes_confirm() {
        let PageCommand::Merge {
            source,
            target,
            confirm,
            json,
        } = page_command(&["merge", "a", "b", "--confirm"])
        else {
            panic!("expected `page merge`");
        };
        assert_eq!((source.as_str(), target.as_str()), ("a", "b"));
        assert!(confirm);
        assert!(!json);
    }

    #[test]
    fn merge_takes_yes_as_an_alias_of_confirm() {
        assert!(matches!(
            page_command(&["merge", "a", "b", "--yes"]),
            PageCommand::Merge { confirm: true, .. }
        ));
    }

    #[test]
    fn merge_is_unconfirmed_unless_asked() {
        assert!(matches!(
            page_command(&["merge", "a", "b"]),
            PageCommand::Merge { confirm: false, .. }
        ));
    }

    #[test]
    fn rename_takes_merge() {
        let PageCommand::Rename {
            old_slug,
            new_slug,
            merge,
            ..
        } = page_command(&["rename", "a", "b", "--merge"])
        else {
            panic!("expected `page rename`");
        };
        assert_eq!((old_slug.as_str(), new_slug.as_str()), ("a", "b"));
        assert!(merge);
        assert!(matches!(
            page_command(&["rename", "a", "b"]),
            PageCommand::Rename { merge: false, .. }
        ));
    }
}
