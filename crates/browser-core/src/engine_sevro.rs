//! BrowserEngine implementation wrapping the Sevro headless engine.
//!
//! Feature-gated behind `sevro`. This is the future default engine —
//! full DOM, CSS layout, and SpiderMonkey JS without Chrome.

use crate::dom::{DomSnapshot, DomElement, PageMeta};
use crate::actions::{BrowserAction, ActionResult};
use crate::engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
use crate::error::{BrowserResult, BrowserError};
use async_trait::async_trait;
use tracing::{info, warn, debug, instrument};

/// Detect the correct API submission endpoint for known ATS platforms.
/// Returns (submit_url, content_type).
fn detect_ats_submit_endpoint(page_url: &str, form_action: &str) -> (String, String) {
    // If form has an explicit action, use it
    if !form_action.is_empty() && form_action != "#" {
        if form_action.starts_with("http") {
            return (form_action.to_string(), "application/json".to_string());
        }
        if let Ok(base) = url::Url::parse(page_url) {
            if let Ok(resolved) = base.join(form_action) {
                return (resolved.to_string(), "application/json".to_string());
            }
        }
    }

    // Greenhouse: job-boards.greenhouse.io/{company}/jobs/{job_id}
    //          or job-boards.eu.greenhouse.io/{company}/jobs/{job_id}
    // API: POST same URL with /applications suffix (or embedded API)
    if page_url.contains("greenhouse.io") {
        if let Ok(parsed) = url::Url::parse(page_url) {
            let host = parsed.host_str().unwrap_or("");
            let path = parsed.path();
            // Extract company and job_id from path: /{company}/jobs/{job_id}
            let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
            if segments.len() >= 3 && segments[1] == "jobs" {
                let company = segments[0];
                let job_id = segments[2];

                // Greenhouse's actual submission endpoint
                // The board API is at boards-api.greenhouse.io
                let api_host = if host.contains(".eu.") {
                    "boards-api.eu.greenhouse.io"
                } else {
                    "boards-api.greenhouse.io"
                };

                let api_url = format!(
                    "https://{}/v1/boards/{}/jobs/{}/applications",
                    api_host, company, job_id
                );
                info!(api_url = %api_url, company = %company, job_id = %job_id, "Detected Greenhouse API endpoint");
                return (api_url, "multipart/form-data".to_string());
            }
        }
    }

    // Lever: jobs.lever.co/{company}/{job_id}/apply
    // API: POST to same URL (Lever accepts form POST on the apply page)
    if page_url.contains("lever.co") {
        let submit = if page_url.ends_with("/apply") {
            page_url.to_string()
        } else {
            format!("{}/apply", page_url.trim_end_matches('/'))
        };
        return (submit, "application/x-www-form-urlencoded".to_string());
    }

    // Ashby: jobs.ashbyhq.com/{company}/{job_id}/application/submit
    if page_url.contains("ashbyhq.com") {
        if let Ok(parsed) = url::Url::parse(page_url) {
            let path = parsed.path();
            // Find job ID and construct API URL
            let submit = format!("{}://{}{}/application/submit",
                parsed.scheme(), parsed.host_str().unwrap_or(""), path.trim_end_matches('/'));
            return (submit, "application/json".to_string());
        }
    }

    // Default: POST to the page URL
    (page_url.to_string(), "application/json".to_string())
}

