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
use clap::Parser;

mod cli;
mod cmd;
mod human;
mod mcp;
mod output;
mod startup;
mod sync_engine;
mod workspace_layout;
mod ws;

pub(crate) use cli::{
    Cli, Command, McpSubcommand, PeerCommand, ThemeSubcommand, WorkspaceSubcommand,
};
use startup::{
    default_device_name, ensure_workspace_or_prompt, init_tracing, resolve_init_path, resolve_path,
};

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
        Some(Command::Init { path, scope, bare }) => {
            let p = resolve_init_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::init::run(&p, &scope, bare)
        }
        Some(Command::MigrateToPerPageOps { path }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::migrate_to_per_page_ops::run(&p)
        }
        Some(Command::Compact {
            path,
            apply,
            no_horizon,
            force,
        }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::compact::run(&p, apply, no_horizon, force)
        }
        Some(Command::Serve {
            path,
            once,
            no_watch,
            no_sync,
        }) => {
            let p = resolve_path(cli.workspace.as_ref(), path.as_ref())?;
            cmd::serve::run(&p, once, !no_watch, !no_sync)
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

                    if let Some(ticket_arg) = ticket {
                        let ticket_str = cmd::peer_qr::read_ticket(Some(&ticket_arg))?;
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
                            |ticket| {
                                println!();
                                // Rendered here rather than used from `qr`:
                                // the callback's copy is unconditional, and
                                // a QR wider than the terminal wraps into
                                // noise that buries the ticket below it.
                                if let Err(e) = cmd::peer_qr::print_ticket_qr_if_it_fits(ticket) {
                                    println!("(could not render the pairing QR: {e:#})");
                                }
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
                PeerCommand::Qr { ticket } => cmd::peer_qr::run(ticket.as_deref())?,
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
                    // Say what this actually does. The command reads as
                    // "revoke", and for a long time it did less than that: the
                    // entry came back from membership gossip within ~5s and the
                    // device kept syncing (issue #158). It now sticks *here*,
                    // and that "here" is the part a user has to be told —
                    // silently implying a mesh-wide revocation is how a lost
                    // laptop stays in someone's graph.
                    true => {
                        println!("Removed peer {id} from this device.");
                        println!(
                            "  This device will no longer sync with it, and won't re-add it \
                             from membership gossip."
                        );
                        println!(
                            "  Your OTHER paired devices still have it. Run the same command \
                             on each of them to cut it off completely."
                        );
                    }
                    false => println!("No peer matching '{id}' found."),
                },
                PeerCommand::RevokeAll { yes } => {
                    // Destructive and not undoable: every pairing goes, and
                    // every device has to be re-paired by hand. Confirm unless
                    // the user opted out explicitly.
                    let store = outl_sync_iroh::PeersStore::load_or_default(&peers_path)?;
                    let count = store.list().len();
                    if !yes {
                        println!(
                            "This will unpair {count} device(s) and change this workspace's identity."
                        );
                        println!("Every device you still have will need `outl peer pair` again.");
                        println!(
                            "A device you do NOT re-pair can never sync with this workspace again."
                        );
                        println!();
                        print!("Type 'revoke' to continue: ");
                        use std::io::Write as _;
                        std::io::stdout().flush().ok();
                        let mut answer = String::new();
                        std::io::stdin().read_line(&mut answer)?;
                        if answer.trim() != "revoke" {
                            println!("Cancelled. Nothing changed.");
                            return Ok(());
                        }
                    }

                    let unpaired = outl_sync_iroh::rotate_workspace_identity(&ws_root)?;
                    println!("Workspace identity rotated. {unpaired} device(s) unpaired.");
                    println!();
                    println!("Next steps:");
                    println!("  1. Run `outl peer pair` on this device and each device you keep.");
                    println!("  2. If a GUI or `outl serve` is running, restart it — it is still");
                    println!("     holding the old identity in memory.");
                    println!();
                    // Never let this read as "the data came back". It did not.
                    println!("The revoked device keeps the copy of your notes it already synced.");
                    println!(
                        "Rotation stops it receiving anything new; it cannot take back history."
                    );
                }
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
