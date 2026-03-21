//! CDP (Chrome DevTools Protocol) browser engine backend.
//!
//! Feature-gated behind `cdp`. Connects to a real Chrome/Chromium instance
//! via WebSocket, controlling it through the DevTools Protocol. This gives
//! full JavaScript execution, screenshots, network interception, and
//! pixel-perfect rendering — at the cost of requiring Chrome installed.
//!
//! # Architecture
//!
//! ```text
//! CdpEngine
//!   ├── Chrome child process (--headless=new --remote-debugging-port=…)
//!   ├── WebSocket (tokio-tungstenite) → JSON-RPC over CDP
//!   └── Temp user-data-dir (cleaned up on shutdown)
//! ```

use crate::actions::{ActionResult, BrowserAction, ScrollDirection};
use crate::dom::{DomElement, DomSnapshot, PageMeta};
use crate::engine::{BrowserEngine, EngineCapabilities, ScreenshotCapability};
use crate::error::{BrowserError, BrowserResult};

use async_trait::async_trait;
use base64::Engine as _;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;
use tracing::{debug, info, warn};

type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Default command timeout for CDP calls (30 seconds).
const CDP_TIMEOUT: Duration = Duration::from_secs(30);

/// Post-navigation hydration wait (2 seconds for React/SPA hydration).
const HYDRATION_WAIT: Duration = Duration::from_secs(2);

/// Time to wait after a click for potential navigation to begin.
const POST_CLICK_NAV_WAIT: Duration = Duration::from_millis(500);

/// Maximum time to poll for a new page target after navigation destroys the old one.
const RECONNECT_POLL_TIMEOUT: Duration = Duration::from_secs(10);

/// Interval between polls for the new page target.
const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Maximum time to wait for Chrome to start and expose its DevTools endpoint.
const CHROME_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// CDP Engine
// ---------------------------------------------------------------------------

/// A `BrowserEngine` implementation backed by a real Chrome process controlled
/// via the Chrome DevTools Protocol over WebSocket.
pub struct CdpEngine {
    /// Sender half of the WebSocket — used to send CDP commands.
    ws_tx: Arc<Mutex<SplitSink<WsStream, Message>>>,

    /// Background reader task handle — reads CDP events/responses and routes them.
    _reader_handle: tokio::task::JoinHandle<()>,

    /// Map of pending CDP request IDs to their response channels.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,

    /// Map of event method names to channels waiting for a single event fire.
    event_waiters: Arc<Mutex<HashMap<String, Vec<oneshot::Sender<Value>>>>>,

    /// Monotonic CDP message ID counter.
    next_id: Arc<AtomicU64>,

    /// Chrome child process — killed on shutdown.
    chrome_process: Option<Child>,

    /// Temporary user-data-dir — deleted on shutdown.
    temp_dir: Option<PathBuf>,

    /// The URL we last navigated to (tracked locally for current_url fallback).
    /// Wrapped in `Arc<Mutex<>>` so the background reader can update it on
    /// `Page.frameNavigated` events.
    last_url: Arc<Mutex<Option<String>>>,

    /// The Chrome remote-debugging port — needed to reconnect after navigation
    /// destroys the old page target.
    port: u16,
}

impl CdpEngine {
    /// Launch a new headless Chrome and connect to it via CDP WebSocket.
    ///
    /// This will:
    /// 1. Create a temporary user-data directory.
    /// 2. Spawn Chrome with `--headless=new --remote-debugging-port={port}`.
    /// 3. Poll `http://127.0.0.1:{port}/json/version` until Chrome is ready.
    /// 4. Connect a WebSocket to the browser's `webSocketDebuggerUrl`.
    /// 5. Enable `Page` and `Runtime` domains.
    pub async fn new() -> BrowserResult<Self> {
        Self::with_port(0).await
    }

    /// Launch with a specific debugging port (0 = pick a free port).
    pub async fn with_port(mut port: u16) -> BrowserResult<Self> {
        // Pick a free port if 0
        if port == 0 {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")
                .map_err(|e| BrowserError::LaunchFailed(format!("bind free port: {e}")))?;
            port = listener.local_addr().unwrap().port();
            drop(listener);
        }

        // Create temp user-data-dir
        let temp_dir = std::env::temp_dir().join(format!("wraith-cdp-{}-{}", std::process::id(), port));
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| BrowserError::LaunchFailed(format!("create temp dir: {e}")))?;

        // Find Chrome binary
        let chrome_bin = find_chrome_binary()
            .ok_or_else(|| BrowserError::LaunchFailed(
                "Chrome/Chromium not found. Install Chrome or set CHROME_PATH env var.".into()
            ))?;

        info!(chrome = %chrome_bin, port, "Launching headless Chrome for CDP");

