use anyhow::{Context, Result};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

pub const PLAN_FREE: &str = "free";
pub const PLAN_PRO: &str = "pro";
pub const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub plan: String,
    pub license_key: String,
    pub max_projects: i32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub team_id: String,
    pub state_json: String,
    pub snapshot_md: String,
    pub config_json: String,
    pub changelog_jsonl: String,
    pub policy_json: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedProjectSummary {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub dashboard_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_label: Option<String>,
    pub compactions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub dashboard_url: String,
    /// Active goal title from synced state (empty if unset).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    pub compactions: u32,
}

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

pub fn free_project_limit() -> i32 {
    crate::env_var("REPOVOW_FREE_PROJECT_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

pub fn pro_project_limit() -> i32 {
    crate::env_var("REPOVOW_PRO_PROJECT_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
}

pub fn init_db(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create database directory {}", parent.display()))?;
    }
    migrate_legacy_database(path)?;

    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=8 {
        match try_init_db(path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("repovow-server: database init attempt {attempt}/8 failed: {e:#}");
                last_err = Some(e);
                if path.exists() && attempt >= 3 {
                    if let Err(be) = backup_unreadable_db(path) {
                        eprintln!("repovow-server: could not rotate database file: {be:#}");
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(400 * attempt as u64));
            }
        }
    }
    Err(last_err.unwrap()).context(format!("init database at {}", path.display()))
}

fn migrate_legacy_database(path: &Path) -> Result<bool> {
    if path.exists() || path.file_name().and_then(|name| name.to_str()) != Some("repovow.db") {
        return Ok(false);
    }
    let legacy = path.with_file_name("keel.db");
    if !legacy.is_file() {
        return Ok(false);
    }

    std::fs::rename(&legacy, path).with_context(|| {
        format!(
            "migrate legacy database {} to {}",
            legacy.display(),
            path.display()
        )
    })?;
    for suffix in ["-wal", "-shm"] {
        let legacy_sidecar = PathBuf::from(format!("{}{suffix}", legacy.display()));
        if legacy_sidecar.exists() {
            let current_sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
            std::fs::rename(&legacy_sidecar, &current_sidecar).with_context(|| {
                format!(
                    "migrate legacy database sidecar {} to {}",
                    legacy_sidecar.display(),
                    current_sidecar.display()
                )
            })?;
        }
    }
    Ok(true)
}

fn backup_unreadable_db(path: &Path) -> Result<()> {
    let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let backup = PathBuf::from(format!("{}.bak.{ts}", path.display()));
    std::fs::rename(path, &backup)
        .with_context(|| format!("rename {} to {}", path.display(), backup.display()))?;
    eprintln!(
        "repovow-server: moved unreadable database to {}",
        backup.display()
    );
    Ok(())
}

fn try_init_db(path: &Path) -> Result<()> {
    let conn = Connection::open(path).context("open database")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS teams (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            plan TEXT NOT NULL DEFAULT 'free',
            license_key TEXT NOT NULL UNIQUE,
            max_projects INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            api_key TEXT NOT NULL UNIQUE,
            team_id TEXT,
            state_json TEXT NOT NULL DEFAULT '{}',
            snapshot_md TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_projects_api_key ON projects(api_key);
        CREATE INDEX IF NOT EXISTS idx_projects_team_id ON projects(team_id);
        CREATE INDEX IF NOT EXISTS idx_teams_license ON teams(license_key);
        CREATE TABLE IF NOT EXISTS browser_sessions (
            token_hash TEXT PRIMARY KEY,
            team_id TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_browser_sessions_team_id ON browser_sessions(team_id);
        CREATE INDEX IF NOT EXISTS idx_browser_sessions_expires_at ON browser_sessions(expires_at);",
    )?;
    migrate_schema(&conn)?;
    migrate_orphan_projects_conn(&conn)?;
    DB.set(Mutex::new(conn))
        .map_err(|_| anyhow::anyhow!("database already initialized"))?;
    Ok(())
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    ensure_column(conn, "projects", "team_id", "team_id TEXT")?;
    ensure_column(conn, "teams", "email", "email TEXT")?;
    ensure_column(
        conn,
        "projects",
        "config_json",
        "config_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    ensure_column(
        conn,
        "projects",
        "changelog_jsonl",
        "changelog_jsonl TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "projects",
        "policy_json",
        "policy_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, col: &str, ddl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if cols.iter().any(|c| c == col) {
        return Ok(());
    }
    if col == "team_id" {
        conn.execute("ALTER TABLE projects ADD COLUMN team_id TEXT", [])?;
    } else {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {ddl}"), [])?;
    }
    Ok(())
}

fn migrate_orphan_projects_conn(conn: &Connection) -> Result<()> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM projects WHERE team_id IS NULL OR team_id = ''")?;
    let orphans: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (pid, pname) in orphans {
        let team =
            create_team_internal_on_conn(conn, &pname, None, PLAN_FREE, free_project_limit())?;
        conn.execute(
            "UPDATE projects SET team_id = ?1 WHERE id = ?2",
            params![team.id, pid],
        )?;
    }
    Ok(())
}

fn conn() -> Result<std::sync::MutexGuard<'static, Connection>> {
    DB.get()
        .context("database not initialized")?
        .lock()
        .map_err(|e| anyhow::anyhow!("db lock poisoned: {e}"))
}

fn new_api_key() -> String {
    format!("repovow_{}", Uuid::new_v4().simple())
}

fn new_team_license() -> String {
    format!("repovow_team_{}", Uuid::new_v4().simple())
}

fn random_session_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn session_token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn create_browser_session(team_id: &str) -> Result<String> {
    let token = random_session_token();
    let token_hash = session_token_hash(&token);
    let now = chrono::Utc::now();
    let expires_at = now.timestamp() + SESSION_TTL_SECS;
    let c = conn()?;
    c.execute(
        "DELETE FROM browser_sessions WHERE expires_at <= ?1",
        params![now.timestamp()],
    )?;
    c.execute(
        "INSERT INTO browser_sessions (token_hash, team_id, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![token_hash, team_id, expires_at, now.to_rfc3339()],
    )?;
    Ok(token)
}

pub fn get_team_by_session(token: &str) -> Result<Option<Team>> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let token_hash = session_token_hash(token);
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT t.id, t.name, t.email, t.plan, t.license_key, t.max_projects, t.created_at
         FROM browser_sessions s
         JOIN teams t ON t.id = s.team_id
         WHERE s.token_hash = ?1 AND s.expires_at > ?2",
    )?;
    let mut rows = stmt.query(params![token_hash, chrono::Utc::now().timestamp()])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(team_from_row(row)?));
    }
    Ok(None)
}

