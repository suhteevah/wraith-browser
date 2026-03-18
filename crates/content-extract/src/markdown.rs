//! HTML to Markdown conversion optimized for LLM consumption.
//!
//! Converts HTML to clean markdown that maximizes information density
//! while minimizing token usage. Uses `scraper` for DOM traversal.

use crate::error::ExtractError;
use ego_tree::NodeRef;
use scraper::{Html, Node};

/// Convert HTML to markdown.
///
/// Handles: headings, paragraphs, links, images, lists, bold/italic,
/// code blocks, blockquotes, tables, horizontal rules.
pub fn html_to_markdown(html: &str) -> Result<String, ExtractError> {
    tracing::debug!(html_len = html.len(), "Converting HTML to markdown");

    let doc = Html::parse_fragment(html);
    let mut output = String::with_capacity(html.len() / 2);
    let mut ctx = ConvertContext::default();

    walk_node(doc.tree.root(), &doc, &mut output, &mut ctx);


    // Clean up excessive blank lines
    let cleaned = collapse_blank_lines(&output);
    Ok(cleaned.trim().to_string())
}

/// Convert HTML to plain text (no formatting).
///
/// Strips all HTML tags, decodes entities, collapses whitespace.
pub fn html_to_plain_text(html: &str) -> Result<String, ExtractError> {
    let doc = Html::parse_fragment(html);
    let mut output = String::with_capacity(html.len() / 2);

    extract_text_recursive(doc.tree.root(), &mut output);

    // Normalize whitespace
    let mut result = String::with_capacity(output.len());
    let mut last_was_space = false;
    let mut last_was_newline = false;

    for ch in output.chars() {
        if ch == '\n' {
            if !last_was_newline {
                result.push('\n');
                last_was_newline = true;
                last_was_space = false;
            }
        } else if ch.is_whitespace() {
            if !last_was_space && !last_was_newline {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
            last_was_newline = false;
        }
    }

    Ok(result.trim().to_string())
}

/// Traversal context for markdown conversion.
#[derive(Default)]
struct ConvertContext {
    /// Current list nesting depth and type (true = ordered)
    list_stack: Vec<ListInfo>,
    /// Whether we're inside a <pre> block
    in_pre: bool,
    /// Whether we're inside a <code> block
    in_code: bool,
    /// Current ordered list item counter
    ol_counter: Vec<usize>,
}

struct ListInfo {
    ordered: bool,
}

fn walk_node(
    node_ref: NodeRef<'_, Node>,
    _doc: &Html,
    out: &mut String,
    ctx: &mut ConvertContext,
) {
    match node_ref.value() {
        Node::Text(text) => {
            let t = text.text.as_ref();
            if ctx.in_pre {
                out.push_str(t);
            } else {
                // Collapse whitespace in normal text
                let collapsed = collapse_whitespace(t);
                if !collapsed.is_empty() {
                    out.push_str(&collapsed);
                }
            }
        }
        Node::Element(el) => {
            let tag = el.name();

            // Opening tag effects
            match tag {
                "h1" => out.push_str("\n\n# "),
                "h2" => out.push_str("\n\n## "),
                "h3" => out.push_str("\n\n### "),
                "h4" => out.push_str("\n\n#### "),
                "h5" => out.push_str("\n\n##### "),
                "h6" => out.push_str("\n\n###### "),
                "p" => out.push_str("\n\n"),
                "br" => out.push('\n'),
                "hr" => {
                    out.push_str("\n\n---\n\n");
                    return; // self-closing
                }
                "blockquote" => out.push_str("\n\n> "),
                "strong" | "b" => out.push_str("**"),
                "em" | "i" => out.push('*'),
                "del" | "s" | "strike" => out.push_str("~~"),
                "code" if !ctx.in_pre => {
                    out.push('`');
                    ctx.in_code = true;
                }
                "pre" => {
                    // Detect language from <code class="language-xxx">
                    let lang = node_ref
                        .children()
                        .find_map(|child| {
                            if let Node::Element(code_el) = child.value() {
                                if code_el.name() == "code" {
                                    return code_el.attr("class").and_then(|c| {
                                        c.split_whitespace()
                                            .find(|cls| cls.starts_with("language-"))
                                            .map(|cls| cls.trim_start_matches("language-"))
                                    });
                                }
                            }
                            None
                        })
                        .unwrap_or("");

                    out.push_str("\n\n```");
                    out.push_str(lang);
                    out.push('\n');
                    ctx.in_pre = true;
                }
                "ul" => {
                    out.push_str("\n\n");
                    ctx.list_stack.push(ListInfo { ordered: false });
                }
                "ol" => {
                    out.push_str("\n\n");
                    ctx.list_stack.push(ListInfo { ordered: true });
                    ctx.ol_counter.push(0);
                }
                "li" => {
                    let indent = "  ".repeat(ctx.list_stack.len().saturating_sub(1));
                    if let Some(info) = ctx.list_stack.last() {
                        if info.ordered {
                            if let Some(counter) = ctx.ol_counter.last_mut() {
                                *counter += 1;
                                out.push_str(&format!("\n{indent}{}. ", counter));
                            }
                        } else {
                            out.push_str(&format!("\n{indent}- "));
                        }
                    }
                }
                "a" => {
                    // Will be handled with closing logic
                    out.push('[');
                }
                "img" => {
                    let alt = el.attr("alt").unwrap_or("");
                    let src = el.attr("src").unwrap_or("");
                    if !src.is_empty() {
                        out.push_str(&format!("![{alt}]({src})"));
                    }
                    return; // self-closing
                }
                "table" => out.push_str("\n\n"),
                "tr" => out.push('\n'),
                "th" => out.push_str("| **"),
                "td" => out.push_str("| "),
                _ => {}
            }

            // Recurse into children
            for child in node_ref.children() {
                walk_node(child, _doc, out, ctx);
            }

            // Closing tag effects
            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.push_str("\n\n"),
                "p" => out.push_str("\n\n"),
                "blockquote" => out.push('\n'),
                "strong" | "b" => out.push_str("**"),
                "em" | "i" => out.push('*'),
                "del" | "s" | "strike" => out.push_str("~~"),
                "code" if !ctx.in_pre => {
                    out.push('`');
                    ctx.in_code = false;
                }
                "pre" => {
                    out.push_str("\n```\n\n");
                    ctx.in_pre = false;
                }
                "ul" => {
                    ctx.list_stack.pop();
                    out.push('\n');
                }
                "ol" => {
                    ctx.list_stack.pop();
                    ctx.ol_counter.pop();
                    out.push('\n');
                }
                "a" => {
                    let href = el.attr("href").unwrap_or("");
                    if !href.is_empty()
                        && !href.starts_with('#')
                        && !href.starts_with("javascript:")
                    {
                        out.push_str(&format!("]({href})"));
                    } else {
                        out.push(']');
                    }
                }
                "th" => out.push_str("** | "),
                "td" => out.push_str(" | "),
                "thead" => {
                    // Add header separator row
                    // Count columns by looking at th count in this thead
                    let col_count = node_ref
                        .descendants()
                        .filter(|n| matches!(n.value(), Node::Element(e) if e.name() == "th"))
                        .count();
                    if col_count > 0 {
                        out.push('\n');
                        for _ in 0..col_count {
                            out.push_str("| --- ");
                        }
                        out.push('|');
                    }
                }
                _ => {}
            }
        }
        Node::Document | Node::Fragment => {
            for child in node_ref.children() {
                walk_node(child, _doc, out, ctx);
            }
        }
        _ => {}
    }
}

