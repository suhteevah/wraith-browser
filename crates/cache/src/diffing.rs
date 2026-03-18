//! Semantic page diffing engine.
//!
//! Compares two versions of a page to detect meaningful changes while ignoring
//! noise (ads, timestamps, random IDs, tracking pixels). The agent uses this to
//! decide whether a page actually changed in ways that matter — a price drop,
//! a status flip, new content sections — rather than just "the HTML is different."
//!
//! Key design choices:
//! - Similarity is computed via Jaccard index on word-level trigrams, which is
//!   robust against minor rewordings and reorderings.
//! - Noise stripping runs before any comparison so ad rotation and cookie banners
//!   don't trigger false positives.
//! - Specific detectors (price, date, status) produce typed `DiffChange` variants
//!   so downstream consumers can react precisely (e.g., alert on price drops).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The result of comparing two page versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDiff {
    /// URL of the page that was diffed.
    pub url: String,

    /// Content hash of the old version.
    pub old_hash: String,

    /// Content hash of the new version.
    pub new_hash: String,

    /// Similarity score: 0.0 = completely different, 1.0 = identical.
    pub similarity_score: f64,

    /// Individual changes detected between the two versions.
    pub changes: Vec<DiffChange>,

    /// Human-readable summary, e.g. "Price dropped from $99 to $79".
    pub summary: String,

    /// When the diff was computed.
    pub diffed_at: DateTime<Utc>,
}

/// A single semantic change between two page versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffChange {
    /// What kind of change this is.
    pub kind: ChangeKind,

    /// Heading or context section where the change occurred.
    pub section: String,

    /// The old text (absent for `Added`).
    pub old_text: Option<String>,

    /// The new text (absent for `Removed`).
    pub new_text: Option<String>,
}

/// Classification of a detected change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeKind {
    /// New content that wasn't present before.
    Added,
    /// Content that was present before but is now gone.
    Removed,
    /// Content that was reworded or updated.
    Modified,
    /// A price value changed.
    PriceChange,
    /// A date value changed.
    DateChange,
    /// A status indicator flipped (e.g. "in stock" -> "out of stock").
    StatusChange,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compare two page versions and produce a semantic diff.
///
/// Both `old_text` and `new_text` should be the extracted page text (not raw
/// HTML). The function normalises both, computes similarity, detects typed
/// changes, and generates a human-readable summary.
#[instrument(skip(old_text, new_text))]
pub fn diff_pages(url: &str, old_text: &str, new_text: &str) -> PageDiff {
    let norm_old = normalize_text(old_text);
    let norm_new = normalize_text(new_text);

    let similarity_score = compute_similarity(&norm_old, &norm_new);
    debug!(similarity_score, "computed similarity");

    let old_hash = simple_hash(&norm_old);
    let new_hash = simple_hash(&norm_new);

    let mut changes = Vec::new();

    // Line-level diff
    detect_line_changes(&norm_old, &norm_new, &mut changes);

    // Typed detectors run on the *original* (un-normalised) text so that
    // casing and formatting are preserved for display.
    detect_price_changes(old_text, new_text, &mut changes);
    detect_date_changes(old_text, new_text, &mut changes);
    detect_status_changes(old_text, new_text, &mut changes);

    let summary = generate_summary(&changes);
    debug!(summary = %summary, change_count = changes.len(), "diff complete");

    PageDiff {
        url: url.to_string(),
        old_hash,
        new_hash,
        similarity_score,
        changes,
        summary,
        diffed_at: Utc::now(),
    }
}

