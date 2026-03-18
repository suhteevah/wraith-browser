//! HTML noise stripping — removes scripts, styles, ads, trackers before extraction.
//! Uses lol_html for streaming performance.

use crate::error::ExtractError;
use lol_html::{element, rewrite_str, RewriteStrSettings};

/// Tag names to remove entirely (element + content).
const STRIP_TAGS: &[&str] = &[
    "script",
    "style",
    "noscript",
    "iframe",
    "object",
    "embed",
    "applet",
    "svg",
    // Tracking / ad containers
    "ins", // Google AdSense
];

/// CSS class/id patterns that strongly indicate non-content (ads, nav, footer, etc.)
const NOISE_PATTERNS: &[&str] = &[
    "sidebar",
    "footer",
    "nav",
    "menu",
    "breadcrumb",
    "advertisement",
    "ad-",
    "ads-",
    "adsbygoogle",
    "social-share",
    "share-buttons",
    "cookie-banner",
    "cookie-consent",
    "popup",
    "modal",
    "newsletter",
    "subscribe",
    "related-posts",
    "recommended",
    "comments",
    "comment-",
    "disqus",
];

/// Returns true if the class or id attribute matches known noise patterns.
fn is_noise_element(class: Option<&str>, id: Option<&str>) -> bool {
    let combined = format!(
        "{} {}",
        class.unwrap_or(""),
        id.unwrap_or("")
    )
    .to_lowercase();

    NOISE_PATTERNS.iter().any(|p| combined.contains(p))
}

/// Strip scripts, styles, iframes, and known ad/tracker patterns from HTML.
///
/// Uses `lol_html` streaming rewriter for performance — handles multi-MB pages
/// without building a full DOM tree.
pub fn strip_noise(html: &str) -> Result<String, ExtractError> {
    tracing::debug!(html_len = html.len(), "Stripping noise from HTML");

    // Build element content handlers for each tag we want to strip
    let handlers: Vec<_> = STRIP_TAGS
        .iter()
        .map(|tag| {
            element!(tag, |el| {
                el.remove();
                Ok(())
            })
        })
        .collect();

    // First pass: strip known bad tags
    let pass1 = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: handlers,
            ..RewriteStrSettings::default()
        },
    )
    .map_err(|e| ExtractError::ParseFailed(format!("lol_html strip pass: {e}")))?;

    // Second pass: strip noise elements by class/id patterns
    let pass2 = rewrite_str(
        &pass1,
        RewriteStrSettings {
            element_content_handlers: vec![
                // Remove hidden elements
                element!("[style]", |el| {
                    if let Some(style) = el.get_attribute("style") {
                        let s = style.to_lowercase();
                        if s.contains("display:none")
                            || s.contains("display: none")
                            || s.contains("visibility:hidden")
                            || s.contains("visibility: hidden")
                        {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                // Remove aria-hidden elements
                element!("[aria-hidden=\"true\"]", |el| {
                    el.remove();
                    Ok(())
                }),
                // Remove elements with noise class/id patterns
                element!("div, section, aside, footer, nav, header", |el| {
                    let class = el.get_attribute("class");
                    let id = el.get_attribute("id");
                    if is_noise_element(class.as_deref(), id.as_deref()) {
                        el.remove();
                    }
                    Ok(())
                }),
                // Strip tracking pixels (1x1 images)
                element!("img", |el| {
                    if let (Some(w), Some(h)) =
                        (el.get_attribute("width"), el.get_attribute("height"))
                    {
                        if (w == "1" || w == "0") && (h == "1" || h == "0") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                // Strip link[rel=preload/prefetch/dns-prefetch]
                element!("link", |el| {
                    if let Some(rel) = el.get_attribute("rel") {
                        let r = rel.to_lowercase();
                        if r.contains("preload")
                            || r.contains("prefetch")
                            || r.contains("dns-prefetch")
                            || r.contains("stylesheet")
                        {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                // Strip meta tags (except description/og)
                element!("meta", |el| {
                    // Keep description and og: tags, remove the rest
                    let dominated_by_name = el
                        .get_attribute("name")
                        .map(|n| {
                            let nl = n.to_lowercase();
                            nl == "description" || nl.starts_with("og:")
                        })
                        .unwrap_or(false);
                    let dominated_by_property = el
                        .get_attribute("property")
                        .map(|p| p.to_lowercase().starts_with("og:"))
                        .unwrap_or(false);
                    if !dominated_by_name && !dominated_by_property {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            ..RewriteStrSettings::default()
        },
    )
    .map_err(|e| ExtractError::ParseFailed(format!("lol_html noise pass: {e}")))?;

    tracing::debug!(
        original = html.len(),
        cleaned = pass2.len(),
        "Noise stripping complete"
    );

    Ok(pass2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_tags() {
        let html = r#"<html><body><p>Hello</p><script>alert('xss')</script></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(!result.contains("script"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn strips_style_tags() {
        let html = r#"<html><body><style>.x{color:red}</style><p>Content</p></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(!result.contains("style"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn strips_hidden_elements() {
        let html =
            r#"<html><body><div style="display:none">Hidden</div><p>Visible</p></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(!result.contains("Hidden"));
        assert!(result.contains("Visible"));
    }

    #[test]
    fn strips_ad_containers() {
        let html = r#"<html><body><div class="advertisement-banner">Buy now!</div><p>Article</p></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(!result.contains("Buy now"));
        assert!(result.contains("Article"));
    }

    #[test]
    fn strips_tracking_pixels() {
        let html =
            r#"<html><body><img width="1" height="1" src="track.gif"/><p>Text</p></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(!result.contains("track.gif"));
        assert!(result.contains("Text"));
    }

    #[test]
    fn preserves_content() {
        let html = r#"<html><body><article><h1>Title</h1><p>Paragraph one.</p><p>Paragraph two.</p></article></body></html>"#;
        let result = strip_noise(html).unwrap();
        assert!(result.contains("Title"));
        assert!(result.contains("Paragraph one"));
        assert!(result.contains("Paragraph two"));
    }
}
