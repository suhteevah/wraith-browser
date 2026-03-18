//! # PDF Text Extraction
//!
//! Pure-Rust PDF text extraction without external dependencies.
//! Implements a basic parser that reads the PDF object structure and
//! extracts text from BT/ET (Begin Text / End Text) operators.
//!
//! ## Supported operators
//!
//! - `Tj` — show a single text string
//! - `TJ` — show an array of text strings (with kerning adjustments)
//!
//! ## Limitations
//!
//! - Only handles basic ASCII/Latin text (no CIDFont or ToUnicode decoding)
//! - Does not decompress FlateDecode streams (only raw text streams)
//! - No font metric handling — all text is treated as unstyled content
//!
//! For production use, consider integrating `pdf-extract` or `lopdf`.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn, instrument};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Extracted content from a PDF document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfContent {
    /// Combined text from all pages.
    pub text: String,
    /// Number of pages detected in the document.
    pub page_count: usize,
    /// Document title from the Info dictionary, if present.
    pub title: Option<String>,
    /// Document author from the Info dictionary, if present.
    pub author: Option<String>,
    /// Per-page extracted text.
    pub pages: Vec<PdfPage>,
}

/// Extracted text from a single PDF page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfPage {
    /// 1-based page number.
    pub page_number: usize,
    /// Extracted text content for this page.
    pub text: String,
    /// Word count for this page.
    pub word_count: usize,
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extract text content from a PDF byte buffer.
///
/// Parses the PDF structure to locate page objects and extract text from
/// BT/ET text blocks. Returns a `PdfContent` with per-page results.
#[instrument(skip(data), fields(data_len = data.len()))]
pub fn extract_pdf_text(data: &[u8]) -> Result<PdfContent, String> {
    info!(bytes = data.len(), "Starting PDF text extraction");

    let text_str = std::str::from_utf8(data)
        .map_err(|_| "PDF contains non-UTF8 binary data in scanned region".to_string());

    // For binary PDFs we work with lossy conversion
    let content = match text_str {
        Ok(s) => s.to_string(),
        Err(_) => String::from_utf8_lossy(data).to_string(),
    };

    // Verify PDF header
    if !content.starts_with("%PDF") {
        return Err("Not a valid PDF file (missing %PDF header)".to_string());
    }

    // Count pages by /Type /Page occurrences (excluding /Type /Pages)
    let page_count = count_pages(&content);
    debug!(page_count, "Detected pages");

    // Extract document metadata
    let title = extract_info_field(&content, "/Title");
    let author = extract_info_field(&content, "/Author");

    // Extract text from BT/ET blocks
    let raw_texts = extract_bt_et_blocks(&content);

    // Build per-page structures
    // Simple heuristic: distribute BT/ET blocks across detected pages
    let mut pages = Vec::new();
    if page_count == 0 {
        // No pages detected, but we may still have text blocks
        if !raw_texts.is_empty() {
            let combined = raw_texts.join(" ");
            let word_count = combined.split_whitespace().count();
            pages.push(PdfPage {
                page_number: 1,
                text: combined,
                word_count,
            });
        }
    } else {
        let blocks_per_page = if page_count > 0 {
            (raw_texts.len() as f64 / page_count as f64).ceil() as usize
        } else {
            raw_texts.len()
        };
        let blocks_per_page = blocks_per_page.max(1);

        for (i, chunk) in raw_texts.chunks(blocks_per_page).enumerate() {
            let page_text = chunk.join(" ");
            let word_count = page_text.split_whitespace().count();
            pages.push(PdfPage {
                page_number: i + 1,
                text: page_text,
                word_count,
            });
        }

        // Ensure we have entries for all pages even if some have no text
        while pages.len() < page_count {
            pages.push(PdfPage {
                page_number: pages.len() + 1,
                text: String::new(),
                word_count: 0,
            });
        }
    }

    let combined_text: String = pages
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");

    info!(
        page_count = pages.len(),
        text_len = combined_text.len(),
        title = ?title,
        "PDF extraction complete"
    );

    Ok(PdfContent {
        text: combined_text,
        page_count: pages.len().max(page_count),
        title,
        author,
        pages,
    })
}

