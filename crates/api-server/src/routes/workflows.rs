use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkflowRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub steps: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ReplayRequest {
    pub variables: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Workflow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workflow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            org_id: row.try_get("org_id")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            steps: row.try_get("steps")?,
            created_by: row.try_get("created_by")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct WorkflowSummary {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowSummary {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct WorkflowList {
    pub items: Vec<WorkflowSummary>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Serialize)]
pub struct ReplayResponse {
    pub task_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct Execution {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub status: ExecutionStatus,
    pub variables: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionStatus {
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

impl Execution {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        let status_str: String = row.try_get("status")?;
        Ok(Self {
            id: row.try_get("id")?,
            workflow_id: row.try_get("workflow_id")?,
            status: ExecutionStatus::from_str(&status_str),
            variables: row.try_get("variables")?,
            result: row.try_get("result")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct ExecutionList {
    pub items: Vec<Execution>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_workflows).post(create_workflow))
        .route(
            "/{id}",
            get(get_workflow).put(update_workflow).delete(delete_workflow),
        )
        .route("/{id}/replay", post(replay_workflow))
        .route("/{id}/executions", get(list_executions))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn create_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateWorkflowRequest>,
) -> Result<Json<Workflow>, AppError> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    let row = sqlx::query(
        r#"
        INSERT INTO workflows (id, org_id, name, description, steps, created_by, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
        RETURNING id, org_id, name, description, steps, created_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .bind(&body.name)
    .bind(&body.description)
    .bind(&body.steps)
    .bind(claims.sub)
    .bind(now)
    .fetch_one(&state.db)
    .await?;

    let workflow = Workflow::from_row(&row)?;
    Ok(Json(workflow))
}

async fn list_workflows(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<WorkflowList>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let total_row = sqlx::query(
        r#"SELECT COUNT(*) as cnt FROM workflows WHERE org_id = $1"#,
    )
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    let total: i64 = total_row.try_get("cnt")?;

    let rows = sqlx::query(
        r#"
        SELECT id, name, description, created_at, updated_at
        FROM workflows
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

    let items: Vec<WorkflowSummary> = rows
        .iter()
        .map(|r| WorkflowSummary::from_row(r))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(WorkflowList {
        items,
        total,
        page,
        per_page,
    }))
}

async fn get_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<Workflow>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, org_id, name, description, steps, created_by, created_at, updated_at
        FROM workflows
        WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Workflow {id} not found")))?;

    let workflow = Workflow::from_row(&row)?;
    Ok(Json(workflow))
}

async fn update_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWorkflowRequest>,
) -> Result<Json<Workflow>, AppError> {
    let now = Utc::now();

    let row = sqlx::query(
        r#"
        UPDATE workflows
        SET name        = COALESCE($3, name),
            description = COALESCE($4, description),
            steps       = COALESCE($5, steps),
            updated_at  = $6
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, name, description, steps, created_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .bind(&body.name)
    .bind(&body.description)
    .bind(&body.steps)
    .bind(now)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Workflow {id} not found")))?;

    let workflow = Workflow::from_row(&row)?;
    Ok(Json(workflow))
}

async fn delete_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = sqlx::query(
        r#"DELETE FROM workflows WHERE id = $1 AND org_id = $2"#,
    )
    .bind(id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Workflow {id} not found")));
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn replay_workflow(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplayRequest>,
) -> Result<Json<ReplayResponse>, AppError> {
    // Verify workflow exists and belongs to the org.
    let exists_row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM workflows WHERE id = $1 AND org_id = $2) as ex",
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    let exists: bool = exists_row.try_get("ex")?;
    if !exists {
        return Err(AppError::NotFound(format!("Workflow {id} not found")));
    }

    let task_id = Uuid::new_v4();
    let now = Utc::now();
    let variables_json = serde_json::to_value(&body.variables)
        .map_err(|e| AppError::BadRequest(format!("Invalid variables: {e}")))?;

    // Create an execution record; an async worker will pick it up.
    sqlx::query(
        r#"
        INSERT INTO workflow_executions (id, workflow_id, org_id, status, variables, started_at)
        VALUES ($1, $2, $3, 'pending', $4, $5)
        "#,
    )
    .bind(task_id)
    .bind(id)
    .bind(claims.org_id)
    .bind(&variables_json)
    .bind(now)
    .execute(&state.db)
    .await?;

    Ok(Json(ReplayResponse { task_id }))
}

async fn list_executions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ExecutionList>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let total_row = sqlx::query(
        r#"
        SELECT COUNT(*) as cnt
        FROM workflow_executions
        WHERE workflow_id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    let total: i64 = total_row.try_get("cnt")?;

    let rows = sqlx::query(
        r#"
        SELECT id, workflow_id, status, variables, result, started_at, finished_at
        FROM workflow_executions
        WHERE workflow_id = $1 AND org_id = $2
        ORDER BY started_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let items: Vec<Execution> = rows
        .iter()
        .map(|r| Execution::from_row(r))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(ExecutionList {
        items,
        total,
        page,
        per_page,
    }))
}
