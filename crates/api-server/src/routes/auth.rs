use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

use crate::error::AppError;
use crate::middleware::Claims;
use crate::models::{User, UserRole};
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    pub org_name: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user: UserProfile,
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub org_id: Uuid,
    pub role: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Internal: refresh-token claims (distinct from access-token Claims)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefreshClaims {
    pub sub: Uuid,
    pub org_id: Uuid,
    pub role: String,
    /// Marks this as a refresh token so it cannot be used as an access token.
    pub token_type: String,
    pub iat: i64,
    pub exp: i64,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
        .route("/refresh", post(refresh))
        .route("/me", get(me))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a role string from the DB into a `UserRole`.
fn parse_role(s: &str) -> UserRole {
    match s {
        "owner" => UserRole::Owner,
        "admin" => UserRole::Admin,
        "member" => UserRole::Member,
        "viewer" => UserRole::Viewer,
        _ => UserRole::Viewer,
    }
}

/// Map a sqlx Row into a `User` struct.
fn user_from_row(row: sqlx::postgres::PgRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        password_hash: row.try_get("password_hash")?,
        display_name: row.try_get("display_name")?,
        org_id: row.try_get("org_id")?,
        role: parse_role(row.try_get::<String, _>("role")?.as_str()),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Issue an access + refresh token pair for the given user.
fn issue_tokens(
    user_id: Uuid,
    org_id: Uuid,
    role: &str,
    jwt_secret: &str,
    expiry_secs: i64,
) -> Result<(String, String, i64), AppError> {
    let now = Utc::now().timestamp();
    let key = EncodingKey::from_secret(jwt_secret.as_bytes());

    // Access token
    let access_claims = Claims {
        sub: user_id,
        org_id,
        role: role.to_string(),
        iat: now,
        exp: now + expiry_secs,
    };
    let access_token = encode(&Header::default(), &access_claims, &key)?;

    // Refresh token — longer-lived (7 days)
    let refresh_exp = now + 7 * 24 * 3600;
    let refresh_claims = RefreshClaims {
        sub: user_id,
        org_id,
        role: role.to_string(),
        token_type: "refresh".to_string(),
        iat: now,
        exp: refresh_exp,
    };
    let refresh_token = encode(&Header::default(), &refresh_claims, &key)?;

    Ok((access_token, refresh_token, expiry_secs))
}

/// Hash a plaintext password using Argon2id.
fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored Argon2 hash.
fn verify_password(password: &str, hash: &str) -> Result<(), AppError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("Invalid stored hash: {e}")))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AppError::Unauthorized("Invalid email or password".into()))
}

/// Convert a `UserRole` to its DB/serde string form.
fn role_to_string(role: &UserRole) -> String {
    match role {
        UserRole::Owner => "owner".to_string(),
        UserRole::Admin => "admin".to_string(),
        UserRole::Member => "member".to_string(),
        UserRole::Viewer => "viewer".to_string(),
    }
}

/// Generate a URL-friendly slug from an organisation name.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    // Look up user by email.
    let row = sqlx::query(
        r#"
        SELECT id, email, password_hash, display_name, org_id,
               role, created_at, updated_at
        FROM users
        WHERE email = $1
        "#,
    )
    .bind(&body.email)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid email or password".into()))?;

    let user = user_from_row(row)
        .map_err(|e| AppError::Internal(format!("Failed to parse user row: {e}")))?;

    // Verify password.
    verify_password(&body.password, &user.password_hash)?;

    let role_str = role_to_string(&user.role);
    let (access_token, refresh_token, expires_in) = issue_tokens(
        user.id,
        user.org_id,
        &role_str,
        &state.config.jwt_secret,
        state.config.jwt_expiry_secs,
    )?;

    Ok(Json(TokenResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    // Validate inputs.
    if body.email.is_empty() || !body.email.contains('@') {
        return Err(AppError::BadRequest("Invalid email address".into()));
    }
    if body.password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if body.org_name.is_empty() {
        return Err(AppError::BadRequest("Organisation name is required".into()));
    }

    // Check for existing user.
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM users WHERE email = $1"#,
    )
    .bind(&body.email)
    .fetch_optional(&state.db)
    .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A user with that email already exists".into(),
        ));
    }

    // Hash password (CPU-intensive — run on blocking thread).
    let password = body.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| AppError::Internal(format!("Blocking task failed: {e}")))??;

    let now = Utc::now();
    let org_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let slug = slugify(&body.org_name);
    let role_str = "owner";

    // users.display_name is NOT NULL in the schema. If the caller didn't
    // supply one, default to the email local-part.
    let display_name = body
        .display_name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| body.email.split('@').next().unwrap_or(&body.email).to_string());

    // Insert organisation and user in a transaction.
    let mut tx = state.db.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO organizations (id, name, slug, plan, created_at)
        VALUES ($1, $2, $3, 'free', $4)
        "#,
    )
    .bind(org_id)
    .bind(&body.org_name)
    .bind(&slug)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO users (id, email, password_hash, display_name, org_id, role, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(user_id)
    .bind(&body.email)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(org_id)
    .bind(role_str)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let (access_token, refresh_token, expires_in) = issue_tokens(
        user_id,
        org_id,
        role_str,
        &state.config.jwt_secret,
        state.config.jwt_expiry_secs,
    )?;

    Ok(Json(RegisterResponse {
        user: UserProfile {
            id: user_id,
            email: body.email,
            display_name: Some(display_name),
            org_id,
            role: role_str.to_string(),
            created_at: now,
            updated_at: now,
        },
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    let key = DecodingKey::from_secret(state.config.jwt_secret.as_bytes());
    let validation = Validation::default();

    // Decode as RefreshClaims (which includes token_type).
    let token_data = decode::<RefreshClaims>(&body.refresh_token, &key, &validation)
        .map_err(|_| AppError::Unauthorized("Invalid or expired refresh token".into()))?;

    let claims = token_data.claims;

    // Ensure this is actually a refresh token, not an access token being reused.
    if claims.token_type != "refresh" {
        return Err(AppError::Unauthorized(
            "Provided token is not a refresh token".into(),
        ));
    }

    // Verify the user still exists and is active.
    let _user: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM users WHERE id = $1"#,
    )
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await?;

    if _user.is_none() {
        return Err(AppError::Unauthorized("User no longer exists".into()));
    }

    // Issue a fresh token pair.
    let (access_token, refresh_token, expires_in) = issue_tokens(
        claims.sub,
        claims.org_id,
        &claims.role,
        &state.config.jwt_secret,
        state.config.jwt_expiry_secs,
    )?;

    Ok(Json(TokenResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

async fn me(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<UserProfile>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, email, password_hash, display_name, org_id,
               role, created_at, updated_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    let user = user_from_row(row)
        .map_err(|e| AppError::Internal(format!("Failed to parse user row: {e}")))?;

    Ok(Json(UserProfile {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        org_id: user.org_id,
        role: role_to_string(&user.role),
        created_at: user.created_at,
        updated_at: user.updated_at,
    }))
}
