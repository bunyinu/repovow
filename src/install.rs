use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::paths::{ensure_repovow_dir, find_project_root};
use crate::snapshot::write_snapshot;
use crate::state::init_config;

const CLAUDE_MD_SNIPPET: &str = r#"## RepoVow (agent state)

The injected RepoVow packet replaces `.repovow/snapshot.md`; never reread the full snapshot after receiving it. Do not query sections already shown.
Start continuations from `Working set` and `Recently completed`. Batch independent reads/searches into one tool turn, avoid repository inventories, and use at most one `repovow progress` checkpoint.
"#;

const AGENTS_MD_SNIPPET: &str = r#"## RepoVow (agent state)

The injected RepoVow packet replaces `.repovow/snapshot.md`; never reread the full snapshot after receiving it. Do not query sections already shown.
Start continuations from `Working set` and `Recently completed`. Batch independent reads/searches into one tool turn, avoid repository inventories, and use at most one `repovow progress` checkpoint.
"#;

const REPOVOW_AGENT_SKILL: &str = include_str!("../templates/agent-skill/SKILL.md");
const REPOVOW_SKILL_MARKER: &str = "<!-- managed-by-repovow -->";
const LEGACY_SKILL_MARKER: &str = "<!-- managed-by-keel -->";

pub fn repovow_binary() -> String {
    if let Ok(bin) = crate::env_var("REPOVOW_BIN") {
        if bin.contains(' ') {
            return format!("\"{bin}\"");
        }
        return bin;
    }

    if let Some(path) = find_repovow_on_path() {
        let s = path.display().to_string();
        return if s.contains(' ') {
            format!("\"{s}\"")
        } else {
            s
        };
    }

    if let Ok(exe) = std::env::current_exe() {
        let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "repovow" || name == "repovow.exe" {
            let s = exe.display().to_string();
            return if s.contains(' ') {
                format!("\"{s}\"")
            } else {
                s
            };
        }
    }

    "repovow".to_string()
}

