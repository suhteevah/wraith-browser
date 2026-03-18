//! Credential types stored in the encrypted vault.
//! Every credential is encrypted individually — compromising one doesn't
//! expose the others (each has a unique nonce/IV).

use chrono::{DateTime, Utc};
use secrecy::{SecretString, ExposeSecret};
use serde::{Deserialize, Serialize};

/// A single stored credential. The `secret` field is always encrypted at rest
/// and wrapped in SecretString (zeroized on drop) in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// Unique credential ID
    pub id: String,

    /// The domain this credential is for (e.g., "github.com")
    pub domain: String,

    /// What kind of credential this is
    pub kind: CredentialKind,

    /// Username / email / account identifier (NOT encrypted — needed for lookup)
    pub identity: String,

    /// The secret value — encrypted in vault, plaintext only in memory as SecretString
    /// This is the serialized form (encrypted bytes as base64)
    #[serde(skip)]
    pub secret_encrypted: Vec<u8>,

    /// Friendly label (e.g., "Personal GitHub", "Work Slack")
    pub label: Option<String>,

    /// URL pattern this credential applies to (e.g., "https://github.com/login")
    pub url_pattern: Option<String>,

    /// When this credential was stored
    pub created_at: DateTime<Utc>,

    /// When this credential was last used by the agent
    pub last_used: Option<DateTime<Utc>>,

    /// When this credential was last rotated/updated
    pub last_rotated: Option<DateTime<Utc>>,

    /// How many times this credential has been used
    pub use_count: u64,

    /// Whether the agent should auto-use this or ask the human first
    pub auto_use: bool,

    /// Additional metadata (JSON) — e.g., OAuth scopes, TOTP digits, etc.
    pub metadata: serde_json::Value,
}

/// What kind of secret is stored.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CredentialKind {
    /// Username + password pair
    Password,

    /// API key or token (single string)
    ApiKey,

    /// OAuth2 access token (may include refresh token in metadata)
    OAuthToken,

    /// OAuth2 refresh token
    OAuthRefresh,

    /// TOTP seed (base32 secret for generating 6-digit codes)
    TotpSeed,

    /// Session cookie value
    SessionCookie,

    /// SSO / SAML token
    SsoToken,

    /// Client certificate (PEM-encoded)
    ClientCert,

    /// SSH private key (for git operations etc.)
    SshKey,

    /// Passkey / WebAuthn credential (serialized attestation)
    Passkey,

    /// Recovery code (one-time use backup codes)
    RecoveryCode,

    /// Generic secret (catch-all)
    Generic,
}

impl CredentialKind {
    /// Whether this credential type should trigger a human-in-the-loop
    /// confirmation before first use on a new domain.
    pub fn requires_human_approval(&self) -> bool {
        matches!(
            self,
            CredentialKind::Password
                | CredentialKind::SshKey
                | CredentialKind::ClientCert
                | CredentialKind::Passkey
        )
    }

    /// Whether this credential type expires and needs refresh.
    pub fn is_expirable(&self) -> bool {
        matches!(
            self,
            CredentialKind::OAuthToken
                | CredentialKind::SsoToken
                | CredentialKind::SessionCookie
        )
    }

    /// Whether this credential can be auto-generated (TOTP codes).
    pub fn is_generatable(&self) -> bool {
        matches!(self, CredentialKind::TotpSeed)
    }
}

/// A decrypted credential ready for use. The secret is exposed only here,
/// wrapped in SecretString so it gets zeroized when dropped.
///
/// NEVER log this. NEVER serialize this. Use it and drop it.
pub struct DecryptedCredential {
    pub id: String,
    pub domain: String,
    pub kind: CredentialKind,
    pub identity: String,
    pub secret: SecretString,
}

impl DecryptedCredential {
    /// Get the secret value. Intentionally verbose name to make accidental exposure obvious.
    pub fn expose_secret_value(&self) -> &str {
        self.secret.expose_secret()
    }
}

impl Drop for DecryptedCredential {
    fn drop(&mut self) {
        // SecretString handles its own zeroize, but let's be explicit
        tracing::trace!(
            domain = %self.domain,
            kind = ?self.kind,
            "Decrypted credential dropped and zeroized"
        );
    }
}

/// Request to store a new credential in the vault.
#[derive(Debug)]
pub struct StoreCredentialRequest {
    pub domain: String,
    pub kind: CredentialKind,
    pub identity: String,
    pub secret: SecretString,
    pub label: Option<String>,
    pub url_pattern: Option<String>,
    pub auto_use: bool,
    pub metadata: serde_json::Value,
}