pub fn revoke_browser_session(token: &str) -> Result<()> {
    let token_hash = session_token_hash(token);
    let c = conn()?;
    c.execute(
        "DELETE FROM browser_sessions WHERE token_hash = ?1",
        params![token_hash],
    )?;
    Ok(())
}

fn create_team_internal(
    name: &str,
    email: Option<&str>,
    plan: &str,
    max_projects: i32,
) -> Result<Team> {
    let c = conn()?;
    create_team_internal_on_conn(&c, name, email, plan, max_projects)
}

fn create_team_internal_on_conn(
    conn: &Connection,
    name: &str,
    email: Option<&str>,
    plan: &str,
    max_projects: i32,
) -> Result<Team> {
    let id = Uuid::new_v4().to_string();
    let license_key = new_team_license();
    let now = chrono::Utc::now().to_rfc3339();
    let email = email.map(str::trim).filter(|s| !s.is_empty());
    conn.execute(
        "INSERT INTO teams (id, name, email, plan, license_key, max_projects, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, name, email, plan, license_key, max_projects, now],
    )?;
    Ok(Team {
        id,
        name: name.to_string(),
        email: email.map(|s| s.to_string()),
        plan: plan.to_string(),
        license_key,
        max_projects,
        created_at: now,
    })
}

pub fn create_team(name: &str, email: Option<&str>) -> Result<Team> {
    create_team_internal(name, email, PLAN_FREE, free_project_limit())
}

pub fn get_team_by_email_and_license(email: &str, license_key: &str) -> Result<Option<Team>> {
    let email = email.trim();
    let license_key = license_key.trim();
    if let Some(team) = get_team_by_license(license_key)? {
        if let Some(stored) = team.email.as_deref() {
            if stored.eq_ignore_ascii_case(email) {
                return Ok(Some(team));
            }
        } else {
            // Legacy accounts created before email was stored.
            return Ok(Some(team));
        }
    }
    Ok(None)
}

fn team_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Team> {
    Ok(Team {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        plan: row.get(3)?,
        license_key: row.get(4)?,
        max_projects: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        api_key: row.get(2)?,
        team_id: row.get(3)?,
        state_json: row.get(4)?,
        snapshot_md: row.get(5)?,
        config_json: row.get(6)?,
        changelog_jsonl: row.get(7)?,
        policy_json: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn policy_label_from_json(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.get("label")
                .and_then(|l| l.as_str())
                .map(|s| s.to_string())
        })
}

pub fn get_team_by_license(license_key: &str) -> Result<Option<Team>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, email, plan, license_key, max_projects, created_at FROM teams WHERE license_key = ?1",
    )?;
    let mut rows = stmt.query(params![license_key])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(team_from_row(row)?));
    }
    Ok(None)
}

