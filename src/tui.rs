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
use crate::sessions::SessionItem;

#[derive(Debug, Clone)]
pub enum ProjectPick {
    New(ProjectTarget),
    Resume(SessionItem),
    OpenConfig,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Projects,
    SessionsScoped,
    SessionsAll,
}

#[derive(Debug, Clone)]
enum View {
    Tab(Tab),
    ProjectSessions {
        target: ProjectTarget,
        sessions: Vec<SessionItem>,
    },
}

pub fn pick_project(
    targets: &[ProjectTarget],
    sessions_scoped: &[SessionItem],
    sessions_all: &[SessionItem],
    per_project_limit: usize,
) -> Result<ProjectPick> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        pick_project_inner(
            &mut stdout,
            targets,
            sessions_scoped,
            sessions_all,
            per_project_limit,
        )
    }));
    match result {
        Ok(r) => r,
        Err(_) => anyhow::bail!(
            "UI crashed (panic). Terminal should be restored; re-run with `--no-ui` if needed."
        ),
    }
}

fn pick_project_inner(
    stdout: &mut io::Stdout,
    targets: &[ProjectTarget],
    sessions_scoped: &[SessionItem],
    sessions_all: &[SessionItem],
    per_project_limit: usize,
) -> Result<ProjectPick> {
    let matcher = SkimMatcherV2::default().ignore_case();

    let mut view = View::Tab(Tab::Projects);

    let mut project_filter = String::new();
    let mut project_cursor: usize = 0;

    let mut sessions_filter = String::new();
    let mut sessions_cursor: usize = 0;

    let mut project_sessions_filter = String::new();
    let mut project_sessions_cursor: usize = 0;

    loop {
        let (cols, rows) = terminal::size()?;
        let cols = cols as usize;
        let rows = rows as usize;

        match &mut view {
            View::Tab(Tab::Projects) => {
                let filtered = filter_targets(targets, &matcher, &project_filter);
                if project_cursor >= filtered.len() && !filtered.is_empty() {
                    project_cursor = filtered.len() - 1;
                }
                render_projects(
                    stdout,
                    targets,
                    &filtered,
                    project_cursor,
                    &project_filter,
                    cols,
                    rows,
                )?;
            }
            View::Tab(tab @ (Tab::SessionsScoped | Tab::SessionsAll)) => {
                let items = match tab {
                    Tab::SessionsScoped => sessions_scoped,
                    Tab::SessionsAll => sessions_all,
                    _ => unreachable!(),
                };
                let filtered = filter_sessions(items, &matcher, &sessions_filter);
                if sessions_cursor >= filtered.len() && !filtered.is_empty() {
                    sessions_cursor = filtered.len() - 1;
                }
                render_sessions(
                    stdout,
                    *tab,
                    items,
                    &filtered,
                    sessions_cursor,
                    &sessions_filter,
                    cols,
                    rows,
                )?;
            }
            View::ProjectSessions { target, sessions } => {
                let filtered = filter_sessions(sessions, &matcher, &project_sessions_filter);
                // Cursor includes "Start new session" at row 0, so the maximum valid
                // cursor position is `filtered.len()` (the last session row).
                if project_sessions_cursor > filtered.len() {
                    project_sessions_cursor = filtered.len();
                }
                render_project_sessions(
                    stdout,
                    target,
                    sessions,
                    &filtered,
                    project_sessions_cursor,
                    &project_sessions_filter,
                    cols,
                    rows,
                )?;
            }
        }

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let ev = event::read()?;
        if let Event::Key(k) = ev {
            // Global actions.
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) => match &view {
                    View::Tab(Tab::Projects) => return Ok(ProjectPick::Quit),
                    View::Tab(Tab::SessionsScoped) => {
                        view = View::Tab(Tab::Projects);
                        continue;
                    }
                    View::Tab(Tab::SessionsAll) => {
                        view = View::Tab(Tab::SessionsScoped);
                        continue;
                    }
                    View::ProjectSessions { .. } => {}
                },
                (KeyCode::Char('q'), KeyModifiers::NONE) => return Ok(ProjectPick::Quit),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(ProjectPick::Quit),
                (KeyCode::Char('o'), KeyModifiers::NONE) => return Ok(ProjectPick::OpenConfig),
                _ => {}
            }

            match &mut view {
                View::Tab(Tab::Projects) => {
                    let filtered = filter_targets(targets, &matcher, &project_filter);
                    if project_cursor >= filtered.len() && !filtered.is_empty() {
                        project_cursor = filtered.len() - 1;
                    }
                    match handle_list_key(
                        k,
                        &mut project_filter,
                        &mut project_cursor,
                        filtered.len(),
                        Tab::Projects,
                    )? {
                        ListOutcome::Continue => {}
                        ListOutcome::SwitchTab(tab) => {
                            sessions_filter.clear();
                            sessions_cursor = 0;
                            view = View::Tab(tab);
                        }
                        ListOutcome::Activate => {
                            if let Some(t) = selected_target(targets, &filtered, project_cursor) {
                                project_sessions_filter.clear();
                                project_sessions_cursor = 0;
                                let sessions =
                                    sessions_for_target(&t, sessions_all, per_project_limit);
                                view = View::ProjectSessions {
                                    target: t,
                                    sessions,
                                };
                            }
                        }
                        ListOutcome::StartNew => {
                            if let Some(t) = selected_target(targets, &filtered, project_cursor) {
                                return Ok(ProjectPick::New(t));
                            }
                        }
                    }
                }
                View::Tab(Tab::SessionsScoped) => {
                    let filtered = filter_sessions(sessions_scoped, &matcher, &sessions_filter);
                    if sessions_cursor >= filtered.len() && !filtered.is_empty() {
                        sessions_cursor = filtered.len() - 1;
                    }
                    match handle_list_key(
                        k,
                        &mut sessions_filter,
                        &mut sessions_cursor,
                        filtered.len(),
                        Tab::SessionsScoped,
                    )? {
                        ListOutcome::Continue => {}
                        ListOutcome::SwitchTab(tab) => {
                            view = View::Tab(tab);
                        }
                        ListOutcome::Activate => {
                            if let Some(s) =
                                selected_session(sessions_scoped, &filtered, sessions_cursor)
                            {
                                return Ok(ProjectPick::Resume(s));
                            }
                        }
                        ListOutcome::StartNew => {}
                    }
                }
                View::Tab(Tab::SessionsAll) => {
                    let filtered = filter_sessions(sessions_all, &matcher, &sessions_filter);
                    if sessions_cursor >= filtered.len() && !filtered.is_empty() {
                        sessions_cursor = filtered.len() - 1;
                    }
                    match handle_list_key(
                        k,
                        &mut sessions_filter,
                        &mut sessions_cursor,
                        filtered.len(),
                        Tab::SessionsAll,
                    )? {
                        ListOutcome::Continue => {}
                        ListOutcome::SwitchTab(tab) => {
                            view = View::Tab(tab);
                        }
                        ListOutcome::Activate => {
                            if let Some(s) =
                                selected_session(sessions_all, &filtered, sessions_cursor)
                            {
                                return Ok(ProjectPick::Resume(s));
                            }
                        }
                        ListOutcome::StartNew => {}
                    }
                }
                View::ProjectSessions { target, sessions } => {
                    match (k.code, k.modifiers) {
                        (KeyCode::Esc, _) => {
                            view = View::Tab(Tab::Projects);
                            continue;
                        }
                        (KeyCode::Left, _) => {
                            view = View::Tab(Tab::Projects);
                            continue;
                        }
                        _ => {}
                    }

                    let filtered = filter_sessions(sessions, &matcher, &project_sessions_filter);
                    if project_sessions_cursor > filtered.len() {
                        project_sessions_cursor = filtered.len();
                    }

                    match handle_project_sessions_key(
                        k,
                        &mut project_sessions_filter,
                        &mut project_sessions_cursor,
                        filtered.len(),
                    )? {
                        ProjectSessionsOutcome::Continue => {}
                        ProjectSessionsOutcome::StartNew => {
                            return Ok(ProjectPick::New(target.clone()));
                        }
                        ProjectSessionsOutcome::Resume { filtered_idx } => {
                            if let Some(s) = filtered
                                .get(filtered_idx)
                                .and_then(|idx| sessions.get(*idx))
                                .cloned()
                            {
                                return Ok(ProjectPick::Resume(s));
                            }
                        }
                    }
                }
            }
        }
    }
}

