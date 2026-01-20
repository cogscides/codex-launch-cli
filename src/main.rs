mod config;
mod pathfmt;
mod projects;
mod sessions;
mod timefmt;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::projects::ProjectTarget;
use crate::sessions::SessionItem;

#[derive(Debug, Parser)]
#[command(
    name = "codex-launch",
    version,
    about = "Interactive launcher for Codex CLI"
)]
struct Cli {
    /// Path to config TOML (default: ~/.codex-launch/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print commands without executing
    #[arg(long)]
    dry_run: bool,

    /// Disable interactive prompts; print lists instead (useful in non-TTY)
    #[arg(long)]
    no_ui: bool,

    /// Shortcut for `recent` interactive picker
    #[arg(long)]
    recent: bool,

    /// With `--recent`, include sessions outside configured targets
    #[arg(long)]
    all_sessions: bool,

    /// With `--recent`, override how many sessions to show
    #[arg(long)]
    limit: Option<usize>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Interactive picker (default)
    Pick,

    /// List discovered targets
    List,

    /// Add a root folder (one-level scan for git repos)
    AddRoot { path: PathBuf },

    /// Add an explicit folder target (git or non-git)
    AddPath { path: PathBuf },

    /// Remove a configured root/path (exact match)
    Rm { path: PathBuf },

    /// Show recent sessions and resume one
    Recent {
        /// Show only sessions whose cwd is under configured roots/paths
        #[arg(long)]
        scoped: bool,

        /// How many sessions to show (default from config)
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Resume a specific session id
    Resume { id: String },

    /// Print resolved config path and exit
    WhereConfig,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = config::resolve_config_path(cli.config.as_deref())?;
    let mut cfg = Config::load_or_init(&config_path)?;

    if cli.cmd.is_none() && cli.recent {
        if !cli.no_ui && (!std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()) {
            anyhow::bail!(
                "The input device is not a TTY. Re-run with `--no-ui` to print lists without prompts."
            );
        }
        let query = if cli.all_sessions {
            sessions::SessionQuery::All {
                limit: cli.limit.unwrap_or(cfg.sessions.limit),
            }
        } else {
            sessions::SessionQuery::Scoped {
                limit: cli.limit.unwrap_or(cfg.sessions.limit),
            }
        };
        let items = sessions::list_recent_sessions(&cfg, query)?;
        if items.is_empty() {
            println!("No sessions found.");
            return Ok(());
        }
        if cli.no_ui {
            for s in items {
                println!(
                    "{}\t{}\t{}\t{}",
                    s.id,
                    s.created_at.as_deref().unwrap_or(""),
                    s.cwd.display(),
                    s.summary.as_deref().unwrap_or("")
                );
            }
            return Ok(());
        } else {
            let picked = ui::pick_session(&items)?;
            return run_codex_resume(&cfg, &picked, cli.dry_run);
        }
    }

    match cli.cmd.unwrap_or(Cmd::Pick) {
        Cmd::WhereConfig => {
            println!("{}", config_path.display());
            Ok(())
        }
        Cmd::AddRoot { path } => {
            cfg.add_root(path)?;
            cfg.save(&config_path)?;
            Ok(())
        }
        Cmd::AddPath { path } => {
            cfg.add_path(path)?;
            cfg.save(&config_path)?;
            Ok(())
        }
        Cmd::Rm { path } => {
            cfg.remove_path_or_root(path)?;
            cfg.save(&config_path)?;
            Ok(())
        }
        Cmd::List => {
            let targets = projects::gather_targets(&cfg)?;
            for t in targets {
                println!("{t}");
            }
            Ok(())
        }
        Cmd::Resume { id } => {
            if let Some(item) = sessions::find_session_by_id(&cfg, &id)? {
                run_codex_resume(&cfg, &item, cli.dry_run)
            } else {
                anyhow::bail!("session id not found: {id}");
            }
        }
        Cmd::Recent { scoped, limit } => {
            if !cli.no_ui && (!std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()) {
                anyhow::bail!(
                    "The input device is not a TTY. Re-run with `--no-ui` to print lists without prompts."
                );
            }
            let query = if scoped {
                sessions::SessionQuery::Scoped {
                    limit: limit.unwrap_or(cfg.sessions.limit),
                }
            } else {
                sessions::SessionQuery::All {
                    limit: limit.unwrap_or(cfg.sessions.limit),
                }
            };
            let items = sessions::list_recent_sessions(&cfg, query)?;
            if items.is_empty() {
                println!("No sessions found.");
                return Ok(());
            }
            if cli.no_ui {
                for s in items {
                    println!(
                        "{}\t{}\t{}\t{}",
                        s.id,
                        s.created_at.as_deref().unwrap_or(""),
                        s.cwd.display(),
                        s.summary.as_deref().unwrap_or("")
                    );
                }
                Ok(())
            } else {
                let picked = ui::pick_session(&items)?;
                run_codex_resume(&cfg, &picked, cli.dry_run)
            }
        }
        Cmd::Pick => {
            let targets = projects::gather_targets(&cfg)?;
            if targets.is_empty() {
                anyhow::bail!(
                    "No targets configured. Add a root with `codex-launch add-root <path>` or an explicit folder with `codex-launch add-path <path>`."
                );
            }

            if cli.no_ui {
                for t in targets {
                    println!("{}", t.path.display());
                }
                Ok(())
            } else {
                if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                    anyhow::bail!(
                        "The input device is not a TTY. Re-run with `--no-ui` to print lists without prompts."
                    );
                }
                let target = ui::pick_target(&targets)?;
                pick_action_loop(&cfg, &target, cli.dry_run)
            }
        }
    }
}

