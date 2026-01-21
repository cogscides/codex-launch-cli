mod config;
mod pathfmt;
mod projects;
mod quick;
mod sessions;
mod timefmt;
mod tui;
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

    /// Quick resume by searching recent sessions (matches id/cwd/summary)
    #[arg(long, value_name = "QUERY")]
    resume: Option<String>,

    /// Quick launch by searching projects (positional query)
    #[arg(value_name = "PROJECT")]
    project: Option<String>,

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

    /// Resume a specific session id (exact)
    ResumeId { id: String },

    /// Print resolved config path and exit
    WhereConfig,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = config::resolve_config_path(cli.config.as_deref())?;
    let mut cfg = Config::load_or_init(&config_path)?;

    if cli.cmd.is_none() && cli.resume.is_some() {
        return quick::resume_by_query(
            &cfg,
            cli.resume.as_deref().unwrap_or_default(),
            cli.dry_run,
        );
    }

    if cli.cmd.is_none() && cli.project.is_some() {
        return quick::launch_by_query(
            &cfg,
            cli.project.as_deref().unwrap_or_default(),
            cli.dry_run,
        );
    }

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
        Cmd::ResumeId { id } => {
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
            let mut targets = projects::gather_targets(&cfg)?;
            if targets.is_empty() {
                anyhow::bail!(
                    "No targets configured. Add a root with `codex-launch add-root <path>` or an explicit folder with `codex-launch add-path <path>`."
                );
            }
            prioritize_current_target(&cfg, &mut targets)?;
            let sessions_index = sessions::list_recent_sessions(
                &cfg,
                sessions::SessionQuery::All {
                    limit: cfg.projects.sessions_limit.max(cfg.sessions.limit),
                },
            )?;
            let sessions_scoped = sessions_index
                .iter()
                .filter(|s| cfg.is_scoped_target(&s.cwd))
                .cloned()
                .collect::<Vec<_>>();

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
                match tui::pick_project(
                    &targets,
                    &sessions_scoped,
                    &sessions_index,
                    cfg.sessions.limit,
                )? {
                    tui::ProjectPick::New(target) => run_codex_new(&cfg, &target, cli.dry_run),
                    tui::ProjectPick::Resume(session) => {
                        run_codex_resume(&cfg, &session, cli.dry_run)
                    }
                    tui::ProjectPick::OpenConfig => open_config(&config_path, cli.dry_run),
                    tui::ProjectPick::Quit => Ok(()),
                }
            }
        }
    }
}

fn prioritize_current_target(cfg: &Config, targets: &mut Vec<ProjectTarget>) -> Result<()> {
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    if !cwd.exists() || !cwd.is_dir() {
        return Ok(());
    }

    // Prefer the git repo root when inside a repo; otherwise just use the cwd.
    let cur_path = sessions::git_root_for_path(&cwd).unwrap_or(cwd);

    if let Some(pos) = targets.iter().position(|t| t.path == cur_path) {
        let t = targets.remove(pos);
        targets.insert(0, t);
        return Ok(());
    }

    // Only insert if it looks like a meaningful directory (avoid adding "/").
    if cur_path.parent().is_none() {
        return Ok(());
    }

    let mut t = ProjectTarget {
        path: cur_path.clone(),
        kind: crate::projects::TargetKind::CurrentWorkingDir,
        label: crate::pathfmt::basename(&cur_path),
        last_session_at: None,
        last_session_summary: None,
    };

    // Best-effort: populate last-session metadata for display.
    let meta_query = sessions::git_root_for_path(&cur_path)
        .map(|repo_root| sessions::SessionQuery::ForRepoRoot {
            repo_root,
            limit: 1,
        })
        .unwrap_or_else(|| sessions::SessionQuery::ForCwd {
            cwd: cur_path.clone(),
            limit: 1,
        });
    if let Ok(mut items) = sessions::list_recent_sessions(cfg, meta_query) {
        if let Some(s) = items.pop() {
            t.last_session_at = s.created_at;
            t.last_session_summary = s.summary;
        }
    }

    targets.insert(0, t);
    Ok(())
}

fn open_config(config_path: &std::path::Path, dry_run: bool) -> Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(config_path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start"]).arg(config_path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(config_path);
        c
    };
    ui::print_info(&format!("Opening config {}", config_path.display()));
    run_command(cmd, dry_run)
}

pub(crate) fn run_codex_new(cfg: &Config, target: &ProjectTarget, dry_run: bool) -> Result<()> {
    let mut cmd = Command::new(&cfg.codex.bin);
    cmd.current_dir(&target.path);
    cmd.args(cfg.codex.args.iter());

    ui::print_info(&format!("Launching Codex in {}", target.path.display()));
    run_command(cmd, dry_run)
}

pub(crate) fn run_codex_resume(cfg: &Config, session: &SessionItem, dry_run: bool) -> Result<()> {
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
