// The docs in `docs/` are the source: readable on GitHub, one file per topic,
// linking to each other the way a reader browsing the repository expects. The
// site needs the same prose with frontmatter and site-absolute links, so it is
// derived here rather than kept as a second copy that drifts.
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const from = join(here, '../../docs');
const to = join(here, '../src/content/docs-en');

function title(markdown) {
  const heading = markdown.match(/^#\s+(.+)$/m);
  if (!heading) throw new Error('a doc page needs an H1 to take its title from');
  return heading[1].trim();
}

// The first real paragraph, flattened: it becomes the meta description, and a
// sentence lifted from the page beats one written once and left behind.
function description(markdown) {
  const body = markdown.replace(/^#\s+.+$/m, '');
  for (const block of body.split(/\n\s*\n/)) {
    const text = block.trim();
    if (!text || text.startsWith('#') || text.startsWith('|') || text.startsWith('```')) continue;
    return text
      .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
      .replace(/[*`]/g, '')
      .replace(/\s+/g, ' ')
      .slice(0, 300)
      .trim();
  }
  return '';
}

function quote(value) {
  return `'${value.replace(/'/g, "''")}'`;
}

rmSync(to, { recursive: true, force: true });
mkdirSync(to, { recursive: true });

const pages = readdirSync(from).filter((name) => name.endsWith('.md'));
for (const name of pages) {
  const source = readFileSync(join(from, name), 'utf8');

  const body = source
    // `reclaiming-space.md` reads as a sibling file on GitHub and as a route here.
    .replace(/\]\(([a-z0-9-]+)\.md\)/g, '](/docs/$1)')
    // Anything pointing back out of `docs/` is a file in the repository, and the
    // site has no copy of it, so it goes to GitHub.
    .replace(/\]\(\.\.\/([^)]+)\)/g, '](https://github.com/FerrLabs/LFSX/blob/main/$1)')
    .replace(/^#\s+.+$/m, '')
    .trim();

  writeFileSync(
    join(to, name),
    `---\ntitle: ${quote(title(source))}\ndescription: ${quote(description(source))}\n---\n\n${body}\n`,
  );
}

console.log(`synced ${pages.length} doc pages from docs/`);
