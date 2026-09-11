//! Command-line surface: every clap declaration for the `outl` binary.
//!
//! Split out of `main.rs`, which now holds dispatch and nothing else.
//! The declarations and the dispatch grew together into one 1,085-line
//! file; keeping them apart is what stops a new subcommand from being a
//! merge conflict with an unrelated one.
//!
//! The user-facing reference is [`docs/cli.md`](../../../docs/cli.md) —
//! a flag added here without a row there is a flag nobody finds.

use crate::cmd;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "outl",
    about = "Local-first outliner with markdown as source of truth.",
    long_about = "Local-first outliner with markdown as source of truth.\n\
                  \n\
                  Running `outl` with no subcommand opens the TUI in the workspace at \
                  `--workspace` (default: current directory).",
    version
)]
pub(crate) struct Cli {
    /// Workspace path. Used by every subcommand that needs one;
    /// defaults to the current directory. Subcommand-level positional
    /// path, when provided, takes precedence.
    #[arg(short = 'w', long, global = true, value_name = "DIR")]
    pub(crate) workspace: Option<PathBuf>,

    /// TUI theme preset (default-dark, light, logseq-light, dracula,
    /// solarized-dark, nord, monokai, gruvbox). Overrides `[theme]
    /// preset` in workspace `config.toml` for this run.
    #[arg(long, global = true, value_name = "PRESET")]
    pub(crate) theme: Option<String>,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    /// Increase verbosity. Pass multiple times for more detail.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,
}

