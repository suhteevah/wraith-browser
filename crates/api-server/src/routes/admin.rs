use axum::{
    extract::{Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// Role guard — only org owners and auditors may access admin endpoints
// ---------------------------------------------------------------------------

const ALLOWED_ROLES: &[&str] = &["owner", "auditor"];

fn require_admin_or_auditor(claims: &Claims) -> Result<(), AppError> {
    if ALLOWED_ROLES.contains(&claims.role.as_str()) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Only org owners and auditors may access admin endpoints".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// DTOs — Compliance
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Soc2ControlStatus {
    pub control_id: String,
    pub name: String,
    pub status: String,
    pub last_evaluated: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ComplianceReport {
    pub org_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub soc2_controls: Vec<Soc2ControlStatus>,
    pub encryption_at_rest: bool,
    pub encryption_in_transit: bool,
    pub gdpr_readiness_score: f64,
}

// ---------------------------------------------------------------------------
// DTOs — Audit Log
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub actor_id: Option<Uuid>,
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub actor_id: Uuid,
    pub actor_email: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogPage {
    pub entries: Vec<AuditLogEntry>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

// ---------------------------------------------------------------------------
// DTOs — Data Residency
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DataResidencyConfig {
    pub org_id: Uuid,
    pub region: String,
    pub locked: bool,
    pub set_at: DateTime<Utc>,
    pub set_by: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct SetDataResidencyRequest {
    /// Allowed values: "us", "eu", "apac".
    pub region: String,
}

// ---------------------------------------------------------------------------
// DTOs — Security
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SecurityOverview {
    pub org_id: Uuid,
    pub total_users: i64,
    pub mfa_enabled_count: i64,
    pub mfa_adoption_pct: f64,
    pub api_key_count: i64,
    pub sso_enabled: bool,
    pub last_access_review: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// DTOs — Access Review
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct UserPermissionEntry {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub mfa_enabled: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub api_key_count: i64,
}

#[derive(Debug, Serialize)]
pub struct AccessReviewReport {
    pub org_id: Uuid,
    pub triggered_by: Uuid,
    pub triggered_at: DateTime<Utc>,
    pub users: Vec<UserPermissionEntry>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/compliance", get(compliance))
        .route("/audit-log", get(audit_log))
        .route("/audit-log/export", get(audit_log_export))
        .route("/data-residency", get(get_data_residency).put(set_data_residency))
        .route("/security", get(security))
        .route("/access-review", post(access_review))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn compliance(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<ComplianceReport>, AppError> {
    require_admin_or_auditor(&claims)?;

    let org_id = claims.org_id;

    // SOC 2 control statuses for this org.
    let control_rows = sqlx::query(
        "SELECT control_id, name, status, last_evaluated \
         FROM soc2_controls \
         WHERE org_id = $1 \
         ORDER BY control_id",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let controls: Vec<Soc2ControlStatus> = control_rows
        .into_iter()
        .map(|r| Soc2ControlStatus {
            control_id: r.get("control_id"),
            name: r.get("name"),
            status: r.get("status"),
            last_evaluated: r.get("last_evaluated"),
        })
        .collect();

    // Encryption and GDPR readiness from org settings.
    let row = sqlx::query(
        "SELECT encryption_at_rest, encryption_in_transit, gdpr_readiness_score \
         FROM org_compliance_settings \
         WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ComplianceReport {
        org_id,
        generated_at: Utc::now(),
        soc2_controls: controls,
        encryption_at_rest: row.get("encryption_at_rest"),
        encryption_in_transit: row.get("encryption_in_transit"),
        gdpr_readiness_score: row.get("gdpr_readiness_score"),
    }))
}

fn row_to_audit_entry(r: sqlx::postgres::PgRow) -> AuditLogEntry {
    AuditLogEntry {
        id: r.get("id"),
        org_id: r.get("org_id"),
        actor_id: r.get("actor_id"),
        actor_email: r.get("actor_email"),
        action: r.get("action"),
        resource_type: r.get("resource_type"),
        resource_id: r.get("resource_id"),
        metadata: r.get("metadata"),
        ip_address: r.get("ip_address"),
        created_at: r.get("created_at"),
    }
}

async fn audit_log(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<AuditLogQuery>,
) -> Result<Json<AuditLogPage>, AppError> {
    require_admin_or_auditor(&claims)?;

    let org_id = claims.org_id;
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * per_page;

    let total: Option<i64> = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM audit_log \
         WHERE org_id = $1 \
           AND ($2::uuid IS NULL OR actor_id = $2) \
           AND ($3::text IS NULL OR action = $3) \
           AND ($4::text IS NULL OR resource_type = $4) \
           AND ($5::date IS NULL OR created_at >= $5::date) \
           AND ($6::date IS NULL OR created_at < $6::date + INTERVAL '1 day')",
    )
    .bind(org_id)
    .bind(params.actor_id)
    .bind(&params.action)
    .bind(&params.resource_type)
    .bind(params.date_from)
    .bind(params.date_to)
    .fetch_one(&state.db)
    .await?;
    let total = total.unwrap_or(0);

    let rows = sqlx::query(
        "SELECT id, org_id, actor_id, actor_email, action, \
               resource_type, resource_id, metadata, ip_address, created_at \
         FROM audit_log \
         WHERE org_id = $1 \
           AND ($2::uuid IS NULL OR actor_id = $2) \
           AND ($3::text IS NULL OR action = $3) \
           AND ($4::text IS NULL OR resource_type = $4) \
           AND ($5::date IS NULL OR created_at >= $5::date) \
           AND ($6::date IS NULL OR created_at < $6::date + INTERVAL '1 day') \
         ORDER BY created_at DESC \
         LIMIT $7 OFFSET $8",
    )
    .bind(org_id)
    .bind(params.actor_id)
    .bind(&params.action)
    .bind(&params.resource_type)
    .bind(params.date_from)
    .bind(params.date_to)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let entries: Vec<AuditLogEntry> = rows.into_iter().map(row_to_audit_entry).collect();

    Ok(Json(AuditLogPage {
        entries,
        page,
        per_page,
        total,
    }))
}

async fn audit_log_export(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<AuditLogQuery>,
) -> Result<(axum::http::HeaderMap, String), AppError> {
    require_admin_or_auditor(&claims)?;

    let org_id = claims.org_id;

    let rows = sqlx::query(
        "SELECT id, org_id, actor_id, actor_email, action, \
               resource_type, resource_id, metadata, ip_address, created_at \
         FROM audit_log \
         WHERE org_id = $1 \
           AND ($2::uuid IS NULL OR actor_id = $2) \
           AND ($3::text IS NULL OR action = $3) \
           AND ($4::text IS NULL OR resource_type = $4) \
           AND ($5::date IS NULL OR created_at >= $5::date) \
           AND ($6::date IS NULL OR created_at < $6::date + INTERVAL '1 day') \
         ORDER BY created_at DESC",
    )
    .bind(org_id)
    .bind(params.actor_id)
    .bind(&params.action)
    .bind(&params.resource_type)
    .bind(params.date_from)
    .bind(params.date_to)
    .fetch_all(&state.db)
    .await?;

    let entries: Vec<AuditLogEntry> = rows.into_iter().map(row_to_audit_entry).collect();

    // Build CSV output.
    let mut csv = String::from(
        "id,org_id,actor_id,actor_email,action,resource_type,resource_id,metadata,ip_address,created_at\n",
    );
    for r in &entries {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            r.id,
            r.org_id,
            r.actor_id,
            csv_escape(&r.actor_email),
            csv_escape(&r.action),
            csv_escape(&r.resource_type),
            r.resource_id.as_deref().unwrap_or(""),
            r.metadata.as_ref().map(|v| v.to_string()).unwrap_or_default(),
            r.ip_address.as_deref().unwrap_or(""),
            r.created_at.to_rfc3339(),
        ));
    }

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::CONTENT_DISPOSITION,
        "attachment; filename=\"audit-log.csv\"".parse().unwrap(),
    );

    Ok((headers, csv))
}

async fn get_data_residency(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<DataResidencyConfig>, AppError> {
    require_admin_or_auditor(&claims)?;

    let row = sqlx::query(
        "SELECT org_id, region, locked, set_at, set_by \
         FROM data_residency \
         WHERE org_id = $1",
    )
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Data residency not configured for this org".into()))?;

    Ok(Json(DataResidencyConfig {
        org_id: row.get("org_id"),
        region: row.get("region"),
        locked: row.get("locked"),
        set_at: row.get("set_at"),
        set_by: row.get("set_by"),
    }))
}

async fn set_data_residency(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<SetDataResidencyRequest>,
) -> Result<Json<DataResidencyConfig>, AppError> {
    require_admin_or_auditor(&claims)?;

    // Validate region.
    let allowed_regions = ["us", "eu", "apac"];
    if !allowed_regions.contains(&body.region.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid region '{}'. Allowed: us, eu, apac",
            body.region
        )));
    }

    // Check whether residency is already locked (can only be set at org creation
    // or by support).
    let existing: Option<bool> = sqlx::query_scalar(
        "SELECT locked FROM data_residency WHERE org_id = $1",
    )
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?;

    if let Some(true) = existing {
        return Err(AppError::Forbidden(
            "Data residency is locked. Contact support to change it.".into(),
        ));
    }

    let row = sqlx::query(
        "INSERT INTO data_residency (org_id, region, locked, set_at, set_by) \
         VALUES ($1, $2, true, NOW(), $3) \
         ON CONFLICT (org_id) DO UPDATE \
           SET region = EXCLUDED.region, \
               locked = true, \
               set_at = NOW(), \
               set_by = EXCLUDED.set_by \
         RETURNING org_id, region, locked, set_at, set_by",
    )
    .bind(claims.org_id)
    .bind(&body.region)
    .bind(claims.sub)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(DataResidencyConfig {
        org_id: row.get("org_id"),
        region: row.get("region"),
        locked: row.get("locked"),
        set_at: row.get("set_at"),
        set_by: row.get("set_by"),
    }))
}

async fn security(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<SecurityOverview>, AppError> {
    require_admin_or_auditor(&claims)?;

    let org_id = claims.org_id;

    let user_counts = sqlx::query(
        "SELECT \
             COUNT(*) AS total, \
             COUNT(*) FILTER (WHERE mfa_enabled = true) AS mfa_count \
         FROM users \
         WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    let total: i64 = user_counts.get("total");
    let mfa_count: i64 = user_counts.get("mfa_count");

    let api_key_count: Option<i64> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_keys WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;
    let api_key_count = api_key_count.unwrap_or(0);

    let sso_enabled: Option<bool> = sqlx::query_scalar(
        "SELECT sso_enabled FROM organizations WHERE id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;
    let sso_enabled = sso_enabled.unwrap_or(false);

    let last_access_review: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT MAX(triggered_at) FROM access_reviews WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    let mfa_pct = if total > 0 {
        (mfa_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(SecurityOverview {
        org_id,
        total_users: total,
        mfa_enabled_count: mfa_count,
        mfa_adoption_pct: mfa_pct,
        api_key_count,
        sso_enabled,
        last_access_review,
    }))
}

async fn access_review(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<AccessReviewReport>, AppError> {
    require_admin_or_auditor(&claims)?;

    let org_id = claims.org_id;
    let now = Utc::now();

    // Record that an access review was triggered.
    sqlx::query(
        "INSERT INTO access_reviews (id, org_id, triggered_by, triggered_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(claims.sub)
    .bind(now)
    .execute(&state.db)
    .await?;

    // Gather all users with their permission details.
    let rows = sqlx::query(
        "SELECT \
             u.id AS user_id, \
             u.email, \
             u.display_name, \
             u.role, \
             u.mfa_enabled, \
             u.last_login, \
             (SELECT COUNT(*) FROM api_keys ak WHERE ak.user_id = u.id) AS api_key_count \
         FROM users u \
         WHERE u.org_id = $1 \
         ORDER BY u.email",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    let users: Vec<UserPermissionEntry> = rows
        .into_iter()
        .map(|r| UserPermissionEntry {
            user_id: r.get("user_id"),
            email: r.get("email"),
            display_name: r.get("display_name"),
            role: r.get("role"),
            mfa_enabled: r.get("mfa_enabled"),
            last_login: r.get("last_login"),
            api_key_count: r.get("api_key_count"),
        })
        .collect();

    Ok(Json(AccessReviewReport {
        org_id,
        triggered_by: claims.sub,
        triggered_at: now,
        users,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal CSV field escaping — wraps in double-quotes if the value contains
/// a comma, quote, or newline, and doubles any internal quotes.
fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
