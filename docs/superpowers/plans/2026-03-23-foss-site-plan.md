# OpenClaw FOSS Site Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the open-source documentation, homepage, and playground site for OpenClaw Browser using Geistdocs, deployed to Vercel via CLI.

**Architecture:** Geistdocs (Next.js 16 + Fumadocs) in `foss-site/` at repo root. Content in MDX under `content/docs/`. MCP tool reference auto-generated from a committed tools manifest. Interactive playground uses pre-recorded session replays. Blog via custom app routes. Deployed to Vercel via CLI.

**Tech Stack:** Next.js 16, Fumadocs/Geistdocs, React 19, Tailwind CSS 4, Geist fonts, pnpm, MDX

**Spec:** `docs/superpowers/specs/2026-03-23-foss-site-design.md`

---

## Dependency Graph

```
Task 1 (scaffold) → all other tasks depend on this
Tasks 3, 4, 6, 7, 8 (doc content) → parallelizable, depend only on Task 1
Task 5 (MCP tools) → depends on Task 1
Task 9 (playground) → depends on Task 1, benefits from Task 5 being done first
Task 10 (blog) → depends on Task 1, parallelizable with 3-9
Task 11 (community) → depends on Task 1, parallelizable with 3-9
Task 2 (homepage) → depends on Task 1 and Task 5 (for tool-count.json and terminal demo)
Task 12 (polish) → depends on all previous tasks
```

---

## File Map

### Scaffold (Task 1)
- Create: `foss-site/package.json`
- Create: `foss-site/next.config.ts`
- Create: `foss-site/geistdocs.tsx`
- Create: `foss-site/tailwind.config.ts`
- Create: `foss-site/tsconfig.json`
- Create: `foss-site/postcss.config.mjs`
- Create: `foss-site/source.config.ts` (Fumadocs content source)
- Create: `foss-site/app/layout.tsx`
- Create: `foss-site/app/global.css`
- Create: `foss-site/.env.example`

### Homepage (Task 2)
- Create: `foss-site/app/page.tsx`
- Create: `foss-site/components/install-block.tsx`
- Create: `foss-site/components/terminal-demo.tsx`
- Create: `foss-site/app/not-found.tsx`

### Docs Content — Getting Started (Task 3)
- Create: `foss-site/content/docs/meta.json`
- Create: `foss-site/content/docs/getting-started/meta.json`
- Create: `foss-site/content/docs/getting-started/installation.mdx`
- Create: `foss-site/content/docs/getting-started/first-session.mdx`
- Create: `foss-site/content/docs/getting-started/hello-world-scrape.mdx`

### Docs Content — Architecture (Task 4)
- Create: `foss-site/content/docs/architecture/meta.json`
- Create: `foss-site/content/docs/architecture/engine-overview.mdx`
- Create: `foss-site/content/docs/architecture/snapshot-model.mdx`
- Create: `foss-site/content/docs/architecture/mcp-protocol.mdx`

### MCP Tools Manifest & Reference (Task 5)
- Create: `foss-site/scripts/generate-tool-docs.ts`
- Create: `foss-site/data/tools-manifest.json` (committed, generated from source)
- Create: `foss-site/data/tool-count.json` (derived from manifest, used by homepage and AI prompt)
- Create: `foss-site/content/docs/mcp-tools/meta.json`
- Create: `foss-site/content/docs/mcp-tools/index.mdx`
- Create: 15 category MDX files in `foss-site/content/docs/mcp-tools/`

### Docs Content — Guides (Task 6)
- Create: `foss-site/content/docs/guides/meta.json`
- Create: `foss-site/content/docs/guides/web-scraping.mdx`
- Create: `foss-site/content/docs/guides/form-filling.mdx`
- Create: `foss-site/content/docs/guides/credential-vault.mdx`
- Create: `foss-site/content/docs/guides/knowledge-graph.mdx`
- Create: `foss-site/content/docs/guides/automation-scripts.mdx`

### Docs Content — Knowledge Graph (Task 7)
- Create: `foss-site/content/docs/knowledge-graph/meta.json`
- Create: `foss-site/content/docs/knowledge-graph/page-cache.mdx`
- Create: `foss-site/content/docs/knowledge-graph/embeddings.mdx`
- Create: `foss-site/content/docs/knowledge-graph/entity-resolution.mdx`
- Create: `foss-site/content/docs/knowledge-graph/full-text-search.mdx`

### Docs Content — Self-Hosting & CLI (Task 8)
- Create: `foss-site/content/docs/self-hosting/meta.json`
- Create: `foss-site/content/docs/self-hosting/docker.mdx`
- Create: `foss-site/content/docs/self-hosting/configuration.mdx`
- Create: `foss-site/content/docs/cli-reference/meta.json`
- Create: `foss-site/content/docs/cli-reference/commands.mdx`
- Create: `foss-site/content/docs/cli-reference/transport-modes.mdx`

### Playground (Task 9)
- Create: `foss-site/components/playground-replay.tsx`
- Create: `foss-site/lib/replay-parser.ts`
- Create: `foss-site/content/playground/first-scrape.json`
- Create: `foss-site/content/playground/fill-a-form.json`
- Create: `foss-site/content/playground/knowledge-graph.json`
- Create: `foss-site/content/playground/vault-and-login.json`
- Create: `foss-site/app/playground/page.tsx`

### Blog (Task 10)
- Create: `foss-site/app/blog/page.tsx`
- Create: `foss-site/app/blog/[slug]/page.tsx`
- Create: `foss-site/lib/blog.ts`
- Create: `foss-site/content/blog/introducing-openclaw.mdx`

### Community Page (Task 11)
- Create: `foss-site/app/community/page.tsx`

### Polish & Deploy (Task 12)
- Create: `foss-site/public/og-image.png`
- Create: `foss-site/public/favicon.ico`
- Create: `foss-site/.gitignore`
- Create: `foss-site/vercel.json` (if needed)

---

## Task 1: Geistdocs Scaffold

**Files:**
- Create: `foss-site/package.json`
- Create: `foss-site/next.config.ts`
- Create: `foss-site/geistdocs.tsx`
- Create: `foss-site/tailwind.config.ts`
- Create: `foss-site/tsconfig.json`
- Create: `foss-site/postcss.config.mjs`
- Create: `foss-site/app/layout.tsx`
- Create: `foss-site/app/global.css`

