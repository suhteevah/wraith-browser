import type { MetadataRoute } from 'next';

export default function robots(): MetadataRoute.Robots {
  return {
    rules: { userAgent: '*', allow: '/' },
    sitemap: 'https://wraith-docs-suhteevahs-projects.vercel.app/sitemap.xml',
  };
}
