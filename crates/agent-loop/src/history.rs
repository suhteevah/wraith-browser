//! Step history — tracks the observation→action pairs for the agent's conversation.
//! Includes token budgeting to keep context within LLM limits.

use crate::llm::{Message, MessageRole};

pub struct StepHistory {
    steps: Vec<Step>,
}

struct Step {
    step_num: usize,
    observation: String,
    action: Option<String>,
}

impl Default for StepHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl StepHistory {
    pub fn new() -> Self {
        Self { steps: vec![] }
    }

    pub fn add_observation(&mut self, step: usize, observation: &str) {
        // If this step already exists (e.g., error feedback), append to observation
        if let Some(_s) = self.steps.iter_mut().find(|s| s.step_num == step && s.action.is_some()) {
            // This is a follow-up observation for the same step (e.g., error)
            self.steps.push(Step {
                step_num: step,
                observation: observation.to_string(),
                action: None,
            });
            return;
        }

        if let Some(s) = self.steps.iter_mut().find(|s| s.step_num == step && s.action.is_none()) {
            s.observation.push('\n');
            s.observation.push_str(observation);
        } else {
            self.steps.push(Step {
                step_num: step,
                observation: observation.to_string(),
                action: None,
            });
        }
    }

    pub fn add_action(&mut self, step: usize, action: &str) {
        if let Some(s) = self.steps.iter_mut().rev().find(|s| s.step_num == step) {
            s.action = Some(action.to_string());
        }
    }

    /// Estimated total tokens across all steps.
    /// Uses the ~4 chars/token heuristic.
    pub fn estimated_tokens(&self) -> usize {
        self.steps.iter().map(|s| {
            let obs_tokens = s.observation.len() / 4;
            let act_tokens = s.action.as_ref().map(|a| a.len() / 4).unwrap_or(0);
            obs_tokens + act_tokens
        }).sum()
    }

    /// Trim oldest steps to fit within a token budget.
    /// Always keeps the first step (initial context) and the last N steps.
    pub fn trim_to_budget(&mut self, max_tokens: usize) {
        if self.estimated_tokens() <= max_tokens {
            return;
        }

        // Keep removing the second-oldest step (preserve first for context)
        while self.steps.len() > 2 && self.estimated_tokens() > max_tokens {
            self.steps.remove(1);
        }

        // If still over budget after removing middle steps, truncate old observations
        if self.estimated_tokens() > max_tokens && !self.steps.is_empty() {
            let max_obs_chars = max_tokens * 4 / self.steps.len();
            for step in &mut self.steps {
                if step.observation.len() > max_obs_chars {
                    step.observation.truncate(max_obs_chars);
                    step.observation.push_str("\n[... truncated to fit context window ...]");
                }
            }
        }
    }

    /// Convert history to LLM message format.
    pub fn to_messages(&self, system_prompt: &str, task: &str) -> Vec<Message> {
        let mut messages = vec![
            Message {
                role: MessageRole::System,
                content: system_prompt.to_string(),
            },
            Message {
                role: MessageRole::User,
                content: format!("Task: {task}"),
            },
        ];

        for step in &self.steps {
            messages.push(Message {
                role: MessageRole::User,
                content: format!("[Step {}] Observation:\n{}", step.step_num, step.observation),
            });
            if let Some(action) = &step.action {
                messages.push(Message {
                    role: MessageRole::Assistant,
                    content: action.clone(),
                });
            }
        }

        messages
    }

    /// Number of steps recorded.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_history() {
        let mut h = StepHistory::new();
        h.add_observation(0, "Page loaded");
        h.add_action(0, "ACTION: click @e1");
        h.add_observation(1, "Button clicked");

        let messages = h.to_messages("system", "do something");
        // system + task + step0 obs + step0 action + step1 obs = 5
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].role, MessageRole::System);
        assert_eq!(messages[1].role, MessageRole::User);
        assert_eq!(messages[2].role, MessageRole::User); // step 0 observation
        assert_eq!(messages[3].role, MessageRole::Assistant); // step 0 action
        assert_eq!(messages[4].role, MessageRole::User); // step 1 observation
    }

    #[test]
    fn test_token_estimation() {
        let mut h = StepHistory::new();
        // 100 chars ≈ 25 tokens
        h.add_observation(0, &"x".repeat(100));
        assert_eq!(h.estimated_tokens(), 25);
    }

    #[test]
    fn test_trim_to_budget() {
        let mut h = StepHistory::new();
        for i in 0..10 {
            h.add_observation(i, &"observation text here ".repeat(20));
            h.add_action(i, "ACTION: click @e1");
        }

        let before = h.estimated_tokens();
        assert!(before > 100);

        h.trim_to_budget(100);
        assert!(h.estimated_tokens() <= before);
        // Should have fewer steps
        assert!(h.step_count() < 10);
    }
}