**Reference:** Read the Geistdocs getting started docs at https://preview.geistdocs.com/docs/getting-started before scaffolding. The exact config API may differ from what's shown below. Use `@geistdocs/create` if available, or scaffold manually from docs.

- [ ] **Step 1: Create the foss-site directory and initialize Geistdocs**

```bash
cd J:/openclaw-browser
mkdir foss-site && cd foss-site
pnpm create geistdocs@latest .
```

If `create-geistdocs` is not available, manually scaffold:

```bash
mkdir foss-site && cd foss-site
pnpm init
pnpm add next@latest react@latest react-dom@latest fumadocs-core fumadocs-mdx fumadocs-ui
pnpm add gray-matter next-mdx-remote
pnpm add -D @types/react @types/react-dom typescript tailwindcss@latest postcss autoprefixer
```

- [ ] **Step 2: Configure `geistdocs.tsx`**

Geistdocs uses named exports, not `defineConfig()`. Read `data/tool-count.json` for the dynamic tool count. Verify exact API from https://preview.geistdocs.com/docs/configuration before writing.

```tsx
// foss-site/geistdocs.tsx
import toolCount from './data/tool-count.json';

export const title = 'OpenClaw Browser';

export function Logo() {
  return <span className="font-semibold">OpenClaw Browser</span>;
}

export const nav = [
  { text: 'Docs', href: '/docs' },
  { text: 'Playground', href: '/playground' },
  { text: 'Blog', href: '/blog' },
  { text: 'Community', href: '/community' },
];

export const github = {
  owner: 'suhteevah',
  repo: 'openclaw-browser',
};

export const prompt = `You are the OpenClaw Browser documentation assistant. Help developers use Wraith — a native, AI-agent-first browser with ${toolCount.count} MCP tools. Answer questions about installation, MCP tool usage, the knowledge graph, vault, scripting, and self-hosting. You only know about the open-source version. Do not reference enterprise features, pricing, or managed hosting.`;

export const suggestions = [
  'How do I install OpenClaw Browser?',
  'How do I scrape a website with MCP tools?',
  'How does the knowledge graph work?',
  'How do I store credentials in the vault?',
];
```

**Note:** The exact Geistdocs config API must be verified from docs at https://preview.geistdocs.com/docs/configuration. If the format differs, adapt the named exports accordingly. The key content (title, nav items, AI prompt, suggestions, footer links) stays the same.

- [ ] **Step 3: Create `source.config.ts` (Fumadocs content source)**

Fumadocs requires a content source configuration that defines where MDX files live and how they're processed. Verify exact API from https://preview.geistdocs.com/docs/getting-started.

```typescript
// foss-site/source.config.ts
import { defineDocs } from 'fumadocs-mdx/config';

export const docs = defineDocs({
  dir: 'content/docs',
});
```

- [ ] **Step 3b: Configure `next.config.ts`**

Follow Geistdocs/Fumadocs setup guide for the Next.js config. It typically uses `createMDX` from `fumadocs-mdx/next`. Verify exact API from docs.

- [ ] **Step 3c: Create `postcss.config.mjs`**

```js
// foss-site/postcss.config.mjs
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
```

Tailwind v4 may handle this differently — verify from Tailwind v4 docs. If Tailwind v4 uses `@config` directives in CSS instead, this file may not be needed.

- [ ] **Step 3d: Create `.env.example`**

```
# AI Chat (optional — for Geistdocs Ask AI feature)
# Requires Vercel AI Gateway OIDC or an API key
# OPENAI_API_KEY=
```

- [ ] **Step 4: Create root layout `app/layout.tsx`**

Geistdocs/Fumadocs requires a provider wrapping children for sidebar, search, and AI chat. Verify the exact provider import from https://preview.geistdocs.com/docs/getting-started. Fonts: use the `geist` npm package (`pnpm add geist`) for local font loading instead of `next/font/google`.

```tsx
// foss-site/app/layout.tsx
import { GeistSans } from 'geist/font/sans';
import { GeistMono } from 'geist/font/mono';
import './global.css';
// Import the Geistdocs/Fumadocs provider — verify exact import from docs
// import { GeistdocsProvider } from 'geistdocs'; // or fumadocs-ui equivalent

export const metadata = {
  title: 'OpenClaw Browser — AI-Agent-First Browser Engine',
  description: 'A native browser engine for AI agents. 143+ MCP tools. No Chrome. ~50ms per page.',
  openGraph: {
    title: 'OpenClaw Browser',
    description: 'A native browser engine for AI agents. 143+ MCP tools. No Chrome. ~50ms per page.',
    images: ['/og-image.png'],
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`dark ${GeistSans.variable} ${GeistMono.variable}`}>
      <body>
        {/* Wrap with Geistdocs/Fumadocs provider for sidebar, search, AI chat */}
        {/* <GeistdocsProvider> */}
          {children}
        {/* </GeistdocsProvider> */}
      </body>
    </html>
  );
}
```

**IMPORTANT:** The provider wrapping is critical — without it, docs pages won't get sidebar, search, or AI chat. The exact provider component and import must be verified from the Geistdocs/Fumadocs docs. Add `pnpm add geist` to the scaffold dependencies.

- [ ] **Step 5: Create `app/global.css`**

Tailwind CSS 4 import + any Geistdocs/Fumadocs required styles. Follow docs for exact imports.

```css
@import 'tailwindcss';
/* Geistdocs/Fumadocs styles — verify exact import from docs */
```

- [ ] **Step 6: Verify dev server starts**

```bash
cd foss-site
pnpm dev
```

Expected: Next.js dev server starts on localhost:3000 with Geistdocs chrome (header, sidebar, search). Pages may be empty but the shell should render.

- [ ] **Step 7: Commit**

```bash
cd J:/openclaw-browser
git add foss-site/
git commit -m "feat(foss-site): scaffold Geistdocs project"
```

---

## Task 2: Homepage

**Files:**
- Create: `foss-site/app/page.tsx`
- Create: `foss-site/components/install-block.tsx`
- Create: `foss-site/app/not-found.tsx`
- Reference: `website/app/page.tsx` (enterprise landing page — adapt content, strip enterprise refs)

- [ ] **Step 1: Create `components/install-block.tsx`**

A copy-to-clipboard install command component.