fn find_repovow_on_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .args(["-c", "command -v repovow"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn hook_cmd(event: &str, agent: &str) -> String {
    format!("{} hook {event} --agent {agent}", repovow_binary())
}

fn claude_hooks() -> Value {
    json!({
        "PreCompact": [{
            "matcher": "",
            "hooks": [{"type": "command", "command": hook_cmd("pre-compact", "claude"), "timeout": 15}]
        }],
        "SessionStart": [{
            "matcher": "startup|resume|clear|compact",
            "hooks": [{"type": "command", "command": hook_cmd("session-start", "claude"), "timeout": 15}]
        }],
        "PreToolUse": [{
            "matcher": "Bash|Edit|Write|ApplyPatch",
            "hooks": [{"type": "command", "command": hook_cmd("pre-tool-use", "claude"), "timeout": 10}]
        }],
        "PostToolUse": [{
            "matcher": "Bash|Edit|Write|ApplyPatch",
            "hooks": [{"type": "command", "command": hook_cmd("post-tool-use", "claude"), "timeout": 10}]
        }],
        "UserPromptSubmit": [{
            "hooks": [{"type": "command", "command": hook_cmd("user-prompt-submit", "claude"), "timeout": 5}]
        }],
        "Stop": [{
            "matcher": "",
            "hooks": [{"type": "command", "command": hook_cmd("stop", "claude"), "timeout": 120}]
        }]
    })
}

fn codex_hooks() -> Value {
    json!({
        "PreCompact": [{
            "matcher": "manual|auto",
            "hooks": [{
                "type": "command",
                "command": hook_cmd("pre-compact", "codex"),
                "timeout": 15,
                "statusMessage": "RepoVow: saving state before compaction"
            }]
        }],
        "PostCompact": [{
            "matcher": "manual|auto",
            "hooks": [{
                "type": "command",
                "command": hook_cmd("post-compact", "codex"),
                "timeout": 15,
                "statusMessage": "RepoVow: restoring state after compaction"
            }]
        }],
        "SessionStart": [{
            "matcher": "startup|resume|clear|compact",
            "hooks": [{
                "type": "command",
                "command": hook_cmd("session-start", "codex"),
                "timeout": 15,
                "statusMessage": "RepoVow: loading session state"
            }]
        }],
        "PreToolUse": [{
            "matcher": "Bash|apply_patch|Edit|Write",
            "hooks": [{
                "type": "command",
                "command": hook_cmd("pre-tool-use", "codex"),
                "timeout": 10,
                "statusMessage": "RepoVow: checking retry loop"
            }]
        }],
        "PostToolUse": [{
            "matcher": "Bash|apply_patch|Edit|Write",
            "hooks": [{"type": "command", "command": hook_cmd("post-tool-use", "codex"), "timeout": 10}]
        }],
        "UserPromptSubmit": [{
            "hooks": [{"type": "command", "command": hook_cmd("user-prompt-submit", "codex"), "timeout": 5}]
        }],
        "Stop": [{
            "hooks": [{
                "type": "command",
                "command": hook_cmd("stop", "codex"),
                "timeout": 120,
                "statusMessage": "RepoVow: running acceptance gate"
            }]
        }]
    })
}

fn load_hook_document(path: &Path) -> Result<Value> {
    if path.exists() {
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    } else {
        Ok(json!({"hooks": {}}))
    }
}

fn write_hook_document(path: &Path, document: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::paths::write_json_atomic(path, document)
}

const CODEX_DEFAULT_HOOK_TIMEOUT: u64 = 600;
const CODEX_DEFAULT_CONTEXT_LIMIT: u64 = 2_500;

fn codex_event_key(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("pre_tool_use"),
        "PostToolUse" => Some("post_tool_use"),
        "PreCompact" => Some("pre_compact"),
        "PostCompact" => Some("post_compact"),
        "SessionStart" => Some("session_start"),
        "UserPromptSubmit" => Some("user_prompt_submit"),
        "Stop" => Some("stop"),
        _ => None,
    }
}

fn codex_event_uses_matcher(event: &str) -> bool {
    !matches!(event, "UserPromptSubmit" | "Stop")
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical_json(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// Reproduce Codex's normalized hook identity for the RepoVow command hooks we install.
/// This is deliberately scoped to RepoVow-owned handlers and covered by a reference vector.
fn codex_command_hook_hash(event: &str, group: &Value, hook: &Value) -> Result<String> {
    let command = hook["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Codex command hook has no command"))?;
    let timeout = hook["timeout"]
        .as_u64()
        .unwrap_or(CODEX_DEFAULT_HOOK_TIMEOUT)
        .max(1);
    let mut normalized_hook = serde_json::Map::new();
    normalized_hook.insert("type".into(), json!("command"));
    normalized_hook.insert("command".into(), json!(command));
    normalized_hook.insert("timeout".into(), json!(timeout));
    normalized_hook.insert(
        "async".into(),
        json!(hook["async"].as_bool().unwrap_or(false)),
    );
    if let Some(message) = hook["statusMessage"].as_str() {
        normalized_hook.insert("statusMessage".into(), json!(message));
    }
    if let Some(limit) = hook["additionalContextLimit"].as_u64() {
        if limit != CODEX_DEFAULT_CONTEXT_LIMIT {
            normalized_hook.insert("additionalContextLimit".into(), json!(limit));
        }
    }

    let event_key = codex_event_key(event)
        .ok_or_else(|| anyhow::anyhow!("unsupported Codex hook event: {event}"))?;
    let mut identity = serde_json::Map::new();
    identity.insert("event_name".into(), json!(event_key));
    identity.insert(
        "hooks".into(),
        Value::Array(vec![Value::Object(normalized_hook)]),
    );
    if codex_event_uses_matcher(event) {
        if let Some(matcher) = group["matcher"].as_str() {
            identity.insert("matcher".into(), json!(matcher));
        }
    }

    let encoded = serde_json::to_vec(&canonical_json(&Value::Object(identity)))?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn codex_repovow_trust_entries(
    codex_path: &Path,
    document: &Value,
) -> Result<Vec<(String, String)>> {
    let source = codex_path
        .canonicalize()
        .unwrap_or_else(|_| codex_path.to_path_buf());
    let mut entries = Vec::new();
    let Some(events) = document["hooks"].as_object() else {
        return Ok(entries);
    };

    for (event, groups) in events {
        let Some(event_key) = codex_event_key(event) else {
            continue;
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let Some(hooks) = group["hooks"].as_array() else {
                continue;
            };
            for (handler_index, hook) in hooks.iter().enumerate() {
                let expected = hook_cmd(&event_to_cli_name(event), "codex");
                let is_repovow = hook["command"].as_str() == Some(expected.as_str());
                if !is_repovow {
                    continue;
                }
                let key = format!(
                    "{}:{event_key}:{group_index}:{handler_index}",
                    source.display()
                );
                entries.push((key, codex_command_hook_hash(event, group, hook)?));
            }
        }
    }
    Ok(entries)
}

fn trust_codex_repovow_hooks(codex_path: &Path, config_path: &Path) -> Result<usize> {
    let document = load_hook_document(codex_path)?;
    let entries = codex_repovow_trust_entries(codex_path, &document)?;
    if entries.is_empty() {
        return Ok(0);
    }

    let raw = if config_path.exists() {
        fs::read_to_string(config_path)?
    } else {
        String::new()
    };
    let mut config = raw
        .parse::<DocumentMut>()
        .with_context(|| format!("parse {}", config_path.display()))?;
    if !config.as_table().contains_key("hooks") {
        config["hooks"] = Item::Table(Table::new());
    }
    let hooks = config["hooks"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("hooks in {} is not a table", config_path.display()))?;
    if !hooks.contains_key("state") {
        hooks["state"] = Item::Table(Table::new());
    }
    let state = hooks["state"].as_table_mut().ok_or_else(|| {
        anyhow::anyhow!("hooks.state in {} is not a table", config_path.display())
    })?;
    for (key, hash) in &entries {
        if !state.contains_key(key) {
            state[key] = Item::Table(Table::new());
        }
        let entry = state[key].as_table_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "hooks.state.{key} in {} is not a table",
                config_path.display()
            )
        })?;
        entry["trusted_hash"] = value(hash);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = config_path.with_extension("toml.repovow.tmp");
    fs::write(&tmp, config.to_string())?;
    fs::rename(tmp, config_path)?;
    Ok(entries.len())
}

fn codex_repovow_hooks_trusted(codex_path: &Path, config_path: &Path) -> Result<bool> {
    if !config_path.exists() {
        return Ok(false);
    }
    let document = load_hook_document(codex_path)?;
    let entries = codex_repovow_trust_entries(codex_path, &document)?;
    if entries.len() != 7 {
        return Ok(false);
    }
    let config = fs::read_to_string(config_path)?
        .parse::<DocumentMut>()
        .with_context(|| format!("parse {}", config_path.display()))?;
    let Some(state) = config["hooks"]["state"].as_table() else {
        return Ok(false);
    };
    Ok(entries
        .iter()
        .all(|(key, hash)| state[key]["trusted_hash"].as_str() == Some(hash.as_str())))
}

fn install_global_hooks_to(claude_path: &Path, codex_path: &Path) -> Result<()> {
    let mut claude = load_hook_document(claude_path)?;
    if claude.get("hooks").is_none() {
        claude["hooks"] = json!({});
    }
    merge_hooks(&mut claude, &claude_hooks());
    write_hook_document(claude_path, &claude)?;

    let mut codex = load_hook_document(codex_path)?;
    if codex.get("hooks").is_none() {
        codex["hooks"] = json!({});
    }
    merge_hooks(&mut codex, &codex_hooks());
    write_hook_document(codex_path, &codex)?;
    Ok(())
}

fn install_global_instructions(home: &Path) -> Result<()> {
    upsert_snippet(&home.join("CLAUDE.md"), CLAUDE_MD_SNIPPET, "## RepoVow")?;
    upsert_snippet(&home.join("AGENTS.md"), AGENTS_MD_SNIPPET, "## RepoVow")?;
    Ok(())
}

/// Install persistent user-level routers. They are loaded once by each agent and
/// automatically bootstrap minimal RepoVow state when opened inside a Git repository.
pub fn install_global_hooks() -> Result<(PathBuf, PathBuf)> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; cannot install user-level agent hooks"))?;
    let claude_path = home.join(".claude/settings.json");
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let codex_path = codex_home.join("hooks.json");
    install_global_hooks_to(&claude_path, &codex_path)?;
    install_global_instructions(&home)?;
    // Running the RepoVow installer is consent to trust RepoVow's own handlers. Never
    // register trust for unrelated commands that share the user's hooks file.
    trust_codex_repovow_hooks(&codex_path, &codex_home.join("config.toml"))?;
    Ok((claude_path, codex_path))
}