fn sessions_for_target(
    target: &ProjectTarget,
    sessions_all: &[SessionItem],
    limit: usize,
) -> Vec<SessionItem> {
    let repo_root = crate::sessions::git_root_for_path(&target.path);
    let target_is_repo_root = repo_root.as_ref().is_some_and(|r| r == &target.path);
    let mut out = Vec::new();
    for s in sessions_all.iter() {
        if out.len() >= limit {
            break;
        }
        if target_is_repo_root && let Some(rr) = repo_root.as_ref() {
            if crate::sessions::git_root_for_path(&s.cwd).is_some_and(|x| x == *rr) {
                out.push(s.clone());
            }
        } else if s.cwd.starts_with(&target.path) {
            out.push(s.clone());
        }
    }
    out
}

fn selected_target(
    targets: &[ProjectTarget],
    filtered: &[usize],
    cursor_idx: usize,
) -> Option<ProjectTarget> {
    filtered
        .get(cursor_idx)
        .and_then(|idx| targets.get(*idx))
        .cloned()
}

fn selected_session(
    items: &[SessionItem],
    filtered: &[usize],
    cursor_idx: usize,
) -> Option<SessionItem> {
    filtered
        .get(cursor_idx)
        .and_then(|idx| items.get(*idx))
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

fn filter_sessions(items: &[SessionItem], matcher: &SkimMatcherV2, filter: &str) -> Vec<usize> {
    let q = filter.trim();
    if q.is_empty() {
        return (0..items.len()).collect();
    }

    let mut scored = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let hay = format!(
                "{} {} {}",
                s.id,
                s.cwd.display(),
                s.summary.as_deref().unwrap_or_default()
            );
            matcher.fuzzy_match(&hay, q).map(|score| (score, i))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, i)| i).collect()
}

