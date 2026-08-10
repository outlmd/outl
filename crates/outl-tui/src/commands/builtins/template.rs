use anyhow::Result;

use crate::commands::SlashCommand;
use crate::state::App;

pub struct TemplateCommand;

impl SlashCommand for TemplateCommand {
    fn name(&self) -> &'static str {
        "template"
    }

    fn description(&self) -> &'static str {
        "Pick a template, or `template <name> key=value …`"
    }

    fn needs_args(&self) -> bool {
        false
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["tpl"]
    }

    fn execute(&self, app: &mut App, args: &str) -> Result<bool> {
        let args = args.trim();
        if args.is_empty() {
            app.open_template_picker();
            return Ok(false);
        }

        let (name, params) = match args.split_once(' ') {
            Some((n, rest)) => (n.trim(), parse_kv(rest)),
            None => (args, Vec::new()),
        };

        let templates = outl_actions::list_templates(&app.workspace);
        if !templates.iter().any(|t| t.name == name) {
            app.status = format!("template `{name}` not found");
            return Ok(false);
        }

        // Callable vs structural is decided by the *presence of a
        // runnable code block* in the template, not by whether `params::`
        // is declared. A callable template with a code block but no
        // `params::` must still execute — routing on param emptiness
        // would deep-copy its fence as literal text instead.
        if outl_actions::resolve_call(&app.workspace, name).is_ok() {
            app.execute_callable_template(name, &params);
        } else {
            app.instantiate_template_at_cursor(name);
        }
        Ok(false)
    }
}

fn parse_kv(s: &str) -> Vec<(String, String)> {
    s.split_whitespace()
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                None
            } else {
                Some((k.to_string(), v.trim().to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::SlashCommand;
    use outl_actions::block::append_block;
    use outl_actions::page::{open_or_create, set_property, PageKind};
    use outl_actions::tree::children_of;
    use outl_actions::TEMPLATE_KEY;
    use outl_core::id::ActorId;
    use outl_core::property::PropValue;
    use outl_core::workspace::Workspace;
    use tempfile::TempDir;

    fn app_with(root: &TempDir) -> App {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        App::new(
            root.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap()
    }

    /// A callable template with a code block but NO `params::` must be
    /// dispatched as callable (execute the fence), never as structural
    /// (deep-copy the fence text as a child block). Regression for the
    /// footgun where dispatch keyed off `params::` emptiness.
    #[test]
    fn callable_template_without_params_is_not_deep_copied() {
        let dir = TempDir::new().unwrap();
        let mut app = app_with(&dir);

        // Callable template: a code block, and deliberately NO `params::`.
        // `fortran` is never a linked runtime, so execution fails with an
        // Exec error naming the language — proving resolution ran (callable
        // path) instead of deep-copying the fence text.
        let tpl = open_or_create(
            &mut app.workspace,
            &app.hlc,
            "template-calc",
            "calc",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut app.workspace,
            &app.hlc,
            tpl,
            TEMPLATE_KEY,
            Some(PropValue::Text("calc".into())),
        )
        .unwrap();
        append_block(
            &mut app.workspace,
            &app.hlc,
            Some(tpl),
            Some("```fortran\nprint *, 1\n```"),
        )
        .unwrap();

        let anchor = app.id_by_flat.get(app.selected).copied();

        TemplateCommand.execute(&mut app, "calc").unwrap();

        // Structural (deep-copy) would append a child whose text is the
        // raw fence. The callable path never does. Assert the fence was
        // NOT deep-copied under the selected block.
        if let Some(anchor) = anchor {
            let deep_copied = children_of(&app.workspace, anchor)
                .into_iter()
                .filter_map(|(id, _)| app.workspace.block_text(id))
                .any(|t| t.contains("```fortran"));
            assert!(
                !deep_copied,
                "callable template without params was deep-copied instead of executed"
            );
        }

        // The status reflects the callable path (execution attempt), not a
        // structural instantiation message.
        assert!(
            !app.status.contains("instantiated template"),
            "should route callable, not structural; status: {}",
            app.status
        );
    }
}
