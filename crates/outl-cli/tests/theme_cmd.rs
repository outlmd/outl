//! `outl theme` over a user's own theme files.
//!
//! The built-ins ship inside the binary; a user's go in
//! `~/.config/outl/themes/*.toml`, same format, resolved by name against
//! the built-in they `extends`. These tests drive the real binary because
//! the wiring is the thing under test: the format and the registry have
//! their own unit tests in `outl-theme`, and neither of them proves that
//! `outl theme list` actually asks for the user's directory.
//!
//! **Every command here runs with `XDG_CONFIG_HOME` pointed at a
//! tempdir**, which `outl_config::config_dir()` honours ahead of every
//! other branch, so the suite never reads the developer's own themes
//! (root `CLAUDE.md` invariant 9, third question).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn outl(config_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_outl"))
        .env("XDG_CONFIG_HOME", config_home)
        .args(args)
        .output()
        .expect("spawning outl")
}

/// Write `themes/<name>.toml` under the isolated config home.
fn write_theme(config_home: &Path, name: &str, body: &str) {
    let dir = config_home.join("outl").join("themes");
    fs::create_dir_all(&dir).expect("mkdir themes");
    fs::write(dir.join(format!("{name}.toml")), body).expect("write theme");
}

#[test]
fn theme_list_shows_a_user_theme_next_to_the_builtins() {
    let home = TempDir::new().expect("tempdir");
    write_theme(
        home.path(),
        "mine",
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n",
    );

    let out = outl(home.path(), &["theme", "list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("mine"),
        "user theme missing from:\n{stdout}"
    );
    assert!(stdout.contains("outl"), "built-ins still listed:\n{stdout}");
    assert!(
        stdout.contains("dracula"),
        "built-ins still listed:\n{stdout}"
    );
}

#[test]
fn theme_show_renders_a_user_theme() {
    let home = TempDir::new().expect("tempdir");
    write_theme(
        home.path(),
        "mine",
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n",
    );

    let out = outl(home.path(), &["theme", "show", "mine"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("Theme: mine"),
        "show should name the theme it rendered:\n{stdout}"
    );
}

#[test]
fn a_user_theme_with_both_sides_lists_both() {
    let home = TempDir::new().expect("tempdir");
    write_theme(
        home.path(),
        "mine",
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n\
         [light]\nextends = \"outl-light\"\naccent = \"#6d28d9\"\n",
    );

    let stdout = String::from_utf8_lossy(&outl(home.path(), &["theme", "list"]).stdout).to_string();

    assert!(stdout.contains("mine"), "{stdout}");
    assert!(stdout.contains("mine-light"), "{stdout}");
}

#[test]
fn a_broken_theme_is_named_on_stderr_and_does_not_take_the_others_down() {
    let home = TempDir::new().expect("tempdir");
    write_theme(home.path(), "broken", "[dark]\naccent = \"nope\"\n");
    write_theme(
        home.path(),
        "fine",
        "[dark]\nextends = \"outl\"\naccent = \"#ff6ac1\"\n",
    );

    let out = outl(home.path(), &["theme", "list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("broken"),
        "the user has to be told which file was ignored, got stderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("broken"),
        "a theme that failed to load must not be offered:\n{stdout}"
    );
    assert!(
        stdout.contains("fine"),
        "one bad file is not the others' problem:\n{stdout}"
    );
    assert!(
        out.status.success(),
        "listing themes still succeeds; a bad file is the user's to fix"
    );
}

#[test]
fn an_unknown_theme_name_still_fails_with_a_pointer_to_list() {
    let home = TempDir::new().expect("tempdir");
    let out = outl(home.path(), &["theme", "show", "vampire"]);

    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("outl theme list"),
        "stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}
