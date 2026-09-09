//! One entry per shared command, grouped by module.
//!
//! A client invokes a whole `*_commands!` or none of it. There is no
//! "take three of the five" on purpose: that is exactly how `history`
//! ended up desktop-only and `run_auto_run_blocks` / `resolve_embeds`
//! ended up unreachable on the phone — nobody decided, the wrappers were
//! simply never typed. See [`super`] for the full reasoning.
//!
//! A command needing more from Tauri than `State<'_, AppState>` — an
//! `AppHandle`, a second `State` — is not boilerplate and is not here.

/// Asset commands: open an uploaded file, read one as a data URL,
/// import one, attach one to a page.
#[macro_export]
macro_rules! asset_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Open an asset in the OS default application.
            fn open_asset(url: String) -> ::std::result::Result<(), String>
                => $crate::commands::asset::open_asset;

            /// Read an asset's bytes back as a `data:` URL the webview
            /// renders inline (image, PDF).
            fn read_asset_data_url(url: String) -> ::std::result::Result<String, String>
                => $crate::commands::asset::read_asset_data_url;

            /// Copy a file into the workspace's asset store.
            fn import_asset_file(source_path: String)
                -> ::std::result::Result<::outl_actions::ImportedAsset, String>
                => $crate::commands::asset::import_asset_file;

            /// Import a file and attach it to `page_id` as a new block.
            fn attach_asset(
                source_path: String,
                page_id: String,
                after_block_id: Option<String>,
            ) -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::asset::attach_asset;
        }
    };
}

