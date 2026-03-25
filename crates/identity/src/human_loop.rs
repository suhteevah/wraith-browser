//! # Human-in-the-Loop System
//!
//! Some actions CANNOT be automated: CAPTCHAs, physical security key taps,
//! SMS codes, push notification approvals. When the agent encounters these,
//! it needs to ask a human.
//!
//! This module defines the callback interface. Implementations can be:
//! - **Terminal prompt**: Ask in the CLI (for local/Claude Code use)
//! - **MCP notification**: Send a tool result requesting human action
//! - **Desktop notification**: Pop up a system notification
//! - **Webhook/API**: Call out to a custom endpoint
//! - **Tauri dialog**: Show a native dialog window

use std::future::Future;
use std::pin::Pin;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Callback trait for requesting human interaction.
/// Implement this for your preferred notification method.
pub trait HumanCallback: Send + Sync {
    /// Request the human to perform an action.
    /// Returns the human's response (e.g., the SMS code they received).
    fn request_action(
        &self,
        request: HumanRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HumanResponse, crate::error::IdentityError>> + Send + '_>>;

    /// Notify the human of something (no response needed).
    fn notify(
        &self,
        message: &str,
        urgency: Urgency,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// A request to the human for help.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanRequest {
    /// Unique request ID
    pub id: String,

    /// What kind of action is needed
    pub kind: HumanRequestKind,

    /// Domain this is for
    pub domain: String,

    /// Human-readable instructions
    pub instructions: String,

    /// Optional screenshot (base64 PNG) showing what the agent sees
    pub screenshot_b64: Option<String>,

    /// How long to wait for the human (seconds)
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HumanRequestKind {
    /// Enter a verification code (SMS, email, etc.)
    EnterCode {
        /// How many digits/characters expected
        expected_length: Option<usize>,
    },

    /// Solve a CAPTCHA (agent shows screenshot, human provides solution)
    SolveCaptcha {
        captcha_type: String,
    },

    /// Tap a physical security key
    TapSecurityKey,

    /// Approve a push notification on another device
    ApprovePush {
        provider: String,
    },

    /// Enter the master vault passphrase
    EnterPassphrase,

    /// Approve credential use on a new domain
    ApproveCredentialUse {
        credential_domain: String,
        target_domain: String,
        credential_kind: String,
    },

    /// Generic: look at something and confirm
    Confirm {
        question: String,
    },

    /// The agent needs the human to take over the browser briefly
    TakeOverBrowser {
        reason: String,
    },
}

/// The human's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HumanResponse {
    /// Human provided text input (code, passphrase, etc.)
    Text(String),

    /// Human approved/confirmed
    Approved,

    /// Human denied/rejected
    Denied { reason: Option<String> },

    /// Human timed out (didn't respond)
    TimedOut,

    /// Human completed a browser takeover
    BrowserReturned,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Urgency {
    Low,      // Informational
    Medium,   // Needs attention soon
    High,     // Needs immediate attention
    Critical, // Auth is blocked, session will fail
}

/// Terminal-based human callback (for CLI / Claude Code use).
/// Prompts on stdin, reads response.
pub struct TerminalHumanCallback;

impl HumanCallback for TerminalHumanCallback {
    fn request_action(
        &self,
        request: HumanRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HumanResponse, crate::error::IdentityError>> + Send + '_>> {
        Box::pin(async move {
        info!(
            kind = ?request.kind,
            domain = %request.domain,
            "Human action required"
        );

        eprintln!("\n╔══════════════════════════════════════════════╗");
        eprintln!("║  🔐 HUMAN ACTION REQUIRED                    ║");
        eprintln!("╠══════════════════════════════════════════════╣");
        eprintln!("║  Domain: {:<36} ║", request.domain);
        eprintln!("║  {:<44} ║", request.instructions);
        eprintln!("╚══════════════════════════════════════════════╝");

        match request.kind {
            HumanRequestKind::EnterCode { .. } => {
                eprint!("Enter code: ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)
                    .map_err(|e| crate::error::IdentityError::HumanRequired {
                        reason: format!("Failed to read input: {}", e),
                    })?;
                Ok(HumanResponse::Text(input.trim().to_string()))
            }

            HumanRequestKind::EnterPassphrase => {
                eprint!("Enter vault passphrase: ");
                // TODO: Use rpassword crate for hidden input
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)
                    .map_err(|e| crate::error::IdentityError::HumanRequired {
                        reason: format!("Failed to read input: {}", e),
                    })?;
                Ok(HumanResponse::Text(input.trim().to_string()))
            }

            HumanRequestKind::ApproveCredentialUse { ref credential_domain, ref target_domain, ref credential_kind } => {
                eprintln!("Allow {} credential from {} on {}? [y/N]",
                    credential_kind, credential_domain, target_domain);
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)
                    .map_err(|e| crate::error::IdentityError::HumanRequired {
                        reason: format!("Failed to read input: {}", e),
                    })?;
                if input.trim().to_lowercase() == "y" {
                    Ok(HumanResponse::Approved)
                } else {
                    Ok(HumanResponse::Denied { reason: Some("User denied".to_string()) })
                }
            }

            HumanRequestKind::Confirm { ref question } => {
                eprintln!("{} [y/N]", question);
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)
                    .map_err(|e| crate::error::IdentityError::HumanRequired {
                        reason: format!("Failed to read input: {}", e),
                    })?;
                if input.trim().to_lowercase() == "y" {
                    Ok(HumanResponse::Approved)
                } else {
                    Ok(HumanResponse::Denied { reason: None })
                }
            }

            _ => {
                eprintln!("Press Enter when done...");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                Ok(HumanResponse::Approved)
            }
        }
        })
    }

    fn notify(
        &self,
        message: &str,
        urgency: Urgency,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let icon = match urgency {
            Urgency::Low => "ℹ️",
            Urgency::Medium => "⚠️",
            Urgency::High => "🔔",
            Urgency::Critical => "🚨",
        };
        eprintln!("{} [Wraith] {}", icon, message);
        Box::pin(async {})
    }
}

/// MCP-based human callback for Claude Code / Cursor integration.
///
/// When the agent needs human interaction during an MCP session, this callback
/// writes a request file to a shared directory and polls for a response file.
/// The MCP server's tool handler shows the request to the user and writes the
/// response file when the human answers.
///
/// Protocol:
/// 1. Write `~/.wraith/human_requests/{id}.request.json`
/// 2. Poll for `~/.wraith/human_requests/{id}.response.json`
/// 3. Parse response and return
pub struct McpHumanCallback {
    /// Directory for request/response exchange
    exchange_dir: std::path::PathBuf,
}

impl Default for McpHumanCallback {
    fn default() -> Self {
        Self::new()
    }
}

impl McpHumanCallback {
    pub fn new() -> Self {
        let exchange_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".wraith")
            .join("human_requests");
        let _ = std::fs::create_dir_all(&exchange_dir);
        Self { exchange_dir }
    }

