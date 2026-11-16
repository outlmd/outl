//! `outl theme` — inspect color presets without opening the TUI.
//!
//! - `outl theme list` prints every preset name.
//! - `outl theme show <name>` prints what each semantic surface looks
//!   like under that preset (style names; the actual colors render in
//!   any compatible terminal).

use crate::ThemeSubcommand;
use anyhow::Result;
use outl_theme::Registry;
// Name resolution comes from `outl-theme`, its owner. Rendering still
// goes through the TUI, because the printed ratatui `Style` values only
// exist once a `Palette` has been lowered to a terminal theme.
use outl_tui::theme_from_palette;

/// The catalog this command resolves against: the built-ins inside the
/// binary plus every `.toml` in `~/.config/outl/themes/`.
///
/// **A file that failed to load is named on stderr**, which is the whole
/// reason the CLI builds the registry itself instead of calling
/// `outl_theme::by_name`. A user whose theme has a typo on line 4 would
/// otherwise see it simply missing from the list, with nothing to fix.
/// Stderr, not stdout: `outl theme list | grep` stays usable.
fn catalog() -> Registry {
    let (reg, errors) = Registry::with_user_dir(&outl_config::theme_dir());
    for e in errors {
        eprintln!("warning: {e}");
    }
    reg
}

/// Run the `theme` subcommand. `None` defaults to `list`.
pub fn run(sub: Option<&ThemeSubcommand>) -> Result<()> {
    match sub {
        None | Some(ThemeSubcommand::List) => list(),
        Some(ThemeSubcommand::Show { name }) => show(name),
    }
}

fn list() -> Result<()> {
    let catalog = catalog();
    println!("Available theme presets:");
    for name in catalog.names() {
        println!("  {name}");
    }
    println!();
    println!("Use --theme <name> to override for one run, or set");
    println!("`[theme] preset = \"<name>\"` in .outl/config.toml for the workspace.");
    println!(
        "Write your own in {}/<name>.toml.",
        outl_config::theme_dir().display()
    );
    Ok(())
}

fn show(name: &str) -> Result<()> {
    match catalog().by_name(name).map(theme_from_palette) {
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
