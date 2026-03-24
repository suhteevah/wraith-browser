import type { Metadata } from 'next';
import PlaygroundHub from './playground-hub';

import firstScrape from '@/content/playground/first-scrape.json';
import fillAForm from '@/content/playground/fill-a-form.json';
import knowledgeGraph from '@/content/playground/knowledge-graph.json';
import vaultAndLogin from '@/content/playground/vault-and-login.json';

import type { SessionRecording } from '@/lib/replay-parser';

export const metadata: Metadata = {
  title: 'Interactive Playground — Wraith Browser',
  description:
    'Step through real browser automation sessions. See how Wraith navigates, extracts, fills forms, and builds knowledge graphs.',
};

const tutorials: SessionRecording[] = [
  firstScrape as SessionRecording,
  fillAForm as SessionRecording,
  knowledgeGraph as SessionRecording,
  vaultAndLogin as SessionRecording,
];

export default function PlaygroundPage() {
  return (
    <main className="min-h-screen bg-fd-background text-fd-foreground">
      <section className="max-w-4xl mx-auto px-6 pt-24 pb-8">
        <h1 className="text-4xl md:text-5xl font-bold tracking-tight">
          Interactive Playground
        </h1>
        <p className="mt-4 text-lg text-fd-muted-foreground max-w-2xl">
          Step through recorded browser automation sessions. See exactly what
          commands are sent and what responses come back — no install required.
        </p>
      </section>

      <section className="max-w-4xl mx-auto px-6 pb-24">
        <PlaygroundHub tutorials={tutorials} />
      </section>
    </main>
  );
}
