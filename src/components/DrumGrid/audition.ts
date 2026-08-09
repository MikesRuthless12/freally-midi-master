import { invoke } from '../../lib/ipc';

/**
 * Sound one drum lane on its own, for the grid's row headers (TASK-043).
 *
 * The roadmap's words: *"clicking a lane's header plays that lane's sound on
 * its own, so a producer can hear which pad they are about to edit without
 * soloing and pressing play."*
 *
 * ⛔ **The lane travels as a name, and the mapping to a pad stays in Rust.**
 * `engine::midi::gm_drum_note` is the one authority on which GM note a lane is,
 * and a JavaScript copy of it would be a second one — the drift class this
 * project has been bitten by three times. The bridge refuses a name it does not
 * know rather than defaulting, so a typo here is silence rather than the wrong
 * drum.
 *
 * ⛔ **Fire-and-forget, and a failure is deliberately silent** — the same
 * argument `PianoRoll/audition.ts` spells out. An audition is feedback on a
 * gesture that has already happened, so a rejected promise has nothing left to
 * correct; the preview being switched off is a *supported* mode rather than a
 * degraded one; and every browser session has no audio thread at all, so "there
 * is nothing to play it with" is the common case for this rejecting.
 */
export async function auditionLane(lane: string): Promise<void> {
  try {
    await invoke('audition_lane', { lane });
  } catch {
    // See above.
  }
}
