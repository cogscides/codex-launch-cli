use std::ffi::OsStr;
use std::fmt;
use std::process::Command;

use anyhow::Result;
use console::style;
use crossterm::terminal;
use inquire::{Select, validator::Validation};

use crate::projects::ProjectTarget;
use crate::sessions::SessionItem;

#[derive(Debug, Clone)]
struct UiOption<T> {
    value: T,
    line: String,
}

impl<T> fmt::Display for UiOption<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.line)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewSession,
    ResumeRecentForTarget,
    BrowseRecentGlobal,
    BrowseRecentAllGlobal,
    OpenConfig,
    Back,
}

pub fn pick_target(targets: &[ProjectTarget]) -> Result<ProjectTarget> {
    let width = terminal_width().saturating_sub(4);
    let options = targets
        .iter()
        .cloned()
        .map(|t| UiOption {
            line: truncate_to_width(t.to_string(), width),
            value: t,
        })
        .collect::<Vec<_>>();
    let picked = Select::new("Pick a folder:", options)
        .with_help_message("↑↓ to move, enter to select, type to filter (name/path)")
        .with_page_size(20.min(targets.len().max(1)))
        .prompt()?;
    Ok(picked.value)
}

pub fn pick_action(target: &ProjectTarget) -> Result<Action> {
    let title = format!("Action for {}:", style(&target.label).cyan());
    let opts = vec![
        "New Codex session",
        "Resume a recent session for this folder",
        "Browse recent sessions (scoped)",
        "Browse recent sessions (all)",
        "Open config",
        "Back",
    ];
    let picked = Select::new(&title, opts).prompt()?;
    Ok(match picked {
        "New Codex session" => Action::NewSession,
        "Resume a recent session for this folder" => Action::ResumeRecentForTarget,
        "Browse recent sessions (scoped)" => Action::BrowseRecentGlobal,
        "Browse recent sessions (all)" => Action::BrowseRecentAllGlobal,
        "Open config" => Action::OpenConfig,
        _ => Action::Back,
    })
}

pub fn pick_session(items: &[SessionItem]) -> Result<SessionItem> {
    let width = terminal_width().saturating_sub(4);
    let options = items
        .iter()
        .cloned()
        .map(|s| UiOption {
            line: truncate_to_width(s.to_string(), width),
            value: s,
        })
        .collect::<Vec<_>>();
    let picked = Select::new("Pick a session to resume:", options)
        .with_help_message("↑↓ to move, enter to select, type to filter")
        .with_page_size(20.min(items.len().max(1)))
        .prompt()?;
    Ok(picked.value)
}

pub fn pick_session_scoped(items: &[SessionItem], hide_path: bool) -> Result<SessionItem> {
    let width = terminal_width().saturating_sub(4);
    let options = items
        .iter()
        .cloned()
        .map(|s| UiOption {
            line: truncate_to_width(
                if hide_path {
                    session_line_no_path(&s)
                } else {
                    s.to_string()
                },
                width,
            ),
            value: s,
        })
        .collect::<Vec<_>>();
    let picked = Select::new("Pick a session to resume:", options)
        .with_help_message("↑↓ to move, enter to select, type to filter")
        .with_page_size(20.min(items.len().max(1)))
        .prompt()?;
    Ok(picked.value)
}

pub fn print_info(msg: &str) {
    eprintln!("{} {}", style("info").dim(), msg);
}

pub fn format_command(cmd: &Command) -> String {
    let program = cmd.get_program().to_string_lossy().to_string();
    let args = cmd
        .get_args()
        .map(shell_escape)
        .collect::<Vec<_>>()
        .join(" ");
    let base = if args.is_empty() {
        program
    } else {
        format!("{program} {args}")
    };
    if let Some(dir) = cmd.get_current_dir() {
        format!("(cd {} && {})", dir.display(), base)
    } else {
        base
    }
}

fn shell_escape(s: &OsStr) -> String {
    let t = s.to_string_lossy();
    if t.is_empty() {
        "''".to_string()
    } else if t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        t.to_string()
    } else {
        format!("'{}'", t.replace('\'', "'\\''"))
    }
}

fn terminal_width() -> usize {
    terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(120)
        .clamp(60, 240)
}

fn truncate_to_width(s: String, max_chars: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() <= max_chars {
        return s;
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i + 1 >= max_chars.saturating_sub(1) {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn session_line_no_path(s: &SessionItem) -> String {
    // SessionItem Display includes cwd; for a scoped picker that's redundant.
    let id_short = s.id.chars().take(8).collect::<String>();
    let when = s
        .created_at
        .as_deref()
        .and_then(crate::timefmt::parse_rfc3339)
        .map(|dt| {
            format!(
                "{} {}",
                crate::timefmt::format_age(dt),
                crate::timefmt::format_short(dt)
            )
        })
        .unwrap_or_else(|| "-".to_string());

    let summary = s
        .summary
        .as_deref()
        .map(|x| x.to_string())
        .unwrap_or_default();

    let mut meta = Vec::new();
    if let Some(p) = s.model_provider.as_deref() {
        meta.push(p);
    }
    if let Some(src) = s.source.as_deref() {
        meta.push(src);
    }
    let meta = if meta.is_empty() {
        String::new()
    } else {
        format!("  [{}]", meta.join(" "))
    };

    if summary.trim().is_empty() {
        format!("{:<12}  {:<8}{}", when, id_short, meta)
    } else {
        format!("{:<12}  {:<8}  {}{}", when, id_short, summary.trim(), meta)
    }
}

#[allow(dead_code)]
fn validate_nonempty(input: &str) -> Result<Validation, inquire::CustomUserError> {
    if input.trim().is_empty() {
        Ok(Validation::Invalid("must not be empty".into()))
    } else {
        Ok(Validation::Valid)
    }
}
