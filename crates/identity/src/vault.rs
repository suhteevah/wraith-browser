//! # Encrypted Credential Vault
//!
//! The vault stores all credentials in an SQLite database with each secret
//! individually encrypted using AES-256-GCM. The encryption key is derived
//! from a master passphrase using argon2id (memory-hard KDF).
//!
//! ## Key Derivation
//!
//! ```text
//! User passphrase ──► argon2id(salt, passphrase) ──► 256-bit master key
//!                                                         │
//!                    ┌────────────────────────────────────┘
//!                    │
//!                    ▼
//!              AES-256-GCM(master_key, unique_nonce, plaintext_secret)
//!                    │
//!                    ▼
//!              encrypted_blob stored in SQLite
//! ```
//!
//! ## File Security
//!
//! - Vault file: `~/.wraith/vault.db` with 0600 permissions
//! - Salt stored alongside vault (not secret, just unique per vault)
//! - Master key NEVER written to disk — derived at unlock, held in memory
//! - On lock: master key zeroized, all decrypted credentials dropped

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use aes_gcm::aead::Aead;
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
#[cfg(test)]
use argon2::{Params, Algorithm, Version};
use base64::Engine as _;
use parking_lot::RwLock;
use rand::rngs::OsRng;
use secrecy::{SecretString, SecretBox, ExposeSecret};
use tracing::{info, warn, debug, instrument};

use serde::{Serialize, Deserialize};

use crate::credential::*;
use crate::error::*;

/// The encrypted credential vault.
pub struct CredentialVault {
    /// Path to the vault SQLite database
    _db_path: PathBuf,

    /// SQLite connection
    db: Arc<parking_lot::Mutex<rusqlite::Connection>>,

    /// The derived master encryption key — None when locked.
    /// Wrapped in SecretBox for zeroize-on-drop.
    master_key: RwLock<Option<SecretBox<Vec<u8>>>>,

    /// Salt for key derivation (stored in vault metadata)
    salt: String,

    /// Whether the vault is currently unlocked
    unlocked: RwLock<bool>,
}

