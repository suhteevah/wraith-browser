import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Community — Wraith Browser',
  description:
    'Join the Wraith community. Chat on Discord, contribute on GitHub, and build with us.',
};

interface CardProps {
  title: string;
  description: string;
  href: string;
  icon: React.ReactNode;
  comingSoon?: boolean;
  external?: boolean;
}

function CommunityCard({
  title,
  description,
  href,
  icon,
  comingSoon,
  external,
}: CardProps) {
  const Tag = comingSoon ? 'div' : 'a';
  return (
    <Tag
      {...(!comingSoon && {
        href,
        ...(external && { target: '_blank', rel: 'noopener noreferrer' }),
      })}
      className={`block bg-fd-card border border-fd-border rounded-xl p-6 transition-colors ${
        comingSoon
          ? 'opacity-50 cursor-default'
          : 'hover:border-fd-foreground/20 cursor-pointer'
      }`}
    >
      <div className="flex items-start gap-4">
        <div className="text-2xl shrink-0">{icon}</div>
        <div>
          <h3 className="text-lg font-semibold flex items-center gap-2">
            {title}
            {comingSoon && (
              <span className="text-xs font-normal bg-fd-border text-fd-muted-foreground px-2 py-0.5 rounded-full">
                Coming soon
              </span>
            )}
          </h3>
          <p className="text-fd-muted-foreground text-sm mt-1">
            {description}
          </p>
        </div>
      </div>
    </Tag>
  );
}

export default function CommunityPage() {
  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      <section className="max-w-3xl mx-auto px-6 pt-28 pb-20">
        <h1 className="text-4xl font-bold tracking-tight mb-2">Community</h1>
        <p className="text-fd-muted-foreground mb-12">
          Wraith is built in the open. Here is how to get involved.
        </p>

        <div className="grid sm:grid-cols-2 gap-6">
          <CommunityCard
            title="GitHub Discussions"
            description="Ask questions, share ideas, and discuss features with the community."
            href="https://github.com/suhteevah/wraith-browser/discussions"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="currentColor"
                className="w-6 h-6 text-[#5865F2]"
                aria-hidden="true"
              >
                <path d="M1.75 1h12.5c.966 0 1.75.784 1.75 1.75v9.5A1.75 1.75 0 0 1 14.25 14H8.061l-2.574 2.573A1.458 1.458 0 0 1 3 15.543V14H1.75A1.75 1.75 0 0 1 0 12.25v-9.5C0 1.784.784 1 1.75 1ZM1.5 2.75v9.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h6.5a.25.25 0 0 0 .25-.25v-9.5a.25.25 0 0 0-.25-.25H1.75a.25.25 0 0 0-.25.25Z" />
                <path d="M22.5 8.75a.25.25 0 0 0-.25-.25h-3.5a.75.75 0 0 1 0-1.5h3.5c.966 0 1.75.784 1.75 1.75v9.5A1.75 1.75 0 0 1 22.25 20H21v1.543a1.457 1.457 0 0 1-2.487 1.03L15.939 20H10.75A1.75 1.75 0 0 1 9 18.25v-1.465a.75.75 0 0 1 1.5 0v1.465c0 .138.112.25.25.25h5.5a.749.749 0 0 1 .53.22l2.72 2.72v-2.19a.75.75 0 0 1 .75-.75h2a.25.25 0 0 0 .25-.25v-9.5Z" />
              </svg>
            }
            external
          />

          <CommunityCard
            title="GitHub"
            description="Browse the source, file issues, and submit pull requests."
            href="https://github.com/suhteevah/wraith-browser"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="currentColor"
                className="w-6 h-6"
              >
                <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
              </svg>
            }
            external
          />

          <CommunityCard
            title="Contributing"
            description="Read the contributing guide to learn how to submit your first pull request."
            href="https://github.com/suhteevah/wraith-browser/blob/main/CONTRIBUTING.md"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                className="w-6 h-6 text-emerald-500"
              >
                <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
                <path d="M9 18c-4.51 2-5-2-7-2" />
              </svg>
            }
            external
          />

          <CommunityCard
            title="Code of Conduct"
            description="We are committed to a welcoming, inclusive community for everyone."
            href="https://github.com/suhteevah/wraith-browser/blob/main/CODE_OF_CONDUCT.md"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                className="w-6 h-6 text-rose-400"
              >
                <path d="M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z" />
              </svg>
            }
            external
          />
        </div>

        {/* Built with Wraith showcase */}
        <section className="mt-16">
          <h2 className="text-2xl font-bold mb-2">Built with Wraith</h2>
          <p className="text-fd-muted-foreground mb-8">
            Projects and tools built by the community using Wraith Browser.
            Want to be featured?{' '}
            <a
              href="https://github.com/suhteevah/wraith-browser/issues"
              target="_blank"
              rel="noopener noreferrer"
              className="text-emerald-400 hover:underline"
            >
              Open an issue
            </a>{' '}
            to submit yours.
          </p>
          <div className="grid sm:grid-cols-2 gap-6">
            <div className="bg-fd-card border border-fd-border rounded-xl p-6">
              <h3 className="text-lg font-semibold mb-1">LLM Token Savings</h3>
              <p className="text-fd-muted-foreground text-sm mb-3">
                Wraith&apos;s snapshot model compresses full-page DOM into compact @ref
                representations — 95%+ token reduction vs raw HTML. Feed pages to
                your LLM at a fraction of the cost.
              </p>
              <span className="text-xs text-emerald-400">Built-in</span>
            </div>
            <a
              href="/docs/guides/web-scraping"
              className="block bg-fd-card border border-fd-border rounded-xl p-6 hover:border-fd-foreground/20 transition-colors"
            >
              <h3 className="text-lg font-semibold mb-1">Research Assistant</h3>
              <p className="text-fd-muted-foreground text-sm mb-3">
                Browse, extract, and build knowledge graphs across news and
                academic sources. Entities auto-linked across pages.
              </p>
              <span className="text-xs text-emerald-400">Example workflow</span>
            </a>
            <a
              href="/docs/knowledge-graph/page-cache"
              className="block bg-fd-card border border-fd-border rounded-xl p-6 hover:border-fd-foreground/20 transition-colors"
            >
              <h3 className="text-lg font-semibold mb-1">Docs Search Index</h3>
              <p className="text-fd-muted-foreground text-sm mb-3">
                Crawl documentation sites and index them into Wraith&apos;s knowledge
                graph with full-text search and vector embeddings.
              </p>
              <span className="text-xs text-emerald-400">Example workflow</span>
            </a>
            <a
              href="/docs/guides/web-scraping"
              className="block bg-fd-card border border-fd-border rounded-xl p-6 hover:border-fd-foreground/20 transition-colors"
            >
              <h3 className="text-lg font-semibold mb-1">Price Monitor</h3>
              <p className="text-fd-muted-foreground text-sm mb-3">
                Track product prices across e-commerce sites with scheduled
                scraping, dedup, and change detection.
              </p>
              <span className="text-xs text-emerald-400">Example workflow</span>
            </a>
          </div>
        </section>
      </section>
    </main>
  );
}
