# Wraith Browser

**The AI-agent-first web browser -- built in Rust, designed for LLM control.**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![MCP Tools](https://img.shields.io/badge/MCP%20tools-116-blue.svg)]()

---

Wraith is a native Rust browser engine purpose-built for AI agents. No Chrome dependency. No Node.js. Ships as a single ~15MB binary or MCP server with 116 tools -- every capability accessible via MCP calls. The user never touches this browser directly; the AI agent has full admin control.

## Why Wraith

| | Wraith | Traditional Automation |
|---|---|---|
| Chrome required | No | Yes (300MB+) |
| Memory per session | 5-50 MB | 300-500 MB |
| Page fetch (static) | ~50ms | 1-3 seconds |
| Binary size | ~15 MB | ~300 MB + runtime |
| Startup time | <100ms | 2-5 seconds |
| Concurrent sessions (16GB) | 50-100+ | 6-8 |
| Protected site handling | Multi-tier adaptive | Limited |
| MCP native | Yes (116 tools) | No |
| ATS-aware form submission | Native API integration | Manual scripting |
| File upload | Yes (DataTransfer API) | Yes (setInputFiles) |
| Cookie import from Chrome | Yes (reads Chrome SQLite DB) | Manual |
| Knowledge graph | Built-in entity resolution | Not available |
| Workflow record/replay | Built-in | Not available |

## Quick Start

### Build

```bash
git clone https://github.com/suhteevah/openclaw-browser.git
cd openclaw-browser
cargo build --release
```

### Connect to Claude Code

```bash
claude mcp add openclaw ./target/release/openclaw-browser -- serve --transport stdio
```

Your AI agent immediately gains 116 browser tools -- full admin control with zero CLI interaction.

### CLI

```bash
# Navigate and see interactive elements
openclaw-browser navigate https://example.com

# Extract content as clean markdown
openclaw-browser extract https://example.com/docs --max-tokens 4000

# Search the web (supports OR queries)
openclaw-browser search "QA engineer OR SDET remote"

# Autonomous browsing task
ANTHROPIC_API_KEY=sk-... openclaw-browser task "Find remote Rust jobs"

# Manage encrypted credentials
openclaw-browser vault store --domain example.com --kind password --identity user@example.com
```

### Environment Variables (MCP mode)

| Variable | Purpose |
|----------|---------|
| `WRAITH_FLARESOLVERR` | URL for external challenge-solving proxy (e.g., `http://localhost:8191`) |
| `WRAITH_PROXY` | Primary HTTP/SOCKS5 proxy URL |
| `WRAITH_FALLBACK_PROXY` | Fallback proxy for IP-blocked sites |
| `ANTHROPIC_API_KEY` | Required for `browse_task` autonomous agent |
| `BRAVE_SEARCH_API_KEY` | Optional Brave Search provider |
| `TWOCAPTCHA_API_KEY` | Required for `browse_solve_captcha` CAPTCHA solving |

---

## Architecture

```
                     AI Agent (Claude Code, Cursor, custom)
                                    |
                              MCP Protocol (stdio)
                                    |
                    +---------------v----------------+
                    |       MCP Server (116 tools)   |
                    +---------------+----------------+
                                    |
                    +---------------v----------------+
                    |     BrowserEngine Trait         |
                    |  SevroEngine  |  NativeEngine  |
                    +------+--------+-------+--------+
                           |                |
          +----------------v--+    +--------v---------+
          | Sevro Headless     |    | Pure HTTP Client  |
          | - QuickJS (JS)     |    | - HTTP/1.1 + 2   |
          | - Full DOM Bridge  |    | - HTML5 parser    |
          | - React form fill  |    | - ~50ms/page      |
          | - Adaptive access  |    +-------------------+
          | - ATS API native   |
          +--------------------+
```

### 10 Crates

| Crate | Purpose |
|-------|---------|
| `browser-core` | Unified engine trait, ATS detection, network layer, swarm, plugins |
| `sevro-headless` | Headless engine -- HTTP, full DOM parsing, QuickJS JS runtime, adaptive site access, SPA hydration |
| `agent-loop` | LLM agent cycle -- MCTS planning, time-travel, workflows, task DAGs |
| `cache` | SQLite knowledge store, full-text search, embeddings, entity graph, semantic diffing |
| `content-extract` | Readability extraction, markdown conversion, OCR, PDF text extraction |
| `identity` | AES-256-GCM encrypted credential vault, browser profiles, TOTP, auth flows |
| `mcp-server` | MCP protocol server (116 tools, stdio transport) |
| `search-engine` | Metasearch (multiple providers), OR query splitting, local index |
| `scripting` | Rhai sandboxed scripting engine (userscripts with navigation triggers) |
| `cli` | Binary with subcommands (`navigate`, `extract`, `search`, `task`, `vault`) |

---

## Platform Integration

Wraith detects common job application platforms and uses their native APIs when available. Instead of scripting against complex React-based SPAs, Wraith speaks the underlying APIs directly.

| Platform Type | Method | Coverage |
|---------------|--------|----------|
| **Direct-hosted ATS forms** | Renders HTML form, fills via React-compatible `browse_fill`, submits to the platform's API as multipart | Full form fill + submit |
| **Wrapped/embedded ATS forms** | Detects job ID parameters on company career sites, probes the platform API to resolve the correct board, redirects to the direct application form | Auto-resolves wrapped URLs |
| **GraphQL-based ATS platforms** | Queries the platform's GraphQL API for form definitions, builds synthetic HTML with real form fields matching the API schema | Full form fill via API |
| **Server-rendered ATS forms** | Auto-appends the correct apply path, works with standard `browse_fill` on server-rendered HTML | Full form fill + submit |
| **Static HTML forms** | Standard DOM interaction | Full support |

### How ATS Resolution Works

When `browse_navigate` is called:

1. **Server-rendered ATS URLs** -- the apply path is automatically appended to reach the application form
2. **Wrapped ATS URLs** (career sites with job ID query parameters) -- the engine extracts the job ID, derives candidate board slugs from the domain, probes the platform API for each, and on success redirects to the direct application form
3. **GraphQL-based ATS URLs** -- fetches the form definition via GraphQL, builds synthetic HTML with `<input>`, `<select>`, `<textarea>`, `<label>` elements matching the real form, loads it as the page DOM

When `browse_submit_form` is called:

1. **Multipart-based platforms** -- serializes all form fields, POSTs to the platform's application endpoint as `multipart/form-data`
2. **Server-rendered platforms** -- POSTs to the apply endpoint as `application/x-www-form-urlencoded`
3. **GraphQL-based platforms** -- POSTs to the submission endpoint as `application/json`

---

## MCP Tools Reference (116 tools)

Every capability has a native MCP tool. The AI agent has full admin control with zero CLI interaction. Below is the complete reference for every tool -- parameters, defaults, and usage patterns.

### Navigation (8 tools)

#### `browse_navigate`
Navigate to a URL and return a DOM snapshot with all interactive elements.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | Full URL including protocol (`https://...`) |
| `wait_for_load` | bool | no | `true` | Wait for page to fully load before returning |

**Behavior:** Fetches the page via HTTP, parses HTML into a full DOM tree, sets up QuickJS JavaScript runtime with DOM bridge, executes inline `<script>` tags, and returns an agent-readable snapshot. Automatically handles ATS URL resolution for supported applicant tracking systems. For pages with fewer than 10 visible elements (SPA indicator), triggers automatic SPA hydration -- fetches dynamically created scripts and executes them.

**Example:**
```
browse_navigate { "url": "https://example.com/careers/apply/12345" }
```

#### `browse_back`
Navigate back in browser history. No parameters.

#### `browse_forward`
Navigate forward in browser history. No parameters.

#### `browse_reload`
Reload the current page. No parameters.

#### `browse_scroll`
Scroll the page viewport.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `direction` | string | yes | -- | `"up"`, `"down"`, `"left"`, or `"right"` |
| `amount` | integer | no | `500` | Pixels to scroll |

#### `browse_scroll_to`
Scroll the viewport to center a specific element by its `@ref` ID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference to scroll into view |

#### `browse_wait`
Wait for a CSS selector to appear or a fixed duration.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `selector` | string | no | -- | CSS selector to wait for (timeout: 5000ms) |
| `ms` | integer | no | `1000` | Fixed wait time in milliseconds (used if no selector) |

#### `browse_wait_navigation`
Wait for a navigation event to complete (page transition after click/submit).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `timeout_ms` | integer | no | `5000` | Maximum wait time |

---

### Interaction (7 tools)

#### `browse_click`
Click an interactive element by its `@ref` ID from the snapshot.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference number from snapshot |

**Behavior:** Looks up the element via `__wraith_ref_index`, calls `focus()`, `click()`, and dispatches a bubbling `click` Event. If the element has an `href`, reports the link URL. Falls back to basic `click_element()` on JS failure.

#### `browse_fill`
Fill a form field with text. React-compatible -- uses native value setter, `_valueTracker` invalidation, and event dispatch.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Form field `@e` reference |
| `text` | string | yes | -- | Text to fill |

**Behavior:**
1. Looks up element via `__wraith_get_by_ref(ref_id)`
2. Calls `focus()` on the element
3. Uses `Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set` to handle React-controlled inputs
4. Invalidates React's `_valueTracker` (forces React to see the change)
5. Dispatches `focus`, `input`, `change`, `blur` events (bubbling)
6. Walks the React fiber tree looking for `__reactProps$` or `__reactFiber$` to call `onChange` directly
7. Reads back the value to verify it persisted -- reports `verified` or `UNVERIFIED`

**Example:**
```
browse_fill { "ref_id": 37, "text": "Jane Smith" }
# Returns: "@e37: FILLED (native_events, verified): Jane Smith"
```

#### `browse_select`
Select an option in a native `<select>` dropdown.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Select element `@e` reference |
| `value` | string | yes | -- | Option value to select |

**Behavior:** Looks up element, focuses it, sets `.value`, dispatches `change` and `input` events.

#### `browse_type`
Type text with realistic per-character keystroke delays.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Input field `@e` reference |
| `text` | string | yes | -- | Text to type character-by-character |
| `delay_ms` | integer | no | `50` | Milliseconds between keystrokes |

**Behavior:** Focuses element, then for each character: appends to `.value`, dispatches `input` event. After all characters: dispatches `change` and `blur`.

#### `browse_hover`
Hover over an element (triggers CSS :hover styles and JS mouseover handlers).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference |

#### `browse_key_press`
Press a keyboard key.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `key` | string | yes | -- | Key name: `"Enter"`, `"Tab"`, `"Escape"`, `"ArrowDown"`, `"Backspace"`, etc. |

#### `dom_focus`
Focus a specific element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference |

---

### Form Automation (5 tools)

#### `browse_upload_file`
Upload a file to an `input[type="file"]` element. Handles hidden file inputs commonly used by modern web applications.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | no | -- | File input `@e` reference (auto-detects first file input if omitted) |
| `file_path` | string | yes | -- | Absolute path to file on disk |

**Behavior:**
1. Reads file from disk, base64-encodes it
2. Looks up element by ref_id, or scans all `input[type="file"]` nodes (including hidden ones)
3. Creates `File` and `DataTransfer` objects in QuickJS
4. Sets `input.files = dt.files`
5. Dispatches `change` and `input` events

**Example:**
```
browse_upload_file { "file_path": "/home/user/resume.pdf", "ref_id": 48 }
# Returns: "OK: uploaded resume.pdf (11403 bytes)"
```

#### `browse_submit_form`
Submit a form. ATS-aware -- detects supported applicant tracking systems and POSTs to the correct API endpoint.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Submit button or form element `@e` reference |

**Behavior:**
1. Serializes all `<input>`, `<select>`, `<textarea>` values from the DOM
2. Detects ATS platform from the current URL
3. Constructs the correct API endpoint and content type (multipart, form-urlencoded, or JSON depending on the platform)
4. POSTs via Wraith's native HTTP client with proper `Origin` and `Referer` headers
5. Reports field count, endpoint, and HTTP response

#### `browse_custom_dropdown`
Interact with React-based custom dropdown components (not native `<select>`).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Combobox trigger element `@e` reference |
| `value` | string | yes | -- | Option text to select |

**Behavior:** Clicks the trigger to open the dropdown, types the value to filter options, looks for a matching option element, clicks it. Reports whether an exact match was found.

#### `browse_dismiss_overlay`
Dismiss a modal, overlay, popup, or cookie banner that is blocking interaction.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | no | -- | Overlay element `@e` reference (auto-detects the topmost overlay if omitted) |

**Behavior:** Automatically finds the close/dismiss/accept button within the overlay and clicks it. Returns an updated page snapshot after dismissal.

#### `browse_enter_iframe`
Enter an iframe's content by switching the page context to the iframe's parsed DOM.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Iframe element `@e` reference from the snapshot |

**Behavior:** Switches DOM context to the iframe's content. After entering, `browse_snapshot` shows the iframe's elements. Use `browse_back` to return to the parent page. Useful for cross-origin iframes used by embedded application forms and third-party widgets.

---

### Extraction & DOM (13 tools)

#### `browse_snapshot`
Get the current page's DOM as an agent-readable snapshot. Shows all interactive elements with `@e` reference IDs.

No parameters.

**Behavior:** Walks the full DOM tree, assigns sequential `@e` ref IDs to visible elements, queries QuickJS for current `.value` properties on form inputs (reflects values set by `browse_fill`), and formats as compact text.

**Output format:**
```
Page: "Job Application" (https://example.com/apply)

@e1   [text]     "Jane" value="Jane" placeholder="First Name"
@e2   [text]     "" placeholder="Last Name"
@e3   [email]    "" placeholder="Email"
@e4   [file]     ""
@e5   [button]   "Submit Application"
```

#### `browse_extract`
Extract page content as clean markdown (removes navigation, ads, boilerplate).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `max_tokens` | integer | no | unlimited | Token budget for output |

#### `browse_screenshot`
Capture a PNG screenshot of the page (base64-encoded).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `full_page` | bool | no | `false` | Capture entire page vs. viewport only |

#### `browse_eval_js`
Execute arbitrary JavaScript in the page's QuickJS context.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `code` | string | yes | -- | JavaScript source code to execute |

**Available globals:** `document`, `window`, `navigator`, `fetch`, `XMLHttpRequest`, `localStorage`, `sessionStorage`, `setTimeout`, `Event`, `InputEvent`, `KeyboardEvent`, `MouseEvent`, `FocusEvent`, `HTMLInputElement`, `HTMLElement`, `FormData`, `DataTransfer`, `File`, `Blob`, `MutationObserver`, `Promise`, `URL`, `TextEncoder`, `TextDecoder`, `crypto.subtle`, `performance`.

**Example:**
```
browse_eval_js { "code": "document.querySelectorAll('input').length" }
# Returns: "22"
```

#### `browse_tabs`
Show current page URL and title. No parameters.

#### `dom_query_selector`
Run a CSS selector query against the page DOM.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `selector` | string | yes | -- | CSS selector (supports tag, `#id`, `.class`, `[attr="val"]`, `*`, comma-separated) |

#### `dom_get_attribute`
Read an HTML attribute from an element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference |
| `name` | string | yes | -- | Attribute name (e.g., `"href"`, `"data-field-id"`, `"class"`) |

#### `dom_set_attribute`
Set an HTML attribute on an element.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `ref_id` | integer | yes | -- | Element `@e` reference |
| `name` | string | yes | -- | Attribute name |
| `value` | string | yes | -- | New attribute value |

#### `extract_pdf`
Fetch a PDF from a URL and extract its text content.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | PDF URL |

#### `extract_article`
Extract the main article body using readability analysis.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `readability` | bool | no | `true` | Use readability extraction algorithm |

#### `extract_markdown`
Convert HTML to clean markdown.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `html` | string | no | -- | Raw HTML (uses current page if omitted) |

#### `extract_plain_text`
Convert HTML to plain text.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `html` | string | no | -- | Raw HTML (uses current page if omitted) |

#### `extract_ocr`
Run OCR text detection on the current page.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | no | -- | What to OCR |

---

### Search (1 tool)

#### `browse_search`
Web metasearch via multiple search providers. Supports OR query splitting.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Search query (supports `OR` for multi-variant search) |
| `max_results` | integer | no | `10` | Maximum results |

**Behavior:** Splits `"QA engineer OR SDET remote"` into two sub-queries, searches each, deduplicates, and returns combined results.

---

### Authentication (2 tools)

#### `browse_login`
Perform a full login flow: navigate to a login page, fill credentials, submit, and follow the entire redirect chain (302 -> 302 -> 200). Captures all cookies at every redirect hop.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | Login page URL |
| `username_ref_id` | integer | yes | -- | `@e` reference of the username/email input field |
| `password_ref_id` | integer | yes | -- | `@e` reference of the password input field |
| `username` | string | yes | -- | Username or email to fill |
| `password` | string | yes | -- | Password to fill |
| `submit_ref_id` | integer | yes | -- | `@e` reference of the submit/login button |

**Behavior:** Navigates to the login URL, fills credentials using React-compatible fill, clicks submit, and follows all OAuth/auth redirects. Captures all `Set-Cookie` headers at every redirect hop. Returns the final page snapshot and all cookies set during the flow.

#### `browse_solve_captcha`
Solve a CAPTCHA on the current page using a third-party solving service. Supports common CAPTCHA types including reCAPTCHA v3 and Turnstile challenges.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `site_key` | string | no | -- | CAPTCHA site key (auto-detected from page if omitted) |
| `url` | string | no | -- | Page URL where the CAPTCHA appears (uses current page if omitted) |
| `captcha_type` | string | no | `"recaptchav3"` | CAPTCHA type: `"recaptchav3"` or `"turnstile"` |

**Requires:** `TWOCAPTCHA_API_KEY` environment variable. Returns the solved token and injects it into the page.

---

### Vault / Credential Management (12 tools)

All credentials are encrypted with AES-256-GCM using an Argon2id-derived master key. Secrets never appear in LLM context windows or logs. Secrets are zeroized from memory immediately after use.

#### `browse_vault_store`
Store a credential in the encrypted vault.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Domain the credential is for (e.g., `"example.com"`) |
| `kind` | string | yes | -- | Type: `"password"`, `"api_key"`, `"oauth_token"`, `"totp_seed"`, `"session_cookie"`, `"generic"` |
| `identity` | string | yes | -- | Username, email, or account identifier |
| `secret` | string | yes | -- | The secret value (encrypted at rest) |

#### `browse_vault_get`
Retrieve a credential from the vault.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Domain to look up |
| `kind` | string | no | -- | Optional type filter |

#### `browse_vault_list`
List all stored credentials (secrets redacted). No parameters.

#### `browse_vault_delete`
Delete a credential by its UUID.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `id` | string | yes | -- | Credential UUID |

#### `browse_vault_totp`
Generate a TOTP 2FA code for a domain (uses stored `totp_seed`).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Domain to generate TOTP for |

#### `browse_vault_rotate`
Rotate a credential's secret value.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `id` | string | yes | -- | Credential UUID |
| `new_secret` | string | yes | -- | New secret value |

#### `browse_vault_audit`
View the vault audit log (who accessed what, when).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `limit` | integer | no | `20` | Number of recent entries |

#### `vault_lock`
Lock the vault and zeroize the master key from memory. No parameters.

#### `vault_unlock`
Unlock the vault with a passphrase.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `passphrase` | string | no | `""` | Vault passphrase (empty string for auto-unlock) |

#### `vault_approve_domain`
Approve a domain to access a specific credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | yes | -- | Credential UUID |
| `domain` | string | yes | -- | Domain to approve |

#### `vault_revoke_domain`
Revoke a domain's access to a credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | yes | -- | Credential UUID |
| `domain` | string | yes | -- | Domain to revoke |

#### `vault_check_approval`
Check if a domain is approved to access a credential.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `credential_id` | string | yes | -- | Credential UUID |
| `domain` | string | yes | -- | Domain to check |

---

### Cookies (5 tools)

#### `cookie_get`
Get all cookies for a domain.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Cookie domain |

#### `cookie_set`
Set a cookie in the cookie jar.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Cookie domain |
| `name` | string | yes | -- | Cookie name |
| `value` | string | yes | -- | Cookie value |
| `path` | string | no | `"/"` | Cookie path |

#### `cookie_save`
Persist cookies to a JSON file on disk.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | no | `~/.openclaw/cookies.json` | File path |

#### `cookie_load`
Load cookies from a JSON file.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | no | `~/.openclaw/cookies.json` | File path |

#### `cookie_import_chrome`
Import cookies directly from Chrome's SQLite cookie database. Reuse existing login sessions without re-authenticating.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `profile` | string | no | `"Default"` | Chrome profile name |
| `domain` | string | no | -- | Optional domain filter (imports all if omitted) |

---

### Cache / Knowledge Store (10 tools)

Every page visited is cached, indexed, and searchable. Cache TTLs adapt automatically per domain based on observed content change frequency.

#### `cache_search`
Full-text search across all cached pages.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Search query |
| `max_results` | integer | no | `10` | Maximum results |

#### `cache_get`
Check if a URL is cached and return its content.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | URL to look up |

#### `cache_stats`
Show cache statistics (total pages, size, oldest/newest). No parameters.

#### `cache_purge`
Purge stale cache entries based on TTL. No parameters.

#### `cache_pin`
Pin a URL so it's never evicted from cache.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | URL to pin |
| `notes` | string | no | -- | Optional note explaining why it's pinned |

#### `cache_tag`
Tag a cached page with labels for organized retrieval.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | URL to tag |
| `tags` | string[] | yes | -- | Tags to apply (e.g., `["job-listing", "remote", "rust"]`) |

#### `cache_domain_profile`
Show a domain's observed change frequency and recommended TTL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Domain to profile |

#### `cache_find_similar`
Find cached pages similar to a given URL (semantic similarity).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | URL to find similar pages for |
| `max_results` | integer | no | `5` | Maximum results |

#### `cache_evict`
Evict cached pages to fit within a byte budget.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `max_bytes` | integer | yes | -- | Maximum cache size in bytes |

#### `cache_raw_html`
Get the raw cached HTML for a URL.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | yes | -- | URL to get HTML for |

---

### Entity Graph / Knowledge Graph (7 tools)

Cross-site entity resolution. Tracks companies, people, technologies, and their relationships across all visited pages.

#### `entity_query`
Ask a natural-language question about the knowledge graph.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `question` | string | yes | -- | Question (e.g., `"what do we know about this company?"`) |

#### `entity_add`
Add an entity to the knowledge graph.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Entity name |
| `entity_type` | string | yes | -- | `"company"`, `"person"`, `"technology"`, `"product"`, `"location"`, `"other"` |
| `attributes` | object | no | -- | Key-value metadata |

#### `entity_relate`
Create a relationship between two entities.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `from` | string | yes | -- | Source entity name |
| `to` | string | yes | -- | Target entity name |
| `relationship` | string | yes | -- | Relationship type (e.g., `"uses"`, `"employs"`, `"competes_with"`) |

#### `entity_merge`
Merge two entities that refer to the same thing.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name_a` | string | yes | -- | First entity |
| `name_b` | string | yes | -- | Second entity |

#### `entity_find_related`
Find all entities related to a given entity.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Entity name |

#### `entity_search`
Fuzzy search entities by name.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | yes | -- | Search query |

#### `entity_visualize`
Generate a Mermaid diagram of the knowledge graph. No parameters.

---

### Embeddings / Semantic Search (2 tools)

#### `embedding_search`
Find content semantically similar to a text query.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `text` | string | yes | -- | Query text |
| `top_k` | integer | no | `5` | Number of results |

#### `embedding_upsert`
Store a text embedding for later semantic search.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `source_id` | string | yes | -- | Unique ID (URL or document ID) |
| `content` | string | yes | -- | Text content to embed |

---

### Identity & Network Intelligence (8 tools)

#### `auth_detect`
Detect authentication flows on a page (login forms, OAuth, SSO).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | -- | URL to analyze (uses current page if omitted) |

#### `fingerprint_list`
List available browser fingerprint profiles. No parameters.

#### `fingerprint_import`
Import a fingerprint profile from a JSON file.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | yes | -- | Path to fingerprint JSON |

#### `identity_profile`
Set the browsing identity profile.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `profile_type` | string | yes | -- | `"personal"` or `"anonymous"` |
| `name` | string | no | -- | Name for personal profile |

#### `network_discover`
Discover API endpoints from network traffic on the current page. No parameters.

#### `dns_resolve`
Resolve a domain via DNS-over-HTTPS.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `domain` | string | yes | -- | Domain to resolve |

#### `stealth_status`
Show current TLS compatibility configuration and status. No parameters.

#### `site_fingerprint`
Detect the technology stack of a website (frameworks, CDN, analytics).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | -- | URL to fingerprint (uses current page if omitted) |

---

### Page Analysis (3 tools)

#### `page_diff`
Compare the current page content against the cached version (semantic diff).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | -- | URL to diff (uses current page if omitted) |

#### `tls_profiles`
List available TLS fingerprint profiles for broad site compatibility. No parameters.

#### `tls_verify`
Verify that Wraith's TLS fingerprint matches a real modern browser. Compares cipher suites, extensions, and HTTP/2 settings against known browser values.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | no | -- | TLS fingerprint analysis service URL |

**Behavior:** Fetches a TLS analysis service using the same HTTP stack as `browse_navigate`, then compares JA3/JA4 hashes, cipher suites, extensions, and HTTP/2 SETTINGS against known browser values. Returns a detailed pass/fail report.

---

### Plugins (4 tools)

#### `plugin_register`
Register a WASM plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Plugin name |
| `wasm_path` | string | yes | -- | Path to `.wasm` file |
| `description` | string | no | -- | Plugin description |
| `domains` | string[] | no | -- | Domains the plugin applies to |

#### `plugin_execute`
Execute a registered plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Plugin name |
| `input` | object | no | -- | JSON input data for the plugin |

#### `plugin_list`
List all registered plugins. No parameters.

#### `plugin_remove`
Remove a registered plugin.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Plugin name |

---

### Scripting / Rhai (3 tools)

#### `script_load`
Load a Rhai userscript with optional navigation triggers.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Unique script name |
| `source` | string | yes | -- | Rhai source code |
| `trigger` | string | no | -- | `"always"`, `"manual"`, or URL substring to match |

#### `script_list`
List all loaded Rhai userscripts. No parameters.

#### `script_run`
Run a loaded script by name.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Script name to execute |

---

### Telemetry (2 tools)

#### `telemetry_metrics`
Show browsing metrics (pages visited, requests made, errors). No parameters.

#### `telemetry_spans`
Export OpenTelemetry-compatible performance trace spans. No parameters.

---

### Workflow Record & Replay (4 tools)

#### `workflow_start_recording`
Start recording a browsing workflow.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Workflow name |

#### `workflow_stop_recording`
Stop recording and save the workflow.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | yes | -- | What the workflow does |

#### `workflow_replay`
Replay a saved workflow with variable substitution.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | Workflow name |
| `variables` | object | no | -- | Key-value variable substitutions |

#### `workflow_list`
List all saved workflows. No parameters.

---

### Time-Travel / Agent Debugging (5 tools)

#### `timetravel_summary`
Show the agent's decision timeline (every action taken). No parameters.

#### `timetravel_branch`
Branch from a decision point to explore an alternative path.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `step` | integer | yes | -- | Step number to branch from (0-indexed) |
| `name` | string | yes | -- | Name for the new branch |

#### `timetravel_replay`
Replay the timeline up to a specific step.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `step` | integer | yes | -- | Replay up to this step |

#### `timetravel_diff`
Diff two timeline branches to see divergent outcomes.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `branch_a` | string | yes | -- | First branch ID |
| `branch_b` | string | yes | -- | Second branch ID |

#### `timetravel_export`
Export the full decision timeline as JSON. No parameters.

---

### Task DAG / Parallel Orchestration (7 tools)

#### `dag_create`
Create a new task DAG (directed acyclic graph).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `name` | string | yes | -- | DAG name |

#### `dag_add_task`
Add a task node to the DAG.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | yes | -- | Unique task ID |
| `description` | string | yes | -- | Human-readable description |
| `action_type` | string | yes | -- | `"navigate"`, `"click"`, `"fill"`, `"extract"`, `"custom"` |
| `target` | string | no | -- | URL, selector, or other target |

#### `dag_add_dependency`
Add a dependency edge between two tasks.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | yes | -- | Task that depends |
| `depends_on` | string | yes | -- | Task that must complete first |

#### `dag_ready`
Get all tasks that are ready to execute (all dependencies satisfied). No parameters.

#### `dag_complete`
Mark a task as completed with its result.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_id` | string | yes | -- | Task ID to mark complete |
| `result` | string | yes | -- | Result output |

#### `dag_progress`
Show DAG completion progress (tasks done, pending, blocked). No parameters.

#### `dag_visualize`
Generate a Mermaid diagram of the task DAG. No parameters.

---

### MCTS Planning (2 tools)

#### `mcts_plan`
Use Monte Carlo Tree Search to determine the best next action.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `state` | string | yes | -- | Current page state description |
| `actions` | string[] | yes | -- | Available actions (e.g., `["click @e1", "fill @e3 'Jane'"]`) |
| `simulations` | integer | no | `100` | Number of MCTS simulations |

#### `mcts_stats`
Show MCTS planner statistics. No parameters.

---

### Swarm / Parallel Browsing (2 tools)

#### `swarm_fan_out`
Visit multiple URLs in parallel.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `urls` | string[] | yes | -- | List of URLs to visit concurrently |
| `max_concurrent` | integer | no | `4` | Maximum concurrent fetches |

#### `swarm_collect`
Collect results from a previous `swarm_fan_out`. No parameters.

---

### Autonomous Agent (1 tool)

#### `browse_task`
Run an autonomous multi-step browsing task using an LLM agent loop.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `description` | string | yes | -- | Natural language task description |
| `url` | string | no | -- | Starting URL (agent decides if omitted) |
| `max_steps` | integer | no | `50` | Maximum steps before stopping |

**Requires:** `ANTHROPIC_API_KEY` environment variable.

---

### Prefetch (1 tool)

#### `prefetch_predict`
Predict URLs the agent will likely need next and pre-fetch them.

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `task_description` | string | yes | -- | Current task description |

---

### Script Execution (1 tool)

#### `browse_fetch_scripts`
Fetch and execute page `<script>` tags (both inline and external). Handles dynamic script creation (SPA bootstrap pattern).

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `max_bytes` | integer | no | `2097152` | Maximum total script bytes (2MB) |

**Behavior:** Finds all `<script>` tags in the page. Executes inline scripts first (may bootstrap SPAs). Then checks for dynamically created `<script>` elements (e.g., patterns where inline JS creates `<script type="module">`). Fetches external scripts via HTTP and executes them in QuickJS. Flushes `setTimeout` callbacks.

---

### Config (1 tool)

#### `browse_config`
Show engine capabilities and current configuration. No parameters.

Returns: JavaScript status, screenshot capability, layout capability, cookie support, compatibility mode, TLS status, proxy configuration.

---

## DOM Snapshots

Wraith produces compact, agent-readable DOM snapshots that show every interactive element with a unique `@e` reference ID. The format is designed for minimal token usage while giving the AI agent full context.

```
Page: "Apply Now" (https://example.com/careers/apply)

@e1   [text]     "Jane" value="Jane" placeholder="First Name"
@e2   [text]     "" placeholder="Last Name"
@e3   [email]    "" placeholder="Email"
@e4   [tel]      "" placeholder="Phone"
@e5   [file]     "" (accept=".pdf,.doc,.docx")
@e6   [select]   "United States" (options: ...)
@e7   [textarea] "" placeholder="Cover Letter"
@e8   [button]   "Submit Application"
```

Each `@e` reference can be used directly with `browse_click`, `browse_fill`, `browse_select`, `browse_upload_file`, and other interaction tools. Values reflect the current DOM state, including changes made by `browse_fill`.

## Form Automation

Wraith handles the full spectrum of web forms:

- **Static HTML forms** -- standard DOM interaction with `browse_fill` and `browse_submit_form`
- **React-controlled inputs** -- uses native value setters and `_valueTracker` invalidation to ensure React sees changes
- **Custom dropdowns** -- `browse_custom_dropdown` opens, filters, and selects from non-native dropdown components
- **File uploads** -- `browse_upload_file` injects files via `DataTransfer` API, handles hidden file inputs
- **ATS platforms** -- automatic API detection and direct submission using the platform's native API format
- **SPA hydration** -- `browse_fetch_scripts` downloads and executes JavaScript bundles so React/Vue/Angular event systems activate
- **Cross-origin iframes** -- `browse_enter_iframe` switches context into embedded forms

## Agent Intelligence

Wraith includes built-in AI planning and orchestration:

- **MCTS Planning** -- Monte Carlo Tree Search explores possible action sequences to find the optimal next step
- **Task DAGs** -- define parallel task graphs with dependencies; execute ready tasks concurrently
- **Time-Travel Debugging** -- replay, branch, and diff the agent's decision timeline
- **Workflow Record/Replay** -- record browsing sessions and replay them with variable substitution
- **Prefetch Prediction** -- anticipate which URLs the agent will need next and pre-fetch them
- **Swarm Browsing** -- visit multiple URLs in parallel and collect results

## Credential Security

- **AES-256-GCM** encryption at rest with **Argon2id** key derivation
- Credentials never appear in LLM context windows or log files
- Per-domain access controls with approval/revocation
- Automatic TOTP 2FA generation from stored seeds
- Chrome cookie import (reuse existing login sessions)
- Full audit trail of every credential access
- Secrets zeroized from memory immediately after use (via `secrecy` crate)

## Intelligent Caching

Every page visited is cached, indexed, and searchable. Cache TTLs adapt automatically per domain based on observed content change frequency.

- **SQLite + full-text search** index
- Semantic page diffing (detects meaningful changes between visits)
- Cross-site entity resolution via knowledge graph
- Embedding store with cosine similarity search
- Pin important pages, tag for organized retrieval
- Domain profiling -- learns how often sites change

## Plugin System

- **WASM plugins** (wasmtime) -- sandboxed, hot-reloadable, domain-specific extractors
- **Rhai scripting** -- userscripts that trigger on navigation events
- **Vision ML pipeline** (ort/ONNX) -- UI element detection for canvas/non-DOM content

## License

**AGPL-3.0** -- free and open source.

Use freely for personal projects, open source, research, and internal tools. If you modify Wraith and deploy it as a network service, modifications must be released under the same license.

### Commercial License

Companies that want to embed Wraith in proprietary products without open-source obligations can obtain a commercial license. Contact [ridgecellrepair@gmail.com](mailto:ridgecellrepair@gmail.com).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines. Key areas:

- ATS platform integrations (new API adapters)
- Search provider integrations
- Auth flow detection patterns
- Documentation and examples

## Acknowledgments

Built with [scraper](https://github.com/causal-agent/scraper), [rquickjs](https://crates.io/crates/rquickjs), [Tantivy](https://github.com/quickwit-oss/tantivy), [rmcp](https://crates.io/crates/rmcp), [ort](https://crates.io/crates/ort), [wasmtime](https://crates.io/crates/wasmtime), [petgraph](https://crates.io/crates/petgraph), and [reqwest](https://crates.io/crates/reqwest).

---

**Wraith** -- *the browser your AI agent deserves.*

Copyright (c) 2026 Matt Gates / Ridge Cell Repair LLC

---

## Support This Project

If you find this project useful, consider buying me a coffee! Your support helps me keep building and sharing open-source tools.

[![Donate via PayPal](https://img.shields.io/badge/Donate-PayPal-blue.svg?logo=paypal)](https://www.paypal.me/baal_hosting)

**PayPal:** [baal_hosting@live.com](https://paypal.me/baal_hosting)

Every donation, no matter how small, is greatly appreciated and motivates continued development. Thank you!