    pub fn with_exchange_dir(dir: std::path::PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { exchange_dir: dir }
    }
}

impl HumanCallback for McpHumanCallback {
    fn request_action(
        &self,
        request: HumanRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HumanResponse, crate::error::IdentityError>> + Send + '_>> {
        Box::pin(async move {
            info!(
                kind = ?request.kind,
                domain = %request.domain,
                id = %request.id,
                "MCP human action required — writing exchange file"
            );

            // Write request file
            let request_path = self.exchange_dir.join(format!("{}.request.json", request.id));
            let request_json = serde_json::to_string_pretty(&request)
                .map_err(|e| crate::error::IdentityError::Internal(
                    anyhow::anyhow!("Failed to serialize request: {e}")
                ))?;
            std::fs::write(&request_path, &request_json)
                .map_err(|e| crate::error::IdentityError::Internal(
                    anyhow::anyhow!("Failed to write request file: {e}")
                ))?;

            // Poll for response file
            let response_path = self.exchange_dir.join(format!("{}.response.json", request.id));
            let timeout = std::time::Duration::from_secs(request.timeout_secs);
            let start = std::time::Instant::now();
            let poll_interval = std::time::Duration::from_millis(500);

            loop {
                if response_path.exists() {
                    let response_json = std::fs::read_to_string(&response_path)
                        .map_err(|e| crate::error::IdentityError::Internal(
                            anyhow::anyhow!("Failed to read response file: {e}")
                        ))?;

                    // Clean up exchange files
                    let _ = std::fs::remove_file(&request_path);
                    let _ = std::fs::remove_file(&response_path);

                    let response: HumanResponse = serde_json::from_str(&response_json)
                        .map_err(|e| crate::error::IdentityError::Internal(
                            anyhow::anyhow!("Failed to parse response: {e}")
                        ))?;

                    info!(id = %request.id, "MCP human response received");
                    return Ok(response);
                }

                if start.elapsed() > timeout {
                    // Clean up request file on timeout
                    let _ = std::fs::remove_file(&request_path);
                    info!(id = %request.id, timeout_secs = request.timeout_secs, "MCP human request timed out");
                    return Ok(HumanResponse::TimedOut);
                }

                tokio::time::sleep(poll_interval).await;
            }
        })
    }

    fn notify(
        &self,
        message: &str,
        urgency: Urgency,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let icon = match urgency {
            Urgency::Low => "INFO",
            Urgency::Medium => "WARN",
            Urgency::High => "ALERT",
            Urgency::Critical => "CRITICAL",
        };
        info!(urgency = %icon, message = %message, "MCP notification");

        // Write a notification file (non-blocking, no response expected)
        let notif_path = self.exchange_dir.join(format!(
            "notification_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let notif = serde_json::json!({
            "type": "notification",
            "urgency": icon,
            "message": message,
        });
        let _ = std::fs::write(&notif_path, notif.to_string());

        Box::pin(async {})
    }
}

/// No-op callback that always returns TimedOut.
/// Useful for fully autonomous operation where no human is available.
pub struct NoHumanCallback;

impl HumanCallback for NoHumanCallback {
    fn request_action(
        &self,
        request: HumanRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HumanResponse, crate::error::IdentityError>> + Send + '_>> {
        Box::pin(async move {
            info!(
                kind = ?request.kind,
                domain = %request.domain,
                "No human available — returning TimedOut"
            );
            Ok(HumanResponse::TimedOut)
        })
    }

    fn notify(
        &self,
        message: &str,
        urgency: Urgency,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let _ = (message, urgency);
        Box::pin(async {})
    }
}
