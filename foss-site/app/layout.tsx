import { RootProvider } from 'fumadocs-ui/provider/next';
import { GeistSans } from 'geist/font/sans';
import { GeistMono } from 'geist/font/mono';
import type { ReactNode } from 'react';
import './global.css';

export const metadata = {
  metadataBase: new URL('https://wraith-docs-suhteevahs-projects.vercel.app'),
  title: {
    default: 'Wraith Browser — AI-Agent-First Browser Engine',
    template: '%s — Wraith Browser',
  },
  description:
    'A native browser engine for AI agents. 130 MCP tools. No Chrome. ~50ms per page.',
  openGraph: {
    title: 'Wraith Browser',
    description:
      'A native browser engine for AI agents. 130 MCP tools. No Chrome. ~50ms per page.',
  },
  twitter: {
    card: 'summary_large_image' as const,
    title: 'Wraith Browser — AI-Agent-First Browser Engine',
    description:
      'A native browser engine for AI agents. 130 MCP tools. No Chrome. ~50ms per page.',
  },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html
      lang="en"
      className={`${GeistSans.variable} ${GeistMono.variable}`}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
