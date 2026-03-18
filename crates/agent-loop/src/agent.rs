//! The main Agent struct — drives the observe→think→act loop.

use crate::{AgentConfig, BrowsingTask, error::AgentError, history::StepHistory, llm::LlmBackend};
use openclaw_browser_core::BrowserSession;
use openclaw_browser_core::actions::{BrowserAction, ScrollDirection};
use tracing::{info, warn, debug, instrument};

/// A parsed action from the LLM response.
#[derive(Debug, Clone)]
pub enum ParsedAction {
    Navigate(String),
    Click(u32),
    Fill(u32, String),
    Select(u32, String),
    ScrollDown(i32),
    ScrollUp(i32),
    Search(String),
    Extract,
    Screenshot,
    KeyPress(String),
    Back,
    Done(String),
    Fail(String),
}

/// The AI browsing agent. Owns a browser session and drives tasks to completion.
pub struct Agent<L: LlmBackend> {
    pub config: AgentConfig,
    pub session: BrowserSession,
    pub llm: L,
    pub history: StepHistory,
}

impl<L: LlmBackend> Agent<L> {
    pub fn new(config: AgentConfig, session: BrowserSession, llm: L) -> Self {
        Self {
            config,
            session,
            llm,
            history: StepHistory::new(),
        }
    }

    /// Run a browsing task to completion.
    #[instrument(skip(self), fields(task = %task.description))]
    pub async fn run(&mut self, task: BrowsingTask) -> Result<String, AgentError> {
        info!(
            task = %task.description,
            max_steps = self.config.max_steps,
            "Starting browsing task"
        );

        // Navigate to start URL if provided
        if let Some(ref url) = task.start_url {
            info!(url = %url, "Navigating to start URL");
            self.session.new_tab(url).await
                .map_err(AgentError::Browser)?;
        }

        for step in 0..self.config.max_steps {
            info!(step, total_steps = self.config.max_steps, "Agent step");

            // 1. OBSERVE — snapshot the current page state
            let tab = self.session.active_tab().await
                .map_err(AgentError::Browser)?;
            let snapshot = tab.snapshot().await
                .map_err(AgentError::Browser)?;

            let observation = snapshot.to_agent_text();
            debug!(step, tokens = snapshot.estimated_tokens(), "Observation captured");

            // 2. THINK — ask the LLM what to do next
            self.history.add_observation(step, &observation);

            // Budget check
            let total_tokens = self.history.estimated_tokens();
            if total_tokens > self.config.max_context_tokens {
                self.history.trim_to_budget(self.config.max_context_tokens);
                debug!(
                    trimmed_to = self.history.estimated_tokens(),
                    "History trimmed to fit context budget"
                );
            }

            let messages = self.history.to_messages(&self.config.system_prompt, &task.description);
            let response = self.llm.complete(&messages, &self.config.model).await?;
            debug!(step, response_len = response.len(), "LLM response received");

            // 3. ACT — parse and execute the action
            self.history.add_action(step, &response);

            let action = parse_action(&response);
            match action {
                Some(ParsedAction::Done(result)) => {
                    info!(step, result = %result, "Task completed successfully");
                    return Ok(result);
                }
                Some(ParsedAction::Fail(reason)) => {
                    warn!(step, reason = %reason, "Task failed");
                    return Ok(format!("FAILED: {reason}"));
                }
                Some(action) => {
                    debug!(step, action = ?action, "Executing action");
                    if let Err(e) = self.execute_action(&action).await {
                        warn!(step, error = %e, action = ?action, "Action execution failed");
                        self.history.add_observation(
                            step,
                            &format!("[Error executing action: {e}]"),
                        );
                    }
                }
                None => {
                    warn!(step, "No action found in LLM response");
                    self.history.add_observation(
                        step,
                        "[No valid ACTION: found in your response. Please end with ACTION: <action>]",
                    );
                }
            }
        }

        Err(AgentError::MaxStepsExceeded {
            max_steps: self.config.max_steps,
        })
    }

