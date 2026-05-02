import type { Metadata } from 'next';
import Link from 'next/link';

export const metadata: Metadata = {
  title: 'Wraith vs Playwright, Puppeteer, Browserbase, Apify, Bright Data',
  description:
    'Honest comparison of Wraith Browser against the dominant browser-automation and scraping tools. Where Wraith wins, where it loses, and when to pick something else.',
};

type Cell = string | { value: string; note?: string };

interface Row {
  dim: string;
  wraith: Cell;
  playwright: Cell;
  puppeteer: Cell;
  browserbase: Cell;
  apify: Cell;
  brightdata: Cell;
}

const rows: Row[] = [
  {
    dim: 'Engine',
    wraith: 'Native Rust (html5ever, rquest, QuickJS). No Chrome dependency.',
    playwright: 'Drives real Chromium / Firefox / WebKit via CDP/RDP.',
    puppeteer: 'Drives real Chrome / Chromium via CDP. Node-only.',
    browserbase: 'Hosted real Chromium fleet (CDP over WebSocket).',
    apify: 'Crawlee wraps Playwright / Puppeteer / Cheerio.',
    brightdata: 'Hosted real Chromium + residential proxy network.',
  },
  {
    dim: 'Cold-start latency per page',
    wraith: '~50ms engine fetch; ~500-700ms full CLI round-trip incl. process startup',
    playwright: '~1-3s with browser launch; ~100-300ms with reused context',
    puppeteer: '~1-3s with browser launch; ~100-300ms with reused context',
    browserbase: '~2-5s session create + page load (network round-trip to their cloud)',
    apify: 'Inherits Playwright/Puppeteer latency',
    brightdata: '~3-8s incl. proxy hop + unblock pipeline',
  },
  {
    dim: 'Memory per concurrent session',
    wraith: '~150MB API server idle; ~8-12MB per logical session in MCP mode',
    playwright: '~300-500MB per Chromium browser process',
    puppeteer: '~300-500MB per Chrome process',
    browserbase: 'Hosted — billed per minute, not per MB',
    apify: 'Inherits underlying browser cost',
    brightdata: 'Hosted — billed per request',
  },
  {
    dim: 'Concurrent sessions on a single 16GB box',
    wraith: 'Hundreds to thousands (MCP-mode logical sessions)',
    playwright: '~30-50 real browser contexts before swap pressure',
    puppeteer: '~30-50 real Chrome instances',
    browserbase: 'Unbounded (their cloud)',
    apify: 'Unbounded (their cloud) or local-bounded',
    brightdata: 'Unbounded (their cloud)',
  },
  {
    dim: 'TLS / JA3 fingerprint',
    wraith: 'Spoofed via rquest/BoringSSL. Chrome 131 (Win/Mac), Firefox 132, Safari 18 profiles.',
    playwright: 'Real Chromium fingerprint — but easily flagged as automation when run headless',
    puppeteer: 'Real Chrome fingerprint — flagged as headless without patches',
    browserbase: 'Real Chromium with their stealth layer',
    apify: 'Inherits Playwright/Puppeteer; "Stealth" plugin available',
    brightdata: 'Real Chromium + their unblock pipeline',
  },
  {
    dim: 'Cloudflare / Turnstile handling',
    wraith: '4-tier auto: direct (rquest) → QuickJS solver → FlareSolverr → proxy fallback',
    playwright: 'Manual; usually requires patched build + 3rd-party solver',
    puppeteer: 'puppeteer-extra-plugin-stealth + 3rd-party solver',
    browserbase: 'Built-in stealth; no Turnstile guarantee',
    apify: 'Apify Anti-Scraping Toolkit (paid add-on)',
    brightdata: 'Built into Web Unlocker product',
  },
  {
    dim: 'AI-agent ergonomics',
    wraith: '130 MCP tools, @ref-based snapshots designed for LLM token budgets',
    playwright: 'Human SDK; LLM has to read full HTML or you build your own snapshot layer',
    puppeteer: 'Human SDK; same situation',
    browserbase: 'Stagehand + AI primitives layered on top of Playwright',
    apify: 'No native MCP; Apify Actors callable from agents via REST',
    brightdata: 'REST API, no MCP layer',
  },
  {
    dim: 'Hosted price (as of 2026-05)',
    wraith: 'Free during beta. Plans: Growth $199/mo, Scale $799/mo, Enterprise custom.',
    playwright: 'N/A — self-host only',
    puppeteer: 'N/A — self-host only',
    browserbase: 'From ~$0.10/browser-min (verify at browserbase.com/pricing)',
    apify: 'Free tier + usage-based on compute units (apify.com/pricing)',
    brightdata: '~$3 / 1,000 requests Web Unlocker (verify at brightdata.com/pricing)',
  },
  {
    dim: 'Self-host license',
    wraith: 'AGPL-3.0',
    playwright: 'Apache-2.0',
    puppeteer: 'Apache-2.0',
    browserbase: 'Closed source',
    apify: 'Crawlee Apache-2.0; platform closed',
    brightdata: 'Closed source',
  },
  {
    dim: 'Defense / enterprise career-site hydrators',
    wraith: 'Native API hydrators for Boeing, L3Harris, Lockheed, MITRE, RTX (Radancy / Phenom / Workday). Bypasses SPA render.',
    playwright: 'Renders the SPA shell, then hits the same APIs through it. Slower.',
    puppeteer: 'Same as Playwright — render first, fetch second.',
    browserbase: 'Same model; you write the hydrator.',
    apify: 'Has Workday / generic ATS actors but they render-then-scrape.',
    brightdata: 'No domain-specific hydrators; raw unblock.',
  },
  {
    dim: 'Visual rendering / pixel-perfect screenshots',
    wraith: 'No layout engine. Servo port in progress for the bare-metal port; not in the API today.',
    playwright: 'Yes — full Chromium rendering',
    puppeteer: 'Yes — full Chrome rendering',
    browserbase: 'Yes — real Chromium',
    apify: 'Yes (when using Playwright/Puppeteer crawler)',
    brightdata: 'Yes',
  },
  {
    dim: 'Browser extension support',
    wraith: 'No',
    playwright: 'Yes (Chromium extensions)',
    puppeteer: 'Yes',
    browserbase: 'Limited',
    apify: 'Inherits Playwright',
    brightdata: 'No',
  },
];