/// Format extracted PDF content as markdown with page separators.
#[instrument(skip(content), fields(page_count = content.page_count))]
pub fn pdf_to_markdown(content: &PdfContent) -> String {
    let mut md = String::new();

    if let Some(ref title) = content.title {
        md.push_str(&format!("# {}\n\n", title));
    }
    if let Some(ref author) = content.author {
        md.push_str(&format!("*Author: {}*\n\n", author));
    }

    for page in &content.pages {
        if content.pages.len() > 1 {
            md.push_str(&format!("---\n\n**Page {}**\n\n", page.page_number));
        }
        if !page.text.is_empty() {
            md.push_str(&page.text);
            md.push_str("\n\n");
        }
    }

    debug!(markdown_len = md.len(), "PDF converted to markdown");
    md
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Count /Type /Page objects (excluding /Type /Pages which is the page tree root).
fn count_pages(content: &str) -> usize {
    let mut count = 0;
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find("/Type") {
        let abs_pos = search_from + pos;
        let after = &content[abs_pos + 5..];
        let trimmed = after.trim_start();
        if trimmed.starts_with("/Page") && !trimmed.starts_with("/Pages") {
            count += 1;
        }
        search_from = abs_pos + 5;
    }
    count
}

/// Extract a string value from the PDF Info dictionary.
fn extract_info_field(content: &str, field: &str) -> Option<String> {
    let field_pos = content.find(field)?;
    let after = &content[field_pos + field.len()..];

    // Look for a parenthesized string: (Some Title)
    let paren_start = after.find('(')?;
    let remaining = &after[paren_start + 1..];
    let paren_end = remaining.find(')')?;
    let value = remaining[..paren_end].trim().to_string();

    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Extract text from BT ... ET blocks, parsing Tj and TJ operators.
fn extract_bt_et_blocks(content: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(bt_pos) = content[search_from..].find("BT") {
        let abs_bt = search_from + bt_pos;
        let after_bt = &content[abs_bt + 2..];

        if let Some(et_pos) = after_bt.find("ET") {
            let block = &after_bt[..et_pos];
            let text = extract_text_from_block(block);
            if !text.is_empty() {
                results.push(text);
            }
            search_from = abs_bt + 2 + et_pos + 2;
        } else {
            break;
        }
    }

    results
}

/// Parse a single BT/ET block for Tj and TJ operators.
fn extract_text_from_block(block: &str) -> String {
    let mut texts = Vec::new();

    // Handle Tj operator: (text) Tj
    let mut pos = 0;
    let bytes = block.as_bytes();
    while pos < bytes.len() {
        if bytes[pos] == b'(' {
            // Find matching closing paren (handle escapes)
            if let Some((text, end)) = extract_paren_string(block, pos) {
                texts.push(text);
                pos = end;
            } else {
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }

    texts.join("")
}

/// Extract a parenthesized string from the given position, handling `\)` escapes.
/// Returns (extracted_text, position_after_closing_paren).
fn extract_paren_string(content: &str, start: usize) -> Option<(String, usize)> {
    if content.as_bytes().get(start)? != &b'(' {
        return None;
    }

    let mut result = String::new();
    let mut depth = 1;
    let mut i = start + 1;
    let bytes = content.as_bytes();

    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'\\' => {
                // Escaped character
                i += 1;
                if i < bytes.len() {
                    match bytes[i] {
                        b'n' => result.push('\n'),
                        b'r' => result.push('\r'),
                        b't' => result.push('\t'),
                        b'(' => result.push('('),
                        b')' => result.push(')'),
                        b'\\' => result.push('\\'),
                        c => result.push(c as char),
                    }
                }
            }
            b'(' => {
                depth += 1;
                result.push('(');
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(')');
                }
            }
            c => {
                // Only include printable ASCII
                if (0x20..0x7F).contains(&c) {
                    result.push(c as char);
                }
            }
        }
        i += 1;
    }

    Some((result, i))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid PDF with the given text content.
    fn make_minimal_pdf(text: &str) -> Vec<u8> {
        // Minimal PDF 1.4 with a single page and text stream
        let stream_content = format!("BT /F1 12 Tf ({}) Tj ET", text);
        let stream_len = stream_content.len();

        let pdf = format!(
            "%PDF-1.4\n\
             1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
             2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n\
             4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n\
             xref\n0 5\n\
             0000000000 65535 f \n\
             0000000009 00000 n \n\
             0000000058 00000 n \n\
             0000000115 00000 n \n\
             0000000190 00000 n \n\
             trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            stream_len, stream_content, 250 + stream_len
        );

        pdf.into_bytes()
    }

    #[test]
    fn extract_simple_pdf() {
        let pdf_bytes = make_minimal_pdf("Hello World");
        let result = extract_pdf_text(&pdf_bytes).unwrap();

        assert!(result.text.contains("Hello World"));
        assert!(result.page_count >= 1);
        assert!(!result.pages.is_empty());
    }

    #[test]
    fn rejects_non_pdf() {
        let result = extract_pdf_text(b"This is not a PDF");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing %PDF header"));
    }

    #[test]
    fn extract_multiple_text_blocks() {
        let stream = "BT /F1 12 Tf (First) Tj ET BT /F1 12 Tf (Second) Tj ET";
        let pdf = format!(
            "%PDF-1.4\n\
             1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
             2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n\
             4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n\
             trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n0\n%%EOF",
            stream.len(), stream
        );

        let result = extract_pdf_text(pdf.as_bytes()).unwrap();
        assert!(result.text.contains("First"));
        assert!(result.text.contains("Second"));
    }

    #[test]
    fn extract_info_title_and_author() {
        let pdf = format!(
            "%PDF-1.4\n\
             1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
             2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n\
             4 0 obj\n<< /Length 30 >>\nstream\nBT (Test Page) Tj ET\nendstream\nendobj\n\
             5 0 obj\n<< /Title (My Document) /Author (Jane Doe) >>\nendobj\n\
             trailer\n<< /Size 6 /Root 1 0 R /Info 5 0 R >>\nstartxref\n0\n%%EOF"
        );

        let result = extract_pdf_text(pdf.as_bytes()).unwrap();
        assert_eq!(result.title, Some("My Document".to_string()));
        assert_eq!(result.author, Some("Jane Doe".to_string()));
    }

    #[test]
    fn pdf_to_markdown_single_page() {
        let content = PdfContent {
            text: "Hello World".to_string(),
            page_count: 1,
            title: Some("Test Doc".to_string()),
            author: Some("Test Author".to_string()),
            pages: vec![PdfPage {
                page_number: 1,
                text: "Hello World".to_string(),
                word_count: 2,
            }],
        };

        let md = pdf_to_markdown(&content);
        assert!(md.contains("# Test Doc"));
        assert!(md.contains("*Author: Test Author*"));
        assert!(md.contains("Hello World"));
        // Single page should not have page separator
        assert!(!md.contains("Page 1"));
    }

    #[test]
    fn pdf_to_markdown_multi_page() {
        let content = PdfContent {
            text: "Page one\n\nPage two".to_string(),
            page_count: 2,
            title: None,
            author: None,
            pages: vec![
                PdfPage {
                    page_number: 1,
                    text: "Page one".to_string(),
                    word_count: 2,
                },
                PdfPage {
                    page_number: 2,
                    text: "Page two".to_string(),
                    word_count: 2,
                },
            ],
        };

        let md = pdf_to_markdown(&content);
        assert!(md.contains("**Page 1**"));
        assert!(md.contains("**Page 2**"));
        assert!(md.contains("---"));
    }

    #[test]
    fn paren_string_with_escapes() {
        let input = r"(Hello \(world\))";
        let (text, _) = extract_paren_string(input, 0).unwrap();
        assert_eq!(text, "Hello (world)");
    }

    #[test]
    fn page_word_count() {
        let pdf_bytes = make_minimal_pdf("one two three four five");
        let result = extract_pdf_text(&pdf_bytes).unwrap();
        let total_words: usize = result.pages.iter().map(|p| p.word_count).sum();
        assert!(total_words >= 5);
    }
}