        let child = std::process::Command::new(&chrome_bin)
            .args([
                &format!("--headless=new"),
                &format!("--remote-debugging-port={port}"),
                &format!("--user-data-dir={}", temp_dir.display()),
                "--no-first-run",
                "--disable-extensions",
                "--disable-gpu",
                "--disable-background-networking",
                "--disable-default-apps",
                "--disable-sync",
                "--disable-translate",
                "--metrics-recording-only",
                "--no-default-browser-check",
                "--mute-audio",
                // Viewport
                "--window-size=1280,720",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| BrowserError::LaunchFailed(format!("spawn Chrome: {e}")))?;

        // Wait for DevTools endpoint to become available
        let ws_url = wait_for_devtools(port).await?;
        debug!(ws_url = %ws_url, "Chrome DevTools ready");

        // Connect WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| BrowserError::LaunchFailed(format!("WebSocket connect: {e}")))?;

        let (ws_tx, ws_rx) = futures::StreamExt::split(ws_stream);

        let ws_tx = Arc::new(Mutex::new(ws_tx));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let event_waiters: Arc<Mutex<HashMap<String, Vec<oneshot::Sender<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));

        let last_url: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        // Spawn background reader
        let reader_handle = {
            let pending = Arc::clone(&pending);
            let event_waiters = Arc::clone(&event_waiters);
            let last_url = Arc::clone(&last_url);
            tokio::spawn(async move {
                cdp_reader_loop(ws_rx, pending, event_waiters, last_url).await;
            })
        };

        let engine = Self {
            ws_tx,
            _reader_handle: reader_handle,
            pending,
            event_waiters,
            next_id,
            chrome_process: Some(child),
            temp_dir: Some(temp_dir),
            last_url,
            port,
        };

        // Enable required CDP domains
        engine.send_cdp_command("Page.enable", json!({})).await?;
        engine.send_cdp_command("Runtime.enable", json!({})).await?;
        engine.send_cdp_command("DOM.enable", json!({})).await?;

        info!("CDP engine ready");
        Ok(engine)
    }

    // -----------------------------------------------------------------------
    // CDP transport
    // -----------------------------------------------------------------------

    /// Send a CDP JSON-RPC command and wait for the response (with 30s timeout).
    async fn send_cdp_command(&self, method: &str, params: Value) -> BrowserResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let msg = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut ws = self.ws_tx.lock().await;
            ws.send(Message::Text(msg.to_string().into()))
                .await
                .map_err(|e| BrowserError::EngineError(format!("CDP send: {e}")))?;
        }

        debug!(id, method, "CDP →");

        match timeout(CDP_TIMEOUT, rx).await {
            Ok(Ok(response)) => {
                if let Some(err) = response.get("error") {
                    Err(BrowserError::EngineError(format!(
                        "CDP {method} error: {}",
                        err
                    )))
                } else {
                    Ok(response.get("result").cloned().unwrap_or(json!({})))
                }
            }
            Ok(Err(_)) => Err(BrowserError::EngineError(format!(
                "CDP {method}: response channel dropped"
            ))),
            Err(_) => {
                // Remove stale pending entry
                self.pending.lock().await.remove(&id);
                Err(BrowserError::Timeout {
                    action: format!("CDP {method}"),
                    ms: CDP_TIMEOUT.as_millis() as u64,
                })
            }
        }
    }

    /// Wait for a specific CDP event to fire (one-shot).
    async fn wait_for_event(&self, method: &str, timeout_dur: Duration) -> BrowserResult<Value> {
        let (tx, rx) = oneshot::channel();
        {
            let mut waiters = self.event_waiters.lock().await;
            waiters.entry(method.to_string()).or_default().push(tx);
        }

        match timeout(timeout_dur, rx).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(_)) => Err(BrowserError::EngineError(format!(
                "Event {method}: channel dropped"
            ))),
            Err(_) => Err(BrowserError::Timeout {
                action: format!("wait for event {method}"),
                ms: timeout_dur.as_millis() as u64,
            }),
        }
    }

    /// Evaluate a JavaScript expression via `Runtime.evaluate` and return the
    /// string result. Handles both primitive and JSON-serialized return values.
    async fn runtime_evaluate(&self, expression: &str) -> BrowserResult<String> {
        let result = self
            .send_cdp_command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        // Check for exception
        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown JS error");
            return Err(BrowserError::JsEvalFailed(text.to_string()));
        }

        let empty = json!({});
        let remote_obj = result.get("result").unwrap_or(&empty);
        let value = remote_obj.get("value");

        match value {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(Value::Null) | None => Ok("undefined".to_string()),
            Some(other) => Ok(other.to_string()),
        }
    }

    /// Reconnect to the new page target after navigation destroyed the old one.
    ///
    /// When a CDP click causes a full-page navigation (e.g. clicking an `<a>` link),
    /// Chrome destroys the old page target and creates a new one. The WebSocket
    /// connection to the old target dies, so we must:
    /// 1. Close the old WebSocket
    /// 2. Poll `/json` for the new page target
    /// 3. Connect a new WebSocket
    /// 4. Re-enable CDP domains (Page, Runtime, DOM)
    /// 5. Wait for the page to finish loading
    async fn reconnect_to_new_target(&mut self) -> BrowserResult<()> {
        info!(port = self.port, "Reconnecting to new page target after navigation");

        // 1. Close old WebSocket gracefully
        {
            let mut ws = self.ws_tx.lock().await;
            let _ = ws.close().await;
        }

        // 2. Poll /json for the new page target
        let targets_url = format!("http://127.0.0.1:{}/json", self.port);
        let start = std::time::Instant::now();
        let ws_url = loop {
            if start.elapsed() > RECONNECT_POLL_TIMEOUT {
                return Err(BrowserError::EngineError(
                    "Timed out waiting for new page target after navigation".into(),
                ));
            }

            match reqwest::get(&targets_url).await {
                Ok(resp) if resp.status().is_success() => {
                    let targets: Vec<Value> = resp.json().await.map_err(|e| {
                        BrowserError::EngineError(format!("parse /json targets: {e}"))
                    })?;

                    // Find a page-type target with a webSocketDebuggerUrl
                    let page_target = targets.iter().find(|t| {
                        t.get("type").and_then(|v| v.as_str()) == Some("page")
                            && t.get("webSocketDebuggerUrl").is_some()
                    });

                    if let Some(target) = page_target {
                        let url = target
                            .get("webSocketDebuggerUrl")
                            .and_then(|v| v.as_str())
                            .unwrap()
                            .to_string();
                        debug!(ws_url = %url, "Found new page target");
                        break url;
                    }
                }
                _ => {}
            }

            tokio::time::sleep(RECONNECT_POLL_INTERVAL).await;
        };

        // 3. Connect new WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| BrowserError::EngineError(format!("Reconnect WebSocket: {e}")))?;

        let (ws_tx, ws_rx) = futures::StreamExt::split(ws_stream);
        self.ws_tx = Arc::new(Mutex::new(ws_tx));

        // Reset pending requests — old ones are invalid after reconnect
        {
            let mut pending = self.pending.lock().await;
            pending.clear();
        }

        // Reset event waiters
        {
            let mut waiters = self.event_waiters.lock().await;
            waiters.clear();
        }

        // Reset message ID counter
        self.next_id = Arc::new(AtomicU64::new(1));

        // Spawn new background reader
        let pending_clone = Arc::clone(&self.pending);
        let event_waiters_clone = Arc::clone(&self.event_waiters);
        let last_url_clone = Arc::clone(&self.last_url);
        self._reader_handle = tokio::spawn(async move {
            cdp_reader_loop(ws_rx, pending_clone, event_waiters_clone, last_url_clone).await;
        });

        // 4. Re-enable required CDP domains
        self.send_cdp_command("Page.enable", json!({})).await?;
        self.send_cdp_command("Runtime.enable", json!({})).await?;
        self.send_cdp_command("DOM.enable", json!({})).await?;

        // 5. Wait for the page to finish loading (best-effort — page may already be loaded)
        let load_result = self
            .wait_for_event("Page.loadEventFired", Duration::from_secs(10))
            .await;
        if load_result.is_err() {
            debug!("Page.loadEventFired not received — page may already be loaded");
        }

        // Wait for hydration
        tokio::time::sleep(HYDRATION_WAIT).await;

        // 6. Update last_url from the new page
        if let Ok(url) = self.runtime_evaluate("window.location.href").await {
            info!(url = %url, "Reconnected — new page URL");
            *self.last_url.lock().await = Some(url);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BrowserEngine trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl BrowserEngine for CdpEngine {
    async fn navigate(&mut self, url: &str) -> BrowserResult<()> {
        info!(url, "CDP navigate");

        let result = self
            .send_cdp_command("Page.navigate", json!({ "url": url }))
            .await?;

        // Check for navigation error
        if let Some(err_text) = result.get("errorText").and_then(|v| v.as_str()) {
            if !err_text.is_empty() {
                return Err(BrowserError::NavigationFailed {
                    url: url.to_string(),
                    reason: err_text.to_string(),
                });
            }
        }

        // Wait for load event (or frameStoppedLoading as fallback)
        let load_wait = self.wait_for_event("Page.loadEventFired", Duration::from_secs(30));
        let stop_wait = self.wait_for_event("Page.frameStoppedLoading", Duration::from_secs(30));

        // Accept whichever fires first
        tokio::select! {
            r = load_wait => { let _ = r; }
            r = stop_wait => { let _ = r; }
        }

        // Wait for React/SPA hydration
        tokio::time::sleep(HYDRATION_WAIT).await;

        *self.last_url.lock().await = Some(url.to_string());
        Ok(())
    }

    async fn snapshot(&self) -> BrowserResult<DomSnapshot> {
        // Inject the snapshot JS that walks the DOM and builds the element list.
        // This produces the SAME format as Sevro's snapshot: @ref numbering,
        // role mapping (a→link, button→button, input→type, select→combobox, etc.)
        let snapshot_js = SNAPSHOT_JS;

        let json_str = self.runtime_evaluate(snapshot_js).await?;

        let raw: Value = serde_json::from_str(&json_str).map_err(|e| {
            BrowserError::EngineError(format!("snapshot JSON parse: {e}"))
        })?;

        // Parse the snapshot JSON into our DomSnapshot struct
        let url = raw
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = raw
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let elements: Vec<DomElement> = raw
            .get("elements")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|el| {
                        Some(DomElement {
                            ref_id: el.get("ref_id")?.as_u64()? as u32,
                            role: el
                                .get("role")
                                .and_then(|v| v.as_str())
                                .unwrap_or("text")
                                .to_string(),
                            text: el.get("text").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            href: el.get("href").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            placeholder: el
                                .get("placeholder")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            value: el
                                .get("value")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            enabled: el.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                            visible: el.get("visible").and_then(|v| v.as_bool()).unwrap_or(true),
                            aria_label: el
                                .get("aria_label")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            selector: el
                                .get("selector")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            bounds: el.get("bounds").and_then(|v| {
                                let x = v.get("x")?.as_f64()?;
                                let y = v.get("y")?.as_f64()?;
                                let w = v.get("width")?.as_f64()?;
                                let h = v.get("height")?.as_f64()?;
                                Some((x, y, w, h))
                            }),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let meta_raw = raw.get("meta").cloned().unwrap_or(json!({}));
        let meta = PageMeta {
            page_type: meta_raw
                .get("page_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            main_content_preview: meta_raw
                .get("main_content_preview")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: meta_raw
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            form_count: meta_raw
                .get("form_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            has_login_form: meta_raw
                .get("has_login_form")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            has_captcha: meta_raw
                .get("has_captcha")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            interactive_element_count: elements
                .iter()
                .filter(|e| matches!(e.role.as_str(), "link" | "button" | "textbox" | "combobox"))
                .count(),
            overlays: meta_raw
                .get("overlays")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|o| {
                            let ref_id = o.get("ref_id")?.to_string().replace('"', "");
                            let otype = o
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("modal")
                                .to_string();
                            let otitle = o
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            Some((format!("e{ref_id}"), otype, otitle))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };

        Ok(DomSnapshot {
            url,
            title,
            elements,
            meta,
            timestamp: chrono::Utc::now(),
        })
    }

    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult> {
        match action {
            BrowserAction::Navigate { url } => {
                self.navigate(&url).await?;
                let title = self.runtime_evaluate("document.title").await.unwrap_or_default();
                Ok(ActionResult::Navigated {
                    url: url.clone(),
                    title,
                })
            }

            BrowserAction::Click { ref_id, force: _ } => {
                // Capture URL before click to detect navigation
                let url_before = self
                    .runtime_evaluate("window.location.href")
                    .await
                    .unwrap_or_default();

                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        el.scrollIntoView({{block: 'center'}});
                        el.click();
                        return 'clicked';
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") {
                    return Ok(ActionResult::Failed { error: result });
                }

                // Wait briefly for potential navigation to start
                tokio::time::sleep(POST_CLICK_NAV_WAIT).await;

                // Check if the click caused navigation (target may be destroyed)
                let needs_reconnect =
                    match self.runtime_evaluate("window.location.href").await {
                        Ok(url_after) => {
                            // Evaluate succeeded — check if URL actually changed
                            if url_after != url_before {
                                debug!(
                                    before = %url_before,
                                    after = %url_after,
                                    "Click caused same-target navigation"
                                );
                                *self.last_url.lock().await = Some(url_after);
                                false // same target, just URL changed (SPA navigation)
                            } else {
                                false // no navigation at all
                            }
                        }
                        Err(e) => {
                            // Evaluate failed — target was likely destroyed by navigation
                            debug!(
                                error = %e,
                                "Click destroyed page target — need reconnect"
                            );
                            true
                        }
                    };

                if needs_reconnect {
                    self.reconnect_to_new_target().await?;
                    let new_url = self.last_url.lock().await.clone().unwrap_or_default();
                    Ok(ActionResult::Navigated {
                        url: new_url,
                        title: self
                            .runtime_evaluate("document.title")
                            .await
                            .unwrap_or_default(),
                    })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Clicked @e{ref_id}"),
                    })
                }
            }

            BrowserAction::Fill {
                ref_id,
                text,
                force: _,
            } => {
                // Use native setter + event dispatch (same technique as QuickJS bridge)
                let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        el.scrollIntoView({{block: 'center'}});
                        el.focus();
                        const nativeSetter = Object.getOwnPropertyDescriptor(
                            window.HTMLInputElement.prototype, 'value'
                        ) || Object.getOwnPropertyDescriptor(
                            window.HTMLTextAreaElement.prototype, 'value'
                        );
                        if (nativeSetter && nativeSetter.set) {{
                            nativeSetter.set.call(el, '{escaped}');
                        }} else {{
                            el.value = '{escaped}';
                        }}
                        el.dispatchEvent(new Event('input', {{bubbles: true}}));
                        el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return 'filled';
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") {
                    Ok(ActionResult::Failed { error: result })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Filled @e{ref_id} with text"),
                    })
                }
            }

            BrowserAction::Select {
                ref_id,
                value,
                force: _,
            } => {
                let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        el.value = '{escaped}';
                        el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        return 'selected';
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") {
                    Ok(ActionResult::Failed { error: result })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Selected '{value}' on @e{ref_id}"),
                    })
                }
            }

            BrowserAction::KeyPress { key } => {
                // Use Input.dispatchKeyEvent for special keys
                let key_code = key_to_cdp_key(&key);
                self.send_cdp_command(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "keyDown",
                        "key": key_code.0,
                        "code": key_code.1,
                        "windowsVirtualKeyCode": key_code.2,
                    }),
                )
                .await?;
                self.send_cdp_command(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "keyUp",
                        "key": key_code.0,
                        "code": key_code.1,
                        "windowsVirtualKeyCode": key_code.2,
                    }),
                )
                .await?;
                Ok(ActionResult::Success {
                    message: format!("Pressed key: {key}"),
                })
            }

            BrowserAction::TypeText {
                ref_id,
                text,
                delay_ms,
                force: _,
            } => {
                // Focus the element first
                let focus_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        el.focus();
                        return 'focused';
                    }})()"#
                );
                let result = self.runtime_evaluate(&focus_js).await?;
                if result.contains("not found") {
                    return Ok(ActionResult::Failed { error: result });
                }

                // Type character by character with delays
                for ch in text.chars() {
                    self.send_cdp_command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "keyDown",
                            "text": ch.to_string(),
                            "key": ch.to_string(),
                        }),
                    )
                    .await?;
                    self.send_cdp_command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "keyUp",
                            "key": ch.to_string(),
                        }),
                    )
                    .await?;
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
                    }
                }

                Ok(ActionResult::Success {
                    message: format!("Typed text into @e{ref_id}"),
                })
            }

            BrowserAction::Scroll { direction, amount } => {
                let (dx, dy) = match direction {
                    ScrollDirection::Up => (0, -amount),
                    ScrollDirection::Down => (0, amount),
                    ScrollDirection::Left => (-amount, 0),
                    ScrollDirection::Right => (amount, 0),
                };
                let js = format!("window.scrollBy({dx}, {dy})");
                self.runtime_evaluate(&js).await?;
                Ok(ActionResult::Success {
                    message: format!("Scrolled {direction:?} by {amount}px"),
                })
            }

            BrowserAction::Hover { ref_id } => {
                // Get element center coordinates and dispatch mouseover
                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return JSON.stringify({{error: 'not found'}});
                        const rect = el.getBoundingClientRect();
                        el.dispatchEvent(new MouseEvent('mouseover', {{bubbles: true}}));
                        el.dispatchEvent(new MouseEvent('mouseenter', {{bubbles: true}}));
                        return JSON.stringify({{x: rect.x + rect.width/2, y: rect.y + rect.height/2}});
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") {
                    Ok(ActionResult::Failed {
                        error: format!("Element @e{ref_id} not found"),
                    })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Hovered @e{ref_id}"),
                    })
                }
            }

            BrowserAction::GoBack => {
                self.runtime_evaluate("window.history.back()").await?;
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(ActionResult::Success {
                    message: "Navigated back".into(),
                })
            }

            BrowserAction::GoForward => {
                self.runtime_evaluate("window.history.forward()").await?;
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(ActionResult::Success {
                    message: "Navigated forward".into(),
                })
            }

            BrowserAction::Reload => {
                self.send_cdp_command("Page.reload", json!({})).await?;
                let _ = self
                    .wait_for_event("Page.loadEventFired", Duration::from_secs(30))
                    .await;
                tokio::time::sleep(HYDRATION_WAIT).await;
                Ok(ActionResult::Success {
                    message: "Page reloaded".into(),
                })
            }

            BrowserAction::Wait { ms } => {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(ActionResult::Success {
                    message: format!("Waited {ms}ms"),
                })
            }

            BrowserAction::WaitForSelector {
                selector,
                timeout_ms,
            } => {
                let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
                let poll_interval = 100u64;
                let max_polls = timeout_ms / poll_interval;

                for _ in 0..max_polls {
                    let js = format!(
                        "document.querySelector('{escaped}') ? 'found' : 'not_found'"
                    );
                    let result = self.runtime_evaluate(&js).await?;
                    if result == "found" {
                        return Ok(ActionResult::Success {
                            message: format!("Selector '{selector}' found"),
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(poll_interval)).await;
                }

                Ok(ActionResult::Failed {
                    error: format!("Selector '{selector}' not found after {timeout_ms}ms"),
                })
            }

            BrowserAction::WaitForNavigation { timeout_ms } => {
                match self
                    .wait_for_event(
                        "Page.loadEventFired",
                        Duration::from_millis(timeout_ms),
                    )
                    .await
                {
                    Ok(_) => Ok(ActionResult::Success {
                        message: "Navigation completed".into(),
                    }),
                    Err(_) => Ok(ActionResult::Failed {
                        error: format!("Navigation did not complete within {timeout_ms}ms"),
                    }),
                }
            }

            BrowserAction::EvalJs { script } => {
                let result = self.runtime_evaluate(&script).await?;
                Ok(ActionResult::JsResult { value: result })
            }

            BrowserAction::Screenshot { full_page } => {
                let params = if full_page {
                    json!({ "format": "png", "captureBeyondViewport": true })
                } else {
                    json!({ "format": "png" })
                };
                let result = self
                    .send_cdp_command("Page.captureScreenshot", params)
                    .await?;
                let data = result
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ActionResult::Screenshot {
                    png_base64: data,
                    width: 1280,
                    height: 720,
                })
            }

            BrowserAction::ExtractContent => {
                let js = r#"(() => {
                    const clone = document.body.cloneNode(true);
                    clone.querySelectorAll('script, style, noscript, svg, iframe').forEach(e => e.remove());
                    return clone.innerText || clone.textContent || '';
                })()"#;
                let text = self.runtime_evaluate(js).await?;
                let word_count = text.split_whitespace().count();
                Ok(ActionResult::Content {
                    markdown: text,
                    word_count,
                })
            }

            BrowserAction::UploadFile {
                ref_id,
                file_name,
                file_data,
                mime_type: _,
            } => {
                // Decode base64 file data and write to a temp file
                let bytes = base64::engine::general_purpose::STANDARD.decode(&file_data)
                .map_err(|e| BrowserError::EngineError(format!("base64 decode: {e}")))?;

                let tmp_path = std::env::temp_dir().join(format!("wraith-upload-{}", &file_name));
                std::fs::write(&tmp_path, &bytes)
                    .map_err(|e| BrowserError::EngineError(format!("write temp file: {e}")))?;

                // Get the DOM nodeId for the file input element
                let node_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return '-1';
                        return el.getAttribute('data-wraith-ref');
                    }})()"#
                );
                let ref_check = self.runtime_evaluate(&node_js).await?;
                if ref_check == "-1" {
                    return Ok(ActionResult::Failed {
                        error: format!("File input @e{ref_id} not found"),
                    });
                }

                // Use Runtime.evaluate to get the RemoteObjectId for the element
                let get_obj = self
                    .send_cdp_command(
                        "Runtime.evaluate",
                        json!({
                            "expression": format!(
                                "document.querySelector('[data-wraith-ref=\"{ref_id}\"]')"
                            ),
                            "returnByValue": false,
                        }),
                    )
                    .await?;

                let object_id = get_obj
                    .get("result")
                    .and_then(|r| r.get("objectId"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BrowserError::EngineError("Could not get objectId for file input".into())
                    })?;

                // Resolve to DOM nodeId
                let dom_result = self
                    .send_cdp_command(
                        "DOM.describeNode",
                        json!({ "objectId": object_id }),
                    )
                    .await?;

                let backend_node_id = dom_result
                    .get("node")
                    .and_then(|n| n.get("backendNodeId"))
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| {
                        BrowserError::EngineError("Could not get backendNodeId".into())
                    })?;

                // Use DOM.setFileInputFiles to set the file
                let file_path_str = tmp_path.to_string_lossy().replace('\\', "/");
                self.send_cdp_command(
                    "DOM.setFileInputFiles",
                    json!({
                        "files": [file_path_str],
                        "backendNodeId": backend_node_id,
                    }),
                )
                .await?;

                // Clean up temp file
                let _ = std::fs::remove_file(&tmp_path);

                Ok(ActionResult::Success {
                    message: format!("Uploaded '{}' to @e{}", file_name, ref_id),
                })
            }

            BrowserAction::SubmitForm { ref_id } => {
                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        const form = el.closest('form') || el;
                        const submit = form.querySelector('[type="submit"], button:not([type])');
                        if (submit) {{
                            submit.click();
                            return 'submitted via button';
                        }}
                        if (form.submit) {{
                            form.submit();
                            return 'submitted via form.submit()';
                        }}
                        return 'no submit mechanism found';
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") || result.contains("no submit") {
                    Ok(ActionResult::Failed { error: result })
                } else {
                    // Wait briefly for form submission navigation
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Ok(ActionResult::Success {
                        message: format!("Submitted form at @e{ref_id}"),
                    })
                }
            }

            BrowserAction::ScrollTo { ref_id } => {
                let js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'Element @e{ref_id} not found';
                        el.scrollIntoView({{block: 'center', behavior: 'smooth'}});
                        return 'scrolled';
                    }})()"#
                );
                let result = self.runtime_evaluate(&js).await?;
                if result.contains("not found") {
                    Ok(ActionResult::Failed { error: result })
                } else {
                    Ok(ActionResult::Success {
                        message: format!("Scrolled to @e{ref_id}"),
                    })
                }
            }
        }
    }

    async fn eval_js(&self, script: &str) -> BrowserResult<String> {
        self.runtime_evaluate(script).await
    }

    async fn page_source(&self) -> BrowserResult<String> {
        self.runtime_evaluate("document.documentElement.outerHTML")
            .await
    }

    async fn current_url(&self) -> Option<String> {
        self.runtime_evaluate("window.location.href").await.ok()
    }

    async fn screenshot(&self) -> BrowserResult<Vec<u8>> {
        let result = self
            .send_cdp_command(
                "Page.captureScreenshot",
                json!({ "format": "png" }),
            )
            .await?;

        let b64 = result
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BrowserError::ScreenshotFailed("no data in response".into()))?;

        base64::engine::general_purpose::STANDARD.decode(b64)
            .map_err(|e| BrowserError::ScreenshotFailed(format!("base64 decode: {e}")))
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            javascript: true,
            screenshots: ScreenshotCapability::FullPage,
            layout: true,
            cookies: true,
            stealth: false, // no stealth evasions by default
        }
    }

    async fn set_cookie_values(&mut self, domain: &str, name: &str, value: &str, path: &str) {
        let _ = self
            .send_cdp_command(
                "Network.setCookie",
                json!({
                    "name": name,
                    "value": value,
                    "domain": domain,
                    "path": path,
                }),
            )
            .await;
    }

    async fn shutdown(&mut self) -> BrowserResult<()> {
        info!("CDP engine shutting down");

        // Try graceful browser close
        let _ = self
            .send_cdp_command("Browser.close", json!({}))
            .await;

        // Kill Chrome process
        if let Some(ref mut child) = self.chrome_process {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.chrome_process = None;

        // Delete temp directory
        if let Some(ref dir) = self.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        self.temp_dir = None;

        Ok(())
    }
}

