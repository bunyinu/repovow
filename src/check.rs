use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::acceptance::run_acceptance_gate;
use crate::cloud::load_cloud_config;
use crate::paths::{find_project_root, repovow_dir};
use crate::policy;
use crate::state::{load_config, load_state, PolicyMode};

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    /// Fail if no active goal (default: true — use in CI after agents claim done).
    pub require_goal: bool,
    /// Ping RepoVow Cloud when `.repovow/cloud.json` exists.
    pub verify_cloud: bool,
}

pub fn run_check(opts: CheckOptions, root: Option<&Path>) -> Result<()> {
    let project = find_project_root(root);
    if !repovow_dir(root).join("config.json").exists() {
        bail!(
            "RepoVow not initialized — run `repovow init` in {}",
            project.display()
        );
    }

    let state = load_state(root)?;
    if opts.require_goal && state.goal.is_none() {
        bail!("No active goal — run `repovow goal set \"...\"` or `repovow tui`");
    }

    let config = load_config(root)?;
    if config.acceptance_gate.enabled && !config.acceptance_gate.command.trim().is_empty() {
        let (ok, reason) = run_acceptance_gate(root)?;
        if !ok {
            bail!("{reason}");
        }
    }

    if opts.verify_cloud {
        verify_cloud_link(root)?;
    }

    if config.policy.mode == PolicyMode::Required {
        let status = policy::verify_policy(root)?;
        if !status.is_ok() {
            bail!(
                "Policy check failed ({}) — {}",
                status.label(),
                status.detail()
            );
        }
    }

    Ok(())
}

fn verify_cloud_link(root: Option<&Path>) -> Result<()> {
    let Some(config) = load_cloud_config(root)? else {
        bail!("Cloud verify requested but no `.repovow/cloud.json` — run `repovow cloud link`");
    };
    let url = format!(
        "{}/api/projects/{}",
        config.url.trim_end_matches('/'),
        config.project_id
    );
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .call()
        .context("RepoVow Cloud unreachable")?;
    if resp.status() >= 400 {
        bail!("RepoVow Cloud project check failed: HTTP {}", resp.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{save_config, save_state, Goal, Progress, RepoVowConfig, RepoVowState};

    #[test]
    fn check_requires_init() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("empty");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let err = run_check(CheckOptions::default(), Some(&root)).unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }

    #[test]
    fn check_requires_goal_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        save_config(&RepoVowConfig::default(), Some(root)).unwrap();
        save_state(&mut RepoVowState::default(), Some(root)).unwrap();

        let err = run_check(
            CheckOptions {
                require_goal: true,
                verify_cloud: false,
            },
            Some(root),
        )
        .unwrap_err();
        assert!(err.to_string().contains("No active goal"));
    }

    #[test]
    fn check_passes_with_goal_and_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        let mut cfg = RepoVowConfig::default();
        cfg.acceptance_gate.enabled = true;
        cfg.acceptance_gate.command = "true".into();
        save_config(&cfg, Some(root)).unwrap();
        let mut state = RepoVowState {
            goal: Some(Goal {
                title: "Ship".into(),
                acceptance: vec![],
                constraints: vec![],
                started_at: "2026-01-01T00:00:00Z".into(),
            }),
            progress: Progress::default(),
            ..RepoVowState::default()
        };
        save_state(&mut state, Some(root)).unwrap();

        run_check(
            CheckOptions {
                require_goal: true,
                verify_cloud: false,
            },
            Some(root),
        )
        .unwrap();
    }
}
