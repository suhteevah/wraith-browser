use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Full-text search query (matched against tsvector).
    pub q: String,
    /// Optional domain filter (e.g. "example.com").
    pub domain: Option<String>,
    /// Maximum number of results to return (default 20, max 100).
    pub max_results: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub url_hash: String,
    pub url: String,
    pub title: Option<String>,
    /// Headline snippet with search-term highlights (`<b>` tags).
    pub snippet: Option<String>,
    pub domain: String,
    pub fetched_at: DateTime<Utc>,
    pub rank: f32,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Serialize)]
pub struct CachedPage {
    pub url_hash: String,
    pub url: String,
    pub title: Option<String>,
    pub domain: String,
    pub body_text: Option<String>,
    pub headers: Option<serde_json::Value>,
    pub status_code: Option<i16>,
    pub content_type: Option<String>,
    pub byte_size: i64,
    pub fetched_at: DateTime<Utc>,
    pub pinned: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PinRequest {
    pub pinned: bool,
}

#[derive(Debug, Serialize)]
pub struct PinResponse {
    pub url_hash: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
pub struct TagRequest {
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TagResponse {
    pub url_hash: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CacheStats {
    pub total_pages: i64,
    pub total_size_bytes: i64,
    pub pinned_pages: i64,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
    pub unique_domains: i64,
}

#[derive(Debug, Deserialize)]
pub struct PurgeRequest {
    /// Purge entries older than this many seconds.
    pub older_than_secs: Option<i64>,
    /// Only purge entries for this domain.
    pub domain: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PurgeResponse {
    pub purged_count: i64,
    pub freed_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct SimilarPage {
    pub url_hash: String,
    pub url: String,
    pub title: Option<String>,
    pub domain: String,
    pub similarity: f32,
}

#[derive(Debug, Deserialize)]
pub struct SimilarQuery {
    pub max_results: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DomainProfile {
    pub domain: String,
    pub page_count: i64,
    pub total_size_bytes: i64,
    pub avg_change_interval_secs: Option<f64>,
    pub recommended_ttl_secs: Option<i64>,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(search))
        .route("/pages/{url_hash}", get(get_page))
        .route("/pages/{url_hash}/pin", post(pin_page))
        .route("/pages/{url_hash}/tag", post(tag_page))
        .route("/stats", get(stats))
        .route("/purge", post(purge))
        .route("/similar/{url_hash}", get(similar))
        .route("/domain/{domain}", get(domain_profile))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Full-text search across cached pages scoped to the caller's organisation.
async fn search(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, AppError> {
    let limit = params.max_results.unwrap_or(20).min(100).max(1);
    let offset = params.offset.unwrap_or(0).max(0);
    let org_id = claims.org_id;

    // Count total matches first.
    let total: i64 = if let Some(ref domain) = params.domain {
        sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM cached_pages
            WHERE org_id = $1
              AND domain = $2
              AND tsv @@ websearch_to_tsquery('english', $3)
            "#,
        )
        .bind(org_id)
        .bind(domain.as_str())
        .bind(&params.q)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM cached_pages
            WHERE org_id = $1
              AND tsv @@ websearch_to_tsquery('english', $2)
            "#,
        )
        .bind(org_id)
        .bind(&params.q)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    };

    // Fetch ranked page of results.
    let rows = if let Some(ref domain) = params.domain {
        sqlx::query(
            r#"
            SELECT
                url_hash,
                url,
                title,
                ts_headline('english', body_text, websearch_to_tsquery('english', $3),
                            'StartSel=<b>, StopSel=</b>, MaxWords=60, MinWords=20') AS snippet,
                domain,
                fetched_at,
                ts_rank_cd(tsv, websearch_to_tsquery('english', $3)) AS rank
            FROM cached_pages
            WHERE org_id = $1
              AND domain = $2
              AND tsv @@ websearch_to_tsquery('english', $3)
            ORDER BY rank DESC, fetched_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(org_id)
        .bind(domain.as_str())
        .bind(&params.q)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        sqlx::query(
            r#"
            SELECT
                url_hash,
                url,
                title,
                ts_headline('english', body_text, websearch_to_tsquery('english', $2),
                            'StartSel=<b>, StopSel=</b>, MaxWords=60, MinWords=20') AS snippet,
                domain,
                fetched_at,
                ts_rank_cd(tsv, websearch_to_tsquery('english', $2)) AS rank
            FROM cached_pages
            WHERE org_id = $1
              AND tsv @@ websearch_to_tsquery('english', $2)
            ORDER BY rank DESC, fetched_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(org_id)
        .bind(&params.q)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    };

    use sqlx::Row;
    let results = rows
        .into_iter()
        .map(|row| SearchResult {
            url_hash: row.get("url_hash"),
            url: row.get("url"),
            title: row.get("title"),
            snippet: row.get("snippet"),
            domain: row.get("domain"),
            fetched_at: row.get("fetched_at"),
            rank: row.get("rank"),
        })
        .collect();

    Ok(Json(SearchResponse {
        results,
        total,
        offset,
        limit,
    }))
}

/// Retrieve a single cached page by its URL hash.
async fn get_page(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(url_hash): Path<String>,
) -> Result<Json<CachedPage>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT
            url_hash,
            url,
            title,
            domain,
            body_text,
            headers,
            status_code,
            content_type,
            byte_size,
            fetched_at,
            pinned,
            tags
        FROM cached_pages
        WHERE org_id = $1 AND url_hash = $2
        "#,
    )
    .bind(claims.org_id)
    .bind(&url_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .ok_or_else(|| AppError::NotFound(format!("page {url_hash}")))?;

    use sqlx::Row;
    let page = CachedPage {
        url_hash: row.get("url_hash"),
        url: row.get("url"),
        title: row.get("title"),
        domain: row.get("domain"),
        body_text: row.get("body_text"),
        headers: row.get("headers"),
        status_code: row.get("status_code"),
        content_type: row.get("content_type"),
        byte_size: row.get("byte_size"),
        fetched_at: row.get("fetched_at"),
        pinned: row.get("pinned"),
        tags: row.get("tags"),
    };

    Ok(Json(page))
}

/// Pin or unpin a cached page so it is never evicted.
async fn pin_page(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(url_hash): Path<String>,
    Json(body): Json<PinRequest>,
) -> Result<Json<PinResponse>, AppError> {
    let rows_affected = sqlx::query(
        r#"
        UPDATE cached_pages
        SET pinned = $1
        WHERE org_id = $2 AND url_hash = $3
        "#,
    )
    .bind(body.pinned)
    .bind(claims.org_id)
    .bind(&url_hash)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .rows_affected();

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!("page {url_hash}")));
    }

    Ok(Json(PinResponse {
        url_hash,
        pinned: body.pinned,
    }))
}

/// Attach or replace labels on a cached page.
async fn tag_page(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(url_hash): Path<String>,
    Json(body): Json<TagRequest>,
) -> Result<Json<TagResponse>, AppError> {
    let rows_affected = sqlx::query(
        r#"
        UPDATE cached_pages
        SET tags = $1
        WHERE org_id = $2 AND url_hash = $3
        "#,
    )
    .bind(&body.tags)
    .bind(claims.org_id)
    .bind(&url_hash)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .rows_affected();

    if rows_affected == 0 {
        return Err(AppError::NotFound(format!("page {url_hash}")));
    }

    Ok(Json(TagResponse {
        url_hash,
        tags: body.tags,
    }))
}

/// Return aggregate cache statistics for the caller's organisation.
async fn stats(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<CacheStats>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT
            COUNT(*)                              AS total_pages,
            COALESCE(SUM(byte_size), 0)           AS total_size_bytes,
            COUNT(*) FILTER (WHERE pinned = true)  AS pinned_pages,
            MIN(fetched_at)                        AS oldest_entry,
            MAX(fetched_at)                        AS newest_entry,
            COUNT(DISTINCT domain)                 AS unique_domains
        FROM cached_pages
        WHERE org_id = $1
        "#,
    )
    .bind(claims.org_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    use sqlx::Row;
    Ok(Json(CacheStats {
        total_pages: row.get("total_pages"),
        total_size_bytes: row.get("total_size_bytes"),
        pinned_pages: row.get("pinned_pages"),
        oldest_entry: row.get("oldest_entry"),
        newest_entry: row.get("newest_entry"),
        unique_domains: row.get("unique_domains"),
    }))
}

/// Purge stale (non-pinned) cache entries.
async fn purge(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<PurgeRequest>,
) -> Result<Json<PurgeResponse>, AppError> {
    let older_than_secs = body.older_than_secs.unwrap_or(86400 * 30); // default 30 days

    let row = if let Some(ref domain) = body.domain {
        sqlx::query(
            r#"
            WITH deleted AS (
                DELETE FROM cached_pages
                WHERE org_id = $1
                  AND pinned = false
                  AND domain = $2
                  AND fetched_at < NOW() - make_interval(secs => $3::double precision)
                RETURNING byte_size
            )
            SELECT
                COUNT(*)                    AS purged_count,
                COALESCE(SUM(byte_size), 0) AS freed_bytes
            FROM deleted
            "#,
        )
        .bind(claims.org_id)
        .bind(domain.as_str())
        .bind(older_than_secs as f64)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    } else {
        sqlx::query(
            r#"
            WITH deleted AS (
                DELETE FROM cached_pages
                WHERE org_id = $1
                  AND pinned = false
                  AND fetched_at < NOW() - make_interval(secs => $2::double precision)
                RETURNING byte_size
            )
            SELECT
                COUNT(*)                    AS purged_count,
                COALESCE(SUM(byte_size), 0) AS freed_bytes
            FROM deleted
            "#,
        )
        .bind(claims.org_id)
        .bind(older_than_secs as f64)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    };

    use sqlx::Row;
    Ok(Json(PurgeResponse {
        purged_count: row.get("purged_count"),
        freed_bytes: row.get("freed_bytes"),
    }))
}

/// Find pages with similar content using tsvector similarity.
async fn similar(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(url_hash): Path<String>,
    Query(params): Query<SimilarQuery>,
) -> Result<Json<Vec<SimilarPage>>, AppError> {
    let limit = params.max_results.unwrap_or(10).min(50).max(1);

    let rows = sqlx::query(
        r#"
        WITH source AS (
            SELECT tsv
            FROM cached_pages
            WHERE org_id = $1 AND url_hash = $2
        )
        SELECT
            cp.url_hash,
            cp.url,
            cp.title,
            cp.domain,
            ts_rank_cd(cp.tsv, querytree) AS similarity
        FROM cached_pages cp,
             source s,
             LATERAL tsquery_from_tsvector(s.tsv) AS querytree
        WHERE cp.org_id = $1
          AND cp.url_hash <> $2
          AND cp.tsv @@ querytree
        ORDER BY similarity DESC
        LIMIT $3
        "#,
    )
    .bind(claims.org_id)
    .bind(&url_hash)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    use sqlx::Row;
    let results: Vec<SimilarPage> = rows
        .into_iter()
        .map(|row| SimilarPage {
            url_hash: row.get("url_hash"),
            url: row.get("url"),
            title: row.get("title"),
            domain: row.get("domain"),
            similarity: row.get("similarity"),
        })
        .collect();

    if results.is_empty() {
        // Check whether the source page even exists for a useful 404.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM cached_pages WHERE org_id = $1 AND url_hash = $2)",
        )
        .bind(claims.org_id)
        .bind(&url_hash)
        .fetch_one(&state.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        if !exists {
            return Err(AppError::NotFound(format!("page {url_hash}")));
        }
    }

    Ok(Json(results))
}

/// Domain profile — aggregated metrics and recommended TTL.
async fn domain_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(domain): Path<String>,
) -> Result<Json<DomainProfile>, AppError> {
    let row = sqlx::query(
        r#"
        WITH page_intervals AS (
            SELECT
                fetched_at,
                EXTRACT(EPOCH FROM (
                    fetched_at - LAG(fetched_at) OVER (ORDER BY fetched_at)
                )) AS interval_secs
            FROM cached_pages
            WHERE org_id = $1 AND domain = $2
        )
        SELECT
            $2                                                     AS domain,
            COUNT(*)                                               AS page_count,
            COALESCE(SUM(cp.byte_size), 0)                         AS total_size_bytes,
            (SELECT AVG(interval_secs) FROM page_intervals
             WHERE interval_secs IS NOT NULL)                      AS avg_change_interval_secs,
            (SELECT (AVG(interval_secs) * 0.5)::bigint
             FROM page_intervals
             WHERE interval_secs IS NOT NULL)                      AS recommended_ttl_secs,
            MIN(cp.fetched_at)                                     AS oldest_entry,
            MAX(cp.fetched_at)                                     AS newest_entry
        FROM cached_pages cp
        WHERE cp.org_id = $1 AND cp.domain = $2
        "#,
    )
    .bind(claims.org_id)
    .bind(&domain)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    use sqlx::Row;
    let page_count: i64 = row.get("page_count");

    if page_count == 0 {
        return Err(AppError::NotFound(format!("domain {domain}")));
    }

    Ok(Json(DomainProfile {
        domain: row.get("domain"),
        page_count,
        total_size_bytes: row.get("total_size_bytes"),
        avg_change_interval_secs: row.get("avg_change_interval_secs"),
        recommended_ttl_secs: row.get("recommended_ttl_secs"),
        oldest_entry: row.get("oldest_entry"),
        newest_entry: row.get("newest_entry"),
    }))
}
