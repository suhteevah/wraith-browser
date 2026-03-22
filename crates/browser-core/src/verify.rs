//! Post-submission success verification.
//!
//! After a form submission (e.g. job application), inspect the resulting page
//! snapshot and URL to determine whether the submission succeeded.

use regex::Regex;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Outcome of a submission verification check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// Strong evidence the submission succeeded (URL or text match).
    Confirmed,
    /// The URL changed after submit — probably succeeded.
    Likely,
    /// No signal either way.
    Uncertain,
    /// Error signals detected on the page.
    Failed(String),
}

/// Details extracted from a confirmation page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationDetails {
    /// A confirmation/application ID if one was found.
    pub confirmation_id: Option<String>,
    /// The primary confirmation message shown on the page.
    pub message: String,
    /// Any "next steps" text (e.g. "we'll email you within 5 days").
    pub next_steps: Option<String>,
}

// ---------------------------------------------------------------------------
// Core verification
// ---------------------------------------------------------------------------

/// Success-signal keywords that may appear in a post-submit URL.
const URL_SUCCESS_KEYWORDS: &[&str] = &[
    "thank", "confirm", "success", "received", "complete",
];

/// Phrases in the page text that strongly indicate success.
const TEXT_SUCCESS_PHRASES: &[&str] = &[
    "application received",
    "thank you for applying",
    "we'll be in touch",
    "application submitted",
    "successfully submitted",
];

/// Error-signal keywords in the page text.
const TEXT_ERROR_KEYWORDS: &[&str] = &[
    "error",
    "required field",
    "please fill",
    "invalid",
];

/// Inspect the post-submission page snapshot and URL to decide whether the
/// submission succeeded.
///
/// * `snapshot_text` — visible text of the page after clicking submit.
/// * `url`          — current page URL after the submit action.
/// * `previous_url` — page URL *before* the submit action.
pub fn verify_submission(
    snapshot_text: &str,
    url: &str,
    previous_url: &str,
) -> VerificationResult {
    let lower_url = url.to_lowercase();
    let lower_text = snapshot_text.to_lowercase();

    // 1. Check for error signals first — they take priority.
    for kw in TEXT_ERROR_KEYWORDS {
        if lower_text.contains(kw) {
            return VerificationResult::Failed(format!(
                "Page contains error keyword: \"{}\"",
                kw,
            ));
        }
    }

    // 2. URL contains a success keyword → Confirmed.
    for kw in URL_SUCCESS_KEYWORDS {
        if lower_url.contains(kw) {
            return VerificationResult::Confirmed;
        }
    }

    // 3. Page text contains a success phrase → Confirmed.
    for phrase in TEXT_SUCCESS_PHRASES {
        if lower_text.contains(phrase) {
            return VerificationResult::Confirmed;
        }
    }

    // 4. URL changed at all → Likely.
    if url != previous_url {
        return VerificationResult::Likely;
    }

    // 5. Nothing detected.
    VerificationResult::Uncertain
}

// ---------------------------------------------------------------------------
// Confirmation detail extraction
// ---------------------------------------------------------------------------

/// Try to pull structured confirmation details out of the visible page text.
pub fn extract_confirmation_details(snapshot_text: &str) -> Option<ConfirmationDetails> {
    let lower = snapshot_text.to_lowercase();

    // Only attempt extraction when the page looks like a confirmation.
    let has_confirmation_signal = TEXT_SUCCESS_PHRASES
        .iter()
        .any(|p| lower.contains(p))
        || lower.contains("confirmation")
        || lower.contains("thank you");

    if !has_confirmation_signal {
        return None;
    }

    let confirmation_id = extract_confirmation_id(snapshot_text);
    let message = extract_confirmation_message(snapshot_text);
    let next_steps = extract_next_steps(snapshot_text);

    Some(ConfirmationDetails {
        confirmation_id,
        message,
        next_steps,
    })
}

/// Look for a confirmation/application ID in the text.
fn extract_confirmation_id(text: &str) -> Option<String> {
    // Patterns like "Confirmation #12345", "Application ID: ABC-789", "Ref: 00042"
    let patterns = [
        r"(?i)(?:confirmation|application|reference|ref)[\s#:]+([A-Za-z0-9\-]{3,})",
        r"(?i)(?:id|number)[\s#:]+([A-Za-z0-9\-]{3,})",
    ];
    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    return Some(m.as_str().to_string());
                }
            }
        }
    }
    None
}

