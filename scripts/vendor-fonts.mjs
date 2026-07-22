#!/usr/bin/env node
/**
 * Vendor the Noto webfonts into the repo. **Run by hand, never by the app.**
 *
 * The product does not touch the network (see scripts/check-denylist.mjs), so
 * every font ships inside the bundle. This script is how those files get here:
 * it asks Google Fonts for the CSS, downloads each woff2 subset, and rewrites
 * the CSS to point at the local copies. Running it again reproduces what is
 * committed, which is the point — hand-downloaded fonts leave no record of
 * which version or which subsets were taken.
 *
 * Why Noto: it is one type family designed to cover every writing system, so a
 * UI that switches language keeps its typography instead of falling back to
 * whatever the OS happens to have — or showing tofu when it has nothing.
 *
 * Licence is the SIL Open Font License 1.1 — not MIT and not Apache 2.0, which
 * is a common assumption. The OFL permits bundling and redistribution inside a
 * product, including a commercial one, on one real condition: the licence text
 * travels with the fonts. That is why OFL-Noto.txt is fetched rather than
 * assumed. The other condition worth knowing is that the fonts may not be sold
 * on their own, which is not something this product does.
 *
 * ## On size
 *
 * CJK is the whole cost here. A Latin face is ~35 KB; Chinese, Japanese and
 * Korean need thousands of glyphs each and run to several MB per family. Google
 * splits them into ~100 numbered chunks precisely so a page loads only the
 * chunks holding the characters actually on screen, and `unicode-range` makes
 * that automatic. So the download is large on disk and stays small at runtime:
 * an English UI still fetches one ~35 KB latin file and nothing else.
 *
 * Usage: node scripts/vendor-fonts.mjs
 */

import { mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src', 'assets', 'fonts');

/** A modern desktop UA, or the API serves ttf instead of woff2. */
const UA =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

/**
 * The three families the UI actually names, and the scripts taken from each.
 *
 * Noto Sans alone carries Latin, Greek, Cyrillic, Devanagari and Vietnamese —
 * most of Europe plus Hindi and Marathi — so these three cover the whole UI for
 * those languages without touching the per-script families below.
 */
const UI_FAMILIES = [
  { name: 'Noto Sans', slug: 'noto-sans' },
  { name: 'Noto Sans Display', slug: 'noto-sans-display' },
  { name: 'Noto Sans Mono', slug: 'noto-sans-mono' },
];

/**
 * One family per writing system Noto Sans does not itself cover.
 *
 * These are fallbacks in the CSS stack, not families the UI names directly: the
 * browser reaches for them character by character, so a Japanese string renders
 * in Noto Sans JP while the Latin around it stays in Noto Sans.
 *
 * Ordering matters for Han. SC before TC before JP before KR means Simplified
 * Chinese wins a shared ideograph, which is the right default for the largest
 * group of readers; a user reading Japanese still gets JP for kana, which is
 * what makes the text unmistakably Japanese.
 */
const SCRIPT_FAMILIES = [
  { name: 'Noto Sans SC', slug: 'noto-sans-sc' },
  { name: 'Noto Sans TC', slug: 'noto-sans-tc' },
  { name: 'Noto Sans JP', slug: 'noto-sans-jp' },
  { name: 'Noto Sans KR', slug: 'noto-sans-kr' },
  { name: 'Noto Sans Arabic', slug: 'noto-sans-arabic' },
  { name: 'Noto Sans Hebrew', slug: 'noto-sans-hebrew' },
  { name: 'Noto Sans Thai', slug: 'noto-sans-thai' },
  { name: 'Noto Sans Bengali', slug: 'noto-sans-bengali' },
  { name: 'Noto Sans Tamil', slug: 'noto-sans-tamil' },
  { name: 'Noto Sans Telugu', slug: 'noto-sans-telugu' },
  { name: 'Noto Sans Kannada', slug: 'noto-sans-kannada' },
  { name: 'Noto Sans Malayalam', slug: 'noto-sans-malayalam' },
  { name: 'Noto Sans Gujarati', slug: 'noto-sans-gujarati' },
  { name: 'Noto Sans Gurmukhi', slug: 'noto-sans-gurmukhi' },
  { name: 'Noto Sans Oriya', slug: 'noto-sans-oriya' },
  { name: 'Noto Sans Sinhala', slug: 'noto-sans-sinhala' },
  { name: 'Noto Sans Khmer', slug: 'noto-sans-khmer' },
  { name: 'Noto Sans Lao', slug: 'noto-sans-lao' },
  { name: 'Noto Sans Myanmar', slug: 'noto-sans-myanmar' },
  { name: 'Noto Sans Georgian', slug: 'noto-sans-georgian' },
  { name: 'Noto Sans Armenian', slug: 'noto-sans-armenian' },
  { name: 'Noto Sans Ethiopic', slug: 'noto-sans-ethiopic' },
  { name: 'Noto Sans Cherokee', slug: 'noto-sans-cherokee' },
  { name: 'Noto Sans Thaana', slug: 'noto-sans-thaana' },
];

/**
 * Scripts kept from the three UI families.
 *
 * Left unfiltered they would duplicate the per-script families above at three
 * times the size, for glyphs the fallback chain already resolves.
 */
const UI_SUBSETS = [
  'latin',
  'latin-ext',
  'greek',
  'greek-ext',
  'cyrillic',
  'cyrillic-ext',
  'vietnamese',
  'devanagari',
];

const DOWNLOAD_CONCURRENCY = 8;

async function get(url, asText, attempt = 1) {
  try {
    const response = await fetch(url, { headers: { 'User-Agent': UA } });
    if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
    return asText ? response.text() : Buffer.from(await response.arrayBuffer());
  } catch (e) {
    // Several hundred requests in a row will occasionally be refused. Retrying
    // matters more here than anywhere else in the repo: a silently missing
    // subset is tofu in someone's language, months later.
    if (attempt >= 4) throw new Error(`${e.message} for ${url}`, { cause: e });
    await new Promise((r) => setTimeout(r, 400 * attempt));
    return get(url, asText, attempt + 1);
  }
}

/**
 * Split the API's CSS into `{ subset, block }` records.
 *
 * The comment before each block is optional, and that is the trap: Google
 * labels the named subsets with a leading `latin` comment, but emits the ~100
 * CJK chunks as bare `@font-face` rules with no comment at all. Requiring a
 * comment silently
 * dropped 97 of Noto Sans SC's 101 faces and produced a 0.05 MB "Chinese" font
 * — which would have shipped as tofu for every CJK reader.
 *
 * So: parse every block, name it from the comment when there is one and from
 * the chunk index in its URL when there is not, and assert at the end that
 * nothing was dropped. A parser that quietly returns fewer faces than the file
 * contains is worse than one that throws.
 */
function parseFaces(css) {
  const faces = [];
  const pattern = /(?:\/\*\s*([^*]+?)\s*\*\/\s*)?(@font-face\s*\{[^}]*\})/g;
  for (const [, comment, block] of css.matchAll(pattern)) {
    // `…VH8V.4.woff2` — the number is Google's own chunk index.
    const chunk = /\.(\d+)\.woff2/.exec(block);
    const subset = comment ?? (chunk ? `chunk${chunk[1]}` : `face${faces.length}`);
    faces.push({ subset: subset.replace(/[^a-z0-9-]/gi, '-'), block });
  }

  const declared = (css.match(/@font-face/g) ?? []).length;
  if (faces.length !== declared) {
    throw new Error(`parsed ${faces.length} of ${declared} @font-face rules — parser is wrong`);
  }
  return faces;
}

/** Run `worker` over `items`, at most `limit` at a time. */
async function pool(items, limit, worker) {
  const results = [];
  let next = 0;
  const runners = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await worker(items[index], index);
    }
  });
  await Promise.all(runners);
  return results;
}

async function collect(family, keepSubsets) {
  const url =
    `https://fonts.googleapis.com/css2?family=${encodeURIComponent(family.name)}:` +
    `wght@100..900&display=swap`;
  let css;
  try {
    css = await get(url, true);
  } catch {
    // Not every Noto family publishes a variable axis; fall back to static.
    css = await get(
      `https://fonts.googleapis.com/css2?family=${encodeURIComponent(family.name)}&display=swap`,
      true,
    );
  }

  let faces = parseFaces(css);
  if (keepSubsets) faces = faces.filter((f) => keepSubsets.includes(f.subset));
  if (faces.length === 0) throw new Error(`no usable subsets for ${family.name}`);
  return faces.map((face) => ({ ...face, family }));
}

const header = `/**
 * Noto — vendored, not fetched. Regenerate with \`node scripts/vendor-fonts.mjs\`.
 *
 * DO NOT EDIT BY HAND.
 *
 * One type family across every writing system the UI can show, so switching
 * language changes the words and not the typography — and never shows tofu.
 * SIL Open Font License 1.1; see OFL-Noto.txt beside the font files.
 *
 * \`unicode-range\` is what keeps this affordable. Every subset below is on disk,
 * but the browser downloads only the ones holding characters actually on
 * screen: an English UI loads one ~35 KB latin file and none of the CJK.
 */
`;

