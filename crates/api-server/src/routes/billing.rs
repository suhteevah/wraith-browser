use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::billing::{BillingError, BillingService, WebhookAction};
use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    /// Stripe Price ID for the target plan (e.g. `price_xxx`).
    pub price_id: String,
    /// URL to redirect to after successful checkout.
    pub success_url: String,
    /// URL to redirect to if the user cancels checkout.
    pub cancel_url: String,
}

#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    /// Stripe Checkout Session URL the frontend should redirect to.
    pub checkout_url: String,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionResponse {
    pub org_id: Uuid,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub status: String,
    pub price_id: Option<String>,
    pub current_period_start: Option<i64>,
    pub current_period_end: Option<i64>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Public billing routes (no auth — webhook uses Stripe signature verification).
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/webhooks/stripe", post(stripe_webhook))
}

/// Protected billing routes (require valid JWT via auth middleware).
pub fn protected_router() -> Router<AppState> {
    Router::new()
        .route("/checkout", post(create_checkout))
        .route("/subscription", get(get_subscription))
}

// ---------------------------------------------------------------------------
// POST /webhooks/stripe — Stripe webhook (no auth, signature-verified)
// ---------------------------------------------------------------------------

async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let signature = headers
        .get("Stripe-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing Stripe-Signature header".into()))?;

    let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").map_err(|_| {
        AppError::Internal("STRIPE_WEBHOOK_SECRET not configured".into())
    })?;
    let api_key = std::env::var("STRIPE_SECRET_KEY").map_err(|_| {
        AppError::Internal("STRIPE_SECRET_KEY not configured".into())
    })?;

    let svc = BillingService::new(api_key, webhook_secret);

    let action = svc
        .handle_webhook(&body, signature)
        .map_err(|e| match e {
            BillingError::InvalidSignature | BillingError::MissingSignatureHeader => {
                AppError::Unauthorized(e.to_string())
            }
            BillingError::TimestampTooOld => AppError::BadRequest(e.to_string()),
            BillingError::UnsupportedEvent(ref evt) => {
                tracing::debug!(event_type = %evt, "Ignoring unsupported Stripe event");
                // Return 200 so Stripe does not retry unsupported events.
                return AppError::BadRequest(e.to_string());
            }
            _ => AppError::Internal(e.to_string()),
        });

    // For unsupported events we still return 200 to prevent Stripe retries.
    let action = match action {
        Ok(a) => a,
        Err(AppError::BadRequest(msg)) if msg.contains("Unsupported") => {
            tracing::debug!("Acknowledged unsupported Stripe event");
            return Ok(StatusCode::OK);
        }
        Err(e) => return Err(e),
    };

    // Persist the webhook outcome in the database.
    match action {
        WebhookAction::SubscriptionUpdated {
            customer_id,
            subscription_id,
            status,
            price_id,
        } => {
            sqlx::query(
                "UPDATE organizations \
                 SET stripe_subscription_id = $1, \
                     stripe_subscription_status = $2, \
                     stripe_price_id = $3, \
                     updated_at = NOW() \
                 WHERE stripe_customer_id = $4",
            )
            .bind(&subscription_id)
            .bind(&status)
            .bind(&price_id)
            .bind(&customer_id)
            .execute(&state.db)
            .await?;

            tracing::info!(
                %customer_id,
                %subscription_id,
                %status,
                "Subscription updated in DB"
            );
        }

        WebhookAction::PaymentSucceeded {
            customer_id,
            invoice_id,
            amount_paid,
            currency,
        } => {
            sqlx::query(
                "INSERT INTO payment_events (stripe_customer_id, stripe_invoice_id, amount_paid, currency, event_type, created_at) \
                 VALUES ($1, $2, $3, $4, 'payment_succeeded', NOW()) \
                 ON CONFLICT (stripe_invoice_id, event_type) DO NOTHING",
            )
            .bind(&customer_id)
            .bind(&invoice_id)
            .bind(amount_paid)
            .bind(&currency)
            .execute(&state.db)
            .await?;

            tracing::info!(
                %customer_id,
                %invoice_id,
                amount_paid,
                %currency,
                "Payment succeeded recorded"
            );
        }

        WebhookAction::PaymentFailed {
            customer_id,
            invoice_id,
            amount_due,
            currency,
            attempt_count,
        } => {
            sqlx::query(
                "INSERT INTO payment_events (stripe_customer_id, stripe_invoice_id, amount_due, currency, attempt_count, event_type, created_at) \
                 VALUES ($1, $2, $3, $4, $5, 'payment_failed', NOW()) \
                 ON CONFLICT (stripe_invoice_id, event_type) DO UPDATE SET attempt_count = $5, updated_at = NOW()",
            )
            .bind(&customer_id)
            .bind(&invoice_id)
            .bind(amount_due)
            .bind(&currency)
            .bind(attempt_count as i64)
            .execute(&state.db)
            .await?;

            tracing::warn!(
                %customer_id,
                %invoice_id,
                amount_due,
                attempt_count,
                "Payment failure recorded"
            );
        }

        WebhookAction::SubscriptionDeleted {
            customer_id,
            subscription_id,
        } => {
            sqlx::query(
                "UPDATE organizations \
                 SET stripe_subscription_id = NULL, \
                     stripe_subscription_status = 'canceled', \
                     stripe_price_id = NULL, \
                     plan_id = (SELECT id FROM plans WHERE tier = 'free' LIMIT 1), \
                     updated_at = NOW() \
                 WHERE stripe_customer_id = $1",
            )
            .bind(&customer_id)
            .execute(&state.db)
            .await?;

            tracing::warn!(
                %customer_id,
                %subscription_id,
                "Subscription deleted — org downgraded to free tier"
            );
        }
    }

    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// POST /checkout — Create Stripe Checkout Session (requires auth)
