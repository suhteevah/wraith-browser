//! # wraith-scripting
//!
//! Rhai-based scripting engine for Wraith that lets users write
//! extraction rules, page transforms, and automation scripts.
//!
//! ```text
//! User Script (.rhai) ──► Compile ──► Sandboxed Engine ──► ScriptResult
//!                                         ▲
//!                                    ScriptContext (page data)
//! ```
//!
//! Scripts run in a restricted sandbox: no file I/O, no system commands,
//! bounded operations and memory. Custom functions expose browser context
//! (URL, title, text) and utility helpers (regex, word count, JSON).

use rhai::{Engine, AST, Scope, Dynamic};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, instrument};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during script compilation or execution.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// The Rhai source failed to compile.
    #[error("script compilation failed: {0}")]
    CompileFailed(String),

    /// A runtime error occurred while executing the script.
    #[error("script runtime error: {0}")]
    RuntimeError(String),

    /// The requested script name was not found.
    #[error("script not found: {0}")]
    ScriptNotFound(String),

    /// The script exceeded the maximum operations limit.
    #[error("script execution timed out (exceeded max operations)")]
    Timeout,
}

// ---------------------------------------------------------------------------
// ScriptTrigger
// ---------------------------------------------------------------------------

/// Determines when a loaded script should fire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ScriptTrigger {
    /// Run when navigating to a URL that matches `url_pattern` (substring match).
    OnNavigate { url_pattern: String },

    /// Run during content extraction for the given domain.
    OnExtract { domain: String },

    /// Run before/after the named action.
    OnAction { action_name: String },

    /// Only run when explicitly invoked by name.
    Manual,

    /// Run on every page load.
    Always,
}

// ---------------------------------------------------------------------------
// ScriptResult
// ---------------------------------------------------------------------------

/// The outcome of running a single script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    /// Whether the script completed without error.
    pub success: bool,

    /// Optional string output returned by the script.
    pub output: Option<String>,

    /// Optional structured data extracted by the script.
    pub extracted_data: Option<serde_json::Value>,

    /// Wall-clock duration of execution in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// ScriptContext
// ---------------------------------------------------------------------------

/// Page data injected into the scripting engine so scripts can read it.
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// Current page URL.
    pub url: String,

    /// Domain portion of the URL.
    pub domain: String,

    /// Page title.
    pub title: String,

    /// Raw HTML of the page.
    pub html: String,

    /// Extracted plain-text content.
    pub text_content: String,

    /// Links found on the page: `(text, href)`.
    pub links: Vec<(String, String)>,

    /// Arbitrary user-defined variables available to scripts.
    pub custom_vars: HashMap<String, String>,
}

impl ScriptContext {
    /// Convert this context into a Rhai [`Dynamic`] map that scripts can access.
    pub fn to_dynamic(&self) -> Dynamic {
        let mut map = rhai::Map::new();
        map.insert("url".into(), Dynamic::from(self.url.clone()));
        map.insert("domain".into(), Dynamic::from(self.domain.clone()));
        map.insert("title".into(), Dynamic::from(self.title.clone()));
        map.insert("html".into(), Dynamic::from(self.html.clone()));
        map.insert("text_content".into(), Dynamic::from(self.text_content.clone()));

        // Links as array of two-element arrays
        let links_arr: Vec<Dynamic> = self
            .links
            .iter()
            .map(|(text, href)| {
                let pair: rhai::Array = vec![Dynamic::from(text.clone()), Dynamic::from(href.clone())];
                Dynamic::from(pair)
            })
            .collect();
        map.insert("links".into(), Dynamic::from(links_arr));

        // Custom vars as a nested map
        let mut vars_map = rhai::Map::new();
        for (k, v) in &self.custom_vars {
            vars_map.insert(k.clone().into(), Dynamic::from(v.clone()));
        }
        map.insert("custom_vars".into(), Dynamic::from(vars_map));

        Dynamic::from(map)
    }
}

