use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
// CACHED PAGES — the core unit of knowledge
// ═══════════════════════════════════════════════════════════════════

/// A cached web page with all extracted data. This is the primary knowledge unit.
/// When an agent asks "what's on this URL?", this is what we return — instantly,
/// without hitting the network — unless it's stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPage {
    /// Primary key — blake3 hash of canonical URL
    pub url_hash: String,

    /// The canonical URL (normalized: no tracking params, no fragments)
    pub url: String,

    /// The domain (e.g., "docs.anthropic.com")
    pub domain: String,

    /// Page title
    pub title: String,

    /// Extracted markdown content (AI-readable)
    pub markdown: String,

    /// Plain text content (for full-text indexing)
    pub plain_text: String,

    /// Brief summary for search result display (first ~200 chars of content)
    pub snippet: String,

    /// Estimated token count of the markdown
    pub token_count: usize,

    /// Links found on this page: (text, url)
    pub links: Vec<(String, String)>,

    /// What type of content this is (affects staleness TTL)
    pub content_type: ContentType,

    /// Content hash — blake3 of the markdown, used for change detection
    pub content_hash: String,

    /// When this page was first cached
    pub first_seen: DateTime<Utc>,

    /// When this page was last fetched from the web
    pub last_fetched: DateTime<Utc>,

    /// When the cached content was last confirmed still valid
    pub last_validated: DateTime<Utc>,

    /// How many times this page has been accessed from cache
    pub hit_count: u64,

    /// How many times the content changed between fetches
    pub change_count: u64,

    /// HTTP status code from last fetch
    pub http_status: u16,

    /// HTTP ETag header (for conditional requests)
    pub etag: Option<String>,

    /// HTTP Last-Modified header (for conditional requests)
    pub last_modified: Option<String>,

    /// Whether an agent explicitly pinned this (never expires)
    pub pinned: bool,

    /// Agent-written notes about this page ("this is the pricing page for X")
    pub agent_notes: Option<String>,

    /// Tags for categorization ("api-docs", "competitor-pricing", etc.)
    pub tags: Vec<String>,

    /// Size of compressed raw HTML in blob store (bytes)
    pub raw_html_size: usize,

    /// Readability confidence score (0.0 - 1.0)
    pub extraction_confidence: f32,
}

// ═══════════════════════════════════════════════════════════════════
// CACHED SEARCH RESULTS — metasearch response caching
// ═══════════════════════════════════════════════════════════════════

/// Cached search query results. When an agent searches for something,
/// we cache the results so the same (or similar) query doesn't re-hit providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSearch {
    /// Primary key — blake3 hash of normalized query
    pub query_hash: String,

    /// The original search query
    pub query: String,

    /// Normalized query (lowercase, stopwords removed, stemmed)
    pub query_normalized: String,

    /// Search results
    pub results: Vec<SearchResultEntry>,

    /// Which providers returned results
    pub providers_used: Vec<String>,

    /// When this search was executed
    pub searched_at: DateTime<Utc>,

    /// How many times this cached result has been served
    pub hit_count: u64,

    /// How long the search took (ms)
    pub search_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultEntry {
    /// Result title
    pub title: String,

    /// Result URL
    pub url: String,

    /// Result snippet / description
    pub snippet: String,

    /// Which provider returned this result
    pub provider: String,

    /// Position in the provider's results (1-indexed)
    pub rank: u32,

    /// Whether we've already cached/extracted the full page
    pub page_cached: bool,

    /// Relevance score (0.0 - 1.0, computed across providers)
    pub relevance: f32,
}

// ═══════════════════════════════════════════════════════════════════
// CACHED DOM SNAPSHOTS — session-scoped agent memory
// ═══════════════════════════════════════════════════════════════════

/// A cached DOM snapshot from an agent browsing session.
/// Session-scoped: these aren't meant to survive across sessions,
/// but they let an agent reference "what I saw 5 steps ago" without re-fetching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSnapshot {
    /// Unique snapshot ID
    pub snapshot_id: String,

    /// Session/task ID this snapshot belongs to
    pub session_id: String,

    /// URL at time of snapshot
    pub url: String,

    /// Step number in the agent's task (0-indexed)
    pub step: usize,

    /// The compact agent-text representation
    pub agent_text: String,

    /// Number of interactive elements
    pub element_count: usize,

    /// Detected page type
    pub page_type: Option<String>,

    /// When the snapshot was taken
    pub taken_at: DateTime<Utc>,

    /// Token count of the agent_text
    pub token_count: usize,
}

// ═══════════════════════════════════════════════════════════════════
// DOMAIN PROFILES — learned behavior per domain
// ═══════════════════════════════════════════════════════════════════

