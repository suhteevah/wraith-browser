//! # Authentication Flow Handler
//!
//! Manages the multi-step authentication sequences that websites throw at you:
//! password login, OAuth redirects, TOTP 2FA, SMS verification, CAPTCHAs, etc.
//!
//! The key insight: some auth steps are fully automatable (fill password, enter
//! TOTP code), while others REQUIRE human interaction (CAPTCHA solving, passkey
//! tap, SMS code entry). The auth flow handler knows the difference and
//! calls out to the human-in-the-loop system only when necessary.

use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug, instrument};

use crate::credential::CredentialKind;
use crate::error::IdentityResult;
use crate::human_loop::HumanCallback;
use crate::vault::CredentialVault;

/// The result of an authentication attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthResult {
    /// Authentication succeeded — session is active
    Success {
        domain: String,
        session_cookies: Vec<(String, String)>,
    },

    /// Authentication needs human intervention
    HumanRequired {
        reason: HumanAuthReason,
        instructions: String,
    },

    /// Authentication failed
    Failed {
        domain: String,
        reason: String,
        recoverable: bool,
    },
}

/// Reasons the auth flow needs a human.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HumanAuthReason {
    /// CAPTCHA needs solving
    Captcha { captcha_type: String },

    /// SMS or email verification code needed
    VerificationCode { delivery_method: String },

    /// Physical passkey/security key tap needed
    PasskeyTap,

    /// Push notification approval needed (e.g., Duo, MS Authenticator)
    PushApproval { provider: String },

    /// The website wants to show something to the human
    ManualReview { description: String },

    /// Unknown 2FA method
    Unknown2FA { description: String },
}

/// Describes an authentication flow the agent should execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthFlow {
    /// Simple username + password form
    PasswordLogin {
        username_selector: String,
        password_selector: String,
        submit_selector: String,
        url: String,
    },

    /// OAuth2 redirect flow
    OAuth2 {
        auth_url: String,
        redirect_uri: String,
        client_id: String,
        scopes: Vec<String>,
    },

    /// TOTP 2FA entry (agent can handle this automatically)
    Totp2FA {
        code_selector: String,
        submit_selector: String,
    },

    /// Cookie injection (skip login entirely — inject saved session)
    CookieInjection {
        cookies: Vec<CookieToInject>,
    },

    /// SSO redirect (follow redirect chain)
    SsoRedirect {
        entry_url: String,
        expected_redirect_domain: String,
    },

    /// Multi-step login (username on page 1, password on page 2)
    MultiStep {
        steps: Vec<AuthStep>,
    },
}

/// A single step in a multi-step auth flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStep {
    /// What to do at this step
    pub action: AuthStepAction,
    /// What indicates this step is complete (CSS selector to wait for)
    pub success_indicator: String,
    /// Max wait time for this step
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthStepAction {
    Fill { selector: String, credential_kind: CredentialKind },
    Click { selector: String },
    WaitForRedirect { url_contains: String },
    InjectTotp { selector: String },
    HumanAction { reason: HumanAuthReason },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieToInject {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
}

/// Orchestrates authentication flows using the vault and browser.
pub struct AuthOrchestrator {
    vault: std::sync::Arc<CredentialVault>,
}

impl AuthOrchestrator {
    pub fn new(vault: std::sync::Arc<CredentialVault>) -> Self {
        Self { vault }
    }

    /// Detect what kind of auth flow a page is presenting.
    /// Analyzes the DOM snapshot to identify login forms, OAuth buttons, etc.
    #[instrument(skip(self, page_html), fields(url = %url))]
    pub fn detect_auth_flow(&self, url: &str, page_html: &str) -> Option<AuthFlow> {
        debug!(url = %url, "Detecting authentication flow");

        // TODO: Analyze the page for common login patterns:
        // 1. Password form: look for input[type=password]
        // 2. OAuth buttons: "Sign in with Google/GitHub/etc."
        // 3. SSO: redirect chains to identity providers
        // 4. 2FA: TOTP input, SMS code input
        // 5. CAPTCHA: reCAPTCHA, hCaptcha, Turnstile iframes

        // Heuristic detection based on common patterns
        let html_lower = page_html.to_lowercase();

        // Check for password fields
        if html_lower.contains("type=\"password\"") || html_lower.contains("type='password'") {
            info!(url = %url, "Detected password login form");
            return Some(AuthFlow::PasswordLogin {
                username_selector: "input[type=email], input[type=text], input[name=username], input[name=email]".to_string(),
                password_selector: "input[type=password]".to_string(),
                submit_selector: "button[type=submit], input[type=submit]".to_string(),
                url: url.to_string(),
            });
        }

        // Check for TOTP input
        if html_lower.contains("totp") || html_lower.contains("verification code")
            || html_lower.contains("authenticator") || html_lower.contains("6-digit")
        {
            info!(url = %url, "Detected TOTP 2FA form");
            return Some(AuthFlow::Totp2FA {
                code_selector: "input[name=totp], input[name=code], input[autocomplete=one-time-code]".to_string(),
                submit_selector: "button[type=submit]".to_string(),
            });
        }

        None
    }

    /// Execute an auth flow against the browser.
    /// Returns instructions for the agent loop to follow.
    #[instrument(skip(self, flow, _human_callback), fields(domain = %domain))]
    pub async fn execute_auth(
        &self,
        domain: &str,
        flow: &AuthFlow,
        _human_callback: &dyn HumanCallback,
    ) -> IdentityResult<AuthResult> {
        info!(domain = %domain, flow = ?std::mem::discriminant(flow), "Executing auth flow");

        match flow {
            AuthFlow::PasswordLogin { .. } => {
                // Get credentials from vault
                let cred = self.vault.get(domain, Some(CredentialKind::Password))?;
                info!(
                    domain = %domain,
                    identity = %cred.identity,
                    "Password credential retrieved for login"
                );
                // The actual form filling happens in the agent loop / browser-core
                // We just provide the credential
                Ok(AuthResult::Success {
                    domain: domain.to_string(),
                    session_cookies: vec![],
                })
            }

            AuthFlow::Totp2FA { .. } => {
                let _code = self.vault.generate_totp(domain)?;
                info!(domain = %domain, "TOTP code generated for 2FA");
                Ok(AuthResult::Success {
                    domain: domain.to_string(),
                    session_cookies: vec![],
                })
            }

            AuthFlow::CookieInjection { cookies } => {
                info!(domain = %domain, cookies = cookies.len(), "Injecting saved session cookies");
                Ok(AuthResult::Success {
                    domain: domain.to_string(),
                    session_cookies: cookies.iter().map(|c| (c.name.clone(), c.value.clone())).collect(),
                })
            }

            _ => {
                warn!(domain = %domain, "Auth flow not yet implemented");
                Ok(AuthResult::Failed {
                    domain: domain.to_string(),
                    reason: "Auth flow type not yet implemented".to_string(),
                    recoverable: false,
                })
            }
        }
    }
}
