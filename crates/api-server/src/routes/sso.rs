use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post, put},
    Form, Json, Router,
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// SAML DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SamlMetadataResponse {
    pub xml: String,
}

#[derive(Debug, Deserialize)]
pub struct SamlAcsRequest {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(rename = "RelayState")]
    pub relay_state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SamlAcsResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

// ---------------------------------------------------------------------------
// OIDC DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OidcAuthorizeQuery {
    pub provider: Option<String>,
    pub redirect_uri: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OidcAuthorizeResponse {
    pub redirect_url: String,
}

#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OidcCallbackResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

// ---------------------------------------------------------------------------
// SCIM 2.0 DTOs
// ---------------------------------------------------------------------------

/// Standard SCIM 2.0 list response envelope.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScimListResponse<T: Serialize> {
    pub schemas: Vec<String>,
    pub total_results: usize,
    pub items_per_page: usize,
    pub start_index: usize,
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScimName {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub formatted: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScimEmail {
    pub value: String,
    #[serde(rename = "type")]
    pub email_type: Option<String>,
    pub primary: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScimMeta {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    pub created: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScimUser {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: String,
    pub name: Option<ScimName>,
    pub emails: Option<Vec<ScimEmail>>,
    pub active: Option<bool>,
    pub meta: Option<ScimMeta>,
}

#[derive(Debug, Deserialize)]
pub struct ScimUserCreateRequest {
    pub schemas: Vec<String>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: String,
    pub name: Option<ScimName>,
    pub emails: Option<Vec<ScimEmail>>,
    pub active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ScimUserUpdateRequest {
    pub schemas: Vec<String>,
    #[serde(rename = "userName")]
    pub user_name: Option<String>,
    pub name: Option<ScimName>,
    pub emails: Option<Vec<ScimEmail>>,
    pub active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScimGroupMember {
    pub value: String,
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScimGroup {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub members: Option<Vec<ScimGroupMember>>,
    pub meta: Option<ScimMeta>,
}

#[derive(Debug, Deserialize)]
pub struct ScimGroupCreateRequest {
    pub schemas: Vec<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub members: Option<Vec<ScimGroupMember>>,
}

#[derive(Debug, Deserialize)]
pub struct ScimGroupUpdateRequest {
    pub schemas: Vec<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub members: Option<Vec<ScimGroupMember>>,
}

#[derive(Debug, Deserialize)]
pub struct ScimListQuery {
    pub filter: Option<String>,
    #[serde(rename = "startIndex")]
    pub start_index: Option<usize>,
    pub count: Option<usize>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        // SSO — SAML
        .route("/sso/saml/metadata", get(saml_metadata))
        .route("/sso/saml/acs", post(saml_acs))
        // SSO — OIDC
        .route("/sso/oidc/authorize", get(oidc_authorize))
        .route("/sso/oidc/callback", get(oidc_callback))
        // SCIM 2.0 — Users
        .route("/scim/v2/Users", get(scim_list_users))
        .route("/scim/v2/Users", post(scim_create_user))
        .route("/scim/v2/Users/{id}", get(scim_get_user))
        .route("/scim/v2/Users/{id}", put(scim_update_user))
        .route("/scim/v2/Users/{id}", delete(scim_delete_user))
        // SCIM 2.0 — Groups
        .route("/scim/v2/Groups", get(scim_list_groups))
        .route("/scim/v2/Groups", post(scim_create_group))
        .route("/scim/v2/Groups/{id}", put(scim_update_group))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SCIM_USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const SCIM_GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const SCIM_LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";

/// Build a SCIM `meta` object for a resource.
fn scim_meta(resource_type: &str, created: &str, modified: &str) -> ScimMeta {
    ScimMeta {
        resource_type: resource_type.to_string(),
        created: Some(created.to_string()),
        last_modified: Some(modified.to_string()),
        location: None,
    }
}

/// Validate the SCIM bearer token from the `Authorization` header.
///
/// The expected token is read from the `SCIM_BEARER_TOKEN` env var.  The
/// token is associated with an `org_id` stored in the `scim_tokens` table.
/// For simplicity we also support a `SCIM_ORG_ID` env var that directly maps
/// the single bearer token to an org.
///
/// Returns the `org_id` that the token is authorised for.
fn validate_scim_token(headers: &HeaderMap) -> Result<Uuid, AppError> {
    let expected = std::env::var("SCIM_BEARER_TOKEN").map_err(|_| {
        AppError::Internal("SCIM_BEARER_TOKEN env var is not configured".into())
    })?;

    let header_val = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".into()))?;

    let token = header_val
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Invalid Authorization scheme".into()))?;

    if token != expected {
        return Err(AppError::Unauthorized("Invalid SCIM bearer token".into()));
    }

    // Resolve the org_id associated with this SCIM token.
    let org_id_str = std::env::var("SCIM_ORG_ID").map_err(|_| {
        AppError::Internal("SCIM_ORG_ID env var is not configured".into())
    })?;
    let org_id = Uuid::parse_str(&org_id_str).map_err(|e| {
        AppError::Internal(format!("SCIM_ORG_ID is not a valid UUID: {e}"))
    })?;

    Ok(org_id)
}

/// Convert a database user row into a SCIM User resource.
fn row_to_scim_user(row: &sqlx::postgres::PgRow) -> ScimUser {
    let id: Uuid = row.get("id");
    let email: String = row.get("email");
    let display_name: Option<String> = row.get("display_name");
    let created_at: chrono::DateTime<Utc> = row.get("created_at");
    let updated_at: chrono::DateTime<Utc> = row.get("updated_at");
    let is_active: bool = row.get("is_active");

    // Split display_name into given/family as a best-effort heuristic.
    let (given, family) = match &display_name {
        Some(dn) => {
            let parts: Vec<&str> = dn.splitn(2, ' ').collect();
            (
                Some(parts[0].to_string()),
                parts.get(1).map(|s| s.to_string()),
            )
        }
        None => (None, None),
    };

    ScimUser {
        schemas: vec![SCIM_USER_SCHEMA.to_string()],
        id: Some(id.to_string()),
        external_id: None,
        user_name: email.clone(),
        name: Some(ScimName {
            given_name: given,
            family_name: family,
            formatted: display_name,
        }),
        emails: Some(vec![ScimEmail {
            value: email,
            email_type: Some("work".to_string()),
            primary: Some(true),
        }]),
        active: Some(is_active),
        meta: Some(scim_meta(
            "User",
            &created_at.to_rfc3339(),
            &updated_at.to_rfc3339(),
        )),
    }
}

/// Parse a simple SCIM filter expression of the form `userName eq "value"`.
/// Returns `Some(value)` when the filter targets `userName`, otherwise `None`.
fn parse_username_filter(filter: &str) -> Option<String> {
    // SCIM filters look like: userName eq "john@example.com"
    let parts: Vec<&str> = filter.splitn(3, ' ').collect();
    if parts.len() == 3
        && parts[0].eq_ignore_ascii_case("userName")
        && parts[1].eq_ignore_ascii_case("eq")
    {
        let val = parts[2].trim_matches('"');
        Some(val.to_string())
    } else {
        None
    }
}

/// Parse a simple SCIM filter for `displayName eq "value"`.
fn parse_displayname_filter(filter: &str) -> Option<String> {
    let parts: Vec<&str> = filter.splitn(3, ' ').collect();
    if parts.len() == 3
        && parts[0].eq_ignore_ascii_case("displayName")
        && parts[1].eq_ignore_ascii_case("eq")
    {
        let val = parts[2].trim_matches('"');
        Some(val.to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// SSO — SAML Handlers
// ---------------------------------------------------------------------------

/// GET /sso/saml/metadata — return SAML SP metadata XML.
async fn saml_metadata(
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let entity_id = state
        .config
        .saml_entity_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("SAML_ENTITY_ID not configured".into()))?;

    // Derive ACS URL from the entity ID base URL.
    let base_url = entity_id.trim_end_matches('/');
    let acs_url = format!("{}/api/v1/sso/saml/acs", base_url);

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"
                     entityID="{entity_id}">
  <md:SPSSODescriptor AuthnRequestsSigned="false"
                      WantAssertionsSigned="true"
                      protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</md:NameIDFormat>
    <md:AssertionConsumerService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
                                Location="{acs_url}"
                                index="0"
                                isDefault="true"/>
  </md:SPSSODescriptor>
</md:EntityDescriptor>"#,
        entity_id = xml_escape(entity_id),
        acs_url = xml_escape(&acs_url),
    );

    Ok(Response::builder()
        .header("Content-Type", "application/xml")
        .body(xml.into())
        .unwrap())
}

/// Escape XML special characters in attribute/text values.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Extract text content between XML tags using simple string matching.
/// Returns `None` if the tag is not found.
fn xml_text_between(xml: &str, open_tag: &str, close_tag: &str) -> Option<String> {
    let start = xml.find(open_tag)? + open_tag.len();
    let end = xml[start..].find(close_tag)? + start;
    Some(xml[start..end].trim().to_string())
}

/// Extract a SAML attribute value by its Name attribute.
/// Looks for `<saml:Attribute Name="..."><saml:AttributeValue>VALUE</saml:AttributeValue>`.
fn saml_attribute_value(xml: &str, attr_name: &str) -> Option<String> {
    let needle = format!("Name=\"{}\"", attr_name);
    let attr_pos = xml.find(&needle)?;
    let rest = &xml[attr_pos..];
    let val_open = "<saml:AttributeValue";
    let val_start_pos = rest.find(val_open)?;
    let after_open = &rest[val_start_pos..];
    // Skip past the opening tag (it may have attributes like xsi:type).
    let tag_end = after_open.find('>')? + 1;
    let after_tag = &after_open[tag_end..];
    let val_end = after_tag.find("</saml:AttributeValue>")?;
    Some(after_tag[..val_end].trim().to_string())
}

/// POST /sso/saml/acs — SAML Assertion Consumer Service (callback from IdP).
///
/// Receives the IdP's SAMLResponse as a form POST, base64-decodes it,
/// extracts the NameID (email) and optional firstName/lastName attributes,
/// finds or creates the user, issues JWT tokens, and redirects to the
/// dashboard.
async fn saml_acs(
    State(state): State<AppState>,
    Form(body): Form<SamlAcsRequest>,
) -> Result<Redirect, AppError> {
    // ---- 1. Base64-decode the SAMLResponse ---------------------------------
    let decoded_bytes = data_encoding::BASE64
        .decode(body.saml_response.as_bytes())
        .map_err(|e| AppError::BadRequest(format!("Invalid SAMLResponse base64: {e}")))?;
    let saml_xml = String::from_utf8(decoded_bytes)
        .map_err(|e| AppError::BadRequest(format!("SAMLResponse is not valid UTF-8: {e}")))?;

    // ---- 2. Extract NameID (email) -----------------------------------------
    // The NameID lives inside <saml:NameID ...>email@example.com</saml:NameID>
    // (or <NameID> without a namespace prefix in some IdPs).
    let email = xml_text_between(&saml_xml, "<saml:NameID", "</saml:NameID>")
        .and_then(|tag_with_attrs| {
            // The extracted text starts after "<saml:NameID" — skip to after ">".
            tag_with_attrs
                .find('>')
                .map(|i| tag_with_attrs[i + 1..].trim().to_string())
        })
        .or_else(|| {
            xml_text_between(&saml_xml, "<NameID", "</NameID>").and_then(|t| {
                t.find('>').map(|i| t[i + 1..].trim().to_string())
            })
        })
        .ok_or_else(|| AppError::BadRequest("SAMLResponse missing NameID element".into()))?;

    if email.is_empty() {
        return Err(AppError::BadRequest("SAMLResponse NameID is empty".into()));
    }

    // ---- 3. Extract optional attributes ------------------------------------
    let first_name = saml_attribute_value(&saml_xml, "firstName")
        .or_else(|| saml_attribute_value(&saml_xml, "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/givenname"))
        .or_else(|| saml_attribute_value(&saml_xml, "givenName"));
    let last_name = saml_attribute_value(&saml_xml, "lastName")
        .or_else(|| saml_attribute_value(&saml_xml, "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/surname"))
        .or_else(|| saml_attribute_value(&saml_xml, "familyName"));

    let display_name = match (&first_name, &last_name) {
        (Some(f), Some(l)) => Some(format!("{} {}", f, l)),
        (Some(f), None) => Some(f.clone()),
        (None, Some(l)) => Some(l.clone()),
        (None, None) => None,
    };

    // ---- 4. Find or create user + org (same pattern as OIDC callback) ------
    let now = Utc::now();

    let existing_row =
        sqlx::query(r#"SELECT id, org_id, role FROM users WHERE email = $1"#)
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;

    let (user_id, org_id, role_str): (Uuid, Uuid, String) = if let Some(row) = existing_row {
        (
            row.try_get("id")
                .map_err(|e| AppError::Internal(e.to_string()))?,
            row.try_get("org_id")
                .map_err(|e| AppError::Internal(e.to_string()))?,
            row.try_get("role")
                .map_err(|e| AppError::Internal(e.to_string()))?,
        )
    } else {
        let org_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let org_name = display_name
            .as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or("user"));
        let slug = slugify(org_name);
        let role_str = "owner".to_string();

        let mut tx = state.db.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO organizations (id, name, slug, plan, created_at)
            VALUES ($1, $2, $3, 'free', $4)
            "#,
        )
        .bind(org_id)
        .bind(org_name)
        .bind(&slug)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let noop_hash = format!("saml::{}", Uuid::new_v4());

        sqlx::query(
            r#"
            INSERT INTO users (id, email, password_hash, display_name, org_id, role, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(user_id)
        .bind(&email)
        .bind(&noop_hash)
        .bind(&display_name)
        .bind(org_id)
        .bind(&role_str)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        (user_id, org_id, role_str)
    };

    // ---- 5. Issue Wraith JWT tokens ----------------------------------------
    let (access_token, refresh_token, expires_in) = issue_tokens(
        user_id,
        org_id,
        &role_str,
        &state.config.jwt_secret,
        state.config.jwt_expiry_secs,
    )?;

    // ---- 6. Redirect to dashboard with tokens ------------------------------
    let dashboard_url = format!(
        "/dashboard?access_token={}&refresh_token={}&token_type=Bearer&expires_in={}",
        urlencoding(&access_token),
        urlencoding(&refresh_token),
        expires_in,
    );

    Ok(Redirect::temporary(&dashboard_url))
}

// ---------------------------------------------------------------------------
// SSO — OIDC helpers
// ---------------------------------------------------------------------------

/// OIDC Discovery document (subset of fields we need).
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    authorization_endpoint: String,
    token_endpoint: String,
    // issuer: String,
}

/// Response from the IdP token endpoint.
#[derive(Debug, Deserialize)]
struct OidcTokenResponse {
    id_token: String,
    // access_token: String,
    // token_type: String,
}

/// Minimal set of claims we extract from the OIDC ID token.
/// We only validate the signature loosely (HS256/RS256) — for production
/// the IdP's JWKS should be fetched and verified.  For now we decode the
/// payload without signature verification (the token was received directly
/// from the IdP over TLS, which is acceptable per the OIDC spec when using
/// the authorization-code flow with a confidential client).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct IdTokenClaims {
    /// Subject identifier at the IdP.
    sub: String,
    /// User email (requires `email` scope).
    email: Option<String>,
    /// Full name (requires `profile` scope).
    name: Option<String>,
}

/// Fetch the OIDC discovery document from the issuer.
async fn fetch_oidc_discovery(issuer_url: &str) -> Result<OidcDiscovery, AppError> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch OIDC discovery: {e}")))?;
    let discovery: OidcDiscovery = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse OIDC discovery: {e}")))?;
    Ok(discovery)
}

/// Decode the payload of a JWT **without** verifying the signature.
/// This is acceptable here because the token was received directly from the
/// IdP token endpoint over TLS (confidential client, authorization-code flow).
fn decode_id_token_unverified(token: &str) -> Result<IdTokenClaims, AppError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::BadRequest("Malformed ID token".into()));
    }
    // Base64url-decode the payload (second segment).
    let payload = data_encoding::BASE64URL_NOPAD
        .decode(parts[1].as_bytes())
        .map_err(|e| AppError::Internal(format!("ID token base64 decode failed: {e}")))?;
    let claims: IdTokenClaims = serde_json::from_slice(&payload)
        .map_err(|e| AppError::Internal(format!("ID token JSON parse failed: {e}")))?;
    Ok(claims)
}

/// Issue an access + refresh token pair (mirrors logic in auth.rs).
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

    // Refresh token — 7 days
    #[derive(Serialize)]
    struct RefreshClaims {
        sub: Uuid,
        org_id: Uuid,
        role: String,
        token_type: String,
        iat: i64,
        exp: i64,
    }
    let refresh_claims = RefreshClaims {
        sub: user_id,
        org_id,
        role: role.to_string(),
        token_type: "refresh".to_string(),
        iat: now,
        exp: now + 7 * 24 * 3600,
    };
    let refresh_token = encode(&Header::default(), &refresh_claims, &key)?;

    Ok((access_token, refresh_token, expiry_secs))
}

/// Read required OIDC config from `AppState`, returning a friendly error when
/// any of the four env vars is missing.
fn oidc_config(
    state: &AppState,
) -> Result<(String, String, String, String), AppError> {
    let client_id = state
        .config
        .oidc_client_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("OIDC_CLIENT_ID not configured".into()))?
        .to_string();
    let client_secret = state
        .config
        .oidc_client_secret
        .as_deref()
        .ok_or_else(|| AppError::Internal("OIDC_CLIENT_SECRET not configured".into()))?
        .to_string();
    let issuer_url = state
        .config
        .oidc_issuer_url
        .as_deref()
        .ok_or_else(|| AppError::Internal("OIDC_ISSUER_URL not configured".into()))?
        .to_string();
    let redirect_uri = state
        .config
        .oidc_redirect_uri
        .as_deref()
        .ok_or_else(|| AppError::Internal("OIDC_REDIRECT_URI not configured".into()))?
        .to_string();
    Ok((client_id, client_secret, issuer_url, redirect_uri))
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
// SSO — OIDC Handlers
// ---------------------------------------------------------------------------

/// GET /sso/oidc/authorize — redirect to IdP authorization endpoint.
///
/// Builds the full authorization URL with `client_id`, `redirect_uri`,
/// `response_type=code`, `scope=openid email profile`, and a random `state`
/// nonce (stored server-side for CSRF validation on callback).
async fn oidc_authorize(
    State(state): State<AppState>,
    Query(params): Query<OidcAuthorizeQuery>,
) -> Result<Redirect, AppError> {
    let (client_id, _client_secret, issuer_url, redirect_uri) = oidc_config(&state)?;

    // Fetch discovery document to get the authorization endpoint.
    let discovery = fetch_oidc_discovery(&issuer_url).await?;

    // Generate a random state nonce for CSRF protection.
    let state_nonce = params
        .state
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Persist the nonce so we can validate it on callback.
    // We store it in the `oidc_states` table with a short TTL.
    let expires_at = Utc::now() + chrono::Duration::minutes(10);
    sqlx::query(
        r#"
        INSERT INTO oidc_states (state, expires_at)
        VALUES ($1, $2)
        ON CONFLICT (state) DO UPDATE SET expires_at = $2
        "#,
    )
    .bind(&state_nonce)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to persist OIDC state: {e}")))?;

    // Allow caller to override the redirect_uri (e.g. for mobile deep links).
    let redirect = params
        .redirect_uri
        .unwrap_or(redirect_uri);

    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        discovery.authorization_endpoint,
        urlencoding(&client_id),
        urlencoding(&redirect),
        urlencoding("openid email profile"),
        urlencoding(&state_nonce),
    );

    Ok(Redirect::temporary(&auth_url))
}

/// GET /sso/oidc/callback — exchange authorization code for tokens.
///
/// 1. Validates that `state` matches a known nonce (CSRF protection).
/// 2. Exchanges `code` for an ID token via the IdP token endpoint.
/// 3. Decodes the ID token to extract `email`, `name`, `sub`.
/// 4. Finds or creates the user (and org) in the DB.
/// 5. Issues Wraith JWT access + refresh tokens.
/// 6. Redirects to the dashboard with tokens in query params.
async fn oidc_callback(
    State(state): State<AppState>,
    Query(params): Query<OidcCallbackQuery>,
) -> Result<Redirect, AppError> {
    let (client_id, client_secret, issuer_url, redirect_uri) = oidc_config(&state)?;

    // ---- 1. Validate state (CSRF) ----------------------------------------
    let state_param = params
        .state
        .ok_or_else(|| AppError::BadRequest("Missing state parameter".into()))?;

    let deleted = sqlx::query_scalar::<_, i64>(
        r#"
        WITH removed AS (
            DELETE FROM oidc_states
            WHERE state = $1 AND expires_at > NOW()
            RETURNING 1
        )
        SELECT COUNT(*) FROM removed
        "#,
    )
    .bind(&state_param)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("State validation query failed: {e}")))?;

    if deleted == 0 {
        return Err(AppError::BadRequest(
            "Invalid or expired state parameter — possible CSRF".into(),
        ));
    }

    // ---- 2. Exchange code for tokens at IdP token endpoint ----------------
    let discovery = fetch_oidc_discovery(&issuer_url).await?;

    let http = reqwest::Client::new();
    let token_resp = http
        .post(&discovery.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &params.code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Token exchange request failed: {e}")))?;

    if !token_resp.status().is_success() {
        let body = token_resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "IdP token endpoint returned error: {body}"
        )));
    }

    let tokens: OidcTokenResponse = token_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse token response: {e}")))?;

    // ---- 3. Decode ID token -----------------------------------------------
    let id_claims = decode_id_token_unverified(&tokens.id_token)?;

    let email = id_claims
        .email
        .ok_or_else(|| AppError::BadRequest("ID token missing email claim".into()))?;
    let display_name = id_claims.name;

    // ---- 4. Find or create user + org -------------------------------------
    let now = Utc::now();

    // Try to find an existing user by email.
    let existing_row = sqlx::query(
        r#"SELECT id, org_id, role FROM users WHERE email = $1"#,
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    let (user_id, org_id, role_str): (Uuid, Uuid, String) = if let Some(row) = existing_row {
        (
            row.try_get("id").map_err(|e| AppError::Internal(e.to_string()))?,
            row.try_get("org_id").map_err(|e| AppError::Internal(e.to_string()))?,
            row.try_get("role").map_err(|e| AppError::Internal(e.to_string()))?,
        )
    } else {
        // Create a new org and user.
        let org_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let org_name = display_name
            .as_deref()
            .unwrap_or_else(|| email.split('@').next().unwrap_or("user"));
        let slug = slugify(org_name);
        let role_str = "owner".to_string();

        let mut tx = state.db.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO organizations (id, name, slug, plan, created_at)
            VALUES ($1, $2, $3, 'free', $4)
            "#,
        )
        .bind(org_id)
        .bind(org_name)
        .bind(&slug)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // SSO users get a random unusable password hash (they authenticate
        // via the IdP, not via password).
        let noop_hash = format!("oidc::{}", Uuid::new_v4());

        sqlx::query(
            r#"
            INSERT INTO users (id, email, password_hash, display_name, org_id, role, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(user_id)
        .bind(&email)
        .bind(&noop_hash)
        .bind(&display_name)
        .bind(org_id)
        .bind(&role_str)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        (user_id, org_id, role_str)
    };

    // ---- 5. Issue Wraith JWT tokens ---------------------------------------
    let (access_token, refresh_token, expires_in) = issue_tokens(
        user_id,
        org_id,
        &role_str,
        &state.config.jwt_secret,
        state.config.jwt_expiry_secs,
    )?;

    // ---- 6. Redirect to dashboard with tokens in query params -------------
    let dashboard_url = format!(
        "/dashboard?access_token={}&refresh_token={}&token_type=Bearer&expires_in={}",
        urlencoding(&access_token),
        urlencoding(&refresh_token),
        expires_in,
    );

    Ok(Redirect::temporary(&dashboard_url))
}

/// Percent-encode a string for use in a URL query parameter.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SCIM 2.0 — User Handlers
// ---------------------------------------------------------------------------

/// GET /scim/v2/Users — list provisioned users, optionally filtered.
///
/// Supports the `filter` query parameter for `userName eq "..."` expressions,
/// and `startIndex`/`count` for pagination (SCIM uses 1-based indexing).
async fn scim_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ScimListQuery>,
) -> Result<Json<ScimListResponse<ScimUser>>, AppError> {
    let org_id = validate_scim_token(&headers)?;

    let start_index = params.start_index.unwrap_or(1);
    let count = params.count.unwrap_or(100);
    // SCIM startIndex is 1-based; convert to SQL OFFSET (0-based).
    let offset = if start_index > 0 { start_index - 1 } else { 0 };

    // Check if the filter targets userName (email).
    let username_filter = params.filter.as_deref().and_then(parse_username_filter);

    let (rows, total): (Vec<sqlx::postgres::PgRow>, i64) = if let Some(ref email) = username_filter
    {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE org_id = $1 AND email = $2",
        )
        .bind(org_id)
        .bind(email)
        .fetch_one(&state.db)
        .await?;

        let rows = sqlx::query(
            "SELECT id, email, display_name, is_active, created_at, updated_at \
             FROM users WHERE org_id = $1 AND email = $2 \
             ORDER BY created_at \
             LIMIT $3 OFFSET $4",
        )
        .bind(org_id)
        .bind(email)
        .bind(count as i64)
        .bind(offset as i64)
        .fetch_all(&state.db)
        .await?;

        (rows, total)
    } else {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_one(&state.db)
        .await?;

        let rows = sqlx::query(
            "SELECT id, email, display_name, is_active, created_at, updated_at \
             FROM users WHERE org_id = $1 \
             ORDER BY created_at \
             LIMIT $2 OFFSET $3",
        )
        .bind(org_id)
        .bind(count as i64)
        .bind(offset as i64)
        .fetch_all(&state.db)
        .await?;

        (rows, total)
    };

    let resources: Vec<ScimUser> = rows.iter().map(row_to_scim_user).collect();

    Ok(Json(ScimListResponse {
        schemas: vec![SCIM_LIST_SCHEMA.to_string()],
        total_results: total as usize,
        items_per_page: count,
        start_index,
        resources,
    }))
}

