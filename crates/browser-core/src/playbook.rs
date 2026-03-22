//! YAML playbook parser and runner for Wraith swarm automation.
//!
//! A playbook is a declarative YAML document that describes a browser automation
//! sequence. The runner parses the playbook, resolves `{{variable}}` templates,
//! and iterates through steps that can later be dispatched as MCP tool calls.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Variable definitions
// ---------------------------------------------------------------------------

/// The type of a playbook variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarType {
    String,
    Text,
    File,
    Number,
    Bool,
    List,
}

impl Default for VarType {
    fn default() -> Self {
        Self::String
    }
}

/// Declaration of a single playbook variable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableDef {
    #[serde(default)]
    pub required: bool,

    #[serde(rename = "type", default)]
    pub var_type: VarType,

    #[serde(default)]
    pub default: Option<String>,

    #[serde(default)]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Error handling policy
// ---------------------------------------------------------------------------

/// Per-step error handling policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnError {
    Abort,
    Skip,
    Retry,
    Screenshot,
}

impl Default for OnError {
    fn default() -> Self {
        Self::Abort
    }
}

// ---------------------------------------------------------------------------
// Playbook steps
// ---------------------------------------------------------------------------

/// A single step in a playbook.
///
/// Each variant maps to a browser automation action. Common per-step metadata
/// (on_error, optional, timeout, delay_before, delay_after) is stored in
/// [`StepMeta`] alongside the action in [`PlaybookEntry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PlaybookStep {
    Navigate {
        url: String,
        #[serde(default)]
        wait_for: Option<String>,
        #[serde(default)]
        timeout: Option<u64>,
    },
    NavigateCdp {
        url: String,
        #[serde(default)]
        wait_for: Option<String>,
    },
    Click {
        selector: String,
        #[serde(default)]
        wait_after: Option<String>,
        #[serde(default)]
        double: Option<bool>,
    },
    Fill {
        selector: String,
        value: String,
        #[serde(default)]
        clear_first: Option<bool>,
        #[serde(default)]
        type_delay: Option<u64>,
    },
    Select {
        selector: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    CustomDropdown {
        selector: String,
        value: String,
        #[serde(default)]
        optional: Option<bool>,
        #[serde(default)]
        option_selector: Option<String>,
        #[serde(default)]
        search_input: Option<String>,
    },
    UploadFile {
        selector: String,
        path: String,
    },
    Submit {
        selector: String,
        #[serde(default)]
        wait_for: Option<String>,
        #[serde(default)]
        wait_for_navigation: Option<bool>,
        #[serde(default)]
        timeout: Option<u64>,
    },
    Wait {
        #[serde(default)]
        ms: Option<u64>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        url_contains: Option<String>,
        #[serde(default)]
        timeout: Option<u64>,
    },
    Extract {
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        attribute: Option<String>,
        #[serde(default)]
        store_as: Option<String>,
        /// Legacy alias kept for backwards compat with `output` field.
        #[serde(default)]
        output: Option<String>,
    },
    EvalJs {
        #[serde(alias = "script")]
        code: String,
        #[serde(default)]
        store_as: Option<String>,
    },
    Verify {
        check: String,
        #[serde(default)]
        or_check: Option<String>,
        #[serde(default)]
        retry_on_fail: bool,
        #[serde(default)]
        retry_count: Option<u32>,
        #[serde(default)]
        retry_delay: Option<u64>,
    },
    Screenshot {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        full_page: Option<bool>,
        #[serde(default)]
        selector: Option<String>,
    },
    Conditional {
        #[serde(default)]
        if_exists: Option<String>,
        #[serde(default)]
        if_visible: Option<String>,
        #[serde(default)]
        if_url_contains: Option<String>,
        #[serde(default)]
        if_variable: Option<String>,
        #[serde(default, rename = "then")]
        then_steps: Vec<PlaybookStep>,
        #[serde(default, rename = "else")]
        else_steps: Vec<PlaybookStep>,
    },
    Repeat {
        for_each: String,
        #[serde(default = "default_item_name")]
        r#as: String,
        steps: Vec<PlaybookStep>,
    },
}

fn default_item_name() -> String {
    "item".to_string()
}

// ---------------------------------------------------------------------------
// Top-level playbook
// ---------------------------------------------------------------------------

/// A complete Wraith playbook parsed from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub on_error: Option<OnError>,
    #[serde(default)]
    pub variables: HashMap<String, VariableDef>,
    pub steps: Vec<PlaybookStep>,
}

fn default_engine() -> String {
    "cdp".to_string()
}

