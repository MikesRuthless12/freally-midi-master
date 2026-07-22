#!/usr/bin/env node
/**
 * Fails the build if a forbidden dependency would ship in the product.
 *
 * Two product guarantees rest on this, and neither is checkable by reading the
 * marketing copy:
 *
 *   1. There is NO AI/ML anywhere in this product. Not a model, not a runtime,
 *      not an inference client. This is the legal architecture, not a
 *      preference — see docs/legal/disclaimer.md.
 *   2. Generation, playback, import, render and export never touch the network.
 *      The only sanctioned outbound traffic is the Havoc-standard update check
 *      and the user-initiated crash report.
 *
 * ## Why this reads build graphs, not lockfiles
 *
 * A lockfile records everything Cargo *resolved*, including feature-gated
 * dependencies that are never compiled. `tauri` resolves `reqwest` that way:
 * it appears in Cargo.lock but is linked on none of the three shipping
 * targets. Failing on lockfile presence would therefore fail on day one and
 * teach everyone to ignore the check — which is worse than not having it.
 *
 * So the two rules are enforced at different strengths, deliberately:
 *
 *   - AI/ML: forbidden even in the lockfile. It should not be resolvable, let
 *     alone linked. There is no legitimate reason for one to appear.
 *   - HTTP clients and telemetry: forbidden in what actually ships — the
 *     normal (non-dev, non-build) dependency graph for the host target, and
 *     the production npm tree.
 *
 * Usage: node scripts/check-denylist.mjs
 */

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/**
 * SGR escape sequences. CI sets CARGO_TERM_COLOR=always, which wraps cargo
 * tree's "(*)" repeat marker in colour codes; without stripping them a crate
 * name never matches its allowlist entry, and a run that is green locally goes
 * red on CI for reasons the output does not explain.
 */
// eslint-disable-next-line no-control-regex
const ANSI = new RegExp(String.fromCharCode(27) + '\\[[0-9;]*m', 'g');

const AI_RUNTIMES = [
  'onnxruntime',
  'onnx',
  'ort',
  'tensorflow',
  'tflite',
  'torch',
  'libtorch',
  'candle-core',
  'candle-nn',
  'tract-onnx',
  'burn-core',
  'llama',
  'llm-chain',
  'rust-bert',
  'openai',
  'anthropic',
  '@anthropic-ai',
  'cohere',
  'replicate',
  'huggingface',
  '@huggingface',
  '@tensorflow',
  'openvino',
  'ncnn',
  'ggml',
  'llama-cpp',
  'llamaindex',
  'langchain',
];

const HTTP_CLIENTS = [
  'reqwest',
  'hyper',
  'ureq',
  'isahc',
  'surf',
  'curl',
  'attohttpc',
  'axios',
  'node-fetch',
  'superagent',
  'got',
  'undici',
  'phin',
  'needle',
  'request',
];

const TELEMETRY = [
  'sentry',
  'mixpanel',
  'amplitude',
  'posthog',
  'bugsnag',
  'datadog',
  'segment-analytics',
  'google-analytics',
];

/**
 * Packages allowed despite matching a rule, each with its reason.
 *
 * Keep this SHORT. Adding a name here is a product decision, not a build fix.
 */
const ALLOWED = {
  // The Havoc-standard update check (TASK-014B), documented in EULA.md § 5 and
  // PRD § 13. This is the product's ONE sanctioned network dependency: it
  // fetches latest.json from GitHub releases to compare version numbers, sends
  // nothing about the user, and installs nothing without an explicit yes.
  //
  // All four arrive transitively under tauri-plugin-updater — verified with
  // `cargo tree -i`. Nothing in engine/, the audio path or the export path may
  // reach them, which `engine`'s own dependency list enforces: it has none of
  // these, and it is a pure library with no Tauri dependency at all.
  reqwest: 'transitive dep of tauri-plugin-updater — update check only',
  hyper: 'transitive dep of reqwest under tauri-plugin-updater',
  hyper_rustls: 'transitive dep of reqwest under tauri-plugin-updater',
  hyper_util: 'transitive dep of reqwest under tauri-plugin-updater',
};

/** Exact name, or a `name-suffix` / `name_suffix` variant of it. */
function matches(name, needle) {
  return name === needle || name.startsWith(`${needle}-`) || name.startsWith(`${needle}_`);
}

function findViolations(names, ecosystem, rules, scope) {
  const seen = new Set();
  const out = [];
  for (const name of names) {
    const lower = name.toLowerCase();
    if (lower in ALLOWED || seen.has(lower)) continue;
    for (const [rule, list] of rules) {
      const hit = list.find((needle) => matches(lower, needle));
      if (hit) {
        seen.add(lower);
        out.push({ ecosystem, name, rule, matched: hit, scope });
        break;
      }
    }
  }
  return out;
}

/** Every crate name in Cargo.lock, linked or merely resolved. */
function lockfileCrates() {
  const path = join(root, 'Cargo.lock');
  if (!existsSync(path)) return [];
  return [...readFileSync(path, 'utf8').matchAll(/^name\s*=\s*"([^"]+)"/gm)].map((m) => m[1]);
}

