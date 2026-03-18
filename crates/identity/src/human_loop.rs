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
        eprintln!("{} [OpenClaw] {}", icon, message);
        Box::pin(async {})
    }
}

/// MCP-based human callback (sends tool results back to Claude Code).
pub struct McpHumanCallback;

// TODO: Implement McpHumanCallback — sends a special MCP tool result
// that Claude Code displays as a prompt to the user