/// Block mutations. Every structural and textual edit a GUI client can
/// make to a page.
#[macro_export]
macro_rules! block_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Create a block — see the shared body for the anchor
            /// precedence and its stale-anchor fallback.
            fn create_block(
                page_id: String,
                after_id: Option<String>,
                before_id: Option<String>,
                parent_id: Option<String>,
                text: Option<String>,
            ) -> ::std::result::Result<$crate::state::CreateBlockReply, String>
                => $crate::commands::block::create_block;

            /// Replace a block's text.
            fn edit_block(page_id: String, id: String, text: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::edit_block;

            /// Split a block at `char_offset`, returning the new block's id.
            fn split_block(page_id: String, id: String, char_offset: u32)
                -> ::std::result::Result<$crate::state::CreateBlockReply, String>
                => $crate::commands::block::split_block;

            /// Cycle a block's TODO / DOING / DONE marker.
            fn toggle_todo(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::toggle_todo;

            /// Toggle a block's blockquote prefix.
            fn toggle_quote(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::toggle_quote;

            /// Move a block to the trash root (invariant 6 — never a
            /// physical removal).
            fn delete_block(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::delete_block;

            /// Make a block the last child of its previous sibling.
            fn indent_block(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::indent_block;

            /// Move a block up one level, after its former parent.
            fn outdent_block(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::outdent_block;

            /// Swap a block with its previous sibling.
            fn move_block_up(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::move_block_up;

            /// Swap a block with its next sibling.
            fn move_block_down(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::move_block_down;

            /// Re-parent a block to sit directly after `after_id`.
            fn move_block_after(page_id: String, id: String, after_id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::move_block_after;

            /// Render a block's subtree as markdown, for the clipboard.
            fn copy_block_markdown(id: String)
                -> ::std::result::Result<String, String>
                => $crate::commands::block::copy_block_markdown;

            /// Paste a markdown subtree after `after_id`.
            fn paste_block_after(page_id: String, after_id: String, text: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::paste_block_after;

            /// Delete a block and return its markdown, so the caller can
            /// hold it in a block clipboard.
            fn cut_block(page_id: String, id: String)
                -> ::std::result::Result<$crate::state::CutBlockReply, String>
                => $crate::commands::block::cut_block;

            /// Fold / unfold a block. The only mutation that deliberately
            /// skips reprojection — see the shared body.
            fn set_block_collapsed(page_id: String, id: String, collapsed: bool)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::set_block_collapsed;

            /// Paste markdown into a block at `caret`, splitting as needed.
            fn paste_markdown_at(
                page_id: String,
                block_id: String,
                caret: u32,
                text: String,
            ) -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::paste_markdown_at;

            /// Paste plain text into a block at `caret`, no outline parsing.
            fn paste_plain_at(
                page_id: String,
                block_id: String,
                caret: u32,
                text: String,
            ) -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::block::paste_plain_at;

            /// Render several blocks' subtrees as one markdown string.
            fn copy_markdown(block_ids: Vec<String>)
                -> ::std::result::Result<String, String>
                => $crate::commands::block::copy_markdown;

            /// The `((block-ref))` handle for a block.
            fn copy_block_ref(id: String)
                -> ::std::result::Result<String, String>
                => $crate::commands::block::copy_block_ref;
        }
    };
}

/// Code-block execution (`run`, auto-run on open, embed resolution).
#[macro_export]
macro_rules! exec_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Execute one fenced code block and record its output.
            fn run_code_block(page_id: String, block_id: String)
                -> ::std::result::Result<$crate::commands::exec::RunCodeBlockReply, String>
                => $crate::commands::exec::run_code_block;

            /// Execute every `auto-run` block on a page.
            fn run_auto_run_blocks(page_id: String)
                -> ::std::result::Result<$crate::commands::exec::AutoRunReply, String>
                => $crate::commands::exec::run_auto_run_blocks;

            /// Resolve `{{embed}}` handles to their current content.
            fn resolve_embeds(handles: Vec<String>)
                -> ::std::result::Result<
                    ::std::collections::HashMap<String, $crate::commands::exec::EmbedContent>,
                    String,
                >
                => $crate::commands::exec::resolve_embeds;
        }
    };
}

/// Undo / redo of committed block mutations.
#[macro_export]
macro_rules! history_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Revert the last committed mutation on a page. Errors with
            /// `"nothing to undo"` on an empty stack.
            fn undo_page(page_id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::history::undo_page;

            /// Re-apply the mutation the last `undo_page` reverted.
            fn redo_page(page_id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::history::redo_page;
        }
    };
}

/// Page and journal navigation, search, and page-level structure.
///
/// `open_ref` is **not** here: it needs an `AppHandle` to emit its
/// projection-failure event, so each client wires it by hand.
#[macro_export]
macro_rules! page_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every page in the workspace.
            fn list_all_pages()
                -> ::std::result::Result<::std::vec::Vec<::outl_actions::PageMeta>, String>
                => $crate::commands::page::list_all_pages;

            /// Fuzzy page search for the picker.
            fn search_pages(query: String)
                -> ::std::result::Result<::std::vec::Vec<::outl_actions::PageMeta>, String>
                => $crate::commands::page::search_pages;

            /// Pages carrying `type:: person`, for `@` mentions.
            fn search_persons(query: String)
                -> ::std::result::Result<::std::vec::Vec<::outl_actions::PageMeta>, String>
                => $crate::commands::page::search_persons;

            /// Full-text block search.
            fn search_blocks(query: String)
                -> ::std::result::Result<::std::vec::Vec<$crate::state::BlockHit>, String>
                => $crate::commands::page::search_blocks;

            /// Emoji autocomplete. Pure — no workspace needed.
            fn outl_emoji_search(query: String, limit: usize)
                -> ::std::result::Result<
                    ::std::vec::Vec<::outl_md::emoji::EmojiHit>,
                    String,
                >
                => plain $crate::commands::page::emoji_search;

            /// Open (creating if absent) today's journal.
            fn open_today_journal()
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::page::open_today_journal;

            /// Open the journal for an ISO date slug.
            fn open_journal_for(slug: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::page::open_journal_for;

            /// Open a page by slug.
            fn open_page_by_slug(slug: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::page::open_page_by_slug;

            /// The day before an ISO date slug.
            fn previous_day(slug: String) -> ::std::result::Result<String, String>
                => plain $crate::commands::page::previous_day;

            /// The day after an ISO date slug.
            fn next_day(slug: String) -> ::std::result::Result<String, String>
                => plain $crate::commands::page::next_day;

            /// Today's ISO date slug on this device.
            fn today_slug_cmd() -> String
                => plain $crate::commands::page::today_slug;

            /// Human-readable title for an ISO date slug.
            fn date_title(slug: String) -> ::std::result::Result<String, String>
                => plain $crate::commands::page::date_title;

            /// Resolve a `[[ref]]` target to its page, without opening it.
            fn resolve_ref(target: String)
                -> ::std::result::Result<Option<::outl_actions::PageMeta>, String>
                => $crate::commands::page::resolve_ref;

            /// Toggle a page's `pinned::` property.
            fn toggle_pin(page_id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::page::toggle_pin;

            /// Move a page to the trash root and drop its projection.
            fn delete_page(slug: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::page::delete_page;

            /// Backlinks for a page, fetched lazily off the open path.
            async fn page_backlinks(slug: String)
                -> ::std::result::Result<$crate::state::BacklinksReply, String>
                => $crate::commands::page::page_backlinks;

            /// Re-sort backlinks and persist the direction.
            async fn set_backlinks_order(order: String, slug: String)
                -> ::std::result::Result<$crate::state::BacklinksReply, String>
                => $crate::commands::page::set_backlinks_order;

            /// Human labels for a list of node ids (breadcrumbs).
            fn resolve_page_labels(node_ids: Vec<String>)
                -> ::std::result::Result<::std::vec::Vec<String>, String>
                => $crate::commands::page::resolve_page_labels;
        }
    };
}

/// Peer listing and status.
///
/// Pairing (`pair_host` / `pair_join`) and `sync_now` are **not** here:
/// they need an `AppHandle` or the concrete transport, so each client
/// wires them by hand.
#[macro_export]
macro_rules! peer_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Known peers from `peers.json`.
            fn outl_peer_list()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::commands::peers::PeerDto>,
                    String,
                >
                => $crate::commands::peers::peer_list;

            /// Forget a peer.
            fn outl_peer_remove(id: String) -> ::std::result::Result<bool, String>
                => $crate::commands::peers::peer_remove;

            /// Live reachability probe for every known peer.
            fn outl_peer_status()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::commands::peers::PeerStatusDto>,
                    String,
                >
                => $crate::commands::peers::peer_status;
        }
    };
}

/// The property catalogue that feeds key autocomplete.
#[macro_export]
macro_rules! property_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Property keys used anywhere in the workspace, most-used
            /// first.
            fn known_property_keys()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::commands::property::PropertyKey>,
                    String,
                >
                => $crate::commands::property::known_property_keys;
        }
    };
}

