//! # Identity Profiles
//!
//! An IdentityProfile bundles everything needed to appear as a specific
//! user on the web: browser fingerprint, credentials, cookies, and
//! behavioral preferences.
//!
//! Use cases:
//! - **Personal**: Your real identity with your real browser fingerprint
//! - **Work**: Separate credentials/cookies for work accounts
//! - **Anonymous**: Clean fingerprint, no stored credentials, aggressive privacy
//! - **Client**: A client's profile for when you're doing work on their behalf

use serde::{Deserialize, Serialize};

/// A complete identity profile: browser characteristics for consistent automated sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityProfile {
    /// Unique profile ID
    pub id: String,

    /// Human-friendly name (e.g., "Personal", "Work", "Client: Brander Group")
    pub name: String,

    /// Which browser fingerprint to use
    pub fingerprint_id: Option<String>,

    /// Domains this profile has credentials for
    pub credential_domains: Vec<String>,

    /// Default behavior when encountering new auth challenges
    pub auth_behavior: AuthBehavior,

    /// Cookie isolation — cookies from this profile don't leak to others
    pub cookie_namespace: String,

    /// Privacy settings
    pub privacy: PrivacySettings,

    /// Whether this is the default profile
    pub is_default: bool,

    /// When this profile was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// When this profile was last used
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
}

/// How the agent handles auth challenges in this profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthBehavior {
    /// Auto-fill credentials if available, ask human for 2FA/CAPTCHA
    AutoFillAskHuman,

    /// Always ask human before using any credentials
    AlwaysAskHuman,

    /// Fully autonomous — use stored credentials, generate TOTP, skip what we can't handle
    FullyAutonomous,

    /// Never authenticate — fail on login pages
    NeverAuth,
}

/// Privacy settings per profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettings {
    /// Block third-party cookies
    pub block_third_party_cookies: bool,

    /// Block known tracker domains
    pub block_trackers: bool,

    /// Clear cookies after each session
    pub ephemeral_cookies: bool,

    /// Don't store browsing history in the knowledge cache
    pub no_cache: bool,

    /// Use a generic fingerprint instead of the captured one
    pub generic_fingerprint: bool,

    /// Blocked domains (ads, analytics, etc.)
    pub blocked_domains: Vec<String>,

    /// Whether to send Do-Not-Track header
    pub do_not_track: bool,

    /// Whether to accept cookie consent banners automatically
    pub auto_accept_cookies: bool,

    /// Whether to auto-dismiss newsletter/notification popups
    pub auto_dismiss_popups: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            block_third_party_cookies: true,
            block_trackers: true,
            ephemeral_cookies: false,
            no_cache: false,
            generic_fingerprint: false,
            blocked_domains: vec![
                "google-analytics.com".to_string(),
                "googletagmanager.com".to_string(),
                "doubleclick.net".to_string(),
                "facebook.com/tr".to_string(),
                "connect.facebook.net".to_string(),
                "hotjar.com".to_string(),
                "intercom.io".to_string(),
                "mixpanel.com".to_string(),
                "segment.io".to_string(),
                "amplitude.com".to_string(),
                "sentry.io".to_string(),
            ],
            do_not_track: false,
            auto_accept_cookies: true,
            auto_dismiss_popups: true,
        }
    }
}

impl IdentityProfile {
    /// Create a new default personal profile.
    pub fn personal(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            fingerprint_id: None,
            credential_domains: Vec::new(),
            auth_behavior: AuthBehavior::AutoFillAskHuman,
            cookie_namespace: format!("personal_{}", uuid::Uuid::new_v4().simple()),
            privacy: PrivacySettings::default(),
            is_default: true,
            created_at: chrono::Utc::now(),
            last_used: None,
        }
    }

    /// Create an anonymous/privacy-focused profile.
    pub fn anonymous() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Anonymous".to_string(),
            fingerprint_id: None,
            credential_domains: Vec::new(),
            auth_behavior: AuthBehavior::NeverAuth,
            cookie_namespace: format!("anon_{}", uuid::Uuid::new_v4().simple()),
            privacy: PrivacySettings {
                block_third_party_cookies: true,
                block_trackers: true,
                ephemeral_cookies: true,
                no_cache: true,
                generic_fingerprint: true,
                do_not_track: true,
                auto_accept_cookies: false,
                auto_dismiss_popups: true,
                ..Default::default()
            },
            is_default: false,
            created_at: chrono::Utc::now(),
            last_used: None,
        }
    }
}
