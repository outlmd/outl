//! `outl theme` — inspect color presets without opening the TUI.
//!
//! - `outl theme list` prints every preset name.
//! - `outl theme show <name>` prints what each semantic surface looks
//!   like under that preset (style names; the actual colors render in
//!   any compatible terminal).

use crate::ThemeSubcommand;
use anyhow::Result;
// The preset *list* comes from `outl-theme`, its owner. `show` still
// goes through the TUI's `by_name` because it prints ratatui `Style`
// values, which only exist once a `Palette` has been lowered to a
// terminal theme.
use outl_theme::PRESETS;
use outl_tui::theme_by_name;

/// Run the `theme` subcommand. `None` defaults to `list`.
pub fn run(sub: Option<&ThemeSubcommand>) -> Result<()> {
    match sub {
        None | Some(ThemeSubcommand::List) => list(),
        Some(ThemeSubcommand::Show { name }) => show(name),
    }
}

fn list() -> Result<()> {
    println!("Available theme presets:");
    for name in PRESETS {
        println!("  {name}");
    }
    println!();
    println!("Use --theme <name> to override for one run, or set");
    println!("`[theme] preset = \"<name>\"` in .outl/config.toml for the workspace.");
    Ok(())
}

fn show(name: &str) -> Result<()> {
    match theme_by_name(name) {
        Some(t) => {
            println!("Theme: {}", t.name);
            println!();
            println!("Semantic surfaces (Style structs):");
            // Print a representative subset — full list lives in
            // outl_tui::theme::Theme and is enforced by the type.
            println!("  bullet           = {:?}", t.bullet);
            println!("  selected_bullet  = {:?}", t.selected_bullet);
            println!("  cursor_block     = {:?}", t.cursor_block);
            println!("  cursor_caret     = {:?}", t.cursor_caret);
            println!("  ref_link         = {:?}", t.ref_link);
            println!("  tag_link         = {:?}", t.tag_link);
            println!("  md_link          = {:?}", t.md_link);
            println!("  bold             = {:?}", t.bold);
            println!("  italic           = {:?}", t.italic);
            println!("  strike           = {:?}", t.strike);
            println!("  code             = {:?}", t.code);
            println!("  todo_open        = {:?}", t.todo_open);
            println!("  todo_done        = {:?}", t.todo_done);
            println!("  property_key     = {:?}", t.property_key);
            println!("  property_value   = {:?}", t.property_value);
            println!("  heading          = {:?}", t.heading);
            println!("  hint             = {:?}", t.hint);
            println!("  status_normal    = {:?}", t.status_normal);
            println!("  status_insert    = {:?}", t.status_insert);
            println!("  status_visual    = {:?}", t.status_visual);
            Ok(())
        }
        None => Err(anyhow::anyhow!(
            "unknown theme preset: {name}\nrun `outl theme list` to see what's available"
        )),
    }
}
