//! Pins mobile's declared column in
//! [`outl_shortcuts::capability_support`] against commands this crate
//! actually registers, so the direct `outl-shortcuts` dependency
//! added for RFC 0253 is load-bearing rather than a `use` statement
//! waiting for the next `cargo machete` run to drop it.
//!
//! Before this dependency was direct, `outl-shortcuts` only reached
//! mobile transitively (via `outl-plugins` / `outl-tauri-shared`), so
//! mobile's column in the shared parity catalog was a claim nobody's
//! build here could falsify. This module narrows that gap, not
//! closes it: each assertion below fails at compile time if the
//! named command's function item disappears from `crate::commands`,
//! and at runtime if the catalog's `Full` verdict for this client
//! changes without this crate noticing.
//!
//! **What this does not catch:** a command dropped from `lib.rs`'s
//! `generate_handler!` list while its function stays defined, for every
//! row above except the three backlink commands, whose registration is
//! read out of `lib.rs` itself. The function-item reference only
//! proves the item still exists in source.
//!
//! The `BacklinkInlineEdit` row asserts on what the client *calls*,
//! not on names that exist: it reads the frontend source (comments
//! stripped) and fails if a backlink row reaches an outline command.
//!
//! Test-only — there is nothing here for the running app to call.

#[cfg(test)]
mod tests {
    use outl_shortcuts::{capability_support, Capability, Support};