// ---------------------------------------------------------------------------
// LoadedScript
// ---------------------------------------------------------------------------

/// A compiled, ready-to-run script stored inside the engine.
#[derive(Debug, Clone)]
pub struct LoadedScript {
    /// Human-readable name for the script.
    pub name: String,

    /// Original Rhai source code.
    pub source: String,

    /// When this script should fire.
    pub trigger: ScriptTrigger,

    /// Pre-compiled AST for fast repeated execution.
    pub compiled: AST,
}

// ---------------------------------------------------------------------------
// ScriptEngine
// ---------------------------------------------------------------------------

/// Sandboxed Rhai scripting engine with browser-context helpers.
pub struct ScriptEngine {
    /// The underlying Rhai engine (shared via Arc for `Send + Sync`).
    engine: Arc<Engine>,

    /// All loaded scripts, keyed by order of insertion.
    scripts: Vec<LoadedScript>,
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("scripts_count", &self.scripts.len())
            .finish()
    }
}

impl ScriptEngine {
    /// Create a new scripting engine with sandbox restrictions and
    /// browser-context helper functions registered.
    #[instrument]
    pub fn new() -> Self {
        let mut engine = Engine::new();

        // ----- Sandbox restrictions -----
        // Disable all file/system access — scripts live in a pure-compute sandbox.
        engine.set_max_operations(100_000);
        engine.set_max_string_size(1_024 * 1_024); // 1 MB
        engine.set_max_array_size(10_000);

        // ----- Register helper functions -----
        // These closures capture nothing from ScriptContext directly; the
        // context is injected into the Scope at call-time via `run_script`.

        engine.register_fn("count_words", |text: &str| -> i64 {
            text.split_whitespace().count() as i64
        });

        engine.register_fn("truncate", |text: &str, max_len: i64| -> String {
            let max = max_len.max(0) as usize;
            if text.len() <= max {
                text.to_string()
            } else {
                text.chars().take(max).collect()
            }
        });

        engine.register_fn("extract_regex", |pattern: &str, text: &str| -> String {
            match regex::Regex::new(pattern) {
                Ok(re) => re
                    .find(text)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            }
        });

        engine.register_fn(
            "extract_all_regex",
            |pattern: &str, text: &str| -> rhai::Array {
                match regex::Regex::new(pattern) {
                    Ok(re) => re
                        .find_iter(text)
                        .map(|m| Dynamic::from(m.as_str().to_string()))
                        .collect(),
                    Err(_) => rhai::Array::new(),
                }
            },
        );

        engine.register_fn("to_json", |value: Dynamic| -> String {
            if value.is_string() {
                // Wrap plain strings in JSON string encoding
                serde_json::to_string(&value.into_string().unwrap_or_default())
                    .unwrap_or_default()
            } else if value.is_int() {
                value.as_int().map(|v| v.to_string()).unwrap_or_default()
            } else if value.is_bool() {
                value
                    .as_bool()
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            } else if value.is_array() {
                // Best-effort array serialization
                let arr = value.into_array().unwrap_or_default();
                let items: Vec<String> = arr
                    .into_iter()
                    .map(|v| {
                        if v.is_string() {
                            serde_json::to_string(&v.into_string().unwrap_or_default())
                                .unwrap_or_default()
                        } else {
                            v.to_string()
                        }
                    })
                    .collect();
                format!("[{}]", items.join(","))
            } else {
                value.to_string()
            }
        });

        engine.register_fn("log", |message: &str| {
            info!(script_log = %message, "rhai script log");
        });

        info!("ScriptEngine created with sandbox restrictions");

        Self {
            engine: Arc::new(engine),
            scripts: Vec::new(),
        }
    }

