//! Readability algorithm — extracts main article content from HTML.
//!
//! Implements a simplified Mozilla Readability algorithm:
//! 1. Parse HTML into a DOM tree via `scraper`
//! 2. Score candidate nodes (paragraphs, divs) by text density
//! 3. Pick the highest-scoring ancestor as the content container
//! 4. Extract title from `<title>`, `<h1>`, or `og:title`
//! 5. Collect links and images from the article subtree

use crate::error::ExtractError;
use scraper::{Html, Selector, ElementRef};
use url::Url;

pub struct Article {
    pub title: String,
    pub content: String,
    pub links: Vec<(String, String)>,
    pub images: Vec<(String, String)>,
    pub description: Option<String>,
    pub confidence: f32,
}

/// Tags that are positive signals for content.
const POSITIVE_TAGS: &[&str] = &["p", "article", "main", "section"];

/// Class/id patterns that boost a node's score.
const POSITIVE_PATTERNS: &[&str] = &[
    "article",
    "body",
    "content",
    "entry",
    "main",
    "page",
    "post",
    "text",
    "blog",
    "story",
];

/// Class/id patterns that penalize a node's score.
const NEGATIVE_PATTERNS: &[&str] = &[
    "combx",
    "comment",
    "contact",
    "foot",
    "footer",
    "footnote",
    "masthead",
    "media",
    "meta",
    "outbrain",
    "promo",
    "related",
    "scroll",
    "shoutbox",
    "sidebar",
    "sponsor",
    "shopping",
    "tags",
    "tool",
    "widget",
    "nav",
    "menu",
    "breadcrumb",
];

/// Compute a readability score for an element based on its class/id attributes.
fn class_weight(el: &ElementRef) -> i32 {
    let mut weight: i32 = 0;

    let class = el.value().attr("class").unwrap_or("");
    let id = el.value().attr("id").unwrap_or("");
    let combined = format!("{class} {id}").to_lowercase();

    for pat in POSITIVE_PATTERNS {
        if combined.contains(pat) {
            weight += 25;
        }
    }
    for pat in NEGATIVE_PATTERNS {
        if combined.contains(pat) {
            weight -= 25;
        }
    }

    weight
}

/// Count text characters in a node's descendants (excluding whitespace-only runs).
fn text_length(el: &ElementRef) -> usize {
    el.text().map(|t| t.trim().len()).sum()
}

/// Count commas in the text — a heuristic for "prose-like" content.
fn comma_count(el: &ElementRef) -> usize {
    el.text()
        .map(|t| t.chars().filter(|c| *c == ',').count())
        .sum()
}

