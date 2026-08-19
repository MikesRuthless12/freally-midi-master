import type { Bass808Role, Scale, SnarePlacement } from '../../lib/ipc-types';

export type Draft = {
  name: string;
  basedOn: string;
  bpmMin: number;
  bpmMax: number;
  swing: number;
  hats: number;
  melodyMin: number;
  melodyMax: number;
  scales: Scale[];
  /**
   * The four blocks TASK-040U's entry named as unreachable (2026-08-15).
   *
   * ⛔ **Every one of them is optional, and "unset" means INHERIT rather than
   * "the default".** That is the rule `scales` already states: an authored value
   * replaces the base's, so writing one the producer never chose would silently
   * overwrite the thing they said their style was based on. Each is therefore
   * empty by default and only reaches `modelFrom` once it is set.
   *
   * ⚠ **`slide` is the exception and is meaningless on its own** — it is written
   * only alongside `bassRole`, because `drums.bass808.slideProb` without a role
   * is half an 808.
   */
  snare: SnarePlacement | '';
  rolls: string[];
  bassRole: Bass808Role | '';
  slide: number;
  progressions: string[];
};

export const BLANK: Draft = {
  name: '',
  basedOn: 'trap',
  bpmMin: 130,
  bpmMax: 150,
  swing: 0.54,
  hats: 0.5,
  melodyMin: 3,
  melodyMax: 7,
  scales: [],
  snare: '',
  rolls: [],
  bassRole: '',
  slide: 0.15,
  progressions: [],
};

/**
 * The fields that are NOT a style of its own, however they are set.
 *
 * ⛔⛔ **Mike, 2026-08-16:** *"the ticking a scale and pressing the save button
 * (if you tick just a scale without generating anything, then it shouldn't save
 * anything)"*. This is the list that decides what "anything" is.
 *
 * ⛔ **`scales` is deliberately NOT in it, and that is the whole rule rather
 * than an oversight.** The scale list this dialog offers is narrowed to the
 * base's own — see the component's doc — so every scale a producer can tick is
 * one their base already uses. Ticking one restates the base; it does not add
 * to it. That is exactly why Mike singled the control out.
 *
 * ⛔ **`name` and `basedOn` are not in it either.** A name is a label on
 * content, not content; and choosing a different base picks *which* thing is
 * being restated, not what is being said over it.
 */
const NOT_CONTENT = new Set<keyof Draft>(['name', 'basedOn', 'scales']);

/**
 * Whether this draft says nothing its base does not already say.
 *
 * ⛔⛔ **Measured against the draft the dialog OPENED with, not against
 * [`BLANK`], and getting that wrong made the rule almost never fire.** The
 * opening draft is seeded from the session — `swing` and the BPM range come
 * from the selected model's `SessionDefaults`, which are populated when an
 * artist is picked rather than when Generate is pressed. `trap`, the default
 * base, authors `swing.amount: 0.50` against `BLANK.swing`'s 0.54 — so in the
 * exact case Mike named, an untouched draft already differed from `BLANK` and
 * saved anyway. The guard only fired when no model was selected at all.
 *
 * ⚠ **The opening state being content in its own right is answered elsewhere.**
 * A draft seeded from a take on screen *has* saved something the producer
 * heard — which is why the call site pairs this with whether there was a beat
 * behind the dialog, rather than trying to tell the two apart here.
 *
 * ⚠ **Stated as what is NOT content, so it fails safe.** A field added to
 * [`Draft`] is content by default; the other way round it would be a control a
 * producer can move while the dialog says there is nothing to save.
 */
export function saysNothingOfItsOwn(draft: Draft, opened: Draft): boolean {
  return (Object.keys(BLANK) as (keyof Draft)[])
    .filter((key) => !NOT_CONTENT.has(key))
    .every((key) => JSON.stringify(draft[key]) === JSON.stringify(opened[key]));
}
