-- ═══════════════════════════════════════════════════════════════════
-- Wraith Browser Knowledge Store Schema
-- SQLite with WAL mode, FTS5 for full-text search backup
-- ═══════════════════════════════════════════════════════════════════

-- Pages: the core knowledge unit
-- Every URL the agent visits gets a row here
CREATE TABLE IF NOT EXISTS pages (
    url_hash        TEXT PRIMARY KEY,
    url             TEXT NOT NULL,
    domain          TEXT NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    markdown        TEXT NOT NULL DEFAULT '',
    plain_text      TEXT NOT NULL DEFAULT '',
    snippet         TEXT NOT NULL DEFAULT '',
    token_count     INTEGER NOT NULL DEFAULT 0,
    links_json      TEXT NOT NULL DEFAULT '[]',
    content_type    TEXT NOT NULL DEFAULT 'Generic',
    content_hash    TEXT NOT NULL DEFAULT '',
    first_seen      TEXT NOT NULL DEFAULT (datetime('now')),
    last_fetched    TEXT NOT NULL DEFAULT (datetime('now')),
    last_validated  TEXT NOT NULL DEFAULT (datetime('now')),
    hit_count       INTEGER NOT NULL DEFAULT 0,
    change_count    INTEGER NOT NULL DEFAULT 0,
    http_status     INTEGER NOT NULL DEFAULT 200,
    etag            TEXT,
    last_modified   TEXT,
    pinned          INTEGER NOT NULL DEFAULT 0,
    agent_notes     TEXT,
    tags_json       TEXT NOT NULL DEFAULT '[]',
    raw_html_size   INTEGER NOT NULL DEFAULT 0,
    extraction_confidence REAL NOT NULL DEFAULT 0.0
);

CREATE INDEX IF NOT EXISTS idx_pages_domain ON pages(domain);
CREATE INDEX IF NOT EXISTS idx_pages_last_fetched ON pages(last_fetched);
CREATE INDEX IF NOT EXISTS idx_pages_pinned ON pages(pinned) WHERE pinned = 1;
CREATE INDEX IF NOT EXISTS idx_pages_content_type ON pages(content_type);
CREATE INDEX IF NOT EXISTS idx_pages_hit_count ON pages(hit_count DESC);

-- FTS5 virtual table for SQLite-level full-text search (backup to Tantivy)
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
    url,
    title,
    plain_text,
    snippet,
    tags_json,
    agent_notes,
    content='pages',
    content_rowid='rowid'
);

-- Triggers to keep FTS index in sync
CREATE TRIGGER IF NOT EXISTS pages_fts_insert AFTER INSERT ON pages BEGIN
    INSERT INTO pages_fts(rowid, url, title, plain_text, snippet, tags_json, agent_notes)
    VALUES (new.rowid, new.url, new.title, new.plain_text, new.snippet, new.tags_json, new.agent_notes);
END;

CREATE TRIGGER IF NOT EXISTS pages_fts_delete AFTER DELETE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, url, title, plain_text, snippet, tags_json, agent_notes)
    VALUES ('delete', old.rowid, old.url, old.title, old.plain_text, old.snippet, old.tags_json, old.agent_notes);
END;

CREATE TRIGGER IF NOT EXISTS pages_fts_update AFTER UPDATE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, url, title, plain_text, snippet, tags_json, agent_notes)
    VALUES ('delete', old.rowid, old.url, old.title, old.plain_text, old.snippet, old.tags_json, old.agent_notes);
    INSERT INTO pages_fts(rowid, url, title, plain_text, snippet, tags_json, agent_notes)
    VALUES (new.rowid, new.url, new.title, new.plain_text, new.snippet, new.tags_json, new.agent_notes);
END;

