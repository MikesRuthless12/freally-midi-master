import { invoke } from '../../lib/ipc';
import type { Part } from '../../lib/ipc-types';

/**
 * Sound one note, for the keyboard gutter (TASK-041).
 *
 * ⛔ **Fire-and-forget, and a failure is deliberately silent.** An audition is
 * feedback on a gesture that has already happened — the click has landed, the
 * note is drawn — so a rejected promise has nothing left to correct. Surfacing
 * it would put an error banner under the Generate button (which is where
 * `session.error` renders) for a preview that did not sound, in a UI where the
 * preview being off is a *supported* mode: `audioEnabled` false is MIDI-only and
 * is a first-class choice, not a degraded one. Every browser and dev session
 * also has no audio thread at all, so the common case for this rejecting is
 * "there is nothing to play it with".
 *
 * The plugin refuses the command outright when the preview is disabled rather
 * than sounding a note the producer has switched off, so this does not need to
 * consult `audioEnabled` itself — one authority, on the side that owns the
 * sampler.
 */
export async function auditionNote(pitch: number, part: Part): Promise<void> {
  // Out of MIDI's range there is no note to sound, and the bridge would reject
  // it — cheaper and clearer to not ask.
  if (pitch < 0 || pitch > 127) return;
  try {
    await invoke('audition_note', { pitch, part });
  } catch {
    // See above.
  }
}
