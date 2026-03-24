'use client';

import { useState } from 'react';
import PlaygroundReplay from '@/components/playground-replay';
import type { SessionRecording } from '@/lib/replay-parser';

const ICONS: Record<string, string> = {
  'Your First Scrape': '1',
  'Fill a Form': '2',
  'Build a Knowledge Graph': '3',
  'Vault & Login': '4',
};

export default function PlaygroundHub({
  tutorials,
}: {
  tutorials: SessionRecording[];
}) {
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);

  return (
    <div className="space-y-4">
      {tutorials.map((tutorial, i) => {
        const isExpanded = expandedIndex === i;
        return (
          <div
            key={tutorial.title}
            className="rounded-xl border border-fd-border bg-fd-card overflow-hidden"
          >
            <button
              onClick={() => setExpandedIndex(isExpanded ? null : i)}
              className="w-full flex items-center gap-4 px-6 py-5 text-left hover:bg-fd-accent/50 transition-colors"
            >
              <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 font-bold text-sm">
                {ICONS[tutorial.title] ?? (i + 1)}
              </div>
              <div className="flex-1 min-w-0">
                <h2 className="text-lg font-semibold text-fd-foreground">
                  {tutorial.title}
                </h2>
                <p className="text-sm text-fd-muted-foreground mt-0.5">
                  {tutorial.description}
                </p>
              </div>
              <div className="flex-shrink-0 flex items-center gap-3">
                <span className="text-xs text-fd-muted-foreground font-mono">
                  {tutorial.steps.length} steps
                </span>
                <svg
                  className={`w-5 h-5 text-fd-muted-foreground transition-transform duration-200 ${
                    isExpanded ? 'rotate-180' : ''
                  }`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M19 9l-7 7-7-7"
                  />
                </svg>
              </div>
            </button>
            {isExpanded && (
              <div className="px-6 pb-6">
                <PlaygroundReplay recording={tutorial} />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