/// Derive candidate Greenhouse board slugs from a career-site domain name.
///
/// E.g. "careers.datadoghq.com" → ["datadog", "datadoghq"]
///      "stripe.com"            → ["stripe"]
///      "www.samsara.com"       → ["samsara"]
///      "jobs.example-corp.io"  → ["example-corp", "examplecorp"]
fn greenhouse_slug_candidates(domain: &str) -> Vec<String> {
    // Strip common prefixes (www, careers, jobs, apply) and suffixes (.com, .io, .co, etc.)
    let domain = domain.to_lowercase();

    // Extract the "main" part: remove subdomains and TLD
    let parts: Vec<&str> = domain.split('.').collect();
    // For "careers.datadoghq.com" → parts = ["careers", "datadoghq", "com"]
    // For "stripe.com"            → parts = ["stripe", "com"]

    let skip_prefixes = ["www", "careers", "jobs", "apply", "boards", "hire"];
    let skip_suffixes = ["com", "io", "co", "org", "net", "co.uk", "jobs", "careers"];

    // Find the "core" domain part(s)
    let meaningful: Vec<&str> = parts.iter()
        .filter(|p| !skip_prefixes.contains(p) && !skip_suffixes.contains(p))
        .copied()
        .collect();

    let base = if meaningful.is_empty() {
        // Fallback: use the second-level domain
        if parts.len() >= 2 { parts[parts.len() - 2] } else { return vec![]; }
    } else {
        meaningful[0]
    };

    let mut candidates = Vec::new();

    // Common pattern: company uses "datadoghq" in domain but "datadog" as slug.
    // Strip common suffixes from the base: hq, inc, corp, io, app, labs, tech
    let slug_suffixes = ["hq", "inc", "corp", "io", "app", "labs", "tech", "dev", "eng"];
    for suffix in &slug_suffixes {
        if base.len() > suffix.len() && base.ends_with(suffix) {
            let stripped = &base[..base.len() - suffix.len()];
            if !stripped.is_empty() {
                candidates.push(stripped.to_string());
            }
        }
    }

    // The base itself is always a candidate
    candidates.push(base.to_string());

    // If base contains hyphens, also try without them
    if base.contains('-') {
        candidates.push(base.replace('-', ""));
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.clone()));

    candidates
}

/// Browser engine backed by Sevro (stripped Servo fork).
///
/// Includes an integrated Rhai scripting engine that triggers userscripts
/// on navigation events (OnNavigate, Always triggers).
pub struct SevroEngineBackend {
    engine: sevro_headless::SevroEngine,
    /// Rhai scripting engine for userscripts
    scripts: openclaw_scripting::ScriptEngine,
}

impl SevroEngineBackend {
    pub fn new() -> Self {
        Self {
            engine: sevro_headless::SevroEngine::default(),
            scripts: openclaw_scripting::ScriptEngine::new(),
        }
    }

    pub fn with_config(config: sevro_headless::SevroConfig) -> Self {
        Self {
            engine: sevro_headless::SevroEngine::new(config),
            scripts: openclaw_scripting::ScriptEngine::new(),
        }
    }

    /// Create with EngineOptions (used by MCP server for env-var config).
    pub fn new_with_options(opts: crate::engine::EngineOptions) -> Self {
        let mut config = sevro_headless::SevroConfig::default();
        config.proxy_url = opts.proxy_url;
        config.flaresolverr_url = opts.flaresolverr_url;
        config.fallback_proxy_url = opts.fallback_proxy_url;
        Self::with_config(config)
    }

    /// Access the scripting engine to load/manage Rhai scripts.
    pub fn scripting(&mut self) -> &mut openclaw_scripting::ScriptEngine {
        &mut self.scripts
    }

