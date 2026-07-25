pub mod db;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};

use crate::goal_edit::{apply_form, GoalForm};
use crate::snapshot::render_from_parts;
use crate::state::{RepoVowConfig, RepoVowState};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use self::db::{
    count_team_projects, create_browser_session, create_project, create_project_for_team,
    create_team, get_by_api_key, get_by_id, get_team_by_email_and_license, get_team_by_id,
    get_team_by_license, get_team_by_session, link_project_to_team, list_team_projects_owned,
    revoke_browser_session, sync_project, upgrade_team_to_pro_by_id, valid_upgrade_code, Project,
    Team, SESSION_TTL_SECS,
};

const SESSION_COOKIE: &str = "repovow_session";
const LEGACY_SESSION_COOKIE: &str = "keel_session";

#[derive(Clone)]
pub struct AppState {
    pub version: String,
    pub stripe_payment_link: String,
    /// When set, new account creation requires a user-supplied signup code.
    pub create_secret: Option<String>,
    pub secure_cookies: bool,
}

#[derive(Deserialize)]
pub struct CreateTeamRequest {
    name: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    account_key: String,
}

#[derive(Serialize)]
pub struct CreateTeamResponse {
    id: String,
    name: String,
    account_key: String,
    plan: String,
    max_projects: i32,
}

#[derive(Deserialize)]
pub struct LinkProjectRequest {
    project_id: String,
    api_key: String,
}

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    name: String,
    #[serde(default)]
    team_license: Option<String>,
}

#[derive(Serialize)]
pub struct CreateProjectResponse {
    id: String,
    name: String,
    api_key: String,
    dashboard_url: String,
    team_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_license: Option<String>,
    plan: String,
    projects_used: i32,
    projects_max: i32,
}

#[derive(Deserialize)]
pub struct UpgradeRequest {
    code: String,
}

#[derive(Serialize)]
pub struct TeamView {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    plan: String,
    max_projects: i32,
}

#[derive(Deserialize)]
pub struct SyncRequest {
    state: Value,
    snapshot: String,
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    changelog: Option<String>,
    #[serde(default)]
    policy: Option<Value>,
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .collect::<Vec<_>>();
    [SESSION_COOKIE, LEGACY_SESSION_COOKIE]
        .into_iter()
        .find_map(|expected| {
            cookies
                .iter()
                .find_map(|(name, value)| (*name == expected).then(|| (*value).to_string()))
        })
}

fn team_from_session(headers: &HeaderMap) -> Result<Option<Team>, StatusCode> {
    let Some(token) = extract_session_token(headers) else {
        return Ok(None);
    };
    get_team_by_session(&token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn auth_team(headers: &HeaderMap) -> Result<Team, StatusCode> {
    if let Some(key) = extract_bearer(headers) {
        return get_team_by_license(&key)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::UNAUTHORIZED);
    }
    team_from_session(headers)?.ok_or(StatusCode::UNAUTHORIZED)
}

fn auth_project(headers: &HeaderMap, project_id: &str) -> Result<Project, StatusCode> {
    if let Some(key) = extract_bearer(headers) {
        if let Some(project) =
            get_by_api_key(&key).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            return if project.id == project_id {
                Ok(project)
            } else {
                Err(StatusCode::FORBIDDEN)
            };
        }
        if let Some(team) =
            get_team_by_license(&key).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            let project = get_by_id(project_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            return match project {
                Some(project) if project.team_id == team.id => Ok(project),
                Some(_) => Err(StatusCode::FORBIDDEN),
                None => Err(StatusCode::NOT_FOUND),
            };
        }
        return Err(StatusCode::UNAUTHORIZED);
    }

    let team = team_from_session(headers)?.ok_or(StatusCode::UNAUTHORIZED)?;
    let project = get_by_id(project_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if project.team_id != team.id {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(project)
}

fn session_cookie(token: &str, secure: bool) -> Result<HeaderValue, StatusCode> {
    let secure = if secure { "; Secure" } else { "" };
    let value = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECS}{secure}"
    );
    let mut header =
        HeaderValue::from_str(&value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    header.set_sensitive(true);
    Ok(header)
}

fn expired_session_cookie(secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}"
    ))
    .expect("static session cookie attributes are valid")
}

