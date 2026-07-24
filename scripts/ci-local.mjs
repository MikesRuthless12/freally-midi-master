#!/usr/bin/env node
/**
 * Run every gate CI runs, in CI's environment, before pushing.
 *
 * This exists because of a specific failure: a denylist run that was green
 * locally went red on CI, purely because the workflow sets
 * `CARGO_TERM_COLOR=always` and the local shell did not. The colour codes
 * changed a crate name, the name stopped matching its allowlist entry, and
 * nothing in the output explained why. Running the same commands under a
 * different environment is not a rehearsal.
 *
 * So this sets CI's environment variables as well as running CI's commands.
 * It cannot catch everything a three-OS matrix catches — a Linux-only link
 * error will still only show up on Linux — but it catches everything that is
 * merely a difference of shell.
 *
 * Usage:
 *   npm run ci:local          every gate
 *   npm run ci:local -- --fast  skip the slow ones (e2e, audit, deny)
 */

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const fast = process.argv.includes('--fast');

/** Exactly what .github/workflows/ci.yml puts in `env:`. */
const CI_ENV = {
  ...process.env,
  CI: 'true',
  CARGO_TERM_COLOR: 'always',
  RUSTFLAGS: '-D warnings',
};

const GATES = [
  { name: 'cargo fmt', cmd: 'cargo', args: ['fmt', '--all', '--check'] },
  {
    name: 'cargo clippy',
    cmd: 'cargo',
    args: ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'],
    slow: true,
  },
  { name: 'cargo test', cmd: 'cargo', args: ['test', '--workspace'], slow: true },
  {
    name: 'ts-rs bindings drift',
    cmd: 'git',
    args: ['diff', '--exit-code', '--', 'src/lib/ipc-types.ts'],
  },
  { name: 'typecheck', cmd: 'npm', args: ['run', '-s', 'typecheck'] },
  { name: 'lint', cmd: 'npm', args: ['run', '-s', 'lint'] },
  { name: 'format check', cmd: 'npm', args: ['run', '-s', 'format:check'] },
  { name: 'unit tests', cmd: 'npm', args: ['run', '-s', 'test'] },
  { name: 'frontend build', cmd: 'npm', args: ['run', '-s', 'build'] },
  { name: 'dataset validate', cmd: 'npm', args: ['run', '-s', 'dataset:validate'] },
  { name: 'denylist', cmd: 'node', args: ['scripts/check-denylist.mjs'] },
  { name: 'e2e', cmd: 'npm', args: ['run', '-s', 'test:e2e'], slow: true },
  // Runs in CI's supply-chain job but was missing here, so a newly published
  // npm advisory failed CI on a tree where every local gate had just passed.
  // A gate that CI runs and this does not is a gate this file is lying about.
  { name: 'npm audit', cmd: 'npm', args: ['audit', '--audit-level=high'], slow: true },
  { name: 'cargo audit', cmd: 'cargo', args: ['audit'], slow: true, optional: true },
  { name: 'cargo deny', cmd: 'cargo', args: ['deny', 'check'], slow: true, optional: true },
];

const results = [];
let failed = 0;

for (const gate of GATES) {
  if (fast && gate.slow) {
    results.push({ name: gate.name, status: 'skipped' });
    continue;
  }

  process.stdout.write(`  ${gate.name.padEnd(24)} `);

  // Windows cannot exec `npm` directly — it is a .cmd shim, so it needs a
  // shell. Passing the whole line as one string rather than args + shell:true
  // avoids Node's DEP0190 warning about unescaped argument concatenation.
  // Every arg here is a fixed literal with nothing interpolated.
  const useShell = process.platform === 'win32' && gate.cmd === 'npm';
  const run = useShell
    ? spawnSync(`${gate.cmd} ${gate.args.join(' ')}`, {
        cwd: root,
        env: CI_ENV,
        encoding: 'utf8',
        shell: true,
      })
    : spawnSync(gate.cmd, gate.args, { cwd: root, env: CI_ENV, encoding: 'utf8' });

  if (run.error) {
    if (gate.optional) {
      // cargo-audit / cargo-deny are not installed everywhere. CI installs
      // them; a contributor's laptop may not, and that must not read as a
      // pass — it reads as "not checked".
      console.log('NOT INSTALLED');
      results.push({ name: gate.name, status: 'unchecked' });
      continue;
    }
    // A gate that could not be launched is not a gate that failed, and
    // reporting it as one sends people hunting for a bug in their code.
    console.log('COULD NOT RUN');
    failed++;
    results.push({
      name: gate.name,
      status: 'fail',
      output: `could not launch \`${gate.cmd}\`: ${run.error.message}`,
    });
    continue;
  }

  if (run.status === 0) {
    console.log('pass');
    results.push({ name: gate.name, status: 'pass' });
  } else {
    console.log('FAIL');
    failed++;
    results.push({
      name: gate.name,
      status: 'fail',
      output: `${run.stdout ?? ''}${run.stderr ?? ''}`.trim(),
    });
  }
}

console.log();
for (const r of results.filter((r) => r.status === 'fail')) {
  console.log(`── ${r.name} ${'─'.repeat(Math.max(0, 60 - r.name.length))}`);
  console.log(r.output.split('\n').slice(-40).join('\n'));
  console.log();
}

const unchecked = results.filter((r) => r.status === 'unchecked');
if (unchecked.length) {
  console.log(
    `note: ${unchecked.map((r) => r.name).join(', ')} not installed locally — CI will still run them.`,
  );
}
if (fast) {
  console.log('note: --fast skipped the slow gates. Run without it before pushing.');
}

if (failed > 0) {
  console.log(
    `\n${failed} gate(s) failed. Fix these before pushing — CI will fail the same way.`,
  );
  process.exit(1);
}

console.log('\nAll gates pass. Note this is one OS; CI still runs ubuntu, windows and macos.');
