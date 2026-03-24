import Link from 'next/link';

export default function HomePage() {
  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      {/* Hero */}
      <section className="flex flex-col items-center justify-center px-6 pt-32 pb-20 text-center">
        <h1 className="text-5xl md:text-6xl font-bold tracking-tight max-w-3xl">
          Run 7,000 browser sessions on a single machine
        </h1>
        <p className="mt-6 text-xl text-fd-muted-foreground max-w-2xl">
          A native browser engine for AI agents. No Chrome. No Selenium. ~50ms
          per page. Free and open source.
        </p>
        <div className="mt-10 w-full max-w-xl mx-auto">
          <div className="flex items-center justify-between bg-fd-card border border-fd-border rounded-lg px-4 py-3 font-mono text-sm">
            <code className="text-emerald-400">
              $ git clone https://github.com/suhteevah/wraith-browser && cd wraith-browser && cargo build --release
            </code>
          </div>
        </div>
        <div className="mt-6 flex gap-4">
          <Link
            href="/docs/getting-started/installation"
            className="px-6 py-2 rounded-lg bg-fd-primary text-fd-primary-foreground font-medium hover:opacity-90 transition-opacity"
          >
            Read the docs
          </Link>
          <Link
            href="/playground"
            className="px-6 py-2 rounded-lg border border-emerald-500 text-emerald-400 hover:bg-emerald-500/10 font-medium transition-colors"
          >
            Try the playground
          </Link>
          <a
            href="https://github.com/suhteevah/wraith-browser"
            className="px-6 py-2 rounded-lg border border-fd-border hover:bg-fd-accent text-fd-foreground font-medium transition-colors"
          >
            GitHub
          </a>
        </div>
      </section>

      {/* How it works */}
      <section className="px-6 py-20 max-w-4xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-12">How it works</h2>
        <div className="grid md:grid-cols-3 gap-8">
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">1</div>
            <h3 className="text-lg font-semibold mb-2">Install</h3>
            <p className="text-fd-muted-foreground text-sm">
              One command. 15MB binary. No Chrome, no Selenium, no browser
              drivers.
            </p>
          </div>
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">2</div>
            <h3 className="text-lg font-semibold mb-2">Connect via MCP</h3>
            <p className="text-fd-muted-foreground text-sm">
              <code className="text-xs bg-fd-card px-1.5 py-0.5 rounded">
                wraith-browser serve --transport stdio
              </code>{' '}
              — works with Claude Code, Cursor, and any MCP client.
            </p>
          </div>
          <div className="text-center">
            <div className="text-4xl font-bold text-emerald-500 mb-3">3</div>
            <h3 className="text-lg font-semibold mb-2">Automate</h3>
            <p className="text-fd-muted-foreground text-sm">
              Navigate, extract, fill forms, build knowledge graphs. 130 tools
              at your fingertips.
            </p>
          </div>
        </div>
      </section>

      {/* Deep dive */}
      <section className="px-6 pb-12 max-w-4xl mx-auto text-center">
        <p className="text-fd-muted-foreground text-sm">
          Want to understand the internals?{' '}
          <Link href="/docs/architecture/engine-overview" className="text-emerald-400 hover:underline">
            Read the engine architecture deep dive
          </Link>{' '}
          or explore the{' '}
          <Link href="/docs/architecture/snapshot-model" className="text-emerald-400 hover:underline">
            snapshot model
          </Link>.
        </p>
      </section>

      {/* Feature cards */}
      <section className="px-6 py-20 max-w-5xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-8">Why Wraith?</h2>
        <div className="grid md:grid-cols-3 gap-6">
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">Native Engine</h3>
            <p className="text-fd-muted-foreground text-sm">
              15MB binary. html5ever-based parsing. ~50ms per page. Run
              thousands of concurrent sessions without Chrome overhead.
            </p>
          </div>
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">130 MCP Tools</h3>
            <p className="text-fd-muted-foreground text-sm">
              Navigation, extraction, vault, knowledge graph, entity resolution,
              time-travel debugging, automation scripts, and more.
            </p>
          </div>
          <div className="bg-fd-card border border-fd-border rounded-xl p-6">
            <h3 className="text-lg font-semibold mb-2">Knowledge Graph</h3>
            <p className="text-fd-muted-foreground text-sm">
              Every page cached and searchable. Vector embeddings, entity
              linking, full-text search via Tantivy.
            </p>
          </div>
        </div>
      </section>

      {/* Footer */}
      <footer className="border-t border-fd-border px-6 py-12 text-center text-sm text-fd-muted-foreground">
        <p>Wraith Browser — AGPL-3.0 — Free and open source, forever.</p>
      </footer>
    </main>
  );
}