enum ListOutcome {
    Continue,
    SwitchTab(Tab),
    Activate,
    StartNew,
}

fn handle_list_key(
    key: KeyEvent,
    filter: &mut String,
    cursor_idx: &mut usize,
    len: usize,
    tab: Tab,
) -> Result<ListOutcome> {
    match (key.code, key.modifiers) {
        (KeyCode::Left, _) => {
            return Ok(ListOutcome::SwitchTab(match tab {
                Tab::Projects => Tab::Projects,
                Tab::SessionsScoped => Tab::Projects,
                Tab::SessionsAll => Tab::SessionsScoped,
            }));
        }
        (KeyCode::Right, _) => {
            return Ok(ListOutcome::SwitchTab(match tab {
                Tab::Projects => Tab::SessionsScoped,
                Tab::SessionsScoped => Tab::SessionsAll,
                Tab::SessionsAll => Tab::SessionsAll,
            }));
        }

        (KeyCode::Char('n'), KeyModifiers::NONE) if tab == Tab::Projects => {
            return Ok(ListOutcome::StartNew);
        }
        (KeyCode::Enter, _) => return Ok(ListOutcome::Activate),

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
    Ok(ListOutcome::Continue)
}

enum ProjectSessionsOutcome {
    Continue,
    StartNew,
    Resume { filtered_idx: usize },
}

fn handle_project_sessions_key(
    key: KeyEvent,
    filter: &mut String,
    cursor_idx: &mut usize,
    sessions_len: usize,
) -> Result<ProjectSessionsOutcome> {
    // Cursor includes the "Start new session" row at index 0.
    let len = sessions_len + 1;
    match (key.code, key.modifiers) {
        (KeyCode::Enter, _) => {
            if *cursor_idx == 0 {
                return Ok(ProjectSessionsOutcome::StartNew);
            }
            return Ok(ProjectSessionsOutcome::Resume {
                filtered_idx: cursor_idx.saturating_sub(1),
            });
        }

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
    Ok(ProjectSessionsOutcome::Continue)
}

fn render_projects(
    stdout: &mut io::Stdout,
    targets: &[ProjectTarget],
    filtered: &[usize],
    cursor_idx: usize,
    filter: &str,
    cols: usize,
    rows: usize,
) -> Result<()> {
    let mut out = String::new();

    out.push_str(&tabs_line(Tab::Projects));
    out.push('\n');
    let help = "⏎ sessions · n new · ←/→ tabs · o config · q quit";
    out.push_str(&format!("{}\n", truncate(help.to_string(), cols).dim()));
    out.push_str(&format!("{} {}\n", "Filter:".bold(), filter));

    let list_rows = rows.saturating_sub(5).max(1);
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

    draw(stdout, out)
}

fn render_sessions(
    stdout: &mut io::Stdout,
    tab: Tab,
    items: &[SessionItem],
    filtered: &[usize],
    cursor_idx: usize,
    filter: &str,
    cols: usize,
    rows: usize,
) -> Result<()> {
    let mut out = String::new();

    out.push_str(&tabs_line(tab));
    out.push('\n');
    let help = match tab {
        Tab::SessionsScoped => "⏎ resume · esc back · ←/→ tabs · o config · q quit",
        Tab::SessionsAll => "⏎ resume · esc back · ← tabs · o config · q quit",
        _ => "⏎ resume · esc back · o config · q quit",
    };
    out.push_str(&format!("{}\n", truncate(help.to_string(), cols).dim()));
    out.push_str(&format!("{} {}\n", "Filter:".bold(), filter));

    let list_rows = rows.saturating_sub(5).max(1);
    let start = cursor_idx.saturating_sub(list_rows / 2);
    let end = (start + list_rows).min(filtered.len());

    for (row_offset, idx) in filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
    {
        let s = &items[*idx];
        let mut line = s.to_string();
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
        format!("{} / {}", filtered.len(), items.len()).dim()
    ));

    draw(stdout, out)
}

fn render_project_sessions(
    stdout: &mut io::Stdout,
    target: &ProjectTarget,
    sessions: &[SessionItem],
    filtered: &[usize],
    cursor_idx: usize,
    filter: &str,
    cols: usize,
    rows: usize,
) -> Result<()> {
    let mut out = String::new();

    out.push_str(&format!(
        "{}  {}\n",
        "Project:".bold(),
        truncate(target.label.clone(), cols.saturating_sub(10))
    ));
    let help = "⏎ select · esc back · o config · q quit";
    out.push_str(&format!("{}\n", truncate(help.to_string(), cols).dim()));
    out.push_str(&format!("{} {}\n", "Filter:".bold(), filter));

    // Cursor includes "Start new session" at row 0.
    let mut lines: Vec<String> = Vec::new();
    lines.push("Start new session".to_string());
    for idx in filtered.iter() {
        if let Some(s) = sessions.get(*idx) {
            lines.push(session_line_no_path(s));
        }
    }

    if cursor_idx >= lines.len() && !lines.is_empty() {
        // Keep cursor valid if filter shrinks.
        // (Callers ensure this, but keep it safe for rendering.)
    }

    let list_rows = rows.saturating_sub(5).max(1);
    let start = cursor_idx.saturating_sub(list_rows / 2);
    let end = (start + list_rows).min(lines.len());

    for (row_offset, line) in lines
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
    {
        let mut line = line.clone();
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
        format!("{} sessions", sessions.len()).dim()
    ));

    draw(stdout, out)
}

fn draw(stdout: &mut io::Stdout, out: String) -> Result<()> {
    stdout
        .queue(terminal::Clear(ClearType::All))?
        .queue(cursor::MoveTo(0, 0))?;
    // In raw mode some terminals don't translate '\n' to CRLF; use explicit CRLF.
    stdout.queue(style::Print(out.replace('\n', "\r\n")))?;
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

fn tabs_line(active: Tab) -> String {
    let mut parts = Vec::new();
    parts.push(tab_label("Projects", active == Tab::Projects));
    parts.push(tab_label(
        "Sessions (scoped)",
        active == Tab::SessionsScoped,
    ));
    parts.push(tab_label("Sessions (all)", active == Tab::SessionsAll));
    format!("{}  {}", "codex-launch".bold(), parts.join("  "))
}

fn tab_label(label: &str, active: bool) -> String {
    if active {
        format!(" {} ", label).reverse().to_string()
    } else {
        format!(" {} ", label).dim().to_string()
    }
}

fn session_line_no_path(s: &SessionItem) -> String {
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
