//! Every GUI client takes every shared command module, or says why not.
//!
//! `outl_tauri_shared::wrappers::catalog` is the one list of Tauri
//! commands. Before it existed, each client hand-wrote its wrappers, and
//! the difference between the two was discovered by a user pressing
//! something that did nothing: `commands/history.rs` was 183 lines on the
//! desktop and 22 on mobile, `commands/exec.rs` 3 commands against 1.
//! Nobody decided that — the wrappers were never typed, and nothing
//! could fail.
//!
//! This is root `CLAUDE.md` invariant 12 one layer below where it
//! normally lives. `outl_shortcuts::capability_support` makes a missing
//! *action* a compile error; it cannot see a *command* that was never
//! registered, because an unregistered command leaves no trace in any
//! exhaustive `match`. So the parity is checked here, against the source
//! the clients actually build.
//!
//! A gap is still allowed. It just has to be written down, in
//! [`DECLARED_GAPS`], with the reason — which is the whole difference
//! between a capability difference and a bug.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Clients that consume the shared catalog: crate name, `commands/`
/// directory, and the `lib.rs` that holds the `generate_handler!` list.
const CLIENTS: &[(&str, &str, &str)] = &[
    (
        "outl-desktop",
        "../outl-desktop/src-tauri/src/commands",
        "../outl-desktop/src-tauri/src/lib.rs",
    ),
    (
        "outl-mobile",
        "../outl-mobile/src-tauri/src/commands",
        "../outl-mobile/src-tauri/src/lib.rs",
    ),
];

/// `(client, macro)` pairs that are deliberately not taken.
///
/// Empty today, and that is the point: both clients register the whole
/// surface. Adding a row is a decision, and the reason belongs next to
/// it — "the frontend does not call it yet" is not one, since an
/// unregistered command costs a feature while an unused one costs a
/// symbol.
const DECLARED_GAPS: &[(&str, &str, &str)] = &[];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Names of every `*_commands!` macro the catalog defines.
fn catalog_macros() -> BTreeSet<String> {
    let path = manifest_dir().join("src/wrappers/catalog.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let found: BTreeSet<String> = src
        .lines()
        .filter_map(|line| line.trim().strip_prefix("macro_rules! "))
        .filter_map(|rest| rest.strip_suffix(" {"))
        .filter(|name| name.ends_with("_commands"))
        .map(str::to_string)
        .collect();
    assert!(
        !found.is_empty(),
        "no `*_commands!` macros found in {} — the parser is reading the \
         wrong shape and this test proves nothing",
        path.display()
    );
    found
}

/// Macros a client invokes anywhere under its `commands/` directory.
fn macros_used_by(dir: &Path) -> BTreeSet<String> {
    let declared = declared_modules(dir);
    let mut used = BTreeSet::new();
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 file stem");
        // A `commands/foo.rs` that `commands/mod.rs` never declares with
        // `mod foo;` is not part of the crate — it compiles to nothing.
        // A directory-only scan would credit the client with a surface it
        // does not have, which is exactly the wiring mobile needed added
        // by hand for `shortcuts` and `timeline`.
        if stem != "mod" && !declared.contains(stem) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("readable command module");
        for (idx, _) in src.match_indices("outl_tauri_shared::") {
            let rest = &src[idx + "outl_tauri_shared::".len()..];
            let Some(bang) = rest.find('!') else { continue };
            let name = &rest[..bang];
            if name.ends_with("_commands") && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                used.insert(name.to_string());
            }
        }
    }
    used
}

/// Module names `commands/mod.rs` declares with `mod <name>;`.
fn declared_modules(dir: &Path) -> BTreeSet<String> {
    let path = dir.join("mod.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    src.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("pub(crate) mod ")
                .or_else(|| line.strip_prefix("pub mod "))
                .or_else(|| line.strip_prefix("mod "))?;
            rest.strip_suffix(';').map(str::to_string)
        })
        .collect()
}