/// Normalise page text for comparison by stripping noise.
///
/// Removes timestamps, counters, ad/tracking boilerplate, collapses whitespace,
/// and lowercases everything.
#[instrument(skip(text))]
pub fn normalize_text(text: &str) -> String {
    let noise_patterns: &[&str] = &[
        "advertisement",
        "sponsored",
        "cookie",
        "accept cookies",
        "privacy policy",
        "terms of service",
        "subscribe to our newsletter",
        "sign up for our",
        "click here to",
        "ad_",
        "tracking",
        "utm_",
        "powered by",
    ];

    let timestamp_re =
        Regex::new(r"^\s*\d{1,2}:\d{2}(:\d{2})?\s*(am|pm|AM|PM)?\s*$").expect("valid regex");
    let date_only_re = Regex::new(
        r"^\s*\d{1,4}[-/]\d{1,2}[-/]\d{1,4}\s*$",
    )
    .expect("valid regex");
    let numbers_only_re = Regex::new(r"^\s*\d+\s*$").expect("valid regex");

    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());

    for line in &lines {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Skip lines that are only timestamps
        if timestamp_re.is_match(trimmed) {
            continue;
        }

        // Skip lines that are only dates
        if date_only_re.is_match(trimmed) {
            continue;
        }

        // Skip lines that are only numbers (counters, IDs)
        if numbers_only_re.is_match(trimmed) {
            continue;
        }

        // Skip lines containing noise patterns
        let lower = trimmed.to_lowercase();
        if noise_patterns.iter().any(|p| lower.contains(p)) {
            continue;
        }

        kept.push(trimmed);
    }

    // Join, collapse whitespace, lowercase
    let joined = kept.join(" ");
    let collapsed = collapse_whitespace(&joined);
    collapsed.to_lowercase()
}

/// Compute Jaccard similarity on word-level trigrams.
///
/// Splits each text into overlapping 3-word windows, then computes
/// `|intersection| / |union|`. Returns 1.0 for identical texts and
/// approaches 0.0 for completely unrelated texts.
#[instrument(skip(a, b))]
pub fn compute_similarity(a: &str, b: &str) -> f64 {
    let trigrams_a = word_trigrams(a);
    let trigrams_b = word_trigrams(b);

    if trigrams_a.is_empty() && trigrams_b.is_empty() {
        return 1.0;
    }

    let intersection = trigrams_a.intersection(&trigrams_b).count();
    let union = trigrams_a.union(&trigrams_b).count();

    if union == 0 {
        return 1.0;
    }

    let score = intersection as f64 / union as f64;
    debug!(intersection, union, score, "jaccard similarity");
    score
}

/// Produce a human-readable summary from a set of detected changes.
///
/// Prioritises specific change types (price, status) over generic counts.
#[instrument(skip(changes))]
pub fn generate_summary(changes: &[DiffChange]) -> String {
    if changes.is_empty() {
        return "No meaningful changes detected".to_string();
    }

    let mut parts: Vec<String> = Vec::new();

    // Price changes
    let price_changes: Vec<&DiffChange> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::PriceChange)
        .collect();
    for pc in &price_changes {
        if let (Some(old), Some(new)) = (&pc.old_text, &pc.new_text) {
            parts.push(format!("Price changed from {} to {}", old, new));
        }
    }

    // Status changes
    let status_changes: Vec<&DiffChange> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::StatusChange)
        .collect();
    for sc in &status_changes {
        if let (Some(old), Some(new)) = (&sc.old_text, &sc.new_text) {
            parts.push(format!("Status changed: {} \u{2192} {}", old, new));
        }
    }

    // Date changes
    let date_changes: Vec<&DiffChange> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::DateChange)
        .collect();
    if !date_changes.is_empty() {
        parts.push(format!(
            "{} date change{} detected",
            date_changes.len(),
            if date_changes.len() == 1 { "" } else { "s" }
        ));
    }

    // Generic counts for remaining types
    if parts.is_empty() || changes.len() > price_changes.len() + status_changes.len() + date_changes.len() {
        let added = changes.iter().filter(|c| c.kind == ChangeKind::Added).count();
        let removed = changes.iter().filter(|c| c.kind == ChangeKind::Removed).count();
        let modified = changes.iter().filter(|c| c.kind == ChangeKind::Modified).count();

        if added + removed + modified > 0 {
            parts.push(format!(
                "{} sections modified, {} added, {} removed",
                modified, added, removed
            ));
        }
    }

    if parts.is_empty() {
        return "Minor changes detected".to_string();
    }

    parts.join("; ")
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a set of word-level trigrams (3-word sliding windows).
fn word_trigrams(text: &str) -> HashSet<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut set = HashSet::new();
    if words.len() < 3 {
        // For very short texts, use whatever words we have as a single gram
        if !words.is_empty() {
            set.insert(words.join(" "));
        }
        return set;
    }
    for window in words.windows(3) {
        set.insert(window.join(" "));
    }
    set
}

