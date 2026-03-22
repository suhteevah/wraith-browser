use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::engine_bridge::EngineBridge;
use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs — Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FanOutRequest {
    pub urls: Vec<String>,
    pub max_concurrent: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ResultsPagination {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

// ---------------------------------------------------------------------------
// DTOs — Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FanOutResponse {
    pub job_id: Uuid,
    pub total_urls: usize,
    pub max_concurrent: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwarmJobStatus {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl SwarmJobStatus {
    fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SwarmJobStatusResponse {
    pub job_id: Uuid,
    pub status: SwarmJobStatus,
    pub total_urls: usize,
    pub completed: usize,
    pub failed: usize,
    pub in_progress: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SwarmUrlResult {
    pub url: String,
    pub status: String,
    pub title: Option<String>,
    pub content_preview: Option<String>,
    pub extracted_data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SwarmResultsResponse {
    pub job_id: Uuid,
    pub page: u32,
    pub per_page: u32,
    pub total: usize,
    pub results: Vec<SwarmUrlResult>,
}

#[derive(Debug, Serialize)]
pub struct SwarmCancelResponse {
    pub job_id: Uuid,
    pub status: SwarmJobStatus,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/fan-out", post(fan_out))
        .route("/{id}", get(get_job_status).delete(cancel_job))
        .route("/{id}/results", get(get_results))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /fan-out — create a new swarm crawl job.
async fn fan_out(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<FanOutRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.urls.is_empty() {
        return Err(AppError::BadRequest("urls must not be empty".into()));
    }

    let job_id = Uuid::new_v4();
    let max_concurrent = body.max_concurrent.unwrap_or(10);
    let total_urls = body.urls.len();
    let now = Utc::now();

    // Insert the swarm job record.
    sqlx::query(
        r#"
        INSERT INTO swarm_jobs (id, org_id, created_by, status, total_urls, max_concurrent, created_at, updated_at)
        VALUES ($1, $2, $3, 'pending', $4, $5, $6, $6)
        "#,
    )
    .bind(job_id)
    .bind(claims.org_id)
    .bind(claims.sub)
    .bind(total_urls as i64)
    .bind(max_concurrent as i32)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("failed to create swarm job: {e}")))?;

    // Bulk-insert the individual URL tasks.
    for url in &body.urls {
        sqlx::query(
            r#"
            INSERT INTO swarm_url_tasks (id, job_id, url, status, created_at, updated_at)
            VALUES ($1, $2, $3, 'pending', $4, $4)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(job_id)
        .bind(url)
        .bind(now)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("failed to insert url task: {e}")))?;
    }

    // Dispatch the actual crawl work to a background task (fire-and-forget).
    {
        let db = state.db.clone();
        let engine = state.engine_bridge.clone();
        tokio::spawn(async move {
            run_swarm_job(db, engine, job_id, max_concurrent).await;
        });
    }

    let resp = FanOutResponse {
        job_id,
        total_urls,
        max_concurrent,
        created_at: now,
    };

    Ok((StatusCode::ACCEPTED, Json(resp)))
}

/// GET /{id} — get the current status of a swarm job.
async fn get_job_status(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<SwarmJobStatusResponse>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT status, total_urls, max_concurrent, created_at, updated_at
        FROM swarm_jobs
        WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?
    .ok_or_else(|| AppError::NotFound(format!("swarm job {id} not found")))?;

    let status_str: String = row.try_get("status")?;
    let total_urls: i64 = row.try_get("total_urls")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    // Count per-URL task states.
    let counts_row = sqlx::query(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'completed') as completed,
            COUNT(*) FILTER (WHERE status = 'failed') as failed,
            COUNT(*) FILTER (WHERE status = 'running') as running
        FROM swarm_url_tasks
        WHERE job_id = $1
        "#,
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    let completed: i64 = counts_row.try_get("completed")?;
    let failed: i64 = counts_row.try_get("failed")?;
    let running: i64 = counts_row.try_get("running")?;

    let status = SwarmJobStatus::from_str(&status_str);

    Ok(Json(SwarmJobStatusResponse {
        job_id: id,
        status,
        total_urls: total_urls as usize,
        completed: completed as usize,
        failed: failed as usize,
        in_progress: running as usize,
        created_at,
        updated_at,
    }))
}

/// GET /{id}/results — paginated list of per-URL crawl results.
async fn get_results(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Query(pagination): Query<ResultsPagination>,
) -> Result<Json<SwarmResultsResponse>, AppError> {
    // Verify job exists and belongs to the caller's org.
    let exists_row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM swarm_jobs WHERE id = $1 AND org_id = $2) as ex",
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    let exists: bool = exists_row.try_get("ex")?;

    if !exists {
        return Err(AppError::NotFound(format!("swarm job {id} not found")));
    }

    let page = pagination.page.unwrap_or(1).max(1);
    let per_page = pagination.per_page.unwrap_or(50).clamp(1, 200);
    let offset = ((page - 1) * per_page) as i64;

    let total_row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM swarm_url_tasks WHERE job_id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    let total: i64 = total_row.try_get("cnt")?;

    let rows = sqlx::query(
        r#"
        SELECT url, status, title, content_preview, extracted_data
        FROM swarm_url_tasks
        WHERE job_id = $1
        ORDER BY created_at ASC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(id)
    .bind(per_page as i64)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    let results = rows
        .iter()
        .map(|r| {
            Ok(SwarmUrlResult {
                url: r.try_get("url")?,
                status: r.try_get("status")?,
                title: r.try_get("title")?,
                content_preview: r.try_get("content_preview")?,
                extracted_data: r.try_get("extracted_data")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

    Ok(Json(SwarmResultsResponse {
        job_id: id,
        page,
        per_page,
        total: total as usize,
        results,
    }))
}

/// DELETE /{id} — cancel a running swarm job.
async fn cancel_job(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<SwarmCancelResponse>, AppError> {
    let result = sqlx::query(
        r#"
        UPDATE swarm_jobs
        SET status = 'cancelled', updated_at = NOW()
        WHERE id = $1 AND org_id = $2 AND status IN ('pending', 'running')
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "swarm job {id} not found or already finished"
        )));
    }

    // Cancel all pending/running URL tasks for this job.
    sqlx::query(
        r#"
        UPDATE swarm_url_tasks
        SET status = 'cancelled', updated_at = NOW()
        WHERE job_id = $1 AND status IN ('pending', 'running')
        "#,
    )
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    // TODO: signal the background worker to stop in-flight crawls.

    Ok(Json(SwarmCancelResponse {
        job_id: id,
        status: SwarmJobStatus::Cancelled,
        message: "job cancelled successfully".into(),
    }))
}

// ---------------------------------------------------------------------------
// Background swarm processing
// ---------------------------------------------------------------------------

/// Runs the actual fan-out crawl for a swarm job in the background.
///
/// 1. Marks the job as "running".
/// 2. Fetches all pending URL tasks.
/// 3. Processes them concurrently (bounded by `max_concurrent`) via a semaphore.
/// 4. For each URL: creates a temporary engine session, navigates, extracts
///    content, and writes the result back into the DB row.
/// 5. When all URLs are done, marks the job as "completed" (or "failed" if
///    every single URL failed).
async fn run_swarm_job(
    db: PgPool,
    engine: Arc<EngineBridge>,
    job_id: Uuid,
    max_concurrent: usize,
) {
    // Mark job as running.
    if let Err(e) = sqlx::query(
        "UPDATE swarm_jobs SET status = 'running', updated_at = NOW() WHERE id = $1",
    )
    .bind(job_id)
    .execute(&db)
    .await
    {
        tracing::error!(job_id = %job_id, "failed to mark job running: {e}");
        return;
    }

    // Fetch all pending URL tasks for this job.
    let tasks = match sqlx::query(
        "SELECT id, url FROM swarm_url_tasks WHERE job_id = $1 AND status = 'pending'",
    )
    .bind(job_id)
    .fetch_all(&db)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(job_id = %job_id, "failed to fetch url tasks: {e}");
            let _ = sqlx::query(
                "UPDATE swarm_jobs SET status = 'failed', updated_at = NOW() WHERE id = $1",
            )
            .bind(job_id)
            .execute(&db)
            .await;
            return;
        }
    };

    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::with_capacity(tasks.len());

    for row in &tasks {
        let task_id: Uuid = row.try_get("id").expect("task row missing id");
        let url: String = row.try_get("url").expect("task row missing url");

        let permit = semaphore.clone().acquire_owned().await;
        if permit.is_err() {
            break; // semaphore closed — shouldn't happen
        }
        let permit = permit.unwrap();

        let db = db.clone();
        let engine = engine.clone();
        let job_id = job_id;

        handles.push(tokio::spawn(async move {
            let result = process_single_url(&db, &engine, task_id, &url).await;
            drop(permit); // release the semaphore slot
            result
        }));
    }

    // Wait for every task to finish.
    let mut all_failed = !handles.is_empty();
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {
                all_failed = false;
            }
            Ok(Err(())) => {
                // task recorded its own failure in DB — continue
            }
            Err(e) => {
                tracing::error!(job_id = %job_id, "url task panicked: {e}");
            }
        }
    }

    // Check if the job was cancelled while we were running.
    let cancelled = sqlx::query(
        "SELECT status FROM swarm_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(&db)
    .await
    .ok()
    .flatten()
    .and_then(|r| r.try_get::<String, _>("status").ok())
    .map(|s| s == "cancelled")
    .unwrap_or(false);

    if !cancelled {
        let final_status = if all_failed { "failed" } else { "completed" };
        let _ = sqlx::query(
            "UPDATE swarm_jobs SET status = $1, updated_at = NOW() WHERE id = $2",
        )
        .bind(final_status)
        .bind(job_id)
        .execute(&db)
        .await;
    }

    tracing::info!(job_id = %job_id, "swarm job finished");
}

/// Process a single URL task: navigate, extract content, update DB.
///
/// Returns `Ok(())` on success, `Err(())` on failure (after updating the DB row).
async fn process_single_url(
    db: &PgPool,
    engine: &EngineBridge,
    task_id: Uuid,
    url: &str,
) -> Result<(), ()> {
    // Mark the task as running.
    let _ = sqlx::query(
        "UPDATE swarm_url_tasks SET status = 'running', updated_at = NOW() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db)
    .await;

    // Create a temporary engine session for this URL.
    let session_id = Uuid::new_v4();
    if let Err(e) = engine.create_session(session_id).await {
        mark_task_failed(db, task_id, &format!("session create: {e}")).await;
        return Err(());
    }

    // Navigate to the URL.
    let nav_result = engine.navigate(session_id, url).await;
    let snapshot = match nav_result {
        Ok(snap) => snap,
        Err(e) => {
            mark_task_failed(db, task_id, &format!("navigate: {e}")).await;
            let _ = engine.destroy_session(session_id).await;
            return Err(());
        }
    };

    // Extract content (try markdown extraction; fall back to snapshot text).
    let content_preview = match engine.extract_markdown(session_id).await {
        Ok(md) if !md.is_empty() => Some(truncate_preview(&md, 2000)),
        _ => snapshot.text.as_deref().map(|t| truncate_preview(t, 2000)),
    };

    let title = snapshot.title.clone();

    // Clean up the temporary session.
    let _ = engine.destroy_session(session_id).await;

    // Update the task row with results.
    if let Err(e) = sqlx::query(
        r#"
        UPDATE swarm_url_tasks
        SET status = 'completed',
            title = $1,
            content_preview = $2,
            updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(&title)
    .bind(&content_preview)
    .bind(task_id)
    .execute(db)
    .await
    {
        tracing::error!(task_id = %task_id, "failed to update completed task: {e}");
        return Err(());
    }

    Ok(())
}

/// Mark a URL task as failed with an error message stored in `extracted_data`.
async fn mark_task_failed(db: &PgPool, task_id: Uuid, error: &str) {
    let error_json = serde_json::json!({ "error": error });
    let _ = sqlx::query(
        r#"
        UPDATE swarm_url_tasks
        SET status = 'failed',
            extracted_data = $1,
            updated_at = NOW()
        WHERE id = $2
        "#,
    )
    .bind(&error_json)
    .bind(task_id)
    .execute(db)
    .await;
}

/// Truncate a string to at most `max_len` bytes on a char boundary.
fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        let mut result = s[..end].to_string();
        result.push_str("…");
        result
    }
}