/// POST /scim/v2/Users — provision a new user from the IdP.
///
/// Creates a user record from the SCIM payload.  `userName` is used as the
/// email address.  The user is assigned to the org identified by the SCIM
/// bearer token.
async fn scim_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ScimUserCreateRequest>,
) -> Result<(StatusCode, Json<ScimUser>), AppError> {
    let org_id = validate_scim_token(&headers)?;

    let email = body.user_name.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("userName must not be empty".into()));
    }

    // Check for duplicate email within the org.
    let exists: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1 AND org_id = $2)",
    )
    .bind(&email)
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    if exists.unwrap_or(false) {
        return Err(AppError::Conflict(format!(
            "User with email {email} already exists in this organisation"
        )));
    }

    // Build display_name from SCIM name fields.
    let display_name = body.name.as_ref().map(|n| {
        match (&n.formatted, &n.given_name, &n.family_name) {
            (Some(f), _, _) => f.clone(),
            (None, Some(g), Some(f)) => format!("{g} {f}"),
            (None, Some(g), None) => g.clone(),
            (None, None, Some(f)) => f.clone(),
            (None, None, None) => email.clone(),
        }
    });

    let now = Utc::now();
    let user_id = Uuid::new_v4();
    let is_active = body.active.unwrap_or(true);

    // SCIM-provisioned users get an unusable password hash.
    let noop_hash = format!("scim::{}", Uuid::new_v4());

    sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name, org_id, role, is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 'member', $6, $7, $8)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&noop_hash)
    .bind(&display_name)
    .bind(org_id)
    .bind(is_active)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?;

    let user = ScimUser {
        schemas: vec![SCIM_USER_SCHEMA.to_string()],
        id: Some(user_id.to_string()),
        external_id: body.external_id,
        user_name: email.clone(),
        name: Some(ScimName {
            given_name: body.name.as_ref().and_then(|n| n.given_name.clone()),
            family_name: body.name.as_ref().and_then(|n| n.family_name.clone()),
            formatted: display_name,
        }),
        emails: Some(vec![ScimEmail {
            value: email,
            email_type: Some("work".to_string()),
            primary: Some(true),
        }]),
        active: Some(is_active),
        meta: Some(scim_meta("User", &now.to_rfc3339(), &now.to_rfc3339())),
    };

    Ok((StatusCode::CREATED, Json(user)))
}

