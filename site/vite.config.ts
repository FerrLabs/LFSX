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
      // Only the locales beyond the default. Passing the default one as well
      // makes Analog prerender the whole site a second time under `/en/`, and a
      // docs site whose reason to exist is being found does not want two of
      // every page, each canonical to itself.
      i18n: {
        defaultLocale: DEFAULT_LOCALE,
        locales: LOCALES.filter((locale) => locale !== DEFAULT_LOCALE),
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