impl Playbook {
    /// Parse a YAML string into a `Playbook`.
    pub fn from_yaml(yaml_str: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml_str)
    }

    /// Validate that all required variables are present in `supplied`.
    /// Returns a list of missing variable names.
    pub fn validate_variables(&self, supplied: &HashMap<String, String>) -> Vec<String> {
        self.variables
            .iter()
            .filter(|(_, def)| def.required && def.default.is_none())
            .filter(|(name, _)| !supplied.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Step execution result
// ---------------------------------------------------------------------------

/// Result of executing a single playbook step.
#[derive(Debug, Clone)]
pub enum StepResult {
    /// Step completed successfully.
    Ok,
    /// Step completed and produced a value to store.
    Value(String),
    /// Step was skipped (optional / on_error: skip).
    Skipped,
    /// Step failed with an error message.
    Failed(String),
}

// ---------------------------------------------------------------------------
// Playbook runner (state machine)
// ---------------------------------------------------------------------------

/// Drives execution of a parsed [`Playbook`], tracking variable resolution
/// and step progress. Does **not** perform actual browser actions — the caller
/// is responsible for dispatching each step to the appropriate engine.
pub struct PlaybookRunner {
    playbook: Playbook,
    /// Static variables supplied at invocation time.
    variables: HashMap<String, String>,
    /// Runtime variables set by `extract` / `eval_js` steps via `store_as`.
    runtime_vars: HashMap<String, String>,
    /// Index of the next step to execute.
    current_step: usize,
    /// Results of completed steps, indexed by step position.
    results: Vec<(usize, StepResult)>,
}

impl PlaybookRunner {
    /// Create a new runner for the given playbook and caller-supplied variables.
    ///
    /// Missing required variables (with no default) are **not** rejected here;
    /// call [`Playbook::validate_variables`] beforehand if you want early
    /// validation.
    pub fn new(playbook: Playbook, variables: HashMap<String, String>) -> Self {
        // Merge defaults from variable definitions.
        let mut merged = HashMap::new();
        for (name, def) in &playbook.variables {
            if let Some(ref default) = def.default {
                merged.insert(name.clone(), default.clone());
            }
        }
        // Caller-supplied values override defaults.
        for (k, v) in &variables {
            merged.insert(k.clone(), v.clone());
        }

        Self {
            playbook,
            variables: merged,
            runtime_vars: HashMap::new(),
            current_step: 0,
            results: Vec::new(),
        }
    }

    /// Replace `{{var_name}}` placeholders in `template` with values from the
    /// combined variable set (runtime > static > built-in).
    pub fn resolve_variable(&self, template: &str) -> String {
        let mut result = template.to_string();
        // Runtime variables take priority.
        for (key, value) in &self.runtime_vars {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }
        // Then static / default variables.
        for (key, value) in &self.variables {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }
        // Built-in: {{timestamp}}
        let placeholder_ts = "{{timestamp}}";
        if result.contains(placeholder_ts) {
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            result = result.replace(placeholder_ts, &ts);
        }
        // Built-in: {{_step}}
        let placeholder_step = "{{_step}}";
        if result.contains(placeholder_step) {
            result = result.replace(placeholder_step, &self.current_step.to_string());
        }
        result
    }

    /// Return the next step to execute, or `None` if the playbook is complete.
    pub fn next_step(&self) -> Option<&PlaybookStep> {
        self.playbook.steps.get(self.current_step)
    }

    /// Record the result of executing step at `step_index` and advance the
    /// cursor. If the result is `StepResult::Value`, store it as a runtime
    /// variable keyed by the step's `store_as` / `output` field (caller must
    /// supply the key via `runtime_key`).
    pub fn mark_complete(&mut self, step_index: usize, result: StepResult, runtime_key: Option<&str>) {
        if let StepResult::Value(ref val) = result {
            if let Some(key) = runtime_key {
                self.runtime_vars.insert(key.to_string(), val.clone());
            }
        }
        self.results.push((step_index, result));
        if self.current_step == step_index {
            self.current_step += 1;
        }
    }

    /// Set a runtime variable (e.g. from an `extract` or `eval_js` step).
    pub fn set_runtime_var(&mut self, key: String, value: String) {
        self.runtime_vars.insert(key, value);
    }

    /// Check whether all top-level steps have been completed.
    pub fn is_complete(&self) -> bool {
        self.current_step >= self.playbook.steps.len()
    }

    /// Return `(completed, total)` step counts.
    pub fn progress(&self) -> (usize, usize) {
        (self.current_step, self.playbook.steps.len())
    }

    /// Borrow the underlying playbook.
    pub fn playbook(&self) -> &Playbook {
        &self.playbook
    }

    /// Borrow the merged variable map (static + defaults, no runtime).
    pub fn variables(&self) -> &HashMap<String, String> {
        &self.variables
    }

    /// Borrow the runtime variable map.
    pub fn runtime_vars(&self) -> &HashMap<String, String> {
        &self.runtime_vars
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PLAYBOOK: &str = r#"
name: greenhouse-apply
description: Apply to a Greenhouse job posting
platform: greenhouse
engine: cdp
version: 1

variables:
  job_url:
    required: true
  first_name:
    required: true
  last_name:
    required: true
  email:
    required: true
  phone:
    required: true
  resume_path:
    required: true
    type: file
  cover_letter:
    required: false
    type: text
  linkedin_url:
    required: false

steps:
  - action: navigate
    url: "{{job_url}}"
    wait_for: "input[name='first_name']"
    timeout: 10000

  - action: fill
    selector: "input[name='first_name']"
    value: "{{first_name}}"

  - action: fill
    selector: "input[name='last_name']"
    value: "{{last_name}}"

  - action: fill
    selector: "input[name='email']"
    value: "{{email}}"

  - action: fill
    selector: "input[name='phone']"
    value: "{{phone}}"

  - action: upload_file
    selector: "input[type='file']"
    path: "{{resume_path}}"

  - action: conditional
    if_exists: "input[name='cover_letter']"
    then:
      - action: fill
        selector: "input[name='cover_letter']"
        value: "{{cover_letter}}"

  - action: custom_dropdown
    selector: "[data-field='location']"
    value: "United States"
    optional: true

  - action: submit
    selector: "button[type='submit']"

  - action: verify
    check: url_contains("/thank")
    retry_on_fail: true
"#;

    #[test]
    fn parse_sample_playbook() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).expect("failed to parse sample playbook");
        assert_eq!(pb.name, "greenhouse-apply");
        assert_eq!(pb.platform, "greenhouse");
        assert_eq!(pb.engine, "cdp");
        assert_eq!(pb.version, 1);
        assert_eq!(pb.variables.len(), 8);
        assert_eq!(pb.steps.len(), 10);
    }

    #[test]
    fn variable_types_parsed() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let resume = pb.variables.get("resume_path").unwrap();
        assert!(resume.required);
        assert_eq!(resume.var_type, VarType::File);

        let cover = pb.variables.get("cover_letter").unwrap();
        assert!(!cover.required);
        assert_eq!(cover.var_type, VarType::Text);

        // Default type is String
        let url = pb.variables.get("job_url").unwrap();
        assert_eq!(url.var_type, VarType::String);
    }

    #[test]
    fn validate_missing_variables() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let supplied: HashMap<String, String> = HashMap::new();
        let missing = pb.validate_variables(&supplied);
        // All 6 required vars should be missing
        assert_eq!(missing.len(), 6);
        assert!(missing.contains(&"job_url".to_string()));
        assert!(missing.contains(&"email".to_string()));
    }

    #[test]
    fn validate_all_supplied() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let mut supplied = HashMap::new();
        supplied.insert("job_url".into(), "https://example.com/jobs/1".into());
        supplied.insert("first_name".into(), "Jane".into());
        supplied.insert("last_name".into(), "Doe".into());
        supplied.insert("email".into(), "jane@example.com".into());
        supplied.insert("phone".into(), "555-0100".into());
        supplied.insert("resume_path".into(), "/tmp/resume.pdf".into());

        let missing = pb.validate_variables(&supplied);
        assert!(missing.is_empty());
    }

    #[test]
    fn resolve_variables() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let mut vars = HashMap::new();
        vars.insert("job_url".into(), "https://boards.greenhouse.io/test/123".into());
        vars.insert("first_name".into(), "Jane".into());

        let runner = PlaybookRunner::new(pb, vars);
        assert_eq!(
            runner.resolve_variable("{{job_url}}"),
            "https://boards.greenhouse.io/test/123"
        );
        assert_eq!(
            runner.resolve_variable("Hello, {{first_name}}!"),
            "Hello, Jane!"
        );
        // Unknown variables are left as-is
        assert_eq!(
            runner.resolve_variable("{{unknown_var}}"),
            "{{unknown_var}}"
        );
    }

    #[test]
    fn runner_iterates_steps() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let total = pb.steps.len();
        let mut runner = PlaybookRunner::new(pb, HashMap::new());

        assert!(!runner.is_complete());
        assert_eq!(runner.progress(), (0, total));

        // Advance through all steps
        for i in 0..total {
            assert!(runner.next_step().is_some());
            runner.mark_complete(i, StepResult::Ok, None);
        }

        assert!(runner.is_complete());
        assert!(runner.next_step().is_none());
        assert_eq!(runner.progress(), (total, total));
    }

    #[test]
    fn runner_runtime_vars() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let mut runner = PlaybookRunner::new(pb, HashMap::new());

        runner.set_runtime_var("extracted_id".into(), "ABC-123".into());
        assert_eq!(
            runner.resolve_variable("Confirmation: {{extracted_id}}"),
            "Confirmation: ABC-123"
        );
    }

    #[test]
    fn runtime_vars_shadow_static() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let mut vars = HashMap::new();
        vars.insert("first_name".into(), "Jane".into());

        let mut runner = PlaybookRunner::new(pb, vars);
        assert_eq!(runner.resolve_variable("{{first_name}}"), "Jane");

        // Runtime override
        runner.set_runtime_var("first_name".into(), "Override".into());
        assert_eq!(runner.resolve_variable("{{first_name}}"), "Override");
    }

    #[test]
    fn mark_complete_with_value() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        let mut runner = PlaybookRunner::new(pb, HashMap::new());

        runner.mark_complete(0, StepResult::Value("hello".into()), Some("result_key"));
        assert_eq!(
            runner.resolve_variable("{{result_key}}"),
            "hello"
        );
    }

    #[test]
    fn parse_conditional_step() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        // Step 6 is the conditional
        match &pb.steps[6] {
            PlaybookStep::Conditional {
                if_exists,
                then_steps,
                else_steps,
                ..
            } => {
                assert_eq!(if_exists.as_deref(), Some("input[name='cover_letter']"));
                assert_eq!(then_steps.len(), 1);
                assert!(else_steps.is_empty());
            }
            other => panic!("Expected Conditional, got {:?}", other),
        }
    }

    #[test]
    fn parse_navigate_step() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        match &pb.steps[0] {
            PlaybookStep::Navigate { url, wait_for, timeout } => {
                assert_eq!(url, "{{job_url}}");
                assert_eq!(wait_for.as_deref(), Some("input[name='first_name']"));
                assert_eq!(*timeout, Some(10000));
            }
            other => panic!("Expected Navigate, got {:?}", other),
        }
    }

    #[test]
    fn parse_verify_step() {
        let pb = Playbook::from_yaml(SAMPLE_PLAYBOOK).unwrap();
        match &pb.steps[9] {
            PlaybookStep::Verify { check, retry_on_fail, .. } => {
                assert_eq!(check, "url_contains(\"/thank\")");
                assert!(*retry_on_fail);
            }
            other => panic!("Expected Verify, got {:?}", other),
        }
    }

    #[test]
    fn parse_eval_js_with_script_alias() {
        let yaml = r#"
name: test
steps:
  - action: eval_js
    script: "document.title"
    store_as: "page_title"
"#;
        let pb = Playbook::from_yaml(yaml).unwrap();
        match &pb.steps[0] {
            PlaybookStep::EvalJs { code, store_as } => {
                assert_eq!(code, "document.title");
                assert_eq!(store_as.as_deref(), Some("page_title"));
            }
            other => panic!("Expected EvalJs, got {:?}", other),
        }
    }

    #[test]
    fn parse_repeat_step() {
        let yaml = r#"
name: test-repeat
steps:
  - action: repeat
    for_each: "{{urls}}"
    as: "url"
    steps:
      - action: navigate
        url: "{{url}}"
      - action: screenshot
"#;
        let pb = Playbook::from_yaml(yaml).unwrap();
        match &pb.steps[0] {
            PlaybookStep::Repeat { for_each, r#as, steps } => {
                assert_eq!(for_each, "{{urls}}");
                assert_eq!(r#as, "url");
                assert_eq!(steps.len(), 2);
            }
            other => panic!("Expected Repeat, got {:?}", other),
        }
    }

    #[test]
    fn parse_defaults_for_variables() {
        let yaml = r##"
name: test-defaults
variables:
  location:
    required: false
    default: "United States"
steps:
  - action: fill
    selector: "#loc"
    value: "{{location}}"
"##;
        let pb = Playbook::from_yaml(yaml).unwrap();
        let loc = pb.variables.get("location").unwrap();
        assert_eq!(loc.default.as_deref(), Some("United States"));

        // Runner should pick up default
        let runner = PlaybookRunner::new(pb, HashMap::new());
        assert_eq!(runner.resolve_variable("{{location}}"), "United States");
    }

    #[test]
    fn default_engine_when_omitted() {
        let yaml = r#"
name: minimal
steps:
  - action: wait
    ms: 100
"#;
        let pb = Playbook::from_yaml(yaml).unwrap();
        assert_eq!(pb.engine, "cdp");
    }
}
