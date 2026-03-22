//! Webhook notification system for swarm events.
//!
//! Sends event payloads to configured HTTP endpoints (Slack, Discord, or generic)
//! with optional HMAC-SHA256 signature verification.

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, error, warn};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Output format for a webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebhookFormat {
    Slack,
    Discord,
    Generic,
}

impl Default for WebhookFormat {
    fn default() -> Self {
        Self::Generic
    }
}

/// A configured webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// The URL to POST events to.
    pub url: String,
    /// Only deliver events whose `event_type` appears in this list.
    /// An empty vec means "deliver everything".
    pub events: Vec<String>,
    /// Optional shared secret for HMAC-SHA256 signing.
    pub secret: Option<String>,
    /// Payload format.
    #[serde(default)]
    pub format: WebhookFormat,
}

/// A single webhook event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

/// Sends webhook notifications for swarm lifecycle events.
pub struct WebhookNotifier {
    endpoints: Vec<WebhookEndpoint>,
    client: Client,
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 (RFC 2104) — implemented with `sha2` only, no extra crate.
// ---------------------------------------------------------------------------

fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;

    // Step 1 — normalise key to exactly BLOCK_SIZE bytes.
    let normalised = if key.len() > BLOCK_SIZE {
        let mut h = Sha256::new();
        h.update(key);
        h.finalize().to_vec()
    } else {
        key.to_vec()
    };

    let mut padded = vec![0u8; BLOCK_SIZE];
    padded[..normalised.len()].copy_from_slice(&normalised);

    // Step 2 — inner / outer pads.
    let mut i_key_pad = vec![0x36u8; BLOCK_SIZE];
    let mut o_key_pad = vec![0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        i_key_pad[i] ^= padded[i];
        o_key_pad[i] ^= padded[i];
    }

    // Step 3 — HMAC = H(o_key_pad || H(i_key_pad || message))
    let mut inner = Sha256::new();
    inner.update(&i_key_pad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&o_key_pad);
    outer.update(inner_hash);
    outer.finalize().to_vec()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl WebhookNotifier {
    /// Create a notifier from an explicit list of endpoints.
    pub fn new(endpoints: Vec<WebhookEndpoint>) -> Self {
        Self {
            endpoints,
            client: Client::new(),
        }
    }

    /// Create a notifier from environment variables.
    ///
    /// Reads:
    /// - `WRAITH_WEBHOOK_URL`    — required, the endpoint URL
    /// - `WRAITH_WEBHOOK_SECRET` — optional HMAC secret
    /// - `WRAITH_WEBHOOK_FORMAT` — optional, one of `slack`, `discord`, `generic` (default)
    pub fn from_env() -> Self {
        let url = std::env::var("WRAITH_WEBHOOK_URL").unwrap_or_default();
        if url.is_empty() {
            return Self::new(Vec::new());
        }

        let secret = std::env::var("WRAITH_WEBHOOK_SECRET").ok().filter(|s| !s.is_empty());
        let format = match std::env::var("WRAITH_WEBHOOK_FORMAT")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "slack" => WebhookFormat::Slack,
            "discord" => WebhookFormat::Discord,
            _ => WebhookFormat::Generic,
        };

        Self::new(vec![WebhookEndpoint {
            url,
            events: Vec::new(), // subscribe to all
            secret,
            format,
        }])
    }

    /// Fire-and-forget: send the event to every matching endpoint.
    ///
    /// Each delivery is spawned as a separate Tokio task so the caller never
    /// blocks waiting for HTTP round-trips.
    pub fn notify(&self, event: WebhookEvent) {
        for ep in &self.endpoints {
            // Filter: skip if the endpoint subscribes to a specific set and
            // this event is not in it.
            if !ep.events.is_empty() && !ep.events.contains(&event.event_type) {
                continue;
            }

            let client = self.client.clone();
            let ep = ep.clone();
            let event = event.clone();

            tokio::spawn(async move {
                let result = deliver(&client, &ep, &event).await;
                match result {
                    Ok(status) => {
                        debug!(url = %ep.url, %status, event = %event.event_type, "webhook delivered");
                    }
                    Err(e) => {
                        error!(url = %ep.url, event = %event.event_type, error = %e, "webhook delivery failed");
                    }
                }
            });
        }
    }

    /// Format and send an event to a Slack-compatible endpoint.
    pub async fn notify_slack(
        &self,
        event: &WebhookEvent,
        url: &str,
    ) -> Result<(), anyhow::Error> {
        let body = format_slack(event);
        self.post(url, &body, None).await
    }