fn with_browser_session(
    state: &AppState,
    team_id: &str,
    mut response: Response,
) -> Result<Response, StatusCode> {
    let token = create_browser_session(team_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token, state.secure_cookies)?,
    );
    Ok(no_store(response))
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "service": "repovow-cloud",
        "version": state.version,
    }))
}

fn create_secret_ok(state: &AppState, headers: &HeaderMap) -> bool {
    match &state.create_secret {
        None => true,
        Some(expected) => headers
            .get("x-repovow-create-secret")
            .or_else(|| headers.get("x-keel-create-secret"))
            .and_then(|v| v.to_str().ok())
            .is_some_and(|got| got == expected),
    }
}

async fn create_team_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateTeamRequest>,
) -> Result<Response, StatusCode> {
    if !create_secret_ok(&state, &headers) {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Account creation is not allowed from this client"})),
        )
            .into_response());
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Account name is required"})),
        )
            .into_response());
    }
    let team =
        create_team(name, body.email.as_deref()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let team_id = team.id.clone();
    let response = Json(CreateTeamResponse {
        id: team.id,
        name: team.name,
        account_key: team.license_key,
        plan: team.plan,
        max_projects: team.max_projects,
    })
    .into_response();
    with_browser_session(&state, &team_id, response)
}

async fn create_project_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateProjectRequest>,
) -> Result<Response, StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Project name is required"})),
        )
            .into_response());
    }
    let session_team = team_from_session(&headers)?;
    let is_admin_create = session_team.is_none();
    if is_admin_create && (state.create_secret.is_none() || !create_secret_ok(&state, &headers)) {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "An authenticated account is required"})),
        )
            .into_response());
    }
    let project_result = match &session_team {
        Some(team) => create_project_for_team(name, &team.id),
        None => create_project(name, body.team_license.as_deref()),
    };
    let project = match project_result {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("project limit") {
                return Ok((
                    StatusCode::PAYMENT_REQUIRED,
                    Json(json!({
                        "error": msg,
                        "upgrade_url": "/pricing",
                        "stripe_url": state.stripe_payment_link,
                    })),
                )
                    .into_response());
            }
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let team = get_team_by_id(&project.team_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let used = count_team_projects(&team.id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(no_store(
        Json(CreateProjectResponse {
            id: project.id.clone(),
            name: project.name,
            api_key: project.api_key,
            dashboard_url: format!("/dashboard/{}", project.id),
            team_id: team.id,
            team_license: is_admin_create.then_some(team.license_key),
            plan: team.plan,
            projects_used: used,
            projects_max: team.max_projects,
        })
        .into_response(),
    ))
}

async fn upgrade_handler(
    headers: HeaderMap,
    Json(body): Json<UpgradeRequest>,
) -> Result<Json<Value>, StatusCode> {
    let account = auth_team(&headers)?;
    if !valid_upgrade_code(&body.code) {
        return Err(StatusCode::FORBIDDEN);
    }
    let team = upgrade_team_to_pro_by_id(&account.id).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(json!({
        "ok": true,
        "team": team_json(&team),
    })))
}

async fn team_projects_handler(headers: HeaderMap) -> Result<Response, StatusCode> {
    let team = auth_team(&headers)?;
    let projects =
        list_team_projects_owned(&team.id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(no_store(
        Json(json!({
            "team": team_json(&team),
            "projects": projects,
        }))
        .into_response(),
    ))
}

async fn link_project_handler(
    headers: HeaderMap,
    Json(body): Json<LinkProjectRequest>,
) -> Result<Response, StatusCode> {
    let team = auth_team(&headers)?;
    let project_id = body.project_id.trim();
    let api_key = body.api_key.trim();
    if project_id.is_empty() || api_key.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Project ID and access key are required"})),
        )
            .into_response());
    }
    let project = match link_project_to_team(project_id, api_key, &team.id) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("project limit") {
                return Ok((
                    StatusCode::PAYMENT_REQUIRED,
                    Json(json!({"error": msg, "upgrade_url": "/pricing"})),
                )
                    .into_response());
            }
            if msg.contains("not found")
                || msg.contains("invalid")
                || msg.contains("another account")
            {
                return Ok((StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response());
            }
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    Ok(no_store(
        Json(json!({
            "ok": true,
            "project": {
                "id": project.id,
                "name": project.name,
                "dashboard_url": format!("/dashboard/{}", project.id),
            }
        }))
        .into_response(),
    ))
}

