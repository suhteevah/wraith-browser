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

    /// Attach to an already-running Chrome via `--remote-debugging-port`.
    ///
    /// Unlike `with_port`, this does NOT spawn Chrome — it expects the user
    /// to be running their daily browser with the debug port exposed:
    ///
    /// ```text
    /// chrome.exe --remote-debugging-port=9222
    /// ```
    ///
    /// The attached browser keeps its real fingerprint, cookies, history,
    /// installed extensions, and active sessions — so anti-bot systems like
    /// reCAPTCHA v3 score it as a real user instead of headless Chrome.
    ///
    /// `target_filter` — if `Some`, find an existing page tab whose URL or
    /// title (case-insensitive) contains this string; useful for attaching
    /// to a tab where the user is already authenticated. If `None`, attach
    /// to the first page target Chrome returns (typically the active tab).
    ///
    /// On shutdown, the engine releases its WebSocket but does NOT kill the
    /// Chrome process — that's the operator's browser.
    pub async fn attach(port: u16, target_filter: Option<String>) -> BrowserResult<Self> {
        // Probe /json/version — fast-fail with a clear error if Chrome isn't
        // running with the debug port exposed.
        let version_url = format!("http://127.0.0.1:{port}/json/version");
        let version_resp = reqwest::get(&version_url).await.map_err(|e| {
            BrowserError::LaunchFailed(format!(
                "Cannot reach Chrome at 127.0.0.1:{port} — {e}. Start Chrome with `--remote-debugging-port={port}`."
            ))
        })?;
        if !version_resp.status().is_success() {
            return Err(BrowserError::LaunchFailed(format!(
                "Chrome /json/version on port {port} returned HTTP {}.",
                version_resp.status()
            )));
        }
        let version_info: Value = version_resp.json().await.map_err(|e| {
            BrowserError::LaunchFailed(format!("Parse /json/version: {e}"))
        })?;
        let browser_label = version_info
            .get("Browser")
            .and_then(|v| v.as_str())
            .unwrap_or("Chrome")
            .to_string();
        info!(port, browser = %browser_label, target_filter = ?target_filter, "Attaching to running Chrome");

        // List page targets
        let targets_url = format!("http://127.0.0.1:{port}/json");
        let targets: Vec<Value> = reqwest::get(&targets_url)
            .await
            .map_err(|e| BrowserError::LaunchFailed(format!("Fetch /json: {e}")))?
            .json()
            .await
            .map_err(|e| BrowserError::LaunchFailed(format!("Parse /json: {e}")))?;

        let page_targets: Vec<&Value> = targets
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .collect();

        if page_targets.is_empty() {
            return Err(BrowserError::LaunchFailed(format!(
                "Chrome on port {port} has no page targets. Open at least one tab and retry."
            )));
        }

        // Pick a target
        let chosen = if let Some(filter) = &target_filter {
            let needle = filter.to_lowercase();
            let matched = page_targets.iter().find(|t| {
                let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                url.contains(&needle) || title.contains(&needle)
            });
            match matched {
                Some(t) => *t,
                None => {
                    let available: Vec<String> = page_targets
                        .iter()
                        .map(|t| {
                            format!(
                                "  [{}] {}",
                                t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                t.get("url").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect();
                    return Err(BrowserError::LaunchFailed(format!(
                        "No tab matched filter '{filter}'. Available tabs:\n{}",
                        available.join("\n")
                    )));
                }
            }
        } else {
            page_targets[0]
        };

        let ws_url = chosen
            .get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BrowserError::LaunchFailed("Chosen tab has no webSocketDebuggerUrl".into())
            })?
            .to_string();
        let tab_url = chosen
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tab_title = chosen
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        info!(ws = %ws_url, title = %tab_title, url = %tab_url, "Attaching to tab");

        // Connect WebSocket (same as with_port from here down, but no chrome_process / temp_dir)
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
        let last_url: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(Some(tab_url.clone())));

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
            chrome_process: None, // attached — don't kill on shutdown
            temp_dir: None,       // attached — no temp dir to clean
            last_url,
            port,
        };

        // Enable required CDP domains
        engine.send_cdp_command("Page.enable", json!({})).await?;
        engine.send_cdp_command("Runtime.enable", json!({})).await?;
        engine.send_cdp_command("DOM.enable", json!({})).await?;

        info!("CDP engine attached and ready");
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

    /// Resolve a `data-wraith-ref` to a viewport-relative click point (center
    /// of its bounding box). Scrolls the element into view first so the click
    /// lands inside the viewport. Returns `None` if the element is missing or
    /// has zero-size geometry.
    async fn cdp_resolve_ref_point(&self, ref_id: u32) -> BrowserResult<Option<(f64, f64)>> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                if (!el) return JSON.stringify({{ok: false, reason: 'not_found'}});
                el.scrollIntoView({{block: 'center', inline: 'center'}});
                const r = el.getBoundingClientRect();
                if (r.width === 0 && r.height === 0) {{
                    return JSON.stringify({{ok: false, reason: 'zero_size'}});
                }}
                return JSON.stringify({{
                    ok: true,
                    x: r.left + r.width / 2,
                    y: r.top + r.height / 2,
                }});
            }})()"#
        );
        let raw = self.runtime_evaluate(&js).await?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| BrowserError::EngineError(format!("cdp_resolve_ref_point parse: {e} :: {raw}")))?;
        if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
            return Ok(None);
        }
        let x = v.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = v.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        Ok(Some((x, y)))
    }

    /// Dispatch a real mouse click at viewport-relative (x, y) using
    /// `Input.dispatchMouseEvent`. Produces a real trusted event sequence —
    /// `mouseMoved` (so :hover styles settle), then `mousePressed` +
    /// `mouseReleased` — so React's delegated `onMouseDown` listeners (used by
    /// react-select, Radix, MUI, Headless UI, Ariakit, etc.) fire.
    ///
    /// `el.click()` only fires the synthetic `click` event, which is why
    /// portal-rendered react-select menus stay closed when driven via JS.
    async fn cdp_dispatch_real_click(&self, x: f64, y: f64) -> BrowserResult<()> {
        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseMoved",
                "x": x,
                "y": y,
                "button": "none",
                "buttons": 0,
            }),
        ).await?;
        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": "left",
                "buttons": 1,
                "clickCount": 1,
            }),
        ).await?;
        self.send_cdp_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseReleased",
                "x": x,
                "y": y,
                "button": "left",
                "buttons": 0,
                "clickCount": 1,
            }),
        ).await?;
        Ok(())
    }

    /// Resolve `ref_id` then dispatch a real CDP mouse click on its center
    /// point. Returns `Ok(true)` if the element was found and clicked,
    /// `Ok(false)` if the element was not found.
    async fn cdp_real_click_at_ref(&self, ref_id: u32) -> BrowserResult<bool> {
        match self.cdp_resolve_ref_point(ref_id).await? {
            Some((x, y)) => {
                self.cdp_dispatch_real_click(x, y).await?;
                Ok(true)
            }
            None => Ok(false),
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

        // --- FR-3: Extract iframe contents via Page.getFrameTree ---
        let mut elements = elements; // make mutable for iframe merging
        let next_ref = elements.iter().map(|e| e.ref_id).max().unwrap_or(0) + 1;
        let mut iframe_ref = next_ref;

        if let Ok(frame_tree_result) = self.send_cdp_command("Page.getFrameTree", json!({})).await {
            if let Some(child_frames) = frame_tree_result
                .get("frameTree")
                .and_then(|ft| ft.get("childFrames"))
                .and_then(|cf| cf.as_array())
            {
                for child in child_frames {
                    let frame_id = child
                        .get("frame")
                        .and_then(|f| f.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let frame_url = child
                        .get("frame")
                        .and_then(|f| f.get("url"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if frame_id.is_empty()
                        || frame_url.is_empty()
                        || frame_url == "about:blank"
                        || frame_url.starts_with("about:")
                    {
                        continue;
                    }

                    // Extract domain for the [iframe: domain] prefix
                    let iframe_domain = url::Url::parse(frame_url)
                        .ok()
                        .and_then(|u| u.host_str().map(|h| h.to_string()))
                        .unwrap_or_else(|| frame_url.to_string());

                    // Create an isolated world in the child frame to run our snapshot JS
                    let world_result = self
                        .send_cdp_command(
                            "Page.createIsolatedWorld",
                            json!({
                                "frameId": frame_id,
                                "worldName": "wraith-snapshot"
                            }),
                        )
                        .await;

                    let context_id = match world_result {
                        Ok(ref wr) => wr
                            .get("executionContextId")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        Err(e) => {
                            debug!(frame_id, error = %e, "Failed to create isolated world for iframe");
                            0
                        }
                    };

                    if context_id == 0 {
                        continue;
                    }

                    // Run the snapshot JS in the iframe's execution context
                    let iframe_eval = self
                        .send_cdp_command(
                            "Runtime.evaluate",
                            json!({
                                "expression": SNAPSHOT_JS,
                                "contextId": context_id,
                                "returnByValue": true,
                                "awaitPromise": true,
                            }),
                        )
                        .await;

                    let iframe_json_str = match iframe_eval {
                        Ok(ref result) => {
                            if result.get("exceptionDetails").is_some() {
                                debug!(frame_id, "Snapshot JS threw in iframe context");
                                continue;
                            }
                            let empty = json!({});
                            let remote_obj = result.get("result").unwrap_or(&empty);
                            match remote_obj.get("value") {
                                Some(Value::String(s)) => s.clone(),
                                Some(other) => other.to_string(),
                                None => continue,
                            }
                        }
                        Err(e) => {
                            debug!(frame_id, error = %e, "Failed to evaluate snapshot in iframe");
                            continue;
                        }
                    };

                    let iframe_raw: Value = match serde_json::from_str(&iframe_json_str) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if let Some(iframe_elements) = iframe_raw
                        .get("elements")
                        .and_then(|v| v.as_array())
                    {
                        for el in iframe_elements {
                            let base_role = el
                                .get("role")
                                .and_then(|v| v.as_str())
                                .unwrap_or("text");
                            let role = format!("[iframe: {iframe_domain}] {base_role}");

                            elements.push(DomElement {
                                ref_id: iframe_ref,
                                role,
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
                            });
                            iframe_ref += 1;
                        }
                    }
                }
            }
        }
        // --- End FR-3 ---

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

                // BR-6: dispatch real CDP mouse events instead of el.click().
                // Synthetic .click() only fires the `click` event — react-select,
                // Radix, Headless UI, MUI all open their menus on `mousedown` so
                // they can preempt focus shift, so .click() leaves their menus
                // closed. Input.dispatchMouseEvent fires the full mouseMoved →
                // mousePressed → mouseReleased trusted-event sequence that React's
                // delegated listeners on the real document pick up natively.
                let clicked = self.cdp_real_click_at_ref(ref_id).await?;
                if !clicked {
                    return Ok(ActionResult::Failed {
                        error: format!("Element @e{ref_id} not found"),
                    });
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
                // BR-8 fix: route fills through CDP `Input.insertText` (real
                // browser input pipeline). The old JS-setter approach had two
                // problems:
                //   (a) the `Object.getOwnPropertyDescriptor(HTMLInputElement
                //       .prototype, 'value') || HTMLTextAreaElement.prototype`
                //       short-circuit always returned truthy from the input
                //       prototype, so calling its setter on a <textarea> threw
                //       `TypeError`. Textareas never filled.
                //   (b) the setter approach updates `el.value` but does NOT
                //       update React's internal state for controlled components
                //       (masked phone fields, react-textarea-autosize, etc.).
                //       The submit handler reads React state → silent bail.
                //
                // `Input.insertText` writes through the real input pipeline:
                // dispatches `beforeinput` + `input` events that React's
                // controlled-component handlers listen to, so React state
                // tracks the DOM. One CDP roundtrip for the whole string —
                // faster than per-char keypress dispatch and framework-agnostic.

                // Step 1: focus the element and select existing content so
                // insertText overwrites instead of appending.
                let prep_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return JSON.stringify({{ok: false, reason: 'not_found'}});
                        const tag = el.tagName;
                        if (tag !== 'INPUT' && tag !== 'TEXTAREA' && el.contentEditable !== 'true') {{
                            return JSON.stringify({{ok: false, reason: 'not_fillable', tag}});
                        }}
                        el.scrollIntoView({{block: 'center', inline: 'center'}});
                        if (typeof el.focus === 'function') el.focus();
                        // Select existing content so insertText replaces it.
                        if (tag === 'INPUT' || tag === 'TEXTAREA') {{
                            try {{
                                el.setSelectionRange(0, (el.value || '').length);
                            }} catch (e) {{
                                // Some input types (number, email, etc.) don't support
                                // setSelectionRange — fall through, insertText will append.
                            }}
                        }}
                        return JSON.stringify({{ok: true, tag, existing: (el.value || '').length}});
                    }})()"#
                );
                let prep_raw = self.runtime_evaluate(&prep_js).await?;
                let prep: Value = serde_json::from_str(&prep_raw)
                    .map_err(|e| BrowserError::EngineError(format!("Fill prep parse: {e} :: {prep_raw}")))?;
                if prep.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    let reason = prep.get("reason").and_then(|v| v.as_str()).unwrap_or("unknown");
                    if reason == "not_found" {
                        return Ok(ActionResult::Failed {
                            error: format!("Element @e{ref_id} not found"),
                        });
                    }
                    return Ok(ActionResult::Failed {
                        error: format!(
                            "Element @e{ref_id} is not fillable ({reason}; tag={})",
                            prep.get("tag").and_then(|v| v.as_str()).unwrap_or("?")
                        ),
                    });
                }
                let had_existing = prep
                    .get("existing")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    > 0;

                // Step 2: if the field had existing content but no selection
                // range support (numeric / email inputs), clear it explicitly
                // via Input.dispatchKeyEvent({type:"rawKeyDown", code:"Delete"})
                // on the selection — skipped for now, the common case is empty
                // fields or text/textarea where setSelectionRange worked.
                let _ = had_existing;

                // Step 3: insert the text via real CDP input pipeline.
                let insert_result = self.send_cdp_command(
                    "Input.insertText",
                    json!({ "text": text }),
                ).await;

                let mut used_fallback = false;
                if let Err(e) = insert_result {
                    debug!(error = %e, "Input.insertText failed; falling back to native setter path");
                    used_fallback = true;
                    // 8a fix in the fallback: branch on tagName for the correct prototype.
                    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                    let fb_js = format!(
                        r#"(() => {{
                            const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                            if (!el) return 'not_found';
                            const proto = el.tagName === 'TEXTAREA'
                                ? window.HTMLTextAreaElement.prototype
                                : window.HTMLInputElement.prototype;
                            const desc = Object.getOwnPropertyDescriptor(proto, 'value');
                            if (desc && desc.set) {{
                                desc.set.call(el, '{escaped}');
                            }} else {{
                                el.value = '{escaped}';
                            }}
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                            return 'ok';
                        }})()"#
                    );
                    let fb_result = self.runtime_evaluate(&fb_js).await?;
                    if fb_result == "not_found" {
                        return Ok(ActionResult::Failed {
                            error: format!("Element @e{ref_id} not found"),
                        });
                    }
                }

                // Step 4: verify the value actually landed. For controlled
                // components, also check React state if the fiber is reachable
                // — that's the real predictor of "will the submit handler
                // accept this?". If DOM matches but React state doesn't, fall
                // back to the setter path which fires the React-friendly input
                // event sequence.
                tokio::time::sleep(Duration::from_millis(50)).await;
                let verify_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return JSON.stringify({{ok: false, reason: 'gone'}});
                        const domValue = el.value !== undefined ? el.value : (el.textContent || '');
                        const propsKey = Object.keys(el).find(k => k.startsWith('__reactProps$'));
                        let reactValue = null;
                        if (propsKey) {{
                            try {{
                                const props = el[propsKey];
                                if (props && 'value' in props) reactValue = props.value;
                            }} catch (e) {{}}
                        }}
                        return JSON.stringify({{ok: true, domValue, reactValue, hasReact: !!propsKey}});
                    }})()"#
                );
                let verify_raw = self.runtime_evaluate(&verify_js).await.unwrap_or_default();
                let verify: Value = serde_json::from_str(&verify_raw).unwrap_or(json!({}));
                let dom_value = verify
                    .get("domValue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let has_react = verify
                    .get("hasReact")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let react_matches = has_react
                    && verify
                        .get("reactValue")
                        .and_then(|v| v.as_str())
                        .map(|s| s == text)
                        .unwrap_or(false);
                let dom_matches = dom_value == text;

                // If DOM and React both agree the value is set, we're done.
                if dom_matches && (!has_react || react_matches) {
                    return Ok(ActionResult::Success {
                        message: format!(
                            "Filled @e{ref_id} ({} chars{})",
                            text.len(),
                            if used_fallback { ", via setter fallback" } else { "" }
                        ),
                    });
                }

                // DOM matches but React state lags — try the setter path with
                // explicit input+change events to nudge controlled components.
                // (insertText already fires these, so this is for rare cases
                // where the component listens for something else.)
                if dom_matches && has_react && !react_matches && !used_fallback {
                    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                    let nudge_js = format!(
                        r#"(() => {{
                            const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                            if (!el) return 'gone';
                            const proto = el.tagName === 'TEXTAREA'
                                ? window.HTMLTextAreaElement.prototype
                                : window.HTMLInputElement.prototype;
                            const desc = Object.getOwnPropertyDescriptor(proto, 'value');
                            if (desc && desc.set) desc.set.call(el, '{escaped}');
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                            return 'ok';
                        }})()"#
                    );
                    let _ = self.runtime_evaluate(&nudge_js).await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    // Re-verify after the nudge.
                    let verify2_raw = self.runtime_evaluate(&verify_js).await.unwrap_or_default();
                    let verify2: Value = serde_json::from_str(&verify2_raw).unwrap_or(json!({}));
                    let dom_after = verify2
                        .get("domValue")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let react_after = verify2
                        .get("reactValue")
                        .and_then(|v| v.as_str())
                        .map(|s| s == text)
                        .unwrap_or(false);
                    if dom_after == text && react_after {
                        return Ok(ActionResult::Success {
                            message: format!(
                                "Filled @e{ref_id} ({} chars, React state nudged via setter)",
                                text.len()
                            ),
                        });
                    }
                    return Ok(ActionResult::Failed {
                        error: format!(
                            "Filled @e{ref_id} DOM-side but React state did not update — controlled component may need per-char keystrokes (dom={dom_after:?}, reactMatch={react_after})"
                        ),
                    });
                }

                // DOM didn't even match — Fill genuinely failed.
                Ok(ActionResult::Failed {
                    error: format!(
                        "Fill @e{ref_id} failed: expected {} chars, got {} chars in DOM (reactState={})",
                        text.len(),
                        dom_value.len(),
                        if has_react {
                            verify
                                .get("reactValue")
                                .and_then(|v| v.as_str())
                                .map(|s| format!("{:?}", s))
                                .unwrap_or_else(|| "null".into())
                        } else {
                            "no-react-fiber".into()
                        }
                    ),
                })
            }

            BrowserAction::Select {
                ref_id,
                value,
                force: _,
            } => {
                // First detect whether this is a native <select> — if so, the
                // .value setter path works fine and is the fastest option.
                let tag_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return 'NOT_FOUND';
                        return el.tagName.toLowerCase();
                    }})()"#
                );
                let tag = self.runtime_evaluate(&tag_js).await?;
                if tag == "NOT_FOUND" {
                    return Ok(ActionResult::Failed {
                        error: format!("Element @e{ref_id} not found"),
                    });
                }

                let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");

                if tag == "select" {
                    let js = format!(
                        r#"(() => {{
                            const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                            if (!el) return JSON.stringify({{error: 'Element @e{ref_id} not found'}});
                            el.value = '{escaped}';
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                            const chosen = el.options[el.selectedIndex];
                            return JSON.stringify({{ok: true, display: chosen ? chosen.textContent.trim() : '{escaped}'}});
                        }})()"#
                    );
                    let _ = self.runtime_evaluate(&js).await?;
                    return Ok(ActionResult::Success {
                        message: format!("SELECTED: {value} on @e{ref_id}"),
                    });
                }

                // --- React / custom dropdown (BR-6 fix) ---
                // Step 1: open the menu by dispatching a real CDP mouse click
                // on the trigger. el.click() fires only the synthetic click
                // event; react-select / Radix / Headless UI listen for
                // mousedown so they can preempt focus shift, so a JS click
                // leaves their menus closed.
                if !self.cdp_real_click_at_ref(ref_id).await? {
                    return Ok(ActionResult::Failed {
                        error: format!("Element @e{ref_id} not found"),
                    });
                }

                // Step 2: wait for the option list to render (portal-rendered
                // menus mount async after mousedown). Poll up to ~1.5s.
                //
                // BR-7 fix: scope the option search to a visibly-open menu
                // container (`[role="listbox"]` or `.select__menu` /
                // `.dropdown-menu`) — NOT document-wide. Use exact match on
                // `data-value` / trimmed text (case-insensitive). Substring
                // matching would catch unrelated page text (e.g. Greenhouse's
                // AI-policy paragraph "...by selecting 'Yes.'") and silently
                // click dead air, returning a false-positive SELECTED.
                let mut option_coords: Option<(f64, f64, String)> = None;
                for _ in 0..15 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let find_js = format!(
                        r#"(() => {{
                            const valueLower = '{escaped}'.toLowerCase();
                            // Find a visibly-open menu container first. If no menu
                            // is open, do NOT search the document — that's how false
                            // positives slip in.
                            const menuContainers = Array.from(document.querySelectorAll(
                                '[role="listbox"], .select__menu, .select__menu-list, '
                                + '[class*="MenuList"], [class*="Menu"][class*="open"], '
                                + '[class*="dropdown-menu"][class*="show"], '
                                + '[class*="popper"], [data-state="open"]'
                            )).filter(m => {{
                                const r = m.getBoundingClientRect();
                                return r.width > 0 && r.height > 0;
                            }});
                            if (menuContainers.length === 0) {{
                                return JSON.stringify({{ok: false, reason: 'menu_not_open'}});
                            }}

                            // Search options as descendants of the open menu only.
                            const optionSelectors = [
                                '[role="option"]',
                                '[class*="select__option"]',
                                '[class*="MenuItem"]',
                                'li[role="menuitem"]',
                                'li',
                            ].join(',');
                            const seen = new Set();
                            for (const menu of menuContainers) {{
                                for (const opt of menu.querySelectorAll(optionSelectors)) {{
                                    if (seen.has(opt)) continue;
                                    seen.add(opt);
                                    const r = opt.getBoundingClientRect();
                                    if (r.width === 0 || r.height === 0) continue;
                                    const text = (opt.innerText || opt.textContent || '').trim();
                                    const tl = text.toLowerCase();
                                    const dv = (opt.getAttribute('data-value') || '').toLowerCase();
                                    // Exact match only — substring match false-positives on
                                    // option-shaped DOM that happens to contain the value
                                    // string somewhere in its descendants.
                                    if (tl === valueLower || dv === valueLower) {{
                                        return JSON.stringify({{
                                            ok: true,
                                            x: r.left + r.width / 2,
                                            y: r.top + r.height / 2,
                                            display: text,
                                        }});
                                    }}
                                }}
                            }}
                            return JSON.stringify({{ok: false, reason: 'no_exact_match'}});
                        }})()"#
                    );
                    let raw = self.runtime_evaluate(&find_js).await?;
                    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                        if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                            let x = v.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let y = v.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let display = v
                                .get("display")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&value)
                                .to_string();
                            option_coords = Some((x, y, display));
                            break;
                        }
                    }
                }

                let display = match option_coords {
                    Some((x, y, disp)) => {
                        // Step 3: click the option with a real CDP mouse event.
                        self.cdp_dispatch_real_click(x, y).await?;
                        disp
                    }
                    None => {
                        // Fallback: try a native .value set on the trigger in
                        // case it's a non-portal custom dropdown that accepts it.
                        let fb_js = format!(
                            r#"(() => {{
                                const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                                if (!el) return '';
                                if ('value' in el) {{
                                    el.value = '{escaped}';
                                    el.dispatchEvent(new Event('change', {{bubbles: true}}));
                                }}
                                return '{escaped}';
                            }})()"#
                        );
                        let _ = self.runtime_evaluate(&fb_js).await?;
                        return Ok(ActionResult::Failed {
                            error: format!(
                                "Could not find option '{value}' in the dropdown opened from @e{ref_id} — menu may not have rendered or option label may not match"
                            ),
                        });
                    }
                };

                // Wait for React re-render after committing the option
                tokio::time::sleep(Duration::from_millis(200)).await;

                // BR-7 fix: verify the commit actually landed before reporting
                // SELECTED. React-select commits manifest as one of:
                //   - the menu closing (no visible .select__menu / [role="listbox"])
                //   - a .select__single-value (or similar committed-value node)
                //     rendering the option text
                //   - the trigger's text/aria-valuenow/aria-label updating to
                //     match the option label
                // If none of those are true, the click hit nothing real and we
                // must NOT return Success — agents downstream would submit a
                // half-filled form thinking we'd succeeded.
                let verify_js = format!(
                    r#"(() => {{
                        const el = document.querySelector('[data-wraith-ref="{ref_id}"]');
                        if (!el) return JSON.stringify({{ok: false, reason: 'trigger_gone'}});
                        const valueLower = '{escaped}'.toLowerCase();

                        // Check for a committed-value display node within the
                        // combobox subtree.
                        const single = el.querySelector(
                            '[class*="singleValue"], [class*="single-value"], '
                            + '[class*="selectedValue"], [class*="selected-value"]'
                        );
                        if (single) {{
                            const t = (single.innerText || single.textContent || '').trim();
                            if (t.toLowerCase() === valueLower) {{
                                return JSON.stringify({{ok: true, via: 'single_value', display: t}});
                            }}
                        }}

                        // Check aria-valuenow / aria-label.
                        const aria = (
                            el.getAttribute('aria-valuenow')
                            || el.getAttribute('aria-label')
                            || ''
                        ).trim();
                        if (aria.toLowerCase() === valueLower) {{
                            return JSON.stringify({{ok: true, via: 'aria', display: aria}});
                        }}

                        // Check the trigger's own visible text (last resort —
                        // some Headless UI variants render the option label
                        // directly inside the trigger).
                        const triggerText = (el.innerText || el.textContent || '').trim();
                        if (triggerText.toLowerCase() === valueLower
                            || triggerText.toLowerCase().endsWith(valueLower)) {{
                            return JSON.stringify({{ok: true, via: 'trigger_text', display: triggerText}});
                        }}

                        // Menu still open and no commit — that means our click
                        // hit dead air or the wrong element.
                        const menuOpen = document.querySelectorAll(
                            '[role="listbox"], .select__menu'
                        ).length > 0;

                        return JSON.stringify({{
                            ok: false,
                            reason: menuOpen ? 'menu_still_open' : 'no_commit_indicator',
                            triggerText,
                        }});
                    }})()"#
                );
                let verify_raw = self.runtime_evaluate(&verify_js).await.unwrap_or_default();
                let verify: Value = serde_json::from_str(&verify_raw).unwrap_or(json!({}));
                if verify.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    let reason = verify
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    return Ok(ActionResult::Failed {
                        error: format!(
                            "Clicked option '{display}' for @e{ref_id} but no commit indicator appeared ({reason}). Trigger text after click: {:?}",
                            verify.get("triggerText").and_then(|v| v.as_str()).unwrap_or("")
                        ),
                    });
                }

                let shown = verify
                    .get("display")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| if !display.is_empty() { display.clone() } else { value.clone() });

                Ok(ActionResult::Success {
                    message: format!("SELECTED: {shown} on @e{ref_id}"),
                })
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

        let attached_mode = self.chrome_process.is_none();
        if attached_mode {
            // Attach mode: this is the operator's daily Chrome. Just disconnect
            // our WebSocket — do NOT send Browser.close (kills the whole window)
            // and do NOT kill any process (we didn't spawn one).
            info!("Attach-mode shutdown — leaving operator Chrome running");
            return Ok(());
        }

        // Try graceful browser close (only valid for spawned Chrome)
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
        // Best-effort cleanup if shutdown() wasn't called. Only act on
        // spawned-Chrome mode; attach mode (chrome_process == None) is a no-op
        // so the operator's daily browser keeps running.
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

    // Selectors for interactive + semantic elements (same set as Sevro,
    // plus role="option" / role="listbox" so portal-rendered react-select /
    // Radix / Headless UI menus get @refs once opened).
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
        '[role="option"]',
        '[role="listbox"]',
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

    // Helper: get combobox display value for custom dropdowns (React, etc.)
    // Native selects use .value; custom dropdowns render text as child nodes.
    function getComboboxValue(el) {
        // 1. Native select / input — .value works
        if (el.tagName === 'SELECT') {
            const opt = el.options[el.selectedIndex];
            return opt ? opt.textContent.trim() : (el.value || '');
        }
        if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
            return el.value || '';
        }
        // 2. Check .value if the element has one (some custom components set it)
        if (typeof el.value === 'string' && el.value !== '') {
            return el.value;
        }
        // 3. ARIA attributes that carry the current value
        const ariaVal = el.getAttribute('aria-valuenow')
            || el.getAttribute('aria-valuetext')
            || el.getAttribute('data-value');
        if (ariaVal) return ariaVal;
        // 4. Child span / div with value-like class (React Select, Radix, MUI, etc.)
        const valueChild = el.querySelector(
            '[class*="singleValue"], [class*="value"], [class*="selected"], '
            + '[class*="placeholder"]:not([class*="hidden"]), '
            + 'span:first-child, [class*="trigger"] > span'
        );
        if (valueChild) {
            const t = (valueChild.innerText || valueChild.textContent || '').trim();
            if (t) return t;
        }
        // 5. Direct textContent of the trigger element
        const directText = (el.innerText || el.textContent || '').trim();
        if (directText) return directText;
        // 6. Check for a hidden input inside / associated with the combobox
        const hidden = el.querySelector('input[type="hidden"]') || el.querySelector('input');
        if (hidden && hidden.value) return hidden.value;
        return '';
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

    // Helper: build a CSS selector for the element (includes name/type for playbook matching)
    function buildSelector(el) {
        const tag = el.tagName.toLowerCase();
        if (el.id) return tag + '#' + CSS.escape(el.id);
        // For form fields, include name and type attributes so playbook selectors match
        const attrs = [];
        if (el.getAttribute('name')) attrs.push('[name=\"' + el.getAttribute('name') + '\"]');
        if (tag === 'input' && el.getAttribute('type') && el.getAttribute('type') !== 'text')
            attrs.push('[type=\"' + el.getAttribute('type') + '\"]');
        if (el.getAttribute('data-field')) attrs.push('[data-field=\"' + el.getAttribute('data-field') + '\"]');
        if (attrs.length > 0) return tag + attrs.join('');
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

        const role = getRole(el);
        // For combobox-role elements, use smart value detection that handles
        // both native <select> and React/custom dropdowns
        const isCombobox = role === 'combobox'
            || el.getAttribute('role') === 'combobox'
            || el.getAttribute('role') === 'listbox'
            || (el.className && typeof el.className === 'string' && /select|dropdown|combobox/i.test(el.className));
        const rawValue = isCombobox
            ? (getComboboxValue(el) || null)
            : ((el.value !== undefined && el.value !== '') ? el.value : null);

        const entry = {
            ref_id: refId,
            role: role,
            text: getTextContent(el) || null,
            href: el.getAttribute('href') || null,
            placeholder: el.getAttribute('placeholder') || null,
            value: rawValue,
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