/// `remind::` rules, snoozing, settings, and the property writers that
/// grew out of them.
///
/// `deliver_due_reminders` is **not** here: it emits an OS notification
/// through the client's `AppHandle`.
#[macro_export]
macro_rules! reminder_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every block carrying a `remind::` rule, with its next fire.
            fn list_reminders()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::commands::reminders::ReminderDto>,
                    String,
                >
                => $crate::commands::reminders::list_reminders;

            /// Current reminder settings from `config.toml`.
            fn reminder_settings() -> $crate::commands::reminders::ReminderSettingsDto
                => plain $crate::commands::reminders::reminder_settings;

            /// Write the two user-facing reminder settings back.
            fn set_reminder_settings(enabled: bool, quiet_hours: String)
                -> ::std::result::Result<
                    $crate::commands::reminders::ReminderSettingsDto,
                    String,
                >
                => plain $crate::commands::reminders::set_reminder_settings;

            /// Snooze options in render order.
            fn snooze_presets()
                -> ::std::vec::Vec<$crate::commands::reminders::SnoozePresetDto>
                => plain $crate::commands::reminders::snooze_presets;

            /// Snooze a block's reminder by a preset id.
            fn snooze_reminder(block_id: String, preset: String)
                -> ::std::result::Result<(), String>
                => $crate::commands::reminders::snooze_reminder;

            /// Clear a snooze so the rule resumes its normal schedule.
            fn clear_reminder_snooze(block_id: String)
                -> ::std::result::Result<(), String>
                => $crate::commands::reminders::clear_reminder_snooze;

            /// Set (or clear, with an empty value) any property on a block.
            fn set_block_property(
                page_id: String,
                block_id: String,
                key: String,
                value: String,
            ) -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::reminders::set_block_property;

            /// Set (or clear) a property on the page itself.
            fn set_page_property(page_id: String, key: String, value: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::reminders::set_page_property;

            /// Mark a block DONE, cancelling every pending fire.
            fn mark_block_done(page_id: String, block_id: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::reminders::mark_block_done;

            /// Set (or clear) a block's `remind::` rule.
            fn set_block_remind(page_id: String, block_id: String, rule: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::reminders::set_block_remind;
        }
    };
}

