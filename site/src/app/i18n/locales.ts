// English only for now. The plumbing is the same one the other FerrLabs sites
// use, so adding a locale is adding it here and translating the content — not
// rebuilding the site.
export const LOCALES = ['en'] as const;
export const DEFAULT_LOCALE = 'en';

export type Locale = (typeof LOCALES)[number];