function renderCell(c: Cell) {
  if (typeof c === 'string') return c;
  return c.value;
}

type ColKey = 'wraith' | 'playwright' | 'puppeteer' | 'browserbase' | 'apify' | 'brightdata';

const cols: ReadonlyArray<{ key: ColKey; label: string; accent?: boolean }> = [
  { key: 'wraith', label: 'Wraith', accent: true },
  { key: 'playwright', label: 'Playwright' },
  { key: 'puppeteer', label: 'Puppeteer' },
  { key: 'browserbase', label: 'Browserbase' },
  { key: 'apify', label: 'Apify / Crawlee' },
  { key: 'brightdata', label: 'Bright Data' },
];

export default function VsPage() {
  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      {/* Hero */}
      <section className="px-6 pt-24 pb-12 max-w-4xl mx-auto text-center">
        <h1 className="text-4xl md:text-5xl font-bold tracking-tight">
          Why Wraith over Chrome-based automation?
        </h1>
        <p className="mt-6 text-lg text-fd-muted-foreground max-w-2xl mx-auto">
          Wraith is a native Rust browser engine. No Chromium process per
          session, no Selenium driver, no headless-detection arms race. Below is
          an honest comparison against the tools you&apos;re probably already
          using — including where Wraith is the wrong choice.
        </p>
      </section>

      {/* Comparison table — desktop */}
      <section className="px-6 pb-16 max-w-7xl mx-auto hidden lg:block">
        <div className="overflow-x-auto rounded-xl border border-fd-border">
          <table className="w-full text-sm">
            <thead>
              <tr className="bg-fd-card">
                <th className="text-left px-4 py-3 font-semibold w-48">Dimension</th>
                {cols.map((c) => (
                  <th
                    key={c.key}
                    className={`text-left px-4 py-3 font-semibold ${
                      c.accent ? 'text-emerald-400' : ''
                    }`}
                  >
                    {c.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr
                  key={r.dim}
                  className={i % 2 === 0 ? 'bg-fd-background' : 'bg-fd-card/40'}
                >
                  <td className="px-4 py-3 font-medium align-top text-fd-foreground">
                    {r.dim}
                  </td>
                  {cols.map((c) => (
                    <td
                      key={c.key}
                      className={`px-4 py-3 align-top text-fd-muted-foreground ${
                        c.accent ? 'text-fd-foreground' : ''
                      }`}
                    >
                      {renderCell(r[c.key])}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="mt-3 text-xs text-fd-muted-foreground">
          Pricing and positioning verified as of 2026-05. Confirm current
          numbers at the linked vendor pages before committing.
        </p>
      </section>

      {/* Comparison cards — mobile */}
      <section className="px-6 pb-16 max-w-4xl mx-auto lg:hidden space-y-6">
        {cols.map((c) => (
          <div
            key={c.key}
            className={`rounded-xl border p-5 ${
              c.accent
                ? 'border-emerald-500/40 bg-emerald-500/5'
                : 'border-fd-border bg-fd-card'
            }`}
          >
            <h3
              className={`text-xl font-semibold mb-4 ${
                c.accent ? 'text-emerald-400' : ''
              }`}
            >
              {c.label}
            </h3>
            <dl className="space-y-3 text-sm">
              {rows.map((r) => (
                <div key={r.dim}>
                  <dt className="font-medium text-fd-foreground">{r.dim}</dt>
                  <dd className="text-fd-muted-foreground mt-1">
                    {renderCell(r[c.key])}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        ))}
      </section>

      {/* When to choose Wraith */}
      <section className="px-6 py-16 max-w-5xl mx-auto">
        <h2 className="text-3xl font-bold mb-10 text-center">
          When to choose Wraith
        </h2>
        <div className="grid md:grid-cols-3 gap-6">
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">
              Defense-contractor recruiting
            </h3>
            <p className="text-fd-muted-foreground text-sm">
              Boeing, L3Harris, Lockheed, MITRE, and RTX run Radancy, Phenom,
              and Workday under the hood. Wraith ships native API hydrators for
              all three platforms — pulling structured job data without
              rendering the SPA. Faster and more reliable than driving a
              Chromium instance through their JavaScript bundle.
            </p>
          </div>
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">
              High-volume API scraping
            </h3>
            <p className="text-fd-muted-foreground text-sm">
              When you need hundreds of concurrent sessions on one box, the
              ~150MB API-server footprint and per-session costs measured in
              megabytes (not hundreds) matter. The token-snapshot model also
              cuts LLM input costs by ~90-95% vs feeding raw HTML.
            </p>
          </div>
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">
              LLM agent control loops
            </h3>
            <p className="text-fd-muted-foreground text-sm">
              130 MCP tools, @ref-addressable snapshots, knowledge graph,
              entity resolution, time-travel debugging. Designed for an LLM to
              call directly — not a human SDK that you wrap in your own
              tool-use layer.
            </p>
          </div>
        </div>
      </section>

      {/* When NOT to choose Wraith */}
      <section className="px-6 py-16 max-w-4xl mx-auto">
        <h2 className="text-3xl font-bold mb-6">When NOT to use Wraith</h2>
        <p className="text-fd-muted-foreground mb-8">
          Honest about the gaps — credibility matters more than coverage.
        </p>
        <ul className="space-y-4 text-fd-muted-foreground">
          <li className="flex gap-3">
            <span className="text-emerald-400 mt-1">—</span>
            <div>
              <strong className="text-fd-foreground">Visual regression testing.</strong>{' '}
              Wraith has no full layout engine in the hosted API today. Pixel
              diffing, full-page screenshots that match real Chrome, and
              CSS-rendering-correctness tests should use Playwright.
            </div>
          </li>
          <li className="flex gap-3">
            <span className="text-emerald-400 mt-1">—</span>
            <div>
              <strong className="text-fd-foreground">Browser extension support.</strong>{' '}
              No Chromium means no Chrome extensions. If your workflow needs
              MetaMask, Authy, a corporate SSO extension, or any WebExtension —
              use Playwright or Puppeteer.
            </div>
          </li>
          <li className="flex gap-3">
            <span className="text-emerald-400 mt-1">—</span>
            <div>
              <strong className="text-fd-foreground">
                Sites that need Servo-incompatible JS APIs.
              </strong>{' '}
              The QuickJS runtime covers the common cases (Cloudflare
              challenges, light SPA hydration). Heavy WebGL apps, WebRTC,
              Service Workers, and deep Shadow DOM patterns will not work.
            </div>
          </li>
          <li className="flex gap-3">
            <span className="text-emerald-400 mt-1">—</span>
            <div>
              <strong className="text-fd-foreground">
                Cross-browser compatibility testing.
              </strong>{' '}
              Wraith is one engine. Playwright drives Chromium, Firefox, and
              WebKit — that&apos;s the right tool when the test matrix is the
              point.
            </div>
          </li>
          <li className="flex gap-3">
            <span className="text-emerald-400 mt-1">—</span>
            <div>
              <strong className="text-fd-foreground">
                Workflows that already work in 50 lines of Playwright.
              </strong>{' '}
              If you&apos;re not running into Chrome cost, anti-bot, or
              LLM-token problems — there&apos;s no reason to switch.
            </div>
          </li>
        </ul>
      </section>

      {/* CTA */}
      <section className="px-6 py-20 max-w-4xl mx-auto text-center border-t border-fd-border">
        <h2 className="text-3xl font-bold mb-4">Try it</h2>
        <p className="text-fd-muted-foreground max-w-2xl mx-auto mb-8">
          The hosted API is free during beta. The engine is AGPL-3.0 and runs
          on any Linux/macOS/Windows box.
        </p>
        <div className="flex flex-wrap gap-4 justify-center">
          <Link
            href="/signup"
            className="px-6 py-2 rounded-lg bg-emerald-600 text-white font-medium hover:bg-emerald-500 transition-colors"
          >
            Try the hosted API free
          </Link>
          <a
            href="https://github.com/suhteevah/wraith-browser"
            className="px-6 py-2 rounded-lg border border-fd-border hover:bg-fd-accent text-fd-foreground font-medium transition-colors"
          >
            Self-host on GitHub
          </a>
          <Link
            href="/docs/architecture/benchmarks"
            className="px-6 py-2 rounded-lg border border-fd-border hover:bg-fd-accent text-fd-foreground font-medium transition-colors"
          >
            See the benchmarks
          </Link>
        </div>
      </section>

      <footer className="border-t border-fd-border px-6 py-8 text-center text-xs text-fd-muted-foreground">
        <p>
          All third-party trademarks (Playwright, Puppeteer, Browserbase,
          Apify, Crawlee, Bright Data) belong to their respective owners.
          Pricing reflects publicly listed values as of 2026-05; confirm at the
          vendor pricing pages.
        </p>
      </footer>
    </main>
  );
}