    /// Resolve ATS wrapper URLs to direct application URLs.
    ///
    /// - Greenhouse wrapped: URLs with `gh_jid=` query param get resolved to
    ///   `https://job-boards.greenhouse.io/{slug}/jobs/{gh_jid}` by probing the
    ///   Greenhouse boards API.
    /// - Lever: URLs containing `lever.co` that don't end with `/apply` get
    ///   `/apply` appended to reach the actual application form.
    async fn resolve_ats_url(&self, url: &str) -> String {
        // --- Lever: ensure we land on the /apply page ---
        if url.contains("lever.co") && !url.trim_end_matches('/').ends_with("/apply") {
            let resolved = format!("{}/apply", url.trim_end_matches('/'));
            info!(original = %url, resolved = %resolved, "Lever URL: appending /apply");
            return resolved;
        }

        // --- Greenhouse wrapped URLs (gh_jid= query param) ---
        if let Ok(parsed) = url::Url::parse(url) {
            let gh_jid: Option<String> = parsed.query_pairs()
                .find(|(k, _)| k == "gh_jid")
                .map(|(_, v)| v.to_string());

            if let Some(jid) = gh_jid {
                // Validate that jid is numeric
                if jid.chars().all(|c| c.is_ascii_digit()) && !jid.is_empty() {
                    let domain = parsed.host_str().unwrap_or("");
                    let candidates = greenhouse_slug_candidates(domain);
                    debug!(domain = %domain, candidates = ?candidates, gh_jid = %jid,
                           "Greenhouse wrapped URL detected, probing board API");

                    for slug in &candidates {
                        let api_url = format!(
                            "https://boards-api.greenhouse.io/v1/boards/{}/jobs/{}",
                            slug, jid
                        );
                        match self.engine.http_get(&api_url).await {
                            Ok((200, _body)) => {
                                let resolved = format!(
                                    "https://job-boards.greenhouse.io/{}/jobs/{}",
                                    slug, jid
                                );
                                info!(original = %url, resolved = %resolved, slug = %slug,
                                      "Greenhouse wrapped URL resolved via boards API");
                                return resolved;
                            }
                            Ok((status, _)) => {
                                debug!(slug = %slug, status = status, "Greenhouse API probe miss");
                            }
                            Err(e) => {
                                warn!(slug = %slug, error = %e, "Greenhouse API probe failed");
                            }
                        }
                    }
                    debug!(gh_jid = %jid, "No Greenhouse board slug matched, using original URL");
                }
            }
        }

        url.to_string()
    }

    /// Run triggered scripts for the current page.
    fn run_page_scripts(&self, url: &str, title: &str) {
        let context = openclaw_scripting::ScriptContext {
            url: url.to_string(),
            domain: url::Url::parse(url)
                .map(|u| u.host_str().unwrap_or("").to_string())
                .unwrap_or_default(),
            title: title.to_string(),
            html: self.engine.page_source().unwrap_or("").to_string(),
            text_content: String::new(),
            links: Vec::new(),
            custom_vars: std::collections::HashMap::new(),
        };

        let trigger = openclaw_scripting::ScriptTrigger::OnNavigate {
            url_pattern: url.to_string(),
        };

        let results = self.scripts.run_triggered(&trigger, &context);
        for (name, result) in &results {
            match result {
                Ok(r) if r.success => {
                    debug!(script = %name, output = ?r.output, "Script executed successfully");
                }
                Ok(_) => {
                    debug!(script = %name, "Script completed with failure status");
                }
                Err(e) => {
                    debug!(script = %name, error = %e, "Script execution error");
                }
            }
        }
        if !results.is_empty() {
            info!(count = results.len(), url = %url, "Triggered scripts executed");
        }
    }
}

impl Default for SevroEngineBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl BrowserEngine for SevroEngineBackend {
    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        let url = self.resolve_ats_url(url).await;
        let url = url.as_str();

