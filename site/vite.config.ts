import { defineConfig } from 'vite';
import { readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import analog from '@analogjs/platform';
import tailwindcss from '@tailwindcss/vite';
import { LOCALES, DEFAULT_LOCALE } from './src/app/i18n/locales';

const here = dirname(fileURLToPath(import.meta.url));

// Every page under `src/content/docs-en`, which `pnpm sync:docs` derives from
// `docs/`. Read here rather than listed, so a new doc page is prerendered by
// existing alone and the build cannot disagree with the directory.
function docSlugs(): string[] {
  return readdirSync(join(here, 'src/content/docs-en'))
    .filter((name) => name.endsWith('.md'))
    .map((name) => name.replace(/\.md$/, ''));
}

const prerenderRoutes = ['/', '/404', ...docSlugs().map((slug) => `/docs/${slug}/`)];

export default defineConfig({
  build: {
    target: ['es2022'],
  },
  resolve: {
    dedupe: [
      '@angular/core',
      '@angular/common',
      '@angular/platform-browser',
      '@angular/router',
      '@angular/forms',
    ],
  },
  optimizeDeps: {
    include: ['@ferrlabs/ui-ng', '@ferrlabs/ui-ng/docs'],
  },
  ssr: {
    noExternal: ['@ferrlabs/ui-ng'],
  },
  plugins: [
    analog({
      static: true,
      // The full list, default first: the runtime treats the first entry as
      // the source locale whose messages are baked into the templates, and
      // only loads a catalog for the others. Filtering the default out hands
      // the runtime a list whose first entry is the translated locale, which
      // silently makes it the one that never loads. Same shape as the
      // FerrFlow-Cloud site, which is the deployment this mirrors.
      i18n: {
        defaultLocale: DEFAULT_LOCALE,
        locales: [...LOCALES],
      },
      content: {
        highlighter: 'prism',
      },
      prerender: {
        routes: prerenderRoutes,
      },
    }),
    tailwindcss(),
  ],
});
