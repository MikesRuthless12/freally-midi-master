import { useMemo, type PointerEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Pause, Play, Repeat, Rewind, Square } from 'lucide-react';

import { formatSeconds, useExplorer } from '../../state/explorer';
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

  if (selected === null) {
    return <p className="browser__hint preview__idle">{t('explorer.pickAFile')}</p>;
  }

  // ⚠ **The waveform's own length wins until the poll has answered.** `total`
  // starts at 0, and dividing by it would put the playhead at the far end of the
  // panel for the first frame after every click.
  const total = position.total > 0 ? position.total : (waveform?.seconds ?? 0);
  const fraction = total > 0 ? Math.min(1, Math.max(0, position.seconds / total)) : 0;

  const seekTo = (event: PointerEvent<HTMLDivElement>) => {
    if (total <= 0) return;
    const box = event.currentTarget.getBoundingClientRect();
    if (box.width <= 0) return;
    const at = ((event.clientX - box.left) / box.width) * total;
    // ⛔ **Seek *and* play** — Mike asked to "click anywhere in the sample and
    // play from there", which is one gesture. Seeking silently and waiting for a
    // second press on Play is not what he described.
    void seek(Math.min(total, Math.max(0, at))).then(() => play());
  };

  return (
    <div className="preview">
      <div
        className="preview__wave"
        style={
          { '--preview-progress': `${(fraction * 100).toFixed(3)}%` } as React.CSSProperties
        }
        onPointerDown={seekTo}
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
