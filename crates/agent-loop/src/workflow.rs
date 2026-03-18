//! Workflow Recording & Replay — records human or agent browser sessions,
//! compiles them into parameterized self-healing workflows, and replays them
//! with variable substitution.
//!
//! A [`WorkflowRecorder`] observes browser actions as they happen and builds a
//! list of [`WorkflowStep`]s. When recording stops, [`WorkflowRecorder::finalize`]
//! compiles the raw steps into a reusable [`Workflow`] with auto-detected
//! [`WorkflowParameter`]s (e.g. numbers and UUIDs in URLs become `{{param_N}}`).
//!
//! [`WorkflowExecutor`] takes a compiled workflow, binds concrete variable values,
//! and yields [`ResolvedStep`]s one at a time for the browser engine to execute.
//! Multiple CSS selectors per step (`selector_hints`) enable self-healing: if the
//! primary selector breaks, the executor can try alternatives.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, instrument};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Core data types
// ---------------------------------------------------------------------------

/// An individual action the browser should perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowAction {
    /// Navigate to a URL. The `url_template` supports `{{variable}}` substitution.
    Navigate { url_template: String },
    /// Click an element identified by `selector`.
    Click { selector: String },
    /// Fill an input identified by `selector`. The `value_template` supports
    /// `{{variable}}` substitution.
    Fill {
        selector: String,
        value_template: String,
    },
    /// Select a dropdown option. The `value_template` supports `{{variable}}`
    /// substitution.
    Select {
        selector: String,
        value_template: String,
    },
    /// Press a keyboard key (e.g. `"Enter"`, `"Tab"`).
    KeyPress { key: String },
    /// Wait until `selector` appears in the DOM, or until `timeout_ms` elapses.
    WaitForSelector {
        selector: String,
        timeout_ms: u64,
    },
    /// Wait for a full navigation event, or until `timeout_ms` elapses.
    WaitForNavigation { timeout_ms: u64 },
    /// Extract the text content of `selector` and store it in `variable_name`
    /// for later steps to reference via `{{variable_name}}`.
    ExtractAndStore {
        selector: String,
        variable_name: String,
    },
    /// Assert that the text of `selector` matches `expected`.
    AssertText {
        selector: String,
        expected: String,
    },
    /// Assert that the current URL matches `pattern` (a regex).
    AssertUrl { pattern: String },
}

/// A single step in a recorded or compiled workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Position of this step in the workflow (1-based).
    pub step_number: usize,
    /// The browser action to perform.
    pub action: WorkflowAction,
    /// Regex pattern the page URL must match for this step to apply.
    pub url_pattern: String,
    /// Multiple CSS/XPath selectors for self-healing; if the first fails the
    /// executor can try the rest.
    pub selector_hints: Vec<String>,
    /// Visible text of the target element (useful for fallback matching).
    pub text_hint: Option<String>,
    /// Milliseconds to sleep after executing this step.
    pub wait_after_ms: u64,
    /// Whether to capture a screenshot before executing this step.
    pub screenshot_before: bool,
    /// Free-form annotation added during recording.
    pub notes: Option<String>,
}

/// A declared parameter that a workflow expects callers to supply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowParameter {
    /// Parameter name (used in `{{name}}` templates).
    pub name: String,
    /// Human-readable description of what this parameter is for.
    pub description: String,
    /// Default value used when the caller does not supply one.
    pub default_value: Option<String>,
    /// Whether the executor should refuse to start without this parameter.
    pub required: bool,
}

/// A compiled, reusable workflow ready for replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Longer description of what this workflow does.
    pub description: String,
    /// Parameters the workflow expects.
    pub parameters: Vec<WorkflowParameter>,
    /// Ordered list of steps.
    pub steps: Vec<WorkflowStep>,
    /// When this workflow was compiled.
    pub created_at: DateTime<Utc>,
    /// Arbitrary tags for organisation / search.
    pub tags: Vec<String>,
}

/// Status of a running workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionStatus {
    /// Created but not yet started.
    Pending,
    /// Currently executing steps.
    Running,
    /// Execution paused (e.g. awaiting user input).
    Paused,
    /// All steps finished successfully.
    Completed,
    /// A step failed and execution was aborted.
    Failed,
}

