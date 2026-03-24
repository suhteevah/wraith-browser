import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: 'OpenClaw Browser',
    },
    links: [
      { text: 'Playground', url: '/playground' },
      { text: 'Blog', url: '/blog' },
      { text: 'Community', url: '/community' },
    ],
  };
}
