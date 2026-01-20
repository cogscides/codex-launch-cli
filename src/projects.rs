use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::pathfmt;
use crate::sessions;
use crate::timefmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TargetKind {
    RootChildGitRepo,
    ExplicitPath,
    SessionHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTarget {
    pub path: PathBuf,
    pub kind: TargetKind,
    pub label: String,
    pub last_session_at: Option<String>,
    pub last_session_summary: Option<String>,
}

impl fmt::Display for ProjectTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = pathfmt::compact_path(&self.path, 52);
        let last = self
            .last_session_at
            .as_deref()
            .and_then(timefmt::parse_rfc3339)
            .map(|dt| format!("{} {}", timefmt::format_age(dt), timefmt::format_short(dt)))
            .unwrap_or_else(|| "-".to_string());
        let summary = self
            .last_session_summary
            .as_deref()
            .map(|s| truncate_one_line(s, 64))
            .unwrap_or_default();

        if summary.is_empty() {
            write!(f, "{:<22}  {:<52}  {}", self.label, path, last)
        } else {
            write!(f, "{:<22}  {:<52}  {}  {}", self.label, path, last, summary)
        }
    }
}

pub fn gather_targets(cfg: &Config) -> Result<Vec<ProjectTarget>> {
    let mut map: BTreeMap<PathBuf, ProjectTarget> = BTreeMap::new();

    for p in cfg.projects.paths.iter() {
        let p = p.clone();
        if !p.exists() {
            continue;
        }
        if !p.is_dir() {
            continue;
        }
        let label = display_name(&p);
        map.entry(p.clone()).or_insert(ProjectTarget {
            path: p,
            kind: TargetKind::ExplicitPath,
            label,
            last_session_at: None,
            last_session_summary: None,
        });
    }

    for root in cfg.projects.roots.iter() {
        if !root.exists() || !root.is_dir() {
            continue;
        }
        let entries =
            fs::read_dir(root).with_context(|| format!("failed to read dir {}", root.display()))?;
        for ent in entries.flatten() {
            let path = ent.path();
            let Ok(ft) = ent.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            if is_hidden_or_noise(&path) {
                continue;
            }
            if !is_git_repo_root(&path) {
                continue;
            }
            let label = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| display_name(&path));
            map.entry(path.clone()).or_insert(ProjectTarget {
                path,
                kind: TargetKind::RootChildGitRepo,
                label,
                last_session_at: None,
                last_session_summary: None,
            });
        }
    }

    if cfg.projects.from_sessions {
        let sessions = sessions::list_recent_sessions(
            cfg,
            sessions::SessionQuery::All {
                limit: cfg.projects.sessions_limit,
            },
        )?;
        for s in sessions {
            let inferred = infer_target_path_from_session_cwd(&s.cwd);
            if inferred.is_none() {
                continue;
            }
            let inferred = inferred.unwrap();
            let label = display_name(&inferred);

            match map.get_mut(&inferred) {
                Some(existing) => {
                    // Only upgrade metadata if this session is newer than what we already have.
                    let replace = match (&existing.last_session_at, &s.created_at) {
                        (None, Some(_)) => true,
                        (Some(a), Some(b)) => b > a,
                        (None, None) => existing.last_session_summary.is_none(),
                        (Some(_), None) => false,
                    };
                    if replace {
                        existing.last_session_at = s.created_at.clone();
                        existing.last_session_summary = s.summary.clone();
                    }
                }
                None => {
                    map.insert(
                        inferred.clone(),
                        ProjectTarget {
                            path: inferred,
                            kind: TargetKind::SessionHistory,
                            label,
                            last_session_at: s.created_at.clone(),
                            last_session_summary: s.summary.clone(),
                        },
                    );
                }
            }
        }
    }

    let mut items: Vec<ProjectTarget> = map.into_values().collect();
    // Prefer targets you used recently, then alphabetical.
    items.sort_by(|a, b| match (&a.last_session_at, &b.last_session_at) {
        (Some(ta), Some(tb)) if ta != tb => tb.cmp(ta),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
    });
    Ok(items)
}

fn is_git_repo_root(p: &Path) -> bool {
    let dotgit = p.join(".git");
    dotgit.is_dir() || dotgit.is_file()
}

fn infer_target_path_from_session_cwd(cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_dir() {
        return None;
    }
    // Only infer "project targets" from sessions when we can resolve a git repo root.
    // Non-git folders can still be added explicitly via `add-path`.
    find_git_root(cwd)
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    for _ in 0..25 {
        if is_git_repo_root(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

fn is_hidden_or_noise(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if name.starts_with('.') {
        return true;
    }
    matches!(name, "node_modules" | "target" | "dist" | "build")
}

fn display_name(p: &Path) -> String {
    pathfmt::basename(p)
}

fn truncate_one_line(s: &str, max_chars: usize) -> String {
    let s = s.replace('\t', " ");
    let first_line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let first_line = first_line.trim();
    if first_line.chars().count() <= max_chars {
        return first_line.to_string();
    }
    let mut out = String::new();
    for (i, c) in first_line.chars().enumerate() {
        if i + 1 >= max_chars {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}