async function main() {
  mkdirSync(outDir, { recursive: true });
  for (const file of readdirSync(outDir)) {
    // Clear the previous run so a dropped subset cannot linger as an orphan.
    if (file.endsWith('.woff2')) rmSync(join(outDir, file));
  }

  console.log('Reading the font CSS...');
  const uiFaces = (
    await pool(UI_FAMILIES, DOWNLOAD_CONCURRENCY, (f) => collect(f, UI_SUBSETS))
  ).flat();
  const scriptFaces = (
    await pool(SCRIPT_FAMILIES, DOWNLOAD_CONCURRENCY, (f) => collect(f, null))
  ).flat();
  const faces = [...uiFaces, ...scriptFaces];
  console.log(
    `${faces.length} faces across ${UI_FAMILIES.length + SCRIPT_FAMILIES.length} families\n`,
  );

  const perFamily = new Map();
  const blocks = await pool(faces, DOWNLOAD_CONCURRENCY, async ({ subset, block, family }) => {
    const remote = /url\((https:[^)]+\.woff2)\)/.exec(block);
    if (!remote) throw new Error(`no woff2 in the ${family.name}/${subset} face`);

    const filename = `${family.slug}-${subset}.woff2`;
    const bytes = await get(remote[1], false);
    writeFileSync(join(outDir, filename), bytes);
    perFamily.set(family.name, (perFamily.get(family.name) ?? 0) + bytes.length);

    return (
      `/* ${family.name} — ${subset} */\n` +
      block.replace(remote[0], `url('./${filename}') format('woff2')`).trim()
    );
  });

  // The fallback chain, emitted here so it can never drift from what was
  // actually downloaded. The UI names 'Noto Sans' / 'Noto Sans Display' /
  // 'Noto Sans Mono' first and lands here for anything they do not cover.
  const fallback = [
    ...SCRIPT_FAMILIES.map((f) => `'${f.name}'`),
    'system-ui',
    'sans-serif',
  ].join(',\n    ');

  // Two sheets, and the split is what keeps startup fast.
  //
  // All 546 faces in one stylesheet is ~470 KB of render-blocking CSS that the
  // browser must parse before it can paint anything — for a UI that is showing
  // English. So the three UI families (small, needed immediately) go in the
  // blocking sheet, and the ~520 per-script faces go in a second one the app
  // loads after first paint. Nothing is lost: `--font-noto-fallback` can name a
  // family before its @font-face exists, and the moment the second sheet lands
  // those names start resolving.
  const uiCount = uiFaces.length;
  writeFileSync(
    join(outDir, 'fonts.css'),
    `${header}\n${blocks.slice(0, uiCount).join('\n\n')}\n\n` +
      `/* Per-script fallbacks, in the order the browser should try them.\n` +
      ` * The families below load from fonts-scripts.css after first paint. */\n` +
      `:root {\n  --font-noto-fallback:\n    ${fallback};\n}\n`,
  );

  writeFileSync(
    join(outDir, 'fonts-scripts.css'),
    `/**
 * Noto, per writing system — the deferred half. DO NOT EDIT BY HAND.
 * Regenerate with \`node scripts/vendor-fonts.mjs\`.
 *
 * Loaded after first paint (see src/main.tsx), because these are ~520 faces and
 * blocking the window on them would make an English UI wait for Chinese fonts
 * it will never show. \`unicode-range\` still means nothing here is downloaded
 * until a character needs it.
 */
${blocks.slice(uiCount).join('\n\n')}
`,
  );

  const licence = await get(
    'https://raw.githubusercontent.com/notofonts/latin-greek-cyrillic/main/OFL.txt',
    true,
  );
  writeFileSync(join(outDir, 'OFL-Noto.txt'), licence);

  const total = [...perFamily.values()].reduce((a, b) => a + b, 0);
  console.log('On disk, by family:');
  for (const [name, size] of [...perFamily].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${name.padEnd(26)} ${(size / 1024 / 1024).toFixed(2)} MB`);
  }
  console.log(`\nok: ${blocks.length} faces, ${(total / 1024 / 1024).toFixed(1)} MB on disk.`);
  console.log('Runtime cost is unchanged — unicode-range loads only what is shown.');
}

await main();
