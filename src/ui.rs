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

#[allow(dead_code)]
fn validate_nonempty(input: &str) -> Result<Validation, inquire::CustomUserError> {
    if input.trim().is_empty() {
        Ok(Validation::Invalid("must not be empty".into()))
    } else {
        Ok(Validation::Valid)
    }
}