/// Extract the primary confirmation message — first sentence that looks like a
/// success statement, or fall back to "Submission confirmed".
fn extract_confirmation_message(text: &str) -> String {
    let lower = text.to_lowercase();
    for phrase in TEXT_SUCCESS_PHRASES {
        if let Some(start) = lower.find(phrase) {
            // Grab the sentence surrounding the match.
            let sentence_start = text[..start]
                .rfind(|c: char| c == '.' || c == '\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let after = start + phrase.len();
            let sentence_end = text[after..]
                .find(|c: char| c == '.' || c == '\n')
                .map(|i| after + i + 1)
                .unwrap_or(text.len());
            let sentence = text[sentence_start..sentence_end].trim();
            if !sentence.is_empty() {
                return sentence.to_string();
            }
        }
    }
    "Submission confirmed".to_string()
}

/// Look for "next steps" / response-time info.
fn extract_next_steps(text: &str) -> Option<String> {
    let patterns = [
        r"(?i)((?:you will|we.ll|expect to)\s.{10,80})",
        r"(?i)((?:within|in)\s+\d+\s+(?:business\s+)?(?:days?|hours?|weeks?).{0,40})",
        r"(?i)(next\s+steps?\s*[:.]?\s*.{10,80})",
    ];
    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    let s = m.as_str().trim().to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Retry logic
// ---------------------------------------------------------------------------

/// Decide whether the submission should be retried.
///
/// * `Failed` → retry if `attempt < max_attempts`
/// * `Uncertain` → retry at most once (i.e. `attempt < 1`)
/// * `Confirmed` / `Likely` → never retry
pub fn should_retry(result: &VerificationResult, attempt: u32, max_attempts: u32) -> bool {
    match result {
        VerificationResult::Failed(_) => attempt < max_attempts,
        VerificationResult::Uncertain => attempt < 1,
        VerificationResult::Confirmed | VerificationResult::Likely => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- verify_submission ---------------------------------------------------

    #[test]
    fn confirmed_by_url_keyword() {
        let result = verify_submission(
            "Some page content",
            "https://jobs.example.com/thank-you",
            "https://jobs.example.com/apply",
        );
        assert_eq!(result, VerificationResult::Confirmed);
    }

    #[test]
    fn confirmed_by_url_success() {
        let result = verify_submission(
            "Your profile",
            "https://example.com/application/success",
            "https://example.com/application/form",
        );
        assert_eq!(result, VerificationResult::Confirmed);
    }

    #[test]
    fn confirmed_by_text_phrase() {
        let result = verify_submission(
            "Thank you for applying! We'll review your application shortly.",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert_eq!(result, VerificationResult::Confirmed);
    }

    #[test]
    fn confirmed_application_received() {
        let result = verify_submission(
            "Your application received. We'll be in touch within 5 business days.",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert_eq!(result, VerificationResult::Confirmed);
    }

    #[test]
    fn confirmed_successfully_submitted() {
        let result = verify_submission(
            "Your resume has been successfully submitted to the hiring team.",
            "https://example.com/careers",
            "https://example.com/careers",
        );
        assert_eq!(result, VerificationResult::Confirmed);
    }

    #[test]
    fn likely_url_changed() {
        let result = verify_submission(
            "Welcome to your dashboard",
            "https://example.com/dashboard",
            "https://example.com/apply",
        );
        assert_eq!(result, VerificationResult::Likely);
    }

    #[test]
    fn failed_error_keyword() {
        let result = verify_submission(
            "Error: could not process your application",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert!(matches!(result, VerificationResult::Failed(_)));
        if let VerificationResult::Failed(msg) = result {
            assert!(msg.contains("error"));
        }
    }

    #[test]
    fn failed_required_field() {
        let result = verify_submission(
            "Please complete the required field: Email",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert!(matches!(result, VerificationResult::Failed(_)));
    }

    #[test]
    fn failed_invalid() {
        let result = verify_submission(
            "Invalid phone number format",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert!(matches!(result, VerificationResult::Failed(_)));
    }

    #[test]
    fn uncertain_no_change() {
        let result = verify_submission(
            "Fill out the form below to apply.",
            "https://example.com/apply",
            "https://example.com/apply",
        );
        assert_eq!(result, VerificationResult::Uncertain);
    }

    // -- extract_confirmation_details ----------------------------------------

    #[test]
    fn extract_details_with_id_and_next_steps() {
        let text = "Thank you for applying! \
                     Your confirmation #APP-12345 has been recorded. \
                     You will hear from us within 5 business days.";
        let details = extract_confirmation_details(text).unwrap();
        assert_eq!(details.confirmation_id.as_deref(), Some("APP-12345"));
        assert!(details.message.contains("Thank you for applying"));
        assert!(details.next_steps.is_some());
    }

    #[test]
    fn extract_details_application_id() {
        let text = "Application submitted. Application ID: JOB-9921. \
                     We'll be in touch soon.";
        let details = extract_confirmation_details(text).unwrap();
        assert_eq!(details.confirmation_id.as_deref(), Some("JOB-9921"));
    }

    #[test]
    fn extract_details_no_id() {
        let text = "Thank you for applying! We have received your application.";
        let details = extract_confirmation_details(text).unwrap();
        assert!(details.confirmation_id.is_none());
        assert!(details.message.contains("Thank you for applying"));
    }

    #[test]
    fn extract_details_no_confirmation_signal() {
        let text = "Please fill out the form below.";
        assert!(extract_confirmation_details(text).is_none());
    }

    #[test]
    fn extract_details_next_steps() {
        let text = "Application received. Next steps: a recruiter will contact you \
                     within 3 business days.";
        let details = extract_confirmation_details(text).unwrap();
        assert!(details.next_steps.is_some());
        let ns = details.next_steps.unwrap();
        assert!(ns.contains("recruiter") || ns.contains("3 business days"));
    }

    // -- should_retry --------------------------------------------------------

    #[test]
    fn retry_on_failure_under_max() {
        let r = VerificationResult::Failed("error".into());
        assert!(should_retry(&r, 0, 3));
        assert!(should_retry(&r, 2, 3));
        assert!(!should_retry(&r, 3, 3));
    }

    #[test]
    fn retry_uncertain_once() {
        let r = VerificationResult::Uncertain;
        assert!(should_retry(&r, 0, 5));
        assert!(!should_retry(&r, 1, 5));
    }

    #[test]
    fn no_retry_confirmed() {
        assert!(!should_retry(&VerificationResult::Confirmed, 0, 5));
    }

    #[test]
    fn no_retry_likely() {
        assert!(!should_retry(&VerificationResult::Likely, 0, 5));
    }
}
