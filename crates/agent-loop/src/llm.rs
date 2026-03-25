//! LLM backend trait — pluggable interface to any language model.
//! Supports Anthropic API, Ollama (local), OpenAI-compatible, etc.

use crate::error::AgentError;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

/// A message in the agent's conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// Trait for LLM backends. Implement this to plug in any model.
pub trait LlmBackend: Send + Sync {
    /// Send a conversation and get a completion response.
    fn complete(
        &self,
        messages: &[Message],
        model: &str,
    ) -> impl std::future::Future<Output = Result<String, AgentError>> + Send;
}

// ═══════════════════════════════════════════════════════════════
// Anthropic Claude API backend
// ═══════════════════════════════════════════════════════════════

/// Anthropic Claude API backend.
pub struct ClaudeBackend {
    pub api_key: String,
    pub base_url: String,
    client: reqwest::Client,
}

impl ClaudeBackend {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

/// Anthropic API request body.
#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    system: Option<String>,
    messages: Vec<ClaudeMessage>,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

/// Anthropic API response body.
#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContentBlock>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    text: Option<String>,
}

impl LlmBackend for ClaudeBackend {
    #[instrument(skip(self, messages), fields(model = %model, message_count = messages.len()))]
    async fn complete(&self, messages: &[Message], model: &str) -> Result<String, AgentError> {
        debug!(model, messages = messages.len(), "Calling Claude API");

        // Separate system message from conversation
        let system = messages
            .iter()
            .find(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone());

        let api_messages: Vec<ClaudeMessage> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| ClaudeMessage {
                role: match m.role {
                    MessageRole::User => "user".to_string(),
                    MessageRole::Assistant => "assistant".to_string(),
                    MessageRole::System => unreachable!(),
                },
                content: m.content.clone(),
            })
            .collect();

        let body = ClaudeRequest {
            model: model.to_string(),
            max_tokens: 4096,
            system,
            messages: api_messages,
        };

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::LlmFailed(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AgentError::LlmFailed(format!(
                "Claude API error {status}: {error_text}"
            )));
        }

        let result: ClaudeResponse = response
            .json()
            .await
            .map_err(|e| AgentError::LlmFailed(format!("Response parse failed: {e}")))?;

        let text = result
            .content
            .iter()
            .filter_map(|b| b.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(AgentError::LlmFailed("Empty response from Claude".to_string()));
        }

        debug!(response_len = text.len(), "Claude response received");
        Ok(text)
    }
}

// ═══════════════════════════════════════════════════════════════
// Ollama backend (local inference)
// ═══════════════════════════════════════════════════════════════

/// Local Ollama backend for Wraith fleet inference.
pub struct OllamaBackend {
    pub base_url: String,
    client: reqwest::Client,
}

impl Default for OllamaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaBackend {
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

/// Ollama chat API request.
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

/// Ollama chat API response.
#[derive(Deserialize)]
struct OllamaResponse {
    message: Option<OllamaResponseMessage>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

impl LlmBackend for OllamaBackend {
    #[instrument(skip(self, messages), fields(model = %model, message_count = messages.len()))]
    async fn complete(&self, messages: &[Message], model: &str) -> Result<String, AgentError> {
        debug!(model, messages = messages.len(), "Calling Ollama API");

        let api_messages: Vec<OllamaMessage> = messages
            .iter()
            .map(|m| OllamaMessage {
                role: match m.role {
                    MessageRole::System => "system".to_string(),
                    MessageRole::User => "user".to_string(),
                    MessageRole::Assistant => "assistant".to_string(),
                },
                content: m.content.clone(),
            })
            .collect();

        let body = OllamaRequest {
            model: model.to_string(),
            messages: api_messages,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::LlmFailed(format!("Ollama request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AgentError::LlmFailed(format!(
                "Ollama error {status}: {error_text}"
            )));
        }

        let result: OllamaResponse = response
            .json()
            .await
            .map_err(|e| AgentError::LlmFailed(format!("Ollama response parse failed: {e}")))?;

        let text = result
            .message
            .map(|m| m.content)
            .unwrap_or_default();

        if text.is_empty() {
            return Err(AgentError::LlmFailed("Empty response from Ollama".to_string()));
        }

        debug!(response_len = text.len(), "Ollama response received");
        Ok(text)
    }
}
