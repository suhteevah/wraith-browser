# Indeed Login Flow — Step-by-Step for Job Hunter MCP

Indeed uses a two-step JS-driven login. Two paths are supported:
- **Path A: Google SSO** (recommended — most users already have this)
- **Path B: Email + Password** (direct Indeed credentials)

Use CDP mode (`browse_navigate_cdp`) because Indeed's login is a React SPA.

## Prerequisites

- FlareSolverr running at localhost:8191
- `WRAITH_FLARESOLVERR=http://localhost:8191` set in environment
- A Google account linked to Indeed (Path A) or Indeed email+password (Path B)

---

## Path A: Google SSO Login (Recommended)

### 1. Navigate to login page via CDP

```
browse_navigate_cdp url="https://secure.indeed.com/auth"
```

Expected: Page titled "Sign In | Indeed Accounts" with a "Continue with Google" button.

### 2. Click "Continue with Google"

Look for: `[button]` or `[a]` containing "Google" in the snapshot. Note the @ref ID.

```
browse_click ref_id=<GOOGLE_BTN_REF>
```

Expected: Redirects to `accounts.google.com` — Google's login page.

### 3. Snapshot Google login page

```
browse_snapshot
```

Look for: `[input] "" type="email"` — the Google email field.

### 4. Fill Google email

```
browse_fill ref_id=<GOOGLE_EMAIL_REF> text="your_google@gmail.com"
```

### 5. Click "Next"

```
browse_click ref_id=<NEXT_BTN_REF>
```

### 6. Wait for password page and snapshot

Google shows email first, then password (same two-step pattern as Indeed).

```
browse_snapshot
```

Look for: `[input] "" type="password"` — the Google password field.

### 7. Fill Google password

```
browse_fill ref_id=<GOOGLE_PASSWORD_REF> text="your_google_password"
```

### 8. Click "Next" (sign in)

```
browse_click ref_id=<SIGNIN_BTN_REF>
```

### 9. Handle 2FA (if enabled)

If your Google account has 2FA, you'll see a verification prompt.

**Option A — Authenticator app / phone prompt**:
```
browse_snapshot
```
Check your phone, approve the prompt, then re-snapshot. Google will redirect automatically.

**Option B — SMS/backup code**:
```
browse_snapshot
browse_fill ref_id=<CODE_REF> text="123456"
browse_click ref_id=<VERIFY_BTN>
```

**Option C — No 2FA**: Skip this step. Google redirects back to Indeed automatically.

### 10. Verify redirect back to Indeed

```
browse_snapshot
```

Expected: Google redirects → Indeed OAuth callback → Indeed homepage/dashboard.
Page title should be something like "Job Search | Indeed" or your Indeed dashboard.

### 11. Save cookies

```
cookie_save path="~/.wraith/indeed_cookies.json"
```

### 12. Use cookies for paginated search

```
cookie_load path="~/.wraith/indeed_cookies.json"
browse_navigate url="https://www.indeed.com/jobs?q=AI+engineer&l=remote&start=0&sort=date"
```

For page 2+:
```
browse_navigate url="https://www.indeed.com/jobs?q=AI+engineer&l=remote&start=10&sort=date"
```

---

## Path B: Email + Password Login

### 1. Navigate to login page via CDP

```
browse_navigate_cdp url="https://secure.indeed.com/auth"
```

### 2. Fill email

Look for: `[input] "" name="__email"` in the snapshot.

```
browse_fill ref_id=<EMAIL_REF> text="your_email@example.com"
```

### 3. Click "Continue"

```
browse_click ref_id=<CONTINUE_REF>
```

### 4. Wait for password field

```
browse_snapshot
```

Look for: `[input] "" type="password"` in the new snapshot.

### 5. Fill password

```
browse_fill ref_id=<PASSWORD_REF> text="your_password"
```

### 6. Click "Sign in"

```
browse_click ref_id=<SIGNIN_REF>
```

### 7. Verify and save cookies

```
browse_snapshot
cookie_save path="~/.wraith/indeed_cookies.json"
```

### 8. Use cookies for pagination

Same as Path A step 12.

---

## Cookie Refresh

Indeed session cookies last ~7 days (based on SURF cookie expiry).
Re-run the login flow weekly, or when you start getting auth redirects again.

## Troubleshooting

- **Turnstile CAPTCHA on login**: Indeed may show a Cloudflare Turnstile challenge during login.
  CDP mode uses real Chrome so it can handle most challenges automatically.
  If interactive Turnstile blocks, manually log in via your browser,
  export cookies with a browser extension, and use `cookie_load`.

- **"Request Blocked" on CDP navigate**: Your IP may be flagged. Try again after a few minutes,
  or use a proxy: `browse_config proxy="socks5://127.0.0.1:1080"`

- **Google 2FA stuck**: If the 2FA prompt requires a phone tap that the agent can't do,
  have Matt approve it on his phone, then the agent can continue with `browse_snapshot`.

- **Google "Choose an account" screen**: If multiple Google accounts are signed in,
  snapshot and click the correct account from the list.

- **Google "This browser or app may not be secure"**: Google sometimes blocks
  automated Chrome. If this happens, use the manual cookie export approach instead.
