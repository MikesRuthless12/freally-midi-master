import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadRoster } from './roster';
import type { RosterSummary } from './ipc-types';

// No Rust backend under jsdom, so `invoke` routes through `ipc-mock` on its own
// — the same path Playwright uses. Nothing to stub for the happy case.

afterEach(() => {
  vi.restoreAllMocks();
});

describe('loadRoster', () => {
  it('returns the roster and logs how many models arrived', async () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => {});

    const summary = await loadRoster();

    expect(summary.entries.length).toBeGreaterThan(0);
    expect(summary.entries.map((e) => e.id)).toContain('trap');
    expect(info).toHaveBeenCalledWith(
      expect.stringContaining(`${summary.entries.length} models`),
    );
  });

  it('names every skipped model rather than only counting them', async () => {
    // A count tells a user nothing they can act on; the file and the reason do.
    // Driven through the real function with a stubbed response, so the
    // formatting under test is the formatting that ships.
    const broken: RosterSummary = {
      datasetVersion: '9.9.9',
      entries: [],
      problems: [{ source: 'genres/torn.json', message: 'invalid JSON at line 3' }],
    };
    const ipc = await import('./ipc');
    vi.spyOn(ipc, 'invoke').mockResolvedValue(broken);
    vi.spyOn(console, 'info').mockImplementation(() => {});
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    await loadRoster();

    expect(warn).toHaveBeenCalledWith(expect.stringContaining('genres/torn.json'));
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('invalid JSON at line 3'));
  });
});
