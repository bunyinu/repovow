use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn bin() -> Command {
    let mut command = Command::cargo_bin("repovow").unwrap();
    command.env("REPOVOW_SKIP_GLOBAL_HOOKS", "1");
    command
}

fn init_git_repo(dir: &std::path::Path) {
    fs::create_dir_all(dir.join(".git")).unwrap();
}

#[test]
fn init_installs_hooks_and_repovow_dir() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    bin()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("RepoVow v0."));

    assert!(tmp.path().join(".repovow").is_dir());
    assert!(tmp.path().join(".claude/settings.json").exists());
    assert!(tmp.path().join(".codex/hooks.json").exists());
    assert!(tmp.path().join(".agents/skills/repovow/SKILL.md").exists());
}

#[test]
fn goal_set_writes_snapshot() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin()
        .current_dir(tmp.path())
        .args(["init"])
        .assert()
        .success();

    bin()
        .current_dir(tmp.path())
        .args([
            "goal",
            "set",
            "Add OAuth",
            "--accept",
            "tests pass",
            "--step",
            "scaffold routes",
        ])
        .assert()
        .success();

    let snap = fs::read_to_string(tmp.path().join(".repovow/snapshot.md")).unwrap();
    assert!(snap.contains("Add OAuth"));
    assert!(snap.contains("scaffold routes"));
}

#[test]
fn loop_breaker_blocks_third_bash_attempt() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();

    let fail_payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "npm test"},
        "exit_code": 1,
        "stderr": "tests failed"
    })
    .to_string();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "post-tool-use", "--agent", "claude"])
        .write_stdin(fail_payload.clone());
    cmd.assert().success();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "post-tool-use", "--agent", "claude"])
        .write_stdin(fail_payload);
    cmd.assert().success();

    let pre_payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "npm test"}
    })
    .to_string();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "pre-tool-use", "--agent", "claude"])
        .write_stdin(pre_payload);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("loop breaker"));
}

#[test]
fn session_start_injects_snapshot() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Ship it"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "claude"])
        .write_stdin("{}");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Ship it"));
}

#[test]
fn global_router_auto_bootstraps_a_git_repository() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let mut hook = bin();
    hook.current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "claude"])
        .write_stdin(r#"{"session_id":"automatic-repovow","source":"startup"}"#);
    hook.assert().success().stdout(predicate::str::is_empty());
    assert!(tmp.path().join(".repovow/config.json").exists());
    assert!(!tmp.path().join(".claude").exists());
    assert!(!tmp.path().join(".codex").exists());
}

#[test]
fn global_router_does_not_modify_a_non_git_directory() {
    let tmp = TempDir::new().unwrap();
    let mut hook = bin();
    hook.current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "claude"])
        .write_stdin(r#"{"session_id":"not-a-repository","source":"startup"}"#);
    hook.assert().success().stdout(predicate::str::is_empty());
    assert!(!tmp.path().join(".repovow").exists());
}

#[test]
fn automatic_bootstrap_can_be_disabled() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    let mut hook = bin();
    hook.env("REPOVOW_AUTO_INIT", "0")
        .current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "codex"])
        .write_stdin(r#"{"session_id":"disabled-bootstrap","source":"startup"}"#);
    hook.assert().success().stdout(predicate::str::is_empty());
    assert!(!tmp.path().join(".repovow").exists());
}

#[test]
fn first_prompt_automatically_creates_the_goal() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());

    let mut prompt = bin();
    prompt
        .current_dir(tmp.path())
        .args(["hook", "user-prompt-submit", "--agent", "codex"])
        .write_stdin(
            r#"{"session_id":"automatic-goal","prompt":"Fix checkout and run the tests"}"#,
        );
    prompt
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix checkout and run the tests"));

    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tmp.path().join(".repovow/state.json")).unwrap())
            .unwrap();
    assert_eq!(state["goal"]["title"], "Fix checkout and run the tests");
    assert_eq!(
        state["progress"]["current_step"],
        "Execute and verify; finish with `repovow progress --done \"...\"`"
    );
    assert_eq!(state["goal"]["acceptance"].as_array().unwrap().len(), 2);
}

#[test]
fn duplicate_global_and_project_hook_delivery_is_processed_once() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Deduplicate hooks"])
        .assert()
        .success();

    let payload = r#"{"session_id":"same-host-event","source":"startup"}"#;
    let mut first = bin();
    first
        .current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "codex"])
        .write_stdin(payload);
    first
        .assert()
        .success()
        .stdout(predicate::str::contains("Deduplicate hooks"));

    let mut duplicate = bin();
    duplicate
        .current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "codex"])
        .write_stdin(payload);
    duplicate
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tmp.path().join(".repovow/state.json")).unwrap())
            .unwrap();
    assert_eq!(state["sessions"], 1);
}

