use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub description: String,
    pub start_url: Option<String>,
    pub max_steps: i32,
    pub status: TaskStatus,
    pub steps_completed: i32,
    pub current_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        let status_str: String = row.try_get("status")?;
        Ok(Self {
            id: row.try_get("id")?,
            org_id: row.try_get("org_id")?,
            user_id: row.try_get("user_id")?,
            description: row.try_get("description")?,
            start_url: row.try_get("start_url")?,
            max_steps: row.try_get("max_steps")?,
            status: TaskStatus::from_str(&status_str),
            steps_completed: row.try_get("steps_completed")?,
            current_url: row.try_get("current_url")?,
            error_message: row.try_get("error_message")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub id: Uuid,
    pub task_id: Uuid,
    pub step_number: i32,
    pub action: String,
    pub result: Option<String>,
    pub url: Option<String>,
    pub screenshot_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl TaskStep {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            task_id: row.try_get("task_id")?,
            step_number: row.try_get("step_number")?,
            action: row.try_get("action")?,
            result: row.try_get("result")?,
            url: row.try_get("url")?,
            screenshot_url: row.try_get("screenshot_url")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// DTOs — Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub description: String,
    pub start_url: Option<String>,
    /// Maximum number of autonomous steps the agent may take (default: 50).
    pub max_steps: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    /// Filter by status (e.g. "running", "completed").
    pub status: Option<String>,
    /// Page number (1-based, default: 1).
    pub page: Option<i64>,
    /// Items per page (default: 20, max: 100).
    pub per_page: Option<i64>,
}

// ---------------------------------------------------------------------------
// DTOs — Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    pub task_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<Task>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Serialize)]
pub struct TimelineResponse {
    pub task_id: Uuid,
    pub steps: Vec<TaskStep>,
}

#[derive(Debug, Serialize)]
pub struct CancelTaskResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_task).get(list_tasks))
        .route("/{id}", get(get_task).delete(cancel_task))
        .route("/{id}/timeline", get(get_timeline))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST / — create a new autonomous browsing task.
async fn create_task(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<CreateTaskResponse>), AppError> {
    let task_id = Uuid::new_v4();
    let max_steps = body.max_steps.unwrap_or(50).min(500).max(1);

    sqlx::query(
        r#"
        INSERT INTO tasks (id, org_id, user_id, description, start_url, max_steps, status,
                           steps_completed, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, NOW(), NOW())
        "#,
    )
    .bind(task_id)
    .bind(claims.org_id)
    .bind(claims.sub)
    .bind(&body.description)
    .bind(&body.start_url)
    .bind(max_steps)
    .execute(&state.db)
    .await?;

    Ok((StatusCode::ACCEPTED, Json(CreateTaskResponse { task_id })))
}

/// GET / — list the authenticated user's tasks with optional status filter and
/// pagination.
async fn list_tasks(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<ListTasksQuery>,
) -> Result<Json<TaskListResponse>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let (tasks, total) = if let Some(ref status) = params.status {
        let rows = sqlx::query(
            r#"
            SELECT id, org_id, user_id, description, start_url, max_steps, status,
                   steps_completed, current_url, error_message, created_at, updated_at
            FROM tasks
            WHERE org_id = $1 AND user_id = $2 AND status = $3
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(claims.org_id)
        .bind(claims.sub)
        .bind(status)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

        let tasks: Vec<Task> = rows
            .iter()
            .map(|r| Task::from_row(r))
            .collect::<Result<Vec<_>, _>>()?;

        let total_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM tasks WHERE org_id = $1 AND user_id = $2 AND status = $3",
        )
        .bind(claims.org_id)
        .bind(claims.sub)
        .bind(status)
        .fetch_one(&state.db)
        .await?;

        let total: i64 = total_row.try_get("cnt")?;

        (tasks, total)
    } else {
        let rows = sqlx::query(
            r#"
            SELECT id, org_id, user_id, description, start_url, max_steps, status,
                   steps_completed, current_url, error_message, created_at, updated_at
            FROM tasks
            WHERE org_id = $1 AND user_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(claims.org_id)
        .bind(claims.sub)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

        let tasks: Vec<Task> = rows
            .iter()
            .map(|r| Task::from_row(r))
            .collect::<Result<Vec<_>, _>>()?;

        let total_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM tasks WHERE org_id = $1 AND user_id = $2",
        )
        .bind(claims.org_id)
        .bind(claims.sub)
        .fetch_one(&state.db)
        .await?;

        let total: i64 = total_row.try_get("cnt")?;

        (tasks, total)
    };

    Ok(Json(TaskListResponse {
        tasks,
        total,
        page,
        per_page,
    }))
}

/// GET /{id} — get task detail including status, progress, and current URL.
async fn get_task(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, org_id, user_id, description, start_url, max_steps, status,
               steps_completed, current_url, error_message, created_at, updated_at
        FROM tasks
        WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Task {id} not found")))?;

    let task = Task::from_row(&row)?;
    Ok(Json(task))
}

/// GET /{id}/timeline — get the step-by-step execution timeline for a task.
async fn get_timeline(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<TimelineResponse>, AppError> {
    // Verify the task belongs to the caller's org.
    let exists_row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = $1 AND org_id = $2) as ex",
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    let exists: bool = exists_row.try_get("ex")?;

    if !exists {
        return Err(AppError::NotFound(format!("Task {id} not found")));
    }

    let rows = sqlx::query(
        r#"
        SELECT id, task_id, step_number, action, result, url, screenshot_url, created_at
        FROM task_steps
        WHERE task_id = $1
        ORDER BY step_number ASC
        "#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let steps: Vec<TaskStep> = rows
        .iter()
        .map(|r| TaskStep::from_row(r))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(TimelineResponse {
        task_id: id,
        steps,
    }))
}

/// DELETE /{id} — cancel a running (or pending) task.
async fn cancel_task(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<CancelTaskResponse>, AppError> {
    let result = sqlx::query(
        r#"
        UPDATE tasks
        SET status = 'cancelled', updated_at = NOW()
        WHERE id = $1 AND org_id = $2 AND status IN ('pending', 'running')
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        // Either the task doesn't exist, doesn't belong to this org, or is
        // already in a terminal state.
        let exists_row = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = $1 AND org_id = $2) as ex",
        )
        .bind(id)
        .bind(claims.org_id)
        .fetch_one(&state.db)
        .await?;

        let exists: bool = exists_row.try_get("ex")?;

        if !exists {
            return Err(AppError::NotFound(format!("Task {id} not found")));
        }

        return Err(AppError::Conflict(
            "Task is already in a terminal state and cannot be cancelled".into(),
        ));
    }

    Ok(Json(CancelTaskResponse {
        task_id: id,
        status: TaskStatus::Cancelled,
    }))
}
