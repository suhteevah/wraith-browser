//! # openclaw-identity
//!
//! Secure credential vault, browser fingerprint management, and authentication
//! flow handling for OpenClaw Browser. This crate is the reason the AI browser
//! can actually log into sites, pass bot detection, and behave like a real user.
//!
//! ## Security Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Identity Manager                          │
//! │                                                                │
//! │  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐   │
//! │  │ Vault        │  │ Fingerprint  │  │ Auth Flows         │   │
//! │  │ (AES-256-GCM)│  │ Manager      │  │                    │   │
//! │  │              │  │              │  │ • Password login    │   │
//! │  │ • passwords  │  │ • user agent │  │ • OAuth/SSO        │   │
//! │  │ • api keys   │  │ • screen res │  │ • TOTP 2FA         │   │
//! │  │ • oauth toks │  │ • timezone   │  │ • Passkey/WebAuthn │   │
//! │  │ • totp seeds │  │ • language   │  │ • Email magic link │   │
//! │  │ • cookies    │  │ • webgl hash │  │ • SMS code (human) │   │
//! │  │ • passkeys   │  │ • fonts      │  │                    │   │
//! │  │              │  │ • canvas fp  │  │                    │   │
//! │  └──────┬───────┘  │ • plugins    │  └────────┬───────────┘   │
//! │         │          │ • headers    │           │               │
//! │         │          └──────┬───────┘           │               │
//! │         │                 │                   │               │
//! │    master key        CDP inject          human-in-loop       │
//! │    (argon2id)        on every             callback for       │
//! │    from user         navigation           CAPTCHAs, SMS,     │
//! │    passphrase                             passkey taps       │
//! │                                                                │
//! │  Encrypted at rest: ~/.openclaw/vault.db (SQLite + AES-GCM)   │
//! │  Never in .env: credentials live ONLY in the encrypted vault   │
//! │  Zero plaintext: secrets zeroized from memory after use        │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Why NOT .env Files
//!
//! .env files are plaintext on disk. One `cat .env` and every password leaks.
//! Instead, OpenClaw uses an encrypted SQLite vault:
//!
//! - Master passphrase → argon2id → 256-bit key
//! - Each credential encrypted individually with AES-256-GCM
//! - Key never written to disk, derived at runtime
//! - Secrets wrapped in `secrecy::SecretString` (zeroized on drop)
//! - Vault file has 0600 permissions (owner-only read/write)
//!
//! The ONLY thing in the environment is the vault path and optional
//! master passphrase (for headless/CI use). In interactive mode,
//! the agent asks the human for the passphrase at startup.

pub mod vault;
pub mod fingerprint;
pub mod auth;
pub mod credential;
pub mod profiles;
pub mod human_loop;
pub mod error;

pub use vault::CredentialVault;
pub use fingerprint::{BrowserFingerprint, FingerprintManager};
pub use auth::{AuthFlow, AuthResult};
pub use credential::{Credential, CredentialKind};
pub use profiles::IdentityProfile;
pub use human_loop::HumanCallback;
pub use error::IdentityError;