pub fn get_team_by_id(id: &str) -> Result<Option<Team>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, email, plan, license_key, max_projects, created_at FROM teams WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(team_from_row(row)?));
    }
    Ok(None)
}

pub fn count_team_projects(team_id: &str) -> Result<i32> {
    let c = conn()?;
    let n: i32 = c.query_row(
        "SELECT COUNT(*) FROM projects WHERE team_id = ?1",
        params![team_id],
        |row| row.get(0),
    )?;
    Ok(n)
}

pub fn upgrade_team_to_pro(team_license: &str) -> Result<Team> {
    let team =
        get_team_by_license(team_license)?.ok_or_else(|| anyhow::anyhow!("team not found"))?;
    upgrade_team_to_pro_by_id(&team.id)
}

pub fn upgrade_team_to_pro_by_id(team_id: &str) -> Result<Team> {
    let team = get_team_by_id(team_id)?.ok_or_else(|| anyhow::anyhow!("team not found"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let max = pro_project_limit();
    let c = conn()?;
    c.execute(
        "UPDATE teams SET plan = ?1, max_projects = ?2 WHERE id = ?3",
        params![PLAN_PRO, max, team.id],
    )?;
    Ok(Team {
        plan: PLAN_PRO.into(),
        max_projects: max,
        created_at: now,
        ..team
    })
}

pub fn create_project(name: &str, team_license: Option<&str>) -> Result<Project> {
    let team = if let Some(key) = team_license {
        get_team_by_license(key)?.ok_or_else(|| anyhow::anyhow!("invalid team license"))?
    } else {
        create_team(name, None)?
    };
    create_project_for_existing_team(name, &team)
}

pub fn create_project_for_team(name: &str, team_id: &str) -> Result<Project> {
    let team = get_team_by_id(team_id)?.ok_or_else(|| anyhow::anyhow!("account not found"))?;
    create_project_for_existing_team(name, &team)
}

fn create_project_for_existing_team(name: &str, team: &Team) -> Result<Project> {
    let count = count_team_projects(&team.id)?;
    if count >= team.max_projects {
        anyhow::bail!(
            "project limit reached for {} plan ({})",
            team.plan,
            team.max_projects
        );
    }
    let id = Uuid::new_v4().to_string();
    let api_key = new_api_key();
    let now = chrono::Utc::now().to_rfc3339();
    let c = conn()?;
    c.execute(
        "INSERT INTO projects (id, name, api_key, team_id, state_json, snapshot_md, config_json, changelog_jsonl, policy_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, '{}', '', '{}', '', '{}', ?5)",
        params![id, name, api_key, team.id, now],
    )?;
    Ok(Project {
        id,
        name: name.to_string(),
        api_key,
        team_id: team.id.clone(),
        state_json: "{}".into(),
        snapshot_md: String::new(),
        config_json: "{}".into(),
        changelog_jsonl: String::new(),
        policy_json: "{}".into(),
        updated_at: now,
    })
}

pub fn list_team_projects(team_id: &str) -> Result<Vec<ProjectSummary>> {
    list_team_projects_inner(team_id, false)
}

/// Project rows for an authenticated account owner. Access keys are never listed.
pub fn list_team_projects_owned(team_id: &str) -> Result<Vec<OwnedProjectSummary>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, updated_at, state_json, policy_json FROM projects WHERE team_id = ?1 ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(params![team_id], |row| {
        let id: String = row.get(0)?;
        let state_json: String = row.get(3)?;
        let policy_json: String = row.get(4)?;
        let (goal_title, current_step, compactions) = fleet_fields_from_state(&state_json);
        Ok(OwnedProjectSummary {
            id: id.clone(),
            name: row.get(1)?,
            updated_at: row.get(2)?,
            dashboard_url: format!("/dashboard/{id}"),
            goal_title,
            current_step,
            policy_label: policy_label_from_json(&policy_json),
            compactions,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn list_team_projects_inner(team_id: &str, _owned: bool) -> Result<Vec<ProjectSummary>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, updated_at, state_json FROM projects WHERE team_id = ?1 ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(params![team_id], |row| {
        let id: String = row.get(0)?;
        let state_json: String = row.get(3)?;
        let (goal_title, current_step, compactions) = fleet_fields_from_state(&state_json);
        Ok(ProjectSummary {
            id: id.clone(),
            name: row.get(1)?,
            updated_at: row.get(2)?,
            dashboard_url: format!("/dashboard/{id}"),
            goal_title,
            current_step,
            compactions,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn fleet_fields_from_state(state_json: &str) -> (Option<String>, Option<String>, u32) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(state_json) else {
        return (None, None, 0);
    };
    let goal_title = v
        .pointer("/goal/title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let current_step = v
        .pointer("/progress/current_step")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let compactions = v.get("compactions").and_then(|c| c.as_u64()).unwrap_or(0) as u32;
    (goal_title, current_step, compactions)
}

pub fn link_project_to_team(project_id: &str, api_key: &str, team_id: &str) -> Result<Project> {
    let team = get_team_by_id(team_id)?.ok_or_else(|| anyhow::anyhow!("invalid account"))?;
    let project = get_by_id(project_id)?.ok_or_else(|| anyhow::anyhow!("project not found"))?;
    if project.api_key != api_key {
        anyhow::bail!("invalid project access key");
    }
    if !project.team_id.is_empty() && project.team_id != team.id {
        anyhow::bail!("project belongs to another account");
    }
    if project.team_id != team.id {
        let count = count_team_projects(&team.id)?;
        if count >= team.max_projects {
            anyhow::bail!(
                "project limit reached for {} plan ({})",
                team.plan,
                team.max_projects
            );
        }
        let c = conn()?;
        c.execute(
            "UPDATE projects SET team_id = ?1 WHERE id = ?2",
            params![team.id, project_id],
        )?;
    }
    get_by_id(project_id)?.ok_or_else(|| anyhow::anyhow!("project not found after link"))
}

pub fn get_by_api_key(api_key: &str) -> Result<Option<Project>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, api_key, team_id, state_json, snapshot_md, config_json, changelog_jsonl, policy_json, updated_at
         FROM projects WHERE api_key = ?1",
    )?;
    let mut rows = stmt.query(params![api_key])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(project_from_row(row)?));
    }
    Ok(None)
}

pub fn get_by_id(id: &str) -> Result<Option<Project>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, api_key, team_id, state_json, snapshot_md, config_json, changelog_jsonl, policy_json, updated_at
         FROM projects WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(project_from_row(row)?));
    }
    Ok(None)
}

pub fn sync_project(
    id: &str,
    state_json: &str,
    snapshot_md: &str,
    config_json: &str,
    changelog_jsonl: &str,
    policy_json: &str,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let c = conn()?;
    let n = c.execute(
        "UPDATE projects SET state_json = ?1, snapshot_md = ?2, config_json = ?3, changelog_jsonl = ?4, policy_json = ?5, updated_at = ?6 WHERE id = ?7",
        params![
            state_json,
            snapshot_md,
            config_json,
            changelog_jsonl,
            policy_json,
            now,
            id
        ],
    )?;
    if n == 0 {
        anyhow::bail!("project not found");
    }
    Ok(())
}

pub fn list_projects(limit: usize) -> Result<Vec<Project>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, name, api_key, team_id, state_json, snapshot_md, config_json, changelog_jsonl, policy_json, updated_at
         FROM projects ORDER BY updated_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], project_from_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn valid_upgrade_code(code: &str) -> bool {
    let configured = crate::env_var("REPOVOW_UPGRADE_CODES").ok();
    valid_upgrade_code_from_config(code, configured.as_deref())
}

fn valid_upgrade_code_from_config(code: &str, configured: Option<&str>) -> bool {
    let code = code.trim();
    if code.is_empty() {
        return false;
    }
    configured.is_some_and(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .any(|candidate| candidate == code)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_codes_fail_closed_without_configuration() {
        assert!(!valid_upgrade_code_from_config(
            "repovow_pro_anything",
            None
        ));
        assert!(!valid_upgrade_code_from_config("", Some("expected")));
        assert!(!valid_upgrade_code_from_config(
            "wrong",
            Some("expected,other")
        ));
        assert!(valid_upgrade_code_from_config(
            "expected",
            Some(" expected, other ")
        ));
    }

    #[test]
    fn migrates_legacy_database_and_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("keel.db");
        let current = tmp.path().join("repovow.db");
        std::fs::write(&legacy, b"database").unwrap();
        std::fs::write(tmp.path().join("keel.db-wal"), b"wal").unwrap();

        assert!(migrate_legacy_database(&current).unwrap());
        assert_eq!(std::fs::read(&current).unwrap(), b"database");
        assert_eq!(
            std::fs::read(tmp.path().join("repovow.db-wal")).unwrap(),
            b"wal"
        );
        assert!(!legacy.exists());
    }
}
