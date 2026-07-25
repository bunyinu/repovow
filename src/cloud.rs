use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::paths::repovow_dir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    pub url: String,
    pub project_id: String,
    pub api_key: String,
}

pub fn cloud_config_path(root: Option<&Path>) -> std::path::PathBuf {
    repovow_dir(root).join("cloud.json")
}

pub fn load_cloud_config(root: Option<&Path>) -> Result<Option<CloudConfig>> {
    let path = cloud_config_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

pub fn save_cloud_config(config: &CloudConfig, root: Option<&Path>) -> Result<()> {
    let path = cloud_config_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config)? + "\n")?;
    Ok(())
}

pub fn push_state(root: Option<&Path>) -> Result<()> {
    let Some(config) = load_cloud_config(root)? else {
        return Ok(());
    };
    let repovow = repovow_dir(root);
    let state_path = repovow.join(crate::paths::STATE_FILE);
    let snapshot_path = repovow.join(crate::paths::SNAPSHOT_FILE);
    let state_json = std::fs::read_to_string(&state_path).unwrap_or_else(|_| "{}".into());
    let snapshot_md = std::fs::read_to_string(&snapshot_path).unwrap_or_default();
    let local_config = crate::state::load_config(root).unwrap_or_default();
    let config_val = serde_json::to_value(&local_config)?;
    let changelog_md =
        std::fs::read_to_string(repovow.join(crate::paths::CHANGELOG_FILE)).unwrap_or_default();
    let changelog_tail = tail_text(&changelog_md, 80);
    let policy = crate::policy::policy_status_json(root);

    let url = format!(
        "{}/api/projects/{}/sync",
        config.url.trim_end_matches('/'),
        config.project_id
    );
    let body = serde_json::json!({
        "state": serde_json::from_str::<serde_json::Value>(&state_json).unwrap_or(serde_json::json!({})),
        "snapshot": snapshot_md,
        "config": config_val,
        "changelog": changelog_tail,
        "policy": policy,
    });

    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .send_json(body)?;

    if resp.status() >= 400 {
        anyhow::bail!("cloud sync failed: HTTP {}", resp.status());
    }
    Ok(())
}

fn tail_text(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

pub fn pull_state(root: Option<&Path>) -> Result<()> {
    let Some(config) = load_cloud_config(root)? else {
        return Ok(());
    };
    let url = format!(
        "{}/api/projects/{}",
        config.url.trim_end_matches('/'),
        config.project_id
    );
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .call()
        .context("cloud pull request")?;

    if resp.status() >= 400 {
        anyhow::bail!("cloud pull failed: HTTP {}", resp.status());
    }

    let body: serde_json::Value = resp.into_json()?;
    let repovow = repovow_dir(root);
    std::fs::create_dir_all(&repovow)?;

    let local_before = crate::state::load_state(root).ok();

    if let Some(state) = body.get("state") {
        crate::paths::write_json_atomic(&repovow.join(crate::paths::STATE_FILE), state)?;
        if let Some(ref before) = local_before {
            let _ = crate::policy::protect_goal_after_pull(root, before);
        }
    }
    // Regenerate snapshot from verified local state (ignore cloud snapshot body).
    let _ = crate::snapshot::write_snapshot(root);
    Ok(())
}
