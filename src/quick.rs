use anyhow::Result;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::config::Config;
use crate::projects::{self, ProjectTarget};
use crate::sessions::{self, SessionItem};
use crate::ui;

pub fn launch_by_query(cfg: &Config, query: &str, dry_run: bool) -> Result<()> {
    let query = query.trim();
    if query.is_empty() {
        anyhow::bail!("empty project query");
    }

    let targets = projects::gather_targets(cfg)?;
    let matcher = SkimMatcherV2::default().ignore_case();
    let mut scored = targets
        .into_iter()
        .filter_map(|t| {
            let hay = format!("{} {}", t.label, t.path.display());
            matcher.fuzzy_match(&hay, query).map(|score| (score, t))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.is_empty() {
        anyhow::bail!("no project matches for: {query}");
    }

    let chosen = choose_target(scored)?;
    crate::run_codex_new(cfg, &chosen, dry_run)
}

pub fn resume_by_query(cfg: &Config, query: &str, dry_run: bool) -> Result<()> {
    let query = query.trim();
    if query.is_empty() {
        anyhow::bail!("empty resume query");
    }

    let items = sessions::list_recent_sessions(
        cfg,
        sessions::SessionQuery::All {
            limit: cfg.projects.sessions_limit.max(cfg.sessions.limit),
        },
    )?;

    let matcher = SkimMatcherV2::default().ignore_case();
    let mut scored = items
        .into_iter()
        .filter_map(|s| {
            let hay = format!(
                "{} {} {}",
                s.id,
                s.cwd.display(),
                s.summary.as_deref().unwrap_or("")
            );
            matcher.fuzzy_match(&hay, query).map(|score| (score, s))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.is_empty() {
        anyhow::bail!("no session matches for: {query}");
    }

    let chosen = choose_session(scored)?;
    crate::run_codex_resume(cfg, &chosen, dry_run)
}

fn choose_target(mut scored: Vec<(i64, ProjectTarget)>) -> Result<ProjectTarget> {
    if scored.len() == 1 {
        return Ok(scored.remove(0).1);
    }
    let (top_score, top) = scored[0].clone();
    let (second_score, _) = scored[1].clone();
    if top_score >= second_score + 25 {
        return Ok(top);
    }
    let options = scored
        .into_iter()
        .take(12)
        .map(|(_, t)| t)
        .collect::<Vec<_>>();
    ui::pick_target(&options)
}

fn choose_session(mut scored: Vec<(i64, SessionItem)>) -> Result<SessionItem> {
    if scored.len() == 1 {
        return Ok(scored.remove(0).1);
    }
    let (top_score, top) = scored[0].clone();
    let (second_score, _) = scored[1].clone();
    if top_score >= second_score + 25 {
        return Ok(top);
    }
    let options = scored
        .into_iter()
        .take(20)
        .map(|(_, s)| s)
        .collect::<Vec<_>>();
    ui::pick_session(&options)
}
