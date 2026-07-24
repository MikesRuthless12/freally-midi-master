import { invoke } from './ipc';
import type { RosterSummary } from './ipc-types';

/**
 * The roster: every artist and genre the user can generate from.
 *
 * Loaded once — the Rust side resolves the whole dataset at startup, so this is
 * a copy of something already in memory, not a read of the disk. Search runs
 * over the result in the frontend (PRD § 3 Indexes) so a keystroke never costs
 * an IPC round trip.
 */
export const rosterSummary = () => invoke<RosterSummary>('roster_summary');

/**
 * Load the roster and say what arrived.
 *
 * The log line is the point: "the dataset loaded" is otherwise invisible until
 * something searches it, and a dataset that failed to bundle looks exactly like
 * one that is simply empty.
 */
export async function loadRoster(): Promise<RosterSummary> {
  const summary = await rosterSummary();
  const skipped = summary.problems.length;

  console.info(
    `dataset ${summary.datasetVersion}: ${summary.entries.length} models` +
      (skipped > 0 ? `, ${skipped} skipped` : ''),
  );
  // Each on its own line, with the file that caused it — a count alone is not
  // something anyone can act on.
  for (const problem of summary.problems) {
    console.warn(`dataset: skipped ${problem.source} — ${problem.message}`);
  }

  return summary;
}
