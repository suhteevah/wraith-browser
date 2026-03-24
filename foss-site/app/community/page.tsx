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
            title="Discord"
            description="Chat with contributors, ask questions, and share what you are building."
            href="#"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="currentColor"
                className="w-6 h-6 text-[#5865F2]"
              >
                <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028 14.09 14.09 0 0 0 1.226-1.994.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
              </svg>
            }
            external
          />

          <CommunityCard
            title="Matrix"
            description="Federated, open-protocol chat. Bridge to Discord coming soon."
            href="#"
            icon={
              <svg
                viewBox="0 0 24 24"
                fill="currentColor"
                className="w-6 h-6 text-fd-muted-foreground"
              >
                <path d="M.632.55v22.9H2.28V24H0V0h2.28v.55zm7.043 7.26v1.157h.033c.309-.443.683-.784 1.117-1.024.433-.245.936-.365 1.5-.365.54 0 1.033.107 1.488.323.45.214.773.553.964 1.014.309-.46.7-.82 1.17-1.08.47-.26.995-.39 1.575-.39.435 0 .842.066 1.22.2.378.132.7.33.967.59.267.265.47.583.614.96.145.374.218.81.218 1.305v5.36h-2.07v-4.64c0-.282-.013-.555-.04-.82a1.678 1.678 0 0 0-.186-.653.974.974 0 0 0-.416-.436c-.18-.107-.418-.16-.715-.16a1.48 1.48 0 0 0-.715.16.97.97 0 0 0-.424.44c-.11.19-.178.398-.2.628-.026.23-.04.46-.04.693v4.79h-2.07V12.4c0-.254-.004-.503-.013-.75a1.86 1.86 0 0 0-.14-.636.946.946 0 0 0-.38-.453c-.17-.116-.416-.174-.735-.174-.09 0-.218.022-.383.065a1.26 1.26 0 0 0-.45.22 1.28 1.28 0 0 0-.37.44c-.104.19-.155.443-.155.76v4.94H5.46V7.81zm13.042 16.19H21.72V24h-2.28V0h2.28v.55h-1.003v22.9z" />
              </svg>
            }
            comingSoon
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
          <div className="bg-fd-card border border-fd-border border-dashed rounded-xl p-12 text-center text-fd-muted-foreground text-sm">
            No showcases yet. Be the first to build something and share it.
          </div>
        </section>
      </section>
    </main>
  );
}
