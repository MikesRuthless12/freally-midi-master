import { useMemo, useRef, type PointerEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Pause, Play, Repeat, Rewind, Square } from 'lucide-react';

import { formatSeconds, useExplorer } from '../../state/explorer';
import { MidiPreview } from './MidiPreview';
import { VIEW_H, VIEW_W, outlineOf } from './waveform';

/**
 * The audition player (TASK-132).
 *
 * Mike's eight items, 2026-08-07: the waveform, play/pause, stop-rewinds,
 * click-to-seek, left-arrow reverse, a progress fill, a playhead marker, the
 * time out of the total, and a loop toggle.
 *
 * ⛔ **Six of those are one number.** The marker, the fill, the readout,
 * click-to-seek, reverse and loop all resolve to the read position, which is why
 * this draws from `position` and nothing else — and why the position arrives as
 * one polled value rather than as six channels that could disagree.
 *
 * ⚠ **The progress fill is CSS, not a redraw.** The outline is memoized on the
 * waveform and the played part is the same path clipped by a custom property,
 * so a playhead moving at 30 Hz never rebuilds an 800-point `d` attribute. That
 * is the same reasoning `subscribeToPlayhead` records for the pattern's marker.
 */
export function PreviewPlayer() {
  const { t } = useTranslation();
  const waveform = useExplorer((s) => s.waveform);
  const selected = useExplorer((s) => s.selected);
  const selectedKind = useExplorer((s) => s.selectedKind);
  const position = useExplorer((s) => s.position);
  const play = useExplorer((s) => s.play);
  const pause = useExplorer((s) => s.pause);
  const stop = useExplorer((s) => s.stop);
  const seek = useExplorer((s) => s.seek);
  const toggleLoop = useExplorer((s) => s.toggleLoop);
  const setReverse = useExplorer((s) => s.setReverse);

  const outline = useMemo(
    () => (waveform ? outlineOf(waveform.peaks) : ''),
    // The whole object, because a new waveform is a new object — and comparing
    // on `peaks` alone would miss a same-length sample replacing another.
    [waveform],
  );
  /**
   * The scrub in flight: the strip's box, measured once, and the last x it acted
   * on. `null` between drags.
   *
   * ⛔ **A ref, not state.** It changes on every pointermove and nothing renders
   * from it — the playhead is already driven by the polled `position`, and a
   * second copy could only disagree with the thing actually making the sound.
   *
   * ⚠ **Declared above the two early returns below**, because a hook after a
   * conditional return is a hook that does not run in every render.
   */
  const scrubbing = useRef<{ box: DOMRect; x: number } | null>(null);

  if (selected === null) {
    return <p className="browser__hint preview__idle">{t('explorer.pickAFile')}</p>;
  }

  // ⛔⛔ **A `.mid` gets its own panel, not this one.** TASK-058's rule: two
  // kinds, two sets of affordances. A MIDI file has no waveform to draw, no PCM
  // to seek through and no reverse to play — every control below would be one
  // that can only fail on it. What it *does* have is parts, which is what
  // `MidiPreview` shows.
  //
  // ⚠ **Asked of `selectedKind`, not of `midiSplit`.** The split arrives after
  // the click and may never arrive at all, so keying on it drew this transport
  // over a MIDI file for as long as that took.
  if (selectedKind === 'midi') return <MidiPreview />;

  // ⚠ **The waveform's own length wins until the poll has answered.** `total`
  // starts at 0, and dividing by it would put the playhead at the far end of the
  // panel for the first frame after every click.
  const total = position.total > 0 ? position.total : (waveform?.seconds ?? 0);
  const fraction = total > 0 ? Math.min(1, Math.max(0, position.seconds / total)) : 0;

  const secondsAt = (clientX: number, box: DOMRect): number | null => {
    if (total <= 0 || box.width <= 0) return null;
    const at = ((clientX - box.left) / box.width) * total;
    return Math.min(total, Math.max(0, at));
  };

  const seekTo = (event: PointerEvent<HTMLDivElement>) => {
    // ⚠ **The box is measured ONCE, here, not per pointermove.** Pointer capture
    // is set below so it cannot move under the drag, and this element carries an
    // inline `--preview-progress` that React rewrites on every position poll —
    // so measuring in the move handler would be a layout read after a style
    // write, every frame. The grid's marquee records the same rule.
    const box = event.currentTarget.getBoundingClientRect();
    const at = secondsAt(event.clientX, box);
    if (at === null) return;
    scrubbing.current = { box, x: event.clientX };
    // ⛔ **Seek *and* play** — Mike asked to "click anywhere in the sample and
    // play from there", which is one gesture. Seeking silently and waiting for a
    // second press on Play is not what he described.
    void seek(at).then(() => play());
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  /**
   * Drag along the waveform and the audio follows the finger (TASK-058B).
   *
   * ⛔ **It seeks without re-playing.** The press already started playback, and
   * calling `play()` again on every pointermove would retrigger the voice ~60
   * times a second — a stutter rather than the tape-rub this gesture is for.
   *
   * ⛔⛔ **Throttled in PIXELS, not seconds.** A "hundredth of a second" reads
   * like a fine threshold and is not one: its pixel size is `0.01 / total *
   * width`, so it is ~4px on a 1.5s one-shot and ~0.2px on a 30s loop — no
   * throttle at all on exactly the long files where a drag produces the most
   * moves. Each accepted move costs a `seek` *and* a forced extra position poll
   * across the bridge, so the units have to be the ones the pointer moves in.
   */
  const scrub = (event: PointerEvent<HTMLDivElement>) => {
    const live = scrubbing.current;
    if (!live || Math.abs(event.clientX - live.x) < 1) return;
    const at = secondsAt(event.clientX, live.box);
    if (at === null) return;
    live.x = event.clientX;
    void seek(at);
  };

  const endScrub = (event: PointerEvent<HTMLDivElement>) => {
    scrubbing.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  return (
    <div className="preview">
      <div
        className="preview__wave"
        style={
          { '--preview-progress': `${(fraction * 100).toFixed(3)}%` } as React.CSSProperties
        }
        onPointerDown={seekTo}
        onPointerMove={scrub}
        onPointerUp={endScrub}
        onPointerCancel={endScrub}
        role="presentation"
        aria-label={waveform ? t('explorer.waveformOf', { name: waveform.name }) : undefined}
      >
        {outline === '' ? (
          <span className="preview__decoding">{t('explorer.decoding')}</span>
        ) : (
          <>
            <svg
              className="preview__svg"
              viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <path className="preview__outline" d={outline} />
            </svg>
            {/* ⛔ **The same outline again, clipped.** Mike: *"the waveform's
                back colour differs for the part already played."* Two layers is
                what makes that a CSS clip rather than a second path rebuilt on
                every frame. */}
            <svg
              className="preview__svg preview__svg--played"
              viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <path className="preview__outline preview__outline--played" d={outline} />
            </svg>
            <span className="preview__head" aria-hidden="true" />
          </>
        )}
      </div>

      <div className="preview__bar">
        <button
          type="button"
          className="btn-ghost preview__button"
          // ⚠ **`transport.*`, not a second set of explorer strings.** Play,
          // Pause, Stop and Loop already exist translated into all eighteen
          // locales for the pattern transport, and they mean the same thing
          // here. Two spellings of one word is how two panels come to disagree
          // about it in fifteen languages nobody on this project reads.
          aria-label={position.playing ? t('transport.pause') : t('transport.play')}
          onClick={() => void (position.playing ? pause() : play())}
        >
          {position.playing ? (
            <Pause size={13} aria-hidden="true" />
          ) : (
            <Play size={13} aria-hidden="true" />
          )}
        </button>

        {/* ⛔ Rewinds to the start; pause holds. Two buttons because they are
            two behaviours, which is how Mike named them. */}
        <button
          type="button"
          className="btn-ghost preview__button"
          aria-label={t('transport.stop')}
          onClick={() => void stop()}
        >
          <Square size={12} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="btn-ghost btn-toggle preview__button"
          aria-label={t('explorer.reverse')}
          aria-pressed={position.reverse}
          data-on={position.reverse}
          onClick={() => void setReverse(!position.reverse).then(() => play())}
        >
          <Rewind size={13} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="btn-ghost btn-toggle preview__button"
          aria-label={t('transport.loop')}
          aria-pressed={position.looping}
          data-on={position.looping}
          onClick={() => void toggleLoop()}
        >
          <Repeat size={13} aria-hidden="true" />
        </button>

        {/* "Playback time out of total time", verbatim. */}
        <span className="preview__time">
          {formatSeconds(position.seconds)} / {formatSeconds(total)}
        </span>
      </div>
    </div>
  );
}