#[test]
fn first_prompt_after_mid_session_init_injects_context_once() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Activate without restart"])
        .assert()
        .success();

    let payload = r#"{"session_id":"already-open-session","prompt":"continue"}"#;
    let mut first_prompt = bin();
    first_prompt
        .current_dir(tmp.path())
        .args(["hook", "user-prompt-submit", "--agent", "claude"])
        .write_stdin(payload);
    first_prompt
        .assert()
        .success()
        .stdout(predicate::str::contains("Activate without restart"));

    let mut next_prompt = bin();
    next_prompt
        .current_dir(tmp.path())
        .args(["hook", "user-prompt-submit", "--agent", "claude"])
        .write_stdin(r#"{"session_id":"already-open-session","prompt":"next"}"#);
    next_prompt
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn compact_context_is_injected_once_with_fallback() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Preserve me"])
        .assert()
        .success();

    let mut pre_compact = bin();
    pre_compact
        .current_dir(tmp.path())
        .args(["hook", "pre-compact", "--agent", "claude"])
        .write_stdin(r#"{"trigger":"manual"}"#);
    pre_compact
        .assert()
        .success()
        .stdout(predicate::str::contains("Preserve me"));

    let mut compact_session = bin();
    compact_session
        .current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "claude"])
        .write_stdin(r#"{"source":"compact"}"#);
    compact_session
        .assert()
        .success()
        .stdout(predicate::str::contains("already preserved"))
        .stdout(predicate::str::contains("Preserve me").not());

    let mut fallback_session = bin();
    fallback_session
        .current_dir(tmp.path())
        .args(["hook", "session-start", "--agent", "claude"])
        .write_stdin(r#"{"source":"compact"}"#);
    fallback_session
        .assert()
        .success()
        .stdout(predicate::str::contains("Preserve me"));
}

#[test]
fn context_tracks_working_set_without_per_prompt_tokens() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Targeted reads"])
        .assert()
        .success();

    let payload = json!({
        "tool_name": "Edit",
        "tool_input": {"file_path": "src/context.rs"},
        "tool_result": {"ok": true}
    })
    .to_string();
    let mut post_tool = bin();
    post_tool
        .current_dir(tmp.path())
        .args(["hook", "post-tool-use", "--agent", "claude"])
        .write_stdin(payload);
    post_tool.assert().success();

    bin()
        .current_dir(tmp.path())
        .args(["context", "--stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Working set:"))
        .stdout(predicate::str::contains("src/context.rs"))
        .stderr(predicate::str::contains("saved:"));

    let mut prompt = bin();
    prompt
        .current_dir(tmp.path())
        .args(["hook", "user-prompt-submit", "--agent", "claude"])
        .write_stdin(r#"{"prompt":"continue"}"#);
    prompt.assert().success().stdout(predicate::str::is_empty());
}

#[test]
fn context_can_retrieve_one_quality_section() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args([
            "goal",
            "set",
            "Ship safely",
            "--accept",
            "unit tests pass",
            "security review passes",
            "--constraint",
            "no new dependencies",
        ])
        .assert()
        .success();

    bin()
        .current_dir(tmp.path())
        .args(["context", "--section", "acceptance", "--stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# RepoVow context: acceptance"))
        .stdout(predicate::str::contains("unit tests pass"))
        .stdout(predicate::str::contains("security review passes"))
        .stdout(predicate::str::contains("no new dependencies").not())
        .stderr(predicate::str::contains("Section acceptance:"));
}

#[test]
fn status_shows_goal() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "My task"])
        .assert()
        .success();

    bin()
        .current_dir(tmp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("My task"));
}

#[test]
fn constraint_guard_blocks_npm_install() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "Ship", "--constraint", "no new deps"])
        .assert()
        .success();

    let payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "npm install left-pad"}
    })
    .to_string();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "pre-tool-use", "--agent", "claude"])
        .write_stdin(payload);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("constraint guard"));
}

#[test]
fn acceptance_gate_blocks_stop_on_failure() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["config", "set", "--acceptance", "false"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.current_dir(tmp.path())
        .args(["hook", "stop", "--agent", "claude"])
        .write_stdin("{}");
    cmd.assert()
        .code(2)
        .stdout(predicate::str::contains("acceptance gate failed"));
}

#[test]
fn doctor_passes_after_init() {
    let tmp = TempDir::new().unwrap();
    init_git_repo(tmp.path());
    bin().current_dir(tmp.path()).arg("init").assert().success();
    bin()
        .current_dir(tmp.path())
        .args(["goal", "set", "My task"])
        .assert()
        .success();

    bin()
        .current_dir(tmp.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains(".repovow initialized"));
}
