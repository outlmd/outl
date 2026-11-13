//! Reasons repeated across rows, named once so a re-wording lands on
//! every row at once instead of some.
//!
//! Its own file because the parent sits against the 600-line guard
//! and these are the half of it that grows with every capability: a
//! verdict is three words, its reason is the sentence a user reads.
//! Keeping them here leaves the parent as the `match` — the part a
//! reader comes for.
//!
//! **The wording lives here, never in a client.** A client that
//! writes its own sentence is a second copy of the fact (root
//! `CLAUDE.md` invariant 12).

/// Mobile is the one client left without a trash surface: the
/// `list_trash` / `restore_trash_entry` commands are registered there
/// (`trash_commands!`), and no sheet calls them yet.
///
/// It does **not** name `outl trash list`, the way this string did
/// while all three clients lacked the surface. There is no `outl`
/// binary on iOS, so telling a phone user to run a command is telling
/// them nothing — the honest route is the computer that shares the
/// workspace. Same reason `aheadOfLogNotice` splits its last sentence
/// by client.
pub const NO_MOBILE_TRASH_SHEET: &str =
    "Restoring a deleted block isn't on the phone yet — open this workspace on your computer \
     and use the trash there. Nothing is lost in the meantime: a delete here is never \
     permanent.";

/// Snapshots fork the `git` binary (`outl_actions::backup`), and a
/// phone has no `git` on `PATH` — an App Store app cannot exec one
/// either. So mobile spawns the same pass every client does and that
/// pass answers `BackupError::GitMissing` on every tick, which is a
/// `warn!` the user never reads.
///
/// `Missing`, not `NotApplicable`: `[backup] enabled` defaults to on,
/// so this user genuinely wants their notes snapshotted rather than
/// being someone the input device makes the question meaningless for
/// (the distinction `NO_TELEMETRY_TOGGLE_IN_THE_TUI` draws). The reach
/// is a computer on the same workspace, and the nudge names it.
pub const NO_GIT_ON_THE_PHONE: &str =
    "Automatic backups need `git`, which phones don't have — a computer syncing this workspace \
     snapshots it for you. Your notes are still safe here: deleting is never permanent, and \
     every edit syncs to your other devices.";

/// Every client renders a table (`outl_md::table` is shared), and
/// only the terminal has nowhere to put one that does not fit: the
/// outline wraps a long line rather than scrolling it sideways, so
/// the rows are all still there and the columns stop lining up.
/// The GUI clients scroll the grid inside the block.
pub const TUI_TABLE_NO_SCROLL: &str =
    "A table wider than the pane wraps instead of scrolling sideways — every row is still \
     there, the columns just stop lining up. Widen the terminal, or open the page in the \
     desktop app.";

/// The TUI has no marketplace UI at all — installing an
/// unlisted or registry plugin from inside the terminal client
/// goes through the CLI, not a browse-and-tap surface.
pub const NO_TUI_MARKETPLACE: &str =
    "Browsing and installing plugins from a marketplace isn't in the TUI — install by id \
     from a terminal with `outl plugin install <id>`, or use the desktop or mobile app.";

/// The TUI *does* draw a month grid (`view/sidebar.rs`
/// `render_calendar`, `Ctrl+E` to open the sidebar) with a dot on
/// every day that has a journal and a bullseye on today — but the
/// grid is a read-out, not a control: `SidebarSection::Calendar`
/// reports `0` items, so the cursor never enters it, and
/// `sidebar_activate` returns without doing anything on that
/// section. The month shown follows wherever the user already is.
///
/// So the *grid* half of this capability is present and the
/// *random-access jump* half is not, which is what `Partial`
/// means. The previous verdict here was `Missing` with the
/// wording "There's no calendar grid here", on both the TUI and
/// the desktop — neither was true.
pub const TUI_CALENDAR_IS_READ_ONLY: &str =
    "The TUI's month grid shows which days have journals but can't open one — press \
     `[` / `]` to step a day, or open the quick switcher (Ctrl+P) and type the date \
     (YYYY-MM-DD) to jump straight there.";