-- ═══════════════════════════════════════════════════════════════════
-- Search result cache
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS searches (
    query_hash        TEXT PRIMARY KEY,
    query             TEXT NOT NULL,
    query_normalized  TEXT NOT NULL,
    results_json      TEXT NOT NULL DEFAULT '[]',
    providers_json    TEXT NOT NULL DEFAULT '[]',
    searched_at       TEXT NOT NULL DEFAULT (datetime('now')),
    hit_count         INTEGER NOT NULL DEFAULT 0,
    search_duration_ms INTEGER NOT NULL DEFAULT 0,
    result_count      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_searches_searched_at ON searches(searched_at);
CREATE INDEX IF NOT EXISTS idx_searches_hit_count ON searches(hit_count DESC);

-- FTS for search queries (find cached searches by content)
CREATE VIRTUAL TABLE IF NOT EXISTS searches_fts USING fts5(
    query,
    query_normalized,
    results_snippets,
    content='searches',
    content_rowid='rowid'
);

-- ═══════════════════════════════════════════════════════════════════
-- DOM snapshots (session-scoped agent memory)
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id     TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    url             TEXT NOT NULL,
    step            INTEGER NOT NULL,
    agent_text      TEXT NOT NULL,
    element_count   INTEGER NOT NULL DEFAULT 0,
    page_type       TEXT,
    taken_at        TEXT NOT NULL DEFAULT (datetime('now')),
    token_count     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_snapshots_session ON snapshots(session_id, step);
CREATE INDEX IF NOT EXISTS idx_snapshots_url ON snapshots(url);

-- ═══════════════════════════════════════════════════════════════════
-- Domain profiles (adaptive staleness learning)
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS domain_profiles (
    domain                    TEXT PRIMARY KEY,
    pages_cached              INTEGER NOT NULL DEFAULT 0,
    avg_change_interval_secs  INTEGER,
    computed_ttl_secs         INTEGER NOT NULL DEFAULT 14400,
    override_ttl_secs         INTEGER,
    requires_auth             INTEGER NOT NULL DEFAULT 0,
    bot_hostile               INTEGER NOT NULL DEFAULT 0,
    avg_extraction_confidence REAL NOT NULL DEFAULT 0.0,
    default_content_type      TEXT NOT NULL DEFAULT 'Generic',
    total_bytes               INTEGER NOT NULL DEFAULT 0,
    total_hits                INTEGER NOT NULL DEFAULT 0,
    last_accessed             TEXT NOT NULL DEFAULT (datetime('now')),
    first_seen                TEXT NOT NULL DEFAULT (datetime('now')),
    supports_conditional      INTEGER NOT NULL DEFAULT 0,
    crawl_delay_secs          INTEGER
);

-- ═══════════════════════════════════════════════════════════════════
-- Agent sessions (for tracking browsing task history)
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS sessions (
    session_id      TEXT PRIMARY KEY,
    task_description TEXT NOT NULL,
    started_at      TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at    TEXT,
    status          TEXT NOT NULL DEFAULT 'running',
    steps_taken     INTEGER NOT NULL DEFAULT 0,
    urls_visited_json TEXT NOT NULL DEFAULT '[]',
    result          TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC);

-- ═══════════════════════════════════════════════════════════════════
-- URL redirect map (follow chains once, cache forever)
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS redirects (
    from_url_hash   TEXT NOT NULL,
    from_url        TEXT NOT NULL,
    to_url_hash     TEXT NOT NULL,
    to_url          TEXT NOT NULL,
    http_status     INTEGER NOT NULL,
    discovered_at   TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (from_url_hash)
);

-- ═══════════════════════════════════════════════════════════════════
-- Content change log (tracks what changed and when)
-- Used to compute adaptive TTLs per domain
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS change_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    url_hash        TEXT NOT NULL,
    old_content_hash TEXT NOT NULL,
    new_content_hash TEXT NOT NULL,
    changed_at      TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (url_hash) REFERENCES pages(url_hash) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_changelog_url ON change_log(url_hash, changed_at DESC);

-- ═══════════════════════════════════════════════════════════════════
-- Cookie/session store (persist login state across tasks)
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS cookies (
    domain          TEXT NOT NULL,
    name            TEXT NOT NULL,
    value           TEXT NOT NULL,
    path            TEXT NOT NULL DEFAULT '/',
    expires_at      TEXT,
    secure          INTEGER NOT NULL DEFAULT 0,
    http_only       INTEGER NOT NULL DEFAULT 0,
    same_site       TEXT NOT NULL DEFAULT 'Lax',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (domain, name, path)
);

CREATE INDEX IF NOT EXISTS idx_cookies_domain ON cookies(domain);

-- ═══════════════════════════════════════════════════════════════════
-- Schema version tracking
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO schema_version (version) VALUES (1);
