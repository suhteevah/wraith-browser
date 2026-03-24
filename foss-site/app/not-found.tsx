import Link from 'next/link';

export default function NotFound() {
  return (
    <main className="min-h-screen flex flex-col items-center justify-center bg-fd-background text-fd-foreground">
      <h1 className="text-6xl font-bold">404</h1>
      <p className="mt-4 text-lg text-fd-muted-foreground">
        This page doesn&apos;t exist.
      </p>
      <div className="mt-8 flex gap-4">
        <Link
          href="/"
          className="px-5 py-2 rounded-lg bg-fd-primary text-fd-primary-foreground font-medium hover:opacity-90 transition-opacity"
        >
          Home
        </Link>
        <Link
          href="/docs"
          className="px-5 py-2 rounded-lg border border-fd-border hover:bg-fd-accent text-fd-foreground font-medium transition-colors"
        >
          Docs
        </Link>
      </div>
    </main>
  );
}
