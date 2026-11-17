//! Reminder commands. Its own macro because `catalog.rs` reached its
//! size ceiling — the same reason `wrappers/trash.rs` and
//! `wrappers/page_merge.rs` exist, and `tests/command_parity.rs` walks
//! every `src/wrappers/*.rs` rather than `catalog.rs` alone, so a
//! client that drops this module fails the build.
//!
//! Both GUI clients register all of it. The authoring pair
//! (`parse_remind_rule` / `format_remind_rule`) is what the mobile
//! "Remind me…" sheet binds its controls to; the desktop has no picker
//! yet and registers them anyway, because a client takes a whole
//! module or records the gap (see [`super`]).

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

            /// Take a `remind::` value apart into the fields a picker
            /// binds to. `null` for a value that does not parse.
            fn parse_remind_rule(value: String)
                -> ::std::option::Option<$crate::commands::reminders::RemindFieldsDto>
                => plain $crate::commands::reminders::parse_remind_rule;

            /// Put a picker's fields back together into a rule.
            fn format_remind_rule(
                fields: $crate::commands::reminders::RemindFieldsDto,
            ) -> ::std::result::Result<String, String>
                => plain $crate::commands::reminders::format_remind_rule;
        }
    };
}
