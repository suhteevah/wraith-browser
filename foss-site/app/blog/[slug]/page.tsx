import { notFound } from 'next/navigation';
import Link from 'next/link';
import type { Metadata } from 'next';
import { getAllPosts, getPost } from '@/lib/blog';

interface Props {
  params: Promise<{ slug: string }>;
}

export async function generateStaticParams() {
  return getAllPosts().map((post) => ({ slug: post.slug }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  const post = getPost(slug);
  if (!post) return {};

  return {
    title: `${post.title} — Wraith Browser`,
    description: post.description,
    openGraph: {
      title: post.title,
      description: post.description,
      type: 'article',
      publishedTime: post.date,
      authors: [post.author],
    },
  };
}

/**
 * Renders trusted MDX content as React elements.
 *
 * SECURITY NOTE: This function uses dangerouslySetInnerHTML for inline
 * formatting (code spans, links). This is safe because the content source
 * is our own MDX files in content/blog/, NOT user-submitted input.
 */
function renderContent(raw: string) {
  const lines = raw.trim().split('\n');
  const elements: React.ReactNode[] = [];
  let key = 0;
  let inCode = false;
  let codeLines: string[] = [];

  function flushCodeBlock() {
    if (inCode) {
      elements.push(
        <pre
          key={key++}
          className="bg-fd-card border border-fd-border rounded-lg p-4 overflow-x-auto my-6 text-sm font-mono"
        >
          <code>{codeLines.join('\n')}</code>
        </pre>,
      );
      codeLines = [];
    }
  }

  for (const line of lines) {
    if (line.startsWith('```')) {
      if (inCode) {
        flushCodeBlock();
        inCode = false;
      } else {
        inCode = true;
      }
      continue;
    }

    if (inCode) {
      codeLines.push(line);
      continue;
    }

    if (line.startsWith('## ')) {
      elements.push(
        <h2 key={key++} className="text-2xl font-bold mt-10 mb-4">
          {line.slice(3)}
        </h2>,
      );
    } else if (line.startsWith('### ')) {
      elements.push(
        <h3 key={key++} className="text-xl font-semibold mt-8 mb-3">
          {line.slice(4)}
        </h3>,
      );
    } else if (line.trim() === '') {
      // skip blank lines
    } else {
      // Apply inline formatting for trusted MDX content (code spans, links, bold)
      const rendered = line
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(
          /`([^`]+)`/g,
          '<code class="bg-fd-card px-1.5 py-0.5 rounded text-sm font-mono">$1</code>',
        )
        .replace(
          /\[([^\]]+)\]\(([^)]+)\)/g,
          '<a href="$2" class="text-emerald-400 hover:underline">$1</a>',
        );

      elements.push(
        <p
          key={key++}
          className="text-fd-muted-foreground leading-relaxed mb-4"
          // Safe: content is from our own MDX files, not user input
          dangerouslySetInnerHTML={{ __html: rendered }}
        />,
      );
    }
  }

  // flush any trailing code block
  if (inCode) flushCodeBlock();

  return elements;
}

export default async function BlogPostPage({ params }: Props) {
  const { slug } = await params;
  const post = getPost(slug);
  if (!post) notFound();

  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      <article className="max-w-3xl mx-auto px-6 pt-28 pb-20">
        <Link
          href="/blog"
          className="text-sm text-fd-muted-foreground hover:text-fd-foreground transition-colors"
        >
          &larr; Back to blog
        </Link>

        <header className="mt-6 mb-10">
          <time className="text-sm text-fd-muted-foreground">
            {new Date(post.date).toLocaleDateString('en-US', {
              year: 'numeric',
              month: 'long',
              day: 'numeric',
            })}
          </time>
          <h1 className="text-4xl font-bold tracking-tight mt-2 mb-3">
            {post.title}
          </h1>
          <p className="text-fd-muted-foreground">By {post.author}</p>
        </header>

        <div>{renderContent(post.content)}</div>
      </article>
    </main>
  );
}
