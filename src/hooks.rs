use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::Duration;

use crate::acceptance::run_acceptance_gate;
use crate::constraints::{check_pre_tool_constraints, record_violation};
use crate::context::{render_context, ContextRender};
use crate::goal_edit::{save_goal, GoalForm};
use crate::loop_breaker::{check_pre_tool, record_tool_result};
use crate::paths::{find_project_root, read_json, repovow_dir, write_json_atomic};
use crate::snapshot::write_snapshot;
use crate::state::{load_config, load_state, log_event, remember_files, save_state, RepoVowState};

const CONTEXT_MARKER_FILE: &str = "context-marker.json";
const CONTEXT_SESSIONS_DIR: &str = "context-sessions";
const HOOK_DEDUP_DIR: &str = "hook-dedup";
const HOOK_DEDUP_WINDOW: Duration = Duration::from_secs(5);
const RECENT_FILE_LIMIT: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
}

impl Agent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "cursor" => Some(Self::Cursor),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

/// Map Cursor tool names to the canonical names used by loop breaker / constraints.
pub fn normalize_tool_name(tool: &str) -> &str {
    match tool {
        "Shell" => "Bash",
        "TabWrite" | "TabEdit" => "Write",
        _ => tool,
    }
}

pub fn read_stdin_json() -> Result<Value> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&raw)?)
}

fn emit_claude_block(reason: &str) -> ! {
    println!("{}", json!({"decision": "block", "reason": reason}));
    exit(0);
}

fn emit_codex_block(reason: &str) -> ! {
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        })
    );
    exit(0);
}

fn emit_cursor_block(reason: &str) -> ! {
    println!(
        "{}",
        json!({
            "permission": "deny",
            "user_message": reason,
            "agent_message": reason,
        })
    );
    exit(2);
}

fn emit_cursor_context(text: &str) -> ! {
    println!("{}", json!({"additional_context": text}));
    exit(0);
}

fn emit_codex_context(event: &str, text: &str) -> ! {
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": event,
                "additionalContext": text,
            }
        })
    );
    exit(0);
}

fn context_text() -> Result<ContextRender> {
    render_context(None)
}

fn log_context_delivery(event: &str, agent: Agent, context: &ContextRender) {
    let _ = log_event(
        None,
        "context_injected",
        json!({
            "hook_event": event,
            "agent": agent.as_str(),
            "estimated_tokens": context.estimated_tokens,
            "snapshot_estimated_tokens": context.snapshot_estimated_tokens,
            "saved_tokens": context.saved_tokens(),
            "savings_percent": context.savings_percent(),
        }),
    );
}

fn context_session_path(agent: Agent, payload: &Value) -> Option<PathBuf> {
    let session_id = payload.get("session_id")?.as_str()?;
    let mut hasher = Sha256::new();
    hasher.update(agent.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    Some(
        repovow_dir(None)
            .join(CONTEXT_SESSIONS_DIR)
            .join(format!("{:x}", hasher.finalize())),
    )
}

fn mark_session_context_delivered(agent: Agent, payload: &Value) -> Result<()> {
    let Some(path) = context_session_path(agent, payload) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, crate::paths::utcnow())?;
    Ok(())
}

