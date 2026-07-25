use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

use crate::cloud::push_state;
use crate::paths::{ensure_repovow_dir, utcnow};
use crate::policy;
use crate::snapshot::write_snapshot;
use crate::state::{load_state, log_event, save_state, Goal, RepoVowState};

/// Shared goal form used by CLI, TUI, and web API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalForm {
    pub title: String,
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

impl GoalForm {
    pub fn from_state(state: &RepoVowState) -> Self {
        let goal = state.goal.as_ref();
        Self {
            title: goal.map(|g| g.title.clone()).unwrap_or_default(),
            step: state.progress.current_step.clone().unwrap_or_default(),
            acceptance: goal.map(|g| g.acceptance.clone()).unwrap_or_default(),
            constraints: goal.map(|g| g.constraints.clone()).unwrap_or_default(),
        }
    }

    pub fn parse_lines(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

pub fn apply_form(state: &mut RepoVowState, form: &GoalForm) {
    let title = form.title.trim();
    if title.is_empty() {
        state.goal = None;
    } else {
        let started = state
            .goal
            .as_ref()
            .map(|g| g.started_at.clone())
            .unwrap_or_else(utcnow);
        state.goal = Some(Goal {
            title: title.to_string(),
            acceptance: form.acceptance.clone(),
            constraints: form.constraints.clone(),
            started_at: started,
        });
    }
    let step = form.step.trim();
    state.progress.current_step = if step.is_empty() {
        None
    } else {
        Some(step.to_string())
    };
}

/// Save goal from any UI (CLI / TUI / web) and sync snapshot + cloud.
pub fn save_goal(form: &GoalForm, root: Option<&Path>, source: &str) -> Result<()> {
    ensure_repovow_dir(root)?;
    let title = form.title.trim();
    if title.is_empty() {
        anyhow::bail!("goal title is required");
    }

    let mut state = load_state(root)?;
    apply_form(&mut state, form);
    save_state(&mut state, root)?;
    policy::after_goal_change(root)?;
    write_snapshot(root)?;
    let _ = push_state(root);
    log_event(root, "goal_set", json!({"title": title, "source": source}))?;
    Ok(())
}

pub fn load_form(root: Option<&Path>) -> Result<GoalForm> {
    ensure_repovow_dir(root).context("run `repovow init` first")?;
    Ok(GoalForm::from_state(&load_state(root)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RepoVowState;

    #[test]
    fn apply_form_sets_goal_and_step() {
        let mut state = RepoVowState::default();
        let form = GoalForm {
            title: "Ship feature".into(),
            step: "write tests".into(),
            acceptance: vec!["CI green".into()],
            constraints: vec!["no new deps".into()],
        };
        apply_form(&mut state, &form);
        assert_eq!(state.goal.as_ref().unwrap().title, "Ship feature");
        assert_eq!(state.progress.current_step.as_deref(), Some("write tests"));
    }
}
