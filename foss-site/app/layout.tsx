import { RootProvider } from 'fumadocs-ui/provider/next';
import { GeistSans } from 'geist/font/sans';
import { GeistMono } from 'geist/font/mono';
import type { ReactNode } from 'react';
import './global.css';

export const metadata = {
  title: 'OpenClaw Browser — AI-Agent-First Browser Engine',
  description:
    'A native browser engine for AI agents. 143 MCP tools. No Chrome. ~50ms per page.',
  openGraph: {
    title: 'OpenClaw Browser',
    description:
      'A native browser engine for AI agents. 143 MCP tools. No Chrome. ~50ms per page.',
    images: ['/og-image.png'],
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
