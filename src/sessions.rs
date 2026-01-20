use std::cmp::Reverse;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::Config;
use crate::pathfmt;
use crate::timefmt;

#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    pub created_at: Option<String>,
    pub cwd: PathBuf,
    pub summary: Option<String>,
    pub cli_version: Option<String>,
    pub model_provider: Option<String>,
    pub source: Option<String>,
    pub path: PathBuf,
}

impl fmt::Display for SessionItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id_short = self.id.chars().take(8).collect::<String>();
        let when = self
            .created_at
            .as_deref()
            .and_then(timefmt::parse_rfc3339)
            .map(|dt| format!("{} {}", timefmt::format_age(dt), timefmt::format_short(dt)))
            .unwrap_or_else(|| "-".to_string());

        let cwd = pathfmt::compact_path(&self.cwd, 56);

        let summary = self
            .summary
            .as_deref()
            .map(|s| truncate_one_line(s, 90))
            .unwrap_or_default();

        let mut meta = Vec::new();
        if let Some(p) = self.model_provider.as_deref() {
            meta.push(p);
        }
        if let Some(s) = self.source.as_deref() {
            meta.push(s);
        }
        if let Some(v) = self.cli_version.as_deref() {
            meta.push(v);
        }
        if let Some(name) = self.path.file_name().and_then(|s| s.to_str()) {
            meta.push(name);
        }
        let meta = if meta.is_empty() {
            String::new()
        } else {
            format!("  [{}]", meta.join(" "))
        };

        if summary.is_empty() {
            write!(f, "{:<12}  {:<8}  {}{}", when, id_short, cwd, meta)
        } else {
            write!(
                f,
                "{:<12}  {:<8}  {}  {}{}",
                when, id_short, cwd, summary, meta
            )
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionQuery {
    All { limit: usize },
    Scoped { limit: usize },
    ForCwd { cwd: PathBuf, limit: usize },
    ForRepoRoot { repo_root: PathBuf, limit: usize },
}

pub fn list_recent_sessions(cfg: &Config, query: SessionQuery) -> Result<Vec<SessionItem>> {
    let (limit, filter) = match query {
        SessionQuery::All { limit } => (limit, Filter::All),
        SessionQuery::Scoped { limit } => (limit, Filter::Scoped),
        SessionQuery::ForCwd { cwd, limit } => (limit, Filter::ForCwd(cwd)),
        SessionQuery::ForRepoRoot { repo_root, limit } => (limit, Filter::ForRepoRoot(repo_root)),
    };

    let sessions_root = cfg.sessions.codex_home.join("sessions");
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();

    for year_path in collect_dirs_desc(&sessions_root)? {
        for month_path in collect_dirs_desc(&year_path)? {
            for day_path in collect_dirs_desc(&month_path)? {
                for p in collect_rollout_files_desc(&day_path)? {
                    if items.len() >= limit {
                        return Ok(items);
                    }
                    let Some(session) = read_session_meta(&p).ok().flatten() else {
                        continue;
                    };
                    if !matches_filter(cfg, &filter, &session.cwd) {
                        continue;
                    }
                    items.push(session);
                }
            }
        }
    }

    Ok(items)
}

pub fn find_session_by_id(cfg: &Config, id: &str) -> Result<Option<SessionItem>> {
    let sessions_root = cfg.sessions.codex_home.join("sessions");
    if !sessions_root.exists() {
        return Ok(None);
    }
    for year_path in collect_dirs_desc(&sessions_root)? {
        for month_path in collect_dirs_desc(&year_path)? {
            for day_path in collect_dirs_desc(&month_path)? {
                for p in collect_rollout_files_desc(&day_path)? {
                    let Some(session) = read_session_meta(&p).ok().flatten() else {
                        continue;
                    };
                    if session.id == id {
                        return Ok(Some(session));
                    }
                }
            }
        }
    }
    Ok(None)
}

enum Filter {
    All,
    Scoped,
    ForCwd(PathBuf),
    ForRepoRoot(PathBuf),
}

fn matches_filter(cfg: &Config, filter: &Filter, cwd: &Path) -> bool {
    match filter {
        Filter::All => true,
        Filter::Scoped => cfg.is_scoped_target(cwd),
        Filter::ForCwd(root) => cwd.starts_with(root),
        Filter::ForRepoRoot(repo_root) => find_git_root(cwd).is_some_and(|r| r == *repo_root),
    }
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    for _ in 0..25 {
        let dotgit = cur.join(".git");
        if dotgit.is_dir() || dotgit.is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

pub fn git_root_for_path(start: &Path) -> Option<PathBuf> {
    find_git_root(start)
}

fn collect_dirs_desc(parent: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for ent in
        fs::read_dir(parent).with_context(|| format!("failed to read {}", parent.display()))?
    {
        let Ok(ent) = ent else { continue };
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.len() != 2 && name.len() != 4 {
            continue;
        }
        if name.chars().all(|c| c.is_ascii_digit()) {
            dirs.push(path);
        }
    }
    dirs.sort_by_key(|p| {
        Reverse(
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        )
    });
    Ok(dirs)
}

fn collect_rollout_files_desc(day_path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for ent in
        fs::read_dir(day_path).with_context(|| format!("failed to read {}", day_path.display()))?
    {
        let Ok(ent) = ent else { continue };
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        files.push(path);
    }
    files.sort_by_key(|p| {
        Reverse(
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        )
    });
    Ok(files)
}

fn read_session_meta(path: &Path) -> Result<Option<SessionItem>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut created_at: Option<String> = None;
    let mut id: Option<String> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut cli_version: Option<String> = None;
    let mut model_provider: Option<String> = None;
    let mut source: Option<String> = None;
    let mut first_user_text: Option<String> = None;
    let mut best_user_text: Option<String> = None;

    for line_result in reader.lines().take(300) {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("session_meta") => {
                created_at = created_at.or_else(|| {
                    v.get("timestamp")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                });
                let Some(payload) = v.get("payload") else {
                    continue;
                };
                id = payload
                    .get("id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                cwd = payload
                    .get("cwd")
                    .and_then(|x| x.as_str())
                    .map(PathBuf::from);
                cli_version = payload
                    .get("cli_version")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                model_provider = payload
                    .get("model_provider")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                source = payload
                    .get("source")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
            }
            Some("response_item") => {
                let Some(payload) = v.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(|x| x.as_str()) != Some("message") {
                    continue;
                }
                if payload.get("role").and_then(|x| x.as_str()) != Some("user") {
                    continue;
                }
                let Some(text) = extract_text_from_message_payload(payload) else {
                    continue;
                };
                if first_user_text.is_none() {
                    first_user_text = Some(text.clone());
                }
                if !looks_like_boilerplate(&text) && best_user_text.is_none() {
                    best_user_text = Some(text);
                }
            }
            _ => {}
        }

        if id.is_some() && cwd.is_some() && best_user_text.is_some() {
            break;
        }
    }

    let (Some(id), Some(cwd)) = (id, cwd) else {
        return Ok(None);
    };
    let summary = best_user_text.map(normalize_summary);
    Ok(Some(SessionItem {
        id,
        created_at,
        cwd,
        summary,
        cli_version,
        model_provider,
        source,
        path: path.to_path_buf(),
    }))
}

fn extract_text_from_message_payload(payload: &Value) -> Option<String> {
    let content = payload.get("content")?.as_array()?;
    for item in content {
        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
            let t = text.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn looks_like_boilerplate(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("# AGENTS.md instructions")
        || t.starts_with("<environment_context>")
        || t.starts_with("<user_shell_command>")
        || t.contains("<INSTRUCTIONS>")
}

fn normalize_summary(s: String) -> String {
    let s = s.replace('\t', " ");
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let first_line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    first_line.trim().to_string()
}

fn truncate_one_line(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_meta_line() {
        let line = r#"{"timestamp":"2026-01-19T15:21:26.203Z","type":"session_meta","payload":{"id":"019bd6d8-b99b-7eb1-847c-87c3da10673a","timestamp":"2026-01-19T15:21:26.171Z","cwd":"/Users/ivan/Documents/Obsidian/Ethea42","originator":"codex_cli_rs","cli_version":"0.88.0-alpha.4","source":"cli","model_provider":"openai"}}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["type"].as_str().unwrap(), "session_meta");
        let payload = &v["payload"];
        assert_eq!(
            payload["id"].as_str().unwrap(),
            "019bd6d8-b99b-7eb1-847c-87c3da10673a"
        );
        assert_eq!(
            payload["cwd"].as_str().unwrap(),
            "/Users/ivan/Documents/Obsidian/Ethea42"
        );
    }

    #[test]
    fn extracts_user_message_text() {
        let line = r#"{"timestamp":"2026-01-19T21:55:21.488Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello there\n\nmore"}]}}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        let payload = v.get("payload").unwrap();
        let text = extract_text_from_message_payload(payload).unwrap();
        assert_eq!(normalize_summary(text), "hello there");
    }
}
