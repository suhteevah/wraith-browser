import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: 'Wraith Browser',
    },
    links: [
      { text: 'Playground', url: '/playground' },
      { text: 'Enterprise', url: '/docs/enterprise/pricing' },
      { text: 'Blog', url: '/blog' },
      { text: 'Community', url: '/community' },
    ],
  };
}