impl Drop for CdpEngine {
    fn drop(&mut self) {
        // Best-effort cleanup if shutdown() wasn't called
        if let Some(ref mut child) = self.chrome_process {
            let _ = child.kill();
        }
        if let Some(ref dir) = self.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

// ---------------------------------------------------------------------------
// Background WebSocket reader
// ---------------------------------------------------------------------------

async fn cdp_reader_loop(
    mut ws_rx: SplitStream<WsStream>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    event_waiters: Arc<Mutex<HashMap<String, Vec<oneshot::Sender<Value>>>>>,
    last_url: Arc<Mutex<Option<String>>>,
) {
    while let Some(msg) = ws_rx.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Err(e) => {
                warn!(error = %e, "CDP WebSocket error");
                break;
            }
            _ => continue,
        };

        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Response to a command (has "id" field)
        if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
            let mut pending = pending.lock().await;
            if let Some(tx) = pending.remove(&id) {
                let _ = tx.send(parsed);
            }
            continue;
        }

        // Event (has "method" field)
        if let Some(method) = parsed.get("method").and_then(|v| v.as_str()) {
            let params = parsed.get("params").cloned().unwrap_or(json!({}));

            // Track Page.frameNavigated — update last_url when navigation happens
            if method == "Page.frameNavigated" {
                if let Some(frame) = params.get("frame") {
                    // Only update for the top-level frame (no parentId)
                    if frame.get("parentId").is_none() {
                        if let Some(url) = frame.get("url").and_then(|v| v.as_str()) {
                            debug!(url = %url, "Page.frameNavigated — updating last_url");
                            *last_url.lock().await = Some(url.to_string());
                        }
                    }
                }
            }

            let mut waiters = event_waiters.lock().await;
            if let Some(senders) = waiters.remove(method) {
                for tx in senders {
                    let _ = tx.send(params.clone());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Chrome discovery helpers
// ---------------------------------------------------------------------------

/// Find the Chrome/Chromium binary on this system.
fn find_chrome_binary() -> Option<String> {
    // Check CHROME_PATH env var first
    if let Ok(path) = std::env::var("CHROME_PATH") {
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Chromium\Application\chrome.exe",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return Some(c.to_string());
            }
        }
        // Check LOCALAPPDATA
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = format!(r"{local}\Google\Chrome\Application\chrome.exe");
            if std::path::Path::new(&p).exists() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return Some(c.to_string());
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ];
        for c in &candidates {
            if std::process::Command::new("which")
                .arg(c)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(c.to_string());
            }
        }
    }