/// Extract the page title from various sources.
fn extract_title(doc: &Html) -> String {
    // Try og:title first
    if let Ok(sel) = Selector::parse("meta[property='og:title']") {
        if let Some(el) = doc.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                let t = content.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }

    // Try <title>
    if let Ok(sel) = Selector::parse("title") {
        if let Some(el) = doc.select(&sel).next() {
            let t: String = el.text().collect::<String>().trim().to_string();
            // Strip site name suffix like " - Site Name" or " | Site Name"
            if let Some(idx) = t.rfind(" - ").or_else(|| t.rfind(" | ")) {
                let candidate = t[..idx].trim();
                if !candidate.is_empty() {
                    return candidate.to_string();
                }
            }
            if !t.is_empty() {
                return t;
            }
        }
    }

    // Try first <h1>
    if let Ok(sel) = Selector::parse("h1") {
        if let Some(el) = doc.select(&sel).next() {
            let t: String = el.text().collect::<String>().trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }

    "Untitled".to_string()
}

/// Extract meta description.
fn extract_description(doc: &Html) -> Option<String> {
    for selector_str in &[
        "meta[property='og:description']",
        "meta[name='description']",
    ] {
        if let Ok(sel) = Selector::parse(selector_str) {
            if let Some(el) = doc.select(&sel).next() {
                if let Some(content) = el.value().attr("content") {
                    let t = content.trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Resolve a potentially relative URL against a base URL.
fn resolve_url(href: &str, base: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("//") {
        return href.to_string();
    }
    if let Ok(base_url) = Url::parse(base) {
        if let Ok(resolved) = base_url.join(href) {
            return resolved.to_string();
        }
    }
    href.to_string()
}

/// Extract links from an HTML fragment.
fn extract_links(html: &str, base_url: &str) -> Vec<(String, String)> {
    let doc = Html::parse_fragment(html);
    let sel = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    doc.select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?;
            let text: String = el.text().collect::<String>().trim().to_string();
            if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
                return None;
            }
            Some((text, resolve_url(href, base_url)))
        })
        .collect()
}

/// Extract images from an HTML fragment.
fn extract_images(html: &str, base_url: &str) -> Vec<(String, String)> {
    let doc = Html::parse_fragment(html);
    let sel = match Selector::parse("img[src]") {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    doc.select(&sel)
        .filter_map(|el| {
            let src = el.value().attr("src")?;
            if src.is_empty() || src.starts_with("data:") {
                return None;
            }
            let alt = el.value().attr("alt").unwrap_or("").trim().to_string();
            Some((alt, resolve_url(src, base_url)))
        })
        .collect()
}

/// Extract the main article content from cleaned HTML.
///
/// Uses a simplified readability algorithm:
/// - Scores each candidate container (div, article, section, td) by text density
/// - Boosts/penalizes based on class/id attribute patterns
/// - Returns the highest-scoring candidate's inner HTML
pub fn extract_article(html: &str, url: &str) -> Result<Article, ExtractError> {
    tracing::debug!(url = %url, "Running readability extraction");

    let doc = Html::parse_document(html);
    let title = extract_title(&doc);
    let description = extract_description(&doc);

    // Try <article> or <main> first — semantic tags are strong signals
    for tag in &["article", "main", "[role='main']"] {
        if let Ok(sel) = Selector::parse(tag) {
            if let Some(el) = doc.select(&sel).next() {
                let content_html = el.inner_html();
                if text_length(&el) > 100 {
                    let links = extract_links(&content_html, url);
                    let images = extract_images(&content_html, url);
                    return Ok(Article {
                        title,
                        content: content_html,
                        links,
                        images,
                        description,
                        confidence: 0.9,
                    });
                }
            }
        }
    }

    // Score all candidate containers
    let candidate_sel = Selector::parse("div, section, td, blockquote")
        .map_err(|e| ExtractError::ParseFailed(format!("selector: {e}")))?;

    let p_sel = Selector::parse("p")
        .map_err(|e| ExtractError::ParseFailed(format!("selector: {e}")))?;

    let mut best_score: i32 = -1;
    let mut best_html: Option<String> = None;

    for el in doc.select(&candidate_sel) {
        let tlen = text_length(&el);
        if tlen < 50 {
            continue;
        }

        let mut score: i32 = 0;

        // Base score from class/id weights
        score += class_weight(&el);

        // Count paragraphs — each adds to score
        let p_count = el.select(&p_sel).count() as i32;
        score += p_count * 3;

        // Commas indicate natural prose
        score += comma_count(&el) as i32;

        // Text length bonus (diminishing returns)
        score += (tlen as f64).sqrt() as i32;

        // Positive tag bonus
        let tag_name = el.value().name();
        if POSITIVE_TAGS.contains(&tag_name) {
            score += 20;
        }

        if score > best_score {
            best_score = score;
            best_html = Some(el.inner_html());
        }
    }

    if let Some(content_html) = best_html {
        let links = extract_links(&content_html, url);
        let images = extract_images(&content_html, url);
        let confidence = (best_score as f32 / 200.0).clamp(0.1, 1.0);

        Ok(Article {
            title,
            content: content_html,
            links,
            images,
            description,
            confidence,
        })
    } else {
        // Fallback: return the body content
        let body_sel = Selector::parse("body")
            .map_err(|e| ExtractError::ParseFailed(format!("selector: {e}")))?;

        if let Some(body) = doc.select(&body_sel).next() {
            let content_html = body.inner_html();
            let links = extract_links(&content_html, url);
            let images = extract_images(&content_html, url);

            Ok(Article {
                title,
                content: content_html,
                links,
                images,
                description,
                confidence: 0.1,
            })
        } else {
            // Last resort: return cleaned input
            Ok(Article {
                title,
                content: html.to_string(),
                links: vec![],
                images: vec![],
                description,
                confidence: 0.0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_from_title_tag() {
        let html = r#"<html><head><title>My Page - Site Name</title></head><body><p>Hello</p></body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert_eq!(article.title, "My Page");
    }

    #[test]
    fn extracts_title_from_og() {
        let html = r#"<html><head><meta property="og:title" content="OG Title"/></head><body><p>Hello</p></body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert_eq!(article.title, "OG Title");
    }

    #[test]
    fn extracts_from_article_tag() {
        let html = r#"<html><body>
            <nav>Navigation here</nav>
            <article>
                <h1>Article Title</h1>
                <p>This is a long paragraph with enough text to pass the threshold. It contains several sentences about various topics to make it substantial enough for the readability algorithm.</p>
            </article>
            <footer>Footer stuff</footer>
        </body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert!(article.content.contains("long paragraph"));
        assert!(article.confidence >= 0.9);
    }

    #[test]
    fn extracts_links() {
        let html = r#"<html><body><article><p>Check out <a href="/page">this page</a> and <a href="https://other.com">other site</a>. This is enough text to pass the threshold for article extraction.</p></article></body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert_eq!(article.links.len(), 2);
        assert_eq!(article.links[0].0, "this page");
        assert!(article.links[0].1.contains("example.com/page"));
    }

    #[test]
    fn extracts_images() {
        let html = r#"<html><body><article><p>Some article text here that is long enough to pass the extraction threshold for readability.</p><img src="/photo.jpg" alt="A photo"/></article></body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert_eq!(article.images.len(), 1);
        assert_eq!(article.images[0].0, "A photo");
    }

    #[test]
    fn extracts_description() {
        let html = r#"<html><head><meta name="description" content="Page desc"/></head><body><p>Hello world</p></body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert_eq!(article.description, Some("Page desc".to_string()));
    }

    #[test]
    fn scores_content_div_higher() {
        let html = r#"<html><body>
            <div class="sidebar"><p>Links here</p></div>
            <div class="article-content">
                <p>Main article text with lots of content. This paragraph has commas, sentences, and plenty of prose to score highly in the readability algorithm.</p>
                <p>Another paragraph of content to boost the score even further with more text and detail.</p>
            </div>
        </body></html>"#;
        let article = extract_article(html, "https://example.com").unwrap();
        assert!(article.content.contains("Main article text"));
    }
}
