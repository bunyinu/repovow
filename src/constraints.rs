use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::path::Path;

use crate::state::{load_state, log_event, save_state};

fn haystack(tool: &str, tool_input: &Value) -> String {
    let mut parts = vec![tool.to_lowercase()];
    if let Some(cmd) = tool_input.get("command").and_then(|c| c.as_str()) {
        parts.push(cmd.to_lowercase());
    }
    for key in ["file_path", "path", "notebook_path", "content"] {
        if let Some(p) = tool_input.get(key).and_then(|v| v.as_str()) {
            parts.push(p.to_lowercase());
        }
    }
    parts.join(" ")
}

fn constraint_texts(state: &crate::state::RepoVowState) -> Vec<String> {
    state
        .goal
        .as_ref()
        .map(|g| {
            g.constraints
                .iter()
                .map(|c| c.to_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn wants_no_deps(constraints: &[String]) -> bool {
    constraints.iter().any(|c| {
        c.contains("no new dep")
            || c.contains("no new deps")
            || c.contains("no dependencies")
            || c.contains("no dependency")
            || c.contains("no npm install")
    })
}

fn wants_read_only(constraints: &[String]) -> bool {
    constraints.iter().any(|c| {
        c.contains("read-only")
            || c.contains("read only")
            || c.contains("no edits")
            || c.contains("no file change")
            || c.contains("no code change")
    })
}

fn banned_tokens(constraints: &[String]) -> Vec<String> {
    let re = Regex::new(r"no\s+([a-z0-9][a-z0-9 _-]{1,40})").unwrap();
    let mut out = Vec::new();
    for c in constraints {
        if wants_no_deps(std::slice::from_ref(c)) || wants_read_only(std::slice::from_ref(c)) {
            continue;
        }
        for cap in re.captures_iter(c) {
            let token = cap[1].trim().to_string();
            if token.len() >= 3 && !token.starts_with("new dep") {
                out.push(token);
            }
        }
        for kw in ["stripe", "paypal", "braintree"] {
            if c.contains(kw) {
                out.push(kw.to_string());
            }
        }
        if c.contains("payment") {
            for kw in ["stripe", "paypal", "braintree"] {
                out.push(kw.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_dep_install(tool: &str, text: &str) -> bool {
    if tool != "Bash" && tool != "Shell" {
        return false;
    }
    [
        "npm install",
        "npm i ",
        "npm i-",
        "yarn add",
        "pnpm add",
        "pip install",
        "cargo add",
        "go get ",
        "bun add",
        "pnpm i ",
    ]
    .iter()
    .any(|p| text.contains(p))
}

fn is_write_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Write" | "Edit" | "ApplyPatch" | "apply_patch" | "NotebookEdit" | "TabWrite" | "TabEdit"
    )
}

pub fn check_pre_tool_constraints(
    root: Option<&Path>,
    tool: &str,
    tool_input: &Value,
) -> Result<(bool, String)> {
    let state = load_state(root)?;
    let constraints = constraint_texts(&state);
    if constraints.is_empty() {
        return Ok((false, String::new()));
    }

    let text = haystack(tool, tool_input);

    if wants_read_only(&constraints) && is_write_tool(tool) {
        return Ok((
            true,
            "RepoVow constraint guard: read-only mode — file edits are blocked. \
             Update constraints or run `repovow progress --blocker` if this is intentional."
                .into(),
        ));
    }

    if wants_no_deps(&constraints) && is_dep_install(tool, &text) {
        return Ok((
            true,
            "RepoVow constraint guard: blocked dependency install (constraint: no new deps). \
             Use existing packages or update the goal constraints."
                .into(),
        ));
    }

    for token in banned_tokens(&constraints) {
        if text.contains(&token) {
            return Ok((
                true,
                format!(
                    "RepoVow constraint guard: blocked action matching constraint \"no {token}\"."
                ),
            ));
        }
    }

    Ok((false, String::new()))
}

pub fn record_violation(root: Option<&Path>, reason: &str) -> Result<()> {
    let mut state = load_state(root)?;
    let short: String = reason.chars().take(200).collect();
    if !state.progress.blockers.iter().any(|b| b == &short) {
        state.progress.blockers.push(short.clone());
        save_state(&mut state, root)?;
        crate::snapshot::write_snapshot(root)?;
    }
    log_event(
        root,
        "constraint_block",
        serde_json::json!({"reason": reason}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::utcnow;
    use crate::state::{Goal, RepoVowState};
    use serde_json::json;

    fn state_with_constraints(constraints: Vec<&str>) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(crate::REPOVOW_DIR)).unwrap();
        let mut state = RepoVowState {
            goal: Some(Goal {
                title: "t".into(),
                acceptance: vec![],
                constraints: constraints.into_iter().map(|s| s.to_string()).collect(),
                started_at: utcnow(),
            }),
            ..RepoVowState::default()
        };
        crate::state::save_state(&mut state, Some(root)).unwrap();
        crate::state::init_config(Some(root)).unwrap();
        tmp
    }

    #[test]
    fn blocks_npm_install_when_no_deps() {
        let tmp = state_with_constraints(vec!["no new deps"]);
        let (block, reason) = check_pre_tool_constraints(
            Some(tmp.path()),
            "Bash",
            &json!({"command": "npm install lodash"}),
        )
        .unwrap();
        assert!(block);
        assert!(reason.contains("no new deps"));
    }

    #[test]
    fn blocks_write_in_read_only() {
        let tmp = state_with_constraints(vec!["read-only"]);
        let (block, _) = check_pre_tool_constraints(
            Some(tmp.path()),
            "Write",
            &json!({"file_path": "src/a.ts"}),
        )
        .unwrap();
        assert!(block);
    }

    #[test]
    fn blocks_stripe_constraint() {
        let tmp = state_with_constraints(vec!["no payment SDK"]);
        let (block, reason) = check_pre_tool_constraints(
            Some(tmp.path()),
            "Bash",
            &json!({"command": "npm install stripe"}),
        )
        .unwrap();
        assert!(block);
        assert!(reason.contains("stripe"));
    }

    #[test]
    fn allows_when_no_constraints() {
        let tmp = state_with_constraints(vec![]);
        let (block, _) = check_pre_tool_constraints(
            Some(tmp.path()),
            "Bash",
            &json!({"command": "npm install lodash"}),
        )
        .unwrap();
        assert!(!block);
    }
}
