use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{info, warn, debug, instrument};
use chrono::{Utc, DateTime, NaiveDateTime};

use crate::error::{CacheError, CacheResult};
use crate::schema::*;
use crate::staleness::StalenessPolicy;
use crate::query::{CacheResult as QueryResult, KnowledgeSource};
use crate::compression;
use crate::fulltext::FulltextIndex;

/// The central knowledge store. Every agent operation flows through here.
pub struct KnowledgeStore {
    /// SQLite connection (single writer, multiple readers via WAL mode)
    db: Arc<Mutex<rusqlite::Connection>>,

    /// Tantivy full-text index for content search
    fulltext: FulltextIndex,

    /// Base directory for blob storage (compressed HTML, screenshots)
    blob_dir: PathBuf,

    /// Staleness policy configuration
    staleness: StalenessPolicy,
}

/// Parse a SQLite datetime string into a chrono DateTime<Utc>.
fn parse_datetime(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc())
        .unwrap_or_else(|_| Utc::now())
}


impl KnowledgeStore {
    /// Open or create a knowledge store at the given directory.
    #[instrument(fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> CacheResult<Self> {
        let base = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&base)
            .map_err(|e| CacheError::IoError(format!("Failed to create store dir: {e}")))?;

        let db_path = base.join("knowledge.db");
        let blob_dir = base.join("blobs");
        let index_dir = base.join("index");

        std::fs::create_dir_all(&blob_dir)
            .map_err(|e| CacheError::IoError(format!("Failed to create blob dir: {e}")))?;

        info!(
            db_path = %db_path.display(),
            blob_dir = %blob_dir.display(),
            index_dir = %index_dir.display(),
            "Opening KnowledgeStore"
        );

        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| CacheError::DatabaseError(e.to_string()))?;

        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA cache_size=-64000;
            PRAGMA foreign_keys=ON;
            PRAGMA busy_timeout=5000;
        ").map_err(|e| CacheError::DatabaseError(e.to_string()))?;

        let db = Arc::new(Mutex::new(conn));
        Self::init_schema(&db)?;
        let fulltext = FulltextIndex::open(&index_dir)?;

        let store = Self {
            db,
            fulltext,
            blob_dir,
            staleness: StalenessPolicy::default(),
        };

        let stats = store.stats()?;
        info!(
            pages = stats.total_pages,
            searches = stats.total_searches,
            snapshots = stats.total_snapshots,
            domains = stats.total_domains,
            disk_bytes = stats.total_disk_bytes,
            "KnowledgeStore opened"
        );

        Ok(store)
    }

    /// Initialize the SQLite schema.
    fn init_schema(db: &Arc<Mutex<rusqlite::Connection>>) -> CacheResult<()> {
        let conn = db.lock();
        conn.execute_batch(include_str!("sql/schema.sql"))
            .map_err(|e| CacheError::DatabaseError(format!("Schema init failed: {e}")))?;
        debug!("Database schema initialized");
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // PAGE CACHE — the primary knowledge store
    // ═══════════════════════════════════════════════════════════════

    /// Look up a cached page by URL. Returns None on cache miss.
    #[instrument(skip(self), fields(url = %url))]
    pub fn get_page(&self, url: &str) -> CacheResult<Option<CachedPage>> {
        let url_hash = Self::hash_url(url);
        debug!(url_hash = %url_hash, "Looking up cached page");

        let conn = self.db.lock();

        // Increment hit count
        conn.execute(
            "UPDATE pages SET hit_count = hit_count + 1 WHERE url_hash = ?1",
            rusqlite::params![url_hash],
        )?;

        let mut stmt = conn.prepare(
            "SELECT url_hash, url, domain, title, markdown, plain_text, snippet,
                    token_count, links_json, content_type, content_hash,
                    first_seen, last_fetched, last_validated, hit_count,
                    change_count, http_status, etag, last_modified,
                    pinned, agent_notes, tags_json, raw_html_size,
                    extraction_confidence
             FROM pages WHERE url_hash = ?1"
        )?;

        let result = stmt.query_row(rusqlite::params![url_hash], |row| {
            let links_json: String = row.get(8)?;
            let tags_json: String = row.get(21)?;
            let content_type_str: String = row.get(9)?;
            let first_seen_str: String = row.get(11)?;
            let last_fetched_str: String = row.get(12)?;
            let last_validated_str: String = row.get(13)?;

            Ok(CachedPage {
                url_hash: row.get(0)?,
                url: row.get(1)?,
                domain: row.get(2)?,
                title: row.get(3)?,
                markdown: row.get(4)?,
                plain_text: row.get(5)?,
                snippet: row.get(6)?,
                token_count: row.get::<_, i64>(7)? as usize,
                links: serde_json::from_str(&links_json).unwrap_or_default(),
                content_type: parse_content_type(&content_type_str),
                content_hash: row.get(10)?,
                first_seen: parse_datetime(&first_seen_str),
                last_fetched: parse_datetime(&last_fetched_str),
                last_validated: parse_datetime(&last_validated_str),
                hit_count: row.get::<_, i64>(14)? as u64,
                change_count: row.get::<_, i64>(15)? as u64,
                http_status: row.get::<_, i32>(16)? as u16,
                etag: row.get(17)?,
                last_modified: row.get(18)?,
                pinned: row.get::<_, bool>(19)?,
                agent_notes: row.get(20)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                raw_html_size: row.get::<_, i64>(22)? as usize,
                extraction_confidence: row.get(23)?,
            })
        });

        match result {
            Ok(page) => {
                debug!(url = %url, hit_count = page.hit_count, "Cache hit");
                Ok(Some(page))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                debug!(url = %url, "Cache miss");
                Ok(None)
            }
            Err(e) => Err(CacheError::DatabaseError(e.to_string())),
        }
    }

    /// Store a page in the cache. Compresses and stores raw HTML as a blob,
    /// indexes markdown content in Tantivy, updates domain profile.
    #[instrument(skip(self, page, raw_html), fields(url = %page.url, content_type = ?page.content_type))]
    pub fn put_page(&self, page: &CachedPage, raw_html: &str) -> CacheResult<()> {
        info!(
            url = %page.url,
            domain = %page.domain,
            content_type = ?page.content_type,
            token_count = page.token_count,
            links = page.links.len(),
            "Caching page"
        );

        // 1. Compress and store raw HTML blob
        let compressed = compression::compress(raw_html.as_bytes())?;
        let blob_path = self.blob_dir.join(&page.url_hash);
        std::fs::write(&blob_path, &compressed)
            .map_err(|e| CacheError::IoError(format!("Blob write failed: {e}")))?;
        debug!(
            original_size = raw_html.len(),
            compressed_size = compressed.len(),
            ratio = format!("{:.1}%", compressed.len() as f64 / raw_html.len().max(1) as f64 * 100.0),
            "Raw HTML compressed and stored"
        );

        // 2. Check for content change (for adaptive TTL)
        {
            let conn = self.db.lock();
            let old_hash: Option<String> = conn.query_row(
                "SELECT content_hash FROM pages WHERE url_hash = ?1",
                rusqlite::params![page.url_hash],
                |row| row.get(0),
            ).ok();

            if let Some(ref old) = old_hash {
                if old != &page.content_hash {
                    conn.execute(
                        "INSERT INTO change_log (url_hash, old_content_hash, new_content_hash)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![page.url_hash, old, page.content_hash],
                    )?;
                    debug!(url = %page.url, "Content changed, logged for TTL computation");
                }
            }

            // 3. Insert/update SQLite row
            let links_json = serde_json::to_string(&page.links)
                .map_err(|e| CacheError::SerializationError(e.to_string()))?;
            let tags_json = serde_json::to_string(&page.tags)
                .map_err(|e| CacheError::SerializationError(e.to_string()))?;

            conn.execute(
                "INSERT OR REPLACE INTO pages (
                    url_hash, url, domain, title, markdown, plain_text, snippet,
                    token_count, links_json, content_type, content_hash,
                    first_seen, last_fetched, last_validated, hit_count,
                    change_count, http_status, etag, last_modified,
                    pinned, agent_notes, tags_json, raw_html_size,
                    extraction_confidence
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10, ?11,
                    COALESCE((SELECT first_seen FROM pages WHERE url_hash = ?1), datetime('now')),
                    datetime('now'), datetime('now'),
                    COALESCE((SELECT hit_count FROM pages WHERE url_hash = ?1), 0),
                    COALESCE((SELECT change_count FROM pages WHERE url_hash = ?1), 0)
                        + CASE WHEN ?11 != COALESCE((SELECT content_hash FROM pages WHERE url_hash = ?1), '') THEN 1 ELSE 0 END,
                    ?12, ?13, ?14,
                    COALESCE((SELECT pinned FROM pages WHERE url_hash = ?1), 0),
                    COALESCE((SELECT agent_notes FROM pages WHERE url_hash = ?1), NULL),
                    ?15, ?16, ?17
                )",
                rusqlite::params![
                    page.url_hash,          // 1
                    page.url,               // 2
                    page.domain,            // 3
                    page.title,             // 4
                    page.markdown,          // 5
                    page.plain_text,        // 6
                    page.snippet,           // 7
                    page.token_count as i64, // 8
                    links_json,             // 9
                    format!("{:?}", page.content_type), // 10
                    page.content_hash,      // 11
                    page.http_status as i32, // 12
                    page.etag,              // 13
                    page.last_modified,     // 14
                    tags_json,              // 15
                    compressed.len() as i64, // 16
                    page.extraction_confidence, // 17
                ],
            )?;

            debug!("Page metadata stored in SQLite");
        }

        // 4. Index in Tantivy for full-text search
        self.fulltext.index_page(
            &page.url_hash,
            &page.url,
            &page.title,
            &page.plain_text,
            &page.snippet,
            &page.tags,
        )?;
        debug!("Page indexed in Tantivy");

        // 5. Update domain profile
        self.update_domain_profile(&page.domain, page)?;

        info!(
            url = %page.url,
            total_stored_bytes = compressed.len(),
            "Page cached successfully"
        );

        Ok(())
    }

    /// Check if a cached page is stale (needs re-fetch).
    #[instrument(skip(self, page), fields(url = %page.url))]
    pub fn is_stale(&self, page: &CachedPage) -> bool {
        if page.pinned {
            debug!(url = %page.url, "Page is pinned — never stale");
            return false;
        }

        let ttl_secs = self.get_effective_ttl(&page.domain, page.content_type);
        let age_secs = (Utc::now() - page.last_fetched).num_seconds() as u64;
        let stale = age_secs > ttl_secs;

        debug!(
            url = %page.url,
            age_secs,
            ttl_secs,
            content_type = ?page.content_type,
            stale,
            "Staleness check"
        );

        stale
    }

    /// Get the raw HTML for a cached page (decompresses from blob store).
    #[instrument(skip(self), fields(url_hash = %url_hash))]
    pub fn get_raw_html(&self, url_hash: &str) -> CacheResult<Option<String>> {
        let blob_path = self.blob_dir.join(url_hash);
        if !blob_path.exists() {
            return Ok(None);
        }

        let compressed = std::fs::read(&blob_path)
            .map_err(|e| CacheError::IoError(format!("Blob read failed: {e}")))?;
        let decompressed = compression::decompress(&compressed)?;
        let html = String::from_utf8(decompressed)
            .map_err(|e| CacheError::IoError(format!("UTF-8 decode failed: {e}")))?;

        debug!(url_hash = %url_hash, html_len = html.len(), "Raw HTML retrieved from blob store");
        Ok(Some(html))
    }

    // ═══════════════════════════════════════════════════════════════
    // SEARCH CACHE — metasearch result caching
    // ═══════════════════════════════════════════════════════════════

    /// Look up cached search results for a query.
    #[instrument(skip(self), fields(query = %query))]
    pub fn get_search(&self, query: &str) -> CacheResult<Option<CachedSearch>> {
        let normalized = Self::normalize_query(query);
        let query_hash = Self::hash_query(&normalized);
        debug!(query = %query, normalized = %normalized, query_hash = %query_hash, "Looking up cached search");

        let conn = self.db.lock();

        // Increment hit count
        conn.execute(
            "UPDATE searches SET hit_count = hit_count + 1 WHERE query_hash = ?1",
            rusqlite::params![query_hash],
        )?;

        let result = conn.query_row(
            "SELECT query_hash, query, query_normalized, results_json, providers_json,
                    searched_at, hit_count, search_duration_ms
             FROM searches WHERE query_hash = ?1",
            rusqlite::params![query_hash],
            |row| {
                let results_json: String = row.get(3)?;
                let providers_json: String = row.get(4)?;
                let searched_at_str: String = row.get(5)?;

                Ok(CachedSearch {
                    query_hash: row.get(0)?,
                    query: row.get(1)?,
                    query_normalized: row.get(2)?,
                    results: serde_json::from_str(&results_json).unwrap_or_default(),
                    providers_used: serde_json::from_str(&providers_json).unwrap_or_default(),
                    searched_at: parse_datetime(&searched_at_str),
                    hit_count: row.get::<_, i64>(6)? as u64,
                    search_duration_ms: row.get::<_, i64>(7)? as u64,
                })
            },
        );

        match result {
            Ok(search) => {
                // Check staleness
                let age = (Utc::now() - search.searched_at).num_seconds() as u64;
                if age > self.staleness.search_max_age_secs {
                    debug!(query = %query, age_secs = age, "Cached search is stale");
                    return Ok(None);
                }
                debug!(query = %query, results = search.results.len(), "Search cache hit");
                Ok(Some(search))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                debug!(query = %query, "Search cache miss");
                Ok(None)
            }
            Err(e) => Err(CacheError::DatabaseError(e.to_string())),
        }
    }

    /// Store search results in the cache.
    #[instrument(skip(self, results), fields(query = %results.query, result_count = results.results.len()))]
    pub fn put_search(&self, results: &CachedSearch) -> CacheResult<()> {
        info!(
            query = %results.query,
            results = results.results.len(),
            providers = ?results.providers_used,
            duration_ms = results.search_duration_ms,
            "Caching search results"
        );

        let results_json = serde_json::to_string(&results.results)?;
        let providers_json = serde_json::to_string(&results.providers_used)?;

        {
            let conn = self.db.lock();
            conn.execute(
                "INSERT OR REPLACE INTO searches (
                    query_hash, query, query_normalized, results_json, providers_json,
                    searched_at, hit_count, search_duration_ms, result_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), 0, ?6, ?7)",
                rusqlite::params![
                    results.query_hash,
                    results.query,
                    results.query_normalized,
                    results_json,
                    providers_json,
                    results.search_duration_ms as i64,
                    results.results.len() as i64,
                ],
            )?;
        }

        // Index search result snippets in Tantivy
        for result in &results.results {
            self.fulltext.index_search_result(
                &results.query_hash,
                &results.query,
                &result.title,
                &result.snippet,
                &result.url,
            )?;
        }

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // SNAPSHOT CACHE — agent session memory
    // ═══════════════════════════════════════════════════════════════

    /// Store a DOM snapshot for an agent session.
    #[instrument(skip(self, snapshot), fields(session = %snapshot.session_id, step = snapshot.step))]
    pub fn put_snapshot(&self, snapshot: &CachedSnapshot) -> CacheResult<()> {
        debug!(
            session = %snapshot.session_id,
            step = snapshot.step,
            url = %snapshot.url,
            elements = snapshot.element_count,
            "Caching DOM snapshot"
        );

        let conn = self.db.lock();
        conn.execute(
            "INSERT OR REPLACE INTO snapshots (
                snapshot_id, session_id, url, step, agent_text,
                element_count, page_type, taken_at, token_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), ?8)",
            rusqlite::params![
                snapshot.snapshot_id,
                snapshot.session_id,
                snapshot.url,
                snapshot.step as i64,
                snapshot.agent_text,
                snapshot.element_count as i64,
                snapshot.page_type,
                snapshot.token_count as i64,
            ],
        )?;

        Ok(())
    }

    /// Retrieve snapshots for a session, optionally filtered by step range.
    #[instrument(skip(self), fields(session = %session_id))]
    pub fn get_snapshots(
        &self,
        session_id: &str,
        from_step: Option<usize>,
        to_step: Option<usize>,
    ) -> CacheResult<Vec<CachedSnapshot>> {
        debug!(
            session = %session_id,
            from = ?from_step,
            to = ?to_step,
            "Retrieving session snapshots"
        );

        let conn = self.db.lock();

        let from = from_step.unwrap_or(0) as i64;
        let to = to_step.unwrap_or(i64::MAX as usize) as i64;

        let mut stmt = conn.prepare(
            "SELECT snapshot_id, session_id, url, step, agent_text,
                    element_count, page_type, taken_at, token_count
             FROM snapshots
             WHERE session_id = ?1 AND step >= ?2 AND step <= ?3
             ORDER BY step ASC"
        )?;

        let rows = stmt.query_map(rusqlite::params![session_id, from, to], |row| {
            let taken_at_str: String = row.get(7)?;
            Ok(CachedSnapshot {
                snapshot_id: row.get(0)?,
                session_id: row.get(1)?,
                url: row.get(2)?,
                step: row.get::<_, i64>(3)? as usize,
                agent_text: row.get(4)?,
                element_count: row.get::<_, i64>(5)? as usize,
                page_type: row.get(6)?,
                taken_at: parse_datetime(&taken_at_str),
                token_count: row.get::<_, i64>(8)? as usize,
            })
        })?;

        let snapshots: Vec<CachedSnapshot> = rows
            .filter_map(|r| r.ok())
            .collect();

        debug!(session = %session_id, count = snapshots.len(), "Snapshots retrieved");
        Ok(snapshots)
    }

    // ═══════════════════════════════════════════════════════════════
    // AI SEARCH — the killer feature
    // ═══════════════════════════════════════════════════════════════

    /// Search the entire knowledge store.
    #[instrument(skip(self), fields(query = %query, max_results))]
    pub fn search_knowledge(
        &self,
        query: &str,
        max_results: usize,
    ) -> CacheResult<Vec<QueryResult>> {
        info!(query = %query, max_results, "Searching knowledge store");

        // 1. Full-text search via Tantivy
        let tantivy_results = self.fulltext.search(query, max_results * 2)?;
        debug!(tantivy_hits = tantivy_results.len(), "Tantivy search complete");

        // 2. Enrich with SQLite metadata
        let mut results: Vec<QueryResult> = Vec::new();
        {
            let conn = self.db.lock();
            for hit in tantivy_results {
                let url_hash = Self::hash_url(&hit.url);

                // Try to get page metadata
                let page_meta = conn.query_row(
                    "SELECT content_type, token_count, last_fetched, hit_count
                     FROM pages WHERE url_hash = ?1",
                    rusqlite::params![url_hash],
                    |row| {
                        let ct_str: String = row.get(0)?;
                        let token_count: i64 = row.get(1)?;
                        let last_fetched_str: String = row.get(2)?;
                        let hit_count: i64 = row.get(3)?;
                        Ok((ct_str, token_count, last_fetched_str, hit_count))
                    },
                );

                let (content_type, token_count, age_secs, hit_count, source) = match page_meta {
                    Ok((ct_str, tc, lf_str, hc)) => {
                        let ct = parse_content_type(&ct_str);
                        let lf = parse_datetime(&lf_str);
                        let age = (Utc::now() - lf).num_seconds().max(0) as u64;
                        (ct, tc as usize, age, hc as u64, KnowledgeSource::CachedPage)
                    }
                    Err(_) => (ContentType::Generic, 0, 0, 0, KnowledgeSource::CachedSearch),
                };

                let is_stale = age_secs > content_type.default_ttl_secs();

                results.push(QueryResult {
                    url: hit.url,
                    title: hit.title,
                    snippet: hit.snippet,
                    relevance_score: hit.score,
                    is_stale,
                    age_secs,
                    hit_count,
                    content_type,
                    token_count,
                    source,
                });
            }
        }

        // 3. Sort by relevance, dedup, limit
        results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.dedup_by(|a, b| a.url == b.url);
        results.truncate(max_results);

        info!(
            query = %query,
            results = results.len(),
            stale_count = results.iter().filter(|r| r.is_stale).count(),
            "Knowledge search complete"
        );

        Ok(results)
    }

    /// Semantic similarity search — finds pages similar to a given URL.
    #[instrument(skip(self), fields(url = %url))]
    pub fn find_similar(&self, url: &str, max_results: usize) -> CacheResult<Vec<QueryResult>> {
        debug!(url = %url, "Finding similar cached pages");

        // Get the page's content and use it as a search query
        if let Some(page) = self.get_page(url)? {
            // Use the first ~200 words of content as a search query
            let query_text: String = page.plain_text
                .split_whitespace()
                .take(200)
                .collect::<Vec<_>>()
                .join(" ");

            if !query_text.is_empty() {
                return self.search_knowledge(&query_text, max_results);
            }
        }

        Ok(vec![])
    }

    // ═══════════════════════════════════════════════════════════════
    // DOMAIN PROFILES — adaptive staleness
    // ═══════════════════════════════════════════════════════════════

    /// Get or create a domain profile.
    pub fn get_domain_profile(&self, domain: &str) -> CacheResult<Option<DomainProfile>> {
        let conn = self.db.lock();
        let result = conn.query_row(
            "SELECT domain, pages_cached, avg_change_interval_secs, computed_ttl_secs,
                    override_ttl_secs, requires_auth, bot_hostile, avg_extraction_confidence,
                    default_content_type, total_bytes, total_hits, last_accessed, first_seen,
                    supports_conditional, crawl_delay_secs
             FROM domain_profiles WHERE domain = ?1",
            rusqlite::params![domain],
            |row| {
                let last_accessed_str: String = row.get(11)?;
                let first_seen_str: String = row.get(12)?;
                let ct_str: String = row.get(8)?;

                Ok(DomainProfile {
                    domain: row.get(0)?,
                    pages_cached: row.get::<_, i64>(1)? as u64,
                    avg_change_interval_secs: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    computed_ttl_secs: row.get::<_, i64>(3)? as u64,
                    override_ttl_secs: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                    requires_auth: row.get(5)?,
                    bot_hostile: row.get(6)?,
                    avg_extraction_confidence: row.get(7)?,
                    default_content_type: parse_content_type(&ct_str),
                    total_bytes: row.get::<_, i64>(9)? as u64,
                    total_hits: row.get::<_, i64>(10)? as u64,
                    last_accessed: parse_datetime(&last_accessed_str),
                    first_seen: parse_datetime(&first_seen_str),
                    supports_conditional: row.get(13)?,
                    crawl_delay_secs: row.get::<_, Option<i64>>(14)?.map(|v| v as u64),
                })
            },
        );

        match result {
            Ok(profile) => Ok(Some(profile)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CacheError::DatabaseError(e.to_string())),
        }
    }

    /// Update domain profile with new page observation.
    fn update_domain_profile(&self, domain: &str, page: &CachedPage) -> CacheResult<()> {
        debug!(domain = %domain, "Updating domain profile");

        let conn = self.db.lock();

        // Compute average change interval from change_log
        let avg_interval: Option<i64> = conn.query_row(
            "SELECT AVG(
                CAST((julianday(cl2.changed_at) - julianday(cl1.changed_at)) * 86400 AS INTEGER)
             )
             FROM change_log cl1
             JOIN change_log cl2 ON cl1.url_hash = cl2.url_hash AND cl2.id = cl1.id + 1
             JOIN pages p ON p.url_hash = cl1.url_hash
             WHERE p.domain = ?1",
            rusqlite::params![domain],
            |row| row.get(0),
        ).unwrap_or(None);

        let computed_ttl = if let Some(avg) = avg_interval {
            let ttl = (avg as f64 * self.staleness.change_factor) as u64;
            ttl.clamp(self.staleness.min_ttl_secs, self.staleness.max_ttl_secs)
        } else {
            page.content_type.default_ttl_secs()
        };

        conn.execute(
            "INSERT INTO domain_profiles (
                domain, pages_cached, avg_change_interval_secs, computed_ttl_secs,
                avg_extraction_confidence, default_content_type, total_bytes,
                last_accessed, first_seen
            ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
            ON CONFLICT(domain) DO UPDATE SET
                pages_cached = pages_cached + 1,
                avg_change_interval_secs = ?2,
                computed_ttl_secs = ?3,
                avg_extraction_confidence = (avg_extraction_confidence + ?4) / 2.0,
                total_bytes = total_bytes + ?6,
                last_accessed = datetime('now')",
            rusqlite::params![
                domain,
                avg_interval,
                computed_ttl as i64,
                page.extraction_confidence,
                format!("{:?}", page.content_type),
                page.raw_html_size as i64,
            ],
        )?;

        Ok(())
    }

    /// Get effective TTL for a domain + content type.
    fn get_effective_ttl(&self, domain: &str, content_type: ContentType) -> u64 {
        if let Ok(Some(profile)) = self.get_domain_profile(domain) {
            if let Some(override_ttl) = profile.override_ttl_secs {
                return override_ttl;
            }
            if profile.computed_ttl_secs > 0 {
                return profile.computed_ttl_secs;
            }
        }
        content_type.default_ttl_secs()
    }

    // ═══════════════════════════════════════════════════════════════
    // AGENT OPERATIONS — high-level convenience methods
    // ═══════════════════════════════════════════════════════════════

    /// Pin a URL — marks it as never-expire in the cache.
    #[instrument(skip(self), fields(url = %url))]
    pub fn pin_page(&self, url: &str, notes: Option<&str>) -> CacheResult<()> {
        info!(url = %url, notes = ?notes, "Pinning page (never expires)");
        let url_hash = Self::hash_url(url);
        let conn = self.db.lock();
        conn.execute(
            "UPDATE pages SET pinned = 1, agent_notes = COALESCE(?1, agent_notes) WHERE url_hash = ?2",
            rusqlite::params![notes, url_hash],
        )?;
        Ok(())
    }

    /// Tag a cached page for organization.
    #[instrument(skip(self), fields(url = %url))]
    pub fn tag_page(&self, url: &str, tags: &[&str]) -> CacheResult<()> {
        debug!(url = %url, tags = ?tags, "Tagging page");
        let url_hash = Self::hash_url(url);
        let tags_json = serde_json::to_string(&tags)?;
        let conn = self.db.lock();
        conn.execute(
            "UPDATE pages SET tags_json = ?1 WHERE url_hash = ?2",
            rusqlite::params![tags_json, url_hash],
        )?;
        Ok(())
    }

    /// Purge stale entries from the cache.
    #[instrument(skip(self))]
    pub fn purge_stale(&self) -> CacheResult<u64> {
        info!("Purging stale cache entries");

        // Get all non-pinned pages and check staleness
        let to_delete: Vec<String> = {
            let conn = self.db.lock();
            let mut stmt = conn.prepare(
                "SELECT url_hash, domain, content_type, last_fetched FROM pages WHERE pinned = 0"
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;

            rows.filter_map(|r| r.ok())
                .filter(|(_, domain, ct_str, lf_str)| {
                    let ct = parse_content_type(ct_str);
                    let ttl = self.get_effective_ttl(domain, ct);
                    let lf = parse_datetime(lf_str);
                    let age = (Utc::now() - lf).num_seconds().max(0) as u64;
                    age > ttl
                })
                .map(|(hash, _, _, _)| hash)
                .collect()
        };

        let count = to_delete.len() as u64;

        for url_hash in &to_delete {
            // Remove blob
            let blob_path = self.blob_dir.join(url_hash);
            let _ = std::fs::remove_file(&blob_path);

            // Remove from Tantivy
            let _ = self.fulltext.delete(url_hash);
        }

        // Remove from SQLite
        if !to_delete.is_empty() {
            let conn = self.db.lock();
            for url_hash in &to_delete {
                conn.execute("DELETE FROM pages WHERE url_hash = ?1", rusqlite::params![url_hash])?;
            }
        }

        info!(purged = count, "Stale entries purged");
        Ok(count)
    }

    /// Evict entries to stay under a storage budget.
    #[instrument(skip(self), fields(max_bytes))]
    pub fn evict_to_budget(&self, max_bytes: u64) -> CacheResult<u64> {
        info!(max_bytes, "Evicting to storage budget");

        let current_bytes: u64 = {
            let conn = self.db.lock();
            conn.query_row(
                "SELECT COALESCE(SUM(raw_html_size), 0) FROM pages",
                [],
                |row| row.get::<_, i64>(0),
            )? as u64
        };

        if current_bytes <= max_bytes {
            return Ok(0);
        }

        let to_free = current_bytes - max_bytes;
        let mut freed: u64 = 0;

        // LRU eviction: oldest, least-hit, non-pinned pages first
        let candidates: Vec<(String, i64)> = {
            let conn = self.db.lock();
            let mut stmt = conn.prepare(
                "SELECT url_hash, raw_html_size FROM pages
                 WHERE pinned = 0
                 ORDER BY hit_count ASC, last_fetched ASC"
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let collected: Vec<(String, i64)> = rows.filter_map(|r| r.ok()).collect();
            collected
        };

        for (url_hash, size) in candidates {
            if freed >= to_free {
                break;
            }

            let blob_path = self.blob_dir.join(&url_hash);
            let _ = std::fs::remove_file(&blob_path);
            let _ = self.fulltext.delete(&url_hash);

            let conn = self.db.lock();
            conn.execute("DELETE FROM pages WHERE url_hash = ?1", rusqlite::params![url_hash])?;

            freed += size as u64;
        }

        info!(freed_bytes = freed, "Eviction complete");
        Ok(freed)
    }

    // ═══════════════════════════════════════════════════════════════
    // UTILITIES
    // ═══════════════════════════════════════════════════════════════

    /// Hash a URL to produce a cache key.
    pub fn hash_url(url: &str) -> String {
        let normalized = Self::normalize_url(url);
        blake3::hash(normalized.as_bytes()).to_hex().to_string()
    }

    /// Hash a search query.
    pub fn hash_query(normalized_query: &str) -> String {
        blake3::hash(normalized_query.as_bytes()).to_hex().to_string()
    }

    /// Normalize a URL: remove tracking params, fragments, trailing slashes.
    pub fn normalize_url(url: &str) -> String {
        let without_fragment = url.split('#').next().unwrap_or(url);
        let trimmed = without_fragment.trim_end_matches('/');

        // Parse and remove tracking params if valid URL
        if let Ok(mut parsed) = url::Url::parse(trimmed) {
            let clean_pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .filter(|(k, _)| {
                    let k = k.to_lowercase();
                    !k.starts_with("utm_")
                        && k != "fbclid"
                        && k != "gclid"
                        && k != "ref"
                        && k != "source"
                        && k != "campaign"
                })
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            if clean_pairs.is_empty() {
                parsed.set_query(None);
            } else {
                let qs: String = clean_pairs
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&");
                parsed.set_query(Some(&qs));
            }

            return parsed.to_string().trim_end_matches('/').to_lowercase();
        }

        trimmed.to_lowercase()
    }

    /// Normalize a search query.
    pub fn normalize_query(query: &str) -> String {
        query.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheResult<CacheStats> {
        let conn = self.db.lock();

        let total_pages: i64 = conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0)).unwrap_or(0);
        let total_searches: i64 = conn.query_row("SELECT COUNT(*) FROM searches", [], |r| r.get(0)).unwrap_or(0);
        let total_snapshots: i64 = conn.query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0)).unwrap_or(0);
        let total_domains: i64 = conn.query_row("SELECT COUNT(*) FROM domain_profiles", [], |r| r.get(0)).unwrap_or(0);
        let total_disk_bytes: i64 = conn.query_row("SELECT COALESCE(SUM(raw_html_size), 0) FROM pages", [], |r| r.get(0)).unwrap_or(0);
        let total_cache_hits: i64 = conn.query_row("SELECT COALESCE(SUM(hit_count), 0) FROM pages", [], |r| r.get(0)).unwrap_or(0);
        let pinned_pages: i64 = conn.query_row("SELECT COUNT(*) FROM pages WHERE pinned = 1", [], |r| r.get(0)).unwrap_or(0);

        Ok(CacheStats {
            total_pages: total_pages as u64,
            total_searches: total_searches as u64,
            total_snapshots: total_snapshots as u64,
            total_domains: total_domains as u64,
            total_disk_bytes: total_disk_bytes as u64,
            total_cache_hits: total_cache_hits as u64,
            stale_pages: 0, // Would need to check each page
            pinned_pages: pinned_pages as u64,
            avg_page_age_secs: 0,
        })
    }
}