/** A check that cannot run must fail. Passing on error is how a gate stops gating. */
function fatal(message) {
  console.error(`error: ${message}`);
  console.error('The denylist could not be evaluated, so the build cannot be trusted.');
  process.exit(2);
}

/** Crates actually compiled into the workspace binaries for this host. */
function linkedCrates() {
  try {
    const out = execFileSync(
      'cargo',
      [
        'tree',
        '--workspace',
        '--edges',
        'normal',
        '--prefix',
        'none',
        '--format',
        '{lib}',
        // CI sets CARGO_TERM_COLOR=always, which wraps the "(*)" repeat marker
        // in escape codes. Ask for plain output, and strip escapes below too —
        // this exact thing already turned a green local run into a red CI one.
        '--color',
        'never',
      ],
      {
        cwd: root,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
        maxBuffer: 32 * 1024 * 1024,
      },
    );
    const names = [
      ...new Set(
        out
          .split('\n')
          // cargo tree marks an already-expanded subtree with a trailing
          // " (*)". Leaving it attached turns `hyper_util` into
          // `hyper_util (*)`, which then misses its allowlist entry.
          // Strip ANSI escapes first: under CARGO_TERM_COLOR=always the marker
          // arrives wrapped in them and the plain "(*)" pattern never matches.
          // eslint-disable-next-line no-control-regex
          .map((l) => l.replace(ANSI, ''))
          .map((l) => l.replace(/\s*\(\*\)$/, '').trim())
          .filter(Boolean),
      ),
    ];
    if (names.length === 0) fatal('`cargo tree` returned nothing');
    return names;
  } catch (e) {
    return fatal(`\`cargo tree\` failed: ${e.message}`);
  }
}

/**
 * npm packages that can reach the shipped bundle.
 *
 * Read straight from package-lock.json rather than shelling out to `npm ls`:
 * the lockfile already marks dev-only entries, and a subprocess that fails on
 * one platform would silently skip the whole check.
 */
function npmLock() {
  const path = join(root, 'package-lock.json');
  if (!existsSync(path)) return fatal('package-lock.json is missing');
  try {
    const lock = JSON.parse(readFileSync(path, 'utf8'));
    if (!lock.packages) fatal('package-lock.json has no `packages` map (lockfileVersion < 2?)');
    return lock;
  } catch (e) {
    return fatal(`package-lock.json is unreadable: ${e.message}`);
  }
}

function npmNames({ productionOnly }) {
  const names = new Set();
  for (const [key, node] of Object.entries(npmLock().packages)) {
    if (!key) continue; // the root project itself
    // `dev` marks a devDependency subtree; `devOptional` means dev-only too.
    if (productionOnly && (node.dev === true || node.devOptional === true)) continue;
    const parts = key.split('node_modules/');
    names.add(parts[parts.length - 1]);
  }
  return [...names];
}

/** Everything in the npm lockfile, dev included. */
const allNpm = () => npmNames({ productionOnly: false });

/** npm packages that can reach the shipped bundle. */
const productionNpm = () => npmNames({ productionOnly: true });

const AI_RULE = [['AI/ML runtime or client', AI_RUNTIMES]];
const SHIPPED_RULES = [
  ['HTTP client', HTTP_CLIENTS],
  ['telemetry/analytics SDK', TELEMETRY],
];

const violations = [
  // AI/ML is absolute on BOTH sides — PRD § 7: "lockfiles contain zero ML/AI
  // packages". Not merely absent from the shipped bundle: absent entirely,
  // including from dev tooling. The claim is that this product has no AI in
  // it anywhere, and a dev-only inference client would make that false.
  ...findViolations(lockfileCrates(), 'cargo', AI_RULE, 'lockfile'),
  ...findViolations(allNpm(), 'npm', AI_RULE, 'lockfile'),

  // Network and telemetry are judged on what actually ships. A dev-time HTTP
  // client (vite's fetch polyfills, npm itself) is unavoidable and harmless;
  // one linked into the binary is not.
  ...findViolations(linkedCrates(), 'cargo', SHIPPED_RULES, 'linked into the binary'),
  ...findViolations(productionNpm(), 'npm', SHIPPED_RULES, 'production dependency'),
];

if (violations.length > 0) {
  console.error('Forbidden dependencies found:\n');
  for (const v of violations) {
    console.error(`  [${v.ecosystem}] ${v.name}  (${v.scope})`);
    console.error(`      matched "${v.matched}" — ${v.rule} is not permitted in this product.`);
  }
  console.error(`
This is an architectural invariant, not a lint.

  - AI/ML is forbidden absolutely, even as an unused resolved dependency. The
    engine is deterministic, rule-based procedural code, and that is what makes
    the product legally defensible.
  - HTTP clients are permitted ONLY as transitive dependencies of the update
    check. If that is what this is, add it to ALLOWED in
    scripts/check-denylist.mjs with the reason.

Do not delete the rule to make the build pass.
`);
  process.exit(1);
}

console.log(
  `ok: no AI/ML in ${lockfileCrates().length} resolved crates; ` +
    `no HTTP client or telemetry in ${linkedCrates().length} linked crates ` +
    `or ${productionNpm().length} production npm packages`,
);