    None
}

/// Poll the Chrome DevTools HTTP endpoint until it responds with the WebSocket URL.
async fn wait_for_devtools(port: u16) -> BrowserResult<String> {
    let targets_url = format!("http://127.0.0.1:{port}/json");
    let version_url = format!("http://127.0.0.1:{port}/json/version");
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > CHROME_STARTUP_TIMEOUT {
            return Err(BrowserError::LaunchFailed(format!(
                "Chrome did not start within {}s — is port {port} available?",
                CHROME_STARTUP_TIMEOUT.as_secs()
            )));
        }

        // First check if Chrome is ready via /json/version
        let version_ok = reqwest::get(&version_url).await.map(|r| r.status().is_success()).unwrap_or(false);
        if !version_ok {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }

        // Get page targets from /json (NOT /json/version which is browser-level)
        // Page.enable only works on page targets, not browser targets
        match reqwest::get(&targets_url).await {
            Ok(resp) if resp.status().is_success() => {
                let targets: Vec<Value> = resp.json().await.map_err(|e| {
                    BrowserError::LaunchFailed(format!("parse /json targets: {e}"))
                })?;

                // Find a page-type target
                let page_target = targets.iter().find(|t| {
                    t.get("type").and_then(|v| v.as_str()) == Some("page")
                });

                if let Some(target) = page_target {
                    let ws_url = target
                        .get("webSocketDebuggerUrl")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            BrowserError::LaunchFailed(
                                "Page target has no webSocketDebuggerUrl".into(),
                            )
                        })?
                        .to_string();
                    debug!(ws_url = %ws_url, targets = targets.len(), "Connected to page target");
                    return Ok(ws_url);
                }

                // No page target yet — Chrome may still be initializing
                debug!(targets = targets.len(), "No page target yet, waiting...");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            _ => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Key mapping helper
// ---------------------------------------------------------------------------

/// Map a human-readable key name to CDP (key, code, virtualKeyCode).
fn key_to_cdp_key(key: &str) -> (&str, &str, u32) {
    match key.to_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13),
        "tab" => ("Tab", "Tab", 9),
        "escape" | "esc" => ("Escape", "Escape", 27),
        "backspace" => ("Backspace", "Backspace", 8),
        "delete" => ("Delete", "Delete", 46),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39),
        "space" | " " => (" ", "Space", 32),
        "home" => ("Home", "Home", 36),
        "end" => ("End", "End", 35),
        "pageup" => ("PageUp", "PageUp", 33),
        "pagedown" => ("PageDown", "PageDown", 34),
        _ => (key, key, 0),
    }
}

