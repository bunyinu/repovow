use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::install::repovow_binary;
use crate::paths::{find_project_root, repovow_dir};
use crate::policy;
use crate::state::{load_config, load_state};
use crate::VERSION;

pub struct Check {
    pub ok: bool,
    pub label: String,
    pub detail: String,
}

pub fn run_doctor() -> Result<Vec<Check>> {
    let mut checks = Vec::new();

    checks.push(Check {
        ok: true,
        label: "RepoVow version".into(),
        detail: VERSION.into(),
    });

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = find_project_root(None);
    let repovow_path = repovow_dir(None);
    let has_config = repovow_path.join("config.json").exists();
    let partial_repovow = repovow_path.is_dir() && !has_config;

    let home_repovow = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".repovow"))
        .filter(|p| p.is_dir());
    let cwd_canon = cwd.canonicalize().unwrap_or(cwd.clone());
    let home_canon = std::env::var_os("HOME").and_then(|h| PathBuf::from(h).canonicalize().ok());
    let mislinked_home = home_repovow.is_some()
        && home_canon.as_ref() != Some(&cwd_canon)
        && repovow_path
            == home_canon
                .as_ref()
                .map(|h| h.join(".repovow"))
                .unwrap_or_default()
        && !cwd.join(".repovow").exists()
        && !cwd.join(".git").exists();

    checks.push(Check {
        ok: has_config,
        label: ".repovow initialized".into(),
        detail: if has_config {
            format!("{}", repovow_path.display())
        } else if partial_repovow {
            format!(
                "partial .repovow at {} — run `repovow onboard \"...\"` or `repovow init`",
                repovow_path.display()
            )
        } else {
            "Run `repovow onboard \"your task\" --accept \"tests pass\"`".into()
        },
    });

    checks.push(Check {
        ok: !mislinked_home,
        label: "Project root".into(),
        detail: if mislinked_home {
            "Looks like ~/.repovow is being used — run `cd your-repo` then `repovow init`".into()
        } else {
            format!("{}", root.display())
        },
    });

    let codex_path = root.join(".codex");
    let codex_ok = !codex_path.exists() || codex_path.is_dir();
    checks.push(Check {
        ok: codex_ok,
        label: ".codex directory".into(),
        detail: if codex_ok {
            "OK".into()
        } else {
            format!(
                "{} is a file — rename it so Codex hooks can install",
                codex_path.display()
            )
        },
    });

    let claude_hooks = root.join(".claude/settings.json");
    let (claude_ok, claude_detail) = hooks_contain_repovow(&claude_hooks);
    checks.push(Check {
        ok: claude_ok,
        label: "Claude Code hooks".into(),
        detail: claude_detail,
    });

    let codex_hooks = root.join(".codex/hooks.json");
    let (codex_ok, codex_detail) = hooks_contain_repovow(&codex_hooks);
    checks.push(Check {
        ok: codex_ok,
        label: "Codex hooks".into(),
        detail: codex_detail,
    });

    let cursor_hooks = root.join(".cursor/hooks.json");
    let (cursor_ok, cursor_detail) = hooks_contain_repovow_cursor(&cursor_hooks);
    checks.push(Check {
        ok: cursor_ok,
        label: "Cursor hooks".into(),
        detail: cursor_detail,
    });

    if crate::env_var("REPOVOW_SKIP_GLOBAL_HOOKS").as_deref() != Ok("1") {
        let statuses = crate::install::global_hooks_status().unwrap_or_default();
        for (agent, path, installed, active) in statuses {
            checks.push(Check {
                ok: installed && active,
                label: format!("{agent} persistent router"),
                detail: if installed && active {
                    path.display().to_string()
                } else if installed {
                    format!(
                        "installed at {} but trust is missing — run `repovow agents install`",
                        path.display()
                    )
                } else {
                    format!(
                        "missing at {} — run `repovow agents install`",
                        path.display()
                    )
                },
            });
        }
    }

    let agent_skill = root.join(".agents/skills/repovow/SKILL.md");
    let skill_ok = agent_skill.exists()
        && std::fs::read_to_string(&agent_skill)
            .map(|text| text.contains("repovow context --section NAME"))
            .unwrap_or(false);
    checks.push(Check {
        ok: skill_ok,
        label: "Agent workflow skill".into(),
        detail: if skill_ok {
            agent_skill.display().to_string()
        } else {
            "missing or stale — run `repovow init`".into()
        },
    });

    let hooks_installed = claude_ok || codex_ok || cursor_ok;
    let cloud_ok = repovow_path.join("cloud.json").exists();
    checks.push(Check {
        ok: true,
        label: "RepoVow Cloud link".into(),
        detail: if cloud_ok {
            "cloud.json present".into()
        } else {
            "optional — `repovow cloud link ...`".into()
        },
    });

    let state = load_state(None).ok();
    let has_goal = state
        .as_ref()
        .and_then(|s| s.goal.as_ref())
        .is_some_and(|g| !g.title.trim().is_empty());
    checks.push(Check {
        ok: !hooks_installed || has_goal,
        label: "Active goal".into(),
        detail: if has_goal {
            state
                .as_ref()
                .and_then(|s| s.goal.as_ref())
                .map(|g| g.title.clone())
                .unwrap_or_default()
        } else if hooks_installed {
            "required when hooks are installed — run `repovow onboard \"...\"`".into()
        } else {
            "optional — `repovow onboard \"...\"` or `repovow tui`".into()
        },
    });

    let config = load_config(None).ok();
    let gate = config.as_ref().map(|c| &c.acceptance_gate);
    let gate_on = gate.is_some_and(|g| g.enabled && !g.command.trim().is_empty());
    checks.push(Check {
        ok: true,
        label: "Acceptance gate".into(),
        detail: if gate_on {
            format!("enabled: `{}`", gate.unwrap().command)
        } else {
            "off — `repovow config set --acceptance \"npm test\"`".into()
        },
    });

    let expected_bin = repovow_binary();
    checks.push(Check {
        ok: true,
        label: "Hook binary".into(),
        detail: expected_bin,
    });

    let (policy_ok, policy_detail) = policy::doctor_detail(None);
    checks.push(Check {
        ok: policy_ok,
        label: "Signed policy".into(),
        detail: policy_detail,
    });

    Ok(checks)
}

fn hooks_contain_repovow_cursor(path: &Path) -> (bool, String) {
    if !path.exists() {
        return (
            false,
            format!("missing {} — run `repovow init`", path.display()),
        );
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return (false, e.to_string()),
    };
    if raw.contains("repovow hook") {
        (true, path.display().to_string())
    } else {
        (false, "no repovow hooks — run `repovow init`".into())
    }
}

fn hooks_contain_repovow(path: &Path) -> (bool, String) {
    if !path.exists() {
        return (false, format!("missing {}", path.display()));
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return (false, e.to_string()),
    };
    let doc: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return (false, format!("invalid JSON: {e}")),
    };
    let text = doc.to_string();
    if text.contains("repovow hook") {
        (true, path.display().to_string())
    } else {
        (false, "no repovow hooks — run `repovow init`".into())
    }
}

pub fn print_report(checks: &[Check]) -> bool {
    let mut all_ok = true;
    for c in checks {
        let icon = if c.ok { "✓" } else { "✗" };
        if !c.ok {
            all_ok = false;
        }
        println!("{icon} {} — {}", c.label, c.detail);
    }
    all_ok
}
