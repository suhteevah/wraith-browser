use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Vault is locked — passphrase required")]
    VaultLocked,

    #[error("Vault unlock failed — wrong passphrase")]
    WrongPassphrase,

    #[error("Vault not found at {path}")]
    VaultNotFound { path: String },

    #[error("Credential not found: {domain}/{kind:?}")]
    CredentialNotFound { domain: String, kind: String },

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Fingerprint capture failed: {0}")]
    FingerprintFailed(String),

    #[error("Auth flow failed for {domain}: {reason}")]
    AuthFailed { domain: String, reason: String },

    #[error("Human interaction required: {reason}")]
    HumanRequired { reason: String },

    #[error("TOTP generation failed: {0}")]
    TotpFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<rusqlite::Error> for IdentityError {
    fn from(e: rusqlite::Error) -> Self {
        IdentityError::DatabaseError(e.to_string())
    }
}

pub type IdentityResult<T> = Result<T, IdentityError>;
