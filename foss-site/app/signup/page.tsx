import type { Metadata } from 'next';
import SignupForm from './signup-form';

export const metadata: Metadata = {
  title: 'Sign up — Wraith Browser',
  description:
    'Get an API key for the hosted Wraith Corpo browser automation API. Free during beta.',
};

export default function SignupPage() {
  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      <section className="max-w-2xl mx-auto px-6 pt-24 pb-8">
        <p className="text-sm font-mono uppercase tracking-widest text-fd-muted-foreground">
          Beta — free
        </p>
        <h1 className="mt-2 text-4xl md:text-5xl font-bold tracking-tight">
          Get an API key
        </h1>
        <p className="mt-4 text-lg text-fd-muted-foreground">
          The hosted Wraith Corpo API runs on a single VPS during beta — fine
          for kicking the tires, not for hammering with a million-page crawl.
          Sign up below and you&apos;ll get a JWT pair (1-hour access token +
          7-day refresh) you can use against{' '}
          <code className="font-mono text-sm bg-fd-muted px-1 py-0.5 rounded">
            https://wraith-browser.vercel.app/api/v1/*
          </code>
          .
        </p>
        <p className="mt-3 text-sm text-fd-muted-foreground">
          For self-hosting (free, AGPL-3.0),{' '}
          <a
            className="underline underline-offset-4"
            href="https://github.com/suhteevah/wraith-browser"
          >
            clone the GitHub repo
          </a>{' '}
          instead — the managed API is for teams that don&apos;t want to run
          the engine themselves.
        </p>
      </section>

      <section className="max-w-2xl mx-auto px-6 pb-24">
        <SignupForm />
      </section>
    </main>
  );
}
