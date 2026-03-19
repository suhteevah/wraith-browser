//! Integration tests against real websites.
//! These require network access — run with `cargo test --test real_sites`.

use sevro_headless::{SevroEngine, SevroConfig, PageEvent};

/// Helper to create an engine and navigate.
async fn navigate_and_check(url: &str) -> (SevroEngine, String) {
    let mut engine = SevroEngine::default();
    let event = engine.navigate(url).await.unwrap();
    assert_eq!(event, PageEvent::DomContentLoaded, "Navigation to {url} should succeed");
    let source = engine.page_source().unwrap().to_string();
    (engine, source)
}

#[tokio::test]
async fn test_example_com() {
    let (engine, source) = navigate_and_check("https://example.com").await;

    assert!(source.contains("Example Domain"), "Should contain 'Example Domain'");

    let nodes = engine.dom_snapshot_fast();
    assert!(!nodes.is_empty(), "Should have DOM nodes");

    // Should find the heading
    let h1 = nodes.iter().find(|n| n.tag_name == "h1");
    assert!(h1.is_some(), "Should have an h1");
    assert!(h1.unwrap().text_content.contains("Example Domain"));

    // Should find the link
    let link = nodes.iter().find(|n| n.tag_name == "a");
    assert!(link.is_some(), "Should have a link");
}

#[tokio::test]
async fn test_github_wraith_browser() {
    let (engine, source) = navigate_and_check("https://github.com/suhteevah/wraith-browser").await;

    // GitHub pages should have content
    assert!(source.len() > 1000, "GitHub page should be substantial");

    let nodes = engine.dom_snapshot_fast();
    assert!(nodes.len() > 10, "Should have many interactive elements");

    // Should be able to run JS
    let result = engine.eval_js("document.title").await;
    assert!(result.is_ok(), "JS eval should work");
}

#[tokio::test]
async fn test_js_execution_on_page() {
    let (engine, _) = navigate_and_check("https://example.com").await;

    // Basic JS should work
    assert_eq!(engine.eval_js("1 + 1").await.unwrap(), "2");
    assert_eq!(engine.eval_js("'hello ' + 'world'").await.unwrap(), "hello world");

    // DOM queries should work
    let result = engine.eval_js("document.querySelector('h1') ? 'found' : 'not found'").await.unwrap();
    assert_eq!(result, "found");

    // navigator should be available
    assert!(engine.eval_js("navigator.userAgent").await.unwrap().contains("Mozilla"));

    // setTimeout should not crash
    engine.eval_js("setTimeout(function() {}, 0)").await.unwrap();

    // localStorage should work
    engine.eval_js("localStorage.setItem('test', 'value')").await.unwrap();
    assert_eq!(engine.eval_js("localStorage.getItem('test')").await.unwrap(), "value");
}

#[tokio::test]
async fn test_query_selector_on_real_page() {
    let (engine, _) = navigate_and_check("https://example.com").await;

    // CSS selector queries via Rust
    let links = engine.query_selector("a");
    assert!(!links.is_empty(), "Should find links on example.com");

    let headings = engine.query_selector("h1");
    assert!(!headings.is_empty(), "Should find headings");
}

#[tokio::test]
async fn test_navigation_history() {
    let mut engine = SevroEngine::default();

    engine.navigate("https://example.com").await.unwrap();
    assert_eq!(engine.current_url().unwrap(), "https://example.com/");

    engine.navigate("https://www.iana.org/domains/reserved").await.unwrap();
    assert!(engine.current_url().unwrap().contains("iana.org"));

    // Go back
    engine.go_back().await.unwrap();
    // Should be back at example.com
    assert!(engine.current_url().unwrap().contains("example.com"));
}

#[tokio::test]
async fn test_cookie_persistence() {
    use sevro_headless::Cookie;

    let mut engine = SevroEngine::default();
    engine.set_cookie(Cookie {
        name: "test_cookie".to_string(),
        value: "abc123".to_string(),
        domain: "example.com".to_string(),
        path: "/".to_string(),
        secure: false,
        http_only: false,
        expires: None,
    });

    let cookies = engine.get_cookies("example.com");
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].value, "abc123");
}