/// Cache statistics for monitoring and display.
#[derive(Debug, Default)]
pub struct CacheStats {
    pub total_pages: u64,
    pub total_searches: u64,
    pub total_snapshots: u64,
    pub total_domains: u64,
    pub total_disk_bytes: u64,
    pub total_cache_hits: u64,
    pub stale_pages: u64,
    pub pinned_pages: u64,
    pub avg_page_age_secs: u64,
}

/// Parse a ContentType from its Debug string representation.
fn parse_content_type(s: &str) -> ContentType {
    match s {
        "Documentation" => ContentType::Documentation,
        "News" => ContentType::News,
        "SearchResults" => ContentType::SearchResults,
        "ApiDocs" => ContentType::ApiDocs,
        "SocialMedia" => ContentType::SocialMedia,
        "Commerce" => ContentType::Commerce,
        "Government" => ContentType::Government,
        "Forum" => ContentType::Forum,
        "SourceCode" => ContentType::SourceCode,
        _ => ContentType::Generic,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (KnowledgeStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = KnowledgeStore::open(dir.path()).unwrap();
        (store, dir)
    }

    fn sample_page(url: &str) -> CachedPage {
        CachedPage {
            url_hash: KnowledgeStore::hash_url(url),
            url: url.to_string(),
            domain: "example.com".to_string(),
            title: "Test Page".to_string(),
            markdown: "# Test\n\nHello world.".to_string(),
            plain_text: "Test. Hello world.".to_string(),
            snippet: "Hello world.".to_string(),
            token_count: 10,
            links: vec![("Link".to_string(), "https://example.com/other".to_string())],
            content_type: ContentType::Generic,
            content_hash: blake3::hash(b"Hello world.").to_hex().to_string(),
            first_seen: Utc::now(),
            last_fetched: Utc::now(),
            last_validated: Utc::now(),
            hit_count: 0,
            change_count: 0,
            http_status: 200,
            etag: None,
            last_modified: None,
            pinned: false,
            agent_notes: None,
            tags: vec!["test".to_string()],
            raw_html_size: 100,
            extraction_confidence: 0.9,
        }
    }

    #[test]
    fn test_put_and_get_page() {
        let (store, _dir) = temp_store();
        let page = sample_page("https://example.com/test");
        let raw_html = "<html><body><h1>Test</h1><p>Hello world.</p></body></html>";

        store.put_page(&page, raw_html).unwrap();

        let cached = store.get_page("https://example.com/test").unwrap();
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.title, "Test Page");
        assert_eq!(cached.domain, "example.com");
        assert!(cached.hit_count >= 1);
    }

    #[test]
    fn test_cache_miss() {
        let (store, _dir) = temp_store();
        let result = store.get_page("https://nonexistent.com/page").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_raw_html_roundtrip() {
        let (store, _dir) = temp_store();
        let page = sample_page("https://example.com/html-test");
        let raw_html = "<html><body><h1>Hello</h1><p>Content here.</p></body></html>";

        store.put_page(&page, raw_html).unwrap();

        let retrieved = store.get_raw_html(&page.url_hash).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), raw_html);
    }

    #[test]
    fn test_snapshots() {
        let (store, _dir) = temp_store();

        let snap = CachedSnapshot {
            snapshot_id: "snap-1".to_string(),
            session_id: "session-abc".to_string(),
            url: "https://example.com".to_string(),
            step: 0,
            agent_text: "Page content snapshot".to_string(),
            element_count: 5,
            page_type: Some("article".to_string()),
            taken_at: Utc::now(),
            token_count: 20,
        };

        store.put_snapshot(&snap).unwrap();

        let snaps = store.get_snapshots("session-abc", None, None).unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].snapshot_id, "snap-1");
        assert_eq!(snaps[0].element_count, 5);
    }

    #[test]
    fn test_search_cache() {
        let (store, _dir) = temp_store();

        let search = CachedSearch {
            query_hash: KnowledgeStore::hash_query("rust web browser"),
            query: "rust web browser".to_string(),
            query_normalized: "rust web browser".to_string(),
            results: vec![SearchResultEntry {
                title: "Wraith".to_string(),
                url: "https://wraith.dev".to_string(),
                snippet: "AI browser in Rust".to_string(),
                provider: "test".to_string(),
                rank: 1,
                page_cached: false,
                relevance: 0.95,
            }],
            providers_used: vec!["test".to_string()],
            searched_at: Utc::now(),
            hit_count: 0,
            search_duration_ms: 150,
        };

        store.put_search(&search).unwrap();

        let cached = store.get_search("rust web browser").unwrap();
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.results.len(), 1);
        assert_eq!(cached.results[0].title, "Wraith");
    }

    #[test]
    fn test_pin_page() {
        let (store, _dir) = temp_store();
        let page = sample_page("https://example.com/pin-test");
        store.put_page(&page, "<html>test</html>").unwrap();

        store.pin_page("https://example.com/pin-test", Some("Important page")).unwrap();

        let cached = store.get_page("https://example.com/pin-test").unwrap().unwrap();
        assert!(cached.pinned);
        assert_eq!(cached.agent_notes, Some("Important page".to_string()));
    }

    #[test]
    fn test_domain_profile() {
        let (store, _dir) = temp_store();
        let page = sample_page("https://example.com/profile-test");
        store.put_page(&page, "<html>test</html>").unwrap();

        let profile = store.get_domain_profile("example.com").unwrap();
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert!(profile.pages_cached >= 1);
    }

    #[test]
    fn test_stats() {
        let (store, _dir) = temp_store();
        let page = sample_page("https://example.com/stats-test");
        store.put_page(&page, "<html>test</html>").unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.total_pages, 1);
    }

    #[test]
    fn test_normalize_url() {
        assert_eq!(
            KnowledgeStore::normalize_url("https://Example.com/Page?utm_source=twitter&id=5#section"),
            "https://example.com/page?id=5"
        );
    }

    #[test]
    fn test_staleness() {
        let (store, _dir) = temp_store();
        let mut page = sample_page("https://example.com/stale-test");

        // Fresh page
        assert!(!store.is_stale(&page));

        // Pinned page is never stale
        page.pinned = true;
        page.last_fetched = chrono::DateTime::from_timestamp(0, 0).unwrap();
        assert!(!store.is_stale(&page));

        // Old page is stale
        page.pinned = false;
        assert!(store.is_stale(&page));
    }
}