    /// Format and send an event to a Discord-compatible endpoint.
    pub async fn notify_discord(
        &self,
        event: &WebhookEvent,
        url: &str,
    ) -> Result<(), anyhow::Error> {
        let body = format_discord(event);
        self.post(url, &body, None).await
    }

    // -- private helpers ----------------------------------------------------

    async fn post(
        &self,
        url: &str,
        body: &serde_json::Value,
        secret: Option<&str>,
    ) -> Result<(), anyhow::Error> {
        let raw = serde_json::to_vec(body)?;

        let mut req = self
            .client
            .post(url)
            .header("Content-Type", "application/json");

        if let Some(secret) = secret {
            let sig = hmac_sha256(secret.as_bytes(), &raw);
            req = req.header("X-Wraith-Signature", format!("sha256={}", hex_encode(&sig)));
        }

        let resp = req.body(raw).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("webhook returned {status}: {text}");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-endpoint delivery (called inside a spawned task)
// ---------------------------------------------------------------------------

async fn deliver(
    client: &Client,
    ep: &WebhookEndpoint,
    event: &WebhookEvent,
) -> Result<u16, anyhow::Error> {
    let body = match ep.format {
        WebhookFormat::Slack => format_slack(event),
        WebhookFormat::Discord => format_discord(event),
        WebhookFormat::Generic => format_generic(event),
    };

    let raw = serde_json::to_vec(&body)?;

    let mut req = client
        .post(&ep.url)
        .header("Content-Type", "application/json");

    if let Some(ref secret) = ep.secret {
        let sig = hmac_sha256(secret.as_bytes(), &raw);
        req = req.header("X-Wraith-Signature", format!("sha256={}", hex_encode(&sig)));
    }

    let resp = req.body(raw).send().await?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("webhook returned {status}: {text}");
    }
    Ok(status)
}

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

fn summary_line(event: &WebhookEvent) -> String {
    match event.event_type.as_str() {
        "swarm_started" => {
            let total = event.data.get("total_jobs").and_then(|v| v.as_u64()).unwrap_or(0);
            let playbook = event.data.get("playbook").and_then(|v| v.as_str()).unwrap_or("unknown");
            format!("Swarm started — {total} jobs, playbook: {playbook}")
        }
        "application_submitted" => {
            let company = event.data.get("company").and_then(|v| v.as_str()).unwrap_or("?");
            let title = event.data.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let platform = event.data.get("platform").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Application submitted — {company} / {title} ({platform})")
        }
        "application_failed" => {
            let company = event.data.get("company").and_then(|v| v.as_str()).unwrap_or("?");
            let err = event.data.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
            let will_retry = event.data.get("will_retry").and_then(|v| v.as_bool()).unwrap_or(false);
            let retry_note = if will_retry { " (will retry)" } else { "" };
            format!("Application failed — {company}: {err}{retry_note}")
        }
        "swarm_completed" => {
            let total = event.data.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
            let ok = event.data.get("succeeded").and_then(|v| v.as_u64()).unwrap_or(0);
            let fail = event.data.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
            let dur = event.data.get("duration_secs").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Swarm completed — {ok}/{total} succeeded, {fail} failed ({dur}s)")
        }
        "rate_limited" => {
            let domain = event.data.get("domain").and_then(|v| v.as_str()).unwrap_or("?");
            let wait = event.data.get("wait_secs").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Rate limited on {domain} — waiting {wait}s")
        }
        "error" => {
            let msg = event.data.get("message").and_then(|v| v.as_str()).unwrap_or("unknown");
            format!("Error — {msg}")
        }
        other => format!("Event: {other}"),
    }
}

fn emoji_for(event_type: &str) -> &'static str {
    match event_type {
        "swarm_started" => ":rocket:",
        "application_submitted" => ":white_check_mark:",
        "application_failed" => ":x:",
        "swarm_completed" => ":checkered_flag:",
        "rate_limited" => ":hourglass_flowing_sand:",
        "error" => ":warning:",
        _ => ":bell:",
    }
}