    /// Compile and store a script under the given `name`.
    ///
    /// If a script with the same name already exists it is replaced.
    #[instrument(skip(self, source), fields(source_len = source.len()))]
    pub fn load_script(
        &mut self,
        name: &str,
        source: &str,
        trigger: ScriptTrigger,
    ) -> Result<(), ScriptError> {
        let compiled = self
            .engine
            .compile(source)
            .map_err(|e| ScriptError::CompileFailed(e.to_string()))?;

        debug!(name = %name, "Script compiled successfully");

        // Replace if already present
        self.scripts.retain(|s| s.name != name);

        self.scripts.push(LoadedScript {
            name: name.to_string(),
            source: source.to_string(),
            trigger,
            compiled,
        });

        info!(name = %name, total_scripts = self.scripts.len(), "Script loaded");
        Ok(())
    }

    /// Read a Rhai script from a file on disk and load it.
    #[instrument(skip(self))]
    pub fn load_from_file(
        &mut self,
        path: &str,
        trigger: ScriptTrigger,
    ) -> Result<(), ScriptError> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| ScriptError::RuntimeError(format!("failed to read {path}: {e}")))?;

        // Derive a name from the filename
        let name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path);

        self.load_script(name, &source, trigger)
    }

    /// Execute a specific script by name with the provided page context.
    #[instrument(skip(self, context), fields(script_name = %name))]
    pub fn run_script(
        &self,
        name: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, ScriptError> {
        let script = self
            .scripts
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| ScriptError::ScriptNotFound(name.to_string()))?;

        self.execute_script(script, context)
    }

    /// Run every loaded script whose trigger matches `trigger`.
    ///
    /// Returns a vec of `(script_name, result)` pairs.
    #[instrument(skip(self, context))]
    pub fn run_triggered(
        &self,
        trigger: &ScriptTrigger,
        context: &ScriptContext,
    ) -> Vec<(String, Result<ScriptResult, ScriptError>)> {
        self.scripts
            .iter()
            .filter(|s| trigger_matches(&s.trigger, trigger, context))
            .map(|s| {
                let result = self.execute_script(s, context);
                (s.name.clone(), result)
            })
            .collect()
    }

    /// List all loaded scripts as `(name, trigger)` pairs.
    #[instrument(skip(self))]
    pub fn list_scripts(&self) -> Vec<(String, ScriptTrigger)> {
        self.scripts
            .iter()
            .map(|s| (s.name.clone(), s.trigger.clone()))
            .collect()
    }

    /// Remove a script by name. Returns `true` if a script was removed.
    #[instrument(skip(self))]
    pub fn remove_script(&mut self, name: &str) -> bool {
        let before = self.scripts.len();
        self.scripts.retain(|s| s.name != name);
        let removed = self.scripts.len() < before;
        if removed {
            info!(name = %name, "Script removed");
        }
        removed
    }

    // ----- internal helpers ------------------------------------------------

    /// Execute a pre-compiled script against a page context.
    fn execute_script(
        &self,
        script: &LoadedScript,
        context: &ScriptContext,
    ) -> Result<ScriptResult, ScriptError> {
        let start = Instant::now();

        // Build a Scope with page context as both a structured map (`page`)
        // and as individual convenience variables for quick access.
        //
        // Scripts can use either style:
        //   - `page.title`      — via the dynamic map
        //   - `get_title()`     — via registered native functions (see below)
        let mut scope = Scope::new();
        scope.push("page", context.to_dynamic());

        // Compile and execute a combined source that wraps the user script
        // with variable definitions for the accessor functions.
        // Rhai native `fn` definitions cannot see scope variables, so we
        // inject the values as string literals in a preamble source.
        let preamble = format!(
            "let _url_ = {:?};\n\
             let _domain_ = {:?};\n\
             let _title_ = {:?};\n\
             let _text_ = {:?};\n",
            context.url,
            context.domain,
            context.title,
            context.text_content,
        );

        // Provide accessor functions using Rhai closures (which DO capture
        // variables from the enclosing block, unlike `fn` which cannot).
        let accessors = "\
let get_url    = || _url_;
let get_domain = || _domain_;
let get_title  = || _title_;
let get_text   = || _text_;
";

        let combined_source = format!("{preamble}{accessors}\n{}", &script.source);
        let combined_ast = self.engine.compile(&combined_source).map_err(|e| {
            ScriptError::RuntimeError(format!("script compile error: {e}"))
        })?;

        let result = self.engine.eval_ast_with_scope::<Dynamic>(&mut scope, &combined_ast);

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(val) => {
                let output_str = if val.is_unit() {
                    None
                } else {
                    Some(val.to_string())
                };

                // Attempt to parse output as JSON for `extracted_data`
                let extracted_data = output_str.as_ref().and_then(|s| {
                    serde_json::from_str::<serde_json::Value>(s).ok()
                });

                debug!(
                    script = %script.name,
                    duration_ms,
                    has_output = output_str.is_some(),
                    "Script executed successfully"
                );

                Ok(ScriptResult {
                    success: true,
                    output: output_str,
                    extracted_data,
                    duration_ms,
                })
            }
            Err(err) => {
                // Detect timeout / max-operations exceeded
                let err_string = err.to_string();
                if err_string.contains("Too many operations") {
                    Err(ScriptError::Timeout)
                } else {
                    Err(ScriptError::RuntimeError(err_string))
                }
            }
        }
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Trigger matching logic
// ---------------------------------------------------------------------------

