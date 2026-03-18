use serde::{Deserialize, Serialize};

/// AI-optimized representation of a web page's interactive state.
/// Inspired by AgentChrome's accessibility tree approach and browsy-core's Spatial DOM.
///
/// Instead of raw HTML (thousands of tokens), this gives agents a flat list of
/// interactive elements with semantic roles, text content, and action refs.
///
/// Example output for an agent:
/// ```text
/// Page: "GitHub - openclaw-browser" (https://github.com/suhteevah/openclaw-browser)
///
/// @e1  [link]      "Code"                    
/// @e2  [link]      "Issues (3)"              
/// @e3  [link]      "Pull requests (1)"       
/// @e4  [button]    "Star"                    
/// @e5  [button]    "Fork"                    
/// @e6  [input]     placeholder="Go to file"  
/// @e7  [link]      "README.md"               
/// @e8  [text]      "An AI-agent-first web browser written in Rust"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomSnapshot {
    /// Page URL at time of snapshot
    pub url: String,

    /// Page title
    pub title: String,

    /// Flat list of interactive and semantic elements
    pub elements: Vec<DomElement>,

    /// Page-level metadata
    pub meta: PageMeta,

    /// Timestamp of snapshot
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl DomSnapshot {
    pub fn empty() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            elements: vec![],
            meta: PageMeta::default(),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Render the snapshot as a compact text representation for LLM context.
    /// This is the primary output format — optimized for token efficiency.
    pub fn to_agent_text(&self) -> String {
        let mut out = format!(
            "Page: \"{}\" ({})\n\n",
            self.title, self.url
        );

        for el in &self.elements {
            let ref_str = format!("@e{}", el.ref_id);
            let role_str = format!("[{}]", el.role);
            let text = el.text.as_deref().unwrap_or("");
            let attrs = if let Some(placeholder) = &el.placeholder {
                format!(" placeholder=\"{}\"", placeholder)
            } else {
                String::new()
            };

            out.push_str(&format!(
                "{:<6} {:<12} \"{}\"{}\n",
                ref_str, role_str, text, attrs
            ));
        }

        if let Some(main_content) = &self.meta.main_content_preview {
            out.push_str(&format!("\n--- Main Content Preview ---\n{}\n", main_content));
        }

        out
    }

    /// Token count estimate for context window budgeting
    pub fn estimated_tokens(&self) -> usize {
        // Rough estimate: ~4 chars per token
        self.to_agent_text().len() / 4
    }
}

/// A single interactive or semantic element on the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomElement {
    /// Unique reference ID for this element (used in actions: "click @e5")
    pub ref_id: u32,

    /// Semantic role: link, button, input, select, textarea, heading, text, image, etc.
    pub role: String,

    /// Visible text content
    pub text: Option<String>,

    /// href for links
    pub href: Option<String>,

    /// Placeholder text for inputs
    pub placeholder: Option<String>,

    /// Current value for inputs/selects
    pub value: Option<String>,

    /// Whether the element is enabled/disabled
    pub enabled: bool,

    /// Whether the element is visible
    pub visible: bool,

    /// ARIA label if present
    pub aria_label: Option<String>,

    /// CSS selector path for fallback targeting
    pub selector: String,

    /// Bounding box (x, y, width, height) for spatial reasoning
    pub bounds: Option<(f64, f64, f64, f64)>,
}

/// Page-level metadata extracted during snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageMeta {
    /// Detected page type: login, search_results, article, form, dashboard, etc.
    pub page_type: Option<String>,

    /// Main content preview (first ~500 chars of readable content)
    pub main_content_preview: Option<String>,

    /// Open Graph / meta description
    pub description: Option<String>,

    /// Number of forms on the page
    pub form_count: usize,

    /// Whether a login form was detected
    pub has_login_form: bool,

    /// Whether a CAPTCHA was detected
    pub has_captcha: bool,

    /// Total interactive element count
    pub interactive_element_count: usize,
}
