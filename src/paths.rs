use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::REPOVOW_DIR;

const LEGACY_STATE_DIR: &str = ".keel";

pub const STATE_FILE: &str = "state.json";
pub const CONFIG_FILE: &str = "config.json";
pub const CHANGELOG_FILE: &str = "changelog.jsonl";
pub const ATTEMPTS_FILE: &str = "attempts.jsonl";
pub const SNAPSHOT_FILE: &str = "snapshot.md";
pub const POLICY_PUB_FILE: &str = "policy.pub";
pub const POLICY_KEY_FILE: &str = "policy.key";
pub const POLICY_SIG_FILE: &str = "policy.sig";

pub fn utcnow() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn find_project_root(start: Option<&Path>) -> PathBuf {
    let start = start
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let start = start.canonicalize().unwrap_or(start);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|h| h.canonicalize().ok());

    for dir in start.ancestors() {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        let is_home = home.as_ref().is_some_and(|h| h == dir);
        if dir.join(REPOVOW_DIR).is_dir() || dir.join(LEGACY_STATE_DIR).is_dir() {
            // Ignore ~/.repovow when working in a subdirectory (common mistake after
            // `repovow cloud link` run from $HOME).
            if !is_home {
                return dir.to_path_buf();
            }
        }
        // A project below $HOME cannot belong to a repository above $HOME. This
        // also prevents unrelated markers in /tmp or / from capturing the path.
        if is_home && dir != start {
            break;
        }
    }
    start
}

pub fn repovow_dir(root: Option<&Path>) -> PathBuf {
    let root = find_project_root(root);
    // State migration is intentionally lazy so the first command or hook after
    // an upgrade preserves existing projects without requiring user action.
    let _ = migrate_legacy_state(&root);
    root.join(REPOVOW_DIR)
}

pub fn ensure_repovow_dir(root: Option<&Path>) -> Result<PathBuf> {
    let dir = repovow_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

pub fn migrate_legacy_state(root: &Path) -> Result<bool> {
    let legacy = root.join(LEGACY_STATE_DIR);
    let current = root.join(REPOVOW_DIR);
    if current.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    fs::rename(&legacy, &current).with_context(|| {
        format!(
            "migrate legacy state {} to {}",
            legacy.display(),
            current.display()
        )
    })?;
    Ok(true)
}

pub fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(value)? + "\n";
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_json(path: &Path, default: serde_json::Value) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(default);
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn append_jsonl(path: &Path, record: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

pub fn read_jsonl_tail(path: &Path, limit: usize) -> Result<Vec<serde_json::Value>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(limit);
    let mut out = Vec::new();
    for line in &lines[start..] {
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_json_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let v = serde_json::json!({"a": 1});
        write_json_atomic(&path, &v).unwrap();
        let back = read_json(&path, serde_json::json!(null)).unwrap();
        assert_eq!(back["a"], 1);
    }

    #[test]
    fn ignores_home_repovow_for_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = home.join("myapp");
        std::fs::create_dir_all(home.join(REPOVOW_DIR)).unwrap();
        std::fs::create_dir_all(&project).unwrap();

        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);

        let root = find_project_root(Some(&project));
        assert_eq!(root, project.canonicalize().unwrap());

        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn migrates_legacy_state_on_first_access() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(LEGACY_STATE_DIR)).unwrap();
        std::fs::write(root.join(LEGACY_STATE_DIR).join(STATE_FILE), "{}\n").unwrap();

        let path = repovow_dir(Some(root));

        assert_eq!(path, root.join(REPOVOW_DIR));
        assert!(path.join(STATE_FILE).is_file());
        assert!(!root.join(LEGACY_STATE_DIR).exists());
    }

    #[test]
    fn current_state_wins_when_both_directories_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(LEGACY_STATE_DIR)).unwrap();
        std::fs::create_dir_all(root.join(REPOVOW_DIR)).unwrap();

        assert!(!migrate_legacy_state(root).unwrap());
        assert!(root.join(LEGACY_STATE_DIR).is_dir());
        assert!(root.join(REPOVOW_DIR).is_dir());
    }
}
