use axum::{
    extract::{Path, Query, State},
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
// DTOs — Current usage
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CurrentUsageResponse {
    pub org_id: Uuid,
    pub pages_used: i64,
    pub pages_limit: i64,
    pub percent_used: f64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DTOs — History
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Day,
    Week,
    Month,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub granularity: Option<Granularity>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct UsageDataPoint {
    pub timestamp: DateTime<Utc>,
    pub pages_used: i64,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub org_id: Uuid,
    pub granularity: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub data: Vec<UsageDataPoint>,
}

// ---------------------------------------------------------------------------
// DTOs — Breakdown
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakdownDimension {
    Team,
    User,
    ActionType,
}

#[derive(Debug, Deserialize)]
pub struct BreakdownQuery {
    pub dimension: Option<BreakdownDimension>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct BreakdownEntry {
    pub id: Uuid,
    pub label: String,
    pub pages_used: i64,
    pub percent_of_total: f64,
}

#[derive(Debug, Serialize)]
pub struct BreakdownResponse {
    pub org_id: Uuid,
    pub dimension: String,
    pub entries: Vec<BreakdownEntry>,
}

// ---------------------------------------------------------------------------
// DTOs — Plan
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub org_id: Uuid,
    pub tier: String,
    pub price_cents: i64,
    pub pages_limit: i64,
    pub features: Vec<String>,
}

// ---------------------------------------------------------------------------
// DTOs — Alerts
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateAlertRequest {
    /// Threshold expressed as a percentage of the plan limit (e.g. 80, 100, 150).
    pub threshold_percent: i32,
    /// Email address to notify; defaults to the org admin contacts when absent.
    pub email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AlertResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub threshold_percent: i32,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DTOs — Invoices
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InvoicesQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct InvoiceSummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub amount_cents: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct InvoiceLineItem {
    pub description: String,
    pub quantity: i64,
    pub unit_price_cents: i64,
    pub total_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct InvoiceDetailResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub amount_cents: i64,
    pub status: String,
    pub line_items: Vec<InvoiceLineItem>,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedInvoices {
    pub data: Vec<InvoiceSummary>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/current", get(current_usage))
        .route("/history", get(usage_history))
        .route("/breakdown", get(usage_breakdown))
        .route("/plan", get(current_plan))
        .route("/alerts", post(create_alert))
        .route("/invoices", get(list_invoices))
        .route("/invoices/{id}", get(get_invoice))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` when the caller's role is `owner` or `admin`.
fn is_admin_or_owner(claims: &Claims) -> bool {
    matches!(claims.role.as_str(), "owner" | "admin")
}

/// Members may only view their own usage. Admins/owners see the full org.
/// Returns `Some(user_id)` to filter by when the caller is a plain member,
/// or `None` when they have full org visibility.
fn visibility_filter(claims: &Claims) -> Option<Uuid> {
    if is_admin_or_owner(claims) {
        None
    } else {
        Some(claims.sub)
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn current_usage(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<CurrentUsageResponse>, AppError> {
    let org_id = claims.org_id;

    // Admins see total org usage; members see only their own page count.
    let row = if let Some(user_id) = visibility_filter(&claims) {
        sqlx::query(
            "SELECT \
                 COALESCE(SUM(pages), 0) AS pages_used, \
                 o.pages_limit, \
                 bp.period_start, \
                 bp.period_end \
             FROM billing_periods bp \
             JOIN organizations o ON o.id = bp.org_id \
             LEFT JOIN usage_events ue \
                 ON ue.org_id = bp.org_id \
                AND ue.user_id = $2 \
                AND ue.created_at >= bp.period_start \
                AND ue.created_at < bp.period_end \
             WHERE bp.org_id = $1 \
               AND bp.period_start <= NOW() \
               AND bp.period_end > NOW() \
             GROUP BY o.pages_limit, bp.period_start, bp.period_end",
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
    } else {
        sqlx::query(
            "SELECT \
                 COALESCE(SUM(ue.pages), 0) AS pages_used, \
                 o.pages_limit, \
                 bp.period_start, \
                 bp.period_end \
             FROM billing_periods bp \
             JOIN organizations o ON o.id = bp.org_id \
             LEFT JOIN usage_events ue \
                 ON ue.org_id = bp.org_id \
                AND ue.created_at >= bp.period_start \
                AND ue.created_at < bp.period_end \
             WHERE bp.org_id = $1 \
               AND bp.period_start <= NOW() \
               AND bp.period_end > NOW() \
             GROUP BY o.pages_limit, bp.period_start, bp.period_end",
        )
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
    };

    let row = row.ok_or_else(|| {
        AppError::NotFound("No active billing period for this organisation".into())
    })?;

    let pages_used: i64 = row.get("pages_used");
    let pages_limit: i64 = row.get("pages_limit");
    let period_start: DateTime<Utc> = row.get("period_start");
    let period_end: DateTime<Utc> = row.get("period_end");

    let percent_used = if pages_limit > 0 {
        (pages_used as f64 / pages_limit as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(CurrentUsageResponse {
        org_id,
        pages_used,
        pages_limit,
        percent_used,
        period_start,
        period_end,
    }))
}

// ---------------------------------------------------------------------------

async fn usage_history(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, AppError> {
    let org_id = claims.org_id;
    let user_filter = visibility_filter(&claims);

    let granularity = params.granularity.unwrap_or(Granularity::Day);
    let trunc = match granularity {
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
    };

    let end_date = params.end_date.unwrap_or_else(|| Utc::now().date_naive());
    let start_date = params.start_date.unwrap_or_else(|| {
        end_date - chrono::Duration::days(30)
    });

    let rows = sqlx::query(
        "SELECT \
             date_trunc($1, ue.created_at) AS timestamp, \
             COALESCE(SUM(ue.pages), 0) AS pages_used \
         FROM usage_events ue \
         WHERE ue.org_id = $2 \
           AND ($3::uuid IS NULL OR ue.user_id = $3) \
           AND ue.created_at >= $4::date::timestamptz \
           AND ue.created_at < ($5::date + INTERVAL '1 day')::timestamptz \
         GROUP BY 1 \
         ORDER BY 1",
    )
    .bind(trunc)
    .bind(org_id)
    .bind(user_filter)
    .bind(start_date)
    .bind(end_date)
    .fetch_all(&state.db)
    .await?;

    let data: Vec<UsageDataPoint> = rows
        .into_iter()
        .map(|r| UsageDataPoint {
            timestamp: r.get("timestamp"),
            pages_used: r.get("pages_used"),
        })
        .collect();

    Ok(Json(HistoryResponse {
        org_id,
        granularity: trunc.to_string(),
        start_date,
        end_date,
        data,
    }))
}

// ---------------------------------------------------------------------------

async fn usage_breakdown(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<BreakdownQuery>,
) -> Result<Json<BreakdownResponse>, AppError> {
    let org_id = claims.org_id;

    // Only admins/owners can view breakdown by team or user.
    if !is_admin_or_owner(&claims) {
        return Err(AppError::Forbidden(
            "Only admins and owners can view usage breakdowns".into(),
        ));
    }

    let dimension = params.dimension.unwrap_or(BreakdownDimension::User);
    let end_date = params.end_date.unwrap_or_else(|| Utc::now().date_naive());
    let start_date = params.start_date.unwrap_or_else(|| {
        end_date - chrono::Duration::days(30)
    });

    let raw_rows = match dimension {
        BreakdownDimension::Team => {
            sqlx::query(
                "SELECT \
                     t.id, \
                     t.name AS label, \
                     COALESCE(SUM(ue.pages), 0) AS pages_used \
                 FROM teams t \
                 LEFT JOIN users u ON u.team_id = t.id \
                 LEFT JOIN usage_events ue \
                     ON ue.user_id = u.id \
                    AND ue.created_at >= $2::date::timestamptz \
                    AND ue.created_at < ($3::date + INTERVAL '1 day')::timestamptz \
                 WHERE t.org_id = $1 \
                 GROUP BY t.id, t.name \
                 ORDER BY pages_used DESC",
            )
            .bind(org_id)
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&state.db)
            .await?
        }
        BreakdownDimension::User => {
            sqlx::query(
                "SELECT \
                     u.id, \
                     COALESCE(u.display_name, u.email) AS label, \
                     COALESCE(SUM(ue.pages), 0) AS pages_used \
                 FROM users u \
                 LEFT JOIN usage_events ue \
                     ON ue.user_id = u.id \
                    AND ue.created_at >= $2::date::timestamptz \
                    AND ue.created_at < ($3::date + INTERVAL '1 day')::timestamptz \
                 WHERE u.org_id = $1 \
                 GROUP BY u.id, u.display_name, u.email \
                 ORDER BY pages_used DESC",
            )
            .bind(org_id)
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&state.db)
            .await?
        }
        BreakdownDimension::ActionType => {
            sqlx::query(
                "SELECT \
                     uuid_generate_v5(uuid_nil(), ue.action_type) AS id, \
                     ue.action_type AS label, \
                     COALESCE(SUM(ue.pages), 0) AS pages_used \
                 FROM usage_events ue \
                 WHERE ue.org_id = $1 \
                   AND ue.created_at >= $2::date::timestamptz \
                   AND ue.created_at < ($3::date + INTERVAL '1 day')::timestamptz \
                 GROUP BY ue.action_type \
                 ORDER BY pages_used DESC",
            )
            .bind(org_id)
            .bind(start_date)
            .bind(end_date)
            .fetch_all(&state.db)
            .await?
        }
    };

    let mut entries: Vec<BreakdownEntry> = raw_rows
        .into_iter()
        .map(|r| BreakdownEntry {
            id: r.get("id"),
            label: r.get("label"),
            pages_used: r.get("pages_used"),
            percent_of_total: 0.0,
        })
        .collect();

    // Compute percent_of_total now that we have all rows.
    let total: i64 = entries.iter().map(|e| e.pages_used).sum();
    for e in &mut entries {
        e.percent_of_total = if total > 0 {
            (e.pages_used as f64 / total as f64) * 100.0
        } else {
            0.0
        };
    }

    let dim_label = match dimension {
        BreakdownDimension::Team => "team",
        BreakdownDimension::User => "user",
        BreakdownDimension::ActionType => "action_type",
    };

    Ok(Json(BreakdownResponse {
        org_id,
        dimension: dim_label.to_string(),
        entries,
    }))
}

// ---------------------------------------------------------------------------

async fn current_plan(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<PlanResponse>, AppError> {
    let org_id = claims.org_id;

    let row = sqlx::query(
        "SELECT p.tier, p.price_cents, p.pages_limit, p.features \
         FROM organizations o \
         JOIN plans p ON p.id = o.plan_id \
         WHERE o.id = $1",
    )
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Organisation or plan not found".into()))?;

    Ok(Json(PlanResponse {
        org_id,
        tier: row.get("tier"),
        price_cents: row.get("price_cents"),
        pages_limit: row.get("pages_limit"),
        features: row.get("features"),
    }))
}

// ---------------------------------------------------------------------------

async fn create_alert(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateAlertRequest>,
) -> Result<Json<AlertResponse>, AppError> {
    let org_id = claims.org_id;

    if !is_admin_or_owner(&claims) {
        return Err(AppError::Forbidden(
            "Only admins and owners can configure usage alerts".into(),
        ));
    }

    let valid_thresholds = [80, 100, 150];
    if !valid_thresholds.contains(&body.threshold_percent) {
        return Err(AppError::BadRequest(format!(
            "threshold_percent must be one of {:?}",
            valid_thresholds
        )));
    }

    let fallback_email = body.email.clone().unwrap_or_else(|| {
        // Will be resolved from the org admin email at query time if empty.
        String::new()
    });

    let row = sqlx::query(
        "INSERT INTO usage_alerts (org_id, threshold_percent, email, created_by) \
         VALUES ($1, $2, NULLIF($3, ''), $4) \
         ON CONFLICT (org_id, threshold_percent) \
             DO UPDATE SET email = EXCLUDED.email, \
                           created_by = EXCLUDED.created_by, \
                           updated_at = NOW() \
         RETURNING \
             id, \
             org_id, \
             threshold_percent, \
             COALESCE(email, ( \
                 SELECT u.email FROM users u \
                 WHERE u.org_id = $1 AND u.role = 'owner' \
                 LIMIT 1 \
             )) AS email, \
             created_at",
    )
    .bind(org_id)
    .bind(body.threshold_percent)
    .bind(&fallback_email)
    .bind(claims.sub)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AlertResponse {
        id: row.get("id"),
        org_id: row.get("org_id"),
        threshold_percent: row.get("threshold_percent"),
        email: row.get("email"),
        created_at: row.get("created_at"),
    }))
}

// ---------------------------------------------------------------------------

async fn list_invoices(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<InvoicesQuery>,
) -> Result<Json<PaginatedInvoices>, AppError> {
    let org_id = claims.org_id;

    if !is_admin_or_owner(&claims) {
        return Err(AppError::Forbidden(
            "Only admins and owners can view invoices".into(),
        ));
    }

    let per_page = params.per_page.unwrap_or(20).min(100).max(1);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let total: Option<i64> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invoices WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;
    let total = total.unwrap_or(0);

    let rows = sqlx::query(
        "SELECT id, org_id, period_start, period_end, amount_cents, status, created_at \
         FROM invoices \
         WHERE org_id = $1 \
         ORDER BY period_start DESC \
         LIMIT $2 OFFSET $3",
    )
    .bind(org_id)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let data: Vec<InvoiceSummary> = rows
        .into_iter()
        .map(|r| InvoiceSummary {
            id: r.get("id"),
            org_id: r.get("org_id"),
            period_start: r.get("period_start"),
            period_end: r.get("period_end"),
            amount_cents: r.get("amount_cents"),
            status: r.get("status"),
            created_at: r.get("created_at"),
        })
        .collect();

    Ok(Json(PaginatedInvoices {
        data,
        page,
        per_page,
        total,
    }))
}

// ---------------------------------------------------------------------------

async fn get_invoice(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(invoice_id): Path<Uuid>,
) -> Result<Json<InvoiceDetailResponse>, AppError> {
    let org_id = claims.org_id;

    if !is_admin_or_owner(&claims) {
        return Err(AppError::Forbidden(
            "Only admins and owners can view invoice details".into(),
        ));
    }

    let invoice = sqlx::query(
        "SELECT id, org_id, period_start, period_end, amount_cents, status, created_at, paid_at \
         FROM invoices \
         WHERE id = $1 AND org_id = $2",
    )
    .bind(invoice_id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Invoice not found".into()))?;

    let line_item_rows = sqlx::query(
        "SELECT description, quantity, unit_price_cents, total_cents \
         FROM invoice_line_items \
         WHERE invoice_id = $1 \
         ORDER BY id",
    )
    .bind(invoice_id)
    .fetch_all(&state.db)
    .await?;

    let line_items: Vec<InvoiceLineItem> = line_item_rows
        .into_iter()
        .map(|r| InvoiceLineItem {
            description: r.get("description"),
            quantity: r.get("quantity"),
            unit_price_cents: r.get("unit_price_cents"),
            total_cents: r.get("total_cents"),
        })
        .collect();

    Ok(Json(InvoiceDetailResponse {
        id: invoice.get("id"),
        org_id: invoice.get("org_id"),
        period_start: invoice.get("period_start"),
        period_end: invoice.get("period_end"),
        amount_cents: invoice.get("amount_cents"),
        status: invoice.get("status"),
        line_items,
        created_at: invoice.get("created_at"),
        paid_at: invoice.get("paid_at"),
    }))
}
