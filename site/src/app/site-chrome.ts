import type { SiteChromeConfig } from '@ferrlabs/ui-ng';

// Two objects, one set of bytes: the mark says what the server does before a
// word of the page is read.
const LOGO_SVG = `<svg viewBox="0 0 32 32" fill="none" aria-hidden="true">
  <rect x="4" y="7" width="12" height="8" rx="1.5" stroke="currentColor" stroke-width="1.5" />
  <rect x="4" y="17" width="12" height="8" rx="1.5" stroke="currentColor" stroke-width="1.5" />
  <path d="M18 11h5a3 3 0 0 1 3 3v4a3 3 0 0 1-3 3h-5" stroke="currentColor" stroke-width="1.5" />
  <circle cx="26" cy="16" r="3" fill="currentColor" opacity="0.6" />
</svg>`;

export function lfsxChrome(): SiteChromeConfig {
  return {
    origin: 'https://lfsx.dev',
    logoSvg: LOGO_SVG,
    wordmark: 'lfsx',
    navLinks: [
      { label: $localize`:@@nav.docs:Docs`, href: '/docs/quickstart' },
      { label: $localize`:@@nav.why:Why LFSX`, href: '/docs/why' },
      { label: $localize`:@@nav.performance:Performance`, href: '/docs/performance' },
      {
        label: $localize`:@@nav.github:GitHub`,
        href: 'https://github.com/FerrLabs/LFSX',
        external: true,
      },
    ],
    cta: { label: $localize`:@@nav.cta:Get started`, href: '/docs/quickstart' },
    footer: {
      tagline: $localize`:@@footer.tagline:Your assets on your disk, your permissions from your forge. One Rust binary, no database.`,
      backLabel: $localize`:@@footer.back:← Back to ferrlabs.com`,
      backHref: 'https://ferrlabs.com',
      columns: [
        {
          title: $localize`:@@footer.resources:Resources`,
          links: [
            { label: $localize`:@@footer.quickstart:Quick start`, href: '/docs/quickstart' },
            { label: $localize`:@@footer.configuration:Configuration`, href: '/docs/configuration' },
            { label: $localize`:@@footer.operations:Operations`, href: '/docs/operations' },
            {
              label: $localize`:@@footer.releases:Releases`,
              href: 'https://github.com/FerrLabs/LFSX/releases',
              external: true,
            },
          ],
        },
        {
          title: $localize`:@@footer.project:Project`,
          links: [
            {
              label: $localize`:@@footer.source:Source`,
              href: 'https://github.com/FerrLabs/LFSX',
              external: true,
            },
            {
              label: $localize`:@@footer.issues:Issues`,
              href: 'https://github.com/FerrLabs/LFSX/issues',
              external: true,
            },
            {
              label: $localize`:@@footer.licence:MPL-2.0`,
              href: 'https://github.com/FerrLabs/LFSX/blob/main/LICENSE',
              external: true,
            },
          ],
        },
      ],
      bottomLeft: $localize`:@@footer.rights:© 2026 FerrLabs. LFSX is a FerrLabs product.`,
      bottomRight: 'MPL-2.0',
    },
    labels: {
      menu: $localize`:@@nav.menu:Menu`,
      close: $localize`:@@nav.close:Close`,
      openMenu: $localize`:@@nav.openMenu:Open menu`,
    },
  };
}