fn extract_text_recursive(
    node_ref: NodeRef<'_, Node>,
    out: &mut String,
) {
    match node_ref.value() {
        Node::Text(text) => {
            out.push_str(text.text.as_ref());
        }
        Node::Element(el) => {
            let tag = el.name();
            match tag {
                "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li"
                | "tr" | "blockquote" | "pre" => {
                    out.push('\n');
                }
                _ => {}
            }

            for child in node_ref.children() {
                extract_text_recursive(child, out);
            }

            if matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote") {
                out.push('\n');
            }
        }
        Node::Document | Node::Fragment => {
            for child in node_ref.children() {
                extract_text_recursive(child, out);
            }
        }
        _ => {}
    }
}

/// Collapse consecutive whitespace into a single space.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    result
}

/// Collapse runs of 3+ newlines into 2.
fn collapse_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut newline_count = 0;

    for ch in s.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(ch);
            }
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_headings() {
        let md = html_to_markdown("<h1>Title</h1><h2>Sub</h2>").unwrap();
        assert!(md.contains("# Title"));
        assert!(md.contains("## Sub"));
    }

    #[test]
    fn converts_paragraphs() {
        let md = html_to_markdown("<p>First paragraph.</p><p>Second paragraph.</p>").unwrap();
        assert!(md.contains("First paragraph."));
        assert!(md.contains("Second paragraph."));
    }

    #[test]
    fn converts_links() {
        let md =
            html_to_markdown(r#"<p>Visit <a href="https://example.com">Example</a></p>"#)
                .unwrap();
        assert!(md.contains("[Example](https://example.com)"));
    }

    #[test]
    fn converts_images() {
        let md =
            html_to_markdown(r#"<img src="photo.jpg" alt="A photo"/>"#).unwrap();
        assert!(md.contains("![A photo](photo.jpg)"));
    }

    #[test]
    fn converts_bold_italic() {
        let md = html_to_markdown("<p><strong>bold</strong> and <em>italic</em></p>").unwrap();
        assert!(md.contains("**bold**"));
        assert!(md.contains("*italic*"));
    }

    #[test]
    fn converts_unordered_lists() {
        let md = html_to_markdown("<ul><li>One</li><li>Two</li><li>Three</li></ul>").unwrap();
        assert!(md.contains("- One"));
        assert!(md.contains("- Two"));
        assert!(md.contains("- Three"));
    }

    #[test]
    fn converts_ordered_lists() {
        let md = html_to_markdown("<ol><li>First</li><li>Second</li></ol>").unwrap();
        assert!(md.contains("1. First"));
        assert!(md.contains("2. Second"));
    }

    #[test]
    fn converts_code_blocks() {
        let md = html_to_markdown(
            r#"<pre><code class="language-rust">fn main() {}</code></pre>"#,
        )
        .unwrap();
        assert!(md.contains("```rust"));
        assert!(md.contains("fn main() {}"));
        assert!(md.contains("```"));
    }

    #[test]
    fn converts_inline_code() {
        let md = html_to_markdown("<p>Use <code>cargo build</code> to compile.</p>").unwrap();
        assert!(md.contains("`cargo build`"));
    }

    #[test]
    fn converts_blockquotes() {
        let md = html_to_markdown("<blockquote>A wise quote.</blockquote>").unwrap();
        assert!(md.contains("> A wise quote."));
    }

    #[test]
    fn plain_text_strips_tags() {
        let text = html_to_plain_text(
            "<h1>Title</h1><p>Hello <strong>world</strong>.</p>",
        )
        .unwrap();
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world."));
        assert!(!text.contains('<'));
    }

    #[test]
    fn converts_hr() {
        let md = html_to_markdown("<p>Above</p><hr/><p>Below</p>").unwrap();
        assert!(md.contains("---"));
    }
}