/// GET /scim/v2/Users/{id} — get a single provisioned user.
async fn scim_get_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<ScimUser>, AppError> {
    let org_id = validate_scim_token(&headers)?;

    let row = sqlx::query(
        "SELECT id, email, display_name, is_active, created_at, updated_at \
         FROM users WHERE id = $1 AND org_id = $2",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("User {id} not found")))?;

    Ok(Json(row_to_scim_user(&row)))
}

/// PUT /scim/v2/Users/{id} — replace user attributes.
///
/// Updates email, display name, and active status.  When `active` is set to
/// `false` the user is soft-deactivated (the record is kept but marked
/// inactive so they can no longer authenticate).
async fn scim_update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ScimUserUpdateRequest>,
) -> Result<Json<ScimUser>, AppError> {
    let org_id = validate_scim_token(&headers)?;

    // Verify the user exists in this org.
    let _existing = sqlx::query("SELECT id FROM users WHERE id = $1 AND org_id = $2")
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("User {id} not found")))?;

    // Build updated display_name from SCIM name fields.
    let display_name: Option<String> = body.name.as_ref().map(|n| {
        match (&n.formatted, &n.given_name, &n.family_name) {
            (Some(f), _, _) => f.clone(),
            (None, Some(g), Some(f)) => format!("{g} {f}"),
            (None, Some(g), None) => g.clone(),
            (None, None, Some(f)) => f.clone(),
            (None, None, None) => String::new(),
        }
    });

    let new_email = body.user_name.as_ref().map(|e| e.trim().to_lowercase());

    let row = sqlx::query(
        "UPDATE users SET \
             email = COALESCE($1, email), \
             display_name = COALESCE($2, display_name), \
             is_active = COALESCE($3, is_active), \
             updated_at = NOW() \
         WHERE id = $4 AND org_id = $5 \
         RETURNING id, email, display_name, is_active, created_at, updated_at",
    )
    .bind(&new_email)
    .bind(&display_name)
    .bind(body.active)
    .bind(id)
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(row_to_scim_user(&row)))
}

