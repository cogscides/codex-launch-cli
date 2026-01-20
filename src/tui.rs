use std::io::{self, Write};
use std::panic;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{self, Stylize};
use crossterm::terminal::{self, ClearType};
use crossterm::{QueueableCommand, execute};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::projects::ProjectTarget;

#[derive(Debug, Clone)]
pub enum ProjectPick {
    New(ProjectTarget),
    Menu(ProjectTarget),
    ResumeRepo(ProjectTarget),
    ResumeScopedGlobal,
    ResumeAllGlobal,
    OpenConfig,
    Quit,
}

pub fn pick_project(targets: &[ProjectTarget]) -> Result<ProjectPick> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        pick_project_inner(&mut stdout, targets)
    }));
    match result {
        Ok(r) => r,
        Err(_) => anyhow::bail!(
            "UI crashed (panic). Terminal should be restored; re-run with `--no-ui` if needed."
        ),
    }
}

fn pick_project_inner(stdout: &mut io::Stdout, targets: &[ProjectTarget]) -> Result<ProjectPick> {
    let mut filter = String::new();
    let mut cursor_idx: usize = 0;
    let matcher = SkimMatcherV2::default().ignore_case();

    loop {
        let (cols, rows) = terminal::size()?;
        let cols = cols as usize;
        let rows = rows as usize;

        let filtered = filter_targets(targets, &matcher, &filter);
        if cursor_idx >= filtered.len() && !filtered.is_empty() {
            cursor_idx = filtered.len() - 1;
        }

        render(stdout, targets, &filtered, cursor_idx, &filter, cols, rows)?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let ev = event::read()?;
        match ev {
            Event::Key(k) => match handle_key(k, &mut filter, &mut cursor_idx, filtered.len())? {
                KeyOutcome::Continue => {}
                KeyOutcome::Quit => return Ok(ProjectPick::Quit),
                KeyOutcome::OpenConfig => return Ok(ProjectPick::OpenConfig),
                KeyOutcome::ResumeScopedGlobal => return Ok(ProjectPick::ResumeScopedGlobal),
                KeyOutcome::ResumeAllGlobal => return Ok(ProjectPick::ResumeAllGlobal),
                KeyOutcome::New => {
                    if let Some(t) = selected(targets, &filtered, cursor_idx) {
                        return Ok(ProjectPick::New(t));
                    }
                }
                KeyOutcome::Menu => {
                    if let Some(t) = selected(targets, &filtered, cursor_idx) {
                        return Ok(ProjectPick::Menu(t));
                    }
                }
                KeyOutcome::ResumeRepo => {
                    if let Some(t) = selected(targets, &filtered, cursor_idx) {
                        return Ok(ProjectPick::ResumeRepo(t));
                    }
                }
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn selected(
    targets: &[ProjectTarget],
    filtered: &[usize],
    cursor_idx: usize,
) -> Option<ProjectTarget> {
    filtered
        .get(cursor_idx)
        .and_then(|idx| targets.get(*idx))
        .cloned()
}

struct TerminalGuard {
    use_alt_screen: bool,
}

impl TerminalGuard {
    fn enter(stdout: &mut io::Stdout) -> Result<Self> {
        terminal::enable_raw_mode()?;

        let use_alt_screen = should_use_alt_screen();
        if use_alt_screen {
            execute!(stdout, terminal::EnterAlternateScreen)?;
        } else {
            execute!(
                stdout,
                terminal::Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
        }
        execute!(stdout, cursor::Hide)?;

        Ok(Self { use_alt_screen })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Show);
        if self.use_alt_screen {
            let _ = execute!(stdout, terminal::LeaveAlternateScreen);
        } else {
            let _ = execute!(stdout, style::Print("\n"));
        }
        let _ = terminal::disable_raw_mode();
    }
}

fn should_use_alt_screen() -> bool {
    if std::env::var_os("CODEX_LAUNCH_NO_ALT_SCREEN").is_some() {
        return false;
    }
    if std::env::var_os("ZELLIJ").is_some() {
        return false;
    }
    true
}

fn filter_targets(targets: &[ProjectTarget], matcher: &SkimMatcherV2, filter: &str) -> Vec<usize> {
    let q = filter.trim();
    if q.is_empty() {
        return (0..targets.len()).collect();
    }

    let mut scored = targets
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let hay = t.to_string();
            matcher.fuzzy_match(&hay, q).map(|score| (score, i))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, i)| i).collect()
}

enum KeyOutcome {
    Continue,
    Quit,
    New,
    Menu,
    ResumeRepo,
    ResumeScopedGlobal,
    ResumeAllGlobal,
    OpenConfig,
}

fn handle_key(
    key: KeyEvent,
    filter: &mut String,
    cursor_idx: &mut usize,
    len: usize,
) -> Result<KeyOutcome> {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => return Ok(KeyOutcome::Quit),
        (KeyCode::Char('q'), KeyModifiers::NONE) => return Ok(KeyOutcome::Quit),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(KeyOutcome::Quit),

        (KeyCode::Enter, _) => return Ok(KeyOutcome::New),
        (KeyCode::Char('m'), KeyModifiers::NONE) => return Ok(KeyOutcome::Menu),
        (KeyCode::Char('r'), KeyModifiers::NONE) => return Ok(KeyOutcome::ResumeRepo),
        (KeyCode::Char('g'), KeyModifiers::NONE) => return Ok(KeyOutcome::ResumeScopedGlobal),
        (KeyCode::Char('a'), KeyModifiers::NONE) => return Ok(KeyOutcome::ResumeAllGlobal),
        (KeyCode::Char('o'), KeyModifiers::NONE) => return Ok(KeyOutcome::OpenConfig),

        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
            if len > 0 {
                *cursor_idx = cursor_idx.saturating_sub(1);
            }
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
            if len > 0 {
                *cursor_idx = (*cursor_idx + 1).min(len.saturating_sub(1));
            }
        }
        (KeyCode::PageUp, _) => {
            *cursor_idx = cursor_idx.saturating_sub(10);
        }
        (KeyCode::PageDown, _) => {
            if len > 0 {
                *cursor_idx = (*cursor_idx + 10).min(len.saturating_sub(1));
            }
        }
        (KeyCode::Home, _) => {
            *cursor_idx = 0;
        }
        (KeyCode::End, _) => {
            if len > 0 {
                *cursor_idx = len - 1;
            }
        }

        (KeyCode::Backspace, _) => {
            filter.pop();
            *cursor_idx = 0;
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            filter.clear();
            *cursor_idx = 0;
        }
        (KeyCode::Char(ch), KeyModifiers::NONE) => {
            if !ch.is_control() {
                filter.push(ch);
                *cursor_idx = 0;
            }
        }
        _ => {}
    }
    Ok(KeyOutcome::Continue)
}

