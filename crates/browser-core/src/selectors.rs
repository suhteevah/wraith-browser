//! # Adaptive Self-Healing Selectors
//!
//! When a CSS selector fails (element not found, site redesign, dynamic content),
//! this module cascades through alternative strategies to find the intended element:
//!
//! 1. **CSS Selector** — the original selector
//! 2. **Text Content** — find by visible text match
//! 3. **Role + Name** — find by ARIA role and accessible name
//! 4. **Attribute Fuzzy** — find by partial attribute match (data-*, aria-*, name)
//! 5. **Structural** — find by tag + position (nth-of-type heuristic)
//! 6. **LLM Fallback** — ask the agent to identify the element from snapshot
//!
//! Each strategy returns a confidence score. The highest-confidence match wins.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn, instrument};

/// A selector that can self-heal when the primary strategy fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveSelector {
    /// The original CSS selector
    pub primary: String,
    /// Optional text content hint (e.g., button label)
    pub text_hint: Option<String>,
    /// Optional ARIA role (e.g., "button", "link", "textbox")
    pub role_hint: Option<String>,
    /// Optional element tag name
    pub tag_hint: Option<String>,
    /// Attributes to match on (name → value fragment)
    pub attribute_hints: Vec<(String, String)>,
}

/// A match result from the selector cascade.
#[derive(Debug, Clone)]
pub struct SelectorMatch {
    /// The ref_id of the matched element
    pub ref_id: u32,
    /// Which strategy found the match
    pub strategy: SelectorStrategy,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
    /// Human-readable explanation
    pub explanation: String,
}

/// Which strategy resolved the selector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SelectorStrategy {
    /// Exact CSS selector match
    CssExact,
    /// Matched by visible text content
    TextContent,
    /// Matched by ARIA role + name
    RoleName,
    /// Matched by attribute similarity
    AttributeFuzzy,
    /// Matched by structural position (tag + nth)
    Structural,
    /// No automatic match — needs LLM
    LlmFallback,
}

/// An element from the DOM snapshot, simplified for matching.
pub struct MatchCandidate {
    pub ref_id: u32,
    pub tag: String,
    pub role: String,
    pub text: String,
    pub attributes: Vec<(String, String)>,
}

impl AdaptiveSelector {
    /// Create a selector from just a CSS string.
    pub fn css(selector: &str) -> Self {
        Self {
            primary: selector.to_string(),
            text_hint: None,
            role_hint: None,
            tag_hint: None,
            attribute_hints: vec![],
        }
    }

    /// Create a rich selector with all available hints.
    pub fn rich(
        selector: &str,
        text: Option<&str>,
        role: Option<&str>,
        tag: Option<&str>,
    ) -> Self {
        Self {
            primary: selector.to_string(),
            text_hint: text.map(String::from),
            role_hint: role.map(String::from),
            tag_hint: tag.map(String::from),
            attribute_hints: vec![],
        }
    }

    /// Add an attribute hint for fuzzy matching.
    pub fn with_attribute(mut self, name: &str, value_fragment: &str) -> Self {
        self.attribute_hints.push((name.to_string(), value_fragment.to_string()));
        self
    }

    /// Resolve this selector against a list of DOM elements.
    /// Cascades through strategies until a match is found.
    #[instrument(skip(self, candidates), fields(selector = %self.primary))]
    pub fn resolve(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        debug!(
            selector = %self.primary,
            candidates = candidates.len(),
            "Resolving adaptive selector"
        );

        // Strategy 1: CSS-based matching (check by tag/class/id parsed from selector)
        if let Some(m) = self.match_css(candidates) {
            info!(ref_id = m.ref_id, strategy = ?m.strategy, "Selector resolved via CSS");
            return Some(m);
        }

        // Strategy 2: Text content match
        if let Some(m) = self.match_text(candidates) {
            info!(ref_id = m.ref_id, strategy = ?m.strategy, confidence = m.confidence, "Selector resolved via text");
            return Some(m);
        }

        // Strategy 3: Role + name match
        if let Some(m) = self.match_role(candidates) {
            info!(ref_id = m.ref_id, strategy = ?m.strategy, confidence = m.confidence, "Selector resolved via role");
            return Some(m);
        }

        // Strategy 4: Attribute fuzzy match
        if let Some(m) = self.match_attributes(candidates) {
            info!(ref_id = m.ref_id, strategy = ?m.strategy, confidence = m.confidence, "Selector resolved via attributes");
            return Some(m);
        }

        // Strategy 5: Structural match (tag + position)
        if let Some(m) = self.match_structural(candidates) {
            info!(ref_id = m.ref_id, strategy = ?m.strategy, confidence = m.confidence, "Selector resolved via structure");
            return Some(m);
        }

        warn!(selector = %self.primary, "All selector strategies exhausted — LLM fallback needed");
        None
    }

