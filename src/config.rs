use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub codex: CodexConfig,

    #[serde(default)]
    pub projects: ProjectsConfig,

    #[serde(default)]
    pub sessions: SessionsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfig {
    #[serde(default = "default_codex_bin")]
    pub bin: String,

    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectsConfig {
    /// Parent folders to scan one-level deep for git repos (directories containing `.git`).
    #[serde(default)]
    pub roots: Vec<PathBuf>,

    /// Explicit folders to show as targets.
    #[serde(default)]
    pub paths: Vec<PathBuf>,

    /// Also populate targets based on recent Codex sessions (from `sessions/` JSONL).
    #[serde(default = "default_projects_from_sessions")]
    pub from_sessions: bool,

    /// How many recent sessions to scan to infer targets.
    #[serde(default = "default_projects_sessions_limit")]
    pub sessions_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionsConfig {
    /// Codex home directory that contains `sessions/`.
    #[serde(default = "default_codex_home")]
    pub codex_home: PathBuf,

    /// Default number of sessions to show.
    #[serde(default = "default_sessions_limit")]
    pub limit: usize,
}

fn default_codex_bin() -> String {
    "codex".to_string()
}

fn default_codex_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".codex")
}

fn default_sessions_limit() -> usize {
    15
}

fn default_projects_from_sessions() -> bool {
    true
}

fn default_projects_sessions_limit() -> usize {
    200
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            bin: default_codex_bin(),
            args: Vec::new(),
        }
    }
}

impl Default for ProjectsConfig {
    fn default() -> Self {
        let mut roots = Vec::new();
        // Best-effort default: ~/Documents/Code (matches your workflow).
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Documents").join("Code"));
        }
        Self {
            roots,
            paths: Vec::new(),
            from_sessions: default_projects_from_sessions(),
            sessions_limit: default_projects_sessions_limit(),
        }
    }
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            codex_home: default_codex_home(),
            limit: default_sessions_limit(),
        }
    }
}

pub fn resolve_config_path(arg: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = arg {
        return Ok(p.to_path_buf());
    }
    let home = dirs::home_dir().context("failed to resolve home dir")?;
    Ok(home.join(".codex-launch").join("config.toml"))
}

impl Config {
    pub fn load_or_init(path: &Path) -> Result<Self> {
        if path.exists() {
            let s = fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let cfg: Config =
                toml::from_str(&s).with_context(|| format!("invalid TOML: {}", path.display()))?;
            return Ok(cfg);
        }

        let cfg = Config::default();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        cfg.save(path)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let s = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, s).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn add_root(&mut self, path: PathBuf) -> Result<()> {
        let p = normalize(path)?;
        if !p.exists() {
            anyhow::bail!("path does not exist: {}", p.display());
        }
        if !p.is_dir() {
            anyhow::bail!("not a directory: {}", p.display());
        }
        if !self.projects.roots.contains(&p) {
            self.projects.roots.push(p);
        }
        Ok(())
    }

    pub fn add_path(&mut self, path: PathBuf) -> Result<()> {
        let p = normalize(path)?;
        if !p.exists() {
            anyhow::bail!("path does not exist: {}", p.display());
        }
        if !p.is_dir() {
            anyhow::bail!("not a directory: {}", p.display());
        }
        if !self.projects.paths.contains(&p) {
            self.projects.paths.push(p);
        }
        Ok(())
    }

    pub fn remove_path_or_root(&mut self, path: PathBuf) -> Result<()> {
        let p = normalize(path)?;
        let before_roots = self.projects.roots.len();
        self.projects.roots.retain(|r| r != &p);
        let before_paths = self.projects.paths.len();
        self.projects.paths.retain(|r| r != &p);
        if before_roots == self.projects.roots.len() && before_paths == self.projects.paths.len() {
            anyhow::bail!("not found in config: {}", p.display());
        }
        Ok(())
    }

    pub fn is_scoped_target(&self, cwd: &Path) -> bool {
        let cwd = match normalize(cwd.to_path_buf()) {
            Ok(p) => p,
            Err(_) => cwd.to_path_buf(),
        };
        self.projects.paths.iter().any(|p| cwd.starts_with(p))
            || self.projects.roots.iter().any(|r| cwd.starts_with(r))
    }
}

pub fn normalize(p: PathBuf) -> Result<PathBuf> {
    let expanded = if let Some(s) = p.to_str()
        && s.starts_with("~/")
        && let Some(home) = dirs::home_dir()
    {
        home.join(s.trim_start_matches("~/"))
    } else if p.to_string_lossy() == "~" {
        dirs::home_dir().unwrap_or(p.clone())
    } else {
        p
    };
    Ok(expanded)
}