/// Learned profile for a domain. The cache observes how content changes
/// over time and adjusts TTLs automatically. If docs.rust-lang.org rarely
/// changes, extend its TTL. If twitter.com changes every minute, shorten it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainProfile {
    /// Domain name (e.g., "github.com")
    pub domain: String,

    /// How many pages we've cached from this domain
    pub pages_cached: u64,

    /// Average time between content changes (seconds)
    /// Computed from observing content_hash changes across fetches.
    pub avg_change_interval_secs: Option<u64>,

    /// Computed optimal TTL based on observed change frequency
    pub computed_ttl_secs: u64,

    /// Override TTL (set by agent or user, takes precedence)
    pub override_ttl_secs: Option<u64>,

    /// Whether this domain requires authentication
    pub requires_auth: bool,

    /// Whether this domain blocks bots (detected by 403/captcha)
    pub bot_hostile: bool,

    /// Average extraction confidence for pages on this domain
    pub avg_extraction_confidence: f32,

    /// Default content type for this domain
    pub default_content_type: ContentType,

    /// Total bytes stored for this domain
    pub total_bytes: u64,

    /// Total cache hits served for this domain
    pub total_hits: u64,

    /// Last time we fetched anything from this domain
    pub last_accessed: DateTime<Utc>,

    /// First time we cached anything from this domain
    pub first_seen: DateTime<Utc>,

    /// Whether the domain supports conditional requests (ETag/If-Modified-Since)
    pub supports_conditional: bool,

    /// Known robots.txt crawl-delay (seconds)
    pub crawl_delay_secs: Option<u64>,
}

// ═══════════════════════════════════════════════════════════════════
// CONTENT TYPE — affects staleness TTL
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContentType {
    /// Documentation, wikis, reference material (TTL: 7 days)
    Documentation,

    /// News articles, blog posts (TTL: 1 hour)
    News,

    /// Search engine result pages (TTL: 6 hours)
    SearchResults,

    /// API documentation (TTL: 24 hours)
    ApiDocs,

    /// Social media feeds (TTL: 15 minutes)
    SocialMedia,

    /// E-commerce, pricing pages (TTL: 1 hour)
    Commerce,

    /// Government, legal pages (TTL: 30 days)
    Government,

    /// Forums, Q&A (TTL: 12 hours)
    Forum,

    /// Source code repositories (TTL: 2 hours)
    SourceCode,

    /// Generic / unclassified (TTL: 4 hours)
    Generic,
}

impl ContentType {
    /// Default TTL in seconds for this content type.
    pub fn default_ttl_secs(&self) -> u64 {
        match self {
            ContentType::Documentation => 7 * 24 * 3600,    // 7 days
            ContentType::News => 3600,                        // 1 hour
            ContentType::SearchResults => 6 * 3600,           // 6 hours
            ContentType::ApiDocs => 24 * 3600,                // 24 hours
            ContentType::SocialMedia => 15 * 60,              // 15 minutes
            ContentType::Commerce => 3600,                    // 1 hour
            ContentType::Government => 30 * 24 * 3600,       // 30 days
            ContentType::Forum => 12 * 3600,                  // 12 hours
            ContentType::SourceCode => 2 * 3600,              // 2 hours
            ContentType::Generic => 4 * 3600,                 // 4 hours
        }
    }

    /// Detect content type from URL and page content heuristics.
    pub fn detect(url: &str, title: &str, _content: &str) -> Self {
        let url_lower = url.to_lowercase();
        let title_lower = title.to_lowercase();

        // Domain-based detection
        if url_lower.contains("docs.") || url_lower.contains("/docs/")
            || url_lower.contains("wiki") || url_lower.contains("readme")
            || url_lower.contains("documentation")
        {
            return ContentType::Documentation;
        }

        if url_lower.contains("api.") || url_lower.contains("/api/")
            || url_lower.contains("swagger") || url_lower.contains("openapi")
            || url_lower.contains("docs.rs") || url_lower.contains("crates.io")
        {
            return ContentType::ApiDocs;
        }

        if url_lower.contains("github.com") || url_lower.contains("gitlab.com")
            || url_lower.contains("bitbucket.org") || url_lower.contains("codeberg.org")
        {
            return ContentType::SourceCode;
        }

        if url_lower.contains("news") || url_lower.contains("blog")
            || url_lower.contains("article") || url_lower.contains("press")
            || url_lower.contains("medium.com") || url_lower.contains("substack.com")
        {
            return ContentType::News;
        }

        if url_lower.contains("twitter.com") || url_lower.contains("x.com")
            || url_lower.contains("mastodon") || url_lower.contains("reddit.com")
            || url_lower.contains("threads.net") || url_lower.contains("bsky")
        {
            return ContentType::SocialMedia;
        }

        if url_lower.contains("shop") || url_lower.contains("price")
            || url_lower.contains("cart") || url_lower.contains("product")
            || url_lower.contains("amazon.") || url_lower.contains("ebay.")
            || title_lower.contains("pricing")
        {
            return ContentType::Commerce;
        }

        if url_lower.contains(".gov") || url_lower.contains("law")
            || url_lower.contains("regulation") || url_lower.contains("legal")
        {
            return ContentType::Government;
        }

        if url_lower.contains("forum") || url_lower.contains("stackoverflow.com")
            || url_lower.contains("stackexchange.com") || url_lower.contains("discuss")
            || url_lower.contains("discourse")
        {
            return ContentType::Forum;
        }

        if url_lower.contains("google.com/search") || url_lower.contains("bing.com/search")
            || url_lower.contains("duckduckgo.com") || url_lower.contains("search?q=")
        {
            return ContentType::SearchResults;
        }

        ContentType::Generic
    }
}