/// Format payload for Slack incoming webhook.
fn format_slack(event: &WebhookEvent) -> serde_json::Value {
    let emoji = emoji_for(&event.event_type);
    let text = summary_line(event);
    serde_json::json!({
        "text": format!("{emoji} {text}"),
        "blocks": [
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!("{emoji} *{}*\n{}", event.event_type, text),
                }
            },
            {
                "type": "context",
                "elements": [
                    {
                        "type": "mrkdwn",
                        "text": format!("ts: {}", event.timestamp.to_rfc3339()),
                    }
                ]
            }
        ]
    })
}

/// Format payload for Discord webhook.
fn format_discord(event: &WebhookEvent) -> serde_json::Value {
    let text = summary_line(event);
    let color = match event.event_type.as_str() {
        "application_submitted" => 0x2ecc71, // green
        "application_failed" | "error" => 0xe74c3c, // red
        "swarm_completed" => 0x3498db, // blue
        "rate_limited" => 0xf39c12, // orange
        _ => 0x95a5a6, // grey
    };

    serde_json::json!({
        "content": text,
        "embeds": [
            {
                "title": event.event_type,
                "description": text,
                "color": color,
                "timestamp": event.timestamp.to_rfc3339(),
                "fields": data_to_embed_fields(&event.data),
            }
        ]
    })
}

/// Format a generic JSON payload.
fn format_generic(event: &WebhookEvent) -> serde_json::Value {
    serde_json::json!({
        "event": event.event_type,
        "data": event.data,
        "timestamp": event.timestamp.to_rfc3339(),
    })
}

/// Convert a flat JSON object into Discord embed field objects.
fn data_to_embed_fields(data: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(obj) = data.as_object() else {
        return Vec::new();
    };
    obj.iter()
        .map(|(k, v)| {
            let display = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            serde_json::json!({
                "name": k,
                "value": display,
                "inline": true,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_rfc_vector() {
        // RFC 4231 Test Case 2
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        let result = hex_encode(&hmac_sha256(key, data));
        assert_eq!(result, expected);
    }

    #[test]
    fn summary_swarm_started() {
        let event = WebhookEvent {
            event_type: "swarm_started".into(),
            timestamp: Utc::now(),
            data: serde_json::json!({ "run_id": "abc", "total_jobs": 42, "playbook": "default" }),
        };
        let line = summary_line(&event);
        assert!(line.contains("42"));
        assert!(line.contains("default"));
    }

    #[test]
    fn generic_format_roundtrip() {
        let event = WebhookEvent {
            event_type: "swarm_completed".into(),
            timestamp: Utc::now(),
            data: serde_json::json!({
                "run_id": "r1",
                "total": 10,
                "succeeded": 8,
                "failed": 2,
                "duration_secs": 300,
            }),
        };
        let body = format_generic(&event);
        assert_eq!(body["event"], "swarm_completed");
        assert_eq!(body["data"]["total"], 10);
    }

    #[test]
    fn slack_format_has_blocks() {
        let event = WebhookEvent {
            event_type: "application_submitted".into(),
            timestamp: Utc::now(),
            data: serde_json::json!({
                "job_url": "https://example.com/job/1",
                "company": "Acme",
                "title": "Engineer",
                "platform": "linkedin",
                "status": "submitted",
                "worker_id": "w1",
            }),
        };
        let body = format_slack(&event);
        assert!(body.get("text").is_some());
        assert!(body.get("blocks").is_some());
    }

    #[test]
    fn discord_format_has_embeds() {
        let event = WebhookEvent {
            event_type: "rate_limited".into(),
            timestamp: Utc::now(),
            data: serde_json::json!({
                "domain": "linkedin.com",
                "wait_secs": 60,
                "requests_this_hour": 100,
            }),
        };
        let body = format_discord(&event);
        assert!(body.get("embeds").is_some());
        let embed = &body["embeds"][0];
        assert_eq!(embed["title"], "rate_limited");
    }

    #[test]
    fn endpoint_event_filter() {
        let ep = WebhookEndpoint {
            url: "https://example.com/hook".into(),
            events: vec!["swarm_started".into(), "swarm_completed".into()],
            secret: None,
            format: WebhookFormat::Generic,
        };
        // Should match
        assert!(ep.events.contains(&"swarm_started".to_string()));
        // Should not match
        assert!(!ep.events.contains(&"application_submitted".to_string()));
    }

    #[test]
    fn from_env_empty_url() {
        // With no env var set, should produce an empty notifier.
        std::env::remove_var("WRAITH_WEBHOOK_URL");
        let notifier = WebhookNotifier::from_env();
        assert!(notifier.endpoints.is_empty());
    }
}
