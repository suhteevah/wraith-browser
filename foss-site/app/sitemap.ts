import type { MetadataRoute } from 'next';
import { source } from '@/lib/source';
import { getAllPosts } from '@/lib/blog';

export default function sitemap(): MetadataRoute.Sitemap {
  const baseUrl = 'https://wraith-docs-suhteevahs-projects.vercel.app';
  const docs = source.getPages().map((page) => ({
    url: `${baseUrl}${page.url}`,
    lastModified: new Date(),
  }));
  const posts = getAllPosts().map((post) => ({
    url: `${baseUrl}/blog/${post.slug}`,
    lastModified: new Date(post.date),
  }));
  return [
    { url: baseUrl, lastModified: new Date() },
    { url: `${baseUrl}/blog`, lastModified: new Date() },
    { url: `${baseUrl}/playground`, lastModified: new Date() },
    { url: `${baseUrl}/community`, lastModified: new Date() },
    ...docs,
    ...posts,
  ];
}