/// DELETE /scim/v2/Users/{id} — deprovision (soft-deactivate) a user.
///
/// Sets `is_active = false` rather than hard-deleting the row.  Returns
/// `204 No Content` per the SCIM spec.
async fn scim_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let org_id = validate_scim_token(&headers)?;

    let result = sqlx::query(
        "UPDATE users SET is_active = false, updated_at = NOW() \
         WHERE id = $1 AND org_id = $2",
    )
    .bind(id)
    .bind(org_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("User {id} not found")));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// SCIM 2.0 — Group Handlers
// ---------------------------------------------------------------------------

/// GET /scim/v2/Groups — list groups (mapped to internal teams).
///
/// Supports `displayName eq "..."` filter and `startIndex`/`count` pagination.
async fn scim_list_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ScimListQuery>,
) -> Result<Json<ScimListResponse<ScimGroup>>, AppError> {
    let org_id = validate_scim_token(&headers)?;

    let start_index = params.start_index.unwrap_or(1);
    let count = params.count.unwrap_or(100);
    let offset = if start_index > 0 { start_index - 1 } else { 0 };

    let name_filter = params.filter.as_deref().and_then(parse_displayname_filter);

    let (team_rows, total): (Vec<sqlx::postgres::PgRow>, i64) = if let Some(ref name) =
        name_filter
    {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM teams WHERE org_id = $1 AND name = $2",
        )
        .bind(org_id)
        .bind(name)
        .fetch_one(&state.db)
        .await?;

        let rows = sqlx::query(
            "SELECT id, name, created_at, updated_at \
             FROM teams WHERE org_id = $1 AND name = $2 \
             ORDER BY created_at \
             LIMIT $3 OFFSET $4",
        )
        .bind(org_id)
        .bind(name)
        .bind(count as i64)
        .bind(offset as i64)
        .fetch_all(&state.db)
        .await?;

        (rows, total)
    } else {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM teams WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_one(&state.db)
        .await?;

        let rows = sqlx::query(
            "SELECT id, name, created_at, updated_at \
             FROM teams WHERE org_id = $1 \
             ORDER BY created_at \
             LIMIT $2 OFFSET $3",
        )
        .bind(org_id)
        .bind(count as i64)
        .bind(offset as i64)
        .fetch_all(&state.db)
        .await?;

        (rows, total)
    };

    // For each team, fetch its members to populate the SCIM Group response.
    let mut resources: Vec<ScimGroup> = Vec::with_capacity(team_rows.len());
    for row in &team_rows {
        let team_id: Uuid = row.get("id");
        let name: String = row.get("name");
        let created_at: chrono::DateTime<Utc> = row.get("created_at");
        let updated_at: chrono::DateTime<Utc> = row.get("updated_at");

        let member_rows = sqlx::query(
            "SELECT tm.user_id, u.email \
             FROM team_members tm \
             JOIN users u ON u.id = tm.user_id \
             WHERE tm.team_id = $1",
        )
        .bind(team_id)
        .fetch_all(&state.db)
        .await?;

        let members: Vec<ScimGroupMember> = member_rows
            .iter()
            .map(|mr| {
                let uid: Uuid = mr.get("user_id");
                let email: String = mr.get("email");
                ScimGroupMember {
                    value: uid.to_string(),
                    display: Some(email),
                }
            })
            .collect();

        resources.push(ScimGroup {
            schemas: vec![SCIM_GROUP_SCHEMA.to_string()],
            id: Some(team_id.to_string()),
            display_name: name,
            members: Some(members),
            meta: Some(scim_meta(
                "Group",
                &created_at.to_rfc3339(),
                &updated_at.to_rfc3339(),
            )),
        });
    }

    Ok(Json(ScimListResponse {
        schemas: vec![SCIM_LIST_SCHEMA.to_string()],
        total_results: total as usize,
        items_per_page: count,
        start_index,
        resources,
    }))
}