/// The `(chord, action)` catalog and the per-client support matrix
/// (root `CLAUDE.md` invariant 12).
///
/// Every entry is `plain` (the catalog is static), so `$state` is
/// never expanded — it is kept so a client invokes all twelve modules
/// the same way, and so adding a stateful command here needs no call-site
/// change.
#[macro_export]
macro_rules! shortcut_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every default binding shipped by `outl-shortcuts`.
            fn list_shortcut_bindings() -> ::std::vec::Vec<::outl_shortcuts::Binding>
                => plain $crate::commands::shortcuts::list_shortcut_bindings;

            /// What every client does with every action in the catalog.
            fn list_action_support()
                -> ::std::vec::Vec<$crate::commands::shortcuts::ActionSupportDto>
                => plain $crate::commands::shortcuts::list_action_support;
        }
    };
}

/// Structural templates.
#[macro_export]
macro_rules! template_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every structural template in the workspace, for the
            /// `/template` picker.
            ///
            /// The `_cmd` suffix avoids a glob-import collision with the
            /// `outl_actions::list_templates` action.
            fn list_templates_cmd()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::state::TemplateDto>,
                    String,
                >
                => $crate::commands::template::list_templates;

            /// Deep-copy a template under `target_block`.
            fn instantiate_template_at(name: String, target_block: String)
                -> ::std::result::Result<$crate::state::PageView, String>
                => $crate::commands::template::instantiate_template_at;
        }
    };
}

/// Theme lookup. Pure — no workspace, no host.
///
/// Every entry is `plain` (config + presets, no workspace), so `$state`
/// is never expanded — kept for call-site uniformity across the twelve
/// modules.
#[macro_export]
macro_rules! theme_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Names of every built-in preset.
            fn list_themes() -> ::std::vec::Vec<String>
                => plain $crate::commands::theme::list_themes;

            /// A preset's palette, or the configured one when `name` is
            /// `None`.
            fn get_theme(name: Option<String>) -> ::outl_theme::Palette
                => plain $crate::commands::theme::get_theme;

            /// The `[theme]` config block (preset + light/dark pair).
            fn get_theme_config() -> $crate::commands::theme::ThemeConfigDto
                => plain $crate::commands::theme::get_theme_config;
        }
    };
}

/// A page's history out of the op log. Read-only, and distinct from
/// [`history_commands`], which is this session's undo stack.
#[macro_export]
macro_rules! timeline_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every change to a page, newest first, read from the op log.
            fn page_timeline(page_id: String, limit: Option<usize>)
                -> ::std::result::Result<
                    $crate::commands::timeline::PageTimelineDto,
                    String,
                >
                => $crate::commands::timeline::page_timeline;
        }
    };
}