pub fn global_hooks_status() -> Result<Vec<(String, PathBuf, bool, bool)>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; cannot inspect user-level agent hooks"))?;
    let claude_path = home.join(".claude/settings.json");
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let codex_path = codex_home.join("hooks.json");
    let claude_installed = hook_document_contains_all(
        &claude_path,
        "claude",
        &[
            "PreCompact",
            "SessionStart",
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "Stop",
        ],
    );
    let codex_installed = hook_document_contains_all(
        &codex_path,
        "codex",
        &[
            "PreCompact",
            "PostCompact",
            "SessionStart",
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "Stop",
        ],
    );
    let codex_trusted = codex_installed
        && codex_repovow_hooks_trusted(&codex_path, &codex_home.join("config.toml"))
            .unwrap_or(false);
    Ok(vec![
        (
            "Claude Code".into(),
            claude_path.clone(),
            claude_installed,
            claude_installed,
        ),
        (
            "Codex".into(),
            codex_path.clone(),
            codex_installed,
            codex_trusted,
        ),
    ])
}

fn hook_document_contains_all(path: &Path, agent: &str, events: &[&str]) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    events.iter().all(|event| {
        let expected = format!("hook {} --agent {agent}", event_to_cli_name(event));
        document["hooks"][event].as_array().is_some_and(|groups| {
            groups.iter().any(|group| {
                group["hooks"].as_array().is_some_and(|hooks| {
                    hooks.iter().any(|hook| {
                        hook["command"]
                            .as_str()
                            .is_some_and(|command| command.contains(&expected))
                    })
                })
            })
        })
    })
}

