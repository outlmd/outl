//! `outl` — the CLI binary.
//!
//! Thin shell over `outl-core`, `outl-md`, `outl-actions`, and
//! `outl-tui`. See `crates/outl-cli/CLAUDE.md` and `docs/cli.md`.
//!
//! UX:
//!
//! - `outl` with no subcommand opens the TUI in the current directory.
//! - `outl --workspace <dir>` opens the TUI in `<dir>` (global flag, works
//!   with any subcommand that needs a workspace path).
//! - Subcommands cover workspace lifecycle, machine-shaped operations
//!   (page/block/daily/search/query/export), and the `mcp serve` shim
//!   that lets Claude Desktop reach the same handlers over stdio.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

mod cmd;
mod human;
mod mcp;
mod output;
mod sync_engine;
mod workspace_layout;
mod ws;

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
struct Cli {
    /// Workspace path. Used by every subcommand that needs one;
    /// defaults to the current directory. Subcommand-level positional
    /// path, when provided, takes precedence.
    #[arg(short = 'w', long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,

    /// TUI theme preset (default-dark, light, logseq-light, dracula,
    /// solarized-dark, nord, monokai). Overrides `[theme] preset` in
    /// workspace `config.toml` for this run.
    #[arg(long, global = true, value_name = "PRESET")]
    theme: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Increase verbosity. Pass multiple times for more detail.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

/// Subcommands for peer/device management.
#[derive(Debug, clap::Subcommand)]
enum PeerCommand {
    /// Pair with another device. Prints a ticket (QR + string); run on both devices.
    Pair {
        /// Accept a ticket from the other device instead of generating one.
        #[arg(long)]
        ticket: Option<String>,
        /// Human-readable name this device advertises to the other (shown in
        /// its `peer list`). Defaults to the machine hostname.
        #[arg(long)]
        name: Option<String>,
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
}

#[derive(Subcommand, Debug)]
enum Command {
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
    },
    /// Migrate a workspace's op log from `Global` (single file per
    /// actor) to `PerPage` (one file per actor + page). RFC #137
    /// Phase B. Reversible — the legacy file is preserved as
    /// `ops-<actor>.jsonl.v0.bak`.
    MigrateToPerPageOps {
        /// Workspace path. Overrides `--workspace`.
        path: Option<PathBuf>,
    },
    /// Run the file watcher; keep the workspace in sync.
    Serve {
        /// Workspace path. Overrides the global `--workspace`.
        path: Option<PathBuf>,
        /// Reconcile every `.md` once and exit (no file watcher).
        #[arg(long)]
        once: bool,
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
    /// on this device holds the endpoint (a GUI, or `outl mcp serve`) plus the
    /// catch-up re-sync instead. `outl sync` is the explicit flush; if one of
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // The TUI installs its own silent subscriber that captures
    // dependency logs (Steel, wasmtime, ...) into
    // `<workspace>/.outl/tui.log`. If we install a stderr subscriber
    // here first, the TUI's `try_init` is a no-op and every dep log
    // ends up *on top of* the rendered UI. So defer: TUI runs install
    // their own; everything else (serve / doctor / reconcile / ...)
    // keeps the stderr subscriber the user expects on a CLI command.
    let is_tui = matches!(cli.command, None | Some(Command::Tui { .. }));
    if !is_tui {
        init_tracing(cli.verbose);
    }

    // Resolve the journal/clock timezone once, before any subcommand
    // computes "today" (#107). Idempotent with the TUI's own init on the
    // no-subcommand path. No `[calendar] timezone` → OS local, as before.
    outl_actions::clock::init(outl_config::load().calendar.timezone.as_deref());

    match cli.command {
        None => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Tui { path }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            ensure_workspace_or_prompt(&p)?;
            outl_tui::run_with_theme_override(&p, cli.theme.as_deref())
        }
        Some(Command::Init { path, scope }) => {
            let p = resolve_init_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::init::run(&p, &scope)
        }
        Some(Command::MigrateToPerPageOps { path }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::migrate_to_per_page_ops::run(&p)
        }
        Some(Command::Serve { path, once }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::serve::run(&p, once)
        }
        Some(Command::Doctor {
            path,
            json,
            repair,
            force,
        }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            let scope = if force {
                cmd::doctor::RepairScope::Forced
            } else {
                cmd::doctor::RepairScope::Guarded
            };
            if json {
                std::process::exit(cmd::doctor::run_json(&p, repair, scope));
            }
            cmd::doctor::run(&p, repair, scope)
        }
        Some(Command::Reconcile {
            path,
            ahead_of_log,
            allow_bulk_delete,
        }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::reconcile::run(&p, ahead_of_log, allow_bulk_delete)
        }
        Some(Command::Recover {
            path,
            apply,
            min_lines,
        }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::recover::run(&p, apply, min_lines)
        }
        Some(Command::Backup { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            cmd::backup::run(&p, &sub)
        }
        Some(Command::Theme { sub }) => cmd::theme::run(sub.as_ref()),
        Some(Command::Import {
            format,
            src,
            dst,
            dry_run,
            json,
            preserve_timestamps,
            no_assets,
            force,
        }) => cmd::import::run(
            &format,
            &src,
            &dst,
            cmd::import::ImportFlags {
                dry_run,
                json,
                preserve_timestamps,
                no_assets,
                force,
            },
        ),
        Some(Command::Asset { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::asset::run(&sub, &p));
        }
        Some(Command::Page { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::page::run(&sub, &p));
        }
        Some(Command::Block { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::block::run(&sub, &p));
        }
        Some(Command::Plugin { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            cmd::plugin::run(&sub, &p)
        }
        Some(Command::Daily { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::daily::run(&sub, &p));
        }
        Some(Command::Search(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::search::run(&args, &p));
        }
        Some(Command::Query(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::query::run(&args, &p));
        }
        Some(Command::Backlinks { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::backlinks::run(&sub, &p));
        }
        Some(Command::Batch(args)) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::batch::run(&args, &p));
        }
        Some(Command::Tag { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::tag::run(&sub, &p));
        }
        Some(Command::Template { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            std::process::exit(cmd::template::run(&sub, &p));
        }
        Some(Command::Export { sub, to }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            match sub {
                Some(ec) => std::process::exit(cmd::export_v2::run(&ec, &p)),
                None => cmd::export::run(to.as_deref().unwrap_or("hugo")),
            }
        }
        Some(Command::Workspace { sub }) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            match sub {
                WorkspaceSubcommand::Info(args) => {
                    std::process::exit(cmd::workspace_info::run(&args, &p));
                }
            }
        }
        Some(Command::Mcp { sub }) => match sub {
            McpSubcommand::Serve {} => {
                let p = resolve_path(cli.workspace.as_ref(), None)?;
                mcp::serve(p)
            }
        },
        Some(Command::Peer { cmd }) => {
            // Identity is per-DEVICE → global `~/.outl/identity.key`.
            let outl_dir = outl_sync_iroh::default_device_dir()?;
            std::fs::create_dir_all(&outl_dir)?;
            let id_path = outl_dir.join("identity.key");
            let identity = outl_sync_iroh::IrohIdentity::load_or_generate(&id_path)?;
            // The peer list is per-GRAPH → `<workspace>/.outl/peers.json`. Pairing
            // writes the new peer into the workspace the user is operating on, so
            // it needs the resolved workspace root (not the OS home).
            let ws_root = resolve_path(cli.workspace.as_ref(), None)?;
            outl_sync_iroh::migrate_global_peers_if_absent(&ws_root);
            let peers_path = outl_sync_iroh::workspace_peers_path(&ws_root);
            let mut peers = outl_sync_iroh::PeersStore::load_or_default(&peers_path)?;

            match cmd {
                PeerCommand::Pair { ticket, name } => {
                    let peers_path = peers_path.clone();
                    let identity = std::sync::Arc::new(identity);
                    // The alias is the label THIS device advertises to the peer
                    // (it persists under our node id in the peer's `peers.json`).
                    // `--name` wins; otherwise fall back to the machine hostname
                    // so the peer list reads "macbook" instead of a node-id stub.
                    let alias = name.or_else(default_device_name);
                    let rt = tokio::runtime::Runtime::new()
                        .context("build tokio runtime for pairing")?;

                    if let Some(ticket_str) = ticket {
                        println!("Connecting to the other device…");
                        let (entry, adopted) = rt.block_on(outl_sync_iroh::join_pairing(
                            identity,
                            &ticket_str,
                            &peers_path,
                            &ws_root,
                            alias,
                        ))?;
                        let prefix = &entry.node_id[..entry.node_id.len().min(12)];
                        println!("Paired with {prefix}");
                        match adopted {
                            outl_sync_iroh::WorkspaceAdoption::Adopted(id) => println!(
                                "Joined the host's workspace ({id}). Run `outl sync` (or just \
                                 `outl`) to pull its notes."
                            ),
                            outl_sync_iroh::WorkspaceAdoption::AlreadyMatched => {
                                println!("Already on the host's workspace — nothing to adopt.")
                            }
                            outl_sync_iroh::WorkspaceAdoption::HostSentNone => println!(
                                "Warning: the host advertised no workspace id (older build?), so \
                                 this device kept its own. Sync won't converge until the host \
                                 upgrades and you re-pair."
                            ),
                        }
                    } else {
                        println!("Node ID: {}", identity.node_id());
                        let entry = rt.block_on(outl_sync_iroh::host_pairing(
                            identity,
                            &peers_path,
                            &ws_root,
                            alias,
                            |ticket, qr| {
                                println!();
                                println!("Scan this QR on the other device, or copy the ticket:");
                                println!();
                                println!("{qr}");
                                println!("Ticket:");
                                println!("{ticket}");
                                println!();
                                println!("On the other device, run:");
                                println!("  outl peer pair --ticket <ticket>");
                                println!();
                                println!("Waiting for the other device to connect…");
                            },
                        ))?;
                        let prefix = &entry.node_id[..entry.node_id.len().min(12)];
                        println!("Paired with {prefix}");
                    }
                }
                PeerCommand::List => {
                    let list = peers.list();
                    if list.is_empty() {
                        println!("No paired devices. Use `outl peer pair` to add one.");
                    } else {
                        println!("{:<20} {:<20} ADDED", "NODE ID (prefix)", "ALIAS");
                        for p in list {
                            let short = &p.node_id[..p.node_id.len().min(20)];
                            let alias = p.alias.as_deref().unwrap_or("-");
                            println!("{:<20} {:<20} {}", short, alias, p.added_at);
                        }
                    }
                }
                PeerCommand::Remove { id } => match peers.remove(&id)? {
                    true => println!("Removed peer {id}"),
                    false => println!("No peer matching '{id}' found."),
                },
                PeerCommand::Status => {
                    use outl_sync_iroh::{LeaseDenied, PeerProbe};
                    match outl_sync_iroh::probe_peers_blocking(&id_path, &peers)? {
                        PeerProbe::EndpointBusy(LeaseDenied::HeldByAnotherProcess) => println!(
                            "Another outl process holds this device's sync endpoint, so \
                             reachability here is unknown rather than offline."
                        ),
                        // Nobody holds it: the lease could not be arbitrated at
                        // all, so pointing the user at a co-resident process to
                        // shut down would send them hunting for one that does
                        // not exist.
                        PeerProbe::EndpointBusy(denied) => println!(
                            "Cannot measure reachability here: {denied}. Peers are \
                             unknown rather than offline."
                        ),
                        PeerProbe::Probed(s) if s.is_empty() => println!("No paired devices."),
                        PeerProbe::Probed(statuses) => {
                            println!("{:<22} {:<16} STATUS", "NODE ID (prefix)", "ALIAS");
                            for s in statuses {
                                let short = &s.node_id[..s.node_id.len().min(22)];
                                let alias = s.alias.as_deref().unwrap_or("-");
                                let state = if s.online {
                                    match s.rtt_ms {
                                        Some(ms) => format!("online ({ms}ms)"),
                                        None => "online".into(),
                                    }
                                } else {
                                    "offline".into()
                                };
                                println!("{short:<22} {alias:<16} {state}");
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        Some(Command::Sync) => {
            let p = resolve_path(cli.workspace.as_ref(), None)?;
            // Exit here, not inside `run_sync`: it has already dropped its
            // transport (and the endpoint lease with it), so nothing is skipped.
            std::process::exit(run_sync(&p)?);
        }
    }
}

/// Exit code for an `outl sync` that flushed nothing. Not [`output::EXIT_OK`]:
/// `outl page create … && outl sync` cannot otherwise tell a real push from a
/// "trust the neighbour process to push within `MAINTENANCE_RESYNC`". Not
/// `EXIT_USER` / `EXIT_INTERNAL` either — nothing is wrong. So: the next free
/// number after [`output`]'s 0/1/2.
const EXIT_NOTHING_FLUSHED: i32 = 3;

/// Force a one-shot P2P sync pass: bring a transport up, let the boot-time +
/// catch-up sync exchange ops with every paired device, then shut down.
///
/// An ephemeral CLI mutation can't keep a QUIC connection alive long enough to
/// push, so this is the explicit flush.
///
/// It takes the device endpoint lease like any other client, and **stands down
/// when it can't get it**. A second endpoint on this device's node id steals
/// the relay route from the process that already has it and breaks that
/// process's sync in both directions for the 25s this command runs — while the
/// holder was already going to push these ops on its next catch-up pass. So the
/// honest answer there is to say who has it and exit, not to flush by breaking
/// the thing doing the flushing. Returns [`output::EXIT_OK`] when a pass ran and
/// [`EXIT_NOTHING_FLUSHED`] when it stood down: printing the reason is not
/// enough for a command built to be scripted, and stdout is not what `&&` reads.
fn run_sync(path: &std::path::Path) -> anyhow::Result<i32> {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};

    use outl_actions::SyncTransport;
    use outl_sync_iroh::{LeaseDenied, TransportOutcome};

    let wc = ws::open(path).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;
    let transport = match outl_sync_iroh::build_default_transport(path)? {
        TransportOutcome::Ready(t) => t,
        TransportOutcome::EndpointBusy(LeaseDenied::HeldByAnotherProcess) => {
            println!(
                "Another outl process on this device holds the sync endpoint \
                 (a GUI, or `outl mcp serve`).\nIt pushes these ops out on its own \
                 pass — nothing to flush here."
            );
            return Ok(EXIT_NOTHING_FLUSHED);
        }
        // Not "someone else has it": there is no arbiter, so no process on this
        // device can bind. Saying "busy" here would promise a holder that will
        // eventually exit and free it, and nothing ever would.
        TransportOutcome::EndpointBusy(denied) => {
            println!(
                "No P2P endpoint here: {denied}.\nOps stay in ops/ and converge \
                 through the file transport; nothing was flushed."
            );
            return Ok(EXIT_NOTHING_FLUSHED);
        }
        TransportOutcome::Disabled => {
            println!("`[sync] transport` is \"file\"; P2P sync is off. Nothing to flush.");
            return Ok(EXIT_NOTHING_FLUSHED);
        }
    };
    if transport.peers().is_empty() {
        println!("No paired devices. Use `outl peer pair` to add one.");
        return Ok(EXIT_NOTHING_FLUSHED);
    }

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    transport.start(wc.root.clone(), wc.actor, tx);
    println!("Syncing with paired devices…");

    // Cross-network connects can take ~20s (iroh multipath), so wait up to
    // `MAX`; but return early once a baseline has passed with no new peer ops
    // (the exchange has gone quiet → converged or nothing to pull).
    const MAX: Duration = Duration::from_secs(25);
    const BASELINE: Duration = Duration::from_secs(6);
    const QUIET: Duration = Duration::from_secs(4);
    let start = Instant::now();
    let mut last_activity = start;
    while start.elapsed() < MAX {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => last_activity = Instant::now(),
            Err(RecvTimeoutError::Timeout) => {
                if start.elapsed() >= BASELINE && last_activity.elapsed() >= QUIET {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let health = transport.peer_health();
    transport.shutdown();

    let online = health.iter().filter(|h| h.reachable).count();
    println!(
        "Sync pass complete — {online}/{} peer(s) reachable.",
        health.len()
    );
    Ok(output::EXIT_OK)
}

/// Resolve which workspace path to operate on.
///
/// Precedence (first hit wins):
///
/// 1. **Subcommand-positional** path (`outl page get … <path>`).
/// 2. **Global `--workspace <DIR>`** flag.
/// 3. **`workspace.last`** from `~/.config/outl/config.toml`
///    (the same file the desktop's Settings modal writes when the
///    user picks a workspace — so `outl` with no args lands on the
///    workspace the user last opened in the GUI, no `--workspace`
///    flag needed).
/// 4. **Current directory** — final fallback (matches the
///    `cd ~/notes && outl` muscle memory).
///
/// A path stored in `config.toml` that no longer exists on disk is
/// skipped silently rather than failing the launch — the user
/// likely deleted / unmounted the folder and would be surprised by
/// a crash. The cwd fallback picks up.
/// Best-effort device label for `outl peer pair` when `--name` is omitted.
///
/// Shells out to the `hostname` command (present on macOS + Linux) so the
/// peer's device list reads "macbook" instead of a node-id stub, trimming the
/// macOS `.local` suffix. Returns `None` if the command is unavailable or
/// empty — pairing then advertises no alias, exactly as before this flag.
/// Kept dependency-free on purpose; `--name` is the explicit override.
fn default_device_name() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let raw = String::from_utf8(out.stdout).ok()?;
    let name = raw.trim();
    let name = name.strip_suffix(".local").unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

fn resolve_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = local {
        return Ok(p.clone());
    }
    if let Some(p) = global {
        return Ok(p.clone());
    }
    if let Some(p) = outl_config::load().workspace.last {
        if p.exists() {
            return Ok(p);
        }
        tracing::warn!(
            "config.toml workspace.last = {} is no longer on disk; falling back to cwd",
            p.display()
        );
    }
    std::env::current_dir().with_context(|| "reading current directory")
}

/// If `path` has no `.outl/` directory yet, prompt the user for
/// permission to initialize one. If they say no, error out cleanly.
///
/// When stdin isn't a TTY (e.g. piped) we don't prompt — instead we
/// error with the same "run `outl init`" message we used before. This
/// keeps scripted callers predictable.
fn ensure_workspace_or_prompt(path: &Path) -> Result<()> {
    let outl_dir = path.join(".outl");
    if outl_dir.exists() {
        return Ok(());
    }

    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        anyhow::bail!(
            "no outl workspace at {} — run `outl init {}` first",
            path.display(),
            path.display()
        );
    }

    use std::io::{BufRead, Write};
    eprintln!("No outl workspace at {}.", path.display());
    eprint!("Initialize a new workspace here? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .with_context(|| "reading prompt response")?;
    let answer = line.trim().to_lowercase();
    if answer == "y" || answer == "yes" {
        cmd::init::run(path, "global")?;
        Ok(())
    } else {
        anyhow::bail!("aborted — no workspace initialized at {}", path.display());
    }
}

/// Same as [`resolve_path`] but errors out when neither flag nor positional
/// was given (init refuses to create a workspace at the cwd by accident).
fn resolve_init_path(global: Option<&PathBuf>, local: Option<&PathBuf>) -> Result<PathBuf> {
    match local.or(global) {
        Some(p) => Ok(p.clone()),
        None => Err(anyhow::anyhow!(
            "`outl init` needs an explicit path: pass a positional argument or `--workspace <DIR>`"
        )),
    }
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        // Logs MUST go to stderr; stdout carries the JSON envelope
        // that scripts/tests parse. Without this, every `INFO` line
        // from `JsonlStorage::reload` corrupts the response.
        .with_writer(std::io::stderr)
        .try_init();
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