/// POST /scim/v2/Groups — create a new group (team).
///
/// Creates a team record and optionally adds members listed in the SCIM
/// `members` array (each `value` must be an existing user UUID in the org).
async fn scim_create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ScimGroupCreateRequest>,
) -> Result<(StatusCode, Json<ScimGroup>), AppError> {
    let org_id = validate_scim_token(&headers)?;

    let name = body.display_name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("displayName must not be empty".into()));
    }

    let now = Utc::now();
    let team_id = Uuid::new_v4();

    let mut tx = state.db.begin().await?;

    sqlx::query(
        "INSERT INTO teams (id, org_id, name, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(team_id)
    .bind(org_id)
    .bind(&name)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Add members if provided.
    let mut members_out: Vec<ScimGroupMember> = Vec::new();
    if let Some(ref members) = body.members {
        for m in members {
            let user_id = Uuid::parse_str(&m.value).map_err(|_| {
                AppError::BadRequest(format!("Invalid member UUID: {}", m.value))
            })?;

            // Verify the user belongs to the same org.
            let user_row = sqlx::query(
                "SELECT id, email FROM users WHERE id = $1 AND org_id = $2",
            )
            .bind(user_id)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("User {user_id} not found in this organisation"))
            })?;

            sqlx::query(
                "INSERT INTO team_members (team_id, user_id, role, joined_at) \
                 VALUES ($1, $2, 'member', $3) \
                 ON CONFLICT (team_id, user_id) DO NOTHING",
            )
            .bind(team_id)
            .bind(user_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            let email: String = user_row.get("email");
            members_out.push(ScimGroupMember {
                value: user_id.to_string(),
                display: Some(email),
            });
        }
    }

    tx.commit().await?;

    let group = ScimGroup {
        schemas: vec![SCIM_GROUP_SCHEMA.to_string()],
        id: Some(team_id.to_string()),
        display_name: name,
        members: Some(members_out),
        meta: Some(scim_meta("Group", &now.to_rfc3339(), &now.to_rfc3339())),
    };

    Ok((StatusCode::CREATED, Json(group)))
}

