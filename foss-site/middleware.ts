/**
 * Edge middleware — rate-limits sensitive corpo-API endpoints exposed via the
 * Vercel rewrite. Direct calls to the VPS (207.244.232.227:8080) bypass this
 * entirely, which is intentional: internal scripts/agents already have to
 * know that direct path.
 *
 * State is per-edge-region in-memory (Vercel doesn't give middleware durable
 * KV without explicit setup). That means a determined attacker can rotate
 * across regions, but it stops drive-by curl loops cold.
 *
 * For the harder limits, swap the in-memory map for Vercel KV / Upstash
 * Redis once we're paying for it.
 */

import { NextResponse, type NextRequest } from 'next/server';

export const config = {
  matcher: [
    '/api/v1/auth/register',
    '/api/v1/auth/login',
    '/api/signup',
  ],
};

const WINDOW_MS = 60 * 60 * 1_000; // 1 hour
const MAX_PER_WINDOW: Record<string, number> = {
  '/api/v1/auth/register': 10,
  '/api/v1/auth/login': 60,
  '/api/signup': 10,
};

type Bucket = { count: number; resetAt: number };
const buckets = new Map<string, Bucket>();

export function middleware(req: NextRequest) {
  const path = new URL(req.url).pathname;
  const limit = MAX_PER_WINDOW[path];
  if (!limit) return NextResponse.next();

  const ip =
    req.headers.get('x-real-ip') ??
    req.headers.get('x-forwarded-for')?.split(',')[0]?.trim() ??
    'unknown';
  const key = `${path}::${ip}`;

  const now = Date.now();
  const bucket = buckets.get(key);
  if (!bucket || bucket.resetAt < now) {
    buckets.set(key, { count: 1, resetAt: now + WINDOW_MS });
  } else {
    bucket.count += 1;
    if (bucket.count > limit) {
      const retryAfter = Math.max(1, Math.ceil((bucket.resetAt - now) / 1000));
      return new NextResponse(
        JSON.stringify({
          error: 'rate_limited',
          message: `Too many requests. Retry in ${retryAfter}s.`,
        }),
        {
          status: 429,
          headers: {
            'content-type': 'application/json',
            'retry-after': String(retryAfter),
            'x-ratelimit-limit': String(limit),
            'x-ratelimit-remaining': '0',
            'x-ratelimit-reset': String(Math.ceil(bucket.resetAt / 1000)),
          },
        },
      );
    }
  }

  return NextResponse.next();
}
