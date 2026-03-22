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
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub entity_type: String,
    pub attributes: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Uuid,
    pub org_id: Uuid,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub relationship_type: String,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntityRequest {
    pub name: String,
    pub entity_type: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct CreateRelationshipRequest {
    pub from: Uuid,
    pub to: Uuid,
    pub relationship_type: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
pub struct MergeRequest {
    /// The entity to keep (primary).
    pub primary_id: Uuid,
    /// The entity to merge into the primary and then soft-delete.
    pub secondary_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct NaturalLanguageQuery {
    pub question: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    25
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Serialize)]
pub struct EntityWithRelationships {
    pub entity: Entity,
    pub relationships: Vec<RelatedEntity>,
}

#[derive(Debug, Serialize)]
pub struct RelatedEntity {
    pub relationship_id: Uuid,
    pub relationship_type: String,
    pub confidence: f64,
    pub direction: String,
    pub entity_id: Uuid,
    pub entity_name: String,
    pub entity_type: String,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Serialize)]
pub struct MermaidDiagram {
    pub mermaid: String,
    pub entity_count: usize,
    pub relationship_count: usize,
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

fn entity_from_row(row: sqlx::postgres::PgRow) -> Entity {
    use sqlx::Row;
    Entity {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        entity_type: row.get("entity_type"),
        attributes: row.get("attributes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn relationship_from_row(row: sqlx::postgres::PgRow) -> Relationship {
    use sqlx::Row;
    Relationship {
        id: row.get("id"),
        org_id: row.get("org_id"),
        from_entity_id: row.get("from_entity_id"),
        to_entity_id: row.get("to_entity_id"),
        relationship_type: row.get("relationship_type"),
        confidence: row.get("confidence"),
        created_at: row.get("created_at"),
    }
}

fn related_entity_from_row(row: sqlx::postgres::PgRow) -> RelatedEntity {
    use sqlx::Row;
    RelatedEntity {
        relationship_id: row.get("relationship_id"),
        relationship_type: row.get("relationship_type"),
        confidence: row.get("confidence"),
        direction: row.get("direction"),
        entity_id: row.get("entity_id"),
        entity_name: row.get("entity_name"),
        entity_type: row.get("entity_type"),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_entity))
        .route("/query", get(query_entities))
        .route("/search", get(search_entities))
        .route("/relate", post(create_relationship))
        .route("/merge", post(merge_entities))
        .route("/visualize", get(visualize))
        .route("/{id}", delete(delete_entity))
        .route("/{id}/related", get(find_related))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /query — natural-language entity query.
///
/// Performs keyword extraction from the question string and matches against
/// entity names, types, and JSONB attributes using full-text search combined
/// with trigram similarity for fuzzy tolerance.
async fn query_entities(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<NaturalLanguageQuery>,
) -> Result<Json<QueryResult>, AppError> {
    let keywords = extract_keywords(&params.question);
    let tsquery = keywords.join(" | ");

    // Match entities whose name or attributes contain the keywords (full-text
    // + trigram similarity fallback).
    let entity_rows = sqlx::query(
        r#"
        SELECT id, org_id, name, entity_type, attributes, created_at, updated_at
        FROM entities
        WHERE org_id = $1
          AND (
            to_tsvector('english', name || ' ' || entity_type || ' ' || attributes::text)
              @@ to_tsquery('english', $2)
            OR similarity(name, $3) > 0.2
          )
        ORDER BY
          ts_rank(
            to_tsvector('english', name || ' ' || entity_type || ' ' || attributes::text),
            to_tsquery('english', $2)
          ) DESC,
          similarity(name, $3) DESC
        LIMIT $4
        "#,
    )
    .bind(claims.org_id)
    .bind(&tsquery)
    .bind(&params.question)
    .bind(params.limit)
    .fetch_all(&state.db)
    .await?;

    let entities: Vec<Entity> = entity_rows.into_iter().map(entity_from_row).collect();
    let entity_ids: Vec<Uuid> = entities.iter().map(|e| e.id).collect();

    // Fetch relationships between any of the matched entities.
    let relationships = if entity_ids.is_empty() {
        vec![]
    } else {
        let rel_rows = sqlx::query(
            r#"
            SELECT id, org_id, from_entity_id, to_entity_id,
                   relationship_type, confidence, created_at
            FROM relationships
            WHERE org_id = $1
              AND (from_entity_id = ANY($2) OR to_entity_id = ANY($2))
            ORDER BY confidence DESC
            "#,
        )
        .bind(claims.org_id)
        .bind(&entity_ids)
        .fetch_all(&state.db)
        .await?;

        rel_rows.into_iter().map(relationship_from_row).collect()
    };

    Ok(Json(QueryResult {
        entities,
        relationships,
    }))
}

/// POST / — create a new entity.
async fn create_entity(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateEntityRequest>,
) -> Result<Json<Entity>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("Entity name cannot be empty".into()));
    }

    let row = sqlx::query(
        r#"
        INSERT INTO entities (id, org_id, name, entity_type, attributes, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
        RETURNING id, org_id, name, entity_type, attributes, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(claims.org_id)
    .bind(body.name.trim())
    .bind(&body.entity_type)
    .bind(&body.attributes)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(entity_from_row(row)))
}

/// POST /relate — create a relationship between two entities.
async fn create_relationship(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateRelationshipRequest>,
) -> Result<Json<Relationship>, AppError> {
    if !(0.0..=1.0).contains(&body.confidence) {
        return Err(AppError::BadRequest(
            "Confidence must be between 0.0 and 1.0".into(),
        ));
    }

    // Verify both entities exist and belong to the same org.
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM entities WHERE id = ANY($1) AND org_id = $2",
    )
    .bind(&[body.from, body.to][..])
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await?;

    if count != 2 {
        return Err(AppError::NotFound(
            "One or both entities not found in this organisation".into(),
        ));
    }

    let row = sqlx::query(
        r#"
        INSERT INTO relationships
            (id, org_id, from_entity_id, to_entity_id, relationship_type, confidence, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        RETURNING id, org_id, from_entity_id, to_entity_id,
                  relationship_type, confidence, created_at
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(claims.org_id)
    .bind(body.from)
    .bind(body.to)
    .bind(&body.relationship_type)
    .bind(body.confidence)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(relationship_from_row(row)))
}

/// POST /merge — merge two entities.
///
/// The secondary entity's relationships are re-pointed to the primary entity,
/// attributes are merged (primary wins on conflicts), and the secondary entity
/// is soft-deleted (removed).
async fn merge_entities(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<MergeRequest>,
) -> Result<Json<Entity>, AppError> {
    if body.primary_id == body.secondary_id {
        return Err(AppError::BadRequest(
            "Cannot merge an entity with itself".into(),
        ));
    }

    let mut tx = state.db.begin().await?;

    // Fetch both entities (locked for update).
    let primary_row = sqlx::query(
        "SELECT id, org_id, name, entity_type, attributes, created_at, updated_at FROM entities WHERE id = $1 AND org_id = $2 FOR UPDATE",
    )
    .bind(body.primary_id)
    .bind(claims.org_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Primary entity not found".into()))?;

    let primary = entity_from_row(primary_row);

    let secondary_row = sqlx::query(
        "SELECT id, org_id, name, entity_type, attributes, created_at, updated_at FROM entities WHERE id = $1 AND org_id = $2 FOR UPDATE",
    )
    .bind(body.secondary_id)
    .bind(claims.org_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Secondary entity not found".into()))?;

    let secondary = entity_from_row(secondary_row);

    // Merge attributes: secondary values fill in gaps, primary wins conflicts.
    let merged_attrs = merge_json_objects(&primary.attributes, &secondary.attributes);

    // Update primary entity with merged attributes.
    let updated_row = sqlx::query(
        r#"
        UPDATE entities
        SET attributes = $1, updated_at = NOW()
        WHERE id = $2 AND org_id = $3
        RETURNING id, org_id, name, entity_type, attributes, created_at, updated_at
        "#,
    )
    .bind(&merged_attrs)
    .bind(body.primary_id)
    .bind(claims.org_id)
    .fetch_one(&mut *tx)
    .await?;

    let updated = entity_from_row(updated_row);

    // Re-point relationships from secondary -> primary.
    sqlx::query(
        r#"
        UPDATE relationships
        SET from_entity_id = $1
        WHERE from_entity_id = $2 AND org_id = $3
        "#,
    )
    .bind(body.primary_id)
    .bind(body.secondary_id)
    .bind(claims.org_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE relationships
        SET to_entity_id = $1
        WHERE to_entity_id = $2 AND org_id = $3
        "#,
    )
    .bind(body.primary_id)
    .bind(body.secondary_id)
    .bind(claims.org_id)
    .execute(&mut *tx)
    .await?;

    // Remove self-referential relationships that may have been created.
    sqlx::query(
        "DELETE FROM relationships WHERE from_entity_id = to_entity_id AND org_id = $1",
    )
    .bind(claims.org_id)
    .execute(&mut *tx)
    .await?;

    // Delete the secondary entity.
    sqlx::query("DELETE FROM entities WHERE id = $1 AND org_id = $2")
        .bind(body.secondary_id)
        .bind(claims.org_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Json(updated))
}

/// GET /search — fuzzy name search using pg_trgm similarity.
async fn search_entities(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<Entity>>, AppError> {
    if params.q.trim().is_empty() {
        return Err(AppError::BadRequest("Search query cannot be empty".into()));
    }

    let rows = sqlx::query(
        r#"
        SELECT id, org_id, name, entity_type, attributes, created_at, updated_at
        FROM entities
        WHERE org_id = $1
          AND (
            similarity(name, $2) > 0.15
            OR name ILIKE '%' || $2 || '%'
          )
        ORDER BY similarity(name, $2) DESC
        LIMIT $3
        "#,
    )
    .bind(claims.org_id)
    .bind(params.q.trim())
    .bind(params.limit)
    .fetch_all(&state.db)
    .await?;

    let entities: Vec<Entity> = rows.into_iter().map(entity_from_row).collect();

    Ok(Json(entities))
}

/// GET /{id}/related — find all entities related to a given entity.
async fn find_related(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<EntityWithRelationships>, AppError> {
    let entity_row = sqlx::query(
        "SELECT id, org_id, name, entity_type, attributes, created_at, updated_at FROM entities WHERE id = $1 AND org_id = $2",
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Entity {id} not found")))?;

    let entity = entity_from_row(entity_row);

    let rel_rows = sqlx::query(
        r#"
        SELECT
            r.id            AS relationship_id,
            r.relationship_type,
            r.confidence,
            CASE
                WHEN r.from_entity_id = $1 THEN 'outgoing'
                ELSE 'incoming'
            END             AS direction,
            e.id            AS entity_id,
            e.name          AS entity_name,
            e.entity_type
        FROM relationships r
        JOIN entities e
          ON e.id = CASE
              WHEN r.from_entity_id = $1 THEN r.to_entity_id
              ELSE r.from_entity_id
          END
        WHERE r.org_id = $2
          AND (r.from_entity_id = $1 OR r.to_entity_id = $1)
        ORDER BY r.confidence DESC
        "#,
    )
    .bind(id)
    .bind(claims.org_id)
    .fetch_all(&state.db)
    .await?;

    let relationships: Vec<RelatedEntity> = rel_rows.into_iter().map(related_entity_from_row).collect();

    Ok(Json(EntityWithRelationships {
        entity,
        relationships,
    }))
}

/// GET /visualize — generate a Mermaid diagram of the org's knowledge graph.
async fn visualize(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<MermaidDiagram>, AppError> {
    let entity_rows = sqlx::query(
        "SELECT id, org_id, name, entity_type, attributes, created_at, updated_at FROM entities WHERE org_id = $1 ORDER BY name LIMIT 200",
    )
    .bind(claims.org_id)
    .fetch_all(&state.db)
    .await?;

    let entities: Vec<Entity> = entity_rows.into_iter().map(entity_from_row).collect();

    let rel_rows = sqlx::query(
        "SELECT id, org_id, from_entity_id, to_entity_id, relationship_type, confidence, created_at FROM relationships WHERE org_id = $1 ORDER BY confidence DESC LIMIT 500",
    )
    .bind(claims.org_id)
    .fetch_all(&state.db)
    .await?;

    let relationships: Vec<Relationship> = rel_rows.into_iter().map(relationship_from_row).collect();

    let mut mermaid = String::from("graph LR\n");

    // Emit node definitions with type labels.
    for e in &entities {
        let safe_name = e.name.replace('"', "'");
        mermaid.push_str(&format!(
            "    {}[\"{} ({})\"];\n",
            short_id(e.id),
            safe_name,
            e.entity_type
        ));
    }

    // Emit edges.
    for r in &relationships {
        let label = r.relationship_type.replace('"', "'");
        mermaid.push_str(&format!(
            "    {} -->|\"{}\"| {};\n",
            short_id(r.from_entity_id),
            label,
            short_id(r.to_entity_id)
        ));
    }

    Ok(Json(MermaidDiagram {
        mermaid,
        entity_count: entities.len(),
        relationship_count: relationships.len(),
    }))
}

/// DELETE /{id} — delete an entity and its relationships.
async fn delete_entity(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut tx = state.db.begin().await?;

    // Remove all relationships involving this entity.
    sqlx::query(
        "DELETE FROM relationships WHERE (from_entity_id = $1 OR to_entity_id = $1) AND org_id = $2",
    )
    .bind(id)
    .bind(claims.org_id)
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query("DELETE FROM entities WHERE id = $1 AND org_id = $2")
        .bind(id)
        .bind(claims.org_id)
        .execute(&mut *tx)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Entity {id} not found")));
    }

    tx.commit().await?;

    Ok(Json(serde_json::json!({
        "deleted": true,
        "id": id
    })))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Naive keyword extraction: lowercases, strips common stop words, and splits
/// on whitespace. Good enough for tsquery construction; swap for an NLP
/// pipeline when one is available.
fn extract_keywords(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "shall",
        "should", "may", "might", "must", "can", "could", "of", "in", "to",
        "for", "with", "on", "at", "by", "from", "as", "into", "about",
        "between", "through", "and", "or", "but", "not", "no", "if", "then",
        "that", "this", "it", "its", "what", "which", "who", "whom", "how",
        "where", "when", "all", "each", "every", "any", "few", "more", "most",
        "other", "some", "such", "than", "too", "very", "just", "also",
        "me", "my", "show", "find", "get", "list", "give", "tell",
    ];

    text.to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(w))
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Merge two JSON objects; keys in `primary` take precedence.
fn merge_json_objects(
    primary: &serde_json::Value,
    secondary: &serde_json::Value,
) -> serde_json::Value {
    match (primary, secondary) {
        (serde_json::Value::Object(p), serde_json::Value::Object(s)) => {
            let mut merged = s.clone();
            for (k, v) in p {
                merged.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(merged)
        }
        _ => primary.clone(),
    }
}

/// Produce a short Mermaid-safe node id from a UUID (first 8 hex chars).
fn short_id(id: Uuid) -> String {
    format!("n{}", &id.to_string()[..8])
}