fn render(
    stdout: &mut io::Stdout,
    targets: &[ProjectTarget],
    filtered: &[usize],
    cursor_idx: usize,
    filter: &str,
    cols: usize,
    rows: usize,
) -> Result<()> {
    let mut out = String::new();

    out.push_str(&format!(
        "{}  {}\n",
        "codex-launch".bold(),
        "enter=new  r=resume(repo)  g=resume(scoped)  a=resume(all)  m=menu  o=open-config  q=quit"
            .dim()
    ));
    out.push_str(&format!("{} {}\n", "Filter:".bold(), filter));

    let list_rows = rows.saturating_sub(4).max(1);
    let start = cursor_idx.saturating_sub(list_rows / 2);
    let end = (start + list_rows).min(filtered.len());

    for (row_offset, idx) in filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
    {
        let t = &targets[*idx];
        let mut line = t.to_string();
        if line.chars().count() > cols.saturating_sub(2) {
            line = truncate(line, cols.saturating_sub(3));
        }
        if row_offset == cursor_idx {
            out.push_str(&format!("{}\n", format!("> {line}").reverse()));
        } else {
            out.push_str(&format!("  {line}\n"));
        }
    }

    out.push_str(&format!(
        "{}\n",
        format!("{} / {}", filtered.len(), targets.len()).dim()
    ));

    stdout
        .queue(terminal::Clear(ClearType::All))?
        .queue(cursor::MoveTo(0, 0))?;
    stdout.queue(style::Print(out))?;
    stdout.flush()?;
    Ok(())
}

fn truncate(mut s: String, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s;
    }
    s = s.replace(['\n', '\r'], " ");
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i + 1 >= max_chars {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}
