//! # wraith-content-extract
//!
//! Extracts readable content from HTML and converts it to markdown
//! optimized for LLM context windows. Implements a pipeline:
//!
//! ```text
//! Raw HTML ──► lol_html (strip scripts/ads) ──► Readability (extract article)
//!          ──► Markdown conversion ──► Token-budgeted output
//! ```
//!
//! Inspired by dom_smoothie + fast_html2md, purpose-built for AI agents.

pub mod readability;
pub mod markdown;
pub mod strip;
pub mod error;
pub mod pdf;
pub mod ocr;

use tracing::{info, debug, instrument};

pub use error::ExtractError;

/// Extracted content ready for LLM consumption.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtractedContent {
    /// Page title
    pub title: String,

    /// Main content as markdown
    pub markdown: String,

    /// Plain text (no formatting)
    pub plain_text: String,

    /// Estimated token count
    pub estimated_tokens: usize,

    /// Links found in content: (text, url)
    pub links: Vec<(String, String)>,

    /// Images found: (alt_text, src_url)
    pub images: Vec<(String, String)>,

    /// Meta description
    pub description: Option<String>,

    /// Content extraction confidence (0.0 - 1.0)
    pub confidence: f32,
}

/// Extract readable content from raw HTML.
#[instrument(skip(html), fields(html_len = html.len()))]
pub fn extract(html: &str, url: &str) -> Result<ExtractedContent, ExtractError> {
    debug!(url = %url, html_bytes = html.len(), "Starting content extraction");

    // Step 1: Strip scripts, styles, ads, trackers
    let cleaned = strip::strip_noise(html)?;
    debug!(
        original_len = html.len(),
        cleaned_len = cleaned.len(),
        reduction_pct = format!("{:.1}%", (1.0 - cleaned.len() as f64 / html.len() as f64) * 100.0),
        "Noise stripped"
    );

    // Step 2: Extract main readable content (readability algorithm)
    let article = readability::extract_article(&cleaned, url)?;
    debug!(
        article_len = article.content.len(),
        title = %article.title,
        "Article extracted"
    );

    // Step 3: Convert to markdown
    let markdown = markdown::html_to_markdown(&article.content)?;
    let plain_text = markdown::html_to_plain_text(&article.content)?;

    let estimated_tokens = markdown.len() / 4;
    info!(
        url = %url,
        title = %article.title,
        markdown_len = markdown.len(),
        estimated_tokens,
        "Content extraction complete"
    );

    Ok(ExtractedContent {
        title: article.title,
        markdown,
        plain_text,
        estimated_tokens,
        links: article.links,
        images: article.images,
        description: article.description,
        confidence: article.confidence,
    })
}

/// Extract content with a token budget — truncates intelligently if over budget.
#[instrument(skip(html), fields(html_len = html.len(), max_tokens))]
pub fn extract_budgeted(
    html: &str,
    url: &str,
    max_tokens: usize,
) -> Result<ExtractedContent, ExtractError> {
    let mut content = extract(html, url)?;

    if content.estimated_tokens > max_tokens {
        let max_chars = max_tokens * 4;
        debug!(
            original_tokens = content.estimated_tokens,
            max_tokens,
            "Truncating content to fit token budget"
        );

        // Truncate at paragraph boundary
        if let Some(cutoff) = content.markdown[..max_chars.min(content.markdown.len())]
            .rfind("\n\n")
        {
            content.markdown.truncate(cutoff);
            content.markdown.push_str("\n\n[... content truncated to fit context window ...]");
        } else {
            content.markdown.truncate(max_chars);
        }

        content.estimated_tokens = content.markdown.len() / 4;
    }

    Ok(content)
}
