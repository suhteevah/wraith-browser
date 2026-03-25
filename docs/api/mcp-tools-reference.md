# Wraith Browser — MCP Tools Reference

> Complete reference for all MCP (Model Context Protocol) tools exposed by the Wraith Browser server.
> Generated from source: `crates/mcp-server/src/server.rs` and `crates/mcp-server/src/tools.rs`

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Transport & Configuration](#transport--configuration)
3. [Tool Annotations (Permission Model)](#tool-annotations-permission-model)
4. [Navigation & Core Browsing](#1-navigation--core-browsing)
5. [Interaction & Form Control](#2-interaction--form-control)
6. [Content Extraction & DOM](#3-content-extraction--dom)
7. [DOM Manipulation](#4-dom-manipulation)
8. [Credential Vault](#5-credential-vault)
9. [Cookie Management](#6-cookie-management)
10. [Knowledge Cache](#7-knowledge-cache)
11. [Knowledge Graph (Entities)](#8-knowledge-graph-entities)
12. [Embeddings & Semantic Search](#9-embeddings--semantic-search)
13. [Authentication Detection](#10-authentication-detection)
14. [Browser Fingerprinting & Identity](#11-browser-fingerprinting--identity)
15. [Network Intelligence](#12-network-intelligence)
16. [TLS & Security](#13-tls--security)
17. [Session Management (CDP)](#14-session-management-cdp)
18. [Plugins (WASM)](#15-plugins-wasm)
19. [Scripting (Rhai)](#16-scripting-rhai)
20. [Telemetry & Monitoring](#17-telemetry--monitoring)
21. [Workflow Recording & Replay](#18-workflow-recording--replay)
22. [Time-Travel Debugging](#19-time-travel-debugging)
23. [Task DAG Orchestration](#20-task-dag-orchestration)
24. [Planning & Prediction](#21-planning--prediction)
25. [Parallel Browsing / Swarm](#22-parallel-browsing--swarm)
26. [Playbook Automation](#23-playbook-automation)
27. [Deduplication & Verification](#24-deduplication--verification)
28. [Environment Variables](#environment-variables)
29. [Feature Flags](#feature-flags)
30. [Engine Architecture](#engine-architecture)

---

## Architecture Overview

The Wraith MCP server exposes browser automation capabilities through the [Model Context Protocol](https://modelcontextprotocol.io). The server is implemented in `crates/mcp-server/` and backed by the `BrowserEngine` trait from `crates/browser-core/`.

```
┌─────────────────────────────────────────────────────────┐
│  AI Agent (Claude Code, Cursor, etc.)                   │
│  ↕ MCP Protocol (JSON-RPC over stdio)                   │
├─────────────────────────────────────────────────────────┤
│  WraithHandler (crates/mcp-server/src/server.rs)        │
│  ├── Tool Registry (143 tools)                          │
│  ├── dispatch_tool() — giant match on tool name         │
│  ├── Session Manager (named sessions, active routing)   │
│  └── Dedup Tracker (SQLite-backed)                      │
├─────────────────────────────────────────────────────────┤
│  BrowserEngine Trait (crates/browser-core/src/engine.rs)│
│  ├── SevroEngine  — Servo + QuickJS (default)           │
│  ├── NativeEngine — Pure HTTP, no JS (~50ms/page)       │
│  └── CdpEngine    — Chrome DevTools Protocol            │
├─────────────────────────────────────────────────────────┤
│  Supporting Crates                                      │
│  ├── wraith-identity    — AES-256-GCM encrypted vault │
│  ├── wraith-cache       — Knowledge cache + dedup DB  │
│  ├── wraith-search      — Web metasearch engine       │
│  ├── wraith-content-extract — HTML→Markdown           │
│  ├── wraith-scripting   — Rhai script engine          │
│  └── wraith-agent-loop  — Autonomous agent execution  │
└─────────────────────────────────────────────────────────┘
```

### Handler Struct

```rust
pub struct WraithHandler {
    tools: Vec<Tool>,                              // All registered MCP tools
    engine: Arc<Mutex<dyn BrowserEngine>>,          // Primary engine (Sevro/Native)
    #[cfg(feature = "cdp")]
    cdp_engine: Option<Arc<Mutex<dyn BrowserEngine>>>,
    #[cfg(feature = "cdp")]
    active_cdp_session: Arc<Mutex<Option<Arc<Mutex<dyn BrowserEngine>>>>>,
    #[cfg(feature = "cdp")]
    sessions: Arc<tokio::sync::Mutex<HashMap<String, Arc<Mutex<dyn BrowserEngine>>>>>,
    #[cfg(feature = "cdp")]
    active_session_name: Arc<tokio::sync::Mutex<String>>,
    dedup_tracker: Arc<wraith_cache::dedup::ApplicationTracker>,
}
```

---

## Transport & Configuration

### MCP Configuration (`.mcp.json`)

```json
{
  "mcpServers": {
    "wraith-browser": {
      "command": "path/to/wraith-browser.exe",
      "args": ["serve", "--transport", "stdio"],
      "description": "Wraith Browser — AI-agent-first web browser"
    }
  }
}
```

### Supported Transports

| Transport | Status | Description |
|-----------|--------|-------------|
| `stdio`   | Stable | Standard I/O via rmcp crate. Primary transport for Claude Code. |

### Server Info

```json
{
  "name": "wraith-browser",
  "version": "0.1.0",
  "capabilities": {
    "tools": { "listChanged": false }
  }
}
```

---

## Tool Annotations (Permission Model)

Every tool carries annotation metadata indicating its access level:

| Annotation | `read_only` | `destructive` | `open_world` | Description |
|------------|-------------|---------------|--------------|-------------|
| `ro_closed` | true | false | false | Read-only, local only (safest) |
| `ro_open`   | true | false | true  | Read-only but accesses the network |
| `rw_closed` | false | false | false | Can modify local state |
| `rw_open`   | false | false | true  | Can modify state + access network |
| `rw_destructive` | false | true | false | Permanent/irreversible modifications |

---

## 1. Navigation & Core Browsing

### `browse_navigate`

Navigate to a URL using the native engine (Sevro). Returns a DOM snapshot with interactive elements, each identified by `@ref` IDs.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | Full URL including protocol (e.g., `https://example.com`) |
| `wait_for_load` | bool | no | `true` | Wait for page load before returning snapshot |

**Annotations:** `rw_open`

**Behavior:**
- Clears any active CDP session — switches back to native engine
- Sets active session to `"native"`
- If `cdp_auto` is enabled and the native snapshot has < 5 interactive elements, automatically falls back to CDP (SPA detection)
- Returns DOM snapshot via `snapshot.to_agent_text()`

**Response format:**
```
Page title: Example
URL: https://example.com

Interactive elements:
@e1 [link] "Home" href=/
@e2 [input:text] placeholder="Search..."
@e3 [button] "Submit"
...
```

---

### `browse_navigate_cdp`

Navigate using Chrome DevTools Protocol for JavaScript-heavy pages (React SPAs, Angular apps). **Requires `cdp` feature flag.**

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | Full URL including protocol |
| `wait_for_load` | bool | no | `true` | Wait for full JS rendering |

**Annotations:** `rw_open`

**Behavior:**
- Lazily launches a Chrome instance via `CdpEngine::new()`
- Stores the CDP engine as active session `"cdp"`
- All subsequent `browse_*` commands route to CDP until `browse_navigate` (native) is called
- Returns snapshot prefixed with `[CDP engine active — all browse_* commands now use Chrome]`

**Error:** If Chrome is not installed, returns: `CDP engine launch failed: ... Ensure Chrome is installed.`

---

### `browse_back`

Go back to the previous page in browser history.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_open`

**Behavior:** Executes `BrowserAction::GoBack`. If navigation occurs, returns new page snapshot. Otherwise returns action result text.

---

### `browse_forward`

Go forward in browser history.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_open`

---

### `browse_reload`

Reload the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_open`

---

### `browse_scroll`

Scroll the current page in a given direction.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `direction` | string | **yes** | — | `"up"`, `"down"`, `"left"`, or `"right"` |
| `amount` | integer | no | `500` | Pixels to scroll |

**Annotations:** `rw_closed`

---

### `browse_scroll_to`

Scroll the viewport to center a specific element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the element to scroll into view |

**Annotations:** `rw_closed`

---

### `browse_wait`

Wait for a CSS selector to appear or a fixed time.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `selector` | string | no | — | CSS selector to wait for (e.g., `"#results"`, `".job-card"`) |
| `ms` | integer | no | `1000` (fixed) / `5000` (selector timeout) | Milliseconds |

**Annotations:** `ro_closed`

**Behavior:**
- If `selector` is provided: executes `BrowserAction::WaitForSelector` with timeout
- If no selector: executes `BrowserAction::Wait` for fixed duration

---

### `browse_wait_navigation`

Wait for navigation to complete after a click or form submission.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `timeout_ms` | integer | no | `5000` | Maximum wait time in milliseconds |

**Annotations:** `ro_closed`

---

### `browse_tabs`

Show the current page URL and title.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Response format:**
```json
{
  "current_tab": {
    "url": "https://example.com/page",
    "title": "Example Page"
  }
}
```

---

### `browse_config`

Show current engine configuration (engine type, proxy, TLS status).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `browse_engine_status`

Check which browser engine is currently active.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** `"native (Sevro)"` or `"CDP (Chrome)"` with explanation of routing behavior.

---

## 2. Interaction & Form Control

### `browse_click`

Click an interactive element by its `@ref` ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID from the snapshot (e.g., `5` for `@e5`) |
| `force` | bool | no | `false` | Bypass hidden/disabled/obscured safety checks |

**Annotations:** `rw_open`

**Behavior:** If the click causes navigation, returns the new page snapshot. Otherwise returns the action result.

---

### `browse_fill`

Fill a form field with text. Replaces existing content.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the form field |
| `text` | string | **yes** | — | Text to fill into the field |
| `force` | bool | no | `false` | Bypass safety checks |

**Annotations:** `rw_open`

---

### `browse_select`

Select a dropdown option by `@ref` ID and value.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the `<select>` dropdown |
| `value` | string | **yes** | — | The option value to select |
| `force` | bool | no | `false` | Bypass safety checks |

**Annotations:** `rw_open`

---

### `browse_type`

Type text into an element with realistic keystroke delays (human-like input simulation).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the input field |
| `text` | string | **yes** | — | Text to type character by character |
| `delay_ms` | integer | no | `50` | Delay between keystrokes in ms. Higher = more human-like |
| `force` | bool | no | `false` | Bypass safety checks |

**Annotations:** `rw_open`

---

### `browse_hover`

Hover over an element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the element to hover over |

**Annotations:** `rw_open`

---

### `browse_key_press`

Press a keyboard key on the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `key` | string | **yes** | — | Key name: `"Enter"`, `"Tab"`, `"Escape"`, `"ArrowDown"`, `"Backspace"`, etc. |

**Annotations:** `rw_open`

**Special behavior:** When `key` is `"Enter"` in native mode, automatically finds and clicks the first submit/button element on the page (simulates form submission).

---

### `browse_upload_file`

Upload a file from disk to an `<input type="file">` element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | no | — | The `@ref` ID of the file input. If omitted, auto-detects the first file input |
| `file_path` | string | **yes** | — | Absolute path to the file on disk |

**Annotations:** `rw_open`

**Behavior:** Reads the file, base64-encodes it, and injects it into the file input via JavaScript. Use for resume uploads, document submissions, image uploads.

---

### `browse_submit_form`

Submit a form by clicking its submit button or calling `form.submit()`.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | `@ref` ID of a button, form, or element inside a form |

**Annotations:** `rw_open`

**Behavior:**
- If `ref_id` points to a button → clicks it
- If `ref_id` points to a `<form>` → calls `form.submit()`
- If `ref_id` points to any element inside a form → submits the parent form
- Handles React-controlled forms that use XHR/fetch submission

---

### `browse_custom_dropdown`

Interact with a custom (non-native `<select>`) dropdown/combobox.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | `@ref` ID of the dropdown trigger element |
| `value` | string | **yes** | — | The option text to select |
| `type_to_filter` | bool | no | `true` | Whether to type the value to filter options |

**Annotations:** `rw_open`

**Behavior:** Handles React/Greenhouse-style dropdowns: clicks to open, types to filter, then clicks the matching option. Essential for country selectors, visa sponsorship fields, EEO fields. Uses mousedown/mouseup events + React's native setter pattern for value persistence.

---

### `browse_fetch_scripts`

Fetch and execute external `<script>` tags from the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_open`

**Use case:** Call AFTER `browse_navigate` when you need React/Vue/Angular to mount for form filling. Downloads JS bundles and runs them in QuickJS so React's event system activates.

---

### `browse_solve_captcha`

Solve a page verification challenge using a third-party solving service.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `captcha_type` | string | no | auto-detect | Challenge type (e.g., `"recaptcha_v2"`, `"hcaptcha"`) |
| `site_key` | string | no | auto-detect | The challenge site key. Auto-detected from page if omitted |

**Annotations:** `rw_open`

**Requires:** `TWOCAPTCHA_API_KEY` environment variable. Returns the solved token and injects it into the page.

---

### `browse_enter_iframe`

Switch the page context to a cross-origin iframe's DOM.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | no | — | `@ref` ID of the iframe element. Auto-detects if omitted |
| `selector` | string | no | — | CSS selector for the iframe |

**Annotations:** `rw_open`

**Behavior:** After entering, `browse_snapshot` shows the iframe's content. Use `browse_back` to return to the parent page.

---

### `browse_dismiss_overlay`

Auto-dismiss a modal, overlay, popup, or cookie banner.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | no | — | `@ref` ID of the overlay. If omitted, auto-detects the topmost overlay |

**Annotations:** `rw_open`

**Returns:** Updated page snapshot after dismissal.

---

### `browse_login`

Perform a full login flow with redirect chain following.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | Login page URL |
| `username` | string | **yes** | — | Username/email |
| `password` | string | **yes** | — | Password |

**Annotations:** `rw_open`

**Behavior:** Navigates to login page, fills username + password, clicks submit, and follows the entire OAuth/auth redirect chain (302 → 302 → 200). Captures all Set-Cookie headers at every redirect hop. Returns the final page snapshot and all cookies set during the flow.

---

## 3. Content Extraction & DOM

### `browse_snapshot`

Get the current page's DOM snapshot showing all interactive elements with `@ref` IDs.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Response format:** Hierarchical text showing page structure with interactive elements tagged as `@e1`, `@e2`, etc. Each element shows its role (link, button, input, select), text content, and key attributes.

---

### `browse_extract`

Extract the current page's content as clean markdown optimized for LLM context windows.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `max_tokens` | integer | no | unlimited | Maximum token budget for extracted content |

**Annotations:** `ro_closed`

**Response format:**
```
# Page Title

[markdown content...]

---
42 links | ~1500 tokens
```

**Implementation:** Uses `wraith_content_extract::extract()` or `::extract_budgeted()` for token-limited extraction.

---

### `browse_screenshot`

Capture a PNG screenshot of the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `full_page` | bool | no | `false` | `true` = entire scrollable page, `false` = visible viewport only |

**Annotations:** `ro_closed`

**Returns:** Text with dimensions and base64-encoded PNG size: `Screenshot captured (1920x1080, 45678 bytes base64)`. Note: actual base64 data is included in production; the format description is simplified here.

---

### `browse_eval_js`

Execute JavaScript code on the current page and return the result.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `code` | string | **yes** | — | JavaScript source code. Returns the last expression's value as a string |

**Annotations:** `rw_destructive`

**Response:** `JS result: <value>` or `JavaScript execution failed: <error>`

**Engine notes:**
- **Sevro:** Executes via QuickJS embedded engine
- **CDP:** Executes via Chrome's V8 engine (full browser APIs available)
- **Native:** Limited/no JS support

---

### `browse_search`

Search the web using metasearch (DuckDuckGo + Brave).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | **yes** | — | Search query string |
| `max_results` | integer | no | `10` | Maximum number of results |

**Annotations:** `ro_open`

**Response format:**
```
Search results for: rust async runtime

1. **Tokio - An asynchronous Rust runtime**
   https://tokio.rs/
   Tokio is an async runtime for Rust...

2. **async-std - Async version of the Rust standard library**
   https://async.rs/
   ...
```

---

### `extract_pdf`

Fetch a PDF from a URL and extract its text content as markdown.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL of the PDF to fetch and parse |

**Annotations:** `ro_open`

---

### `extract_article`

Extract the main article body from the current page using readability algorithms.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `readability` | bool | no | `true` | If true, strips nav, ads, sidebars — article body only |

**Annotations:** `ro_closed`

---

### `extract_markdown`

Convert HTML to clean markdown.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `html` | string | no | current page source | Raw HTML string to convert. Uses current page if omitted |

**Annotations:** `ro_closed`

---

### `extract_plain_text`

Convert HTML to plain text with no formatting.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `html` | string | no | current page source | Raw HTML string. Uses current page if omitted |

**Annotations:** `ro_closed`

---

### `extract_ocr`

Run OCR text detection on the current page screenshot.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | no | — | Description of what to OCR |

**Annotations:** `ro_closed`

**Requires:** `vision-ml` feature flag for ONNX-based OCR. Without it, returns a simplified text extraction.

---

## 4. DOM Manipulation

### `dom_query_selector`

Run a CSS selector query against the current page DOM.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `selector` | string | **yes** | — | CSS selector (e.g., `"div.job-card"`, `"#main-content"`, `"a[href*='apply']"`) |

**Annotations:** `ro_closed`

---

### `dom_get_attribute`

Read an HTML attribute from an element by `@ref` ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the element |
| `name` | string | **yes** | — | Attribute name (e.g., `"href"`, `"class"`, `"data-job-id"`, `"aria-label"`) |

**Annotations:** `ro_closed`

---

### `dom_set_attribute`

Set an HTML attribute on an element by `@ref` ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the element |
| `name` | string | **yes** | — | Attribute name |
| `value` | string | **yes** | — | New attribute value |

**Annotations:** `rw_closed`

---

### `dom_focus`

Focus an element by `@ref` ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | **yes** | — | The `@ref` ID of the element to focus |

**Annotations:** `rw_closed`

---

## 5. Credential Vault

The vault uses AES-256-GCM encryption backed by an SQLite database at `~/.wraith/vault.db`. All secrets are encrypted at rest. The vault auto-unlocks with an empty passphrase by default in MCP mode.

### `browse_vault_store`

Store a credential in the encrypted vault.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain (e.g., `"github.com"`, `"indeed.com"`) |
| `kind` | string | **yes** | — | Type: `"password"`, `"api_key"`, `"oauth_token"`, `"totp_seed"`, `"session_cookie"`, or `"generic"` |
| `identity` | string | **yes** | — | Username, email, or account identifier |
| `secret` | string | **yes** | — | The secret value |

**Annotations:** `rw_closed`

**Returns:** `Credential stored: <uuid> (identity@domain, Kind)` or error.

---

### `browse_vault_get`

Retrieve a credential from the vault.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain to look up |
| `kind` | string | no | — | Optional type filter |

**Annotations:** `ro_closed`

**Returns:** Credential ID, identity, kind, and **decrypted secret value**.

---

### `browse_vault_list`

List all stored credentials (metadata only — secrets stay encrypted).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Response format:**
```
3 credential(s):

  a1b2c3d4 | indeed.com | Password | user@email.com | 5 uses
  e5f6g7h8 | github.com | ApiKey | my-app | 12 uses
  ...
```

---

### `browse_vault_delete`

Delete a credential by ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `id` | string | **yes** | — | Full credential UUID (get from `browse_vault_list`) |

**Annotations:** `rw_destructive`

---

### `browse_vault_totp`

Generate a current TOTP 2FA code for a domain.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain with a stored `totp_seed` credential |

**Annotations:** `ro_closed`

**Returns:** `TOTP code for example.com: 123456`

---

### `browse_vault_rotate`

Rotate a credential's secret value.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `id` | string | **yes** | — | Credential UUID |
| `new_secret` | string | **yes** | — | New secret value |

**Annotations:** `rw_closed`

---

### `browse_vault_audit`

View recent vault audit log entries.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `limit` | integer | no | `20` | Number of entries to return |

**Annotations:** `ro_closed`

**Response format:**
```
5 audit entries:

  2026-03-22T10:00:00Z | STORE | indeed.com | OK
  2026-03-22T09:55:00Z | GET | github.com | OK
  ...
```

---

### `vault_lock`

Lock the vault and zeroize the master key from memory.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_closed`

---

### `vault_unlock`

Unlock the vault with a passphrase.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `passphrase` | string | no | `""` | Vault passphrase. Empty string = auto-unlock (MCP default) |

**Annotations:** `rw_closed`

---

### `vault_approve_domain`

Approve a domain to use a specific credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | **yes** | — | Credential UUID |
| `domain` | string | **yes** | — | Domain to approve (e.g., `"login.indeed.com"`) |

**Annotations:** `rw_closed`

---

### `vault_revoke_domain`

Revoke a domain's access to a credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | **yes** | — | Credential UUID |
| `domain` | string | **yes** | — | Domain to revoke |

**Annotations:** `rw_closed`

---

### `vault_check_approval`

Check if a domain is approved for a credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | **yes** | — | Credential UUID |
| `domain` | string | **yes** | — | Domain to check |

**Annotations:** `ro_closed`

---

## 6. Cookie Management

### `cookie_get`

Get cookies for a domain from the browser's cookie jar.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain to get cookies for |

**Annotations:** `ro_closed`

---

### `cookie_set`

Set a cookie in the browser's cookie jar.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Cookie domain (e.g., `".indeed.com"`) |
| `name` | string | **yes** | — | Cookie name |
| `value` | string | **yes** | — | Cookie value |
| `path` | string | no | `"/"` | Cookie path |

**Annotations:** `rw_closed`

---

### `cookie_save`

Save browser cookies to a JSON file for persistence across sessions.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | no | `~/.wraith/cookies.json` | File path to save cookies to |

**Annotations:** `rw_closed`

---

### `cookie_load`

Load cookies from a JSON file into the browser.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | no | `~/.wraith/cookies.json` | File path to load cookies from |

**Annotations:** `rw_closed`

---

### `cookie_import_chrome`

Import cookies from the user's Chrome browser profile (Windows DPAPI decryption).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `profile` | string | no | `"Default"` | Chrome profile name |
| `domains` | array[string] | no | all | Filter to specific domains |

**Annotations:** `rw_open`

**Platform:** Windows only (uses DPAPI for `encrypted_value` decryption). On other platforms returns an error.

**Behavior:** Reads Chrome's encrypted SQLite cookie database, decrypts v10/v20 cookies using the master key (DPAPI → AES-256-GCM), and loads them into Wraith's cookie jar.

---

## 7. Knowledge Cache

The knowledge cache stores previously visited pages for fast retrieval, full-text search, and change detection. Backed by an in-memory store with persistence.

### `cache_search`

Full-text search across all cached pages.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | **yes** | — | Search query |
| `max_results` | integer | no | `10` | Maximum results |

**Annotations:** `ro_closed`

---

### `cache_get`

Check if a URL is in the cache and return cached content.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to look up |

**Annotations:** `ro_closed`

---

### `cache_stats`

Show knowledge cache statistics.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Page count, total size, domains covered, hit/miss rates.

---

### `cache_purge`

Purge stale entries from the knowledge cache.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `rw_destructive`

---

### `cache_pin`

Pin a URL so it is never evicted from cache.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to pin |
| `notes` | string | no | — | Optional note explaining why pinned |

**Annotations:** `rw_closed`

---

### `cache_tag`

Tag a cached page with labels for organized retrieval.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to tag |
| `tags` | array[string] | **yes** | — | Tags (e.g., `["job-listing", "remote", "rust"]`) |

**Annotations:** `rw_closed`

---

### `cache_domain_profile`

Show how often a domain's content changes and its computed TTL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain to profile (e.g., `"indeed.com"`) |

**Annotations:** `ro_closed`

---

### `cache_find_similar`

Find cached pages similar to a given URL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to find similar pages for |
| `max_results` | integer | no | `5` | Maximum results |

**Annotations:** `ro_closed`

---

### `cache_evict`

Evict cached pages to fit within a byte budget.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `max_bytes` | integer | **yes** | — | Maximum cache size. Pages evicted oldest-first until under budget |

**Annotations:** `rw_destructive`

---

### `cache_raw_html`

Get the raw cached HTML for a URL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to get raw HTML for |

**Annotations:** `ro_closed`

---

## 8. Knowledge Graph (Entities)

An in-memory knowledge graph for tracking entities (companies, people, technologies) discovered across browsing sessions.

### `entity_query`

Query the knowledge graph with a natural language question.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `question` | string | **yes** | — | e.g., `"what do we know about Stripe?"` |

**Annotations:** `ro_closed`

---

### `entity_add`

Add an entity to the knowledge graph.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Entity name (e.g., `"Stripe"`, `"Rust"`) |
| `entity_type` | string | **yes** | — | `"company"`, `"person"`, `"technology"`, `"product"`, `"location"`, or `"other"` |
| `attributes` | object | no | — | Key-value attribute pairs |

**Annotations:** `rw_closed`

---

### `entity_relate`

Add a relationship between two entities.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `from` | string | **yes** | — | Source entity name |
| `to` | string | **yes** | — | Target entity name |
| `relationship` | string | **yes** | — | e.g., `"uses"`, `"employs"`, `"competes_with"`, `"acquired"` |

**Annotations:** `rw_closed`

---

### `entity_merge`

Merge two entities (second is merged into first).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name_a` | string | **yes** | — | Primary entity name |
| `name_b` | string | **yes** | — | Entity to merge into primary |

**Annotations:** `rw_closed`

---

### `entity_find_related`

Find entities connected to a given entity.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Entity name to find connections for |

**Annotations:** `ro_closed`

---

### `entity_search`

Fuzzy search entities by name.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | **yes** | — | Search query (fuzzy name match) |

**Annotations:** `ro_closed`

---

### `entity_visualize`

Generate a Mermaid diagram of the knowledge graph.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Mermaid graph syntax that can be rendered as a diagram.

---

## 9. Embeddings & Semantic Search

### `embedding_search`

Semantic similarity search across cached content.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `text` | string | **yes** | — | Text to find semantically similar content for |
| `top_k` | integer | no | `5` | Maximum results |

**Annotations:** `ro_closed`

---

### `embedding_upsert`

Store a text embedding for semantic search.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `source_id` | string | **yes** | — | Unique source ID (usually URL or document ID) |
| `content` | string | **yes** | — | Text content to embed |

**Annotations:** `rw_closed`

---

## 10. Authentication Detection

### `auth_detect`

Detect authentication flows on a page (password forms, OAuth buttons, 2FA, CAPTCHA).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | current page | URL to analyze |

**Annotations:** `ro_closed`

**Returns:** Detected auth mechanisms: password fields, OAuth/SSO redirect buttons, TOTP inputs, CAPTCHA challenges.

---

## 11. Browser Fingerprinting & Identity

### `fingerprint_list`

List available browser fingerprint profiles.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Available profiles (Chrome, Firefox, Safari) with user agents, screen resolutions, and platform details.

---

### `fingerprint_import`

Import a browser fingerprint profile from a JSON file.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | **yes** | — | File path to the fingerprint JSON file |

**Annotations:** `rw_closed`

---

### `identity_profile`

Set the browsing identity profile.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `profile_type` | string | **yes** | — | `"personal"` (use real name) or `"anonymous"` |
| `name` | string | no | — | Name for personal profile |

**Annotations:** `rw_closed`

---

### `site_fingerprint`

Detect the technology stack of a website (React, WordPress, Shopify, etc.).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | current page | URL to fingerprint |

**Annotations:** `ro_closed`

---

### `stealth_status`

Show current TLS compatibility status and evasion configuration.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

## 12. Network Intelligence

### `network_discover`

Discover API endpoints from captured network traffic patterns.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Detected API patterns: URL templates, HTTP methods, auth types, request/response schemas.

---

### `dns_resolve`

Resolve a domain name to IP addresses via DNS-over-HTTPS.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | **yes** | — | Domain to resolve (e.g., `"indeed.com"`) |

**Annotations:** `ro_open`

---

### `page_diff`

Compare current page content to the cached version.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | current page | URL to diff against cached version |

**Annotations:** `ro_closed`

**Returns:** Content changes detected between live and cached versions.

---

## 13. TLS & Security

### `tls_profiles`

List available TLS fingerprint profiles for compatible browsing.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `tls_verify`

Verify that Wraith's TLS fingerprint matches a real Chrome 136 browser.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_open`

**Behavior:** Fetches a TLS fingerprinting service using the same HTTP stack as `browse_navigate`, then compares JA3/JA4 hashes, cipher suites, extensions, and HTTP/2 SETTINGS against known Chrome 136 values. Returns a detailed pass/fail report.

---

## 14. Session Management (CDP)

**Requires:** `cdp` feature flag.

Sessions allow multiple browser instances to run simultaneously. Each session is named and backed by its own engine (native Sevro or CDP Chrome).

### `browse_session_create`

Create a new named browser session.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Session name (e.g., `"job-search"`, `"linkedin"`) |
| `engine_type` | string | no | `"native"` | `"native"` (Sevro) or `"cdp"` (Chrome) |

**Annotations:** `rw_closed`

**Behavior:** Multiple sessions can be active simultaneously. Each has its own cookie jar, history, and page state.

---

### `browse_session_switch`

Switch the active session. All subsequent `browse_*` commands route to the new session's engine.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Session name to switch to |

**Annotations:** `rw_closed`

---

### `browse_session_list`

List all open sessions with engine type and current URL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `browse_session_close`

Close a named session and shut down its engine.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Session to close |

**Annotations:** `rw_destructive`

**Constraints:** Cannot close the `"native"` session. If closing the active session, auto-switches to `"native"`.

---

## 15. Plugins (WASM)

WASM plugin system for extending browser capabilities. **Requires:** `wasm` feature flag for Wasmtime execution.

### `plugin_register`

Register a WASM plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Plugin name |
| `wasm_path` | string | **yes** | — | Path to the `.wasm` file |
| `description` | string | no | — | Plugin description |
| `domains` | array[string] | no | — | Domains this plugin targets (e.g., `["amazon.com", "ebay.com"]`) |

**Annotations:** `rw_closed`

---

### `plugin_execute`

Execute a registered WASM plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Plugin name |
| `input` | JSON | no | — | Input data for the plugin |

**Annotations:** `rw_closed`

---

### `plugin_list`

List all registered WASM plugins.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `plugin_remove`

Unregister a WASM plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Plugin name to remove |

**Annotations:** `rw_closed`

---

## 16. Scripting (Rhai)

Rhai scripting engine for lightweight automation scripts.

### `script_load`

Load a Rhai userscript.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Unique script name |
| `source` | string | **yes** | — | Rhai source code |
| `trigger` | string | no | `"manual"` | `"always"` (every page), `"manual"` (explicit only), or a URL substring for on-navigate triggers |

**Annotations:** `rw_closed`

---

### `script_list`

List all loaded Rhai userscripts.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `script_run`

Execute a loaded Rhai script by name against the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Script name to execute |

**Annotations:** `rw_closed`

---

## 17. Telemetry & Monitoring

### `telemetry_metrics`

Show browsing metrics.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Cache hits, cache misses, errors, navigations, total requests, average response time.

---

### `telemetry_spans`

Export performance trace spans as JSON.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** JSON array of performance spans with timing data for each operation.

---

## 18. Workflow Recording & Replay

Record and replay multi-step browser automations with variable substitution.

### `workflow_start_recording`

Begin recording a replayable workflow.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Workflow name (e.g., `"indeed-job-apply"`) |

**Annotations:** `rw_closed`

**Behavior:** All subsequent `browse_*` tool calls are captured as workflow steps. Stop recording with `workflow_stop_recording`.

---

### `workflow_stop_recording`

Stop recording and save the workflow.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | **yes** | — | Description of what this workflow does |

**Annotations:** `rw_closed`

---

### `workflow_replay`

Replay a saved workflow with variable substitution.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | Workflow name |
| `variables` | object | no | — | Variable substitutions (e.g., `{"job_title": "Rust Engineer", "location": "Remote"}`) |

**Annotations:** `rw_open`

---

### `workflow_list`

List all saved workflows.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

## 19. Time-Travel Debugging

Inspect and branch from the agent's decision timeline.

### `timetravel_summary`

Show the agent decision timeline summary.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `timetravel_branch`

Branch from a decision point to explore alternatives.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `step` | integer | **yes** | — | Step number to branch from (0-indexed) |
| `name` | string | **yes** | — | Name for the new branch |

**Annotations:** `rw_closed`

---

### `timetravel_replay`

Replay the timeline to a specific step.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `step` | integer | **yes** | — | Step number to replay to |

**Annotations:** `ro_closed`

---

### `timetravel_diff`

Diff two timeline branches to see where decisions diverged.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `branch_a` | string | **yes** | — | First branch ID |
| `branch_b` | string | **yes** | — | Second branch ID |

**Annotations:** `ro_closed`

---

### `timetravel_export`

Export the full timeline as JSON.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

## 20. Task DAG Orchestration

Declarative task graphs with dependency management for parallel execution.

### `dag_create`

Create a new task DAG.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | **yes** | — | DAG name |

**Annotations:** `rw_closed`

---

### `dag_add_task`

Add a task node to the DAG.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | **yes** | — | Unique task ID within the DAG |
| `description` | string | **yes** | — | Human-readable description |
| `action_type` | string | **yes** | — | `"navigate"`, `"click"`, `"fill"`, `"extract"`, or `"custom"` |
| `target` | string | no | — | Action target (URL, selector, etc.) |

**Annotations:** `rw_closed`

---

### `dag_add_dependency`

Add a dependency edge between tasks.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | **yes** | — | Task that depends on another |
| `depends_on` | string | **yes** | — | Task that must complete first |

**Annotations:** `rw_closed`

---

### `dag_ready`

Get tasks that are ready to execute (all dependencies met).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `dag_complete`

Mark a DAG task as completed.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | **yes** | — | Task ID to mark complete |
| `result` | string | **yes** | — | Result or output from the task |

**Annotations:** `rw_closed`

---

### `dag_progress`

Show DAG completion progress.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `dag_visualize`

Generate a Mermaid diagram of the DAG.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

## 21. Planning & Prediction

### `mcts_plan`

Use Monte Carlo Tree Search to plan the best next action.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `state` | string | **yes** | — | Current page state description |
| `actions` | array[string] | **yes** | — | Available actions (e.g., `["click @e1", "fill @e3", "navigate /next"]`) |
| `simulations` | integer | no | `100` | Number of MCTS simulations |

**Annotations:** `ro_closed`

**Returns:** Ranked action recommendations with UCB1 scores and confidence levels.

---

### `mcts_stats`

Show MCTS planner statistics.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `prefetch_predict`

Predict which URLs to prefetch based on the current task.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_description` | string | **yes** | — | Current task description |

**Annotations:** `ro_closed`

---

### `browse_task`

Run an autonomous multi-step browsing task using the AI agent loop.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | **yes** | — | Natural language task (e.g., `"Find remote Rust jobs on Indeed and extract titles and URLs"`) |
| `url` | string | no | — | Starting URL. If omitted, agent decides |
| `max_steps` | integer | no | `50` | Maximum action steps before stopping |

**Annotations:** `rw_open`

---

## 22. Parallel Browsing / Swarm

### `swarm_fan_out`

Visit multiple URLs in parallel and collect results.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `urls` | array[string] | **yes** | — | URLs to visit in parallel |
| `max_concurrent` | integer | no | `4` | Maximum concurrent sessions |

**Annotations:** `rw_open`

---

### `swarm_collect`

Collect results from a parallel browsing swarm.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

## 23. Playbook Automation

Pre-authored automation scripts for common flows (job applications on Greenhouse, Ashby, Lever, Indeed).

### `swarm_run_playbook`

Execute a YAML playbook or built-in named playbook.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `playbook` | string | **yes** | — | Built-in name (e.g., `"greenhouse-apply"`) or path to YAML file |
| `variables` | object | no | — | Variable substitutions for the playbook |
| `url` | string | no | — | Starting URL (overrides playbook's default) |

**Annotations:** `rw_open`

**Built-in playbooks:**
- `greenhouse-apply` — Greenhouse job application flow
- `ashby-apply` — Ashby ATS application flow
- `lever-apply` — Lever ATS application flow
- `indeed-apply` — Indeed Easy Apply flow

**Playbook step types:** Navigate, Click, Fill, Select, Wait, Extract, Verify, Screenshot, CustomDropdown, UploadFile, SubmitForm, EvalJs, Conditional (`if_url_contains`, `if_variable`), Repeat (for-each loops).

**Error handling:** Each step can define `on_error`: Abort (stop), Skip (continue), Retry (with count/delay), Screenshot (capture state).

---

### `swarm_list_playbooks`

List all built-in playbook names with descriptions.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

---

### `swarm_playbook_status`

Check progress of a running or completed playbook.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Completed/total steps, current step name, errors encountered.

---

## 24. Deduplication & Verification

SQLite-backed deduplication tracking for job applications (or any submission).

### `swarm_dedup_check`

Check if a URL has already been processed.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to check |

**Annotations:** `ro_closed`

**Returns:** `{ applied: bool, applied_at: timestamp, status: string }`

---

### `swarm_dedup_record`

Record that a submission was completed.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL that was processed |
| `company` | string | no | — | Company name |
| `title` | string | no | — | Job/item title |
| `platform` | string | no | — | Platform name (Greenhouse, Indeed, etc.) |

**Annotations:** `rw_closed`

---

### `swarm_dedup_stats`

Return aggregate deduplication statistics.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** Total applications, breakdown by platform and status, today's count, this week's count.

---

### `swarm_verify_submission`

Verify that a submission went through by checking the current page for success/error indicators.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| *(none)* | — | — | — | — |

**Annotations:** `ro_closed`

**Returns:** `{ result: "confirmed"|"likely"|"uncertain"|"failed", message: string }`

---

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `WRAITH_FLARESOLVERR` | External challenge-solving proxy URL | *(none)* |
| `WRAITH_PROXY` | HTTP proxy URL for all requests | *(none)* |
| `WRAITH_FALLBACK_PROXY` | Fallback proxy URL | *(none)* |
| `WRAITH_CDP_CHROME` | Path to Chrome/Chromium binary (enables CDP) | *(none, CDP disabled)* |
| `WRAITH_CDP_AUTO` | Auto-fallback to CDP on SPA detection (`"true"`) | `false` |
| `TWOCAPTCHA_API_KEY` | API key for CAPTCHA solving service | *(none)* |

---

## Feature Flags

Compile-time feature flags in `Cargo.toml`:

| Feature | Default | Description |
|---------|---------|-------------|
| `sevro` | **yes** | Servo-based browser with QuickJS JavaScript engine |
| `cdp` | no | Chrome DevTools Protocol support (session management, full JS) |
| `stealth-tls` | no | TLS fingerprint evasion via rquest |
| `vision-ml` | no | ONNX Runtime for OCR and vision detection |
| `tor` | no | Tor onion routing support |
| `wasm` | no | WASM plugin execution via Wasmtime |

### Tool availability by feature

| Feature | Tools Added | Tools Removed |
|---------|-------------|---------------|
| `cdp` | `browse_navigate_cdp`, `browse_session_create`, `browse_session_switch`, `browse_session_close` | *(none)* |
| Without `cdp` | — | The 4 CDP tools above are absent; `browse_session_list` is still available |

---

## Engine Architecture

### BrowserEngine Trait

```rust
#[async_trait]
pub trait BrowserEngine: Send + Sync {
    async fn navigate(&mut self, url: &str) -> BrowserResult<()>;
    async fn snapshot(&self) -> BrowserResult<DomSnapshot>;
    async fn execute_action(&mut self, action: BrowserAction) -> BrowserResult<ActionResult>;
    async fn eval_js(&self, script: &str) -> BrowserResult<String>;
    async fn page_source(&self) -> BrowserResult<String>;
    async fn current_url(&self) -> Option<String>;
    async fn screenshot(&self) -> BrowserResult<Vec<u8>>;
    fn capabilities(&self) -> EngineCapabilities;
    async fn set_cookie_values(&mut self, domain: &str, name: &str, value: &str, path: &str);
    async fn shutdown(&mut self) -> BrowserResult<()>;
}
```

### Engine Implementations

| Engine | Module | JS Support | Speed | Use Case |
|--------|--------|------------|-------|----------|
| **SevroEngine** | `engine_sevro.rs` | QuickJS (ES2023) | ~100ms/page | Default — good balance of speed and JS support |
| **NativeEngine** | `engine_native.rs` | None | ~50ms/page | Fastest — static pages, API scraping |
| **CdpEngine** | `engine_cdp.rs` | Chrome V8 (full) | ~500ms+/page | Full browser — React SPAs, complex JS apps |

### BrowserAction Enum

All 19 action variants supported by the engine:

```rust
pub enum BrowserAction {
    Navigate { url: String },
    Click { ref_id: u32, force: Option<bool> },
    Fill { ref_id: u32, text: String, force: Option<bool> },
    Select { ref_id: u32, value: String, force: Option<bool> },
    KeyPress { key: String },
    TypeText { ref_id: u32, text: String, delay_ms: u32, force: Option<bool> },
    Scroll { direction: ScrollDirection, amount: i32 },
    ScrollTo { ref_id: u32 },
    Hover { ref_id: u32 },
    GoBack,
    GoForward,
    Reload,
    Wait { ms: u64 },
    WaitForSelector { selector: String, timeout_ms: u64 },
    WaitForNavigation { timeout_ms: u64 },
    EvalJs { code: String },
    Screenshot { full_page: bool },
    ExtractContent { max_tokens: Option<usize> },
    UploadFile { ref_id: Option<u32>, file_path: String },
    SubmitForm { ref_id: u32 },
}
```

### Session Routing

When the `cdp` feature is enabled:

```
browse_navigate       → switches to native engine ("native" session)
browse_navigate_cdp   → switches to CDP engine ("cdp" session)
browse_session_switch → switches to named session
All other browse_*    → routes to active session's engine
```

The `active_engine_async()` method resolves the current engine:
1. Look up `active_session_name` in the sessions map
2. Fall back to legacy `active_cdp_session`
3. Fall back to `self.engine` (native)

### Dedup Database

SQLite database at `~/.wraith/dedup.db` tracks processed URLs to prevent duplicates. Created automatically on handler initialization.

---

## Tool Count Summary

| Category | Count |
|----------|-------|
| Navigation & Core Browsing | 11 |
| Interaction & Form Control | 13 |
| Content Extraction & DOM | 8 |
| DOM Manipulation | 4 |
| Credential Vault | 11 |
| Cookie Management | 5 |
| Knowledge Cache | 11 |
| Knowledge Graph | 7 |
| Embeddings & Semantic Search | 2 |
| Authentication Detection | 1 |
| Browser Fingerprinting & Identity | 5 |
| Network Intelligence | 3 |
| TLS & Security | 2 |
| Session Management (CDP) | 4 |
| Plugins (WASM) | 4 |
| Scripting (Rhai) | 3 |
| Telemetry & Monitoring | 2 |
| Workflow Recording & Replay | 4 |
| Time-Travel Debugging | 5 |
| Task DAG Orchestration | 7 |
| Planning & Prediction | 4 |
| Parallel Browsing / Swarm | 2 |
| Playbook Automation | 3 |
| Deduplication & Verification | 4 |
| **Total** | **~143** |

> Exact count varies by feature flags: `cdp` adds 4 session tools + `browse_navigate_cdp`.
