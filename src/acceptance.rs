use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::paths::find_project_root;
use crate::state::{load_config, log_event};

pub fn run_acceptance_gate(root: Option<&Path>) -> Result<(bool, String)> {
    let config = load_config(root)?;
    if !config.acceptance_gate.enabled || config.acceptance_gate.command.trim().is_empty() {
        return Ok((true, String::new()));
    }

    let cmd = config.acceptance_gate.command.trim();
    let project = find_project_root(root);
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&project)
        .output()?;

    if output.status.success() {
        log_event(
            root,
            "acceptance_gate_pass",
            serde_json::json!({"command": cmd}),
        )?;
        return Ok((true, String::new()));
    }

    let mut detail = String::from_utf8_lossy(&output.stderr).to_string();
    if detail.trim().is_empty() {
        detail = String::from_utf8_lossy(&output.stdout).to_string();
    }
    if detail.trim().is_empty() {
        detail = format!("exit code {}", output.status.code().unwrap_or(-1));
    }
    let detail_short: String = detail.chars().take(500).collect();
    log_event(
        root,
        "acceptance_gate_fail",
        serde_json::json!({"command": cmd, "detail": detail_short}),
    )?;
    Ok((
        false,
        format!("RepoVow acceptance gate failed (`{cmd}`): {detail_short}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{save_config, RepoVowConfig};

    #[test]
    fn gate_passes_on_true_command() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        let mut cfg = RepoVowConfig::default();
        cfg.acceptance_gate.enabled = true;
        cfg.acceptance_gate.command = "true".into();
        save_config(&cfg, Some(root)).unwrap();

        let (ok, _) = run_acceptance_gate(Some(root)).unwrap();
        assert!(ok);
    }

    #[test]
    fn gate_fails_on_false_command() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        let mut cfg = RepoVowConfig::default();
        cfg.acceptance_gate.enabled = true;
        cfg.acceptance_gate.command = "false".into();
        save_config(&cfg, Some(root)).unwrap();

        let (ok, msg) = run_acceptance_gate(Some(root)).unwrap();
        assert!(!ok);
        assert!(msg.contains("acceptance gate failed"));
    }
}