    /// Execute a parsed action against the browser.
    async fn execute_action(&mut self, action: &ParsedAction) -> Result<(), AgentError> {
        let mut tab = self.session.active_tab().await
            .map_err(AgentError::Browser)?;

        match action {
            ParsedAction::Navigate(url) => {
                tab.navigate(url).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Click(ref_id) => {
                tab.execute(BrowserAction::Click { ref_id: *ref_id }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Fill(ref_id, text) => {
                tab.execute(BrowserAction::Fill {
                    ref_id: *ref_id,
                    text: text.clone(),
                }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Select(ref_id, value) => {
                tab.execute(BrowserAction::Select {
                    ref_id: *ref_id,
                    value: value.clone(),
                }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::ScrollDown(amount) => {
                tab.execute(BrowserAction::Scroll {
                    direction: ScrollDirection::Down,
                    amount: *amount,
                }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::ScrollUp(amount) => {
                tab.execute(BrowserAction::Scroll {
                    direction: ScrollDirection::Up,
                    amount: *amount,
                }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::KeyPress(key) => {
                tab.execute(BrowserAction::KeyPress {
                    key: key.clone(),
                }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Back => {
                tab.execute(BrowserAction::GoBack).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Screenshot => {
                tab.execute(BrowserAction::Screenshot { full_page: false }).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Extract => {
                tab.execute(BrowserAction::ExtractContent).await
                    .map_err(AgentError::Browser)?;
            }
            ParsedAction::Search(_) => {
                // Search is handled at orchestrator level, not browser action
                debug!(action = ?action, "Search action (handled by orchestrator)");
            }
            ParsedAction::Done(_) | ParsedAction::Fail(_) => {
                // Terminal actions — handled by the caller
            }
        }

        Ok(())
    }
}

/// Parse an action from the LLM response text.
///
/// Looks for lines starting with `ACTION:` (case-insensitive, last one wins).
/// Supports formats:
/// - `ACTION: navigate https://example.com`
/// - `ACTION: click @e5`
/// - `ACTION: fill @e3 "hello world"`
/// - `ACTION: scroll down 500`
/// - `ACTION: done "the answer is 42"`
pub fn parse_action(response: &str) -> Option<ParsedAction> {
    // Find the last ACTION: line (the LLM might have reasoning before it)
    let action_line = response
        .lines()
        .rev()
        .find(|line| {
            let trimmed = line.trim().to_lowercase();
            trimmed.starts_with("action:")
        })?;

    let action_str = action_line
        .trim().split_once(':')?.1
        .trim();

    parse_action_str(action_str)
}

/// Parse the action string after "ACTION: ".
fn parse_action_str(s: &str) -> Option<ParsedAction> {
    let s = s.trim();

    // done "result"
    if let Some(rest) = strip_prefix_ci(s, "done ") {
        return Some(ParsedAction::Done(unquote(rest)));
    }

    // fail "reason"
    if let Some(rest) = strip_prefix_ci(s, "fail ") {
        return Some(ParsedAction::Fail(unquote(rest)));
    }

    // navigate <url>
    if let Some(url) = strip_prefix_ci(s, "navigate ") {
        return Some(ParsedAction::Navigate(url.trim().to_string()));
    }

    // click @e<N>
    if let Some(rest) = strip_prefix_ci(s, "click ") {
        if let Some(ref_id) = parse_ref_id(rest.trim()) {
            return Some(ParsedAction::Click(ref_id));
        }
    }

    // fill @e<N> "text"
    if let Some(rest) = strip_prefix_ci(s, "fill ") {
        let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            if let Some(ref_id) = parse_ref_id(parts[0]) {
                return Some(ParsedAction::Fill(ref_id, unquote(parts[1])));
            }
        }
    }

    // select @e<N> "value"
    if let Some(rest) = strip_prefix_ci(s, "select ") {
        let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            if let Some(ref_id) = parse_ref_id(parts[0]) {
                return Some(ParsedAction::Select(ref_id, unquote(parts[1])));
            }
        }
    }

    // scroll down/up <amount>
    if let Some(rest) = strip_prefix_ci(s, "scroll ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let direction = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();
        let amount: i32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(500);
        match direction.as_str() {
            "down" => return Some(ParsedAction::ScrollDown(amount)),
            "up" => return Some(ParsedAction::ScrollUp(amount)),
            _ => {}
        }
    }

    // search "query"
    if let Some(rest) = strip_prefix_ci(s, "search ") {
        return Some(ParsedAction::Search(unquote(rest)));
    }

    // extract
    if s.eq_ignore_ascii_case("extract") {
        return Some(ParsedAction::Extract);
    }

    // screenshot
    if s.eq_ignore_ascii_case("screenshot") {
        return Some(ParsedAction::Screenshot);
    }

    // back
    if s.eq_ignore_ascii_case("back") {
        return Some(ParsedAction::Back);
    }

    // key <key>
    if let Some(rest) = strip_prefix_ci(s, "key ") {
        return Some(ParsedAction::KeyPress(rest.trim().to_string()));
    }

    None
}

/// Case-insensitive prefix strip.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Parse a ref ID from "@e5" or "@e12" or just "5".
fn parse_ref_id(s: &str) -> Option<u32> {
    let s = s.trim();
    let num_str = if let Some(rest) = s.strip_prefix("@e") {
        rest
    } else if let Some(rest) = s.strip_prefix("@E") {
        rest
    } else {
        s
    };
    num_str.parse().ok()
}

/// Remove surrounding quotes from a string.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_navigate() {
        let action = parse_action("Let me go to the page.\nACTION: navigate https://example.com").unwrap();
        assert!(matches!(action, ParsedAction::Navigate(url) if url == "https://example.com"));
    }

    #[test]
    fn parse_click() {
        let action = parse_action("ACTION: click @e5").unwrap();
        assert!(matches!(action, ParsedAction::Click(5)));
    }

    #[test]
    fn parse_click_bare_number() {
        let action = parse_action("ACTION: click 12").unwrap();
        assert!(matches!(action, ParsedAction::Click(12)));
    }

    #[test]
    fn parse_fill() {
        let action = parse_action(r#"ACTION: fill @e3 "hello world""#).unwrap();
        assert!(matches!(action, ParsedAction::Fill(3, ref text) if text == "hello world"));
    }

    #[test]
    fn parse_scroll_down() {
        let action = parse_action("ACTION: scroll down 300").unwrap();
        assert!(matches!(action, ParsedAction::ScrollDown(300)));
    }

    #[test]
    fn parse_scroll_default() {
        let action = parse_action("ACTION: scroll down").unwrap();
        assert!(matches!(action, ParsedAction::ScrollDown(500)));
    }

    #[test]
    fn parse_done() {
        let action = parse_action("I found the answer.\nACTION: done \"the price is $42\"").unwrap();
        assert!(matches!(action, ParsedAction::Done(ref r) if r == "the price is $42"));
    }

    #[test]
    fn parse_fail() {
        let action = parse_action(r#"ACTION: fail "page requires login""#).unwrap();
        assert!(matches!(action, ParsedAction::Fail(ref r) if r == "page requires login"));
    }

    #[test]
    fn parse_extract() {
        let action = parse_action("ACTION: extract").unwrap();
        assert!(matches!(action, ParsedAction::Extract));
    }

    #[test]
    fn parse_search() {
        let action = parse_action(r#"ACTION: search "rust web browser""#).unwrap();
        assert!(matches!(action, ParsedAction::Search(ref q) if q == "rust web browser"));
    }

    #[test]
    fn parse_back() {
        let action = parse_action("ACTION: back").unwrap();
        assert!(matches!(action, ParsedAction::Back));
    }

    #[test]
    fn parse_screenshot() {
        let action = parse_action("ACTION: screenshot").unwrap();
        assert!(matches!(action, ParsedAction::Screenshot));
    }

    #[test]
    fn parse_last_action_wins() {
        let response = "I'll click the button.\nACTION: click @e1\nWait, actually:\nACTION: click @e7";
        let action = parse_action(response).unwrap();
        assert!(matches!(action, ParsedAction::Click(7)));
    }

    #[test]
    fn parse_case_insensitive() {
        let action = parse_action("action: Navigate https://test.com").unwrap();
        assert!(matches!(action, ParsedAction::Navigate(url) if url == "https://test.com"));
    }

    #[test]
    fn no_action_returns_none() {
        assert!(parse_action("Just some thinking, no action here").is_none());
    }

    #[test]
    fn parse_select() {
        let action = parse_action(r#"ACTION: select @e8 "option-2""#).unwrap();
        assert!(matches!(action, ParsedAction::Select(8, ref v) if v == "option-2"));
    }

    #[test]
    fn parse_key() {
        let action = parse_action("ACTION: key Enter").unwrap();
        assert!(matches!(action, ParsedAction::KeyPress(ref k) if k == "Enter"));
    }
}
