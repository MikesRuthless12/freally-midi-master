/**
 * What a drag out of the sample browser carries.
 *
 * ⛔⛔ **Two halves of one rule, and they were five string literals in four
 * files.** `FileTree` sets the type; `PadGrid`, `KitPanel` and `CenterStage` each
 * decide whether to accept it. A drop target that refuses a `.mid` and a drop
 * target that accepts one are the *same rule*, and expressed as separate literals
 * there is no compiler relationship between them at all — a typo in either half
 * produces a target that silently never activates. The cursor just says no, which
 * is indistinguishable from the intended behaviour.
 *
 * ⚠ This is the argument `engine/src/formats.rs` makes in its own header about
 * the extension lists, one layer up: the Rust side centralised in this change
 * while the TypeScript side was quietly growing a fifth copy.
 */

/** An audio file: a one-shot for a drum pad or an instrument slot. */
export const SAMPLE_TYPE = 'application/x-freally-sample';

/**
 * A MIDI file: notes for a generator, or a whole song for the Song tab.
 *
 * ⛔ **Deliberately not [`SAMPLE_TYPE`].** A `.mid` on a drum pad and a `.wav` on
 * a generator are both controls that can only fail, and two types is what makes
 * each target refuse the other *before* the drop rather than erroring after it.
 */
export const MIDI_TYPE = 'application/x-freally-midi';

/**
 * A drum pad being dragged to a new slot (2026-08-11).
 *
 * ⛔ **Its own type, because a pad and a sample land on the same element and
 * mean opposite things.** Mike: *"you should be able to click and drag your drum
 * pads and replace the ordering."* Sharing [`SAMPLE_TYPE`] would make a pad drag
 * look like a file drop and try to assign a lane name as a path.
 *
 * The payload is the source pad's **slot index**, as a string — the slot rather
 * than the lane, because two pads may hold the same lane.
 */
export const PAD_TYPE = 'application/x-freally-pad';

/**
 * The sample path a drop carries, or `null` if it carries none.
 *
 * ⚠ `getData` answers `''` rather than `null` for a type that is not present, and
 * three call sites were each turning that into a null check of their own.
 */
export function droppedSample(transfer: DataTransfer): string | null {
  const path = transfer.getData(SAMPLE_TYPE);
  return path === '' ? null : path;
}

/** The same, for a MIDI file. */
export function droppedMidi(transfer: DataTransfer): string | null {
  const path = transfer.getData(MIDI_TYPE);
  return path === '' ? null : path;
}