fn team_json(team: &Team) -> TeamView {
    TeamView {
        id: team.id.clone(),
        name: team.name.clone(),
        email: team.email.clone(),
        plan: team.plan.clone(),
        max_projects: team.max_projects,
    }
}

async fn login_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, StatusCode> {
    let email = body.email.trim();
    let key = body.account_key.trim();
    if email.is_empty() || key.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Email and account key are required"})),
        )
            .into_response());
    }
    let team =
        get_team_by_email_and_license(email, key).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some(team) = team else {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid email or account key"})),
        )
            .into_response());
    };
    let response = Json(json!({ "team": team_json(&team) })).into_response();
    with_browser_session(&state, &team.id, response)
}

async fn session_handler(headers: HeaderMap) -> Result<Response, StatusCode> {
    let team = auth_team(&headers)?;
    Ok(no_store(
        Json(json!({ "team": team_json(&team) })).into_response(),
    ))
}

async fn logout_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    if let Some(token) = extract_session_token(&headers) {
        revoke_browser_session(&token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        expired_session_cookie(state.secure_cookies),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn parse_changelog(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

async fn pricing_page(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let mut html = include_str!("../../web/pricing.html").to_string();
    let link = html_escape(&state.stripe_payment_link);
    html = html.replace(
        "window.REPOVOW_STRIPE_PAYMENT_LINK || stripeDefault",
        &format!("\"{link}\" || stripeDefault"),
    );
    html_no_cache(html)
}

async fn get_project(Path(id): Path<String>, headers: HeaderMap) -> Result<Response, StatusCode> {
    let project = auth_project(&headers, &id)?;
    let state: Value = serde_json::from_str(&project.state_json).unwrap_or(json!({}));
    let config: Value = serde_json::from_str(&project.config_json).unwrap_or(json!({}));
    let policy: Value = serde_json::from_str(&project.policy_json).unwrap_or(json!({}));
    let changelog = parse_changelog(&project.changelog_jsonl);
    Ok(no_store(
        Json(json!({
            "id": project.id,
            "name": project.name,
            "state": state,
            "config": config,
            "policy": policy,
            "changelog": changelog,
            "snapshot": project.snapshot_md,
            "updated_at": project.updated_at,
        }))
        .into_response(),
    ))
}

async fn sync_handler(
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SyncRequest>,
) -> Result<StatusCode, StatusCode> {
    let _project = auth_project(&headers, &id)?;
    let state_json = serde_json::to_string(&body.state).map_err(|_| StatusCode::BAD_REQUEST)?;
    let config_json = serde_json::to_string(&body.config.clone().unwrap_or(json!({})))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let changelog_jsonl = body.changelog.clone().unwrap_or_default();
    let policy_json = serde_json::to_string(&body.policy.clone().unwrap_or(json!({})))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    sync_project(
        &id,
        &state_json,
        &body.snapshot,
        &config_json,
        &changelog_jsonl,
        &policy_json,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct GoalResponse {
    snapshot: String,
    state: Value,
}

async fn update_goal_handler(
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(form): Json<GoalForm>,
) -> Result<Response, StatusCode> {
    if form.title.trim().is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Goal title is required"})),
        )
            .into_response());
    }
    let project = auth_project(&headers, &id)?;
    let mut state: RepoVowState = serde_json::from_str(&project.state_json).unwrap_or_default();
    apply_form(&mut state, &form);
    let snapshot = render_from_parts(&state, &RepoVowConfig::default(), &[]);
    let state_json =
        serde_json::to_string(&state).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sync_project(&id, &state_json, &snapshot, "{}", "", "{}")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let state_value: Value = serde_json::from_str(&state_json).unwrap_or(json!({}));
    Ok(no_store(
        Json(GoalResponse {
            snapshot,
            state: state_value,
        })
        .into_response(),
    ))
}

async fn dashboard_edit(Path(id): Path<String>) -> Result<Response, StatusCode> {
    dashboard_page(include_str!("../../web/dashboard-edit.html"), &id)
}

async fn dashboard(Path(id): Path<String>) -> Result<Response, StatusCode> {
    dashboard_page(include_str!("../../web/dashboard.html"), &id)
}

fn dashboard_page(template: &str, id: &str) -> Result<Response, StatusCode> {
    if get_by_id(id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(html_no_cache(
        template.replace("__PROJECT_ID__", &html_escape(id)),
    ))
}

async fn home() -> Response {
    html_static_no_cache(include_str!("../../web/index.html"))
}

async fn account_page() -> Response {
    html_static_no_cache(include_str!("../../web/account.html"))
}

async fn team_redirect() -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/account")]).into_response()
}

async fn start_page() -> Response {
    html_static_no_cache(include_str!("../../web/start.html"))
}

async fn login_redirect(request: axum::extract::Request) -> impl IntoResponse {
    let query = request.uri().query().unwrap_or("");
    redirect_to_start(query)
}

async fn new_redirect() -> impl IntoResponse {
    redirect_to_start("")
}

fn redirect_to_start(query: &str) -> Response {
    let loc = if query.is_empty() {
        "/start".to_string()
    } else {
        format!("/start?{query}")
    };
    (StatusCode::SEE_OTHER, [(header::LOCATION, loc)]).into_response()
}

fn html_no_cache(body: String) -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            (header::PRAGMA, "no-cache"),
        ],
        Html(body),
    )
        .into_response()
}