```tsx
// foss-site/components/install-block.tsx
'use client';

import { useState } from 'react';

const methods = [
  { label: 'Cargo', command: 'cargo install openclaw-browser' },
  { label: 'Docker', command: 'docker pull openclaw/browser:latest' },
  { label: 'Binary', command: 'curl -sSL https://get.openclaw.dev | sh' },
];

export function InstallBlock() {
  const [active, setActive] = useState(0);
  const [copied, setCopied] = useState(false);

  const copy = () => {
    navigator.clipboard.writeText(methods[active].command);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="w-full max-w-xl mx-auto">
      <div className="flex gap-1 mb-2">
        {methods.map((m, i) => (
          <button
            key={m.label}
            onClick={() => setActive(i)}
            className={`px-3 py-1 rounded-md text-sm font-medium transition-colors ${
              i === active
                ? 'bg-zinc-700 text-white'
                : 'text-zinc-400 hover:text-zinc-200'
            }`}
          >
            {m.label}
          </button>
        ))}
      </div>
      <div
        className="flex items-center justify-between bg-zinc-900 border border-zinc-800 rounded-lg px-4 py-3 font-mono text-sm cursor-pointer hover:border-zinc-600 transition-colors"
        onClick={copy}
      >
        <code className="text-emerald-400">$ {methods[active].command}</code>
        <span className="text-zinc-500 text-xs ml-4">
          {copied ? 'Copied!' : 'Click to copy'}
        </span>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Create `app/page.tsx`**

Adapt content from `website/app/page.tsx`. Key changes:
- Remove all pricing tiers
- Remove `sales@wraith.dev` CTAs
- Remove enterprise feature gates
- Replace "Get API Key" with install commands
- Keep: hero headline, feature descriptions, competitor comparison (FOSS-only columns)
- Add: `<InstallBlock />`, "How it works" for local/MCP usage, terminal demo placeholder

The homepage is intentionally a custom page that bypasses the Geistdocs docs chrome (sidebar, search). It's a marketing-style landing page. The Geistdocs header/nav should still render via the root layout provider.

Split this into sub-steps for granularity:

**Step 2a: Hero + Install block + Terminal demo**

Add JSON-LD structured data for SoftwareApplication (spec Section 11). Use Next.js `metadata.other` or a `<script>` tag with the static JSON-LD object. Since the JSON-LD content is a static constant defined in source code (not user input), this is safe.

```tsx
// foss-site/app/page.tsx
import { InstallBlock } from '@/components/install-block';
import { TerminalDemo } from '@/components/terminal-demo';

export default function HomePage() {
  return (
    <main className="min-h-screen bg-zinc-950 text-zinc-100">
      {/* Hero */}
      <section className="flex flex-col items-center justify-center px-6 pt-32 pb-20 text-center">
        <h1 className="text-5xl md:text-6xl font-bold tracking-tight max-w-3xl">
          Run 7,000 browser sessions on a single machine
        </h1>
        <p className="mt-6 text-xl text-zinc-400 max-w-2xl">
          A native browser engine for AI agents. No Chrome. No Selenium. ~50ms per page.
          Free and open source.
        </p>
        <div className="mt-10">
          <InstallBlock />
        </div>
        <div className="mt-6 flex gap-4">
          <a
            href="/docs/getting-started/installation"
            className="px-6 py-2 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium transition-colors"
          >
            Read the docs
          </a>
          <a
            href="https://github.com/suhteevah/openclaw-browser"
            className="px-6 py-2 rounded-lg border border-zinc-700 hover:border-zinc-500 text-zinc-300 font-medium transition-colors"
          >
            GitHub
          </a>
        </div>
      </section>

      {/* Terminal demo — auto-playing "first scrape" tutorial */}
      <section className="px-6 py-16 max-w-3xl mx-auto">
        <TerminalDemo />
      </section>

      {/* How it works */}
      <section className="px-6 py-20 max-w-4xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-12">How it works</h2>
        <div className="grid md:grid-cols-3 gap-8">
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">1</div>
            <h3 className="text-lg font-semibold mb-2">Install</h3>
            <p className="text-zinc-400 text-sm">
              One command. 15MB binary. No Chrome, no Selenium, no browser drivers.
            </p>
          </div>
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">2</div>
            <h3 className="text-lg font-semibold mb-2">Connect via MCP</h3>
            <p className="text-zinc-400 text-sm">
              <code className="text-xs bg-zinc-800 px-1.5 py-0.5 rounded">openclaw-browser serve --transport stdio</code>
              {' '}— works with Claude Code, Cursor, and any MCP client.
            </p>
          </div>
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">3</div>
            <h3 className="text-lg font-semibold mb-2">Automate</h3>
            <p className="text-zinc-400 text-sm">
              Navigate, extract, fill forms, build knowledge graphs. 143 tools at your fingertips.
            </p>
          </div>
        </div>
      </section>

      {/* Feature cards */}
      <section className="px-6 py-20 max-w-5xl mx-auto">
        <div className="grid md:grid-cols-3 gap-6">
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">Native Engine</h3>
            <p className="text-zinc-400 text-sm">
              15MB binary. Servo-derived rendering. ~50ms per page. Run thousands of
              concurrent sessions without Chrome overhead.
            </p>
          </div>
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">143 MCP Tools</h3>
            <p className="text-zinc-400 text-sm">
              Navigation, extraction, vault, knowledge graph, entity resolution,
              time-travel debugging, automation scripts, and more.
            </p>
          </div>
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">Knowledge Graph</h3>
            <p className="text-zinc-400 text-sm">
              Every page cached and searchable. Vector embeddings, entity linking,
              full-text search via Tantivy. Your browsing becomes a queryable database.
            </p>
          </div>
        </div>
      </section>

      {/* Competitor comparison — FOSS columns only */}
      <section className="px-6 py-20 max-w-5xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-12">How Wraith compares</h2>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-zinc-800 text-left">
                <th className="pb-3 pr-4 text-zinc-400 font-medium">Feature</th>
                <th className="pb-3 pr-4 font-medium text-emerald-400">Wraith</th>
                <th className="pb-3 pr-4 text-zinc-400 font-medium">Browserbase</th>
                <th className="pb-3 pr-4 text-zinc-400 font-medium">Browserless</th>
                <th className="pb-3 text-zinc-400 font-medium">Apify</th>
              </tr>
            </thead>
            <tbody className="text-zinc-400">
              <tr className="border-b border-zinc-800/50">
                <td className="py-3 pr-4">Engine</td>
                <td className="py-3 pr-4 text-zinc-100">Native (Servo)</td>
                <td className="py-3 pr-4">Chrome</td>
                <td className="py-3 pr-4">Chrome</td>
                <td className="py-3">Chrome</td>
              </tr>
              <tr className="border-b border-zinc-800/50">
                <td className="py-3 pr-4">Binary size</td>
                <td className="py-3 pr-4 text-zinc-100">~15 MB</td>
                <td className="py-3 pr-4">Cloud only</td>
                <td className="py-3 pr-4">~300 MB</td>
                <td className="py-3">Cloud only</td>
              </tr>
              <tr className="border-b border-zinc-800/50">
                <td className="py-3 pr-4">Self-hosted</td>
                <td className="py-3 pr-4 text-emerald-400">Yes (AGPL-3.0)</td>
                <td className="py-3 pr-4">No</td>
                <td className="py-3 pr-4">Paid</td>
                <td className="py-3">No</td>
              </tr>
              <tr className="border-b border-zinc-800/50">
                <td className="py-3 pr-4">MCP tools</td>
                <td className="py-3 pr-4 text-zinc-100">143</td>
                <td className="py-3 pr-4">Limited</td>
                <td className="py-3 pr-4">None</td>
                <td className="py-3">None</td>
              </tr>
              <tr className="border-b border-zinc-800/50">
                <td className="py-3 pr-4">Credential vault</td>
                <td className="py-3 pr-4 text-emerald-400">Built-in (AES-256)</td>
                <td className="py-3 pr-4">No</td>
                <td className="py-3 pr-4">No</td>
                <td className="py-3">No</td>
              </tr>
              <tr>
                <td className="py-3 pr-4">Knowledge graph</td>
                <td className="py-3 pr-4 text-emerald-400">Built-in</td>
                <td className="py-3 pr-4">No</td>
                <td className="py-3 pr-4">No</td>
                <td className="py-3">No</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      {/* Footer */}
      <footer className="border-t border-zinc-800 px-6 py-12 text-center text-sm text-zinc-500">
        <p>OpenClaw Browser — AGPL-3.0 — Free and open source, forever.</p>
        <p className="mt-2">
          Need managed hosting &amp; team features?{' '}
          <a href="https://wraith.dev/enterprise" className="text-zinc-400 hover:text-zinc-200 underline">
            Learn about Wraith Enterprise
          </a>
        </p>
      </footer>
    </main>
  );
}
```

- [ ] **Step 3: Create `components/terminal-demo.tsx`**

A wrapper around `<PlaygroundReplay />` (from Task 9) that auto-plays the "first scrape" tutorial on the homepage. Since the playground component may not exist yet, create a placeholder that renders a static terminal mockup initially, then swap in the real `<PlaygroundReplay autoPlay speed={1} />` after Task 9.

```tsx
// foss-site/components/terminal-demo.tsx
'use client';