    /// Referencing a command as a function *item* (never calling it)
    /// proves the path still resolves in `crate::commands`, without
    /// needing a live `tauri::State`. It does **not** prove the
    /// command is registered in `lib.rs`'s `invoke_handler!` — see
    /// this module's doc comment for that limit.
    #[test]
    fn mobile_capability_column_is_backed_by_real_commands() {
        // The catalog called this `Missing` on mobile — "open the same
        // page in the desktop app and use the ⏱ button" — while this
        // client registered `page_timeline` and mounted
        // `HistorySheet.tsx` off the page properties sheet, which
        // `docs/mobile-internals.md` → "Trust surfaces" records.
        //
        // That is the failure mode an exhaustive `match` does **not**
        // close. Invariant 12 forces every client to *declare* a
        // verdict; it cannot force the verdict to be true. A wrong
        // `Missing` is worse than an undeclared gap, because the
        // sentence is shown to the user: it sent someone who had the
        // feature to another device to go and find it.
        let _ = crate::commands::list_trash;
        let _ = crate::commands::restore_trash_entry;
        assert_eq!(
            capability_support(Capability::Trash).mobile,
            Support::Full,
            "outl-mobile registers list_trash and restore_trash_entry and mounts \
             TrashSheet.tsx from Settings (and the palette's \"Open trash\") — the \
             catalog must not tell the user the phone cannot restore a block",
        );
        let _ = crate::commands::page_timeline;
        assert_eq!(
            capability_support(Capability::PageHistory).mobile,
            Support::Full,
            "outl-mobile registers page_timeline and mounts HistorySheet.tsx \
             from PropertiesSheet — the catalog must not call this client's \
             PageHistory missing and send the user to the desktop for a \
             feature that is right there",
        );

        // Rename is reached from the page title (tap for a name sheet)
        // and from the page switcher's long-press menu; merge from the
        // page switcher's
        // long-press sheet (RFC 0557). The catalog says `Full` for this
        // client, so both commands the sheet calls must still exist.
        let _ = crate::commands::rename_page;
        assert_eq!(
            capability_support(Capability::PageRename).mobile,
            Support::Full,
            "outl-mobile registers rename_page (the page switcher's \
             \"Rename page\" action) - the catalog's PageRename verdict for \
             mobile must track that.",
        );

        let _ = crate::commands::merge_pages;
        assert_eq!(
            capability_support(Capability::PageMerge).mobile,
            Support::Full,
            "outl-mobile registers merge_pages (the page switcher's \
             \"Merge into\" action) - the catalog's PageMerge verdict for \
             mobile must track that.",
        );

        let _ = crate::commands::plugin_registry_list;
        assert_eq!(
            capability_support(Capability::PluginMarketplace).mobile,
            Support::Full,
            "outl-mobile registers plugin_registry_list (the marketplace \
             browse tab in PluginSheet.tsx) — the shared catalog must not \
             downgrade this client's PluginMarketplace support without this \
             crate's own build noticing",
        );

        let _ = crate::commands::list_templates_cmd;
        let _ = crate::commands::instantiate_template_at;
        assert_eq!(
            capability_support(Capability::Templates).mobile,
            Support::Full,
            "outl-mobile registers list_templates_cmd / instantiate_template_at \
             (TemplateSheet) — the catalog's Templates verdict for mobile must \
             track that.",
        );

        let _ = crate::commands::attach_asset;
        let _ = crate::commands::import_asset_file;
        assert_eq!(
            capability_support(Capability::Assets).mobile,
            Support::Full,
            "outl-mobile registers attach_asset / import_asset_file (Journal.tsx's \
             paste + attach path) — the catalog's Assets verdict for mobile must \
             track that.",
        );

        let _ = crate::commands::outl_peer_pair_host;
        let _ = crate::commands::outl_peer_pair_join;
        assert_eq!(
            capability_support(Capability::PeerPairing).mobile,
            Support::Full,
            "outl-mobile registers outl_peer_pair_host / outl_peer_pair_join \
             (DevicesSheet's scan + \"show my QR\" paths) — the catalog's \
             PeerPairing verdict for mobile must track that.",
        );

        // Sharper than the rows above: reporting is on by default, so a
        // mobile build without this pair is a build with no opt-out at
        // all, and the phone has no `outl` binary to fall back on.
        let _ = crate::commands::get_telemetry_enabled;
        let _ = crate::commands::set_telemetry_enabled;
        assert_eq!(
            capability_support(Capability::CrashReportingOptOut).mobile,
            Support::Full,
            "outl-mobile registers get_telemetry_enabled / set_telemetry_enabled \
             (SettingsSheet's Privacy row) — the catalog's CrashReportingOptOut \
             verdict for mobile must track that, because a downgrade here means \
             a user with no way to turn reporting off.",
        );

        let _ = crate::commands::reminder_action_catalog;
        assert_eq!(
            capability_support(Capability::ReminderNotificationActions).mobile,
            Support::Full,
            "outl-mobile registers reminder_action_catalog (the category + buttons \
             the frontend hands to registerActionTypes, and the same ids \
             deliver_due_reminders stamps onto every banner) — the catalog's \
             ReminderNotificationActions verdict for mobile must track that.",
        );

        // The one row where the catalog's verdict covers *nine*
        // operations rather than one surface, which is exactly how a
        // per-client difference goes unrecorded: `Full` stayed `Full`
        // while this client performed a subset of them, and nothing in
        // any build could notice. The tap reaches the text edit and the
        // long-press sheet (`buildBacklinkContextActions`, eight rows)
        // reaches the rest, each addressed to the backlink's *source*
        // page.
        //
        // A one-tap **mark DONE** is not one of the nine, and this is
        // why the count is worth stating: this client has no mark-done
        // row anywhere (`Action::MarkDone` is `Missing` on mobile, and
        // `Journal.buildContextActions.test.ts` pins the eight ids), so
        // a capability row claiming ten would contradict the action
        // catalog about one operation. The TUI and the desktop perform
        // the nine as well; what the TUI lacks is a gesture that opens
        // the source page, which is its own `Partial` and not this
        // client's business.
        // Naming the cross-page commands proves they exist; it says
        // nothing about whether a row *calls* them. The outline twins
        // (`toggle_todo`, `create_block`, ...) still exist and are still
        // registered for the outline's own use, so a pin that named them
        // stayed green when the rows were routed back through them. What
        // would be the defect is the rows' source calling a twin, so that
        // is what is asserted.
        let _ = crate::commands::edit_backlink_block;
        let _ = crate::commands::backlink_row_op;
        let _ = crate::commands::create_backlink_sibling;
        backlink_rows_call_only_the_cross_page_commands();
        assert_eq!(
            capability_support(Capability::BacklinkInlineEdit).mobile,
            Support::Full,
            "outl-mobile edits a backlink row in place (the tap) and reaches \
             task state, create below / above, indent, outdent, both moves and \
             delete from its long-press sheet — all through the cross-page commands \
             asserted above, addressed to the citing block's own page. `Full` here is a claim \
             about every one of them; drop a row from the sheet and this is the \
             assertion that has to change with it.",
        );
    }

