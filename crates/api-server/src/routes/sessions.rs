use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::models::{BrowserSession, SessionStatus};
use crate::AppState;

/// Snapshot returned by the engine after navigation or on demand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub html: Option<String>,
    pub text: Option<String>,
    pub screenshot_url: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
}

/// Generic result for an engine action (click, fill, submit, upload, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineActionResult {
    pub success: bool,
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub target_url: Option<String>,
    pub task_description: Option<String>,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: Uuid,
    pub status: SessionStatus,
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    /// Filter by session status (e.g. `running`, `completed`).
    pub status: Option<String>,
    /// 1-based page number (default 1).
    pub page: Option<i64>,
    /// Items per page (default 20, max 100).
    pub per_page: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedSessions {
    pub sessions: Vec<BrowserSession>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ClickRequest {
    pub ref_id: String,
}

#[derive(Debug, Deserialize)]
pub struct FillRequest {
    pub ref_id: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct EvalJsRequest {
    pub script: String,
}

/// File upload via JSON with base64-encoded content (avoids requiring the
/// `multipart` axum feature for now).
#[derive(Debug, Deserialize)]
pub struct UploadFileRequest {
    pub filename: String,
    /// Base64-encoded file content.
    pub content_base64: String,
}

#[derive(Debug, Deserialize)]
pub struct SubmitFormRequest {
    pub ref_id: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub session_id: Uuid,
    pub html: Option<String>,
    pub text: Option<String>,
    pub screenshot_url: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtractResponse {
    pub session_id: Uuid,
    pub markdown: String,
}

#[derive(Debug, Serialize)]
pub struct EvalJsResponse {
    pub session_id: Uuid,
    pub result: String,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub session_id: Uuid,
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteSessionResponse {
    pub session_id: Uuid,
    pub status: SessionStatus,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/{id}", get(get_session).delete(delete_session))
        .route("/{id}/navigate", post(navigate))
        .route("/{id}/click", post(click))
        .route("/{id}/fill", post(fill))
        .route("/{id}/snapshot", get(snapshot))
        .route("/{id}/extract", post(extract))
        .route("/{id}/eval-js", post(eval_js))
        .route("/{id}/upload-file", post(upload_file))
        .route("/{id}/submit-form", post(submit_form))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a status string into a `SessionStatus`, returning `BadRequest` on
/// unrecognised values.
fn parse_status_filter(raw: &str) -> Result<SessionStatus, AppError> {
    match raw.to_lowercase().as_str() {
        "pending" => Ok(SessionStatus::Pending),
        "running" => Ok(SessionStatus::Running),
        "paused" => Ok(SessionStatus::Paused),
        "completed" => Ok(SessionStatus::Completed),
        "failed" => Ok(SessionStatus::Failed),
        other => Err(AppError::BadRequest(format!(
            "Unknown session status filter: {other}"
        ))),
    }
}

/// Map a `SessionStatus` variant to the lowercase DB string representation.
fn status_to_db(s: &SessionStatus) -> &'static str {
    match s {
        SessionStatus::Pending => "pending",
        SessionStatus::Running => "running",
        SessionStatus::Paused => "paused",
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
    }
}

fn status_from_db(s: &str) -> SessionStatus {
    match s {
        "running" => SessionStatus::Running,
        "paused" => SessionStatus::Paused,
        "completed" => SessionStatus::Completed,
        "failed" => SessionStatus::Failed,
        _ => SessionStatus::Pending,
    }
}

/// Map a `sqlx::postgres::PgRow` into a `BrowserSession`.
fn row_to_session(row: &sqlx::postgres::PgRow) -> BrowserSession {
    let status_str: String = row.get("status");
    BrowserSession {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        status: status_from_db(&status_str),
        engine_snapshot_url: row.get("engine_snapshot_url"),
        config_json: row.get("config_json"),
        task_description: row.get("task_description"),
        steps_taken: row.get("steps_taken"),
        urls_visited: row.get("urls_visited"),
        result: row.get("result"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
    }
}

/// Fetch a single session row and verify it belongs to the caller's org.
async fn fetch_session_owned(
    db: &sqlx::PgPool,
    session_id: Uuid,
    org_id: Uuid,
) -> Result<BrowserSession, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, org_id, user_id, status, engine_snapshot_url, config_json,
               task_description, steps_taken, urls_visited, result,
               created_at, updated_at, completed_at
        FROM browser_sessions
        WHERE id = $1
        "#,
    )
    .bind(session_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Session {session_id} not found")))?;

    let session = row_to_session(&row);

    if session.org_id != org_id {
        return Err(AppError::Forbidden(
            "Session does not belong to your organisation".into(),
        ));
    }

    Ok(session)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST / — create a new browser session.
async fn create_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, AppError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let status_str = status_to_db(&SessionStatus::Pending);

    let config = body.config.unwrap_or(serde_json::json!({}));
    let task_desc = body.task_description.as_deref().unwrap_or("");
    let urls: Vec<String> = body.target_url.iter().cloned().collect();

    sqlx::query(
        r#"
        INSERT INTO browser_sessions (id, org_id, user_id, status, config_json, task_description, urls_visited, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .bind(claims.sub)
    .bind(status_str)
    .bind(&config)
    .bind(task_desc)
    .bind(&urls)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?;

    Ok(Json(CreateSessionResponse {
        session_id: id,
        status: SessionStatus::Pending,
    }))
}

/// GET / — list the caller's sessions with optional status filter and pagination.
async fn list_sessions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedSessions>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let (sessions, total) = if let Some(ref status_raw) = params.status {
        let status = parse_status_filter(status_raw)?;
        let status_str = status_to_db(&status);

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, user_id, status, engine_snapshot_url, config_json,
               task_description, steps_taken, urls_visited, result,
               created_at, updated_at, completed_at
            FROM browser_sessions
            WHERE org_id = $1 AND status = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(claims.org_id)
        .bind(status_str)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

        let count_row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM browser_sessions
            WHERE org_id = $1 AND status = $2
            "#,
        )
        .bind(claims.org_id)
        .bind(status_str)
        .fetch_one(&state.db)
        .await?;

        let count: i64 = count_row.get("count");

        let sessions: Vec<BrowserSession> = rows.iter().map(row_to_session).collect();
        (sessions, count)
    } else {
        let rows = sqlx::query(
            r#"
            SELECT id, org_id, user_id, status, engine_snapshot_url, config_json,
               task_description, steps_taken, urls_visited, result,
               created_at, updated_at, completed_at
            FROM browser_sessions
            WHERE org_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(claims.org_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

        let count_row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM browser_sessions
            WHERE org_id = $1
            "#,
        )
        .bind(claims.org_id)
        .fetch_one(&state.db)
        .await?;

        let count: i64 = count_row.get("count");

        let sessions: Vec<BrowserSession> = rows.iter().map(row_to_session).collect();
        (sessions, count)
    };

    Ok(Json(PaginatedSessions {
        sessions,
        total,
        page,
        per_page,
    }))
}

/// GET /:id — get session detail.
async fn get_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<BrowserSession>, AppError> {
    let session = fetch_session_owned(&state.db, id, claims.org_id).await?;
    Ok(Json(session))
}

/// DELETE /:id — terminate / soft-delete a session.
async fn delete_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteSessionResponse>, AppError> {
    // Verify ownership first.
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let completed = status_to_db(&SessionStatus::Completed);
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE browser_sessions
        SET status = $1, updated_at = $2
        WHERE id = $3
        "#,
    )
    .bind(completed)
    .bind(now)
    .bind(id)
    .execute(&state.db)
    .await?;

    Ok(Json(DeleteSessionResponse {
        session_id: id,
        status: SessionStatus::Completed,
    }))
}

/// POST /:id/navigate — navigate the session's browser to a URL.
async fn navigate(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<NavigateRequest>,
) -> Result<Json<SnapshotResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    // Mark as running and append the URL to urls_visited.
    let running = status_to_db(&SessionStatus::Running);
    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE browser_sessions
        SET status = $1, urls_visited = array_append(urls_visited, $2), updated_at = $3
        WHERE id = $4
        "#,
    )
    .bind(running)
    .bind(&body.url)
    .bind(now)
    .bind(id)
    .execute(&state.db)
    .await?;

    let snap = state.engine_bridge.navigate(id, &body.url).await?;

    Ok(Json(SnapshotResponse {
        session_id: id,
        html: snap.html,
        text: snap.text,
        screenshot_url: snap.screenshot_url,
        url: snap.url,
        title: snap.title,
    }))
}

/// POST /:id/click — click an element by accessibility ref_id.
async fn click(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<ClickRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let result = state.engine_bridge.click(id, &body.ref_id).await?;

    Ok(Json(ActionResponse {
        session_id: id,
        success: result.success,
        message: result.message,
    }))
}

/// POST /:id/fill — fill an element with text.
async fn fill(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<FillRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let result = state.engine_bridge.fill(id, &body.ref_id, &body.text).await?;

    Ok(Json(ActionResponse {
        session_id: id,
        success: result.success,
        message: result.message,
    }))
}

/// GET /:id/snapshot — get the current DOM snapshot.
async fn snapshot(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<SnapshotResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let snap = state.engine_bridge.snapshot(id).await?;

    Ok(Json(SnapshotResponse {
        session_id: id,
        html: snap.html,
        text: snap.text,
        screenshot_url: snap.screenshot_url,
        url: snap.url,
        title: snap.title,
    }))
}

/// POST /:id/extract — extract the visible page content as Markdown.
async fn extract(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<ExtractResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let md = state.engine_bridge.extract_markdown(id).await?;

    Ok(Json(ExtractResponse {
        session_id: id,
        markdown: md,
    }))
}

/// POST /:id/eval-js — execute JavaScript in the session and return the result.
async fn eval_js(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<EvalJsRequest>,
) -> Result<Json<EvalJsResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let result = state.engine_bridge.eval_js(id, &body.script).await?;

    Ok(Json(EvalJsResponse {
        session_id: id,
        result,
    }))
}

/// POST /:id/upload-file — upload a file into the session (base64-encoded JSON body).
async fn upload_file(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<UploadFileRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    if body.filename.is_empty() {
        return Err(AppError::BadRequest("filename must not be empty".into()));
    }
    if body.content_base64.is_empty() {
        return Err(AppError::BadRequest(
            "content_base64 must not be empty".into(),
        ));
    }

    let result = state.engine_bridge.upload_file(id, &body.filename, &body.content_base64).await?;

    Ok(Json(ActionResponse {
        session_id: id,
        success: result.success,
        message: result.message,
    }))
}

/// POST /:id/submit-form — submit a form identified by ref_id.
async fn submit_form(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<SubmitFormRequest>,
) -> Result<Json<ActionResponse>, AppError> {
    let _session = fetch_session_owned(&state.db, id, claims.org_id).await?;

    let result = state.engine_bridge.submit_form(id, &body.ref_id).await?;

    Ok(Json(ActionResponse {
        session_id: id,
        success: result.success,
        message: result.message,
    }))
}