fn html_static_no_cache(body: &'static str) -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            (header::PRAGMA, "no-cache"),
        ],
        Html(body),
    )
        .into_response()
}

async fn app_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../../web/app.js"),
    )
}

async fn site_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
        ],
        include_str!("../../web/site.css"),
    )
}

async fn trust_page() -> Response {
    html_static_no_cache(include_str!("../../web/trust.html"))
}

async fn demo_gif() -> impl IntoResponse {
    let path = std::path::Path::new("/app/web/demo.gif");
    if let Ok(bytes) = std::fs::read(path) {
        return ([(header::CONTENT_TYPE, "image/gif")], bytes).into_response();
    }
    (
        [(header::CONTENT_TYPE, "image/gif")],
        include_bytes!("../../web/demo.gif").as_slice(),
    )
        .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/start", get(start_page))
        .route("/account", get(account_page))
        .route("/login", get(login_redirect))
        .route("/new", get(new_redirect))
        .route("/site.css", get(site_css))
        .route("/app.js", get(app_js))
        .route("/demo.gif", get(demo_gif))
        .route("/trust", get(trust_page))
        .route("/pricing", get(pricing_page))
        .route("/team", get(team_redirect))
        .route("/health", get(health))
        .route("/api/teams", post(create_team_handler))
        .route("/api/auth/login", post(login_handler))
        .route("/api/auth/session", get(session_handler))
        .route("/api/auth/logout", post(logout_handler))
        .route("/api/projects", post(create_project_handler))
        .route("/api/projects/{id}", get(get_project))
        .route("/api/projects/{id}/sync", post(sync_handler))
        .route("/api/projects/{id}/goal", put(update_goal_handler))
        .route("/api/teams/projects/link", post(link_project_handler))
        .route("/api/teams/projects", get(team_projects_handler))
        .route("/api/billing/upgrade", post(upgrade_handler))
        .route("/dashboard/{id}", get(dashboard))
        .route("/dashboard/{id}/edit", get(dashboard_edit))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request},
    };
    use tower::ServiceExt;

    fn json_request(method: Method, uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn accepts_legacy_session_cookie_during_upgrade() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("keel_session=legacy-token"),
        );
        assert_eq!(
            extract_session_token(&headers).as_deref(),
            Some("legacy-token")
        );

        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("keel_session=legacy; repovow_session=current"),
        );
        assert_eq!(extract_session_token(&headers).as_deref(), Some("current"));
    }

    #[tokio::test]
    async fn browser_security_flow_uses_server_sessions_and_redacts_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("security.db");
        db::init_db(&db_path).unwrap();
        std::env::set_var("REPOVOW_UPGRADE_CODES", "test-upgrade-code");

        let app = router(AppState {
            version: "test".to_string(),
            stripe_payment_link: "https://example.test/checkout".to_string(),
            create_secret: Some("server-signup-code".to_string()),
            secure_cookies: true,
        });

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/start")
                    .header(header::ORIGIN, "https://attacker.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(start_response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        let start_body = to_bytes(start_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&start_body).contains("server-signup-code"));

        let denied_signup = app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/api/teams",
                json!({"name": "Security", "email": "security@example.test"}),
            ))
            .await
            .unwrap();
        assert_eq!(denied_signup.status(), StatusCode::FORBIDDEN);

        let mut signup = json_request(
            Method::POST,
            "/api/teams",
            json!({"name": "Security", "email": "security@example.test"}),
        );
        signup.headers_mut().insert(
            "x-repovow-create-secret",
            HeaderValue::from_static("server-signup-code"),
        );
        let signup_response = app.clone().oneshot(signup).await.unwrap();
        assert_eq!(signup_response.status(), StatusCode::OK);
        assert_eq!(
            signup_response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let set_cookie = signup_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        assert!(set_cookie.contains("Secure"));
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let raw_token = cookie.split_once('=').unwrap().1;
        let signup_body = response_json(signup_response).await;
        assert!(signup_body["account_key"]
            .as_str()
            .is_some_and(|key| key.starts_with("repovow_team_")));

        let session_db = rusqlite::Connection::open(&db_path).unwrap();
        let raw_token_rows: i64 = session_db
            .query_row(
                "SELECT COUNT(*) FROM browser_sessions WHERE token_hash = ?1",
                rusqlite::params![raw_token],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw_token_rows, 0, "raw session tokens must not be stored");

        let mut create_project = json_request(
            Method::POST,
            "/api/projects",
            json!({"name": "first-project"}),
        );
        create_project
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        let create_response = app.clone().oneshot(create_project).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::OK);
        let created = response_json(create_response).await;
        let project_id = created["id"].as_str().unwrap().to_string();
        let project_key = created["api_key"].as_str().unwrap().to_string();
        assert!(created.get("team_license").is_none());

        let mut over_limit =
            json_request(Method::POST, "/api/projects", json!({"name": "over-limit"}));
        over_limit
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            app.clone().oneshot(over_limit).await.unwrap().status(),
            StatusCode::PAYMENT_REQUIRED
        );

        let unauthenticated_project = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated_project.status(), StatusCode::UNAUTHORIZED);

        let mut browser_project = Request::builder()
            .uri(format!("/api/projects/{project_id}"))
            .body(Body::empty())
            .unwrap();
        browser_project
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            app.clone().oneshot(browser_project).await.unwrap().status(),
            StatusCode::OK
        );

        let cli_project = Request::builder()
            .uri(format!("/api/projects/{project_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {project_key}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(cli_project).await.unwrap().status(),
            StatusCode::OK
        );

        let mut fleet = Request::builder()
            .uri("/api/teams/projects")
            .body(Body::empty())
            .unwrap();
        fleet
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        let fleet_body = response_json(app.clone().oneshot(fleet).await.unwrap()).await;
        assert!(fleet_body["team"].get("license").is_none());
        assert!(fleet_body["projects"][0].get("api_key").is_none());

        let mut upgrade = json_request(
            Method::POST,
            "/api/billing/upgrade",
            json!({"code": "test-upgrade-code"}),
        );
        upgrade
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            app.clone().oneshot(upgrade).await.unwrap().status(),
            StatusCode::OK
        );

        let mut create_second = json_request(
            Method::POST,
            "/api/projects",
            json!({"name": "second-project"}),
        );
        create_second
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            app.clone().oneshot(create_second).await.unwrap().status(),
            StatusCode::OK
        );

        let mut logout = Request::builder()
            .method(Method::POST)
            .uri("/api/auth/logout")
            .body(Body::empty())
            .unwrap();
        logout
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        let logout_response = app.clone().oneshot(logout).await.unwrap();
        assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
        assert!(logout_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));

        let mut revoked_session = Request::builder()
            .uri("/api/teams/projects")
            .body(Body::empty())
            .unwrap();
        revoked_session
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        assert_eq!(
            app.oneshot(revoked_session).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );

        std::env::remove_var("REPOVOW_UPGRADE_CODES");
    }
}
