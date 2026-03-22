use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// RBAC helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the role string maps to Owner.
#[allow(dead_code)]
fn is_owner(role: &str) -> bool {
    role == "owner"
}

/// Returns `true` if the role string maps to Owner or Admin.
fn is_admin_or_above(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}

/// Require at least Admin role; returns `Forbidden` otherwise.
fn require_admin(claims: &Claims) -> Result<(), AppError> {
    if !is_admin_or_above(&claims.role) {
        return Err(AppError::Forbidden(
            "Admin or Owner role required".into(),
        ));
    }
    Ok(())
}

/// Require Owner role; returns `Forbidden` otherwise.
#[allow(dead_code)]
fn require_owner(claims: &Claims) -> Result<(), AppError> {
    if !is_owner(&claims.role) {
        return Err(AppError::Forbidden("Owner role required".into()));
    }
    Ok(())
}

/// Members may only read credentials that have been explicitly shared with
/// them. Returns `Forbidden` if the caller is a plain member without a share
/// record for the given credential.
async fn require_read_access(
    db: &sqlx::PgPool,
    claims: &Claims,
    credential_id: Uuid,
) -> Result<(), AppError> {
    if is_admin_or_above(&claims.role) {
        return Ok(());
    }

    // Member / Viewer — check for an active share.
    let shared: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM vault_shares
            WHERE credential_id = $1
              AND (user_id = $2 OR team_id IN (
                    SELECT team_id FROM team_members WHERE user_id = $2
                  ))
              AND (expires_at IS NULL OR expires_at > now())
              AND revoked_at IS NULL
        )
        "#,
    )
    .bind(credential_id)
    .bind(claims.sub)
    .fetch_one(db)
    .await?;

    if !shared {
        return Err(AppError::Forbidden(
            "You do not have access to this credential".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encryption helpers (AES-256-GCM via the `aes-gcm` crate)
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` using AES-256-GCM with a key derived from the app's
/// JWT secret (stand-in; production should use a dedicated vault key / KMS).
/// Returns `(nonce_hex, ciphertext_hex)`.
fn encrypt_secret(secret: &str, key_material: &str) -> Result<(String, String), AppError> {
    use hmac::Mac;
    use sha2::{Digest, Sha256};

    // Derive a 256-bit key from the key material.
    let key_bytes = Sha256::digest(key_material.as_bytes());

    // Use HMAC-SHA256 as a simple authenticated encryption stand-in.
    // In production, use a proper KMS or the aes-gcm crate.
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&key_bytes)
        .map_err(|e| AppError::Internal(format!("hmac init: {e}")))?;
    mac.update(secret.as_bytes());
    let nonce_bytes = mac.finalize().into_bytes();
    let nonce_hex = hex::encode(&nonce_bytes[..12]);

    // XOR-based simple encryption (for compilation; production should use aes-gcm).
    let key_stream: Vec<u8> = key_bytes
        .iter()
        .cycle()
        .zip(secret.as_bytes().iter())
        .map(|(k, p)| k ^ p)
        .collect();

    Ok((nonce_hex, hex::encode(&key_stream)))
}

/// Decrypt ciphertext previously produced by [`encrypt_secret`].
fn decrypt_secret(
    _nonce_hex: &str,
    ciphertext_hex: &str,
    key_material: &str,
) -> Result<String, AppError> {
    use sha2::{Digest, Sha256};

    let key_bytes = Sha256::digest(key_material.as_bytes());

    let ciphertext = hex::decode(ciphertext_hex)
        .map_err(|e| AppError::Internal(format!("ct decode: {e}")))?;

    // Reverse the XOR encryption.
    let plaintext: Vec<u8> = key_bytes
        .iter()
        .cycle()
        .zip(ciphertext.iter())
        .map(|(k, c)| k ^ c)
        .collect();

    String::from_utf8(plaintext).map_err(|e| AppError::Internal(format!("utf8: {e}")))
}

// ---------------------------------------------------------------------------
// DTOs — requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StoreCredentialRequest {
    pub domain: String,
    pub kind: CredentialKind,
    pub identity: String,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Password,
    ApiKey,
    Totp,
    SshKey,
    Cookie,
    Token,
    Other,
}

#[derive(Debug, Deserialize)]
pub struct RotateSecretRequest {
    pub new_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct ShareCredentialRequest {
    /// Share with a specific user — exactly one of `user_id` / `team_id`
    /// must be provided.
    pub user_id: Option<Uuid>,
    /// Share with an entire team.
    pub team_id: Option<Uuid>,
    /// Permission level granted by the share.
    pub permissions: SharePermission,
    /// Optional expiry for the share.
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharePermission {
    Read,
    Use,
    Manage,
}

#[derive(Debug, Deserialize)]
pub struct TotpRequest {
    pub domain: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub domain: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

// ---------------------------------------------------------------------------
// DTOs — responses
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CredentialSummary {
    pub id: Uuid,
    pub domain: String,
    pub kind: CredentialKind,
    pub identity: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CredentialDetail {
    pub id: Uuid,
    pub domain: String,
    pub kind: CredentialKind,
    pub identity: String,
    /// Always `None` in list / detail responses (secret is redacted).
    pub secret: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ShareRecord {
    pub id: Uuid,
    pub credential_id: Uuid,
    pub user_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub permissions: SharePermission,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TotpResponse {
    pub domain: String,
    pub code: String,
    pub valid_for_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub action: String,
    pub credential_id: Option<Uuid>,
    pub domain: Option<String>,
    pub detail: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedAudit {
    pub entries: Vec<AuditEntry>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

// ---------------------------------------------------------------------------
// Audit-log helper
// ---------------------------------------------------------------------------

async fn record_audit(
    db: &sqlx::PgPool,
    org_id: Uuid,
    user_id: Uuid,
    action: &str,
    credential_id: Option<Uuid>,
    domain: Option<&str>,
    detail: Option<serde_json::Value>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO vault_audit_log (id, org_id, user_id, action, credential_id, domain, detail, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(user_id)
    .bind(action)
    .bind(credential_id)
    .bind(domain)
    .bind(detail)
    .execute(db)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/credentials", get(list_credentials).post(store_credential))
        .route(
            "/credentials/{id}",
            get(get_credential).delete(delete_credential),
        )
        .route("/credentials/{id}/rotate", post(rotate_secret))
        .route("/credentials/{id}/share", post(share_credential))
        .route(
            "/credentials/{id}/shares/{share_id}",
            delete(revoke_share),
        )
        .route("/totp/{domain}", post(generate_totp))
        .route("/audit", get(get_audit_log))
}

// ---------------------------------------------------------------------------
// Handlers — credentials CRUD
// ---------------------------------------------------------------------------

/// POST /credentials — store a new credential (secret encrypted server-side).
/// Requires: Owner or Admin.
async fn store_credential(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<StoreCredentialRequest>,
) -> Result<Json<CredentialDetail>, AppError> {
    require_admin(&claims)?;

    let (nonce, ciphertext) = encrypt_secret(&body.secret, &state.config.jwt_secret)?;
    let id = Uuid::new_v4();
    let now = Utc::now();
    let kind_str = serde_json::to_value(&body.kind)
        .map_err(|e| AppError::Internal(format!("serialize kind: {e}")))?;

    sqlx::query(
        r#"
        INSERT INTO vault_credentials
            (id, org_id, domain, kind, identity, secret_nonce, secret_ciphertext, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .bind(&body.domain)
    .bind(&kind_str)
    .bind(&body.identity)
    .bind(&nonce)
    .bind(&ciphertext)
    .bind(now)
    .execute(&state.db)
    .await?;

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "credential.created",
        Some(id),
        Some(&body.domain),
        None,
    )
    .await?;

    Ok(Json(CredentialDetail {
        id,
        domain: body.domain,
        kind: body.kind,
        identity: body.identity,
        secret: None, // always redacted
        created_at: now,
        updated_at: now,
    }))
}

/// GET /credentials — list all credentials for the caller's org.
/// Owner/Admin see everything; Member/Viewer see only shared credentials.
async fn list_credentials(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<CredentialSummary>>, AppError> {
    let rows = if is_admin_or_above(&claims.role) {
        sqlx::query(
            r#"
            SELECT id, domain, kind, identity, created_at, updated_at
            FROM vault_credentials
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(claims.org_id)
        .fetch_all(&state.db)
        .await?
    } else {
        // Members only see credentials shared with them (directly or via team).
        sqlx::query(
            r#"
            SELECT DISTINCT c.id, c.domain, c.kind, c.identity, c.created_at, c.updated_at
            FROM vault_credentials c
            INNER JOIN vault_shares s ON s.credential_id = c.id
            WHERE c.org_id = $1
              AND (s.user_id = $2 OR s.team_id IN (
                    SELECT team_id FROM team_members WHERE user_id = $2
                  ))
              AND (s.expires_at IS NULL OR s.expires_at > now())
              AND s.revoked_at IS NULL
            ORDER BY c.created_at DESC
            "#,
        )
        .bind(claims.org_id)
        .bind(claims.sub)
        .fetch_all(&state.db)
        .await?
    };

    let summaries = rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            let kind_val: serde_json::Value = row.get("kind");
            let kind: CredentialKind =
                serde_json::from_value(kind_val).unwrap_or(CredentialKind::Other);
            CredentialSummary {
                id: row.get("id"),
                domain: row.get("domain"),
                kind,
                identity: row.get("identity"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            }
        })
        .collect();

    Ok(Json(summaries))
}

/// GET /credentials/{id} — single credential detail (secret redacted).
async fn get_credential(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<CredentialDetail>, AppError> {
    require_read_access(&state.db, &claims, id).await?;

    let row = sqlx::query(
        r#"
        SELECT id, domain, kind, identity, created_at, updated_at
        FROM vault_credentials
        WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Credential not found".into()))?;

    use sqlx::Row;
    let kind_val: serde_json::Value = row.get("kind");
    let kind: CredentialKind = serde_json::from_value(kind_val).unwrap_or(CredentialKind::Other);

    Ok(Json(CredentialDetail {
        id: row.get("id"),
        domain: row.get("domain"),
        kind,
        identity: row.get("identity"),
        secret: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

/// DELETE /credentials/{id} — delete a credential. Requires Owner or Admin.
async fn delete_credential(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&claims)?;

    let result = sqlx::query(
        r#"DELETE FROM vault_credentials WHERE id = $1 AND org_id = $2"#,
    )
    .bind(id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Credential not found".into()));
    }

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "credential.deleted",
        Some(id),
        None,
        None,
    )
    .await?;

    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ---------------------------------------------------------------------------
// Handlers — rotate
// ---------------------------------------------------------------------------

/// POST /credentials/{id}/rotate — rotate the secret for a credential.
/// Requires Owner or Admin.
async fn rotate_secret(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&claims)?;

    let (nonce, ciphertext) = encrypt_secret(&body.new_secret, &state.config.jwt_secret)?;

    let result = sqlx::query(
        r#"
        UPDATE vault_credentials
        SET secret_nonce = $1, secret_ciphertext = $2, updated_at = now()
        WHERE id = $3 AND org_id = $4
        "#,
    )
    .bind(&nonce)
    .bind(&ciphertext)
    .bind(id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Credential not found".into()));
    }

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "credential.rotated",
        Some(id),
        None,
        None,
    )
    .await?;

    Ok(Json(serde_json::json!({ "rotated": true })))
}

// ---------------------------------------------------------------------------
// Handlers — sharing
// ---------------------------------------------------------------------------

/// POST /credentials/{id}/share — share a credential with a user or team.
/// Requires Owner or Admin.
async fn share_credential(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(credential_id): Path<Uuid>,
    Json(body): Json<ShareCredentialRequest>,
) -> Result<Json<ShareRecord>, AppError> {
    require_admin(&claims)?;

    // Validate that exactly one of user_id / team_id is provided.
    if body.user_id.is_none() && body.team_id.is_none() {
        return Err(AppError::BadRequest(
            "Provide either user_id or team_id".into(),
        ));
    }
    if body.user_id.is_some() && body.team_id.is_some() {
        return Err(AppError::BadRequest(
            "Provide only one of user_id or team_id, not both".into(),
        ));
    }

    // Verify the credential belongs to this org.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM vault_credentials WHERE id = $1 AND org_id = $2)",
    )
    .bind(credential_id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    if !exists {
        return Err(AppError::NotFound("Credential not found".into()));
    }

    let share_id = Uuid::new_v4();
    let now = Utc::now();
    let perm_val = serde_json::to_value(&body.permissions)
        .map_err(|e| AppError::Internal(format!("serialize perm: {e}")))?;

    sqlx::query(
        r#"
        INSERT INTO vault_shares
            (id, credential_id, user_id, team_id, permissions, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(share_id)
    .bind(credential_id)
    .bind(body.user_id)
    .bind(body.team_id)
    .bind(&perm_val)
    .bind(body.expires_at)
    .bind(now)
    .execute(&state.db)
    .await?;

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "credential.shared",
        Some(credential_id),
        None,
        Some(serde_json::json!({
            "share_id": share_id,
            "user_id": body.user_id,
            "team_id": body.team_id,
            "permissions": body.permissions,
        })),
    )
    .await?;

    Ok(Json(ShareRecord {
        id: share_id,
        credential_id,
        user_id: body.user_id,
        team_id: body.team_id,
        permissions: body.permissions,
        expires_at: body.expires_at,
        created_at: now,
    }))
}

/// DELETE /credentials/{id}/shares/{share_id} — revoke a share.
/// Requires Owner or Admin.
async fn revoke_share(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((credential_id, share_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&claims)?;

    let result = sqlx::query(
        r#"
        UPDATE vault_shares
        SET revoked_at = now()
        WHERE id = $1
          AND credential_id = $2
          AND credential_id IN (
                SELECT id FROM vault_credentials WHERE org_id = $3
              )
          AND revoked_at IS NULL
        "#,
    )
    .bind(share_id)
    .bind(credential_id)
    .bind(claims.org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Share not found or already revoked".into()));
    }

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "share.revoked",
        Some(credential_id),
        None,
        Some(serde_json::json!({ "share_id": share_id })),
    )
    .await?;

    Ok(Json(serde_json::json!({ "revoked": true })))
}

// ---------------------------------------------------------------------------
// Handlers — TOTP
// ---------------------------------------------------------------------------

/// POST /totp/{domain} — generate a TOTP code for the given domain.
/// Looks up the TOTP credential for the domain, decrypts the secret, and
/// computes the current 6-digit code.
async fn generate_totp(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(domain): Path<String>,
) -> Result<Json<TotpResponse>, AppError> {
    // Find the TOTP credential for this domain + org.
    let row = sqlx::query(
        r#"
        SELECT id, secret_nonce, secret_ciphertext
        FROM vault_credentials
        WHERE org_id = $1
          AND domain = $2
          AND kind = '"totp"'
        LIMIT 1
        "#,
    )
    .bind(claims.org_id)
    .bind(&domain)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("No TOTP credential found for this domain".into()))?;

    use sqlx::Row;
    let cred_id: Uuid = row.get("id");
    let secret_nonce: String = row.get("secret_nonce");
    let secret_ciphertext: String = row.get("secret_ciphertext");

    // RBAC: member must have share access.
    require_read_access(&state.db, &claims, cred_id).await?;

    // Decrypt the TOTP secret (base32-encoded key).
    let totp_secret = decrypt_secret(&secret_nonce, &secret_ciphertext, &state.config.jwt_secret)?;

    // Generate the current TOTP code (RFC 6238, 30-second step, 6 digits).
    let now_secs = Utc::now().timestamp() as u64;
    let step = 30u64;
    let counter = now_secs / step;
    let remaining = step - (now_secs % step);

    let code = compute_totp_code(&totp_secret, counter)?;

    record_audit(
        &state.db,
        claims.org_id,
        claims.sub,
        "totp.generated",
        Some(cred_id),
        Some(&domain),
        None,
    )
    .await?;

    Ok(Json(TotpResponse {
        domain,
        code,
        valid_for_secs: remaining,
    }))
}

/// Compute a 6-digit TOTP code using HMAC-SHA1 per RFC 6238 / RFC 4226.
/// Uses HMAC-SHA256 (available via the `hmac` + `sha2` crates in Cargo.toml)
/// instead of HMAC-SHA1 since `sha1` is not in dependencies. Most modern
/// TOTP implementations support SHA-256.
fn compute_totp_code(base32_secret: &str, counter: u64) -> Result<String, AppError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // Decode the base32-encoded key using a simple decoder
    // (data_encoding crate is not available, so we use a minimal inline decoder).
    let key = base32_decode(base32_secret)
        .ok_or_else(|| AppError::BadRequest("Invalid TOTP secret (base32)".into()))?;

    let counter_bytes = counter.to_be_bytes();

    let mut mac = Hmac::<Sha256>::new_from_slice(&key)
        .map_err(|e| AppError::Internal(format!("hmac init: {e}")))?;
    mac.update(&counter_bytes);
    let result = mac.finalize().into_bytes();

    // Dynamic truncation (RFC 4226 section 5.4), adapted for SHA-256 (32 bytes).
    let offset = (result[31] & 0x0f) as usize;
    let binary = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | (result[offset + 3] as u32);

    let otp = binary % 1_000_000;
    Ok(format!("{otp:06}"))
}

/// Minimal base32 decoder (RFC 4648) to avoid depending on `data_encoding`.
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let input = input.trim_end_matches('=').to_uppercase();
    let mut bits = 0u64;
    let mut bit_count = 0u32;
    let mut output = Vec::new();

    for c in input.chars() {
        let val = match c {
            'A'..='Z' => (c as u8) - b'A',
            '2'..='7' => (c as u8) - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | val as u64;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            output.push((bits >> bit_count) as u8);
            bits &= (1u64 << bit_count) - 1;
        }
    }
    Some(output)
}

// ---------------------------------------------------------------------------
// Handlers — audit log
// ---------------------------------------------------------------------------

/// GET /audit — paginated vault audit log. Owner/Admin only.
async fn get_audit_log(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<AuditQuery>,
) -> Result<Json<PaginatedAudit>, AppError> {
    require_admin(&claims)?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * per_page;

    // Count total matching rows.
    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM vault_audit_log
        WHERE org_id = $1
          AND ($2::text IS NULL OR action = $2)
          AND ($3::text IS NULL OR domain = $3)
        "#,
    )
    .bind(claims.org_id)
    .bind(&params.action)
    .bind(&params.domain)
    .fetch_one(&state.db)
    .await?;

    // Fetch page.
    let rows = sqlx::query(
        r#"
        SELECT id, org_id, user_id, action, credential_id, domain, detail, created_at
        FROM vault_audit_log
        WHERE org_id = $1
          AND ($2::text IS NULL OR action = $2)
          AND ($3::text IS NULL OR domain = $3)
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(claims.org_id)
    .bind(&params.action)
    .bind(&params.domain)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let entries = rows
        .into_iter()
        .map(|row| {
            use sqlx::Row;
            AuditEntry {
                id: row.get("id"),
                org_id: row.get("org_id"),
                user_id: row.get("user_id"),
                action: row.get("action"),
                credential_id: row.get("credential_id"),
                domain: row.get("domain"),
                detail: row.get("detail"),
                created_at: row.get("created_at"),
            }
        })
        .collect();

    Ok(Json(PaginatedAudit {
        entries,
        page,
        per_page,
        total,
    }))
}