fn event_to_cli_name(event: &str) -> String {
    event
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            if index > 0 && character.is_ascii_uppercase() {
                vec!['-', character.to_ascii_lowercase()]
            } else {
                vec![character.to_ascii_lowercase()]
            }
        })
        .collect()
}

fn cursor_hooks() -> Value {
    json!({
        "preCompact": [{
            "command": hook_cmd("pre-compact", "cursor"),
            "timeout": 15
        }],
        "sessionStart": [{
            "command": hook_cmd("session-start", "cursor"),
            "timeout": 15
        }],
        "preToolUse": [{
            "command": hook_cmd("pre-tool-use", "cursor"),
            "timeout": 10,
            "matcher": "Shell|Write|Edit"
        }],
        "postToolUse": [{
            "command": hook_cmd("post-tool-use", "cursor"),
            "timeout": 10,
            "matcher": "Shell|Write|Edit"
        }],
        "beforeSubmitPrompt": [{
            "command": hook_cmd("user-prompt-submit", "cursor"),
            "timeout": 5
        }],
        "stop": [{
            "command": hook_cmd("stop", "cursor"),
            "timeout": 120,
            "failClosed": true
        }]
    })
}

fn is_managed_hook_command(command: &str) -> bool {
    command.contains("repovow hook") || command.contains("keel hook")
}