// Placeholder — replace with PlaygroundReplay import after Task 9
export function TerminalDemo() {
  return (
    <div className="w-full max-w-3xl mx-auto bg-zinc-950 border border-zinc-800 rounded-xl p-6 font-mono text-sm">
      <div className="flex gap-2 mb-4">
        <div className="w-3 h-3 rounded-full bg-red-500/50" />
        <div className="w-3 h-3 rounded-full bg-yellow-500/50" />
        <div className="w-3 h-3 rounded-full bg-green-500/50" />
      </div>
      <div className="space-y-2 text-zinc-400">
        <p><span className="text-emerald-400">$</span> browse_navigate url="https://example.com"</p>
        <p className="text-zinc-500">{"{"} "url": "https://example.com", "title": "Example Domain" {"}"}</p>
        <p><span className="text-emerald-400">$</span> extract_markdown</p>
        <p className="text-zinc-500"># Example Domain</p>
        <p className="text-zinc-500">This domain is for use in illustrative examples...</p>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Create `app/not-found.tsx`**

```tsx
// foss-site/app/not-found.tsx
export default function NotFound() {
  return (
    <div className="flex flex-col items-center justify-center min-h-screen bg-zinc-950 text-zinc-100 px-6">
      <h1 className="text-6xl font-bold mb-4">404</h1>
      <p className="text-xl text-zinc-400 mb-8">This page doesn't exist.</p>
      <div className="flex gap-4">
        <a href="/" className="px-6 py-2 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium">
          Go home
        </a>
        <a href="/docs" className="px-6 py-2 rounded-lg border border-zinc-700 hover:border-zinc-500 text-zinc-300 font-medium">
          Read the docs
        </a>
      </div>
      <p className="mt-8 text-sm text-zinc-600">
        Looking for enterprise features?{' '}
        <a href="https://wraith.dev/enterprise" className="text-zinc-500 hover:text-zinc-300 underline">
          Visit Wraith Enterprise
        </a>
      </p>
    </div>
  );
}
```

- [ ] **Step 4: Verify homepage renders**

```bash
cd foss-site && pnpm dev
# Open http://localhost:3000 — hero, install block, features, comparison table should render
```

- [ ] **Step 5: Commit**

```bash
cd J:/openclaw-browser
git add foss-site/app/page.tsx foss-site/components/install-block.tsx foss-site/app/not-found.tsx
git commit -m "feat(foss-site): add homepage with install block and comparison table"
```

---

## Task 3: Docs Content — Getting Started

**Files:**
- Create: `foss-site/content/docs/meta.json`
- Create: `foss-site/content/docs/getting-started/meta.json`
- Create: `foss-site/content/docs/getting-started/installation.mdx`
- Create: `foss-site/content/docs/getting-started/first-session.mdx`
- Create: `foss-site/content/docs/getting-started/hello-world-scrape.mdx`

- [ ] **Step 1: Create top-level sidebar ordering**

```json
// foss-site/content/docs/meta.json
{
  "title": "Documentation",
  "pages": [
    "getting-started",
    "mcp-tools",
    "guides",
    "architecture",
    "knowledge-graph",
    "self-hosting",
    "cli-reference"
  ]
}
```

- [ ] **Step 2: Create getting-started section ordering**

```json
// foss-site/content/docs/getting-started/meta.json
{
  "title": "Getting Started",
  "pages": ["installation", "first-session", "hello-world-scrape"]
}
```

- [ ] **Step 3: Write `installation.mdx`**

Cover: cargo install, Docker pull, binary download, verify with `openclaw-browser --version`. Reference the existing `docs/api/quickstart.md` for accuracy but rewrite for FOSS context (no API keys, no enterprise endpoints).

```mdx
---
title: Installation
description: Install OpenClaw Browser in under a minute
---

# Installation

## Cargo (recommended)

\`\`\`bash
cargo install openclaw-browser
\`\`\`

Requires Rust 1.78+. The binary is ~15MB.

## Docker

\`\`\`bash
docker pull openclaw/browser:latest
docker run --rm openclaw/browser:latest --version
\`\`\`

## Binary download

Download pre-built binaries from the [GitHub releases page](https://github.com/suhteevah/openclaw-browser/releases).

| Platform | Architecture | Download |
|----------|-------------|----------|
| Linux | x86_64 | `openclaw-browser-linux-amd64` |
| macOS | Apple Silicon | `openclaw-browser-darwin-arm64` |
| macOS | Intel | `openclaw-browser-darwin-amd64` |
| Windows | x86_64 | `openclaw-browser-windows-amd64.exe` |

## Verify

\`\`\`bash
openclaw-browser --version
# OpenClaw Browser v0.1.0
\`\`\`

## Next steps

[Start your first MCP session →](/docs/getting-started/first-session)
```

- [ ] **Step 4: Write `first-session.mdx`**

Cover: starting the MCP server (`openclaw-browser serve --transport stdio`), connecting from Claude Code, connecting from Cursor. Reference `docs/api/mcp-tools-reference.md` for architecture diagram.

- [ ] **Step 5: Write `hello-world-scrape.mdx`**

Cover: navigate to example.com, take a snapshot, extract markdown. Show the MCP tool calls and responses. This is the "hello world" that gets people hooked.

- [ ] **Step 6: Verify docs render in sidebar**

```bash
cd foss-site && pnpm dev
# Navigate to /docs — sidebar should show Getting Started with 3 sub-pages
```

- [ ] **Step 7: Commit**

```bash
cd J:/openclaw-browser
git add foss-site/content/
git commit -m "feat(foss-site): add getting started docs"
```

---

## Task 4: Docs Content — Architecture

**Files:**
- Create: `foss-site/content/docs/architecture/meta.json`
- Create: `foss-site/content/docs/architecture/engine-overview.mdx`
- Create: `foss-site/content/docs/architecture/snapshot-model.mdx`
- Create: `foss-site/content/docs/architecture/mcp-protocol.mdx`
- Reference: `docs/api/mcp-tools-reference.md` lines 43-71 (architecture diagram)

- [ ] **Step 1: Create section ordering**

```json
{
  "title": "Architecture",
  "pages": ["engine-overview", "snapshot-model", "mcp-protocol"]
}
```

- [ ] **Step 2: Write `engine-overview.mdx`**

Content: The three engine backends (SevroEngine, NativeEngine, CdpEngine), how they implement the BrowserEngine trait, the "no Chrome" story, binary size comparison, concurrency model. Adapt the architecture diagram from `docs/api/mcp-tools-reference.md`.

- [ ] **Step 3: Write `snapshot-model.mdx`**

Content: How `@ref` IDs work, DOM snapshots, how agents interact with page elements. This is key for users to understand how `browse_click(@ref=42)` works.

- [ ] **Step 4: Write `mcp-protocol.mdx`**

Content: JSON-RPC over stdio, tool registration, the dispatch model, transport modes (stdio, HTTP). Reference the MCP spec.

- [ ] **Step 5: Commit**

```bash
git add foss-site/content/docs/architecture/
git commit -m "feat(foss-site): add architecture docs"
```

---

## Task 5: MCP Tools Manifest & Reference

**Files:**
- Create: `foss-site/scripts/generate-tool-docs.ts`
- Create: `foss-site/data/tools-manifest.json`
- Create: `foss-site/content/docs/mcp-tools/meta.json`
- Create: `foss-site/content/docs/mcp-tools/index.mdx`
- Create: 15 category MDX files
- Reference: `docs/api/mcp-tools-reference.md` (existing 24-category reference — primary content source)
- Reference: `crates/mcp-server/src/server.rs` (143 `make_tool()` calls — authoritative tool list)

This is the largest task. The strategy is:
1. Generate a `tools-manifest.json` from the Rust source (one-time, committed to repo)
2. Write a build script that reads the manifest and generates per-category MDX
3. Hand-write category intros that wrap the generated content

- [ ] **Step 1: Generate tools manifest from source**

Write `scripts/generate-tool-docs.ts` — a Node.js script that parses `make_tool()` calls from `crates/mcp-server/src/server.rs`. Each call has the pattern:

```rust
make_tool("tool_name", "Tool description", json!({ ... schema ... }))
```

The script extracts: tool name, description, parameter schema. Outputs `data/tools-manifest.json`:

```json
{
  "generated_at": "2026-03-23T00:00:00Z",
  "tool_count": 143,
  "tools": [
    {
      "name": "browse_navigate",
      "description": "Navigate to a URL",
      "category": "navigation",
      "parameters": { ... }
    }
  ]
}
```

Category assignment is based on prefix mapping from the spec (§6, 15 categories).

- [ ] **Step 2: Run the script to generate the manifest and tool-count.json**

```bash
cd foss-site
npx tsx scripts/generate-tool-docs.ts
# Output: data/tools-manifest.json with 143 tools
# Output: data/tool-count.json with { "count": 143 }
```

The script must also emit `data/tool-count.json`:
```json
{ "count": 143 }
```

This file is read by `geistdocs.tsx` for the AI prompt and by the homepage for display. Both files are committed to the repo and regenerated when tools change.

Verify the count: `cat data/tools-manifest.json | python3 -c "import sys,json; print(json.load(sys.stdin)['tool_count'])"`

- [ ] **Step 3: Create MCP tools section ordering**

```json
// foss-site/content/docs/mcp-tools/meta.json
{
  "title": "MCP Tools Reference",
  "pages": [
    "index",
    "navigation",
    "interaction",
    "extraction",
    "dom",
    "cookies",
    "cache",
    "identity",
    "session",
    "search",
    "entities",
    "automation",
    "time-travel",
    "plugins",
    "telemetry",
    "advanced"
  ]
}
```

- [ ] **Step 4: Write `index.mdx` (overview)**

Overview page with: total tool count (from manifest), category listing with descriptions and tool counts, link to each category page. Architecture diagram from existing reference doc.

- [ ] **Step 5: Write 15 category MDX files**

For each category, adapt content from the existing `docs/api/mcp-tools-reference.md` which already has detailed tool documentation organized into 24 sections. Consolidate into our 15 categories. Each file contains:
- Category intro (hand-written, 2-3 sentences)
- Tool table (name, description, key parameters)
- Example usage for the 2-3 most important tools in the category

The existing reference doc at `docs/api/mcp-tools-reference.md` is the primary content source. Do not write tool descriptions from scratch — adapt from the existing reference.

**White-room notes for specific categories:**
- **Automation category (automation.mdx):** Include a note clarifying that `swarm_*` tools are local parallelism tools running on the user's machine. They are FOSS. The enterprise "swarm orchestration" is a separate multi-tenant coordination layer — do not reference it.
- **Automation category — playbooks:** Document `swarm_run_playbook`, `swarm_list_playbooks` as "automation templates." The built-in playbooks (greenhouse-apply, ashby-apply, lever-apply) should be described as job application automation scripts, NOT as "ATS integrations" (which is enterprise language for managed API connectors).

Split this step into 3 sub-steps for granularity:
- **Step 5a:** Write navigation, interaction, extraction, dom, cookies MDX (5 files)
- **Step 5b:** Write cache, identity, session, search, entities MDX (5 files)
- **Step 5c:** Write automation, time-travel, plugins, telemetry, advanced MDX (5 files)

- [ ] **Step 6: Commit manifest and docs**

```bash
git add foss-site/scripts/ foss-site/data/ foss-site/content/docs/mcp-tools/
git commit -m "feat(foss-site): add MCP tools reference with 143 tools across 15 categories"
```

---

## Task 6: Docs Content — Guides

**Files:**
- Create: `foss-site/content/docs/guides/meta.json`
- Create: 5 guide MDX files
- Reference: Existing tool documentation for accurate tool names and parameters

- [ ] **Step 1: Create section ordering**

```json
{
  "title": "Guides",
  "pages": ["web-scraping", "form-filling", "credential-vault", "knowledge-graph", "automation-scripts"]
}
```

- [ ] **Step 2: Write `web-scraping.mdx`**

Step-by-step guide: navigate to a URL, extract content, handle pagination, cache results. Use real MCP tool call examples.

- [ ] **Step 3: Write `form-filling.mdx`**

Guide: snapshot a page, identify form fields by @ref, fill them, submit. Cover select dropdowns, file uploads.

- [ ] **Step 4: Write `credential-vault.mdx`**

Guide: store credentials, use them for login, TOTP generation, vault audit. Cover AES-256-GCM encryption.

- [ ] **Step 5: Write `knowledge-graph.mdx`**

Guide: scrape multiple pages, search the cache, query entities, visualize the graph. This is the "killer feature" guide.

- [ ] **Step 6: Write `automation-scripts.mdx`**

Guide: Rhai scripting, workflow recording/replay, DAG orchestration, playbooks.

- [ ] **Step 7: Commit**

```bash
git add foss-site/content/docs/guides/
git commit -m "feat(foss-site): add 5 practical guides"
```

---

## Task 7: Docs Content — Knowledge Graph

**Files:**
- Create: `foss-site/content/docs/knowledge-graph/meta.json`
- Create: 4 MDX files

- [ ] **Step 1: Create section ordering**

```json
{
  "title": "Knowledge Graph",
  "pages": ["page-cache", "embeddings", "entity-resolution", "full-text-search"]
}
```

- [ ] **Step 2: Write `page-cache.mdx`**

Cover: SQLite-backed cache, `cache_get`, `cache_search`, `cache_stats`, `cache_pin`, raw HTML storage, domain profiles.

- [ ] **Step 3: Write `embeddings.mdx`**

Cover: vector embeddings, `embedding_upsert`, `embedding_search`, similarity search, how embeddings are computed.

- [ ] **Step 4: Write `entity-resolution.mdx`**

Cover: `entity_add`, `entity_query`, `entity_relate`, `entity_merge`, `entity_visualize`. How entities are linked across pages.

- [ ] **Step 5: Write `full-text-search.mdx`**

Cover: Tantivy full-text index, how it integrates with the cache, search queries.

- [ ] **Step 6: Commit**

```bash
git add foss-site/content/docs/knowledge-graph/
git commit -m "feat(foss-site): add knowledge graph docs"
```

---

## Task 8: Docs Content — Self-Hosting & CLI

**Files:**
- Create: `foss-site/content/docs/self-hosting/meta.json`
- Create: `foss-site/content/docs/self-hosting/docker.mdx`
- Create: `foss-site/content/docs/self-hosting/configuration.mdx`
- Create: `foss-site/content/docs/cli-reference/meta.json`
- Create: `foss-site/content/docs/cli-reference/commands.mdx`
- Create: `foss-site/content/docs/cli-reference/transport-modes.mdx`
- Reference: `deploy/docker-compose.yml`, `deploy/Dockerfile`, `deploy/README.md` for Docker content

- [ ] **Step 1: Create section orderings**

```json
// self-hosting/meta.json
{ "title": "Self-Hosting", "pages": ["docker", "configuration"] }

// cli-reference/meta.json
{ "title": "CLI Reference", "pages": ["commands", "transport-modes"] }
```

- [ ] **Step 2: Write `docker.mdx`**

Cover: Docker run (single binary), docker-compose (with Redis for optional features), environment variables, health check verification. Adapt from `deploy/README.md` but exclude enterprise API server references.

- [ ] **Step 3: Write `configuration.mdx`**

Cover: all environment variables for the FOSS binary, feature flags, engine selection, logging levels.

- [ ] **Step 4: Write `commands.mdx`**

Cover: `openclaw-browser serve`, `openclaw-browser --version`, any other CLI subcommands. Parse from `crates/cli/src/` for accuracy.

- [ ] **Step 5: Write `transport-modes.mdx`**

Cover: stdio (default, for MCP clients), HTTP (for remote access), transport selection, port configuration.

- [ ] **Step 6: Commit**

```bash
git add foss-site/content/docs/self-hosting/ foss-site/content/docs/cli-reference/
git commit -m "feat(foss-site): add self-hosting and CLI reference docs"
```

---

## Task 9: Playground

**Files:**
- Create: `foss-site/components/playground-replay.tsx`
- Create: `foss-site/lib/replay-parser.ts`
- Create: 4 recording JSON files in `foss-site/content/playground/`
- Create: `foss-site/app/playground/page.tsx`

- [ ] **Step 1: Define the recording schema in `lib/replay-parser.ts`**

```typescript
// foss-site/lib/replay-parser.ts
export interface SessionStep {
  type: 'command' | 'output' | 'annotation';
  content: string;
  delay_ms: number;
  annotation?: string;
}

export interface SessionRecording {
  title: string;
  description: string;
  steps: SessionStep[];
}

export function parseRecording(json: unknown): SessionRecording {
  const data = json as SessionRecording;
  if (!data.title || !data.steps || !Array.isArray(data.steps)) {
    throw new Error('Invalid recording format');
  }
  return data;
}
```

- [ ] **Step 2: Build `components/playground-replay.tsx`**

A `'use client'` component that:
- Renders steps one at a time in a terminal-style container
- "Next" button advances to the next step
- "Auto-play" mode advances on `delay_ms` timing
- Commands render with JSON syntax highlighting (use a simple regex highlighter or `prism-react-renderer`)
- Outputs render as formatted text
- Annotations render as callout boxes above/below the terminal

Key UI:
- Dark terminal container (`bg-zinc-950 border border-zinc-800 rounded-xl`)
- Green prompt (`$`) for commands
- White text for output
- Blue callout boxes for annotations
- Progress indicator (step 3 of 8)
- Speed control (0.5x, 1x, 2x)

- [ ] **Step 3: Create 4 recording JSON files**

Capture real MCP sessions by running Wraith locally and recording the tool calls and responses. If the binary is not available on the dev machine, write realistic mock recordings based on the tool documentation in `docs/api/mcp-tools-reference.md`.

Each recording should be a JSON file matching the `SessionRecording` interface.

Files:
- `content/playground/first-scrape.json` — 3 steps: navigate, snapshot, extract
- `content/playground/fill-a-form.json` — 5 steps: navigate, snapshot, fill x2, submit
- `content/playground/knowledge-graph.json` — 8 steps: scrape 3 pages, cache search, entity query, visualize
- `content/playground/vault-and-login.json` — 6 steps: vault store, navigate, login, verify

- [ ] **Step 4: Create `app/playground/page.tsx`**

Hub page listing all 4 tutorials with title, description, and step count. Click to expand the `<PlaygroundReplay />` component inline.

**Important:** The JSON recording files live in `content/playground/` (not `public/`), so they must be imported at build time. Use static `import` for each JSON file:

```tsx
import firstScrape from '@/content/playground/first-scrape.json';
import fillForm from '@/content/playground/fill-a-form.json';
import knowledgeGraph from '@/content/playground/knowledge-graph.json';
import vaultLogin from '@/content/playground/vault-and-login.json';

const tutorials = [firstScrape, fillForm, knowledgeGraph, vaultLogin];
```

- [ ] **Step 5: Verify playground renders and auto-play works**

```bash
cd foss-site && pnpm dev
# Navigate to /playground — 4 tutorials listed, click one, steps advance correctly
```

- [ ] **Step 6: Commit**

```bash
git add foss-site/components/playground-replay.tsx foss-site/lib/replay-parser.ts foss-site/content/playground/ foss-site/app/playground/
git commit -m "feat(foss-site): add interactive playground with 4 tutorials"
```

---

## Task 10: Blog

**Files:**
- Create: `foss-site/lib/blog.ts`
- Create: `foss-site/app/blog/page.tsx`
- Create: `foss-site/app/blog/[slug]/page.tsx`
- Create: `foss-site/content/blog/introducing-openclaw.mdx`

Blog is a custom app route, not part of the Fumadocs docs tree.

- [ ] **Step 1: Create `lib/blog.ts`**

Utility to load blog MDX files from `content/blog/`. Uses `next-mdx-remote` or Fumadocs' content loading. Each MDX file has frontmatter: `title`, `date`, `description`, `author`.

```typescript
// foss-site/lib/blog.ts
import fs from 'fs';
import path from 'path';
import matter from 'gray-matter';

const BLOG_DIR = path.join(process.cwd(), 'content/blog');

export interface BlogPost {
  slug: string;
  title: string;
  date: string;
  description: string;
  author: string;
  content: string;
}

export function getAllPosts(): BlogPost[] {
  const files = fs.readdirSync(BLOG_DIR).filter(f => f.endsWith('.mdx'));
  return files
    .map(file => {
      const raw = fs.readFileSync(path.join(BLOG_DIR, file), 'utf-8');
      const { data, content } = matter(raw);
      return {
        slug: file.replace('.mdx', ''),
        title: data.title,
        date: data.date,
        description: data.description,
        author: data.author || 'OpenClaw Team',
        content,
      };
    })
    .sort((a, b) => new Date(b.date).getTime() - new Date(a.date).getTime());
}

export function getPost(slug: string): BlogPost | undefined {
  return getAllPosts().find(p => p.slug === slug);
}
```

- [ ] **Step 2: Create blog listing page `app/blog/page.tsx`**

Lists all posts sorted by date. Each entry shows title, date, description, link to full post.

- [ ] **Step 3: Create blog post page `app/blog/[slug]/page.tsx`**

Renders a single blog post. Uses `next-mdx-remote` (or Fumadocs MDX rendering) for the content. Must include `generateStaticParams` for static builds:

```tsx
import { getAllPosts, getPost } from '@/lib/blog';

export function generateStaticParams() {
  return getAllPosts().map(post => ({ slug: post.slug }));
}
```

- [ ] **Step 4: Write `introducing-openclaw.mdx`**

```mdx
---
title: Introducing OpenClaw Browser
date: "2026-03-23"
description: "An open-source, AI-agent-first browser engine with 143 MCP tools."
author: Matt Gates
---

# Introducing OpenClaw Browser

Today we're open-sourcing OpenClaw Browser (Wraith) — a native browser engine built from scratch for AI agents.

## Why we built it

[Content: Chrome overhead problem, selector fragility, credential management gap]

## What makes it different

[Content: Native Servo-derived engine, 143 MCP tools, knowledge graph, vault]

## Get started

[Content: Install command, link to docs, link to playground]

## What's next

[Content: Roadmap highlights, community links, how to contribute]
```

- [ ] **Step 5: Commit**

```bash
git add foss-site/lib/blog.ts foss-site/app/blog/ foss-site/content/blog/
git commit -m "feat(foss-site): add blog with launch announcement"
```

---

## Task 11: Community Page

**Files:**
- Create: `foss-site/app/community/page.tsx`

- [ ] **Step 1: Create community page**

```tsx
// foss-site/app/community/page.tsx
export default function CommunityPage() {
  return (
    <main className="min-h-screen bg-zinc-950 text-zinc-100 px-6 py-20">
      <div className="max-w-3xl mx-auto">
        <h1 className="text-4xl font-bold mb-4">Community</h1>
        <p className="text-zinc-400 text-lg mb-12">
          Join the OpenClaw community. Ask questions, share what you're building, and help shape the future of AI-native browsing.
        </p>

        <div className="space-y-6">
          <a href="#" className="block bg-zinc-900 border border-zinc-800 rounded-xl p-6 hover:border-zinc-600 transition-colors">
            <h2 className="text-xl font-semibold mb-2">Discord</h2>
            <p className="text-zinc-400">Chat with the community and get help in real-time.</p>
            <span className="text-emerald-400 text-sm mt-2 inline-block">Join Discord →</span>
          </a>

          <a href="#" className="block bg-zinc-900 border border-zinc-800 rounded-xl p-6 hover:border-zinc-600 transition-colors opacity-60">
            <h2 className="text-xl font-semibold mb-2">Matrix</h2>
            <p className="text-zinc-400">Bridged to Discord. Coming soon.</p>
          </a>

          <a href="https://github.com/suhteevah/openclaw-browser" className="block bg-zinc-900 border border-zinc-800 rounded-xl p-6 hover:border-zinc-600 transition-colors">
            <h2 className="text-xl font-semibold mb-2">GitHub</h2>
            <p className="text-zinc-400">Star the repo, file issues, and submit pull requests.</p>
            <span className="text-emerald-400 text-sm mt-2 inline-block">View on GitHub →</span>
          </a>

          <a href="https://github.com/suhteevah/openclaw-browser/blob/main/CONTRIBUTING.md" className="block bg-zinc-900 border border-zinc-800 rounded-xl p-6 hover:border-zinc-600 transition-colors">
            <h2 className="text-xl font-semibold mb-2">Contributing</h2>
            <p className="text-zinc-400">Read the contributing guide and start building.</p>
            <span className="text-emerald-400 text-sm mt-2 inline-block">Contributing guide →</span>
          </a>

          <a href="https://github.com/suhteevah/openclaw-browser/blob/main/CODE_OF_CONDUCT.md" className="block bg-zinc-900 border border-zinc-800 rounded-xl p-6 hover:border-zinc-600 transition-colors">
            <h2 className="text-xl font-semibold mb-2">Code of Conduct</h2>
            <p className="text-zinc-400">Our commitment to a welcoming, inclusive community.</p>
            <span className="text-emerald-400 text-sm mt-2 inline-block">Read the code of conduct →</span>
          </a>
        </div>

        <div className="mt-16">
          <h2 className="text-2xl font-bold mb-4">Built with Wraith</h2>
          <p className="text-zinc-500">
            Showcase coming soon. Built something with OpenClaw Browser?{' '}
            <a href="https://github.com/suhteevah/openclaw-browser/discussions" className="text-zinc-400 hover:text-zinc-200 underline">
              Share it on GitHub Discussions
            </a>.
          </p>
        </div>
      </div>
    </main>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add foss-site/app/community/
git commit -m "feat(foss-site): add community page"
```

---

## Task 12: Polish & Deploy

**Files:**
- Create: `foss-site/public/og-image.png` (generate or design)
- Create: `foss-site/public/favicon.ico`
- Create: `foss-site/.gitignore`

- [ ] **Step 1: Create `.gitignore`**

```
node_modules/
.next/
.env*.local
out/
```

- [ ] **Step 2: Generate OG image**

Create a simple OG image (1200x630) with: "OpenClaw Browser" title, "AI-Agent-First Browser Engine" subtitle, dark background. Can use any image tool or generate with Satori at build time later.

- [ ] **Step 3: Add favicon**

Use the OpenClaw logo or a simple placeholder favicon.

- [ ] **Step 4: Full site smoke test**

```bash
cd foss-site && pnpm dev
```

Verify:
- [ ] Homepage renders with hero, install block, features, comparison
- [ ] Docs sidebar shows all 7 sections with correct ordering
- [ ] Getting Started pages render with correct content
- [ ] MCP Tools Reference shows all 15 categories
- [ ] Playground page lists 4 tutorials, replay works
- [ ] Blog listing shows the launch post, post page renders
- [ ] Community page renders with all links
- [ ] 404 page renders for invalid routes
- [ ] AI chat (Ask AI) responds to queries about Wraith
- [ ] `llms.txt` endpoint serves all docs as plain text
- [ ] No enterprise references anywhere on the site

- [ ] **Step 5: Build for production**

```bash
cd foss-site && pnpm build
```

Fix any build errors.

- [ ] **Step 6: Deploy to Vercel**

```bash
cd foss-site
vercel link  # connect to a new Vercel project
vercel deploy  # preview deployment
# Verify preview URL works
vercel --prod  # production deployment
```

- [ ] **Step 7: Final commit**

```bash
cd J:/openclaw-browser
git add foss-site/
git commit -m "feat(foss-site): polish and deploy to Vercel"
```

---

## White-Room Verification Checklist

Run this after all tasks are complete:

- [ ] `grep -r "api-server" foss-site/` returns nothing
- [ ] `grep -r "dashboard" foss-site/` returns nothing (except maybe a generic word in prose)
- [ ] `grep -r "enterprise" foss-site/` returns only the one outbound link in footer and 404
- [ ] `grep -r "billing\|stripe\|invoice" foss-site/` returns nothing
- [ ] `grep -r "SSO\|SAML\|OIDC" foss-site/` returns nothing
- [ ] `grep -r "teams\|RBAC\|organization" foss-site/` returns nothing (except community/contributing context)
- [ ] `grep -r "sales@\|pricing" foss-site/` returns nothing
- [ ] `grep -r "fly\.io\|fly\.toml" foss-site/` returns nothing
- [ ] `grep -r "JWT\|refresh.token" foss-site/` returns nothing
- [ ] `grep -ri "MPL-2.0" foss-site/` returns nothing (license is AGPL-3.0)