impl CredentialVault {
    /// Open an existing vault or create a new one.
    /// The vault starts LOCKED — call `unlock()` before accessing credentials.
    #[instrument(fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> IdentityResult<Self> {
        let db_path = path.as_ref().to_path_buf();
        let parent = db_path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|e| IdentityError::Internal(anyhow::anyhow!("Failed to create vault dir: {}", e)))?;

        info!(path = %db_path.display(), "Opening credential vault");

        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;
        ")?;

        // Initialize schema
        conn.execute_batch(include_str!("sql/vault_schema.sql"))?;

        // Set file permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = std::fs::set_permissions(&db_path, perms) {
                warn!(error = %e, "Failed to set vault file permissions to 0600");
            } else {
                debug!("Vault file permissions set to 0600 (owner-only)");
            }
        }

        // Get or create salt
        let salt = Self::get_or_create_salt(&conn)?;

        let vault = Self {
            _db_path: db_path,
            db: Arc::new(parking_lot::Mutex::new(conn)),
            master_key: RwLock::new(None),
            salt,
            unlocked: RwLock::new(false),
        };

        info!("Credential vault opened (LOCKED — call unlock() to access credentials)");
        Ok(vault)
    }

    /// Create a new vault with a master passphrase.
    /// This sets the passphrase for the first time.
    #[instrument(skip(self, passphrase))]
    pub fn initialize(&self, passphrase: &SecretString) -> IdentityResult<()> {
        info!("Initializing vault with master passphrase");

        // Derive key from passphrase
        let key = self.derive_key(passphrase)?;

        // Store a verification token — encrypt a known value so we can
        // verify the passphrase is correct on unlock
        let verification = b"WRAITH_VAULT_V1";
        let encrypted = self.encrypt_bytes(&key, verification)?;

        let conn = self.db.lock();
        conn.execute(
            "INSERT OR REPLACE INTO vault_meta (key, value) VALUES ('verification', ?1)",
            rusqlite::params![base64::engine::general_purpose::STANDARD.encode(&encrypted)],
        )?;

        // Store the key in memory
        *self.master_key.write() = Some(key);
        *self.unlocked.write() = true;

        info!("Vault initialized and unlocked");
        Ok(())
    }

    /// Unlock the vault with the master passphrase.
    #[instrument(skip(self, passphrase))]
    pub fn unlock(&self, passphrase: &SecretString) -> IdentityResult<()> {
        info!("Unlocking credential vault");

        // Derive key from passphrase
        let key = self.derive_key(passphrase)?;

        // Verify against stored verification token
        let conn = self.db.lock();
        let encrypted_b64: Option<String> = conn.query_row(
            "SELECT value FROM vault_meta WHERE key = 'verification'",
            [],
            |row| row.get(0),
        ).ok();

        if let Some(encrypted_b64) = encrypted_b64 {
            let encrypted = base64::engine::general_purpose::STANDARD.decode(&encrypted_b64)
                .map_err(|e| IdentityError::DecryptionFailed(e.to_string()))?;

            let decrypted = self.decrypt_bytes(&key, &encrypted)?;
            if decrypted != b"WRAITH_VAULT_V1" {
                warn!("Vault unlock failed — wrong passphrase");
                return Err(IdentityError::WrongPassphrase);
            }
        } else {
            // No verification token — vault needs initialization
            drop(conn);
            return self.initialize(passphrase);
        }

        drop(conn);

        *self.master_key.write() = Some(key);
        *self.unlocked.write() = true;

        let cred_count = self.list_credentials()?.len();
        info!(credentials = cred_count, "Vault unlocked successfully");
        Ok(())
    }

    /// Lock the vault — zeroize the master key from memory.
    #[instrument(skip(self))]
    pub fn lock(&self) {
        info!("Locking credential vault");
        *self.master_key.write() = None; // SecretBox handles zeroize on drop
        *self.unlocked.write() = false;
        info!("Vault locked — master key zeroized from memory");
    }

    pub fn is_unlocked(&self) -> bool {
        *self.unlocked.read()
    }

    /// Store a credential in the vault.
    #[instrument(skip(self, request), fields(domain = %request.domain, kind = ?request.kind))]
    pub fn store(&self, request: StoreCredentialRequest) -> IdentityResult<String> {
        self.require_unlocked()?;

        let key = self.get_key()?;
        let id = uuid::Uuid::new_v4().to_string();

        info!(
            id = %id,
            domain = %request.domain,
            kind = ?request.kind,
            identity = %request.identity,
            auto_use = request.auto_use,
            "Storing credential in vault"
        );

        // Encrypt the secret
        let secret_bytes = request.secret.expose_secret().as_bytes();
        let encrypted = self.encrypt_bytes(&key, secret_bytes)?;

        let conn = self.db.lock();
        conn.execute(
            "INSERT INTO credentials (
                id, domain, kind, identity, secret_encrypted,
                label, url_pattern, auto_use, metadata_json,
                created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            rusqlite::params![
                id,
                request.domain,
                format!("{:?}", request.kind),
                request.identity,
                encrypted,
                request.label,
                request.url_pattern,
                request.auto_use,
                request.metadata.to_string(),
            ],
        )?;

        info!(id = %id, domain = %request.domain, "Credential stored");
        Ok(id)
    }

    /// Retrieve and decrypt a credential for a domain.
    #[instrument(skip(self), fields(domain = %domain))]
    pub fn get(
        &self,
        domain: &str,
        kind: Option<CredentialKind>,
    ) -> IdentityResult<DecryptedCredential> {
        self.require_unlocked()?;
        let key = self.get_key()?;

        debug!(domain = %domain, kind = ?kind, "Retrieving credential");

        let conn = self.db.lock();

        let (id, identity, encrypted, cred_kind): (String, String, Vec<u8>, String) = if let Some(k) = kind {
            conn.query_row(
                "SELECT id, identity, secret_encrypted, kind FROM credentials
                 WHERE domain = ?1 AND kind = ?2
                 ORDER BY last_used DESC NULLS LAST, created_at DESC
                 LIMIT 1",
                rusqlite::params![domain, format!("{:?}", k)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|_| IdentityError::CredentialNotFound {
                domain: domain.to_string(),
                kind: format!("{:?}", k),
            })?
        } else {
            conn.query_row(
                "SELECT id, identity, secret_encrypted, kind FROM credentials
                 WHERE domain = ?1
                 ORDER BY last_used DESC NULLS LAST, created_at DESC
                 LIMIT 1",
                rusqlite::params![domain],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|_| IdentityError::CredentialNotFound {
                domain: domain.to_string(),
                kind: "any".to_string(),
            })?
        };

        // Update last_used and use_count
        conn.execute(
            "UPDATE credentials SET last_used = datetime('now'), use_count = use_count + 1 WHERE id = ?1",
            rusqlite::params![id],
        )?;

        drop(conn);

        // Decrypt the secret
        let decrypted_bytes = self.decrypt_bytes(&key, &encrypted)?;
        let secret_str = String::from_utf8(decrypted_bytes)
            .map_err(|e| IdentityError::DecryptionFailed(format!("UTF-8 decode: {}", e)))?;

        debug!(
            id = %id,
            domain = %domain,
            kind = %cred_kind,
            "Credential decrypted (secret in memory, will zeroize on drop)"
        );

        Ok(DecryptedCredential {
            id,
            domain: domain.to_string(),
            kind: kind.unwrap_or(CredentialKind::Generic),
            identity,
            secret: SecretString::from(secret_str),
        })
    }

    /// Generate a current TOTP code for a domain.
    #[instrument(skip(self), fields(domain = %domain))]
    pub fn generate_totp(&self, domain: &str) -> IdentityResult<String> {
        let seed_cred = self.get(domain, Some(CredentialKind::TotpSeed))?;
        let seed = seed_cred.expose_secret_value();

        debug!(domain = %domain, "Generating TOTP code");

        let totp = totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA1,
            6,
            1,
            30,
            seed.as_bytes().to_vec(),
            None,
            domain.to_string(),
        ).map_err(|e| IdentityError::TotpFailed(e.to_string()))?;

        let code = totp.generate_current()
            .map_err(|e| IdentityError::TotpFailed(e.to_string()))?;

        info!(domain = %domain, "TOTP code generated (valid for ~30s)");
        Ok(code)
    }

    /// List all credentials (metadata only — secrets stay encrypted).
    pub fn list_credentials(&self) -> IdentityResult<Vec<CredentialSummary>> {
        let conn = self.db.lock();
        let mut stmt = conn.prepare(
            "SELECT id, domain, kind, identity, label, url_pattern,
                    auto_use, created_at, last_used, use_count
             FROM credentials ORDER BY domain, kind"
        )?;

        let results = stmt.query_map([], |row| {
            Ok(CredentialSummary {
                id: row.get(0)?,
                domain: row.get(1)?,
                kind: row.get(2)?,
                identity: row.get(3)?,
                label: row.get(4)?,
                url_pattern: row.get(5)?,
                auto_use: row.get(6)?,
                created_at: row.get(7)?,
                last_used: row.get(8)?,
                use_count: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    /// Delete a credential by ID.
    #[instrument(skip(self), fields(id = %id))]
    pub fn delete(&self, id: &str) -> IdentityResult<()> {
        self.require_unlocked()?;
        info!(id = %id, "Deleting credential from vault");

        let conn = self.db.lock();
        let affected = conn.execute(
            "DELETE FROM credentials WHERE id = ?1",
            rusqlite::params![id],
        )?;

        if affected == 0 {
            return Err(IdentityError::CredentialNotFound {
                domain: "unknown".to_string(),
                kind: id.to_string(),
            });
        }

        info!(id = %id, "Credential deleted");
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // INTERNAL CRYPTO
    // ═══════════════════════════════════════════════════════════════

    /// Derive a 256-bit encryption key from passphrase using argon2id.
    fn derive_key(&self, passphrase: &SecretString) -> IdentityResult<SecretBox<Vec<u8>>> {
        debug!("Deriving encryption key with argon2id");

        let salt = SaltString::from_b64(&self.salt)
            .map_err(|e| IdentityError::EncryptionFailed(format!("Invalid salt: {}", e)))?;

        // In tests, use minimal params for speed. In production, use OWASP-recommended defaults.
        #[cfg(test)]
        let argon2 = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(1024, 1, 1, Some(32))
                .expect("valid argon2 test params"),
        );
        #[cfg(not(test))]
        let argon2 = Argon2::default();

        // Hash the passphrase — we extract the raw hash bytes as our AES key
        let hash = argon2
            .hash_password(passphrase.expose_secret().as_bytes(), &salt)
            .map_err(|e| IdentityError::EncryptionFailed(format!("Argon2 failed: {}", e)))?;

        let hash_bytes = hash.hash.ok_or_else(|| {
            IdentityError::EncryptionFailed("Argon2 produced no hash".to_string())
        })?;

        // Take first 32 bytes for AES-256
        let key_bytes: Vec<u8> = hash_bytes.as_bytes()[..32].to_vec();
        Ok(SecretBox::new(Box::new(key_bytes)))
    }

    /// Encrypt bytes with AES-256-GCM using a random nonce.
    /// Returns: nonce (12 bytes) || ciphertext
    fn encrypt_bytes(&self, key: &SecretBox<Vec<u8>>, plaintext: &[u8]) -> IdentityResult<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key.expose_secret())
            .map_err(|e| IdentityError::EncryptionFailed(e.to_string()))?;

        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher.encrypt(nonce, plaintext)
            .map_err(|e| IdentityError::EncryptionFailed(e.to_string()))?;

        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt bytes: expects nonce (12 bytes) || ciphertext.
    fn decrypt_bytes(&self, key: &SecretBox<Vec<u8>>, data: &[u8]) -> IdentityResult<Vec<u8>> {
        if data.len() < 12 {
            return Err(IdentityError::DecryptionFailed("Data too short".to_string()));
        }

        let (nonce_bytes, ciphertext) = data.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(key.expose_secret())
            .map_err(|e| IdentityError::DecryptionFailed(e.to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher.decrypt(nonce, ciphertext)
            .map_err(|e| IdentityError::DecryptionFailed(e.to_string()))
    }

    fn require_unlocked(&self) -> IdentityResult<()> {
        if !*self.unlocked.read() {
            Err(IdentityError::VaultLocked)
        } else {
            Ok(())
        }
    }

    fn get_key(&self) -> IdentityResult<SecretBox<Vec<u8>>> {
        self.master_key
            .read()
            .as_ref()
            .map(|k: &SecretBox<Vec<u8>>| SecretBox::new(Box::new(k.expose_secret().clone())))
            .ok_or(IdentityError::VaultLocked)
    }

    fn get_or_create_salt(conn: &rusqlite::Connection) -> IdentityResult<String> {
        let existing: Option<String> = conn.query_row(
            "SELECT value FROM vault_meta WHERE key = 'salt'",
            [],
            |row| row.get(0),
        ).ok();

        if let Some(salt) = existing {
            debug!("Using existing vault salt");
            Ok(salt)
        } else {
            let salt = SaltString::generate(&mut OsRng);
            let salt_str = salt.as_str().to_string();
            conn.execute(
                "INSERT INTO vault_meta (key, value) VALUES ('salt', ?1)",
                rusqlite::params![salt_str],
            )?;
            info!("Generated new vault salt");
            Ok(salt_str)
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // AUDIT LOGGING — every credential access is recorded
    // ═══════════════════════════════════════════════════════════════

    /// Log a credential access to the audit trail.
    fn audit_log(
        &self,
        credential_id: &str,
        action: &str,
        domain: Option<&str>,
        session_id: Option<&str>,
        success: bool,
        details: Option<&str>,
    ) {
        let conn = self.db.lock();
        let result = conn.execute(
            "INSERT INTO vault_audit_log (credential_id, action, domain, session_id, success, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                credential_id,
                action,
                domain,
                session_id,
                success as i32,
                details,
            ],
        );
        if let Err(e) = result {
            warn!(error = %e, credential_id, action, "Failed to write audit log");
        }
    }

    /// Get recent audit log entries.
    pub fn audit_history(&self, limit: usize) -> IdentityResult<Vec<AuditEntry>> {
        let conn = self.db.lock();
        let mut stmt = conn.prepare(
            "SELECT id, credential_id, action, domain, session_id, timestamp, success, details
             FROM vault_audit_log ORDER BY timestamp DESC LIMIT ?1"
        )?;

        let results = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok(AuditEntry {
                id: row.get(0)?,
                credential_id: row.get(1)?,
                action: row.get(2)?,
                domain: row.get(3)?,
                session_id: row.get(4)?,
                timestamp: row.get(5)?,
                success: row.get(6)?,
                details: row.get(7)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    // ═══════════════════════════════════════════════════════════════
    // DOMAIN APPROVAL — prevent credential leakage to unexpected domains
    // ═══════════════════════════════════════════════════════════════

    /// Check if a credential is approved for use on a given domain.
    pub fn is_domain_approved(&self, credential_id: &str, domain: &str) -> IdentityResult<bool> {
        let conn = self.db.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM approved_domains
             WHERE credential_id = ?1 AND (domain_pattern = ?2 OR ?2 LIKE REPLACE(domain_pattern, '*', '%'))",
            rusqlite::params![credential_id, domain],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Approve a credential for use on a domain.
    #[instrument(skip(self), fields(credential_id = %credential_id, domain = %domain))]
    pub fn approve_domain(&self, credential_id: &str, domain: &str) -> IdentityResult<()> {
        self.require_unlocked()?;
        info!(credential_id, domain, "Approving domain for credential use");

        {
            let conn = self.db.lock();
            conn.execute(
                "INSERT OR IGNORE INTO approved_domains (credential_id, domain_pattern)
                 VALUES (?1, ?2)",
                rusqlite::params![credential_id, domain],
            )?;
        }

        self.audit_log(credential_id, "approve_domain", Some(domain), None, true, None);
        Ok(())
    }

    /// Revoke domain approval for a credential.
    pub fn revoke_domain(&self, credential_id: &str, domain: &str) -> IdentityResult<()> {
        self.require_unlocked()?;
        {
            let conn = self.db.lock();
            conn.execute(
                "DELETE FROM approved_domains WHERE credential_id = ?1 AND domain_pattern = ?2",
                rusqlite::params![credential_id, domain],
            )?;
        }
        self.audit_log(credential_id, "revoke_domain", Some(domain), None, true, None);
        Ok(())
    }

    /// Get a credential with domain approval check.
    /// Returns HumanRequired error if the credential isn't approved for the target domain.
    #[instrument(skip(self), fields(domain = %domain, target_url = %target_url))]
    pub fn get_with_approval(
        &self,
        domain: &str,
        kind: Option<CredentialKind>,
        target_url: &str,
    ) -> IdentityResult<DecryptedCredential> {
        let cred = self.get(domain, kind)?;

        // Auto-use credentials skip approval
        // Check by querying the DB directly since DecryptedCredential doesn't have auto_use
        let auto_use: bool = {
            let conn = self.db.lock();
            conn.query_row(
                "SELECT auto_use FROM credentials WHERE id = ?1",
                rusqlite::params![cred.id],
                |row| row.get::<_, bool>(0),
            ).unwrap_or(false)
        };

        if auto_use {
            self.audit_log(&cred.id, "use", Some(domain), None, true, Some("auto_use"));
            return Ok(cred);
        }

        // Check if domain is approved
        if !self.is_domain_approved(&cred.id, domain)? {
            // Credentials that require human approval always need domain approval
            if cred.kind.requires_human_approval() {
                self.audit_log(&cred.id, "use_blocked", Some(domain), None, false,
                    Some("domain not approved, human approval required"));
                return Err(IdentityError::HumanRequired {
                    reason: format!(
                        "{:?} credential '{}' for {} needs approval before use on {}",
                        cred.kind, cred.identity, domain, target_url
                    ),
                });
            }
        }

        self.audit_log(&cred.id, "use", Some(domain), None, true, None);
        Ok(cred)
    }

    /// Rotate a credential — update the secret value.
    #[instrument(skip(self, new_secret), fields(credential_id = %credential_id))]
    pub fn rotate(&self, credential_id: &str, new_secret: &SecretString) -> IdentityResult<()> {
        self.require_unlocked()?;
        let key = self.get_key()?;

        info!(credential_id, "Rotating credential secret");

        let encrypted = self.encrypt_bytes(&key, new_secret.expose_secret().as_bytes())?;

        let affected = {
            let conn = self.db.lock();
            conn.execute(
                "UPDATE credentials SET secret_encrypted = ?1, last_rotated = datetime('now') WHERE id = ?2",
                rusqlite::params![encrypted, credential_id],
            )?
        };

        if affected == 0 {
            return Err(IdentityError::CredentialNotFound {
                domain: "unknown".to_string(),
                kind: credential_id.to_string(),
            });
        }

        self.audit_log(credential_id, "rotate", None, None, true, None);
        info!(credential_id, "Credential rotated successfully");
        Ok(())
    }
}

/// Audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub credential_id: String,
    pub action: String,
    pub domain: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: String,
    pub success: bool,
    pub details: Option<String>,
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        tracing::debug!("CredentialVault dropping — master key will be zeroized");
        // SecretBox handles zeroize automatically
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn temp_vault() -> (CredentialVault, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_vault.db");
        let vault = CredentialVault::open(&path).unwrap();
        (vault, dir)
    }

    #[test]
    fn test_initialize_and_unlock() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("test-passphrase-123".to_string());

        // Initialize
        vault.initialize(&passphrase).unwrap();
        assert!(vault.is_unlocked());

        // Lock
        vault.lock();
        assert!(!vault.is_unlocked());

        // Unlock with correct passphrase
        vault.unlock(&passphrase).unwrap();
        assert!(vault.is_unlocked());
    }

    #[test]
    fn test_wrong_passphrase_rejected() {
        let (vault, _dir) = temp_vault();
        let correct = SecretString::from("correct-passphrase".to_string());
        let wrong = SecretString::from("wrong-passphrase".to_string());

        vault.initialize(&correct).unwrap();
        vault.lock();

        let result = vault.unlock(&wrong);
        assert!(result.is_err());
        assert!(!vault.is_unlocked());
    }

    #[test]
    fn test_store_and_retrieve_credential() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("my-vault-pass".to_string());
        vault.initialize(&passphrase).unwrap();

        // Store
        let request = StoreCredentialRequest {
            domain: "github.com".to_string(),
            kind: CredentialKind::Password,
            identity: "user@example.com".to_string(),
            secret: SecretString::from("super-secret-password".to_string()),
            label: Some("Personal GitHub".to_string()),
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let id = vault.store(request).unwrap();
        assert!(!id.is_empty());

        // Retrieve
        let cred = vault.get("github.com", Some(CredentialKind::Password)).unwrap();
        assert_eq!(cred.identity, "user@example.com");
        assert_eq!(cred.expose_secret_value(), "super-secret-password");
        assert_eq!(cred.domain, "github.com");

        // List
        let list = vault.list_credentials().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].domain, "github.com");
        assert_eq!(list[0].identity, "user@example.com");
    }

    #[test]
    fn test_credential_not_found() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        let result = vault.get("nonexistent.com", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_credential() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        let request = StoreCredentialRequest {
            domain: "example.com".to_string(),
            kind: CredentialKind::ApiKey,
            identity: "api-user".to_string(),
            secret: SecretString::from("sk-12345".to_string()),
            label: None,
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let id = vault.store(request).unwrap();
        assert_eq!(vault.list_credentials().unwrap().len(), 1);

        vault.delete(&id).unwrap();
        assert_eq!(vault.list_credentials().unwrap().len(), 0);
    }

    #[test]
    fn test_rotate_credential() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        let request = StoreCredentialRequest {
            domain: "example.com".to_string(),
            kind: CredentialKind::Password,
            identity: "admin".to_string(),
            secret: SecretString::from("old-password".to_string()),
            label: None,
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let id = vault.store(request).unwrap();

        // Rotate
        let new_secret = SecretString::from("new-password".to_string());
        vault.rotate(&id, &new_secret).unwrap();

        // Verify new secret
        let cred = vault.get("example.com", Some(CredentialKind::Password)).unwrap();
        assert_eq!(cred.expose_secret_value(), "new-password");
    }

    #[test]
    fn test_domain_approval() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        let request = StoreCredentialRequest {
            domain: "github.com".to_string(),
            kind: CredentialKind::Password,
            identity: "user".to_string(),
            secret: SecretString::from("pass123".to_string()),
            label: None,
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let id = vault.store(request).unwrap();

        // Not approved yet
        assert!(!vault.is_domain_approved(&id, "github.com").unwrap());

        // Approve
        vault.approve_domain(&id, "github.com").unwrap();
        assert!(vault.is_domain_approved(&id, "github.com").unwrap());

        // Revoke
        vault.revoke_domain(&id, "github.com").unwrap();
        assert!(!vault.is_domain_approved(&id, "github.com").unwrap());
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("roundtrip-test".to_string());
        vault.initialize(&passphrase).unwrap();
        let key = vault.get_key().unwrap();

        let plaintext = b"The quick brown fox jumps over the lazy dog";
        let encrypted = vault.encrypt_bytes(&key, plaintext).unwrap();

        // Encrypted is larger (nonce + ciphertext + tag)
        assert!(encrypted.len() > plaintext.len());
        // Encrypted differs from plaintext
        assert_ne!(&encrypted[12..], plaintext);

        let decrypted = vault.decrypt_bytes(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_vault_locked_rejects_operations() {
        let (vault, _dir) = temp_vault();
        // Vault starts locked
        assert!(!vault.is_unlocked());

        let request = StoreCredentialRequest {
            domain: "test.com".to_string(),
            kind: CredentialKind::Password,
            identity: "user".to_string(),
            secret: SecretString::from("pass".to_string()),
            label: None,
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        // Store should fail when locked
        let result = vault.store(request);
        assert!(result.is_err());
    }

    #[test]
    fn test_audit_log() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        let request = StoreCredentialRequest {
            domain: "audit-test.com".to_string(),
            kind: CredentialKind::ApiKey,
            identity: "key-user".to_string(),
            secret: SecretString::from("api-key-123".to_string()),
            label: None,
            url_pattern: None,
            auto_use: false,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        };

        let id = vault.store(request).unwrap();
        vault.approve_domain(&id, "audit-test.com").unwrap();

        let history = vault.audit_history(10).unwrap();
        assert!(!history.is_empty());
        assert_eq!(history[0].action, "approve_domain");
    }

    #[test]
    fn test_multiple_credentials_same_domain() {
        let (vault, _dir) = temp_vault();
        let passphrase = SecretString::from("pass".to_string());
        vault.initialize(&passphrase).unwrap();

        // Store password
        vault.store(StoreCredentialRequest {
            domain: "multi.com".to_string(),
            kind: CredentialKind::Password,
            identity: "user@multi.com".to_string(),
            secret: SecretString::from("pass123".to_string()),
            label: None, url_pattern: None, auto_use: false,
            metadata: serde_json::json!({}),
        }).unwrap();

        // Store API key for same domain
        vault.store(StoreCredentialRequest {
            domain: "multi.com".to_string(),
            kind: CredentialKind::ApiKey,
            identity: "api-key".to_string(),
            secret: SecretString::from("sk-abc123".to_string()),
            label: None, url_pattern: None, auto_use: false,
            metadata: serde_json::json!({}),
        }).unwrap();

        // Retrieve by kind
        let pw = vault.get("multi.com", Some(CredentialKind::Password)).unwrap();
        assert_eq!(pw.identity, "user@multi.com");

        let api = vault.get("multi.com", Some(CredentialKind::ApiKey)).unwrap();
        assert_eq!(api.identity, "api-key");
        assert_eq!(api.expose_secret_value(), "sk-abc123");

        assert_eq!(vault.list_credentials().unwrap().len(), 2);
    }
}

/// Summary of a credential (no secrets exposed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialSummary {
    pub id: String,
    pub domain: String,
    pub kind: String,
    pub identity: String,
    pub label: Option<String>,
    pub url_pattern: Option<String>,
    pub auto_use: bool,
    pub created_at: String,
    pub last_used: Option<String>,
    pub use_count: i64,
}