/// Cursor uses `{ "version": 1, "hooks": { "event": [ { "command": ... } ] } }`.
fn merge_cursor_hooks(existing: &mut Value, new_hooks: &Value) {
    if !existing.is_object() {
        *existing = json!({"version": 1, "hooks": {}});
    }
    let obj = existing.as_object_mut().unwrap();
    obj.entry("version").or_insert(json!(1));
    let hooks = obj
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .unwrap();
    let Some(new_obj) = new_hooks.as_object() else {
        return;
    };

    for (event, entries) in new_obj {
        let current = hooks
            .entry(event.clone())
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap();
        current.retain(|entry| {
            !entry
                .get("command")
                .and_then(|c| c.as_str())
                .map(is_managed_hook_command)
                .unwrap_or(false)
        });
        if let Some(arr) = entries.as_array() {
            for entry in arr {
                current.push(entry.clone());
            }
        }
    }
}

fn merge_hooks(existing: &mut Value, new_hooks: &Value) {
    let hooks = existing
        .as_object_mut()
        .and_then(|o| o.entry("hooks").or_insert(json!({})).as_object_mut());

    let Some(hooks) = hooks else { return };
    let Some(new_obj) = new_hooks.as_object() else {
        return;
    };

    for (event, groups) in new_obj {
        let current = hooks
            .entry(event.clone())
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap();

        let repovow_markers: Vec<String> = groups
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|g| g["hooks"][0]["command"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        current.retain_mut(|group| {
            let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            hooks.retain(|hook| {
                !hook["command"]
                    .as_str()
                    .map(|command| {
                        is_managed_hook_command(command)
                            || repovow_markers.iter().any(|marker| command == marker)
                    })
                    .unwrap_or(false)
            });
            !hooks.is_empty()
        });

        if let Some(new_groups) = groups.as_array() {
            for g in new_groups {
                current.push(g.clone());
            }
        }
    }
}

fn upsert_snippet(path: &Path, snippet: &str, marker: &str) -> Result<()> {
    let text = if path.exists() {
        let existing = fs::read_to_string(path)?;
        let markers = if marker == "## RepoVow" {
            vec![marker, "## Keel"]
        } else {
            vec![marker]
        };
        let marker_match = markers
            .iter()
            .filter_map(|candidate| {
                existing.match_indices(candidate).find_map(|(position, _)| {
                    (position == 0 || existing.as_bytes().get(position - 1) == Some(&b'\n'))
                        .then_some((position, candidate.len()))
                })
            })
            .min_by_key(|(position, _)| *position);
        if let Some((marker_pos, marker_len)) = marker_match {
            let start = existing[..marker_pos]
                .rfind('\n')
                .map_or(0, |position| position + 1);
            let search_from = marker_pos + marker_len;
            let end = existing[search_from..]
                .find("\n## ")
                .map_or(existing.len(), |position| search_from + position + 1);
            let before = existing[..start].trim_end();
            let after = existing[end..].trim_start();
            match (before.is_empty(), after.is_empty()) {
                (true, true) => format!("{}\n", snippet.trim()),
                (true, false) => format!("{}\n\n{after}\n", snippet.trim()),
                (false, true) => format!("{before}\n\n{}\n", snippet.trim()),
                (false, false) => format!("{before}\n\n{}\n\n{after}\n", snippet.trim()),
            }
        } else {
            format!("{}\n\n{snippet}\n", existing.trim_end())
        }
    } else {
        format!("{snippet}\n")
    };
    fs::write(path, text)?;
    Ok(())
}

fn ensure_runtime_gitignore(root: &Path) -> Result<()> {
    let path = root.join(crate::REPOVOW_DIR).join(".gitignore");
    let required = [
        "*.tmp",
        "cloud.json",
        "context-marker.json",
        "context-sessions/",
        "hook-dedup/",
        "policy-warning-marker.json",
        "policy.key",
    ];
    let mut lines: Vec<String> = if path.exists() {
        fs::read_to_string(&path)?
            .lines()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    for entry in required {
        if !lines.iter().any(|line| line.trim() == entry) {
            lines.push(entry.to_string());
        }
    }
    fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

fn ensure_agent_skill(root: &Path) -> Result<()> {
    let legacy_dir = root.join(".agents/skills/keel");
    let legacy_path = legacy_dir.join("SKILL.md");
    if legacy_path.exists() && fs::read_to_string(&legacy_path)?.contains(LEGACY_SKILL_MARKER) {
        fs::remove_file(&legacy_path)?;
        if legacy_dir.read_dir()?.next().is_none() {
            fs::remove_dir(&legacy_dir)?;
        }
    }

    let skill_dir = root.join(".agents/skills/repovow");
    let skill_path = skill_dir.join("SKILL.md");
    if skill_path.exists() {
        let existing = fs::read_to_string(&skill_path)?;
        if !existing.contains(REPOVOW_SKILL_MARKER) {
            return Ok(());
        }
    }
    fs::create_dir_all(skill_dir)?;
    fs::write(skill_path, REPOVOW_AGENT_SKILL)?;
    Ok(())
}

pub fn install(project: Option<&Path>) -> Result<PathBuf> {
    let root = find_project_root(project);
    ensure_repovow_dir(Some(&root))?;
    init_config(Some(&root))?;
    ensure_runtime_gitignore(&root)?;
    ensure_agent_skill(&root)?;

    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir)?;
    let settings_path = claude_dir.join("settings.json");
    let mut settings: Value = if settings_path.exists() {
        serde_json::from_str(&fs::read_to_string(&settings_path)?)?
    } else {
        json!({})
    };
    merge_hooks(&mut settings, &claude_hooks());
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings)? + "\n",
    )?;

    let codex_dir = root.join(".codex");
    if codex_dir.exists() && !codex_dir.is_dir() {
        anyhow::bail!(
            "{} exists as a file, not a directory. Remove or rename it, then run `repovow init` again.",
            codex_dir.display()
        );
    }
    fs::create_dir_all(&codex_dir)?;
    let hooks_path = codex_dir.join("hooks.json");
    let mut codex_doc: Value = if hooks_path.exists() {
        serde_json::from_str(&fs::read_to_string(&hooks_path)?)?
    } else {
        json!({"hooks": {}})
    };
    if codex_doc.get("hooks").is_none() {
        codex_doc["hooks"] = json!({});
    }
    merge_hooks(&mut codex_doc, &codex_hooks());
    fs::write(
        &hooks_path,
        serde_json::to_string_pretty(&codex_doc)? + "\n",
    )?;

    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir)?;
    let cursor_hooks_path = cursor_dir.join("hooks.json");
    let mut cursor_doc: Value = if cursor_hooks_path.exists() {
        serde_json::from_str(&fs::read_to_string(&cursor_hooks_path)?)?
    } else {
        json!({"version": 1, "hooks": {}})
    };
    if cursor_doc.get("hooks").is_none() {
        cursor_doc["hooks"] = json!({});
    }
    merge_cursor_hooks(&mut cursor_doc, &cursor_hooks());
    fs::write(
        &cursor_hooks_path,
        serde_json::to_string_pretty(&cursor_doc)? + "\n",
    )?;

    upsert_snippet(&root.join("CLAUDE.md"), CLAUDE_MD_SNIPPET, "## RepoVow")?;
    upsert_snippet(&root.join("AGENTS.md"), AGENTS_MD_SNIPPET, "## RepoVow")?;
    write_snapshot(Some(&root))?;

    Ok(root)
}