    /// Strategy 1: Parse the CSS selector for tag, class, id and match directly.
    fn match_css(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        let sel = &self.primary;

        // Extract ID from selector (e.g., "#login-btn" → "login-btn")
        if let Some(id) = sel.strip_prefix('#') {
            let id_clean = id.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .next()
                .unwrap_or(id);
            for c in candidates {
                for (name, value) in &c.attributes {
                    if name == "id" && value == id_clean {
                        return Some(SelectorMatch {
                            ref_id: c.ref_id,
                            strategy: SelectorStrategy::CssExact,
                            confidence: 1.0,
                            explanation: format!("Matched #{}", id_clean),
                        });
                    }
                }
            }
        }

        // Extract class from selector (e.g., ".submit-btn")
        if let Some(class) = sel.strip_prefix('.') {
            let class_clean = class.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .next()
                .unwrap_or(class);
            for c in candidates {
                for (name, value) in &c.attributes {
                    if name == "class" && value.split_whitespace().any(|cls| cls == class_clean) {
                        return Some(SelectorMatch {
                            ref_id: c.ref_id,
                            strategy: SelectorStrategy::CssExact,
                            confidence: 0.95,
                            explanation: format!("Matched .{}", class_clean),
                        });
                    }
                }
            }
        }

        // Extract tag + attribute selector (e.g., "input[type=password]")
        if sel.contains('[') {
            if let Some((tag, rest)) = sel.split_once('[') {
                let attr_part = rest.trim_end_matches(']');
                let (attr_name, attr_val) = if let Some((n, v)) = attr_part.split_once('=') {
                    (n.trim(), v.trim().trim_matches('"').trim_matches('\''))
                } else {
                    (attr_part.trim(), "")
                };

                for c in candidates {
                    let tag_match = tag.is_empty() || c.tag.eq_ignore_ascii_case(tag);
                    if tag_match {
                        for (name, value) in &c.attributes {
                            if name == attr_name && (attr_val.is_empty() || value == attr_val) {
                                return Some(SelectorMatch {
                                    ref_id: c.ref_id,
                                    strategy: SelectorStrategy::CssExact,
                                    confidence: 0.9,
                                    explanation: format!("Matched {}[{}={}]", tag, attr_name, attr_val),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Simple tag selector (e.g., "button", "a")
        if sel.chars().all(|c| c.is_alphanumeric()) {
            // Only match if there's exactly one of this tag
            let matches: Vec<_> = candidates.iter()
                .filter(|c| c.tag.eq_ignore_ascii_case(sel))
                .collect();
            if matches.len() == 1 {
                return Some(SelectorMatch {
                    ref_id: matches[0].ref_id,
                    strategy: SelectorStrategy::CssExact,
                    confidence: 0.7,
                    explanation: format!("Matched single <{}>", sel),
                });
            }
        }

        None
    }

    /// Strategy 2: Match by visible text content.
    fn match_text(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        let hint = self.text_hint.as_ref()?;
        let hint_lower = hint.to_lowercase();

        let mut best: Option<(u32, f64)> = None;
        for c in candidates {
            let text_lower = c.text.to_lowercase();
            let score = if text_lower == hint_lower {
                1.0
            } else if text_lower.contains(&hint_lower) {
                0.8
            } else if hint_lower.contains(&text_lower) && !text_lower.is_empty() {
                0.6
            } else {
                // Word overlap
                let hint_words: Vec<&str> = hint_lower.split_whitespace().collect();
                let text_words: Vec<&str> = text_lower.split_whitespace().collect();
                let common = hint_words.iter().filter(|w| text_words.contains(w)).count();
                if common > 0 {
                    0.4 * (common as f64 / hint_words.len().max(1) as f64)
                } else {
                    0.0
                }
            };

            if score > 0.3 && best.map(|(_, s)| score > s).unwrap_or(true) {
                best = Some((c.ref_id, score));
            }
        }

        best.map(|(ref_id, confidence)| SelectorMatch {
            ref_id,
            strategy: SelectorStrategy::TextContent,
            confidence: confidence * 0.85, // text matches are slightly less reliable
            explanation: format!("Text match for '{}'", hint),
        })
    }

    /// Strategy 3: Match by ARIA role and accessible name.
    fn match_role(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        let role = self.role_hint.as_ref()?;
        let role_lower = role.to_lowercase();

        let role_matches: Vec<_> = candidates.iter()
            .filter(|c| c.role.to_lowercase() == role_lower)
            .collect();

        if role_matches.len() == 1 {
            return Some(SelectorMatch {
                ref_id: role_matches[0].ref_id,
                strategy: SelectorStrategy::RoleName,
                confidence: 0.8,
                explanation: format!("Single element with role '{}'", role),
            });
        }

        // If we also have a text hint, narrow down
        if let Some(ref text) = self.text_hint {
            let text_lower = text.to_lowercase();
            for c in &role_matches {
                if c.text.to_lowercase().contains(&text_lower) {
                    return Some(SelectorMatch {
                        ref_id: c.ref_id,
                        strategy: SelectorStrategy::RoleName,
                        confidence: 0.85,
                        explanation: format!("Role '{}' with text '{}'", role, text),
                    });
                }
            }
        }

        None
    }

    /// Strategy 4: Match by attribute similarity.
    fn match_attributes(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        if self.attribute_hints.is_empty() {
            return None;
        }

        let mut best: Option<(u32, f64, String)> = None;
        for c in candidates {
            let mut match_count = 0;
            let mut matched_attrs = Vec::new();

            for (hint_name, hint_val) in &self.attribute_hints {
                for (name, value) in &c.attributes {
                    if name == hint_name && value.contains(hint_val.as_str()) {
                        match_count += 1;
                        matched_attrs.push(hint_name.clone());
                        break;
                    }
                }
            }

            if match_count > 0 {
                let score = match_count as f64 / self.attribute_hints.len() as f64;
                if best.as_ref().map(|(_, s, _)| score > *s).unwrap_or(true) {
                    best = Some((c.ref_id, score, matched_attrs.join(", ")));
                }
            }
        }

        best.map(|(ref_id, score, attrs)| SelectorMatch {
            ref_id,
            strategy: SelectorStrategy::AttributeFuzzy,
            confidence: score * 0.7,
            explanation: format!("Matched attributes: {}", attrs),
        })
    }

    /// Strategy 5: Match by structural position.
    fn match_structural(&self, candidates: &[MatchCandidate]) -> Option<SelectorMatch> {
        let tag = self.tag_hint.as_ref()?;
        let tag_lower = tag.to_lowercase();

        let matches: Vec<_> = candidates.iter()
            .filter(|c| c.tag.to_lowercase() == tag_lower)
            .collect();

        // If there's only one element of this tag type, it's a decent match
        if matches.len() == 1 {
            return Some(SelectorMatch {
                ref_id: matches[0].ref_id,
                strategy: SelectorStrategy::Structural,
                confidence: 0.5,
                explanation: format!("Only <{}> on page", tag),
            });
        }

        // If the primary selector contains a position hint like ":first", use first match
        if self.primary.contains("first") || self.primary.contains(":nth") {
            if let Some(first) = matches.first() {
                return Some(SelectorMatch {
                    ref_id: first.ref_id,
                    strategy: SelectorStrategy::Structural,
                    confidence: 0.4,
                    explanation: format!("First <{}> on page", tag),
                });
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(ref_id: u32, tag: &str, role: &str, text: &str, attrs: Vec<(&str, &str)>) -> MatchCandidate {
        MatchCandidate {
            ref_id,
            tag: tag.to_string(),
            role: role.to_string(),
            text: text.to_string(),
            attributes: attrs.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn css_id_match() {
        let sel = AdaptiveSelector::css("#login-btn");
        let candidates = vec![
            make_candidate(1, "button", "button", "Login", vec![("id", "login-btn")]),
            make_candidate(2, "button", "button", "Register", vec![("id", "register-btn")]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 1);
        assert_eq!(m.strategy, SelectorStrategy::CssExact);
        assert_eq!(m.confidence, 1.0);
    }

    #[test]
    fn css_class_match() {
        let sel = AdaptiveSelector::css(".submit-btn");
        let candidates = vec![
            make_candidate(1, "button", "button", "Cancel", vec![("class", "cancel-btn")]),
            make_candidate(2, "button", "button", "Submit", vec![("class", "submit-btn primary")]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
    }

    #[test]
    fn css_attribute_match() {
        let sel = AdaptiveSelector::css("input[type=password]");
        let candidates = vec![
            make_candidate(1, "input", "textbox", "", vec![("type", "text")]),
            make_candidate(2, "input", "textbox", "", vec![("type", "password")]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
    }

    #[test]
    fn text_fallback() {
        let sel = AdaptiveSelector::rich("#nonexistent", Some("Sign In"), None, None);
        let candidates = vec![
            make_candidate(1, "button", "button", "Sign Up", vec![]),
            make_candidate(2, "button", "button", "Sign In", vec![]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
        assert_eq!(m.strategy, SelectorStrategy::TextContent);
    }

    #[test]
    fn role_fallback() {
        let sel = AdaptiveSelector::rich("#gone", None, Some("textbox"), None);
        let candidates = vec![
            make_candidate(1, "button", "button", "Click", vec![]),
            make_candidate(2, "input", "textbox", "", vec![]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
        assert_eq!(m.strategy, SelectorStrategy::RoleName);
    }

    #[test]
    fn attribute_fallback() {
        let sel = AdaptiveSelector::css("#missing")
            .with_attribute("name", "email");
        let candidates = vec![
            make_candidate(1, "input", "textbox", "", vec![("name", "username")]),
            make_candidate(2, "input", "textbox", "", vec![("name", "email")]),
        ];
        // CSS fails, text fails, role fails (two textboxes), attribute matches
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
        assert_eq!(m.strategy, SelectorStrategy::AttributeFuzzy);
    }

    #[test]
    fn structural_fallback() {
        let sel = AdaptiveSelector {
            primary: "#missing".to_string(),
            text_hint: None,
            role_hint: None,
            tag_hint: Some("textarea".to_string()),
            attribute_hints: vec![],
        };
        let candidates = vec![
            make_candidate(1, "input", "textbox", "", vec![]),
            make_candidate(2, "textarea", "textbox", "", vec![]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 2);
        assert_eq!(m.strategy, SelectorStrategy::Structural);
    }

    #[test]
    fn no_match_returns_none() {
        let sel = AdaptiveSelector::css("#totally-gone");
        let candidates = vec![
            make_candidate(1, "div", "generic", "Hello", vec![]),
        ];
        assert!(sel.resolve(&candidates).is_none());
    }

    #[test]
    fn cascade_order() {
        // CSS should win over text even if both could match
        let sel = AdaptiveSelector::rich("#btn", Some("Click Me"), None, None);
        let candidates = vec![
            make_candidate(1, "button", "button", "Click Me", vec![("id", "btn")]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.strategy, SelectorStrategy::CssExact);
    }

    #[test]
    fn partial_text_match() {
        let sel = AdaptiveSelector::rich("#missing", Some("Submit Order"), None, None);
        let candidates = vec![
            make_candidate(1, "button", "button", "Submit Order Now", vec![]),
            make_candidate(2, "button", "button", "Cancel", vec![]),
        ];
        let m = sel.resolve(&candidates).unwrap();
        assert_eq!(m.ref_id, 1);
    }
}