fn claim_initial_session_context(agent: Agent, payload: &Value) -> Result<bool> {
    let Some(path) = context_session_path(agent, payload) else {
        return Ok(false);
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            writeln!(file, "{}", crate::paths::utcnow())?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn mark_compaction_context(agent: Agent, compaction: u32) -> Result<()> {
    write_json_atomic(
        &repovow_dir(None).join(CONTEXT_MARKER_FILE),
        &json!({
            "agent": agent.as_str(),
            "compaction": compaction,
        }),
    )
}

fn consume_compaction_context(agent: Agent, compaction: u32) -> Result<bool> {
    let path = repovow_dir(None).join(CONTEXT_MARKER_FILE);
    let marker = read_json(&path, json!({}))?;
    let matches = marker["agent"].as_str() == Some(agent.as_str())
        && marker["compaction"].as_u64() == Some(compaction as u64);
    if matches && path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(matches)
}

fn bump(state: &mut RepoVowState, field: &str) {
    match field {
        "compactions" => state.compactions += 1,
        "sessions" => state.sessions += 1,
        _ => {}
    }
}

pub fn run_hook(event: &str, agent: Agent) -> Result<()> {
    if !matches!(
        event,
        "pre-compact"
            | "post-compact"
            | "session-start"
            | "pre-tool-use"
            | "post-tool-use"
            | "stop"
            | "user-prompt-submit"
    ) {
        anyhow::bail!("unknown hook: {event}");
    }

    let payload = read_stdin_json()?;
    let root = find_project_root(None);
    if !root.join(crate::REPOVOW_DIR).join("config.json").is_file() {
        let auto_init_setting = crate::env_var("REPOVOW_AUTO_INIT");
        let auto_init = auto_init_setting.as_deref() != Ok("0")
            && root.join(".git").exists()
            && !root.join(".repovow-disabled").exists()
            && !root.join(".keel-disabled").exists();
        if !auto_init {
            return Ok(());
        }
        crate::install::bootstrap(Some(&root))?;
        log_event(
            Some(&root),
            "auto_initialized",
            json!({"agent": agent.as_str(), "hook_event": event}),
        )?;
    }
    if is_duplicate_hook_invocation(event, agent, &payload)? {
        return Ok(());
    }

    match event {
        "pre-compact" => handle_pre_compact(agent, &payload),
        "post-compact" => handle_post_compact(agent),
        "session-start" => handle_session_start(agent, &payload),
        "pre-tool-use" => handle_pre_tool_use(agent, &payload),
        "post-tool-use" => handle_post_tool_use(agent, &payload),
        "stop" => handle_stop(agent),
        "user-prompt-submit" => handle_user_prompt_submit(agent, &payload),
        _ => unreachable!(),
    }
}

fn is_duplicate_hook_invocation(event: &str, agent: Agent, payload: &Value) -> Result<bool> {
    let session_id = ["session_id", "sessionId", "conversation_id", "thread_id"]
        .iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str));
    let Some(session_id) = session_id else {
        // Synthetic/manual invocations commonly omit an ID and must retain their
        // original behavior, including rapid repeated loop-breaker events.
        return Ok(false);
    };

    let mut hasher = Sha256::new();
    hasher.update(event.as_bytes());
    hasher.update([0]);
    hasher.update(agent.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(session_id.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(payload)?);
    let key = format!("{:x}", hasher.finalize());
    let dir = repovow_dir(None).join(HOOK_DEDUP_DIR);
    fs::create_dir_all(&dir)?;
    let path = dir.join(key);

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            writeln!(file, "{}", crate::paths::utcnow())?;
            prune_hook_claims(&dir, &path);
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let recent = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                .map(|elapsed| elapsed <= HOOK_DEDUP_WINDOW)
                .unwrap_or(true);
            if recent {
                return Ok(true);
            }
            let _ = fs::remove_file(&path);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", crate::paths::utcnow())?;
                    prune_hook_claims(&dir, &path);
                    Ok(false)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(true),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn prune_hook_claims(dir: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == current {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .map(|elapsed| elapsed > HOOK_DEDUP_WINDOW)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

fn sync_cloud_quiet() {
    let _ = crate::cloud::push_state(None);
}

fn handle_pre_compact(agent: Agent, payload: &Value) -> Result<()> {
    let mut state = load_state(None)?;
    bump(&mut state, "compactions");
    state.last_agent = Some(agent.as_str().into());
    save_state(&mut state, None)?;
    write_snapshot(None)?;
    sync_cloud_quiet();
    log_event(
        None,
        "pre_compact",
        json!({"agent": agent.as_str(), "trigger": payload.get("trigger")}),
    )?;

    if agent == Agent::Claude {
        let ctx = context_text()?;
        log_context_delivery("PreCompact", agent, &ctx);
        // Exit 0 + systemMessage: preserve task state through compaction (do NOT exit 2 — that blocks compact).
        println!(
            "{}",
            json!({
                "systemMessage": format!(
                    "RepoVow task state to preserve through compaction:\n\n{}",
                    ctx.text
                ),
            })
        );
        mark_compaction_context(agent, state.compactions)?;
    } else if agent == Agent::Cursor {
        let ctx = context_text()?;
        log_context_delivery("PreCompact", agent, &ctx);
        println!(
            "{}",
            json!({
                "agent_message": format!(
                    "RepoVow task state to preserve through compaction:\n\n{}",
                    ctx.text
                ),
            })
        );
        mark_compaction_context(agent, state.compactions)?;
    }
    Ok(())
}

fn handle_post_compact(agent: Agent) -> Result<()> {
    write_snapshot(None)?;
    log_event(None, "post_compact", json!({"agent": agent.as_str()}))?;
    let state = load_state(None)?;
    let ctx = context_text()?;
    mark_compaction_context(agent, state.compactions)?;
    log_context_delivery("PostCompact", agent, &ctx);
    match agent {
        Agent::Codex => emit_codex_context("PostCompact", &ctx.text),
        Agent::Cursor => emit_cursor_context(&ctx.text),
        Agent::Claude => print!("{}", ctx.text),
    }
    Ok(())
}

fn handle_session_start(agent: Agent, payload: &Value) -> Result<()> {
    let _ = crate::cloud::pull_state(None);
    let mut state = load_state(None)?;
    bump(&mut state, "sessions");
    state.last_agent = Some(agent.as_str().into());
    save_state(&mut state, None)?;
    write_snapshot(None)?;
    sync_cloud_quiet();
    let source = payload
        .get("source")
        .or(payload.get("session_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    log_event(
        None,
        "session_start",
        json!({"agent": agent.as_str(), "source": source}),
    )?;
    if state.goal.is_none() {
        return Ok(());
    }
    if source.to_ascii_lowercase().contains("compact")
        && consume_compaction_context(agent, state.compactions)?
    {
        let message = "RepoVow context was already preserved by the compaction hook. Request only missing detail with `repovow context --section NAME`.";
        log_event(
            None,
            "context_deduplicated",
            json!({"hook_event": "SessionStart", "agent": agent.as_str()}),
        )?;
        mark_session_context_delivered(agent, payload)?;
        match agent {
            Agent::Codex => emit_codex_context("SessionStart", message),
            Agent::Cursor => emit_cursor_context(message),
            Agent::Claude => println!("{message}"),
        }
        return Ok(());
    }

    let ctx = context_text()?;
    log_context_delivery("SessionStart", agent, &ctx);
    mark_session_context_delivered(agent, payload)?;
    match agent {
        Agent::Codex => emit_codex_context("SessionStart", &ctx.text),
        Agent::Cursor => emit_cursor_context(&ctx.text),
        Agent::Claude => print!("{}", ctx.text),
    }
    Ok(())
}

fn cursor_tool_input(payload: &Value) -> Value {
    if let Some(input) = payload.get("tool_input").or(payload.get("input")) {
        return input.clone();
    }
    // Cursor beforeShellExecution-style payloads
    if let Some(cmd) = payload.get("command").and_then(|c| c.as_str()) {
        return json!({"command": cmd});
    }
    json!({})
}

fn recent_file_paths(tool: &str, input: &Value) -> Vec<String> {
    if !matches!(tool, "Write" | "Edit" | "ApplyPatch" | "apply_patch") {
        return Vec::new();
    }
    let mut raw = Vec::new();
    collect_structured_paths(input, &mut raw);
    collect_patch_paths(input, &mut raw);

    let root = find_project_root(None);
    let mut seen = HashSet::new();
    raw.into_iter()
        .filter_map(|path| normalize_project_path(&root, &path))
        .filter(|path| seen.insert(path.clone()))
        .take(RECENT_FILE_LIMIT)
        .collect()
}

fn collect_structured_paths(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let path_key = matches!(
                    key.as_str(),
                    "file_path" | "path" | "relative_path" | "target_file" | "target_path"
                );
                if path_key {
                    if let Some(path) = value.as_str() {
                        paths.push(path.to_string());
                    }
                } else {
                    collect_structured_paths(value, paths);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_structured_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn collect_patch_paths(value: &Value, paths: &mut Vec<String>) {
    let Some(patch) = value
        .as_str()
        .or_else(|| value.get("patch").and_then(Value::as_str))
        .or_else(|| value.get("input").and_then(Value::as_str))
    else {
        return;
    };
    for line in patch.lines() {
        for prefix in [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                paths.push(path.trim().to_string());
            }
        }
    }
}

fn normalize_project_path(root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("://") || raw.contains('\n') {
        return None;
    }
    let path = PathBuf::from(raw);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path
    };
    let display = relative
        .to_string_lossy()
        .trim_start_matches("./")
        .to_string();
    (!display.is_empty()).then_some(display)
}

fn handle_pre_tool_use(agent: Agent, payload: &Value) -> Result<()> {
    let raw_tool = payload
        .get("tool_name")
        .or(payload.get("tool"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let tool = normalize_tool_name(raw_tool);
    let tool_input = cursor_tool_input(payload);

    let (block, reason) = check_pre_tool(None, agent.as_str(), tool, &tool_input)?;
    if block {
        match agent {
            Agent::Codex => emit_codex_block(&reason),
            Agent::Cursor => emit_cursor_block(&reason),
            Agent::Claude => emit_claude_block(&reason),
        }
    }

    if let Some(reason) = crate::policy::hook_block_reason(None)? {
        match agent {
            Agent::Codex => emit_codex_block(&reason),
            Agent::Cursor => emit_cursor_block(&reason),
            Agent::Claude => emit_claude_block(&reason),
        }
    }

    let (block, reason) = check_pre_tool_constraints(None, tool, &tool_input)?;
    if block {
        let _ = record_violation(None, &reason);
        match agent {
            Agent::Codex => emit_codex_block(&reason),
            Agent::Cursor => emit_cursor_block(&reason),
            Agent::Claude => emit_claude_block(&reason),
        }
    }
    Ok(())
}

fn handle_post_tool_use(agent: Agent, payload: &Value) -> Result<()> {
    let raw_tool = payload
        .get("tool_name")
        .or(payload.get("tool"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let tool = normalize_tool_name(raw_tool);
    let tool_input = cursor_tool_input(payload);

    let (ok, _, _) = crate::loop_breaker::detect_tool_failure(payload);
    record_tool_result(None, agent.as_str(), tool, &tool_input, payload)?;
    if ok {
        let files = recent_file_paths(tool, &tool_input);
        if !files.is_empty() {
            let mut state = load_state(None)?;
            if remember_files(&mut state, &files, RECENT_FILE_LIMIT) {
                save_state(&mut state, None)?;
            }
        }
    }
    if !ok {
        write_snapshot(None)?;
        sync_cloud_quiet();
    }
    Ok(())
}

fn handle_stop(agent: Agent) -> Result<()> {
    let (ok, reason) = run_acceptance_gate(None)?;
    if ok {
        return Ok(());
    }
    match agent {
        Agent::Codex => {
            println!(
                "{}",
                json!({
                    "continue": false,
                    "systemMessage": reason,
                })
            );
            exit(0);
        }
        Agent::Cursor => {
            println!(
                "{}",
                json!({
                    "followup_message": reason,
                    "agent_message": reason,
                })
            );
            exit(2);
        }
        Agent::Claude => {
            println!(
                "{}",
                json!({
                    "continue": false,
                    "systemMessage": reason,
                })
            );
            exit(2);
        }
    }
}

fn handle_user_prompt_submit(agent: Agent, payload: &Value) -> Result<()> {
    let prompt = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    ensure_prompt_goal(prompt)?;
    let short: String = prompt.chars().take(500).collect();
    log_event(
        None,
        "user_prompt",
        json!({"agent": agent.as_str(), "prompt": short}),
    )?;
    if claim_initial_session_context(agent, payload)? {
        let ctx = context_text()?;
        log_context_delivery("UserPromptSubmit", agent, &ctx);
        match agent {
            Agent::Codex => emit_codex_context("UserPromptSubmit", &ctx.text),
            Agent::Cursor => emit_cursor_context(&ctx.text),
            Agent::Claude => print!("{}", ctx.text),
        }
        return Ok(());
    }
    if !load_config(None)?.context.prompt_reminder {
        return Ok(());
    }
    let ctx = "RepoVow: If context is missing, use `repovow context --section NAME` and start from the working set. \
               Do not reread the repository or full snapshot.";
    match agent {
        Agent::Codex => emit_codex_context("UserPromptSubmit", ctx),
        Agent::Cursor => emit_cursor_context(ctx),
        Agent::Claude => println!("{ctx}"),
    }
    Ok(())
}

fn ensure_prompt_goal(prompt: &str) -> Result<()> {
    let state = load_state(None)?;
    if state.goal.is_some() {
        return Ok(());
    }
    let title: String = prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect();
    if title.is_empty() {
        return Ok(());
    }
    save_goal(
        &GoalForm {
            title,
            step: "Execute and verify; finish with `repovow progress --done \"...\"`".into(),
            acceptance: vec![
                "The requested outcome is implemented".into(),
                "Relevant repository checks pass".into(),
            ],
            constraints: Vec::new(),
        },
        None,
        "agent-prompt",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_recent_files_from_structured_and_patch_inputs() {
        let structured = json!({"file_path": "src/context.rs"});
        assert_eq!(
            recent_file_paths("Edit", &structured),
            vec!["src/context.rs"]
        );

        let patch = json!({
            "patch": "*** Begin Patch\n*** Update File: src/hooks.rs\n*** Add File: tests/context.rs\n*** End Patch\n"
        });
        assert_eq!(
            recent_file_paths("apply_patch", &patch),
            vec!["src/hooks.rs", "tests/context.rs"]
        );
    }

    #[test]
    fn ignores_paths_for_read_only_tools() {
        assert!(recent_file_paths("Read", &json!({"path": "src/lib.rs"})).is_empty());
    }
}
