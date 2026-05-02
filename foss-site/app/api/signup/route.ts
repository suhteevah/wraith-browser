/**
 * /api/signup — abuse-hardened proxy to the corpo API's /api/v1/auth/register.
 *
 * Layers (cheapest first):
 *   1. Origin / Referer must be wraith-browser.vercel.app (or env override)
 *   2. Hidden honeypot field must be empty
 *   3. Time-gate — submission must be >= 2 s after the form was rendered
 *   4. Optional Cloudflare Turnstile, gated behind CF_TURNSTILE_SECRET_KEY
 *
 * Direct calls to /api/v1/auth/register still work (preserves the curl
 * bootstrap path for internal agents). That endpoint is rate-limited at the
 * edge via middleware.ts to bound abuse there too.
 *
 * NOTE: Each layer is cheap and bypassable in isolation. Together they raise
 * the bar enough to deflect drive-by automated abuse during beta. They are
 * NOT a substitute for proper rate limiting + auth at the api-server level
 * — those land when we exit beta.
 */

import { NextResponse } from 'next/server';

const ALLOWED_ORIGINS = (process.env.SIGNUP_ALLOWED_ORIGINS ??
  'https://wraith-browser.vercel.app,http://localhost:3000')
  .split(',')
  .map((s) => s.trim())
  .filter(Boolean);

const UPSTREAM =
  process.env.WRAITH_CORPO_API ?? 'http://207.244.232.227:8080';

const MIN_FILL_MS = 2_000;
const MAX_FILL_MS = 60 * 60 * 1_000; // 1h — guard against pre-rendered ancient forms
const TURNSTILE_SECRET = process.env.CF_TURNSTILE_SECRET_KEY;

type Body = {
  email?: unknown;
  password?: unknown;
  org_name?: unknown;
  display_name?: unknown;
  honeypot?: unknown;
  rendered_at?: unknown;
  cf_turnstile_token?: unknown;
};

export async function POST(req: Request) {
  // ── 1. Origin check ──────────────────────────────────────────────────
  const origin = req.headers.get('origin') ?? req.headers.get('referer') ?? '';
  if (!ALLOWED_ORIGINS.some((o) => origin.startsWith(o))) {
    return json(403, {
      error: 'forbidden_origin',
      message: 'Submit the signup form from the website.',
    });
  }

  let body: Body;
  try {
    body = (await req.json()) as Body;
  } catch {
    return json(400, { error: 'invalid_json' });
  }

  // ── 2. Honeypot — must be empty / absent ─────────────────────────────
  if (typeof body.honeypot === 'string' && body.honeypot.length > 0) {
    // Don't tell the bot it was caught; pretend success silently.
    // Returning 200 with a fake-looking error keeps it expensive to debug.
    return json(200, {
      error: 'temporarily_unavailable',
      message: 'Please try again later.',
    });
  }

  // ── 3. Time-gate ─────────────────────────────────────────────────────
  const renderedAt = Number(body.rendered_at);
  if (!Number.isFinite(renderedAt)) {
    return json(400, { error: 'missing_rendered_at' });
  }
  const elapsed = Date.now() - renderedAt;
  if (elapsed < MIN_FILL_MS || elapsed > MAX_FILL_MS) {
    return json(429, {
      error: 'rate_limited',
      message: 'Slow down or refresh and try again.',
    });
  }

  // ── 4. Optional Turnstile ────────────────────────────────────────────
  if (TURNSTILE_SECRET) {
    const token =
      typeof body.cf_turnstile_token === 'string'
        ? body.cf_turnstile_token
        : '';
    if (!token) {
      return json(400, { error: 'missing_captcha' });
    }
    const ok = await verifyTurnstile(token, clientIp(req));
    if (!ok) {
      return json(403, { error: 'captcha_failed' });
    }
  }

  // ── 5. Validate the actual signup payload ────────────────────────────
  const { email, password, org_name, display_name } = body;
  if (typeof email !== 'string' || !email.includes('@')) {
    return json(400, { error: 'invalid_email' });
  }
  if (typeof password !== 'string' || password.length < 8) {
    return json(400, { error: 'weak_password' });
  }
  if (typeof org_name !== 'string' || org_name.trim().length === 0) {
    return json(400, { error: 'missing_org_name' });
  }

  // ── 6. Forward to the api-server ─────────────────────────────────────
  const upstream = await fetch(`${UPSTREAM}/api/v1/auth/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      email,
      password,
      org_name,
      display_name:
        typeof display_name === 'string' && display_name.length > 0
          ? display_name
          : undefined,
    }),
    // Don't keep alive forever — Vercel rewrite limit is ~30s anyway.
    signal: AbortSignal.timeout(25_000),
  });

  const upstreamBody = await upstream.text();
  return new NextResponse(upstreamBody, {
    status: upstream.status,
    headers: {
      'content-type':
        upstream.headers.get('content-type') ?? 'application/json',
    },
  });
}

async function verifyTurnstile(token: string, ip: string): Promise<boolean> {
  try {
    const res = await fetch(
      'https://challenges.cloudflare.com/turnstile/v0/siteverify',
      {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({
          secret: TURNSTILE_SECRET ?? '',
          response: token,
          remoteip: ip,
        }),
        signal: AbortSignal.timeout(5_000),
      },
    );
    const data = (await res.json()) as { success?: boolean };
    return data.success === true;
  } catch {
    return false;
  }
}

function clientIp(req: Request): string {
  return (
    req.headers.get('x-real-ip') ??
    req.headers.get('x-forwarded-for')?.split(',')[0]?.trim() ??
    ''
  );
}

function json(status: number, body: unknown) {
  return NextResponse.json(body, { status });
}