/// Subcommands for peer/device management.
#[derive(Debug, clap::Subcommand)]
pub(crate) enum PeerCommand {
    /// Pair with another device. Prints a ticket (QR + string); run on both devices.
    Pair {
        /// Accept a ticket from the other device instead of generating one.
        ///
        /// `-` reads the ticket from stdin, which is how a 400-to-750-character
        /// ticket gets into a container without going through argv:
        /// `pbpaste | docker compose run -i --rm outl peer pair --ticket -`.
        #[arg(long)]
        ticket: Option<String>,
        /// Human-readable name this device advertises to the other (shown in
        /// its `peer list`). Defaults to the machine hostname.
        #[arg(long)]
        name: Option<String>,
    },
    /// Render a pairing ticket you already have as a scannable QR code.
    ///
    /// `outl peer pair` prints one itself, and skips it when the terminal is
    /// too narrow for it to be readable. This is the way to get it back:
    /// widen a window, or pipe the ticket in from wherever you copied it.
    /// The phone app pairs by camera, so for a phone the QR is the only
    /// practical route.
    Qr {
        /// The ticket. Omit it, or pass `-`, to read stdin.
        ticket: Option<String>,
    },
    /// List all paired devices.
    List,
    /// Unpair a device by node-id prefix.
    Remove {
        /// Node-id prefix of the device to remove.
        id: String,
    },
    /// Show connection status of all paired devices.
    Status,
    /// Lock out every paired device by rotating this workspace's identity.
    ///
    /// For a lost or stolen device. `peer remove` only takes effect on the
    /// machine you run it on; this changes the workspace identity itself, so
    /// no device that is not re-paired can sync again — including one you no
    /// longer control.
    ///
    /// You will have to re-pair every device you still have.
    RevokeAll {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Open the TUI on the workspace (default: `--workspace` or current dir).
    Tui {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
    },
    /// Initialize a new workspace at the given path.
    Init {
        /// Workspace path. Created if it does not exist. Overrides `--workspace`.
        path: Option<PathBuf>,
        /// Op-log layout: `global` (single file per actor, legacy) or
        /// `per-page` (one file per (actor, page) — Phase B of RFC #137).
        /// New workspaces default to `global` for back-compat.
        #[arg(long, default_value = "global", value_parser = ["global", "per-page"])]
        scope: String,
        /// Create the layout and config but write no ops: no
        /// `templates/journal` page, no journal for today.
        ///
        /// For a workspace that exists only to become a replica of an
        /// existing graph — a self-hosted `outl serve` box — and will
        /// join it with `outl peer pair --ticket`. Pairing adopts the
        /// host's workspace id but keeps this device's ops, so a seeded
        /// replica pushes a second `templates/journal` page into the
        /// host's graph. Do NOT use it for a workspace you intend to
        /// write notes in directly; the journal template is missing.
        #[arg(long)]
        bare: bool,
    },
    /// Migrate a workspace's op log from `Global` (single file per
    /// actor) to `PerPage` (one file per actor + page). RFC #137
    /// Phase B. Reversible — the legacy file is preserved as
    /// `ops-<actor>.jsonl.v0.bak`.
    MigrateToPerPageOps {
        /// Workspace path. Overrides `--workspace`.
        path: Option<PathBuf>,
    },
    /// Drop provably-inert ops from the op log.
    ///
    /// The op log is append-only and grows forever: it replays on every
    /// boot and ships whole to every newly paired device. This removes
    /// one shape and only that shape — a `Move` that restates the
    /// placement its own `Create` made immediately before it, verified
    /// inert against a full replay. Every other op is copied through
    /// byte for byte.
    ///
    /// Reports and writes nothing unless `--apply` is passed. See
    /// `docs/rfcs/0256-op-log-compaction.md` for the predicate and what
    /// it deliberately refuses to touch.
    Compact {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Rewrite the op log.
        ///
        /// Refuses while any other outl process holds the workspace, and
        /// copies every file it rewrites to
        /// `.outl/compact-backup/<timestamp>/` before touching it.
        #[arg(long)]
        apply: bool,
        /// Also compact history newer than the 30-day settling horizon.
        ///
        /// The horizon is not about the ops — they are inert the moment
        /// they are written — it is about peers that have not delivered
        /// theirs yet. Drop it only on a workspace with no other device.
        #[arg(long = "no-horizon")]
        no_horizon: bool,
        /// Also rewrite the op logs of *other* devices.
        ///
        /// By default `--apply` rewrites only this device's
        /// `ops-<actor>.jsonl`. A peer's file is that device's own
        /// append-only log, and every file transport (iCloud, Syncthing,
        /// a shared filesystem) reconciles per path, last-write-wins — so
        /// a shortened copy can delete ops that device never shipped.
        /// Pass this only when no file transport carries this workspace;
        /// otherwise run `outl compact --apply` on each device.
        #[arg(long)]
        force: bool,
    },
    /// Run the background daemon: watch `.md` files and hold this device's
    /// P2P endpoint so paired peers sync continuously.
    ///
    /// Both halves are on by default. The watcher reconciles external `.md`
    /// edits into the op log; the sync half holds the iroh endpoint, deferring
    /// to a GUI or TUI that already has it and taking over when that exits.
    ///
    /// `--no-watch` is the cheaper mode to leave running: without the watcher
    /// there is nothing to write, so it takes no per-actor write lock and
    /// cannot push a GUI onto a fresh ephemeral actor (and a fresh
    /// `ops-<ulid>.jsonl`) on every launch. It still takes the P2P endpoint,
    /// though, and does not give it back, so a GUI opened after this daemon
    /// syncs through the shared ops/ dir until the daemon stops.
    Serve {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Reconcile every `.md` once and exit (no file watcher, no sync).
        #[arg(long, conflicts_with_all = ["no_watch", "no_sync"])]
        once: bool,
        /// Skip the file watcher; only hold the P2P endpoint.
        ///
        /// Fails when `[sync] transport` is "file": with no watcher and no
        /// endpoint to hold there is nothing left to do, and exiting 0 would
        /// have a process manager restart it into that config forever.
        #[arg(long)]
        no_watch: bool,
        /// Skip P2P sync; only run the file watcher.
        #[arg(long)]
        no_sync: bool,
    },
    /// Check workspace integrity.
    Doctor {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Emit the report as the JSON envelope instead of a human view.
        #[arg(long)]
        json: bool,
        /// Apply the safe, reversible fixes the report lists as
        /// repairable: re-project a stale `.md` from the op log,
        /// rebuild a missing sidecar, drop a corrupt snapshot.
        ///
        /// Never touches `ops/`, never deletes a `.md`, never moves a
        /// block to the trash. Every file it writes is copied to
        /// `.outl/repair-backup/<timestamp>/` first.
        #[arg(long)]
        repair: bool,
        /// Authorise a `--repair` whose measured volume is past the
        /// point it runs unattended.
        ///
        /// The report always states how many content lines the page
        /// re-projections would remove, and from how many pages, before
        /// anything is written. Past the ceiling those writes stand
        /// down; this is how you say "yes, I read that and I meant it".
        #[arg(long, requires = "repair")]
        force: bool,
    },
    /// Resolve orphan matches via the TUI.
    Reconcile {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Reconcile the pages whose `.md` holds content that exists in
        /// no op, ignoring the sidecar hash gate.
        ///
        /// A page can be hash-faithful (sidecar agrees with the bytes on
        /// disk) and still carry content the op log never saw, so the
        /// ordinary reconcile skips it as in-sync — see issue #210. This
        /// writes ops for that content, which is why it is opt-in.
        ///
        /// Run it only on a build whose parser preserves the content:
        /// reconciling with a parser that drops prose after a block
        /// property writes the truncated text into the log, making the
        /// loss permanent.
        #[arg(long = "ahead-of-log")]
        ahead_of_log: bool,
        /// Apply a deletion the orphan-volume guard refused.
        ///
        /// A reconcile that would trash more than 500 blocks of a page,
        /// or more than 75% of one, stops and writes nothing — a `.md`
        /// that arrived truncated (an undownloaded iCloud placeholder, a
        /// half-flushed write) is indistinguishable from a real bulk
        /// delete by shape, only by scale.
        ///
        /// This is the way to say the deletion was intended. Check what
        /// the `.md` actually holds before reaching for it: the guard
        /// fires on the case where the file is the thing that is wrong.
        #[arg(long = "allow-bulk-delete")]
        allow_bulk_delete: bool,
    },
    /// Recover block text that an `Op::Edit` truncated.
    ///
    /// The mirror of `reconcile --ahead-of-log`, reading the other
    /// source. That one reads the `.md` and can only recover content
    /// still on disk; this reads the **op log**, where a truncating
    /// edit's predecessor still carries the full text — the only route
    /// left for a page whose `.md` was already overwritten (issue #210).
    ///
    /// Read-only unless `--apply`. A restore is a new op; the op log is
    /// never rewritten.
    Recover {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Write the recovered text back as new `Op::Edit`s.
        ///
        /// Additive by construction: a block only qualifies when its
        /// current text is a prefix of the revision being restored, so
        /// nothing it shows today is dropped.
        #[arg(long)]
        apply: bool,
        /// Only report blocks that lost at least this many non-blank
        /// lines. Raise it when the listing is too long to read; a
        /// one-line loss is often ordinary editing.
        #[arg(long = "min-lines", default_value_t = cmd::recover::DEFAULT_MIN_LINES, value_parser = clap::value_parser!(u16).range(1..))]
        min_lines: u16,
    },
    /// Take, list, and restore local snapshots of the workspace.
    ///
    /// Uses the global `--workspace` for the target, like every other
    /// subcommand-carrying command (a positional path would be
    /// ambiguous against the subcommand name).
    Backup {
        #[command(subcommand)]
        sub: cmd::backup::BackupSubcommand,
    },
    /// Inspect or list theme presets.
    Theme {
        #[command(subcommand)]
        sub: Option<ThemeSubcommand>,
    },
    /// Import a graph from another outliner.
    Import {
        /// Source format: `roam` (JSON file), `logseq` (graph
        /// directory), `obsidian` (vault directory), or `auto`
        /// (detect from the source's shape).
        format: String,
        /// Path to the Logseq graph directory, the Roam backup file,
        /// or the Obsidian vault directory.
        src: PathBuf,
        /// Destination workspace. Created if it doesn't exist yet.
        dst: PathBuf,
        /// Parse and report only — write nothing to the destination.
        #[arg(long)]
        dry_run: bool,
        /// Print the import report as JSON.
        #[arg(long)]
        json: bool,
        /// Keep source create/edit timestamps as `created::` /
        /// `edited::` block properties.
        #[arg(long)]
        preserve_timestamps: bool,
        /// Don't pull referenced files into `assets/` — keep the
        /// original relative/remote links verbatim.
        #[arg(long)]
        no_assets: bool,
        /// Import even when the destination already holds content —
        /// overwrites those pages and discards anything written in outl,
        /// or received from a paired device, since the last import.
        #[arg(long)]
        force: bool,
    },
    /// Import a file (PDF, image, …) and link it into the workspace.
    Asset {
        #[command(subcommand)]
        sub: cmd::asset::AssetCommand,
    },
    /// Page-level operations.
    Page {
        #[command(subcommand)]
        sub: cmd::page::PageCommand,
    },
    /// Manage workspace plugins (list / install / run / enable / disable).
    Plugin {
        #[command(subcommand)]
        sub: cmd::plugin::PluginCommand,
    },
    /// Block-level operations.
    Block {
        #[command(subcommand)]
        sub: cmd::block::BlockCommand,
    },
    /// Daily journal operations.
    Daily {
        #[command(subcommand)]
        sub: cmd::daily::DailyCommand,
    },
    /// Full-text search.
    Search(cmd::search::SearchArgs),
    /// Structured query over pages.
    Query(cmd::query::QueryArgs),
    /// Backlinks and reference lookups.
    Backlinks {
        #[command(subcommand)]
        sub: cmd::backlinks::BacklinksCommand,
    },
    /// Apply a list of write ops sequentially in one workspace session.
    /// Reads `{"ops": [...]}` from stdin by default.
    Batch(cmd::batch::BatchArgs),
    /// Tag listing and lookups.
    Tag {
        #[command(subcommand)]
        sub: cmd::tag::TagCommand,
    },
    /// Template operations (list, apply, resolve callable).
    Template {
        #[command(subcommand)]
        sub: cmd::template::TemplateCommand,
    },
    /// Render a page in a target format (hugo / md / json).
    Export {
        #[command(subcommand)]
        sub: Option<cmd::export_v2::ExportCommand>,
        /// Legacy placeholder for `--to <fmt>` shape; only `hugo` was
        /// ever accepted. Kept so prior scripts don't break.
        #[arg(long)]
        to: Option<String>,
    },
    /// Workspace summary (path, actor, counts).
    Workspace {
        #[command(subcommand)]
        sub: WorkspaceSubcommand,
    },
    /// Run the MCP (Model Context Protocol) server over stdio. Wire
    /// this into `claude_desktop_config.json` to expose every CLI
    /// subcommand as an MCP tool.
    Mcp {
        #[command(subcommand)]
        sub: McpSubcommand,
    },
    /// Manage peer devices for P2P sync.
    Peer {
        #[command(subcommand)]
        cmd: PeerCommand,
    },
    /// Force a one-shot P2P sync pass against every paired device, then exit.
    ///
    /// For scripts that mutate via the CLI and must flush to peers before the
    /// process dies — a normal `outl page/block/...` command is too short-lived
    /// to bind an iroh endpoint, so it relies on whichever long-lived process
    /// on this device holds the endpoint (a GUI, a TUI, `outl mcp serve`, or
    /// `outl serve`) plus the catch-up re-sync instead. `outl sync` is the explicit flush; if one of
    /// those already holds the endpoint it says so and exits, since that
    /// process is already pushing these ops out.
    ///
    /// Exit codes: 0 a flush ran; 3 nothing was flushed (endpoint held
    /// elsewhere, P2P off, or no paired device) so the ops are still local
    /// until another process converges them; 1/2 the command failed.
    Sync,
}

#[derive(Subcommand, Debug)]
pub enum ThemeSubcommand {
    /// Print every available preset, one per line.
    List,
    /// Describe a specific preset (palette + style names).
    Show {
        /// Preset name (case- and separator-insensitive).
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceSubcommand {
    /// Workspace info — path, actor, counts.
    Info(cmd::workspace_info::WorkspaceInfoArgs),
}

#[derive(Subcommand, Debug)]
pub enum McpSubcommand {
    /// Start the MCP stdio server. Targets the workspace at the global
    /// `--workspace` (or current directory if unset).
    Serve {},
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The orphan-volume guard refuses a bulk delete, and that refusal is
    /// only defensible while the user has a way to say the deletion was
    /// meant. `OrphanGuard::Disabled` is that way, and this binary is the
    /// only thing that reaches it — so the flag existing is what keeps the
    /// guard from being a wall (root `CLAUDE.md` invariant 9).
    ///
    /// The escape hatch was unreachable from any user-facing surface for
    /// one commit, which is exactly the state this pins against.
    #[test]
    fn the_bulk_delete_escape_hatch_is_reachable_from_the_command_line() {
        let cli =
            Cli::try_parse_from(["outl", "reconcile", "--ahead-of-log", "--allow-bulk-delete"])
                .expect("`--allow-bulk-delete` must parse");
        assert!(matches!(
            cli.command,
            Some(Command::Reconcile {
                allow_bulk_delete: true,
                ahead_of_log: true,
                ..
            })
        ));
    }

    /// And it is off unless asked for.
    #[test]
    fn the_guard_is_enforced_by_default() {
        let cli = Cli::try_parse_from(["outl", "reconcile"]).expect("plain reconcile must parse");
        assert!(matches!(
            cli.command,
            Some(Command::Reconcile {
                allow_bulk_delete: false,
                ..
            })
        ));
    }
}
