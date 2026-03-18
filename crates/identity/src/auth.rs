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

        let html_lower = page_html.to_lowercase();

        // 1. Check for CAPTCHA (highest priority — blocks everything else)
        if html_lower.contains("recaptcha") || html_lower.contains("hcaptcha")
            || html_lower.contains("cf-turnstile") || html_lower.contains("captcha")
        {
            info!(url = %url, "Detected CAPTCHA challenge");
            // CAPTCHA pages often also have a login form behind them,
            // but the CAPTCHA must be solved first by a human
        }

        // 2. Check for OAuth/SSO buttons
        let oauth_providers = [
            ("google", "accounts.google.com"),
            ("github", "github.com/login/oauth"),
            ("microsoft", "login.microsoftonline.com"),
            ("apple", "appleid.apple.com"),
            ("facebook", "facebook.com/v"),
        ];
        for (provider, domain) in &oauth_providers {
            if html_lower.contains(&format!("sign in with {provider}"))
                || html_lower.contains(&format!("log in with {provider}"))
                || html_lower.contains(&format!("continue with {provider}"))
                || html_lower.contains(domain)
            {
                info!(url = %url, provider = %provider, "Detected OAuth flow");
                return Some(AuthFlow::OAuth2 {
                    auth_url: url.to_string(),
                    redirect_uri: String::new(), // Determined at runtime from the OAuth link
                    client_id: String::new(),
                    scopes: vec!["openid".to_string(), "profile".to_string(), "email".to_string()],
                });
            }
        }

        // 3. Check for SSO redirect patterns (e.g., Okta, Auth0, Azure AD)
        let sso_indicators = ["saml", "sso", "idp", "adfs", "okta.com", "auth0.com"];
        if sso_indicators.iter().any(|ind| html_lower.contains(ind)) {
            info!(url = %url, "Detected SSO redirect");
            return Some(AuthFlow::SsoRedirect {
                entry_url: url.to_string(),
                expected_redirect_domain: url::Url::parse(url)
                    .map(|u| u.host_str().unwrap_or("").to_string())
                    .unwrap_or_default(),
            });
        }

        // 4. Check for TOTP / 2FA input
        if html_lower.contains("totp") || html_lower.contains("verification code")
            || html_lower.contains("authenticator") || html_lower.contains("6-digit")
            || html_lower.contains("two-factor") || html_lower.contains("2fa")
        {
            // Distinguish SMS/email code from TOTP
            if html_lower.contains("sms") || html_lower.contains("text message")
                || html_lower.contains("sent a code") || html_lower.contains("check your email")
            {
                info!(url = %url, "Detected SMS/email verification code form");
                // This requires human — agent can't read SMS
                return None; // Let the caller handle via HumanCallback
            }

            info!(url = %url, "Detected TOTP 2FA form");
            return Some(AuthFlow::Totp2FA {
                code_selector: "input[name=totp], input[name=code], input[autocomplete=one-time-code], input[type=tel]".to_string(),
                submit_selector: "button[type=submit]".to_string(),
            });
        }

        // 5. Check for password fields (most common)
        if html_lower.contains("type=\"password\"") || html_lower.contains("type='password'") {
            // Detect multi-step: some sites show username first, then password on next page
            let has_username_field = html_lower.contains("type=\"email\"")
                || html_lower.contains("type='email'")
                || html_lower.contains("name=\"username\"")
                || html_lower.contains("name=\"email\"")
                || html_lower.contains("name='username'");

            if has_username_field {
                info!(url = %url, "Detected password login form");
                return Some(AuthFlow::PasswordLogin {
                    username_selector: "input[type=email], input[type=text], input[name=username], input[name=email], input[name=login]".to_string(),
                    password_selector: "input[type=password]".to_string(),
                    submit_selector: "button[type=submit], input[type=submit], button:not([type])".to_string(),
                    url: url.to_string(),
                });
            } else {
                // Password field without username — likely step 2 of a multi-step flow
                info!(url = %url, "Detected password-only step (multi-step login)");
                return Some(AuthFlow::MultiStep {
                    steps: vec![
                        AuthStep {
                            action: AuthStepAction::Fill {
                                selector: "input[type=password]".to_string(),
                                credential_kind: CredentialKind::Password,
                            },
                            success_indicator: "a[href*=logout], button[aria-label*=account], .user-menu, .avatar".to_string(),
                            timeout_ms: 10_000,
                        },
                        AuthStep {
                            action: AuthStepAction::Click {
                                selector: "button[type=submit], input[type=submit], button:not([type])".to_string(),
                            },
                            success_indicator: "a[href*=logout], .dashboard, .user-menu".to_string(),
                            timeout_ms: 15_000,
                        },
                    ],
                });
            }
        }

        // 6. Check for "username only" step (multi-step: username first, password on redirect)
        if (html_lower.contains("type=\"email\"") || html_lower.contains("name=\"username\""))
            && !html_lower.contains("type=\"password\"")
            && (html_lower.contains("next") || html_lower.contains("continue") || html_lower.contains("sign in"))
        {
            info!(url = %url, "Detected username-first multi-step login");
            return Some(AuthFlow::MultiStep {
                steps: vec![
                    AuthStep {
                        action: AuthStepAction::Fill {
                            selector: "input[type=email], input[name=username], input[name=email], input[name=login]".to_string(),
                            credential_kind: CredentialKind::Password,
                        },
                        success_indicator: "input[type=password]".to_string(),
                        timeout_ms: 10_000,
                    },
                    AuthStep {
                        action: AuthStepAction::Click {
                            selector: "button[type=submit], button:contains('Next'), button:contains('Continue')".to_string(),
                        },
                        success_indicator: "input[type=password]".to_string(),
                        timeout_ms: 10_000,
                    },
                ],
            });
        }

        debug!(url = %url, "No auth flow detected");
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

            AuthFlow::OAuth2 { auth_url, .. } => {
                info!(domain = %domain, auth_url = %auth_url, "OAuth2 flow requires browser interaction");
                // OAuth2 flows need the browser to follow the redirect chain.
                // The agent should navigate to the auth_url and follow the flow.
                // If the user has saved session cookies, we can try cookie injection first.
                match self.vault.get(domain, Some(CredentialKind::SessionCookie)) {
                    Ok(cred) => {
                        info!(domain = %domain, "Found saved session cookie for OAuth domain");
                        Ok(AuthResult::Success {
                            domain: domain.to_string(),
                            session_cookies: vec![("session".to_string(), cred.identity.clone())],
                        })
                    }
                    Err(_) => {
                        // No saved cookies — the agent needs to follow the OAuth flow
                        // This typically requires human approval for the consent screen
                        Ok(AuthResult::HumanRequired {
                            reason: HumanAuthReason::ManualReview {
                                description: format!(
                                    "OAuth2 consent screen at {}. Please complete the authorization.",
                                    auth_url
                                ),
                            },
                            instructions: format!(
                                "Navigate to {} and complete the OAuth authorization. \
                                 The browser will redirect back automatically.",
                                auth_url
                            ),
                        })
                    }
                }
            }

            AuthFlow::SsoRedirect { entry_url, expected_redirect_domain } => {
                info!(
                    domain = %domain,
                    entry_url = %entry_url,
                    redirect_to = %expected_redirect_domain,
                    "SSO redirect flow"
                );
                // SSO flows: navigate to entry URL, follow redirects to IdP, auth there,
                // follow redirect back. Try saved session first.
                match self.vault.get(domain, Some(CredentialKind::SessionCookie)) {
                    Ok(cred) => {
                        info!(domain = %domain, "Found saved session for SSO domain");
                        Ok(AuthResult::Success {
                            domain: domain.to_string(),
                            session_cookies: vec![("session".to_string(), cred.identity.clone())],
                        })
                    }
                    Err(_) => {
                        // Try password credentials for the IdP
                        match self.vault.get(domain, Some(CredentialKind::Password)) {
                            Ok(cred) => {
                                info!(domain = %domain, identity = %cred.identity, "Password credential found for SSO IdP");
                                Ok(AuthResult::Success {
                                    domain: domain.to_string(),
                                    session_cookies: vec![],
                                })
                            }
                            Err(_) => {
                                Ok(AuthResult::HumanRequired {
                                    reason: HumanAuthReason::ManualReview {
                                        description: format!(
                                            "SSO login required at {}. No saved credentials found.",
                                            entry_url
                                        ),
                                    },
                                    instructions: format!(
                                        "Complete SSO login starting at {}. \
                                         Expected to redirect to {}.",
                                        entry_url, expected_redirect_domain
                                    ),
                                })
                            }
                        }
                    }
                }
            }

            AuthFlow::MultiStep { steps } => {
                info!(domain = %domain, step_count = steps.len(), "Multi-step auth flow");

                // For multi-step flows, we validate that we have the credentials
                // needed for each step, then return success so the agent loop
                // can execute each step against the browser.
                for (i, step) in steps.iter().enumerate() {
                    match &step.action {
                        AuthStepAction::Fill { credential_kind, .. } => {
                            match self.vault.get(domain, Some(*credential_kind)) {
                                Ok(cred) => {
                                    debug!(
                                        step = i,
                                        kind = ?credential_kind,
                                        identity = %cred.identity,
                                        "Credential available for multi-step"
                                    );
                                }
                                Err(_) => {
                                    return Ok(AuthResult::Failed {
                                        domain: domain.to_string(),
                                        reason: format!(
                                            "No {:?} credential found for step {}",
                                            credential_kind, i
                                        ),
                                        recoverable: true,
                                    });
                                }
                            }
                        }
                        AuthStepAction::InjectTotp { .. } => {
                            if let Err(e) = self.vault.generate_totp(domain) {
                                return Ok(AuthResult::Failed {
                                    domain: domain.to_string(),
                                    reason: format!("TOTP generation failed at step {}: {}", i, e),
                                    recoverable: true,
                                });
                            }
                        }
                        AuthStepAction::HumanAction { reason } => {
                            return Ok(AuthResult::HumanRequired {
                                reason: reason.clone(),
                                instructions: format!(
                                    "Human action required at step {} of multi-step login for {}.",
                                    i, domain
                                ),
                            });
                        }
                        AuthStepAction::Click { .. } | AuthStepAction::WaitForRedirect { .. } => {
                            // These are browser actions — no credential check needed
                        }
                    }
                }

                info!(domain = %domain, "All multi-step credentials available");
                Ok(AuthResult::Success {
                    domain: domain.to_string(),
                    session_cookies: vec![],
                })
            }
        }
    }
}
