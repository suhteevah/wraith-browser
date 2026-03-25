-- ═══════════════════════════════════════════════════════════════════
-- Wraith Credential Vault Schema
-- Encrypted at rest — each secret blob is AES-256-GCM encrypted
-- ═══════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS vault_meta (
    key     TEXT PRIMARY KEY,
    value   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credentials (
    id                TEXT PRIMARY KEY,
    domain            TEXT NOT NULL,
    kind              TEXT NOT NULL,
    identity          TEXT NOT NULL,
    secret_encrypted  BLOB NOT NULL,
    label             TEXT,
    url_pattern       TEXT,
    auto_use          INTEGER NOT NULL DEFAULT 0,
    metadata_json     TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    last_used         TEXT,
    last_rotated      TEXT,
    use_count         INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_creds_domain ON credentials(domain);
CREATE INDEX IF NOT EXISTS idx_creds_kind ON credentials(domain, kind);
CREATE INDEX IF NOT EXISTS idx_creds_auto ON credentials(auto_use) WHERE auto_use = 1;

-- Approved domains — agent has been approved to use credentials here
-- Prevents credential use on unexpected domains without human confirmation
CREATE TABLE IF NOT EXISTS approved_domains (
    credential_id   TEXT NOT NULL,
    domain_pattern  TEXT NOT NULL,
    approved_at     TEXT NOT NULL DEFAULT (datetime('now')),
    approved_by     TEXT NOT NULL DEFAULT 'human',
    PRIMARY KEY (credential_id, domain_pattern),
    FOREIGN KEY (credential_id) REFERENCES credentials(id) ON DELETE CASCADE
);

-- Audit log — every credential access is logged
CREATE TABLE IF NOT EXISTS vault_audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    credential_id TEXT NOT NULL,
    action      TEXT NOT NULL,  -- 'read', 'store', 'delete', 'use', 'generate_totp'
    domain      TEXT,
    url         TEXT,
    session_id  TEXT,
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    success     INTEGER NOT NULL DEFAULT 1,
    details     TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_cred ON vault_audit_log(credential_id);
CREATE INDEX IF NOT EXISTS idx_audit_time ON vault_audit_log(timestamp DESC);

-- Human approval requests — pending requests for credential use
CREATE TABLE IF NOT EXISTS approval_queue (
    id              TEXT PRIMARY KEY,
    credential_id   TEXT NOT NULL,
    domain          TEXT NOT NULL,
    url             TEXT NOT NULL,
    reason          TEXT NOT NULL,
    requested_at    TEXT NOT NULL DEFAULT (datetime('now')),
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending, approved, denied, expired
    responded_at    TEXT,
    FOREIGN KEY (credential_id) REFERENCES credentials(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO vault_meta (key, value) VALUES ('schema_version', '1');