/// The TUI paints the nested-pages list below the backlinks
/// (`view/namespace.rs`) but `j` / `k` stop at the backlinks
/// section — `Focus` has exactly two variants (`Outline`,
/// `Backlink`) and adding a third touches 36 call sites across
/// 12 files, each one a decision about what `d`, `x` and `i`
/// mean over a page row. So the *list* half of this capability
/// is present and the *open it from the list* half is not,
/// which is what `Partial` means (same shape as
/// `TUI_CALENDAR_IS_READ_ONLY`).
pub const TUI_NESTED_PAGES_ARE_READ_ONLY: &str =
    "The TUI lists nested pages but can't open one from that list — press Ctrl+P and type \
     the page name (`os/linux`) to jump there.";

/// The TUI has no pairing flow (no ticket display, no scanner) —
/// pairing a device against a TUI-run workspace goes through the
/// CLI's `outl peer pair` / `outl peer qr`, run from a separate
/// terminal.
pub const NO_TUI_PAIRING: &str = "Pairing a new device isn't in the TUI — run `outl peer \
     pair` or `outl peer qr` from a terminal, or pair from the desktop or mobile app.";

/// `SyncPanel.tsx` only ever calls `peerPairHost()` — its own doc
/// comment says "There is no camera path here." A desktop user
/// can invite a device in, but has no way to *join* an existing
/// workspace from the desktop UI itself.
/// Neither the TUI nor the desktop can put buttons on the banner
/// it already shows, and the reason is the same in both: the
/// delivery channel carries no callback. The TUI writes an OSC 9
/// escape, which is one string to the terminal. The desktop goes
/// through `tauri-plugin-notification`, whose `ActionType` /
/// `register_action_types` / `onAction` surface is `#[cfg(mobile)]`
/// — its desktop `show()` spawns `notify-rust` and drops the
/// handle, so there is nothing to attach an action to and nothing
/// to hear back from.
///
/// Both clients already have a surface that does the same job in
/// one more keystroke, so the nudge names it rather than
/// apologising.
pub const NO_BANNER_ACTIONS: &str =
    "Reminder banners can't carry buttons here — open the reminders list (Ctrl+R in the \
     TUI, Cmd/Ctrl+Shift+R on desktop) to snooze or tick off what came due.";

/// `outl capture` is a terminal command, so the TUI user already
/// has every route a desktop has — and the TUI drains the inbox
/// once, at boot, because nothing in it watches the folder. So a
/// capture made while the TUI is up waits for the next launch,
/// where a GUI client lands it within a second of coming back.
pub const TUI_CAPTURE_LANDS_AT_LAUNCH: &str =
    "A line captured with `outl capture` lands the next time you start outl here — the \
     desktop and phone apps pick one up while they're running.";

/// The desktop drains the inbox, so every capture lands; what it
/// does not have is a gesture of its own. There is no outl menu bar
/// item, Service or global hotkey that writes a capture, so the
/// entry point is `outl capture` and whatever the user wires it
/// into. Named as the gap it is rather than claimed as parity with
/// a widget and a share sheet.
pub const DESKTOP_CAPTURE_HAS_NO_SYSTEM_GESTURE: &str =
    "Capturing from outside the app runs `outl capture \"your line\"` — wire it to a \
     Shortcut, a macOS Service or a hotkey and it lands here on the next window focus.";

/// Neither the TUI nor the desktop renders a keyboard accessory
/// bar: both are driven by a physical keyboard, where every
/// toolbar action is a fixed chord from `defaults.rs`. A chord
/// does not move, so there is no order to pin and nothing a
/// lock would buy — this is a property of the input device, not
/// work left undone.
/// Capture on the keyboard clients is two steps: jump to today,
/// then add the block. Every piece exists; the one-gesture shape
/// from mobile's bottom bar does not.
pub const TUI_CAPTURE_IS_TWO_STEPS: &str =
    "There's no one-key capture here. Press `t` to jump to today's journal, then add the \
     block there. Press `[` to go back to the day you were on.";
pub const DESKTOP_CAPTURE_IS_TWO_STEPS: &str =
    "There's no one-click capture here. Press Cmd+J to open today's journal, then add the \
     block at the end. The quick switcher (Cmd+P) takes you back.";