/// Collapse runs of whitespace into a single space.
fn collapse_whitespace(s: &str) -> String {
    let ws_re = Regex::new(r"\s+").expect("valid regex");
    ws_re.replace_all(s.trim(), " ").to_string()
}

/// Cheap non-cryptographic hash for content fingerprinting.
fn simple_hash(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Detect line-level additions, removals, and modifications.
fn detect_line_changes(old_norm: &str, new_norm: &str, changes: &mut Vec<DiffChange>) {
    let old_lines: Vec<&str> = old_norm.lines().collect();
    let new_lines: Vec<&str> = new_norm.lines().collect();

    let old_set: HashSet<&str> = old_lines.iter().copied().collect();
    let new_set: HashSet<&str> = new_lines.iter().copied().collect();

    // Lines present in old but not in new → Removed
    for line in &old_lines {
        if !new_set.contains(line) && !line.trim().is_empty() {
            changes.push(DiffChange {
                kind: ChangeKind::Removed,
                section: extract_section_context(line),
                old_text: Some(line.to_string()),
                new_text: None,
            });
        }
    }

    // Lines present in new but not in old → Added
    for line in &new_lines {
        if !old_set.contains(line) && !line.trim().is_empty() {
            changes.push(DiffChange {
                kind: ChangeKind::Added,
                section: extract_section_context(line),
                old_text: None,
                new_text: Some(line.to_string()),
            });
        }
    }
}

/// Try to extract a section heading / context label from a line.
fn extract_section_context(line: &str) -> String {
    let trimmed = line.trim();
    let preview_len = 60.min(trimmed.len());
    let preview = &trimmed[..preview_len];
    if preview_len < trimmed.len() {
        format!("{}...", preview)
    } else {
        preview.to_string()
    }
}

/// Detect price changes between old and new text.
fn detect_price_changes(old_text: &str, new_text: &str, changes: &mut Vec<DiffChange>) {
    let price_re = Regex::new(r"\$\d+\.?\d*").expect("valid regex");

    let old_prices: Vec<&str> = price_re.find_iter(old_text).map(|m| m.as_str()).collect();
    let new_prices: Vec<&str> = price_re.find_iter(new_text).map(|m| m.as_str()).collect();

    let old_set: HashSet<&str> = old_prices.iter().copied().collect();
    let new_set: HashSet<&str> = new_prices.iter().copied().collect();

    // Pair up prices positionally when counts match, otherwise report set diffs
    if old_prices.len() == new_prices.len() {
        for (old_p, new_p) in old_prices.iter().zip(new_prices.iter()) {
            if old_p != new_p {
                changes.push(DiffChange {
                    kind: ChangeKind::PriceChange,
                    section: "price".to_string(),
                    old_text: Some(old_p.to_string()),
                    new_text: Some(new_p.to_string()),
                });
            }
        }
    } else {
        // Different number of prices — report prices that appeared/disappeared
        for p in old_set.difference(&new_set) {
            changes.push(DiffChange {
                kind: ChangeKind::PriceChange,
                section: "price".to_string(),
                old_text: Some(p.to_string()),
                new_text: None,
            });
        }
        for p in new_set.difference(&old_set) {
            changes.push(DiffChange {
                kind: ChangeKind::PriceChange,
                section: "price".to_string(),
                old_text: None,
                new_text: Some(p.to_string()),
            });
        }
    }
}

/// Detect date changes between old and new text.
fn detect_date_changes(old_text: &str, new_text: &str, changes: &mut Vec<DiffChange>) {
    // Match common date formats: YYYY-MM-DD, MM/DD/YYYY, DD-MM-YYYY,
    // "January 1, 2025", "Jan 1 2025", etc.
    let date_re = Regex::new(
        r"(?x)
          \d{4}-\d{1,2}-\d{1,2}                          # 2025-01-15
        | \d{1,2}/\d{1,2}/\d{2,4}                        # 01/15/2025
        | \d{1,2}-\d{1,2}-\d{2,4}                        # 15-01-2025
        | (?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|Jun(?:e)?
           |Jul(?:y)?|Aug(?:ust)?|Sep(?:tember)?|Oct(?:ober)?|Nov(?:ember)?
           |Dec(?:ember)?)\s+\d{1,2},?\s+\d{4}           # January 15, 2025
        "
    )
    .expect("valid regex");

    let old_dates: HashSet<String> = date_re
        .find_iter(old_text)
        .map(|m| m.as_str().to_string())
        .collect();
    let new_dates: HashSet<String> = date_re
        .find_iter(new_text)
        .map(|m| m.as_str().to_string())
        .collect();

    for d in old_dates.difference(&new_dates) {
        changes.push(DiffChange {
            kind: ChangeKind::DateChange,
            section: "date".to_string(),
            old_text: Some(d.clone()),
            new_text: None,
        });
    }
    for d in new_dates.difference(&old_dates) {
        changes.push(DiffChange {
            kind: ChangeKind::DateChange,
            section: "date".to_string(),
            old_text: None,
            new_text: Some(d.clone()),
        });
    }
}

/// Detect status changes (stock, availability, open/closed).
fn detect_status_changes(old_text: &str, new_text: &str, changes: &mut Vec<DiffChange>) {
    let status_pairs: &[(&str, &str)] = &[
        ("in stock", "out of stock"),
        ("available", "unavailable"),
        ("open", "closed"),
        ("active", "inactive"),
        ("enabled", "disabled"),
    ];

    let old_lower = old_text.to_lowercase();
    let new_lower = new_text.to_lowercase();

    for &(status_a, status_b) in status_pairs {
        // Check A → B transition
        if old_lower.contains(status_a) && !old_lower.contains(status_b)
            && new_lower.contains(status_b) && !new_lower.contains(status_a)
        {
            changes.push(DiffChange {
                kind: ChangeKind::StatusChange,
                section: "status".to_string(),
                old_text: Some(status_a.to_string()),
                new_text: Some(status_b.to_string()),
            });
        }
        // Check B → A transition
        else if old_lower.contains(status_b) && !old_lower.contains(status_a)
            && new_lower.contains(status_a) && !new_lower.contains(status_b)
        {
            changes.push(DiffChange {
                kind: ChangeKind::StatusChange,
                section: "status".to_string(),
                old_text: Some(status_b.to_string()),
                new_text: Some(status_a.to_string()),
            });
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
    fn normalize_strips_noise() {
        let text = "Hello World\n\
                     Advertisement\n\
                     Buy our product\n\
                     12:30 PM\n\
                     2024-01-15\n\
                     98765\n\
                     Sponsored content\n\
                     Real content here";
        let result = normalize_text(text);
        assert!(result.contains("hello world"));
        assert!(result.contains("buy our product"));
        assert!(result.contains("real content here"));
        assert!(!result.contains("advertisement"));
        assert!(!result.contains("12:30"));
        assert!(!result.contains("98765"));
        assert!(!result.contains("sponsored"));
    }

    #[test]
    fn similarity_identical_texts() {
        let text = "the quick brown fox jumps over the lazy dog";
        let score = compute_similarity(text, text);
        assert!(
            (score - 1.0).abs() < f64::EPSILON,
            "identical texts should have similarity 1.0, got {}",
            score
        );
    }

    #[test]
    fn similarity_completely_different() {
        let a = "the quick brown fox jumps over the lazy dog";
        let b = "alpha beta gamma delta epsilon zeta eta theta";
        let score = compute_similarity(a, b);
        assert!(
            score < 0.1,
            "completely different texts should have low similarity, got {}",
            score
        );
    }

    #[test]
    fn similarity_empty_texts() {
        let score = compute_similarity("", "");
        assert!(
            (score - 1.0).abs() < f64::EPSILON,
            "two empty texts should be identical, got {}",
            score
        );
    }

    #[test]
    fn diff_detects_price_changes() {
        let old = "Product X costs $99.99. Great value!";
        let new = "Product X costs $79.99. Great value!";
        let diff = diff_pages("https://example.com/product", old, new);

        let price_changes: Vec<&DiffChange> = diff
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::PriceChange)
            .collect();
        assert!(
            !price_changes.is_empty(),
            "should detect price change, changes: {:?}",
            diff.changes
        );
        assert_eq!(price_changes[0].old_text.as_deref(), Some("$99.99"));
        assert_eq!(price_changes[0].new_text.as_deref(), Some("$79.99"));
        assert!(diff.summary.contains("Price changed"));
    }

    #[test]
    fn diff_detects_added_removed_sections() {
        let old = "Section A content\nSection B content";
        let new = "Section A content\nSection C content";
        let diff = diff_pages("https://example.com/page", old, new);

        let added: Vec<&DiffChange> = diff
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Added)
            .collect();
        let removed: Vec<&DiffChange> = diff
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Removed)
            .collect();

        assert!(!added.is_empty(), "should detect added content");
        assert!(!removed.is_empty(), "should detect removed content");
    }

    #[test]
    fn diff_detects_status_changes() {
        let old = "This item is In Stock and ready to ship.";
        let new = "This item is Out of Stock. Check back later.";
        let diff = diff_pages("https://example.com/item", old, new);

        let status_changes: Vec<&DiffChange> = diff
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::StatusChange)
            .collect();
        assert!(
            !status_changes.is_empty(),
            "should detect status change, changes: {:?}",
            diff.changes
        );
        assert_eq!(status_changes[0].old_text.as_deref(), Some("in stock"));
        assert_eq!(status_changes[0].new_text.as_deref(), Some("out of stock"));
        assert!(diff.summary.contains("Status changed"));
    }

    #[test]
    fn generate_summary_price_format() {
        let changes = vec![DiffChange {
            kind: ChangeKind::PriceChange,
            section: "price".to_string(),
            old_text: Some("$99".to_string()),
            new_text: Some("$79".to_string()),
        }];
        let summary = generate_summary(&changes);
        assert_eq!(summary, "Price changed from $99 to $79");
    }

    #[test]
    fn generate_summary_status_format() {
        let changes = vec![DiffChange {
            kind: ChangeKind::StatusChange,
            section: "status".to_string(),
            old_text: Some("available".to_string()),
            new_text: Some("unavailable".to_string()),
        }];
        let summary = generate_summary(&changes);
        assert!(
            summary.contains("Status changed: available \u{2192} unavailable"),
            "got: {}",
            summary
        );
    }

    #[test]
    fn generate_summary_generic_counts() {
        let changes = vec![
            DiffChange {
                kind: ChangeKind::Added,
                section: "intro".to_string(),
                old_text: None,
                new_text: Some("new stuff".to_string()),
            },
            DiffChange {
                kind: ChangeKind::Removed,
                section: "footer".to_string(),
                old_text: Some("old stuff".to_string()),
                new_text: None,
            },
            DiffChange {
                kind: ChangeKind::Modified,
                section: "body".to_string(),
                old_text: Some("was this".to_string()),
                new_text: Some("now this".to_string()),
            },
        ];
        let summary = generate_summary(&changes);
        assert!(
            summary.contains("1 sections modified") && summary.contains("1 added") && summary.contains("1 removed"),
            "got: {}",
            summary
        );
    }

    #[test]
    fn generate_summary_no_changes() {
        let summary = generate_summary(&[]);
        assert_eq!(summary, "No meaningful changes detected");
    }
}
