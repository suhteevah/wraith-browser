//! Declarative Task DAGs — define complex browsing tasks as directed acyclic
//! graphs where independent subtasks run in parallel.
//!
//! Each [`TaskDag`] contains a set of [`TaskNode`]s connected by dependency
//! edges. The scheduler can query [`TaskDag::ready_nodes`] to discover which
//! nodes are eligible to execute concurrently, drive them through their
//! lifecycle, and repeat until [`TaskDag::is_complete`].

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{debug, info, instrument};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A single unit of work within a task DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// Unique identifier for this node within the DAG.
    pub id: String,
    /// Human-readable description of what this node does.
    pub description: String,
    /// The action to perform when this node executes.
    pub action: TaskAction,
    /// IDs of prerequisite nodes that must complete before this node can run.
    pub depends_on: Vec<String>,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// Output produced by this node (populated on completion or failure).
    pub result: Option<String>,
}

/// The concrete action a [`TaskNode`] should perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskAction {
    /// Navigate the browser to the given URL.
    Navigate(String),
    /// Perform a web search with the given query.
    Search(String),
    /// Extract content from the current page using the given description/selector.
    Extract(String),
    /// Click an element described by text or CSS selector.
    Click(String),
    /// Fill a form field — (selector, value).
    Fill(String, String),
    /// Arbitrary instruction for the LLM to interpret.
    Custom(String),
    /// A nested sub-DAG to execute as a single logical step.
    SubDag(TaskDag),
}

/// Lifecycle status of a [`TaskNode`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    /// Not yet eligible to run (has unmet dependencies).
    Pending,
    /// All dependencies satisfied — eligible for execution.
    Ready,
    /// Currently being executed.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Intentionally skipped (e.g., because a dependency failed).
    Skipped,
}