        match self.engine.navigate(url).await {
            Ok(sevro_headless::PageEvent::Error(e)) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e,
            }),
            Ok(sevro_headless::PageEvent::Cancelled) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: "Cancelled".to_string(),
            }),
            Ok(_) => {
                // Run any Rhai scripts triggered by this URL
                let title = self.engine.current_url().unwrap_or("").to_string();
                self.run_page_scripts(url, &title);
                Ok(())
            }
            Err(e) => Err(BrowserError::NavigationFailed {
                url: url.to_string(),
                reason: e,
            }),
        }
    }

    async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        let sevro_nodes = self.engine.dom_snapshot_with_layout();

        // Query QuickJS for current input values (browse_fill sets values in JS, not Rust)
        let js_values: std::collections::HashMap<u32, String> = match self.engine.eval_js(
            r#"(() => {
                try {
                    var result = {};
                    var keys = Object.keys(__wraith_ref_index);
                    for (var i = 0; i < keys.length; i++) {
                        var ref_id = keys[i];
                        var el = __wraith_ref_index[ref_id];
                        if (!el) continue;
                        if (el.tag === 'input' || el.tag === 'textarea' || el.tag === 'select') {
                            var val = '';
                            try { val = el._value || ''; } catch(e) {}
                            if (!val) { try { val = el.value || ''; } catch(e) {} }
                            if (val) result[ref_id] = val;
                        }
                    }
                    return JSON.stringify(result);
                } catch(e) {
                    return '{"__error":"' + String(e) + '"}';
                }
            })()"#
        ).await {
            Ok(json) => {
                debug!(js_values_json = %json, "Snapshot: read JS input values");
                serde_json::from_str::<std::collections::HashMap<String, String>>(&json)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(k, v)| {
                        if k == "__error" { debug!(error = %v, "Snapshot JS error"); return None; }
                        k.parse::<u32>().ok().map(|id| (id, v))
                    })
                    .collect()
            }
            Err(e) => {
                debug!(error = %e, "Snapshot: failed to read JS input values");
                std::collections::HashMap::new()
            }
        };

        let elements: Vec<DomElement> = sevro_nodes.iter()
            .filter(|n| n.node_type == sevro_headless::DomNodeType::Element && n.is_visible)
            .enumerate()
            .map(|(i, node)| {
                let ref_id = (i + 1) as u32;
                let role = match node.tag_name.as_str() {
                    "a" => "link".to_string(),
                    "button" => "button".to_string(),
                    "input" => node.attributes.get("type")
                        .cloned()
                        .unwrap_or_else(|| "textbox".to_string()),
                    "select" => "combobox".to_string(),
                    "textarea" => "textbox".to_string(),
                    other => other.to_string(),
                };

                // Prefer JS-set value over HTML attribute value
                let value = js_values.get(&ref_id).cloned()
                    .or_else(|| node.attributes.get("value").cloned());

                DomElement {
                    ref_id,
                    role,
                    text: if node.text_content.is_empty() { None } else { Some(node.text_content.clone()) },
                    href: node.attributes.get("href").cloned(),
                    placeholder: node.attributes.get("placeholder").cloned(),
                    value,
                    enabled: true,
                    visible: node.is_visible,
                    aria_label: node.attributes.get("aria-label").cloned(),
                    selector: format!("{}", node.tag_name),
                    bounds: node.bounding_box.map(|b| (b.x, b.y, b.width, b.height)),
                }
            })
            .collect();

        let url = self.engine.current_url().unwrap_or("").to_string();
        let title = sevro_nodes.iter()
            .find(|n| n.tag_name == "title")
            .map(|n| n.text_content.clone())
            .unwrap_or_default();

        Ok(DomSnapshot {
            url,
            title,
            elements,
            meta: PageMeta {
                page_type: None,
                main_content_preview: None,
                description: None,
                form_count: 0,
                has_login_form: false,
                has_captcha: false,
                interactive_element_count: 0,
            },
            timestamp: chrono::Utc::now(),
        })
    }

    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        match action {
            BrowserAction::Navigate { url } => {
                self.navigate(&url).await?;
                Ok(ActionResult::Navigated { url, title: String::new() })
            }
            BrowserAction::Click { ref_id } => {
                // Click via JS — ref_id matches snapshot's @e numbering via __wraith_ref_index
                let js = format!(
                    r#"(() => {{
                        var el = __wraith_get_by_ref({ref_id});
                        if (!el) return 'NOT_FOUND: @e{ref_id} not in ref_index (' + Object.keys(__wraith_ref_index).length + ' refs)';
                        try {{ el.focus(); }} catch(e) {{}}
                        try {{ el.click(); }} catch(e) {{}}
                        try {{ el.dispatchEvent(new Event('click', {{ bubbles: true }})); }} catch(e) {{}}
                        var href = el.attrs ? el.attrs.href : null;
                        if (href) return 'CLICKED_LINK: ' + href;
                        return 'CLICKED: ' + (el.textContent || el.tag || '').substring(0, 50);
                    }})()"#,
                    ref_id = ref_id,
                );
                match self.engine.eval_js(&js).await {
                    Ok(result) => Ok(ActionResult::Success { message: format!("@e{}: {}", ref_id, result) }),
                    Err(_) => {
                        self.engine.click_element(ref_id as u64);
                        Ok(ActionResult::Success { message: format!("Clicked @e{} (basic)", ref_id) })
                    }
                }
            }
            BrowserAction::Fill { ref_id, text } => {
                // Set value + dispatch React-compatible events via ref_id lookup
                let text_escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                let js = format!(
                    r#"(() => {{
                        var el = __wraith_get_by_ref({ref_id});
                        if (!el) return 'NOT_FOUND: @e{ref_id} not in ref_index (' + Object.keys(__wraith_ref_index).length + ' refs)';

                        // Focus the element first
                        try {{ el.focus(); }} catch(e) {{}}

                        // Use __wraith_react_set_value which handles native setter + events + fiber
                        var result = __wraith_react_set_value(el, '{text_escaped}');

                        // Verify: read back the value to confirm it persisted
                        var readback = el.value || el._value || '';
                        var verified = (readback === '{text_escaped}');

                        return 'FILLED (' + result + (verified ? ', verified' : ', UNVERIFIED: got "' + readback + '"') + '): ' + readback;
                    }})()"#,
                    ref_id = ref_id,
                    text_escaped = text_escaped,
                );
                match self.engine.eval_js(&js).await {
                    Ok(result) => Ok(ActionResult::Success { message: format!("@e{}: {}", ref_id, result) }),
                    Err(e) => {
                        // Fallback to basic fill
                        self.engine.fill_element(ref_id as u64, &text);
                        Ok(ActionResult::Success { message: format!("Filled @e{} (basic): {}", ref_id, e) })
                    }
                }
            }
            BrowserAction::EvalJs { script } => {
                match self.engine.eval_js(&script).await {
                    Ok(result) => Ok(ActionResult::JsResult { value: result }),
                    Err(e) => Ok(ActionResult::Failed { error: e }),
                }
            }
            BrowserAction::Screenshot { .. } => {
                Err(BrowserError::ScreenshotFailed("Not available in Sevro (Phase 3)".to_string()))
            }
            BrowserAction::UploadFile { ref_id, file_name, file_data, mime_type } => {
                // Use JS to create a File object from base64 data and set it on the input
                let js = format!(
                    r#"(() => {{
                        // First try: direct ref_id lookup
                        var el = __wraith_get_by_ref({ref_id});

                        // If ref target isn't a file input, search all file inputs
                        if (el && el.attrs && el.attrs.type !== 'file') el = null;

                        // Fallback: find ANY file input (including hidden ones like Greenhouse's visually-hidden)
                        if (!el) {{
                            for (var i = 0; i < __wraith_nodes.length; i++) {{
                                var n = __wraith_nodes[i];
                                if (n.tag === 'input' && n.attrs && n.attrs.type === 'file') {{
                                    el = n;
                                    break;
                                }}
                            }}
                        }}

                        if (!el) return 'NOT_FOUND: no file input found (ref @e{ref_id}, searched ' + __wraith_nodes.length + ' nodes)';
                        try {{
                            var b64 = '{file_data}';
                            var binary = atob(b64);
                            var bytes = new Uint8Array(binary.length);
                            for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
                            var file = new File([bytes], '{file_name}', {{ type: '{mime_type}' }});
                            var dt = new DataTransfer();
                            dt.items.add(file);
                            el.files = dt.files;
                            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                            return 'OK: uploaded ' + '{file_name}' + ' (' + bytes.length + ' bytes)';
                        }} catch(e) {{
                            return 'ERROR: ' + e.message;
                        }}
                    }})()"#
                );
                match self.engine.eval_js(&js).await {
                    Ok(result) => {
                        if result.starts_with("OK:") {
                            Ok(ActionResult::Success { message: result })
                        } else {
                            Ok(ActionResult::Failed { error: result })
                        }
                    }
                    Err(e) => Ok(ActionResult::Failed { error: format!("File upload JS failed: {e}") })
                }
            }
            BrowserAction::TypeText { ref_id, text, delay_ms: _ } => {
                // Simulate character-by-character input with focus + value set + events
                let text_escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                let js = format!(r#"(() => {{
    var el = __wraith_get_by_ref({ref_id});
    if (!el) return 'NOT_FOUND: @e{ref_id}';
    try {{ el.focus(); }} catch(e) {{}}
    var text = '{text_escaped}';
    for (var i = 0; i < text.length; i++) {{
        el.value = (el.value || el._value || '') + text.charAt(i);
        if (el._value !== undefined) el._value = el.value;
        try {{ el.dispatchEvent(new Event('input', {{ bubbles: true }})); }} catch(e) {{}}
    }}
    try {{ el.dispatchEvent(new Event('change', {{ bubbles: true }})); }} catch(e) {{}}
    try {{ el.dispatchEvent(new Event('blur', {{ bubbles: true }})); }} catch(e) {{}}
    return 'TYPED: ' + (el.value || el._value || '');
}})()"#, ref_id = ref_id, text_escaped = text_escaped);
                match self.engine.eval_js(&js).await {
                    Ok(result) => Ok(ActionResult::Success { message: format!("@e{}: {}", ref_id, result) }),
                    Err(e) => {
                        self.engine.fill_element(ref_id as u64, &text);
                        Ok(ActionResult::Success { message: format!("Typed @e{} (basic): {}", ref_id, e) })
                    }
                }
            }
            BrowserAction::Select { ref_id, value } => {
                // Set the selected option value via ref_id lookup
                let value_escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
                let js = format!(r#"(() => {{
    var el = __wraith_get_by_ref({ref_id});
    if (!el) return 'NOT_FOUND: @e{ref_id}';
    try {{ el.focus(); }} catch(e) {{}}
    el.value = '{value_escaped}';
    if (el._value !== undefined) el._value = '{value_escaped}';
    try {{ el.dispatchEvent(new Event('change', {{ bubbles: true }})); }} catch(e) {{}}
    try {{ el.dispatchEvent(new Event('input', {{ bubbles: true }})); }} catch(e) {{}}
    return 'SELECTED: ' + (el.value || el._value || '');
}})()"#, ref_id = ref_id, value_escaped = value_escaped);
                match self.engine.eval_js(&js).await {
                    Ok(result) => Ok(ActionResult::Success { message: format!("@e{}: {}", ref_id, result) }),
                    Err(e) => Ok(ActionResult::Failed { error: format!("Select @e{} failed: {}", ref_id, e) })
                }
            }
            BrowserAction::SubmitForm { ref_id } => {
                // Serialize all form fields from the DOM and attempt direct HTTP POST.
                // React forms don't use traditional form action — they POST via XHR to an API.
                // Since React isn't loaded in QuickJS, we serialize the DOM values ourselves
                // and POST them directly via Wraith's HTTP client.
                let js = format!(
                    r#"(() => {{
                        var el = __wraith_get_by_ref({ref_id});
                        if (!el) return JSON.stringify({{ error: 'NOT_FOUND: @e{ref_id}' }});

                        // Find the containing form
                        var form = null;
                        if (el.tag === 'form') {{ form = el; }}
                        else {{
                            // Walk up parents to find form
                            var parent = el.parentNode;
                            var depth = 0;
                            while (parent && depth < 20) {{
                                if (parent.tag === 'form') {{ form = parent; break; }}
                                parent = parent.parentNode;
                                depth++;
                            }}
                        }}

                        // Collect all input values from the page (form or global)
                        var fields = {{}};
                        var fileFields = {{}};
                        var inputs = document.querySelectorAll('input, select, textarea');
                        for (var i = 0; i < inputs.length; i++) {{
                            var inp = inputs[i];
                            var name = inp.name || (inp.attrs ? inp.attrs.name : null) || (inp.attrs ? inp.attrs.id : null) || inp.id;
                            if (!name) continue;
                            var val = inp.value || inp._value || '';
                            var type = (inp.attrs ? inp.attrs.type : null) || 'text';
                            if (type === 'file') {{
                                // Track file inputs separately
                                if (inp.files && inp.files.length > 0) {{
                                    fileFields[name] = {{
                                        fileName: inp.files[0].name,
                                        fileType: inp.files[0].type,
                                        fileSize: inp.files[0].size
                                    }};
                                }}
                                continue;
                            }}
                            if (type === 'checkbox' || type === 'radio') {{
                                if (inp.checked || (inp.attrs && inp.attrs.checked)) {{
                                    fields[name] = val || 'on';
                                }}
                                continue;
                            }}
                            if (val) fields[name] = val;
                        }}

                        // Get the form action URL
                        var action = '';
                        if (form && form.attrs) {{
                            action = form.attrs.action || form.attrs['data-action'] || '';
                        }}

                        return JSON.stringify({{
                            action: action,
                            method: (form && form.attrs ? form.attrs.method : '') || 'POST',
                            fields: fields,
                            fileFields: fileFields,
                            formFound: !!form,
                            fieldCount: Object.keys(fields).length
                        }});
                    }})()"#,
                    ref_id = ref_id,
                );

                match self.engine.eval_js(&js).await {
                    Ok(result) => {
                        // Try to parse the serialized form data
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&result) {
                            if let Some(error) = data.get("error") {
                                return Ok(ActionResult::Failed { error: error.as_str().unwrap_or("unknown").to_string() });
                            }

                            let fields = data.get("fields").and_then(|f| f.as_object());
                            let field_count = data.get("fieldCount").and_then(|c| c.as_u64()).unwrap_or(0);
                            let action = data.get("action").and_then(|a| a.as_str()).unwrap_or("");
                            let form_found = data.get("formFound").and_then(|f| f.as_bool()).unwrap_or(false);

                            if field_count == 0 {
                                return Ok(ActionResult::Failed {
                                    error: format!("No field values found to submit (form_found={}). Fields may not have name attributes.", form_found)
                                });
                            }

                            // Build the submission URL — ATS-aware endpoint detection
                            let current_url = self.engine.current_url().unwrap_or("").to_string();
                            let (submit_url, content_type) = detect_ats_submit_endpoint(&current_url, action);

                            // Serialize fields in the format the ATS expects
                            let fields_json = if let Some(f) = fields {
                                serde_json::Value::Object(f.clone()).to_string()
                            } else {
                                "{}".to_string()
                            };

                            // For Greenhouse, we need multipart form data
                            // For others, JSON is fine
                            let body = if content_type.contains("multipart") {
                                // Greenhouse API expects specific field mappings
                                fields_json.clone() // submit_form_data_multipart handles conversion
                            } else {
                                fields_json.clone()
                            };

                            // Do the actual HTTP POST via Wraith's native client
                            match self.engine.submit_form_data_with_content_type(&submit_url, &body, &content_type).await {
                                Ok(response) => {
                                    Ok(ActionResult::Success {
                                        message: format!(
                                            "SUBMITTED: POST {} ({} fields) → {}",
                                            submit_url, field_count, response
                                        )
                                    })
                                }
                                Err(e) => {
                                    // Fallback: report what we collected so the caller can retry
                                    Ok(ActionResult::Success {
                                        message: format!(
                                            "FORM_SERIALIZED: {} fields collected, POST to {} failed: {}. Fields: {}",
                                            field_count, submit_url, e,
                                            fields_json.chars().take(500).collect::<String>()
                                        )
                                    })
                                }
                            }
                        } else {
                            Ok(ActionResult::Success { message: format!("Form data: {}", result) })
                        }
                    }
                    Err(e) => Ok(ActionResult::Failed { error: format!("Submit @e{} failed: {}", ref_id, e) })
                }
            }
            _ => {
                Ok(ActionResult::Success { message: "Action acknowledged (Sevro stub)".to_string() })
            }
        }
    }

    async fn eval_js(&self, script: &str) -> BrowserResult<String> {
        self.engine.eval_js(script).await
            .map_err(BrowserError::JsEvalFailed)
    }

    async fn page_source(&self) -> BrowserResult<String> {
        self.engine.page_source()
            .map(|s| s.to_string())
            .ok_or_else(|| BrowserError::EngineError("No page loaded".to_string()))
    }

    async fn current_url(&self) -> Option<String> {
        self.engine.current_url().map(|s| s.to_string())
    }

    async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        Err(BrowserError::ScreenshotFailed("Not available in Sevro (Phase 3)".to_string()))
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            javascript: self.engine.config().enable_javascript,
            screenshots: ScreenshotCapability::None,
            layout: true,
            cookies: true,
            stealth: true,
        }
    }

    async fn shutdown(&mut self) -> BrowserResult<()> {
        info!("Shutting down Sevro engine");
        self.engine.shutdown();
        Ok(())
    }
}