// ---------------------------------------------------------------------------

async fn create_checkout(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CheckoutRequest>,
) -> Result<Json<CheckoutResponse>, AppError> {
    let org_id = claims.org_id;

    // Only admins/owners may initiate a plan change.
    if !matches!(claims.role.as_str(), "owner" | "admin") {
        return Err(AppError::Forbidden(
            "Only admins and owners can manage billing".into(),
        ));
    }

    let api_key = std::env::var("STRIPE_SECRET_KEY").map_err(|_| {
        AppError::Internal("STRIPE_SECRET_KEY not configured".into())
    })?;
    let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
    let svc = BillingService::new(api_key, webhook_secret);

    // Look up the Stripe customer ID for this org.
    let row = sqlx::query(
        "SELECT stripe_customer_id FROM organizations WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Organisation not found".into()))?;

    let customer_id: Option<String> = row.get("stripe_customer_id");
    let customer_id = customer_id.ok_or_else(|| {
        AppError::BadRequest(
            "Organisation has no Stripe customer. Please contact support.".into(),
        )
    })?;

    let session = svc
        .create_checkout_session(
            &customer_id,
            &body.price_id,
            &body.success_url,
            &body.cancel_url,
        )
        .await
        .map_err(|e| AppError::Internal(format!("Stripe checkout error: {e}")))?;

    let checkout_url = session.url.ok_or_else(|| {
        AppError::Internal("Stripe returned a session with no URL".into())
    })?;

    Ok(Json(CheckoutResponse {
        checkout_url,
        session_id: session.id,
    }))
}

// ---------------------------------------------------------------------------
// GET /subscription — Current subscription status (requires auth)
// ---------------------------------------------------------------------------

async fn get_subscription(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<SubscriptionResponse>, AppError> {
    let org_id = claims.org_id;

    let row = sqlx::query(
        "SELECT \
             stripe_customer_id, \
             stripe_subscription_id, \
             stripe_subscription_status, \
             stripe_price_id, \
             stripe_period_start, \
             stripe_period_end \
         FROM organizations \
         WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Organisation not found".into()))?;

    Ok(Json(SubscriptionResponse {
        org_id,
        stripe_customer_id: row.get("stripe_customer_id"),
        stripe_subscription_id: row.get("stripe_subscription_id"),
        status: row
            .get::<Option<String>, _>("stripe_subscription_status")
            .unwrap_or_else(|| "none".to_string()),
        price_id: row.get("stripe_price_id"),
        current_period_start: row.get("stripe_period_start"),
        current_period_end: row.get("stripe_period_end"),
    }))
}