    const ROW_OPS: &str = include_str!("../../src/components/BacklinksSection.row-ops.ts");
    const SECTION: &str = include_str!("../../src/components/BacklinksSection.tsx");
    const ROW_EDITOR: &str = include_str!("../../src/components/BacklinkRowEditor.tsx");
    const LIB: &str = include_str!("lib.rs");

    /// Outline commands a backlink row must never reach: each tolerates
    /// a stale block id (appends to the page's end, reports a ULID, or
    /// answers success) where the cross-page family refuses with the
    /// catalog's sentence.
    const OUTLINE_TWINS: [&str; 7] = [
        "toggleTodo",
        "createBlock",
        "deleteBlock",
        "indentBlock",
        "outdentBlock",
        "moveBlockUp",
        "moveBlockDown",
    ];
    const CROSS_PAGE: [&str; 3] = [
        "backlinkRowOp",
        "createBacklinkSibling",
        "editBacklinkBlock",
    ];

    /// Comments quote the names under test ("never `createBlock`"), so a
    /// scan of raw source is satisfied by prose.
    fn strip_comments(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut chars = src.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('/', Some('/')) => {
                    for n in chars.by_ref() {
                        if n == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                ('/', Some('*')) => {
                    chars.next();
                    let mut prev = ' ';
                    for n in chars.by_ref() {
                        if prev == '*' && n == '/' {
                            break;
                        }
                        prev = n;
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn is_ident(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '$'
    }

    /// Whether `name(` occurs as a call, not as a suffix of a longer name.
    fn calls(src: &str, name: &str) -> bool {
        let needle = format!("{name}(");
        src.match_indices(&needle)
            .any(|(i, _)| !src[..i].chars().next_back().is_some_and(is_ident))
    }

    /// Names imported from `@outl/shared/api/commands`, if the file has
    /// such an import.
    fn commands_imports(src: &str) -> String {
        let marker = "from \"@outl/shared/api/commands\"";
        let Some(end) = src.find(marker) else {
            return String::new();
        };
        let start = src[..end].rfind("import").unwrap_or(0);
        src[start..end].to_string()
    }

    fn backlink_rows_call_only_the_cross_page_commands() {
        let row_ops = strip_comments(ROW_OPS);
        for name in CROSS_PAGE {
            assert!(
                calls(&row_ops, name),
                "BacklinksSection.row-ops.ts no longer calls {name}: the mobile \
                 backlink gestures must reach the source page through the \
                 cross-page command family",
            );
        }
        for (file, src) in [
            ("BacklinksSection.row-ops.ts", row_ops.as_str()),
            ("BacklinksSection.tsx", &strip_comments(SECTION)),
            ("BacklinkRowEditor.tsx", &strip_comments(ROW_EDITOR)),
        ] {
            let imports = commands_imports(src);
            for twin in OUTLINE_TWINS {
                assert!(
                    !calls(src, twin) && !imports.contains(twin),
                    "{file} reaches the outline command {twin}: it tolerates a stale \
                     block id (appends to the end of the page, prints a ULID, or \
                     reports success) where the cross-page family refuses with the \
                     catalog's sentence",
                );
            }
        }

        // A command defined but dropped from the handler list is a
        // button that does nothing.
        let lib = strip_comments(LIB);
        let handlers = &lib[lib
            .find("generate_handler!")
            .expect("lib.rs registers its commands through generate_handler!")..];
        for command in [
            "edit_backlink_block",
            "backlink_row_op",
            "create_backlink_sibling",
        ] {
            assert!(
                handlers.contains(&format!("{command},")),
                "lib.rs's generate_handler! no longer registers {command}",
            );
        }
    }
}