/// Determine whether a script's trigger matches the requested trigger + context.
fn trigger_matches(
    script_trigger: &ScriptTrigger,
    requested: &ScriptTrigger,
    context: &ScriptContext,
) -> bool {
    match (script_trigger, requested) {
        (ScriptTrigger::Always, _) => true,
        (ScriptTrigger::Manual, ScriptTrigger::Manual) => true,
        (
            ScriptTrigger::OnNavigate { url_pattern },
            ScriptTrigger::OnNavigate { .. },
        ) => context.url.contains(url_pattern.as_str()),
        (
            ScriptTrigger::OnExtract { domain },
            ScriptTrigger::OnExtract { .. },
        ) => context.domain == *domain || context.domain.ends_with(&format!(".{domain}")),
        (
            ScriptTrigger::OnAction { action_name: a },
            ScriptTrigger::OnAction { action_name: b },
        ) => a == b,
        _ => false,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal ScriptContext for testing.
    fn test_context() -> ScriptContext {
        ScriptContext {
            url: "https://example.com/article/123".to_string(),
            domain: "example.com".to_string(),
            title: "Test Page Title".to_string(),
            html: "<html><body><p>hello world</p></body></html>".to_string(),
            text_content: "hello world from the test page".to_string(),
            links: vec![
                ("Home".to_string(), "https://example.com".to_string()),
                ("About".to_string(), "https://example.com/about".to_string()),
            ],
            custom_vars: HashMap::from([
                ("user_agent".to_string(), "wraith/0.1".to_string()),
            ]),
        }
    }

    #[test]
    fn test_load_script_compiles_valid_rhai() {
        let mut engine = ScriptEngine::new();
        let result = engine.load_script("hello", r#"let x = 1 + 2; x"#, ScriptTrigger::Manual);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_script_rejects_invalid_rhai() {
        let mut engine = ScriptEngine::new();
        let result = engine.load_script(
            "bad",
            r#"let x = @@@ invalid syntax"#,
            ScriptTrigger::Manual,
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            ScriptError::CompileFailed(_) => {} // expected
            other => panic!("expected CompileFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_run_script_simple_extraction() {
        let mut engine = ScriptEngine::new();
        engine
            .load_script("title_grab", r#"let x = get_title.call(); x"#, ScriptTrigger::Manual)
            .unwrap();

        let ctx = test_context();
        let result = engine.run_script("title_grab", &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("Test Page Title"));
    }

    #[test]
    fn test_run_script_not_found() {
        let engine = ScriptEngine::new();
        let ctx = test_context();
        let err = engine.run_script("nonexistent", &ctx).unwrap_err();
        match err {
            ScriptError::ScriptNotFound(name) => assert_eq!(name, "nonexistent"),
            other => panic!("expected ScriptNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_run_triggered_runs_matching_only() {
        let mut engine = ScriptEngine::new();

        engine
            .load_script(
                "nav_match",
                r#""matched navigation""#,
                ScriptTrigger::OnNavigate {
                    url_pattern: "example.com".to_string(),
                },
            )
            .unwrap();

        engine
            .load_script(
                "nav_nomatch",
                r#""should not run""#,
                ScriptTrigger::OnNavigate {
                    url_pattern: "other-site.org".to_string(),
                },
            )
            .unwrap();

        engine
            .load_script("always_run", r#""always""#, ScriptTrigger::Always)
            .unwrap();

        let ctx = test_context();
        let trigger = ScriptTrigger::OnNavigate {
            url_pattern: String::new(), // the pattern is on the script side
        };

        let results = engine.run_triggered(&trigger, &ctx);

        let names: Vec<&str> = results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"nav_match"), "should include matching nav script");
        assert!(
            !names.contains(&"nav_nomatch"),
            "should exclude non-matching nav script"
        );
        assert!(names.contains(&"always_run"), "should include Always trigger");
    }

    #[test]
    fn test_sandbox_max_operations() {
        let mut engine = ScriptEngine::new();
        // Infinite loop should be caught by the operations limit
        engine
            .load_script("infinite", r#"loop { let x = 1; }"#, ScriptTrigger::Manual)
            .unwrap();

        let ctx = test_context();
        let err = engine.run_script("infinite", &ctx).unwrap_err();
        match err {
            ScriptError::Timeout => {} // expected
            other => panic!("expected Timeout, got: {other:?}"),
        }
    }

    #[test]
    fn test_extract_regex_function() {
        let mut engine = ScriptEngine::new();
        engine
            .load_script(
                "regex_test",
                r#"extract_regex("\\d+", "article 42 is here")"#,
                ScriptTrigger::Manual,
            )
            .unwrap();

        let ctx = test_context();
        let result = engine.run_script("regex_test", &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("42"));
    }

    #[test]
    fn test_count_words_function() {
        let mut engine = ScriptEngine::new();
        engine
            .load_script(
                "words",
                r#"count_words("hello world foo bar")"#,
                ScriptTrigger::Manual,
            )
            .unwrap();

        let ctx = test_context();
        let result = engine.run_script("words", &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("4"));
    }

    #[test]
    fn test_list_and_remove_scripts() {
        let mut engine = ScriptEngine::new();
        engine
            .load_script("a", r#"1"#, ScriptTrigger::Manual)
            .unwrap();
        engine
            .load_script("b", r#"2"#, ScriptTrigger::Always)
            .unwrap();

        let list = engine.list_scripts();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "a");
        assert_eq!(list[1].0, "b");

        assert!(engine.remove_script("a"));
        assert!(!engine.remove_script("nonexistent"));

        let list = engine.list_scripts();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "b");
    }

    #[test]
    fn test_script_context_to_dynamic() {
        let ctx = test_context();
        let dyn_val = ctx.to_dynamic();
        let map = dyn_val.cast::<rhai::Map>();

        assert_eq!(
            map.get("url").unwrap().clone().into_string().unwrap(),
            "https://example.com/article/123"
        );
        assert_eq!(
            map.get("domain").unwrap().clone().into_string().unwrap(),
            "example.com"
        );
        assert_eq!(
            map.get("title").unwrap().clone().into_string().unwrap(),
            "Test Page Title"
        );

        let links = map.get("links").unwrap().clone().into_array().unwrap();
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_script.rhai");
        std::fs::write(&file_path, r#"let x = 42; x"#).unwrap();

        let mut engine = ScriptEngine::new();
        engine
            .load_from_file(file_path.to_str().unwrap(), ScriptTrigger::Manual)
            .unwrap();

        let list = engine.list_scripts();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "test_script");

        let ctx = test_context();
        let result = engine.run_script("test_script", &ctx).unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("42"));
    }
}