/// Outcome of a single step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Which step this result belongs to.
    pub step_number: usize,
    /// Whether the step succeeded.
    pub success: bool,
    /// Wall-clock time spent executing the step.
    pub duration_ms: u64,
    /// Value extracted by an [`WorkflowAction::ExtractAndStore`] step.
    pub extracted_value: Option<String>,
    /// Error message if the step failed.
    pub error: Option<String>,
}

/// A step with all `{{variable}}` placeholders resolved to concrete values.
#[derive(Debug, Clone)]
pub struct ResolvedStep {
    /// The step number within the workflow.
    pub step_number: usize,
    /// The action with templates already substituted.
    pub action: WorkflowAction,
}

/// Tracks the mutable state of a workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowExecution {
    /// The workflow being executed.
    pub workflow_id: String,
    /// Current variable bindings (parameter values + extracted values).
    pub variables: HashMap<String, String>,
    /// Index of the next step to execute (0-based).
    pub current_step: usize,
    /// Overall execution status.
    pub status: ExecutionStatus,
    /// Results collected so far.
    pub step_results: Vec<StepResult>,
    /// When execution was started.
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Template resolution
// ---------------------------------------------------------------------------

/// Replace all `{{var_name}}` placeholders in `template` with the corresponding
/// values from `variables`. Unresolved placeholders are left as-is.
pub fn resolve_template(template: &str, variables: &HashMap<String, String>) -> String {
    let re = Regex::new(r"\{\{(\w+)\}\}").expect("static regex");
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        variables
            .get(key)
            .cloned()
            .unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
}

// ---------------------------------------------------------------------------
// Recorder
// ---------------------------------------------------------------------------

/// Records browser actions and compiles them into a parameterized [`Workflow`].
pub struct WorkflowRecorder {
    /// Steps collected so far.
    steps: Vec<WorkflowStep>,
    /// Names of variables / parameters detected during recording.
    variables_seen: HashSet<String>,
    /// Monotonically increasing counter for auto-generated parameter names.
    param_counter: usize,
}