fn pick_action_loop(cfg: &Config, target: &ProjectTarget, dry_run: bool) -> Result<()> {
    loop {
        match ui::pick_action(target)? {
            ui::Action::NewSession => return run_codex_new(cfg, target, dry_run),
            ui::Action::ResumeRecentForTarget => {
                let scoped_repo_root = sessions::git_root_for_path(&target.path);
                let hide_path = scoped_repo_root.is_some();
                let items = sessions::list_recent_sessions(
                    cfg,
                    scoped_repo_root
                        .map(|repo_root| sessions::SessionQuery::ForRepoRoot {
                            repo_root,
                            limit: cfg.sessions.limit,
                        })
                        .unwrap_or_else(|| sessions::SessionQuery::ForCwd {
                            cwd: target.path.clone(),
                            limit: cfg.sessions.limit,
                        }),
                )?;
                if items.is_empty() {
                    ui::print_info("No recent sessions for this folder.");
                    continue;
                }
                let picked = ui::pick_session_scoped(&items, hide_path)?;
                return run_codex_resume(cfg, &picked, dry_run);
            }
            ui::Action::BrowseRecentGlobal => {
                let items = sessions::list_recent_sessions(
                    cfg,
                    sessions::SessionQuery::Scoped {
                        limit: cfg.sessions.limit,
                    },
                )?;
                if items.is_empty() {
                    ui::print_info("No recent sessions found.");
                    continue;
                }
                let picked = ui::pick_session(&items)?;
                return run_codex_resume(cfg, &picked, dry_run);
            }
            ui::Action::Back => return Ok(()),
        }
    }
}

fn run_codex_new(cfg: &Config, target: &ProjectTarget, dry_run: bool) -> Result<()> {
    let mut cmd = Command::new(&cfg.codex.bin);
    cmd.current_dir(&target.path);
    cmd.args(cfg.codex.args.iter());

    ui::print_info(&format!("Launching Codex in {}", target.path.display()));
    run_command(cmd, dry_run)
}

fn run_codex_resume(cfg: &Config, session: &SessionItem, dry_run: bool) -> Result<()> {
    let mut cmd = Command::new(&cfg.codex.bin);
    cmd.current_dir(&session.cwd);
    cmd.args(cfg.codex.args.iter());
    cmd.arg("resume");
    cmd.arg(&session.id);

    ui::print_info(&format!(
        "Resuming {} in {}",
        session.id,
        session.cwd.display()
    ));
    run_command(cmd, dry_run)
}

fn run_command(mut cmd: Command, dry_run: bool) -> Result<()> {
    if dry_run {
        ui::print_info(&format!("DRY RUN: {}", ui::format_command(&cmd)));
        return Ok(());
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {}", ui::format_command(&cmd)))?;
    if !status.success() {
        anyhow::bail!("command exited with status: {status}");
    }
    Ok(())
}