#[test]
fn every_client_takes_every_shared_command_module() {
    let catalog = catalog_macros();

    for (client, rel, _lib) in CLIENTS {
        let dir = manifest_dir().join(rel);
        let used = macros_used_by(&dir);

        let missing: Vec<&String> = catalog
            .difference(&used)
            .filter(|m| {
                !DECLARED_GAPS
                    .iter()
                    .any(|(c, g, _)| c == client && *g == m.as_str())
            })
            .collect();

        assert!(
            missing.is_empty(),
            "{client} does not invoke {missing:?}.\n\
             Those commands exist in `outl_tauri_shared::commands` and this \
             client does not register them, so the feature is absent on this \
             client and nothing else says so. Either add the \
             `outl_tauri_shared::<name>!(crate::state::AppState);` line, or \
             record the gap in DECLARED_GAPS with the reason."
        );
    }
}

/// Every declared gap must still be a real, current gap.
///
/// Two ways a row goes bad, and both hide the next real gap behind a
/// stale exemption: the client started taking the module again (the row
/// is obsolete), or the row names a client or macro that does not exist
/// (a typo, exempting nothing while looking like it exempts something).
#[test]
fn every_declared_gap_is_real_and_current() {
    let catalog = catalog_macros();
    for (client, macro_name, why) in DECLARED_GAPS {
        let rel = CLIENTS
            .iter()
            .find(|(c, _, _)| c == client)
            .map(|(_, rel, _)| *rel)
            .unwrap_or_else(|| panic!("DECLARED_GAPS names unknown client {client}"));
        assert!(
            catalog.contains(*macro_name),
            "DECLARED_GAPS names unknown macro {macro_name}"
        );
        assert!(
            !why.trim().is_empty(),
            "the gap ({client}, {macro_name}) has no reason — the reason is \
             the only thing separating a decision from a defect"
        );
        assert!(
            !macros_used_by(&manifest_dir().join(rel)).contains(*macro_name),
            "{client} now invokes {macro_name}! — drop its DECLARED_GAPS row"
        );
    }
}

/// Every generated command reaches Tauri's dispatcher.
///
/// This is the hole the macro widened, and it is the reason this test
/// exists. Before, a command missing from a client was a missing
/// function — visible. Now the macro defines all of them
/// unconditionally, so the only place one can go missing is the
/// `generate_handler![…]` list in `lib.rs`. A wrapper that is defined
/// and not listed compiles, exports nothing, and fails at runtime as
/// `Command <name> not found` in a console the user never opens.
///
/// `outl-mobile`'s `capability_parity.rs` documents this exact gap as
/// out of its reach ("it says nothing about whether Tauri still
/// dispatches to it"). This closes it.
#[test]
fn every_generated_command_is_registered_with_tauri() {
    let generated = generated_command_names();
    assert!(
        generated.len() > 50,
        "only {} command names parsed out of the catalog — the parser is \
         reading the wrong shape and this test proves nothing",
        generated.len()
    );

    for (client, _rel, lib) in CLIENTS {
        let path = manifest_dir().join(lib);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let listed = handler_list(&src, client);

        let unregistered: Vec<&String> = generated.difference(&listed).collect();
        assert!(
            unregistered.is_empty(),
            "{client} generates {unregistered:?} but does not list them in \
             generate_handler![…].\n\
             The wrapper exists and Tauri will not dispatch to it: the \
             frontend's invoke() fails at runtime with no compile-time \
             warning. Add them to the handler list in {lib}."
        );
    }
}

/// Command names the catalog's `*_commands!` macros generate.
fn generated_command_names() -> BTreeSet<String> {
    let path = manifest_dir().join("src/wrappers/catalog.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    src.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("fn ")
                .or_else(|| line.strip_prefix("async fn "))?;
            let name = rest.split('(').next()?;
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then(|| name.to_string())
        })
        .collect()
}

/// Bare identifiers inside `generate_handler![ … ]`.
///
/// Path-qualified entries (`workspace_picker::set_workspace`) keep only
/// their last segment, which is the command name Tauri registers.
fn handler_list(src: &str, client: &str) -> BTreeSet<String> {
    let start = src
        .find("generate_handler![")
        .unwrap_or_else(|| panic!("{client}: no generate_handler![ in lib.rs"))
        + "generate_handler![".len();
    let end = start
        + src[start..]
            .find(']')
            .unwrap_or_else(|| panic!("{client}: generate_handler![ is never closed"));

    src[start..end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .flat_map(|line| line.split(','))
        .map(|entry| entry.trim().rsplit("::").next().unwrap_or("").trim())
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .map(str::to_string)
        .collect()
}
