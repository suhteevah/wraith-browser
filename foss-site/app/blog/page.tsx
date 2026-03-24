import Link from 'next/link';
import { getAllPosts } from '@/lib/blog';

export const metadata = {
  title: 'Blog — Wraith Browser',
  description: 'News, releases, and deep dives from the Wraith project.',
};

export default function BlogListPage() {
  const posts = getAllPosts();

  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      <section className="max-w-3xl mx-auto px-6 pt-28 pb-20">
        <h1 className="text-4xl font-bold tracking-tight mb-2">Blog</h1>
        <p className="text-fd-muted-foreground mb-12">
          News, releases, and deep dives from the Wraith project.
        </p>

        {posts.length === 0 && (
          <p className="text-fd-muted-foreground">No posts yet. Check back soon.</p>
        )}

        <div className="flex flex-col gap-6">
          {posts.map((post) => (
            <Link
              key={post.slug}
              href={`/blog/${post.slug}`}
              className="block bg-fd-card border border-fd-border rounded-xl p-6 hover:border-fd-foreground/20 transition-colors"
            >
              <time className="text-sm text-fd-muted-foreground">
                {new Date(post.date).toLocaleDateString('en-US', {
                  year: 'numeric',
                  month: 'long',
                  day: 'numeric',
                })}
              </time>
              <h2 className="text-xl font-semibold mt-1 mb-2">{post.title}</h2>
              <p className="text-fd-muted-foreground text-sm">
                {post.description}
              </p>
            </Link>
          ))}
        </div>
      </section>
    </main>
  );
}
