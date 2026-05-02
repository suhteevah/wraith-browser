import { createMDX } from 'fumadocs-mdx/next';

// Wraith Corpo API origin. Vercel proxies the user-facing HTTPS request from
// wraith-browser.vercel.app to the VPS over plaintext HTTP.
//
// Caveats:
//   - WebSocket upgrades are NOT proxied by Vercel rewrites. /ws/* routes
//     must hit the IP directly (or a WS-capable proxy) until we put real TLS
//     on the VPS via Caddy.
//   - The Vercel→VPS leg is cleartext HTTP. JWTs and credentials are exposed
//     to anyone with packet capture on that hop. OK for internal use; NOT
//     production-grade. Real fix is `https://api.<domain>` with Caddy + LE.
//   - Vercel applies a per-request timeout to rewrite proxies (~30s on Hobby).
//     Long-running endpoints (swarm fan-out, browser navigate-and-wait) may
//     need to be called against the IP directly.
const WRAITH_CORPO_API = process.env.WRAITH_CORPO_API
  ?? 'http://207.244.232.227:8080';

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  async rewrites() {
    return [
      { source: '/api/v1/:path*',  destination: `${WRAITH_CORPO_API}/api/v1/:path*` },
      { source: '/health',         destination: `${WRAITH_CORPO_API}/health` },
      { source: '/metrics',        destination: `${WRAITH_CORPO_API}/metrics` },
    ];
  },
};

const withMDX = createMDX();

export default withMDX(config);