// ---------------------------------------------------------------------------
// Snapshot JavaScript
// ---------------------------------------------------------------------------

/// JavaScript injected via Runtime.evaluate to build an agent-readable DOM
/// snapshot. Produces the SAME format as the Sevro engine:
/// - Sequential @ref numbering starting at 1
/// - Role mapping: a→link, button→button, input→type, select→combobox, textarea→textbox
/// - Assigns `data-wraith-ref` attributes for action targeting
const SNAPSHOT_JS: &str = r#"(() => {
    const elements = [];
    let refId = 0;

    // Selectors for interactive + semantic elements (same set as Sevro)
    const interactiveSelectors = [
        'a[href]',
        'button',
        'input',
        'select',
        'textarea',
        '[role="button"]',
        '[role="link"]',
        '[role="textbox"]',
        '[role="combobox"]',
        '[role="checkbox"]',
        '[role="radio"]',
        '[role="tab"]',
        '[role="menuitem"]',
        '[contenteditable="true"]',
    ].join(',');

    // Helper: is the element visible?
    function isVisible(el) {
        if (!el.offsetParent && el.tagName !== 'BODY' && el.tagName !== 'HTML') {
            const style = window.getComputedStyle(el);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            if (style.position !== 'fixed' && style.position !== 'sticky') return false;
        }
        const rect = el.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) return false;
        return true;
    }

    // Helper: get visible text content (shallow, not deep)
    function getTextContent(el) {
        // For inputs, return placeholder or empty
        if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
            return '';
        }
        // For select, return selected option text
        if (el.tagName === 'SELECT') {
            const opt = el.options[el.selectedIndex];
            return opt ? opt.textContent.trim() : '';
        }
        // Get direct text, trimmed, max 200 chars
        const text = (el.innerText || el.textContent || '').trim();
        return text.length > 200 ? text.substring(0, 200) + '...' : text;
    }

    // Helper: map tag to role (same as Sevro engine)
    function getRole(el) {
        // Check explicit ARIA role first
        const ariaRole = el.getAttribute('role');
        if (ariaRole) return ariaRole;

        const tag = el.tagName.toLowerCase();
        switch (tag) {
            case 'a': return 'link';
            case 'button': return 'button';
            case 'input': return el.getAttribute('type') || 'textbox';
            case 'select': return 'combobox';
            case 'textarea': return 'textbox';
            default:
                if (el.contentEditable === 'true') return 'textbox';
                return tag;
        }
    }

    // Helper: build a simple CSS selector for the element
    function buildSelector(el) {
        const tag = el.tagName.toLowerCase();
        if (el.id) return tag + '#' + CSS.escape(el.id);
        const cls = Array.from(el.classList).slice(0, 2).map(c => '.' + CSS.escape(c)).join('');
        return tag + cls;
    }

    // Walk all interactive elements
    const nodes = document.querySelectorAll(interactiveSelectors);
    for (const el of nodes) {
        if (!isVisible(el)) continue;

        refId++;

        // Tag element with data-wraith-ref for action targeting
        el.setAttribute('data-wraith-ref', String(refId));

        const rect = el.getBoundingClientRect();
        const isDisabled = el.disabled ||
            el.getAttribute('aria-disabled') === 'true' ||
            el.hasAttribute('readonly');

        const entry = {
            ref_id: refId,
            role: getRole(el),
            text: getTextContent(el) || null,
            href: el.getAttribute('href') || null,
            placeholder: el.getAttribute('placeholder') || null,
            value: (el.value !== undefined && el.value !== '') ? el.value : null,
            enabled: !isDisabled,
            visible: true,
            aria_label: el.getAttribute('aria-label') || null,
            selector: buildSelector(el),
            bounds: {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            },
        };

        elements.push(entry);
    }

    // --- Also capture semantic heading + landmark text ---
    const headings = document.querySelectorAll('h1, h2, h3');
    for (const h of headings) {
        if (!isVisible(h)) continue;
        const text = (h.innerText || h.textContent || '').trim();
        if (!text) continue;
        refId++;
        h.setAttribute('data-wraith-ref', String(refId));
        const rect = h.getBoundingClientRect();
        elements.push({
            ref_id: refId,
            role: h.tagName.toLowerCase(),
            text: text.length > 200 ? text.substring(0, 200) + '...' : text,
            href: null,
            placeholder: null,
            value: null,
            enabled: true,
            visible: true,
            aria_label: null,
            selector: buildSelector(h),
            bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        });
    }

    // --- Page metadata ---
    const forms = document.querySelectorAll('form');
    const hasLoginForm = Array.from(forms).some(f => {
        const html = f.innerHTML.toLowerCase();
        return html.includes('password') || html.includes('login') || html.includes('sign in');
    });
    const hasCaptcha = !!(
        document.querySelector('[class*="captcha"], [id*="captcha"], [class*="recaptcha"], iframe[src*="recaptcha"], iframe[src*="hcaptcha"]')
    );
    const description = document.querySelector('meta[name="description"]');
    const mainContent = document.querySelector('main, article, [role="main"]');
    const contentPreview = mainContent
        ? (mainContent.innerText || '').trim().substring(0, 500)
        : null;

    // --- Overlay detection (matches Sevro's __wraith_detect_overlays) ---
    const overlays = [];
    const overlaySelectors = [
        '[role="dialog"]',
        '[role="alertdialog"]',
        '[class*="modal"]',
        '[class*="overlay"]',
        '[class*="popup"]',
        '[class*="cookie"]',
    ];
    for (const sel of overlaySelectors) {
        for (const el of document.querySelectorAll(sel)) {
            if (!isVisible(el)) continue;
            const refAttr = el.getAttribute('data-wraith-ref');
            overlays.push({
                ref_id: refAttr || '?',
                type: el.getAttribute('role') || 'modal',
                title: (el.getAttribute('aria-label') || el.querySelector('h1,h2,h3')?.textContent || '').trim().substring(0, 100),
            });
        }
    }

    return JSON.stringify({
        url: window.location.href,
        title: document.title,
        elements: elements,
        meta: {
            page_type: null,
            main_content_preview: contentPreview,
            description: description ? description.getAttribute('content') : null,
            form_count: forms.length,
            has_login_form: hasLoginForm,
            has_captcha: hasCaptcha,
            overlays: overlays,
        },
    });
})()"#;
