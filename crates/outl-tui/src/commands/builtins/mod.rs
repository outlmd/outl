//! Built-in slash / palette commands.
//!
//! Each command is a small struct implementing the `SlashCommand`
//! trait from the parent `commands` module. They're grouped into
//! sibling modules by concern so this file stays a glance-able index:
//!
//! - `page` — block / page property mutations, including `pin`
//! - `workspace` — navigation, save, reload, help, quit, open
//! - `exec` — search overlay, code-block runner, theme swap
//! - `dates` — date / time / ISO / week-tag inserters
//!
//! Adding a new command:
//!
//! 1. Drop the `pub struct FooCommand;` + `impl SlashCommand for FooCommand` in the
//!    appropriate sibling (or create a new sibling if it's a fresh concern).
//! 2. Register it below in `register_all`.
//!
//! Convention:
//!
//! - `name()` is lowercase, single word.
//! - `description()` is one short sentence in present-tense English.
//! - Args-less commands (`needs_args = false`) usually toggle UI or
//!   navigate. They run immediately from `/` without a second prompt.
//! - Arg-taking commands explain expected format in `description`.
#![allow(missing_docs)]

mod asset;
mod dates;
mod exec;
mod page;
mod refer;
mod template;
mod workspace;

use super::CommandRegistry;

use asset::UploadCommand;
use dates::{
    DateCommand, DateLastWeekCommand, DateNextFridayCommand, DateNextMondayCommand,
    DateNextSaturdayCommand, DateNextSundayCommand, DateNextThursdayCommand,
    DateNextTuesdayCommand, DateNextWednesdayCommand, DateNextWeekCommand, DateTimeNowCommand,
    DateTodayCommand, DateTomorrowCommand, DateYesterdayCommand, IsoDateTodayCommand,
    IsoDateTomorrowCommand, IsoDateYesterdayCommand, TimeNowCommand, WeekNumCommand,
};
use exec::{RunCommand, SearchCommand, ThemeCommand};
use page::{PinCommand, PropBlockCommand, PropPageCommand};
use refer::{ReferCommand, ReferEmbedCommand};
use template::TemplateCommand;
use workspace::{
    HelpCommand, OpenCommand, PluginSettingsCommand, QuitCommand, RefreshCommand, SyncCommand,
    TodayCommand, TrashCommand, WriteCommand,
};

/// Hook for `super::CommandRegistry::with_builtins`.
pub(super) fn register_all(reg: &mut CommandRegistry) {
    // page — properties and the pinned toggle
    reg.register(PropBlockCommand);
    reg.register(PropPageCommand);
    reg.register(PinCommand);

    // workspace — chrome verbs
    reg.register(TodayCommand);
    reg.register(RefreshCommand);
    reg.register(WriteCommand);
    reg.register(HelpCommand);
    reg.register(QuitCommand);
    reg.register(OpenCommand);
    reg.register(PluginSettingsCommand);
    reg.register(SyncCommand);
    reg.register(TrashCommand);

    // exec — side-effecting actions
    reg.register(SearchCommand);
    reg.register(RunCommand);
    reg.register(ThemeCommand);

    // refer — capture block ref / embed handles
    reg.register(ReferCommand);
    reg.register(ReferEmbedCommand);

    // template — instantiate structural templates
    reg.register(TemplateCommand);

    // asset — upload a file and insert its link
    reg.register(UploadCommand);

    // dates — Insert-mode inserters. The slash dispatcher uses
    // `inserts_inline()` (set true in each command) to skip
    // `commit_insert()` and write straight into the live buffer.
    reg.register(DateTodayCommand);
    reg.register(DateTomorrowCommand);
    reg.register(DateYesterdayCommand);
    reg.register(DateNextWeekCommand);
    reg.register(DateLastWeekCommand);
    reg.register(DateNextMondayCommand);
    reg.register(DateNextTuesdayCommand);
    reg.register(DateNextWednesdayCommand);
    reg.register(DateNextThursdayCommand);
    reg.register(DateNextFridayCommand);
    reg.register(DateNextSaturdayCommand);
    reg.register(DateNextSundayCommand);
    reg.register(DateCommand);
    reg.register(IsoDateTodayCommand);
    reg.register(IsoDateTomorrowCommand);
    reg.register(IsoDateYesterdayCommand);
    reg.register(TimeNowCommand);
    reg.register(DateTimeNowCommand);
    reg.register(WeekNumCommand);
}