/// PUT /scim/v2/Groups/{id} — replace group display name and membership.
///
/// This is a full replace operation per the SCIM spec: the `members` array
/// in the request body becomes the definitive membership list.  Any existing
/// members not present in the new list are removed; new members are added.
async fn scim_update_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ScimGroupUpdateRequest>,
) -> Result<Json<ScimGroup>, AppError> {
    let org_id = validate_scim_token(&headers)?;

    // Verify team exists in this org.
    let _team_row = sqlx::query(
        "SELECT id FROM teams WHERE id = $1 AND org_id = $2",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Group {id} not found")))?;

    let mut tx = state.db.begin().await?;

    // Update display name if provided.
    if let Some(ref new_name) = body.display_name {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err(AppError::BadRequest("displayName must not be empty".into()));
        }
        sqlx::query(
            "UPDATE teams SET name = $1, updated_at = NOW() WHERE id = $2 AND org_id = $3",
        )
        .bind(trimmed)
        .bind(id)
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    }

    // Replace membership if members array is provided.
    let mut members_out: Vec<ScimGroupMember> = Vec::new();
    if let Some(ref members) = body.members {
        // Remove all current members — full replace semantics.
        sqlx::query("DELETE FROM team_members WHERE team_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let now = Utc::now();
        for m in members {
            let user_id = Uuid::parse_str(&m.value).map_err(|_| {
                AppError::BadRequest(format!("Invalid member UUID: {}", m.value))
            })?;

            let user_row = sqlx::query(
                "SELECT id, email FROM users WHERE id = $1 AND org_id = $2",
            )
            .bind(user_id)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("User {user_id} not found in this organisation"))
            })?;

            sqlx::query(
                "INSERT INTO team_members (team_id, user_id, role, joined_at) \
                 VALUES ($1, $2, 'member', $3)",
            )
            .bind(id)
            .bind(user_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            let email: String = user_row.get("email");
            members_out.push(ScimGroupMember {
                value: user_id.to_string(),
                display: Some(email),
            });
        }
    } else {
        // No members array supplied — keep existing membership; fetch for response.
        let member_rows = sqlx::query(
            "SELECT tm.user_id, u.email \
             FROM team_members tm \
             JOIN users u ON u.id = tm.user_id \
             WHERE tm.team_id = $1",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;

        members_out = member_rows
            .iter()
            .map(|mr| {
                let uid: Uuid = mr.get("user_id");
                let email: String = mr.get("email");
                ScimGroupMember {
                    value: uid.to_string(),
                    display: Some(email),
                }
            })
            .collect();
    }

    tx.commit().await?;

    // Re-fetch the team for the updated timestamps.
    let updated_row = sqlx::query(
        "SELECT id, name, created_at, updated_at FROM teams WHERE id = $1 AND org_id = $2",
    )
    .bind(id)
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;

    let final_name: String = updated_row.get("name");
    let created_at: chrono::DateTime<Utc> = updated_row.get("created_at");
    let updated_at: chrono::DateTime<Utc> = updated_row.get("updated_at");

    Ok(Json(ScimGroup {
        schemas: vec![SCIM_GROUP_SCHEMA.to_string()],
        id: Some(id.to_string()),
        display_name: final_name,
        members: Some(members_out),
        meta: Some(scim_meta(
            "Group",
            &created_at.to_rfc3339(),
            &updated_at.to_rfc3339(),
        )),
    }))
}