impl Default for WorkflowRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowRecorder {
    /// Create a new, empty recorder.
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            variables_seen: HashSet::new(),
            param_counter: 0,
        }
    }

    /// Record a navigation action.
    ///
    /// Numeric and UUID segments in `url` are automatically replaced with
    /// `{{param_N}}` placeholders so the resulting workflow is parameterized.
    #[instrument(skip(self))]
    pub fn record_navigate(&mut self, url: &str) {
        let url_template = self.parameterize_url(url);
        let step_number = self.steps.len() + 1;
        debug!(step_number, url, url_template = %url_template, "recorded navigate");

        self.steps.push(WorkflowStep {
            step_number,
            action: WorkflowAction::Navigate { url_template },
            url_pattern: ".*".to_string(),
            selector_hints: Vec::new(),
            text_hint: None,
            wait_after_ms: 1000,
            screenshot_before: false,
            notes: None,
        });
    }

    /// Record a click action.
    #[instrument(skip(self))]
    pub fn record_click(&mut self, selector: &str, text_hint: Option<&str>) {
        let step_number = self.steps.len() + 1;
        debug!(step_number, selector, "recorded click");

        self.steps.push(WorkflowStep {
            step_number,
            action: WorkflowAction::Click {
                selector: selector.to_string(),
            },
            url_pattern: ".*".to_string(),
            selector_hints: vec![selector.to_string()],
            text_hint: text_hint.map(|s| s.to_string()),
            wait_after_ms: 500,
            screenshot_before: false,
            notes: None,
        });
    }

    /// Record a fill (text input) action.
    ///
    /// If `param_name` is provided the value is stored as `{{param_name}}` in
    /// the template so it can be supplied at replay time.
    #[instrument(skip(self))]
    pub fn record_fill(&mut self, selector: &str, value: &str, param_name: Option<&str>) {
        let step_number = self.steps.len() + 1;
        let value_template = if let Some(name) = param_name {
            self.variables_seen.insert(name.to_string());
            format!("{{{{{}}}}}", name)
        } else {
            value.to_string()
        };
        debug!(step_number, selector, %value_template, "recorded fill");

        self.steps.push(WorkflowStep {
            step_number,
            action: WorkflowAction::Fill {
                selector: selector.to_string(),
                value_template,
            },
            url_pattern: ".*".to_string(),
            selector_hints: vec![selector.to_string()],
            text_hint: None,
            wait_after_ms: 300,
            screenshot_before: false,
            notes: None,
        });
    }

    /// Record an extraction step that reads text from the DOM and stores it in
    /// `variable_name` for subsequent steps to reference.
    #[instrument(skip(self))]
    pub fn record_extract(&mut self, selector: &str, variable_name: &str) {
        let step_number = self.steps.len() + 1;
        self.variables_seen.insert(variable_name.to_string());
        debug!(step_number, selector, variable_name, "recorded extract");

        self.steps.push(WorkflowStep {
            step_number,
            action: WorkflowAction::ExtractAndStore {
                selector: selector.to_string(),
                variable_name: variable_name.to_string(),
            },
            url_pattern: ".*".to_string(),
            selector_hints: vec![selector.to_string()],
            text_hint: None,
            wait_after_ms: 0,
            screenshot_before: false,
            notes: None,
        });
    }

    /// Consume the recorder and produce a compiled [`Workflow`].
    ///
    /// Auto-detected parameters (from URL parameterization and explicit
    /// `param_name` arguments) are included in the workflow's `parameters` list.
    #[instrument(skip(self))]
    pub fn finalize(self, name: &str, description: &str) -> Workflow {
        let parameters: Vec<WorkflowParameter> = self
            .variables_seen
            .iter()
            .map(|v| WorkflowParameter {
                name: v.clone(),
                description: format!("Auto-detected parameter: {}", v),
                default_value: None,
                required: true,
            })
            .collect();

        let workflow = Workflow {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            steps: self.steps,
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        info!(
            workflow_id = %workflow.id,
            name = %workflow.name,
            step_count = workflow.steps.len(),
            param_count = workflow.parameters.len(),
            "workflow finalized"
        );

        workflow
    }

    // -- internal helpers ---------------------------------------------------

    /// Replace bare numeric segments and UUID-shaped segments in a URL with
    /// `{{param_N}}` placeholders.
    fn parameterize_url(&mut self, url: &str) -> String {
        let uuid_re =
            Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
                .expect("static regex");
        let mut result = url.to_string();

        // UUIDs first (longer match takes priority).
        for _mat in uuid_re.find_iter(url) {
            let param_name = self.next_param_name();
            result = result.replacen(_mat.as_str(), &format!("{{{{{}}}}}", param_name), 1);
        }

        // Plain numbers in path segments — replace /123/ with /{{param_N}}/
        let num_re = Regex::new(r"/(\d+)(/|$)").expect("static regex");
        let snapshot = result.clone();
        for _mat in num_re.find_iter(&snapshot) {
            let param_name = self.next_param_name();
            result = result.replacen(_mat.as_str(), &format!("{{{{{}}}}}", param_name), 1);
        }

        result
    }

    /// Generate the next `param_N` name and register it.
    fn next_param_name(&mut self) -> String {
        self.param_counter += 1;
        let name = format!("param_{}", self.param_counter);
        self.variables_seen.insert(name.clone());
        name
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Replays a compiled [`Workflow`] step-by-step, substituting variables into
/// action templates and tracking execution state.
pub struct WorkflowExecutor {
    /// The underlying execution state.
    execution: WorkflowExecution,
    /// The workflow being replayed.
    workflow: Workflow,
}

impl WorkflowExecutor {
    /// Create an executor for `workflow`. All required parameters must be set
    /// via [`set_variable`] / [`set_variables`] before calling [`next_step`].
    #[instrument(skip(workflow), fields(workflow_id = %workflow.id))]
    pub fn new(workflow: Workflow) -> Self {
        info!(workflow_name = %workflow.name, "executor created");
        let execution = WorkflowExecution {
            workflow_id: workflow.id.clone(),
            variables: HashMap::new(),
            current_step: 0,
            status: ExecutionStatus::Pending,
            step_results: Vec::new(),
            started_at: Utc::now(),
        };
        Self {
            execution,
            workflow,
        }
    }

    /// Bind a single variable for template substitution.
    #[instrument(skip(self))]
    pub fn set_variable(&mut self, name: &str, value: &str) {
        debug!(name, value, "variable set");
        self.execution
            .variables
            .insert(name.to_string(), value.to_string());
    }

    /// Bind multiple variables at once.
    #[instrument(skip(self, vars))]
    pub fn set_variables(&mut self, vars: HashMap<String, String>) {
        debug!(count = vars.len(), "bulk variables set");
        self.execution.variables.extend(vars);
    }

    /// Check that every required parameter has a value. Returns `Err` with a
    /// message listing the missing parameters if validation fails.
    #[instrument(skip(self))]
    pub fn validate(&self) -> Result<(), String> {
        let missing: Vec<&str> = self
            .workflow
            .parameters
            .iter()
            .filter(|p| p.required && !self.execution.variables.contains_key(&p.name))
            .map(|p| p.name.as_str())
            .collect();

        if missing.is_empty() {
            debug!("validation passed");
            Ok(())
        } else {
            let msg = format!("Missing required parameters: {}", missing.join(", "));
            debug!(%msg, "validation failed");
            Err(msg)
        }
    }

    /// Advance to the next step, resolving any `{{variable}}` placeholders.
    ///
    /// Returns `None` when all steps have been yielded. On the first call the
    /// execution status transitions from `Pending` to `Running`.
    #[instrument(skip(self))]
    pub fn next_step(&mut self) -> Option<ResolvedStep> {
        if self.execution.current_step >= self.workflow.steps.len() {
            self.execution.status = ExecutionStatus::Completed;
            return None;
        }

        if self.execution.status == ExecutionStatus::Pending {
            self.execution.status = ExecutionStatus::Running;
        }

        let step = &self.workflow.steps[self.execution.current_step];
        let resolved_action = self.resolve_action(&step.action);
        let resolved = ResolvedStep {
            step_number: step.step_number,
            action: resolved_action,
        };

        debug!(step_number = step.step_number, "yielding resolved step");
        self.execution.current_step += 1;
        Some(resolved)
    }

    /// Record the result of the most recently yielded step.
    ///
    /// If the step extracted a value, it is automatically merged into the
    /// variable map so later steps can reference it.
    #[instrument(skip(self, result))]
    pub fn record_result(&mut self, result: StepResult) {
        if !result.success {
            self.execution.status = ExecutionStatus::Failed;
        }

        // Auto-merge extracted values into the variable map.
        if let Some(ref val) = result.extracted_value {
            if let Some(step) = self
                .workflow
                .steps
                .iter()
                .find(|s| s.step_number == result.step_number)
            {
                if let WorkflowAction::ExtractAndStore {
                    ref variable_name, ..
                } = step.action
                {
                    self.execution
                        .variables
                        .insert(variable_name.clone(), val.clone());
                    debug!(variable_name, value = %val, "extracted value stored");
                }
            }
        }

        self.execution.step_results.push(result);
    }

    /// Whether every step has been executed (successfully or not).
    #[instrument(skip(self))]
    pub fn is_complete(&self) -> bool {
        self.execution.current_step >= self.workflow.steps.len()
    }

    /// Returns `(completed_steps, total_steps)`.
    #[instrument(skip(self))]
    pub fn progress(&self) -> (usize, usize) {
        (self.execution.current_step, self.workflow.steps.len())
    }

    /// Serialize the execution results as a JSON string.
    #[instrument(skip(self))]
    pub fn export_results(&self) -> String {
        let summary = serde_json::json!({
            "workflow_id": self.execution.workflow_id,
            "status": self.execution.status,
            "progress": {
                "completed": self.execution.current_step,
                "total": self.workflow.steps.len(),
            },
            "step_results": self.execution.step_results,
            "variables": self.execution.variables,
            "started_at": self.execution.started_at.to_rfc3339(),
        });

        serde_json::to_string_pretty(&summary).unwrap_or_else(|e| {
            format!("{{\"error\": \"failed to serialize: {}\"}}", e)
        })
    }

    // -- internal helpers ---------------------------------------------------

    /// Substitute `{{variables}}` inside every template field of `action`.
    fn resolve_action(&self, action: &WorkflowAction) -> WorkflowAction {
        let vars = &self.execution.variables;
        match action {
            WorkflowAction::Navigate { url_template } => WorkflowAction::Navigate {
                url_template: resolve_template(url_template, vars),
            },
            WorkflowAction::Fill {
                selector,
                value_template,
            } => WorkflowAction::Fill {
                selector: selector.clone(),
                value_template: resolve_template(value_template, vars),
            },
            WorkflowAction::Select {
                selector,
                value_template,
            } => WorkflowAction::Select {
                selector: selector.clone(),
                value_template: resolve_template(value_template, vars),
            },
            // Actions without templates are returned as-is.
            other => other.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_navigate_parameterizes_numbers() {
        let mut rec = WorkflowRecorder::new();
        rec.record_navigate("https://example.com/users/42/posts/7");

        assert_eq!(rec.steps.len(), 1);
        if let WorkflowAction::Navigate { ref url_template } = rec.steps[0].action {
            assert!(
                url_template.contains("{{param_"),
                "expected parameterized URL, got: {}",
                url_template
            );
            assert!(
                !url_template.contains("/42/"),
                "numeric segment 42 should be replaced"
            );
            assert!(
                !url_template.contains("/7"),
                "numeric segment 7 should be replaced"
            );
        } else {
            panic!("expected Navigate action");
        }
    }

    #[test]
    fn test_record_fill_with_param_name() {
        let mut rec = WorkflowRecorder::new();
        rec.record_fill("#email", "test@example.com", Some("email"));

        assert_eq!(rec.steps.len(), 1);
        if let WorkflowAction::Fill {
            ref value_template, ..
        } = rec.steps[0].action
        {
            assert_eq!(value_template, "{{email}}");
        } else {
            panic!("expected Fill action");
        }
    }

    #[test]
    fn test_finalize_produces_workflow_with_parameters() {
        let mut rec = WorkflowRecorder::new();
        rec.record_navigate("https://example.com/items/99");
        rec.record_fill("#search", "query", Some("search_term"));

        let wf = rec.finalize("Test Workflow", "A test");

        assert_eq!(wf.name, "Test Workflow");
        assert_eq!(wf.description, "A test");
        assert_eq!(wf.steps.len(), 2);
        // Should have at least the URL param + the fill param.
        assert!(
            wf.parameters.len() >= 2,
            "expected >=2 params, got {}",
            wf.parameters.len()
        );
        let names: HashSet<String> = wf.parameters.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains("search_term"), "missing search_term param");
    }

    #[test]
    fn test_resolve_template_substitutes_variables() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("age".to_string(), "30".to_string());

        let result = resolve_template("Hello {{name}}, age {{age}}!", &vars);
        assert_eq!(result, "Hello Alice, age 30!");
    }

    #[test]
    fn test_resolve_template_leaves_unknown_vars() {
        let vars = HashMap::new();
        let result = resolve_template("Hello {{unknown}}!", &vars);
        assert_eq!(result, "Hello {{unknown}}!");
    }

    #[test]
    fn test_validate_catches_missing_required_params() {
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: vec![
                WorkflowParameter {
                    name: "required_param".to_string(),
                    description: "a required param".to_string(),
                    default_value: None,
                    required: true,
                },
                WorkflowParameter {
                    name: "optional_param".to_string(),
                    description: "an optional param".to_string(),
                    default_value: Some("default".to_string()),
                    required: false,
                },
            ],
            steps: Vec::new(),
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        let exec = WorkflowExecutor::new(wf);
        let err = exec.validate().unwrap_err();
        assert!(
            err.contains("required_param"),
            "error should mention missing param"
        );
    }

    #[test]
    fn test_next_step_advances_through_steps() {
        let wf = Workflow {
            id: "w1".to_string(),
            name: "nav test".to_string(),
            description: "".to_string(),
            parameters: Vec::new(),
            steps: vec![
                WorkflowStep {
                    step_number: 1,
                    action: WorkflowAction::Navigate {
                        url_template: "https://example.com".to_string(),
                    },
                    url_pattern: ".*".to_string(),
                    selector_hints: Vec::new(),
                    text_hint: None,
                    wait_after_ms: 0,
                    screenshot_before: false,
                    notes: None,
                },
                WorkflowStep {
                    step_number: 2,
                    action: WorkflowAction::Click {
                        selector: "#btn".to_string(),
                    },
                    url_pattern: ".*".to_string(),
                    selector_hints: vec!["#btn".to_string()],
                    text_hint: Some("Submit".to_string()),
                    wait_after_ms: 0,
                    screenshot_before: false,
                    notes: None,
                },
            ],
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        let mut exec = WorkflowExecutor::new(wf);

        let s1 = exec.next_step().expect("should yield step 1");
        assert_eq!(s1.step_number, 1);

        let s2 = exec.next_step().expect("should yield step 2");
        assert_eq!(s2.step_number, 2);

        assert!(exec.next_step().is_none(), "no more steps");
    }

    #[test]
    fn test_is_complete_after_all_steps() {
        let wf = Workflow {
            id: "w2".to_string(),
            name: "single".to_string(),
            description: "".to_string(),
            parameters: Vec::new(),
            steps: vec![WorkflowStep {
                step_number: 1,
                action: WorkflowAction::Navigate {
                    url_template: "https://example.com".to_string(),
                },
                url_pattern: ".*".to_string(),
                selector_hints: Vec::new(),
                text_hint: None,
                wait_after_ms: 0,
                screenshot_before: false,
                notes: None,
            }],
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        let mut exec = WorkflowExecutor::new(wf);
        assert!(!exec.is_complete());

        exec.next_step();
        assert!(exec.is_complete());
    }

    #[test]
    fn test_export_results_produces_valid_json() {
        let wf = Workflow {
            id: "w3".to_string(),
            name: "export test".to_string(),
            description: "".to_string(),
            parameters: Vec::new(),
            steps: vec![WorkflowStep {
                step_number: 1,
                action: WorkflowAction::Navigate {
                    url_template: "https://example.com".to_string(),
                },
                url_pattern: ".*".to_string(),
                selector_hints: Vec::new(),
                text_hint: None,
                wait_after_ms: 0,
                screenshot_before: false,
                notes: None,
            }],
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        let mut exec = WorkflowExecutor::new(wf);
        let _ = exec.next_step();
        exec.record_result(StepResult {
            step_number: 1,
            success: true,
            duration_ms: 42,
            extracted_value: None,
            error: None,
        });

        let json = exec.export_results();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("export should be valid JSON");
        assert_eq!(parsed["workflow_id"], "w3");
        assert_eq!(parsed["progress"]["completed"], 1);
        assert_eq!(parsed["progress"]["total"], 1);
    }

    #[test]
    fn test_navigate_with_variable_substitution() {
        let wf = Workflow {
            id: "w4".to_string(),
            name: "var sub".to_string(),
            description: "".to_string(),
            parameters: vec![WorkflowParameter {
                name: "user_id".to_string(),
                description: "".to_string(),
                default_value: None,
                required: true,
            }],
            steps: vec![WorkflowStep {
                step_number: 1,
                action: WorkflowAction::Navigate {
                    url_template: "https://example.com/users/{{user_id}}/profile".to_string(),
                },
                url_pattern: ".*".to_string(),
                selector_hints: Vec::new(),
                text_hint: None,
                wait_after_ms: 0,
                screenshot_before: false,
                notes: None,
            }],
            created_at: Utc::now(),
            tags: Vec::new(),
        };

        let mut exec = WorkflowExecutor::new(wf);
        exec.set_variable("user_id", "123");

        let step = exec.next_step().expect("should yield step");
        if let WorkflowAction::Navigate { ref url_template } = step.action {
            assert_eq!(url_template, "https://example.com/users/123/profile");
        } else {
            panic!("expected Navigate action");
        }
    }
}
