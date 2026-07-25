use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::paths::{read_jsonl_tail, repovow_dir, ATTEMPTS_FILE};
use crate::state::{load_config, log_attempt};

fn collapse_ws(s: &str) -> String {
    Regex::new(r"\s+")
        .unwrap()
        .replace_all(s.trim(), " ")
        .to_string()
}

pub fn normalize_action(tool: &str, tool_input: &Value) -> String {
    let tool = crate::hooks::normalize_tool_name(tool);
    if tool == "Bash" {
        if let Some(cmd) = tool_input.get("command").and_then(|c| c.as_str()) {
            let cmd = collapse_ws(cmd);
            let short: String = cmd.chars().take(500).collect();
            return format!("bash:{short}");
        }
    }
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(p) = tool_input.get(key).and_then(|v| v.as_str()) {
            return format!("{}:{p}", tool.to_lowercase());
        }
    }
    if let Some(cmd) = tool_input.get("command").and_then(|c| c.as_str()) {
        let short: String = cmd.chars().take(500).collect();
        return format!("{}:{short}", tool.to_lowercase());
    }
    let hash = Sha256::digest(tool_input.to_string().as_bytes());
    let hex: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{}:{hex}", tool.to_lowercase())
}

pub fn should_block_retry(root: Option<&Path>, tool: &str, action: &str) -> Result<(bool, String)> {
    let config = load_config(root)?;
    let max_fail = config.loop_breaker.max_same_failure;
    let window = config.loop_breaker.window_minutes;
    let cutoff = Utc::now() - Duration::minutes(window as i64);

    let attempts = read_jsonl_tail(&repovow_dir(root).join(ATTEMPTS_FILE), 500)?;
    let failures: Vec<&Value> = attempts
        .iter()
        .filter(|a| {
            a["ok"] == false
                && a["tool"].as_str() == Some(tool)
                && a["action"].as_str() == Some(action)
                && parse_ts(a["at"].as_str().unwrap_or(""))
                    .map(|t| t >= cutoff)
                    .unwrap_or(false)
        })
        .collect();

    if failures.len() < max_fail as usize {
        return Ok((false, String::new()));
    }

    let last = failures.last().unwrap();
    let detail = last["detail"].as_str().unwrap_or("");
    let detail_short: String = detail.chars().take(300).collect();
    let action_short: String = action.chars().take(100).collect();
    let reason = format!(
        "RepoVow loop breaker: `{action_short}` already failed {} times in the last {window}m. \
         Read failures with `repovow context --section failures` and try a different approach. Last error: {detail_short}",
        failures.len()
    );
    Ok((true, reason))
}

fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn check_pre_tool(
    root: Option<&Path>,
    _agent: &str,
    tool: &str,
    tool_input: &Value,
) -> Result<(bool, String)> {
    let action = normalize_action(tool, tool_input);
    should_block_retry(root, tool, &action)
}

/// v0.2: richer failure detection from Codex/Claude PostToolUse payloads.
pub fn detect_tool_failure(payload: &Value) -> (bool, String, Option<i64>) {
    if let Some(err) = payload.get("error").and_then(|e| e.as_str()) {
        if !err.is_empty() {
            return (false, err.to_string(), None);
        }
    }

    if let Some(result) = payload.get("tool_result").or(payload.get("result")) {
        if result.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
            let content = result
                .get("content")
                .map(|c| c.to_string())
                .unwrap_or_default();
            return (false, content, None);
        }
    }

    for key in ["stderr", "output"] {
        if let Some(val) = payload.get(key).and_then(|v| v.as_str()) {
            let lower = val.to_lowercase();
            if lower.contains("error")
                || lower.contains("failed")
                || lower.contains("command not found")
                || lower.contains("permission denied")
            {
                let code = payload.get("exit_code").and_then(|v| v.as_i64());
                return (false, val.to_string(), code);
            }
        }
    }

    if let Some(code) = payload
        .pointer("/tool_result/metadata/exit_code")
        .or(payload.pointer("/metadata/exit_code"))
        .or(payload.get("exit_code"))
        .and_then(|v| v.as_i64())
    {
        if code != 0 {
            let detail = payload
                .get("stderr")
                .or(payload.get("output"))
                .and_then(|v| v.as_str())
                .unwrap_or("non-zero exit")
                .to_string();
            return (false, detail, Some(code));
        }
        return (true, String::new(), Some(code));
    }

    (true, String::new(), None)
}

pub fn record_tool_result(
    root: Option<&Path>,
    agent: &str,
    tool: &str,
    tool_input: &Value,
    payload: &Value,
) -> Result<()> {
    let action = normalize_action(tool, tool_input);
    let (ok, detail, exit_code) = detect_tool_failure(payload);
    log_attempt(root, agent, tool, &action, ok, &detail, exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_bash_collapses_whitespace() {
        let action = normalize_action("Bash", &json!({"command": "npm   test"}));
        assert_eq!(action, "bash:npm test");
    }

    #[test]
    fn detect_exit_code_failure() {
        let (ok, _, code) = detect_tool_failure(&json!({
            "tool_name": "Bash",
            "exit_code": 1,
            "stderr": "tests failed"
        }));
        assert!(!ok);
        assert_eq!(code, Some(1));
    }

    #[test]
    fn detect_error_field() {
        let (ok, detail, _) = detect_tool_failure(&json!({"error": "boom"}));
        assert!(!ok);
        assert!(detail.contains("boom"));
    }

    #[test]
    fn loop_breaker_blocks_after_two_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        crate::state::init_config(Some(root)).unwrap();

        let tool = "Bash";
        let action = "bash:npm test";
        log_attempt(Some(root), "claude", tool, action, false, "fail 1", Some(1)).unwrap();
        log_attempt(Some(root), "claude", tool, action, false, "fail 2", Some(1)).unwrap();

        let (block, reason) = should_block_retry(Some(root), tool, action).unwrap();
        assert!(block);
        assert!(reason.contains("loop breaker"));
    }
}
