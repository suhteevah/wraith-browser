use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::models::UserRole;
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTeamRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TeamResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct TeamDetailResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub members: Vec<TeamMemberResponse>,
}

#[derive(Debug, Serialize)]
pub struct TeamMemberResponse {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AddMemberRequest {
    pub user_id: Uuid,
    pub role: TeamMemberRole,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemberRoleRequest {
    pub role: TeamMemberRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl std::fmt::Display for TeamMemberRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TeamMemberRole::Owner => write!(f, "owner"),
            TeamMemberRole::Admin => write!(f, "admin"),
            TeamMemberRole::Member => write!(f, "member"),
            TeamMemberRole::Viewer => write!(f, "viewer"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InviteRequest {
    pub email: String,
    pub role: TeamMemberRole,
}

#[derive(Debug, Serialize)]
pub struct InviteResponse {
    pub id: Uuid,
    pub team_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub invited_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_teams).post(create_team))
        .route("/{id}", get(get_team).put(update_team).delete(delete_team))
        .route("/{id}/members", post(add_member))
        .route(
            "/{id}/members/{user_id}",
            put(update_member_role).delete(remove_member),
        )
        .route("/{id}/invite", post(invite_by_email))
}

// ---------------------------------------------------------------------------
// RBAC helpers
// ---------------------------------------------------------------------------

/// Returns the caller's org-level role parsed from JWT claims.
fn org_role(claims: &Claims) -> Result<UserRole, AppError> {
    match claims.role.as_str() {
        "owner" => Ok(UserRole::Owner),
        "admin" => Ok(UserRole::Admin),
        "member" => Ok(UserRole::Member),
        "viewer" => Ok(UserRole::Viewer),
        other => Err(AppError::Internal(format!("Unknown org role: {other}"))),
    }
}

/// Returns true when the org-level role may manage any team.
fn is_org_manager(role: &UserRole) -> bool {
    matches!(role, UserRole::Owner | UserRole::Admin)
}

/// Fetch the caller's team-level role (if they are a member of the team).
async fn team_role_for_user(
    db: &sqlx::PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<Option<TeamMemberRole>, AppError> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT role FROM team_members WHERE team_id = $1 AND user_id = $2",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    match row.as_deref() {
        Some("owner") => Ok(Some(TeamMemberRole::Owner)),
        Some("admin") => Ok(Some(TeamMemberRole::Admin)),
        Some("member") => Ok(Some(TeamMemberRole::Member)),
        Some("viewer") => Ok(Some(TeamMemberRole::Viewer)),
        Some(other) => Err(AppError::Internal(format!(
            "Unknown team member role: {other}"
        ))),
        None => Ok(None),
    }
}

/// Require that the caller can manage the given team.
/// Org owners/admins always can; team owners/admins can manage their team.
async fn require_team_manager(
    db: &sqlx::PgPool,
    claims: &Claims,
    team_id: Uuid,
) -> Result<(), AppError> {
    let org = org_role(claims)?;
    if is_org_manager(&org) {
        return Ok(());
    }
    match team_role_for_user(db, team_id, claims.sub).await? {
        Some(TeamMemberRole::Owner) | Some(TeamMemberRole::Admin) => Ok(()),
        _ => Err(AppError::Forbidden(
            "You must be a team admin or org admin to perform this action".into(),
        )),
    }
}

/// Require that the caller can view the given team.
/// Org owners/admins or any team member can view.
async fn require_team_viewer(
    db: &sqlx::PgPool,
    claims: &Claims,
    team_id: Uuid,
) -> Result<(), AppError> {
    let org = org_role(claims)?;
    if is_org_manager(&org) {
        return Ok(());
    }
    match team_role_for_user(db, team_id, claims.sub).await? {
        Some(_) => Ok(()),
        None => Err(AppError::Forbidden(
            "You are not a member of this team".into(),
        )),
    }
}

/// Only org owners can delete teams.
fn require_org_owner(claims: &Claims) -> Result<(), AppError> {
    let org = org_role(claims)?;
    if org == UserRole::Owner {
        return Ok(());
    }
    Err(AppError::Forbidden(
        "Only organisation owners can delete teams".into(),
    ))
}

fn row_to_team(row: sqlx::postgres::PgRow) -> TeamResponse {
    TeamResponse {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Ensure the team belongs to the caller's org and return it.
async fn fetch_team_in_org(
    db: &sqlx::PgPool,
    team_id: Uuid,
    org_id: Uuid,
) -> Result<TeamResponse, AppError> {
    let team = sqlx::query(
        "SELECT id, org_id, name, description, created_at, updated_at \
         FROM teams WHERE id = $1 AND org_id = $2",
    )
    .bind(team_id)
    .bind(org_id)
    .fetch_optional(db)
    .await?
    .map(row_to_team)
    .ok_or_else(|| AppError::NotFound("Team not found".into()))?;

    Ok(team)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET / — list all teams in the caller's organisation.
async fn list_teams(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<TeamResponse>>, AppError> {
    let rows = sqlx::query(
        "SELECT id, org_id, name, description, created_at, updated_at \
         FROM teams WHERE org_id = $1 ORDER BY name",
    )
    .bind(claims.org_id)
    .fetch_all(&state.db)
    .await?;

    let teams: Vec<TeamResponse> = rows.into_iter().map(row_to_team).collect();
    Ok(Json(teams))
}

/// POST / — create a new team.
async fn create_team(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateTeamRequest>,
) -> Result<Json<TeamResponse>, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Team name must not be empty".into()));
    }

    let now = Utc::now();
    let team_id = Uuid::new_v4();

    // Insert the team.
    sqlx::query(
        "INSERT INTO teams (id, org_id, name, description, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $5)",
    )
    .bind(team_id)
    .bind(claims.org_id)
    .bind(&name)
    .bind(&body.description)
    .bind(now)
    .execute(&state.db)
    .await?;

    // The creator is automatically the team owner.
    sqlx::query(
        "INSERT INTO team_members (team_id, user_id, role, joined_at) \
         VALUES ($1, $2, 'owner', $3)",
    )
    .bind(team_id)
    .bind(claims.sub)
    .bind(now)
    .execute(&state.db)
    .await?;

    let team = TeamResponse {
        id: team_id,
        org_id: claims.org_id,
        name,
        description: body.description,
        created_at: now,
        updated_at: now,
    };

    Ok(Json(team))
}

/// GET /:id — get team detail including members.
async fn get_team(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(team_id): Path<Uuid>,
) -> Result<Json<TeamDetailResponse>, AppError> {
    let team = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_viewer(&state.db, &claims, team_id).await?;

    let member_rows = sqlx::query(
        "SELECT tm.user_id, u.email, u.display_name, tm.role, tm.joined_at \
         FROM team_members tm \
         JOIN users u ON u.id = tm.user_id \
         WHERE tm.team_id = $1 \
         ORDER BY tm.joined_at",
    )
    .bind(team_id)
    .fetch_all(&state.db)
    .await?;

    let members: Vec<TeamMemberResponse> = member_rows
        .into_iter()
        .map(|r| TeamMemberResponse {
            user_id: r.get("user_id"),
            email: r.get("email"),
            display_name: r.get("display_name"),
            role: r.get("role"),
            joined_at: r.get("joined_at"),
        })
        .collect();

    Ok(Json(TeamDetailResponse {
        id: team.id,
        org_id: team.org_id,
        name: team.name,
        description: team.description,
        created_at: team.created_at,
        updated_at: team.updated_at,
        members,
    }))
}

/// PUT /:id — update team name and/or description.
async fn update_team(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(team_id): Path<Uuid>,
    Json(body): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_manager(&state.db, &claims, team_id).await?;

    if let Some(ref n) = body.name {
        if n.trim().is_empty() {
            return Err(AppError::BadRequest("Team name must not be empty".into()));
        }
    }

    let row = sqlx::query(
        "UPDATE teams \
         SET name = COALESCE($1, name), \
             description = COALESCE($2, description), \
             updated_at = NOW() \
         WHERE id = $3 AND org_id = $4 \
         RETURNING id, org_id, name, description, created_at, updated_at",
    )
    .bind(&body.name)
    .bind(&body.description)
    .bind(team_id)
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(row_to_team(row)))
}

/// DELETE /:id — delete a team. Only org owners/admins may do this.
async fn delete_team(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(team_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_org_owner(&claims)?;

    // Remove members first, then the team.
    sqlx::query("DELETE FROM team_members WHERE team_id = $1")
        .bind(team_id)
        .execute(&state.db)
        .await?;

    sqlx::query("DELETE FROM teams WHERE id = $1 AND org_id = $2")
        .bind(team_id)
        .bind(claims.org_id)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// POST /:id/members — add a member to the team.
async fn add_member(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(team_id): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> Result<Json<TeamMemberResponse>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_manager(&state.db, &claims, team_id).await?;

    // Verify the target user exists in the same org.
    let user = sqlx::query(
        "SELECT id, email, display_name FROM users WHERE id = $1 AND org_id = $2",
    )
    .bind(body.user_id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found in this organisation".into()))?;

    // Check not already a member.
    let exists: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM team_members WHERE team_id = $1 AND user_id = $2)",
    )
    .bind(team_id)
    .bind(body.user_id)
    .fetch_one(&state.db)
    .await?;

    if exists.unwrap_or(false) {
        return Err(AppError::Conflict(
            "User is already a member of this team".into(),
        ));
    }

    let now = Utc::now();
    let role_str = body.role.to_string();

    sqlx::query(
        "INSERT INTO team_members (team_id, user_id, role, joined_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(team_id)
    .bind(body.user_id)
    .bind(&role_str)
    .bind(now)
    .execute(&state.db)
    .await?;

    Ok(Json(TeamMemberResponse {
        user_id: user.get("id"),
        email: user.get("email"),
        display_name: user.get("display_name"),
        role: role_str,
        joined_at: now,
    }))
}

/// PUT /:id/members/:user_id — update a member's role.
async fn update_member_role(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((team_id, user_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateMemberRoleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_manager(&state.db, &claims, team_id).await?;

    // Non-org-managers cannot promote to owner.
    if body.role == TeamMemberRole::Owner {
        let org = org_role(&claims)?;
        if !is_org_manager(&org) {
            let caller_team_role = team_role_for_user(&state.db, team_id, claims.sub).await?;
            if caller_team_role != Some(TeamMemberRole::Owner) {
                return Err(AppError::Forbidden(
                    "Only team owners or org admins can promote to owner".into(),
                ));
            }
        }
    }

    let role_str = body.role.to_string();

    let result = sqlx::query(
        "UPDATE team_members SET role = $1 WHERE team_id = $2 AND user_id = $3",
    )
    .bind(&role_str)
    .bind(team_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "User is not a member of this team".into(),
        ));
    }

    Ok(Json(serde_json::json!({ "updated": true, "role": role_str })))
}

/// DELETE /:id/members/:user_id — remove a member from the team.
async fn remove_member(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((team_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_manager(&state.db, &claims, team_id).await?;

    // Prevent removing yourself if you are the sole owner.
    if user_id == claims.sub {
        let owner_count: Option<i64> = sqlx::query_scalar(
            "SELECT COUNT(*) FROM team_members WHERE team_id = $1 AND role = 'owner'",
        )
        .bind(team_id)
        .fetch_one(&state.db)
        .await?;

        let current_role = team_role_for_user(&state.db, team_id, user_id).await?;
        if current_role == Some(TeamMemberRole::Owner) && owner_count.unwrap_or(0) <= 1 {
            return Err(AppError::BadRequest(
                "Cannot remove the last owner of a team".into(),
            ));
        }
    }

    let result = sqlx::query(
        "DELETE FROM team_members WHERE team_id = $1 AND user_id = $2",
    )
    .bind(team_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "User is not a member of this team".into(),
        ));
    }

    Ok(Json(serde_json::json!({ "removed": true })))
}

/// POST /:id/invite — invite a user by email (creates a pending invitation).
async fn invite_by_email(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(team_id): Path<Uuid>,
    Json(body): Json<InviteRequest>,
) -> Result<Json<InviteResponse>, AppError> {
    let _existing = fetch_team_in_org(&state.db, team_id, claims.org_id).await?;
    require_team_manager(&state.db, &claims, team_id).await?;

    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("Email must not be empty".into()));
    }

    // Check for an existing pending invitation to the same team and email.
    let already_pending: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS(\
             SELECT 1 FROM team_invitations \
             WHERE team_id = $1 AND email = $2 AND status = 'pending'\
         )",
    )
    .bind(team_id)
    .bind(&email)
    .fetch_one(&state.db)
    .await?;

    if already_pending.unwrap_or(false) {
        return Err(AppError::Conflict(
            "A pending invitation already exists for this email".into(),
        ));
    }

    let now = Utc::now();
    let expires_at = now + chrono::Duration::days(7);
    let invite_id = Uuid::new_v4();
    let role_str = body.role.to_string();

    sqlx::query(
        "INSERT INTO team_invitations (id, team_id, email, role, status, invited_by, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7)",
    )
    .bind(invite_id)
    .bind(team_id)
    .bind(&email)
    .bind(&role_str)
    .bind(claims.sub)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    Ok(Json(InviteResponse {
        id: invite_id,
        team_id,
        email,
        role: role_str,
        status: "pending".into(),
        invited_by: claims.sub,
        created_at: now,
        expires_at,
    }))
}
