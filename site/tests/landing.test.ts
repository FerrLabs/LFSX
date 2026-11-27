import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const appDir = join(root, 'src/app');

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      return sources(path);
    }
    return /\.(ts|html)$/.test(entry.name) ? [path] : [];
  });
}

const files = sources(appDir);
const text = (path: string) => readFileSync(path, 'utf8');
const french: Record<string, string> = JSON.parse(
  readFileSync(join(root, 'src/i18n/fr.json'), 'utf8'),
);

const messageIds = new Set(files.flatMap((file) => [...text(file).matchAll(/@@([\w.-]+)/g)]).map((m) => m[1]));

test('every message id used by a component has a French translation', () => {
  const missing = [...messageIds].filter((id) => !(id in french));
  assert.deepEqual(missing, []);
});

test('the French catalog holds no message that no component uses', () => {
  const stale = Object.keys(french).filter((id) => !messageIds.has(id));
  assert.deepEqual(stale, []);
});

test('French messages carry no empty value and no em or en dash', () => {
  const offenders = Object.entries(french)
    .filter(([, value]) => value.trim() === '' || /[–—]/.test(value))
    .map(([id]) => id);
  assert.deepEqual(offenders, []);
});

test('landing and header sources carry no em or en dash', () => {
  const offenders = files
    .filter((file) => /(landing|site-header|code-block|install-tabs|forge-matrix|storage-cards)/.test(file))
    .filter((file) => /[–—]/.test(text(file)));
  assert.deepEqual(offenders, []);
});

test('every header section link and documented anchor resolves to an id on the landing', () => {
  const landing = text(join(appDir, 'components/landing.component.html'));
  const ids = new Set([...landing.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]));
  const header = text(join(appDir, 'chrome/site-header.component.ts'));
  const linked = [...header.matchAll(/\/#([\w-]+)/g)].map((m) => m[1]);

  assert.ok(linked.length > 0);
  for (const id of [...linked, 'why', 'quickstart', 'auth', 'storage', 'faq', 'config', 'deploy']) {
    assert.ok(ids.has(id), `landing has no #${id}`);
  }
});

test('the CI snippet keeps its literal workflow expression', () => {
  const landing = text(join(appDir, 'components/landing.component.ts'));
  assert.ok(landing.includes('"${{ secrets.GITHUB_TOKEN }}"'));
});