/// Minimal automatic setup used by persistent agent hooks. This deliberately
/// avoids changing project instruction and hook files; the user-level router
/// is already active and only repo-local state is required.
pub fn bootstrap(project: Option<&Path>) -> Result<PathBuf> {
    let root = find_project_root(project);
    ensure_repovow_dir(Some(&root))?;
    init_config(Some(&root))?;
    ensure_runtime_gitignore(&root)?;
    write_snapshot(Some(&root))?;
    Ok(root)
}

/// Initialize a project and ensure future/current agent sessions have the
/// persistent user-level RepoVow router available.
pub fn install_for_user(project: Option<&Path>) -> Result<PathBuf> {
    let root = install(project)?;
    let skip_global_hooks = crate::env_var("REPOVOW_SKIP_GLOBAL_HOOKS");
    if skip_global_hooks.as_deref() != Ok("1") {
        install_global_hooks()?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_creates_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::env::set_var("REPOVOW_BIN", "repovow");
        install(Some(root)).unwrap();
        assert!(root.join(".repovow").is_dir());
        assert!(std::fs::read_to_string(root.join(".repovow/.gitignore"))
            .unwrap()
            .contains("context-marker.json"));
        assert!(root.join(".claude/settings.json").exists());
        assert!(root.join(".codex/hooks.json").exists());
        assert!(root.join(".cursor/hooks.json").exists());
        assert!(root.join(".agents/skills/repovow/SKILL.md").exists());
        assert!(
            std::fs::read_to_string(root.join(".agents/skills/repovow/SKILL.md"))
                .unwrap()
                .contains("Batch independent file reads")
        );
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(hooks[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("repovow hook"));
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["matcher"],
            "startup|resume|clear|compact"
        );
    }

    #[test]
    fn bootstrap_creates_only_repo_state() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        bootstrap(Some(root)).unwrap();

        assert!(root.join(".repovow/config.json").exists());
        assert!(
            root.join(".repovow/state.json").exists() || root.join(".repovow/snapshot.md").exists()
        );
        assert!(!root.join(".claude").exists());
        assert!(!root.join(".codex").exists());
        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join("AGENTS.md").exists());
    }

    #[test]
    fn managed_snippet_is_upgraded_without_removing_user_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        std::fs::write(
            &path,
            "## RepoVow (agent state)\n\nRead `.repovow/snapshot.md` every time.\n\n## Project\n\nKeep this.\n",
        )
        .unwrap();
        upsert_snippet(&path, AGENTS_MD_SNIPPET, "## RepoVow").unwrap();
        let updated = std::fs::read_to_string(path).unwrap();
        assert!(updated.contains("The injected RepoVow packet replaces"));
        assert!(updated.contains("## Project\n\nKeep this."));
        assert!(!updated.contains("snapshot.md` every time"));
    }

    #[test]
    fn legacy_managed_snippet_is_rebranded_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        std::fs::write(
            &path,
            "## Keel (agent state)\n\nRead `.keel/snapshot.md` every time.\n\n## Project\n\nKeep this.\n",
        )
        .unwrap();

        upsert_snippet(&path, CLAUDE_MD_SNIPPET, "## RepoVow").unwrap();

        let updated = std::fs::read_to_string(path).unwrap();
        assert!(updated.contains("## RepoVow (agent state)"));
        assert!(!updated.contains("## Keel"));
        assert!(updated.contains("## Project\n\nKeep this."));
    }

    #[test]
    fn global_instructions_upgrade_legacy_sections_and_preserve_user_content() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("CLAUDE.md"),
            "## Keel (agent state)\n\nRead `.keel/snapshot.md`.\n\n## User rule\n\nKeep this.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "## Keel (agent state)\n\nRun `keel progress`.\n\n## Tools\n\nKeep tools.\n",
        )
        .unwrap();

        install_global_instructions(tmp.path()).unwrap();
        install_global_instructions(tmp.path()).unwrap();

        let claude = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
        let agents = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert_eq!(claude.matches("## RepoVow").count(), 1);
        assert_eq!(agents.matches("## RepoVow").count(), 1);
        assert!(!claude.contains("## Keel"));
        assert!(!agents.contains("## Keel"));
        assert!(claude.contains("## User rule\n\nKeep this."));
        assert!(agents.contains("## Tools\n\nKeep tools."));
    }

    #[test]
    fn agent_skill_does_not_overwrite_an_unmanaged_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = tmp.path().join(".agents/skills/repovow/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "user-owned skill\n").unwrap();
        ensure_agent_skill(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(skill).unwrap(),
            "user-owned skill\n"
        );
    }

    #[test]
    fn managed_legacy_skill_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join(".agents/skills/keel/SKILL.md");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, format!("{LEGACY_SKILL_MARKER}\nold skill\n")).unwrap();

        ensure_agent_skill(tmp.path()).unwrap();

        assert!(!legacy.exists());
        assert!(tmp.path().join(".agents/skills/repovow/SKILL.md").exists());
    }

    #[test]
    fn global_hooks_merge_without_removing_existing_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude/settings.json");
        let codex = tmp.path().join(".codex/hooks.json");
        std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(
            &claude,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"toll hook"},{"type":"command","command":"/usr/local/bin/keel hook pre-tool-use --agent claude"}]}]},"theme":"dark"}"#,
        )
        .unwrap();
        std::fs::write(
            &codex,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-stop"}]}]}}"#,
        )
        .unwrap();

        install_global_hooks_to(&claude, &codex).unwrap();
        install_global_hooks_to(&claude, &codex).unwrap();

        let claude_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&claude).unwrap()).unwrap();
        let codex_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&codex).unwrap()).unwrap();
        assert_eq!(claude_doc["theme"], "dark");
        assert!(claude_doc.to_string().contains("toll hook"));
        assert!(!claude_doc.to_string().contains("/usr/local/bin/keel"));
        assert_eq!(claude_doc.to_string().matches("repovow hook").count(), 6);
        assert!(codex_doc.to_string().contains("my-stop"));
        assert_eq!(codex_doc.to_string().matches("repovow hook").count(), 7);
        assert!(hook_document_contains_all(
            &claude,
            "claude",
            &[
                "PreCompact",
                "SessionStart",
                "PreToolUse",
                "PostToolUse",
                "UserPromptSubmit",
                "Stop"
            ]
        ));
        assert!(hook_document_contains_all(
            &codex,
            "codex",
            &[
                "PreCompact",
                "PostCompact",
                "SessionStart",
                "PreToolUse",
                "PostToolUse",
                "UserPromptSubmit",
                "Stop"
            ]
        ));
    }

    #[test]
    fn codex_hook_hash_matches_reference_vector() {
        let group = json!({
            "matcher": "startup|resume",
            "hooks": [{
                "type": "command",
                "command": "bash ~/.codex/hooks/session_start.sh",
                "statusMessage": "Loading Codex session context",
                "timeout": 10
            }]
        });
        let hash = codex_command_hook_hash("SessionStart", &group, &group["hooks"][0]).unwrap();
        assert_eq!(
            hash,
            "sha256:a7938f11f7510d8a4d841f90f2c1b049f5faf58c7f30ea4682c656ecd02f4a6d"
        );
    }

    #[test]
    fn codex_trust_registration_only_trusts_repovow_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude/settings.json");
        let codex = tmp.path().join(".codex/hooks.json");
        let config = tmp.path().join(".codex/config.toml");
        std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(
            &codex,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-stop"}]}]}}"#,
        )
        .unwrap();
        std::fs::write(
            &config,
            "# preserved comment\nmodel = \"gpt-test\"\n\n[hooks.state.unrelated]\ntrusted_hash = \"sha256:keep\"\n",
        )
        .unwrap();

        install_global_hooks_to(&claude, &codex).unwrap();
        let mut codex_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&codex).unwrap()).unwrap();
        codex_doc["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "hooks": [{
                    "type": "command",
                    "command": "sh -c 'repovow hook stop --agent codex; run-unrelated-command'"
                }]
            }));
        std::fs::write(
            &codex,
            serde_json::to_string_pretty(&codex_doc).unwrap() + "\n",
        )
        .unwrap();
        assert_eq!(trust_codex_repovow_hooks(&codex, &config).unwrap(), 7);
        assert!(codex_repovow_hooks_trusted(&codex, &config).unwrap());

        let raw = std::fs::read_to_string(&config).unwrap();
        assert!(raw.contains("# preserved comment"));
        assert!(raw.contains("model = \"gpt-test\""));
        assert!(raw.contains("trusted_hash = \"sha256:keep\""));
        let parsed = raw.parse::<DocumentMut>().unwrap();
        let state = parsed["hooks"]["state"].as_table().unwrap();
        assert_eq!(state.len(), 8);
        assert_eq!(
            state["unrelated"]["trusted_hash"].as_str(),
            Some("sha256:keep")
        );
        assert!(!state.iter().any(|(key, _)| key.ends_with(":stop:2:0")));
    }
}