/// A directed acyclic graph of browsing tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDag {
    /// Unique identifier for this DAG.
    pub id: String,
    /// Human-readable name describing the overall goal.
    pub name: String,
    /// The set of task nodes in this DAG.
    pub nodes: Vec<TaskNode>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl TaskDag {
    /// Create a new, empty DAG with the given human-readable name.
    #[instrument(skip_all, fields(name = %name))]
    pub fn new(name: &str) -> Self {
        let id = Uuid::new_v4().to_string();
        info!(dag_id = %id, "Creating new TaskDag");
        Self {
            id,
            name: name.to_string(),
            nodes: Vec::new(),
        }
    }

    /// Append a node to the DAG. Returns `&mut Self` for chaining.
    #[instrument(skip(self), fields(node_id = %node.id))]
    pub fn add_node(&mut self, node: TaskNode) -> &mut Self {
        debug!(node_id = %node.id, description = %node.description, "Adding node to DAG");
        self.nodes.push(node);
        self
    }

    /// Declare that `node_id` depends on `depends_on`.
    ///
    /// Returns an error if either ID is not found in the DAG.
    #[instrument(skip(self))]
    pub fn add_dependency(&mut self, node_id: &str, depends_on: &str) -> Result<(), String> {
        // Verify both nodes exist.
        if !self.nodes.iter().any(|n| n.id == depends_on) {
            return Err(format!("Dependency node '{depends_on}' not found in DAG"));
        }
        let node = self
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("Node '{node_id}' not found in DAG"))?;

        if !node.depends_on.contains(&depends_on.to_string()) {
            node.depends_on.push(depends_on.to_string());
        }
        debug!(node_id, depends_on, "Dependency added");
        Ok(())
    }

    /// Return references to all nodes whose status is [`TaskStatus::Pending`]
    /// and whose dependencies have all reached [`TaskStatus::Completed`].
    #[instrument(skip(self))]
    pub fn ready_nodes(&self) -> Vec<&TaskNode> {
        let completed: HashSet<&str> = self
            .nodes
            .iter()
            .filter(|n| n.status == TaskStatus::Completed)
            .map(|n| n.id.as_str())
            .collect();

        self.nodes
            .iter()
            .filter(|n| {
                n.status == TaskStatus::Pending
                    && n.depends_on.iter().all(|dep| completed.contains(dep.as_str()))
            })
            .collect()
    }

    /// Transition a node to [`TaskStatus::Running`].
    #[instrument(skip(self))]
    pub fn mark_running(&mut self, node_id: &str) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.status = TaskStatus::Running;
            debug!(node_id, "Node marked running");
        }
    }

    /// Transition a node to [`TaskStatus::Completed`] and store its result.
    #[instrument(skip(self, result))]
    pub fn mark_completed(&mut self, node_id: &str, result: String) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.status = TaskStatus::Completed;
            node.result = Some(result);
            info!(node_id, "Node completed");
        }
    }

    /// Transition a node to [`TaskStatus::Failed`] and store the reason.
    #[instrument(skip(self, reason))]
    pub fn mark_failed(&mut self, node_id: &str, reason: String) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.status = TaskStatus::Failed;
            node.result = Some(reason);
            info!(node_id, "Node failed");
        }
    }

    /// Returns `true` when every node is [`TaskStatus::Completed`],
    /// [`TaskStatus::Failed`], or [`TaskStatus::Skipped`].
    #[instrument(skip(self))]
    pub fn is_complete(&self) -> bool {
        self.nodes.iter().all(|n| {
            matches!(
                n.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Skipped
            )
        })
    }

    /// Returns `(completed_count, total_count)`.
    #[instrument(skip(self))]
    pub fn progress(&self) -> (usize, usize) {
        let completed = self
            .nodes
            .iter()
            .filter(|n| n.status == TaskStatus::Completed)
            .count();
        (completed, self.nodes.len())
    }

    /// Validate the DAG has no cycles using Kahn's algorithm (topological sort).
    ///
    /// Returns `Ok(())` if the graph is acyclic, or an error describing the
    /// cycle.
    #[instrument(skip(self))]
    pub fn validate(&self) -> Result<(), String> {
        self.topological_order().map(|_| ())
    }

    /// Compute a valid topological execution order using Kahn's algorithm.
    ///
    /// Returns the ordered list of node IDs, or an error if a cycle is
    /// detected.
    #[instrument(skip(self))]
    pub fn topological_order(&self) -> Result<Vec<String>, String> {
        let node_ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();

        // Build adjacency + in-degree maps.
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for node in &self.nodes {
            in_degree.entry(node.id.as_str()).or_insert(0);
            for dep in &node.depends_on {
                if !node_ids.contains(dep.as_str()) {
                    return Err(format!(
                        "Node '{}' depends on '{}', which does not exist",
                        node.id, dep
                    ));
                }
                *in_degree.entry(node.id.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(node.id.as_str());
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut order: Vec<String> = Vec::with_capacity(self.nodes.len());

        while let Some(current) = queue.pop_front() {
            order.push(current.to_string());
            if let Some(deps) = dependents.get(current) {
                for &dep in deps {
                    if let Some(deg) = in_degree.get_mut(dep) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dep);
                        }
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            Err("Cycle detected in task DAG".to_string())
        } else {
            debug!(order = ?order, "Topological order computed");
            Ok(order)
        }
    }

    /// Generate a [Mermaid](https://mermaid.js.org/) diagram string for
    /// visualization.
    #[instrument(skip(self))]
    pub fn to_mermaid(&self) -> String {
        let mut lines = vec![format!("graph TD")];

        for node in &self.nodes {
            let label = node.description.replace('"', "'");
            lines.push(format!("    {}[\"{}\"]", node.id, label));
        }

        for node in &self.nodes {
            for dep in &node.depends_on {
                lines.push(format!("    {} --> {}", dep, node.id));
            }
        }

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Helpers for building nodes quickly
// ---------------------------------------------------------------------------

impl TaskNode {
    /// Convenience constructor with sensible defaults.
    pub fn new(id: &str, description: &str, action: TaskAction) -> Self {
        Self {
            id: id.to_string(),
            description: description.to_string(),
            action,
            depends_on: Vec::new(),
            status: TaskStatus::Pending,
            result: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dag() -> TaskDag {
        let mut dag = TaskDag::new("test plan");
        // Force a deterministic id for assertions.
        dag.id = "dag-1".to_string();

        dag.add_node(TaskNode::new("a", "Navigate to site", TaskAction::Navigate("https://example.com".into())));
        dag.add_node(TaskNode::new("b", "Search for info", TaskAction::Search("rust concurrency".into())));
        dag.add_node(TaskNode::new("c", "Extract results", TaskAction::Extract("main content".into())));

        dag.add_dependency("b", "a").unwrap();
        dag.add_dependency("c", "b").unwrap();

        dag
    }

    #[test]
    fn ready_nodes_returns_only_nodes_with_completed_deps() {
        let dag = sample_dag();
        let ready = dag.ready_nodes();
        // Only node "a" has no deps, so it is the only ready node.
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");
    }

    #[test]
    fn completing_a_node_makes_dependents_ready() {
        let mut dag = sample_dag();

        dag.mark_running("a");
        dag.mark_completed("a", "done".into());

        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "b");

        dag.mark_running("b");
        dag.mark_completed("b", "searched".into());

        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "c");
    }

    #[test]
    fn validate_detects_cycles() {
        let mut dag = TaskDag::new("cyclic");
        dag.add_node(TaskNode::new("x", "X", TaskAction::Custom("x".into())));
        dag.add_node(TaskNode::new("y", "Y", TaskAction::Custom("y".into())));
        dag.add_node(TaskNode::new("z", "Z", TaskAction::Custom("z".into())));

        dag.add_dependency("x", "z").unwrap();
        dag.add_dependency("y", "x").unwrap();
        dag.add_dependency("z", "y").unwrap();

        assert!(dag.validate().is_err());
        assert!(dag.validate().unwrap_err().contains("Cycle"));
    }

    #[test]
    fn topological_order_returns_valid_order() {
        let dag = sample_dag();
        let order = dag.topological_order().unwrap();

        // "a" must come before "b", "b" before "c".
        let pos_a = order.iter().position(|id| id == "a").unwrap();
        let pos_b = order.iter().position(|id| id == "b").unwrap();
        let pos_c = order.iter().position(|id| id == "c").unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn is_complete_when_all_terminal() {
        let mut dag = sample_dag();
        assert!(!dag.is_complete());

        dag.mark_running("a");
        dag.mark_completed("a", "ok".into());
        dag.mark_running("b");
        dag.mark_failed("b", "timeout".into());

        // "c" is still Pending.
        assert!(!dag.is_complete());

        // Skip "c".
        if let Some(node) = dag.nodes.iter_mut().find(|n| n.id == "c") {
            node.status = TaskStatus::Skipped;
        }

        assert!(dag.is_complete());
    }

    #[test]
    fn progress_counts_completed() {
        let mut dag = sample_dag();
        assert_eq!(dag.progress(), (0, 3));

        dag.mark_running("a");
        dag.mark_completed("a", "ok".into());
        assert_eq!(dag.progress(), (1, 3));

        dag.mark_running("b");
        dag.mark_completed("b", "ok".into());
        assert_eq!(dag.progress(), (2, 3));
    }

    #[test]
    fn to_mermaid_contains_node_descriptions() {
        let dag = sample_dag();
        let mermaid = dag.to_mermaid();

        assert!(mermaid.contains("graph TD"));
        assert!(mermaid.contains("Navigate to site"));
        assert!(mermaid.contains("Search for info"));
        assert!(mermaid.contains("Extract results"));
        assert!(mermaid.contains("a --> b"));
        assert!(mermaid.contains("b --> c"));
    }

    #[test]
    fn parallel_nodes_are_all_ready() {
        let mut dag = TaskDag::new("parallel test");
        dag.add_node(TaskNode::new("root", "Start", TaskAction::Navigate("https://example.com".into())));
        dag.add_node(TaskNode::new("p1", "Parallel 1", TaskAction::Search("q1".into())));
        dag.add_node(TaskNode::new("p2", "Parallel 2", TaskAction::Search("q2".into())));
        dag.add_node(TaskNode::new("p3", "Parallel 3", TaskAction::Search("q3".into())));

        dag.add_dependency("p1", "root").unwrap();
        dag.add_dependency("p2", "root").unwrap();
        dag.add_dependency("p3", "root").unwrap();

        dag.mark_running("root");
        dag.mark_completed("root", "ok".into());

        let ready = dag.ready_nodes();
        assert_eq!(ready.len(), 3);
    }

    #[test]
    fn add_dependency_rejects_unknown_nodes() {
        let mut dag = TaskDag::new("test");
        dag.add_node(TaskNode::new("a", "A", TaskAction::Custom("a".into())));

        assert!(dag.add_dependency("a", "missing").is_err());
        assert!(dag.add_dependency("missing", "a").is_err());
    }
}