/// Only the TUI lacks a task list now. The query block answers the
/// same question inside any page, so the nudge points there.
pub const NO_TASK_LIST: &str =
    "There's no task list here yet. Add a `query` code block with `status: open` to any page \
     and it lists every open task in the workspace.";

/// The TUI paints the fence bytes. Diagrams are a GUI surface.
pub const TUI_KEEPS_MERMAID_SOURCE: &str =
    "The terminal keeps a mermaid fence as source. Open the page in the desktop or mobile \
     app to see the diagram.";

pub const NO_SOFT_KEYBOARD_BAR: &str =
    "There's no on-screen toolbar to pin here — these actions have fixed keyboard \
     shortcuts, so they never move.";

/// The TUI is a process a user starts from a shell; no desktop
/// environment hands it a double-clicked file, and there is no
/// bundle for a file manager to offer. The equivalent is the
/// shell, which the user is already in.
pub const NO_TUI_FILE_HANDOFF: &str =
    "A file manager has no terminal client to hand a file to — open the file in the \
     desktop app, or paste its text into a block.";

/// The command is registered on mobile and nothing delivers a
/// path to it: neither `tauri.conf.json` declares a share-sheet
/// / document-type association, so iOS and Android never offer
/// outl in "Open in". The import itself would work the moment
/// one does.
pub const NO_MOBILE_SHARE_TARGET: &str =
    "Opening a file into outl from another app isn't wired up on mobile yet — send it to a \
     computer and open it there, or paste the text into a block.";

/// The TUI has no settings screen of any kind, so there is no
/// surface for a toggle to live on — and the command that does the
/// same job runs in the terminal the TUI was started from, which
/// the user already has open.
///
/// `Missing`, not `NotApplicable`: reporting is on by default, so
/// the opt-out is something this user genuinely needs rather than
/// something the input device makes meaningless. The reach is one
/// command away, and the nudge names it.
pub const NO_TELEMETRY_TOGGLE_IN_THE_TUI: &str =
    "Crash reporting is switched from the terminal here — run `outl telemetry off` to stop \
     sending reports, or `outl telemetry status` to see where it stands.";

/// The TUI and the CLI both report and neither says so unprompted.
///
/// Not a gap in the *information*: `docs/privacy.md` carries the
/// full field list and `outl telemetry status` answers honestly.
/// It is a gap in **when** the user meets it, which is the half
/// RFC 0377 section 6 says the legitimate-interest footing needs.
///
/// Left open rather than closed quietly, because the obvious fix
/// is not obviously right: a terminal's first-run output is a
/// contract with whatever is reading the pipe, and printing into
/// somebody's script is its own defect. That is a product call,
/// recorded in RFC 0377's declared gaps.
pub const NO_PROACTIVE_NOTICE_IN_THE_TERMINAL: &str =
    "Crash reporting is on, and the terminal does not announce it the way the apps do — run \
     `outl telemetry status` to see where it stands, or read the full list of what a report \
     carries at outl.app/privacy.";

/// Mobile types `/` like any other character: nothing in
/// `BlockTextarea.tsx` watches for it, and `@outl/shared/autocomplete`
/// exposes `detectSlashContext` that this client simply does not call.
///
/// The nudge names the two surfaces rather than apologising, because
/// the commands themselves are reachable here — and it names them
/// where they actually are, which is **not** one menu: the plugins
/// button sits in the journal's top bar (`JournalChrome`, aria-label
/// "Plugin commands"), while "Insert template" is an entry in a
/// block's long-press menu (`Journal.context-actions.ts`).
///
/// Worth the extra clause. A nudge that sends a user to a menu that
/// does not hold the thing is the failure the `PageHistory` row's
/// comment records: an exhaustive `match` forces a verdict and nothing
/// forces the verdict to be true.
pub const NO_SLASH_MENU_ON_THE_PHONE: &str =
    "Typing `/` doesn't open a command list on the phone yet — plugin commands are behind the \
     plugins button in the top bar, and templates are under \"Insert template\" when you \
     long-press a block.";

pub const DESKTOP_HOSTS_ONLY: &str =
    "The desktop can host a pairing (show the QR / ticket) but has no camera to scan one — \
     to join an existing workspace from a desktop, run `outl peer pair` in a terminal.";
