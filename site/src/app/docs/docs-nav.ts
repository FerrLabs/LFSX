import type { DocSection } from '@ferrlabs/ui-ng/docs';

// One entry per file in `docs/`. Grouped the way somebody arrives at them:
// getting it running, then where the bytes go, then who is allowed near them.
export const DOCS_NAV: readonly DocSection[] = [
  {
    label: 'Getting started',
    items: [
      { label: 'Why LFSX', slug: 'why' },
      { label: 'Quick start', slug: 'quickstart' },
      { label: 'Configuration', slug: 'configuration' },
    ],
  },
  {
    label: 'Storage',
    items: [
      { label: 'Storage layout', slug: 'storage-layout' },
      { label: 'Objects in a bucket', slug: 'buckets' },
      { label: 'Compression', slug: 'compression' },
      { label: 'Encryption at rest', slug: 'encryption' },
    ],
  },
  {
    label: 'Access',
    items: [
      { label: 'Authentication', slug: 'authentication' },
      { label: 'Anonymous read', slug: 'anonymous-read' },
      { label: 'Size limits', slug: 'size-limits' },
    ],
  },
  {
    label: 'Using it',
    items: [
      { label: 'Locking', slug: 'locking' },
      { label: 'Reclaiming space', slug: 'reclaiming-space' },
      { label: 'Inspecting a repository', slug: 'inspecting' },
      { label: 'Clients', slug: 'clients' },
    ],
  },
  {
    label: 'Running it',
    items: [
      { label: 'Operations', slug: 'operations' },
      { label: 'Observability', slug: 'observability' },
      { label: 'Kubernetes', slug: 'kubernetes' },
      { label: 'Reverse proxy', slug: 'reverse-proxy' },
    ],
  },
  {
    label: 'Reference',
    items: [
      { label: 'API', slug: 'api' },
      { label: 'Protocol coverage', slug: 'protocol' },
      { label: 'Performance', slug: 'performance' },
      { label: 'Releases', slug: 'releases' },
      { label: 'Development', slug: 'development' },
    ],
  },
];
