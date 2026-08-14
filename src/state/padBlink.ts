/**
 * Light a pad as its lane fires (TASK-054C).
 *
 * Mike, 2026-08-11: *"the drum pads should blink with a color as the drum parts
 * play or the melodic parts play for the drum pads or the melody/chords/
 * basslines/counter melody."*
 *
 * ## ⛔ Nothing here re-renders, and that is the whole design
 *
 * `playhead` is written to the session **thirty times a second** while anything
 * is rolling. A component that subscribed to it to decide which pads are lit
 * would re-render the eight pads and every kit row at that rate, which is
 * precisely the cost `DrumGrid` documents at length and avoids by drawing its
 * marker from a CSS custom property. So this is a subscription that writes a
 * `data-lit` attribute straight onto the DOM nodes — the same shape
 * `WindowFit.tsx::applyZoom` uses, and for the same reason.
 *
 * ## ⛔ Derived on this side, not published by the audio thread
 *
 * The page already has both halves: the notes (`session.patterns`) and the read
 * position (`session.playhead`, a fraction of the pattern). A second channel
 * from the audio thread would be a per-lane trigger flag polled at frame rate —
 * more round trips on the thread a DAW draws its editor from, to say something
 * derivable from two values already in hand. ⚠ The cost of deriving is that a
 * pad lights when the note is *scheduled*, not when the sampler actually voices
 * it; at 30 Hz those are the same frame.
 */

import { laneAudible, useSession } from './session';
import { patternTicks } from '../components/PianoRoll/notes';
import type { Lane, Pattern } from '../lib/ipc-types';

/**
 * How long a pad stays lit, in milliseconds.
 *
 * ⚠ **Long enough to see, shorter than a 16th at any tempo anybody works at.**
 * A 16th at 200 BPM is 75 ms, so a light that outlived one would still be on
 * when the next hit arrived and a fast hat roll would read as solid rather than
 * as a roll. The fade is CSS; this is only how long the attribute is set.
 */
export const BLINK_MS = 70;

/** One note start, as a fraction of the pattern. */
type Hit = { at: number; lane: Lane };

/**
 * Every note start in the session, sorted, as fractions of the pattern.
 *
 * ⛔ **Rebuilt only when the patterns change**, not per frame. A four-bar drum
 * pattern is a few hundred notes and the playhead ticks 30 times a second;
 * walking the whole session on every tick would be the one expensive thing in a
 * feature whose entire point is that it is cheap.
 *
 * ⚠ **Every part, not only the drums.** Mike named the melodic lanes in the same
 * sentence as the pads, and a melodic part's lane (`melody`, `chords`, `bass`,
 * `counter`) is what the KIT row is keyed on — so the same list drives both.
 */
function hitsOf(patterns: Record<string, Pattern>): Hit[] {
  const hits: Hit[] = [];
  for (const pattern of Object.values(patterns)) {
    const total = patternTicks(pattern);
    for (const track of pattern.lanes) {
      for (const note of track.notes) {
        // ⚠ Clamped rather than dropped. A note past the clip's end is not
        // something this should have an opinion about — `within_clip` on the
        // plugin side owns that rule — and letting it sort past 1.0 would only
        // mean it never lights.
        hits.push({ at: Math.min(1, note.startTick / total), lane: track.lane });
      }
    }
  }
  return hits.sort((a, b) => a.at - b.at);
}

/**
 * Follow the playhead and light each lane as it is crossed.
 *
 * Returns the unsubscribe, like `subscribeToPlayhead` beside it in `App.tsx`.
 */
export function subscribeToPadBlink(): () => void {
  let hits = hitsOf(useSession.getState().patterns);
  let previous = useSession.getState().playhead;
  const timers = new Map<Lane, number>();

  const light = (lane: Lane) => {
    // ⛔ **Both surfaces, named explicitly.** `data-lane` is also on the drum
    // grid's rows and on the velocity lane's stems, and lighting those would be
    // a second animation nobody asked for running across the editor. The pads
    // and the kit rows are what Mike named.
    const nodes = document.querySelectorAll<HTMLElement>(
      `.pad[data-lane="${lane}"], .kit-lane[data-lane="${lane}"]`,
    );
    if (nodes.length === 0) return;
    for (const node of nodes) node.dataset.lit = 'true';

    // ⚠ **One timer per lane, restarted.** Two hits inside the blink window —
    // a flam, or two pads holding the same lane — would otherwise have the
    // first one's timer switch the light off while the second was still meant
    // to be on.
    window.clearTimeout(timers.get(lane));
    timers.set(
      lane,
      window.setTimeout(() => {
        for (const node of nodes) delete node.dataset.lit;
        timers.delete(lane);
      }, BLINK_MS),
    );
  };

  // ⚠ True whenever the playhead has not been advanced *through* since the
  // transport started or was moved — see `crossed` for what it changes.
  let fromRest = true;

  const unsubscribe = useSession.subscribe((state, before) => {
    if (state.patterns !== before.patterns) hits = hitsOf(state.patterns);

    const now = state.playhead;
    if (now === previous) return;

    // ⛔ **Nothing lights while the transport is stopped.** `stop` sets the
    // playhead to 0, which is a *movement* — without this, pressing Stop would
    // fire every lane that has a note on beat 1.
    if (!state.playing) {
      previous = now;
      fromRest = true;
      return;
    }

    // ⛔⛔ **A SCRUB IS NOT A LOOP POINT.** `seek` writes the playhead backwards
    // with the transport still rolling, so the wrap window below would light
    // every hit from the old position to the end *and* every hit from zero to
    // the new one — the whole kit flashing at once on a drag of the ruler.
    // Nothing between actually sounded, so nothing lights; the note under the
    // new position gets its blink on the next tick, because this leaves
    // `fromRest` set.
    if (state.seekNonce !== before.seekNonce) {
      previous = now;
      fromRest = true;
      return;
    }

    // ⚠ **The loop point is a jump backwards, not a gap.** At the wrap the
    // window is "after `previous`, or at-or-before `now`" — two ranges rather
    // than one — and treating it as an ordinary step would silently skip every
    // hit between the last frame and the end of the bar on every repeat.
    // ⛔⛔ **CLOSED AT THE LOW END ON THE FIRST STEP, OR THE DOWNBEAT IS LOST.**
    // At rest `previous` is 0, so the first tick of playback asked `0 > 0` and a
    // kick on beat 1 did not light — on the *first* bar only, because from the
    // second pass onwards the wrap branch's `at <= now` catches it. One bar in
    // every playback missing its downbeat is exactly the kind of readout fault
    // that reads as the feature being unreliable rather than wrong.
    const from = (at: number) => (fromRest ? at >= previous : at > previous);
    const crossed = (at: number) =>
      now < previous ? at > previous || at <= now : from(at) && at <= now;

    // ⚠ **Muted lanes stay dark.** A pad lighting for something the producer
    // cannot hear is the readout-that-lies failure the pads' own dot exists to
    // prevent — and `PadGrid` already draws `data-audible` from exactly this
    // pair.
    for (const hit of hits) {
      if (crossed(hit.at) && laneAudible(hit.lane, state.mutedLanes, state.soloedLanes)) {
        light(hit.lane);
      }
    }
    previous = now;
    fromRest = false;
  });

  return () => {
    unsubscribe();
    for (const timer of timers.values()) window.clearTimeout(timer);
    timers.clear();
  };
}
