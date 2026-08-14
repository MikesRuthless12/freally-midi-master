import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RotateCcw, X } from 'lucide-react';

import { invoke } from '../../lib/ipc';
import { useKit, type KitLane, type PadTweaks } from '../../state/kit';
import { outlineOf, VIEW_H, VIEW_W } from '../Explorer/waveform';
import { auditionLane } from '../DrumGrid/audition';
import { ENV_H, ENV_SPAN_MS, ENV_W, MIN_DB, envelopePath } from './padEnvelope';

/**
 * The per-pad sound editor (TASK-055A, TASK-164).
 *
 * ⛔⛔ **Mike, 2026-08-11**, with a Drum Monkey screenshot: *"it should have an
 * editor like what's in drum monkey, not just an attack/release, decay, filter,
 * or gain, but like this — with an envelope editor etc."* He had asked first
 * whether the drum lanes had an editor at all — *"Kick, Sub Bass, Rim Shot,
 * etc."* — and the answer then was that the **notes** had one and the **sound**
 * did not.
 *
 * ⛔ **Two things the reference shows are deliberately NOT here, and saying so
 * is the point.** A control that cannot work is worse than a missing one:
 *
 * - **`NOTE`** — which MIDI note a lane answers to is
 *   `engine::midi::gm_drum_note`, a constant that the exporter, the sampler and
 *   the drum grid all agree on, and `every_pitched_lane_gets_a_channel_of_its_own`
 *   is built on it. Making it per-pad is a change to that agreement, not a knob.
 * - **`OUT MAIN / ALT`** — there is one output bus.
 *
 * ⚠ **Not the same task as TASK-161 and neither closes the other.** That is
 * per-*hit* pitch inside the pattern — the 808 climbing semitones across a bar.
 * `semis`/`cents` here are per-*pad*: one offset for the whole lane.
 */

type Peaks = readonly [number, number][];

/**
 * The shortest gap between two auditions while a control is being dragged.
 *
 * TASK-055's own number: *"live re-trigger (≤ 150 ms throttle)"*.
 */
const AUDITION_MS = 150;

/**
 * One labelled number the producer can drag or type into.
 *
 * ⚠ A range **and** a number input rather than a bespoke rotary knob: a range
 * is draggable, focusable and reachable from the keyboard for free, and the
 * number beside it is what makes the value readable — which is the half the
 * reference screenshot actually shows (`−1.83 dB`, `0 st`).
 */
function Dial({
  label,
  value,
  min,
  max,
  step,
  unit,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  onChange: (value: number) => void;
}) {
  return (
    <label className="pad-dial">
      <span className="pad-dial__label">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        aria-label={label}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <output className="pad-dial__value">
        {Number.isInteger(step) ? value.toFixed(0) : value.toFixed(2)} {unit}
      </output>
    </label>
  );
}

/**
 * @param entry the row this is editing. Its `tweaks` are the plugin's own —
 * this component never constructs a `PadTweaks`, so the defaults have exactly
 * one owner. See `state/kit.ts`.
 */
export function PadEditor({ entry, onClose }: { entry: KitLane; onClose: () => void }) {
  const { t } = useTranslation();
  const setTweaks = useKit((s) => s.setTweaks);
  const tweaks = entry.tweaks;
  /**
   * The peaks that arrived, **with the path they were read for**.
   *
   * ⛔ **Paired rather than cleared, and that is not style.** Clearing on a path
   * change means a `setState` in the effect body, which is a cascading render —
   * and the lint rule that names it is right here for a second reason: a reply
   * for the *previous* path can land after the switch, and comparing the path is
   * what makes that impossible to draw. Nothing below sets state outside an
   * async callback.
   */
  const [drawn, setDrawn] = useState<{ path: string | null; peaks: Peaks }>({
    path: null,
    peaks: [],
  });
  const peaks: Peaks = drawn.path === entry.path ? drawn.peaks : [];
  const dialogRef = useRef<HTMLDivElement>(null);
  /** When the pad was last heard, and the timer that will play it once more. */
  const heardAt = useRef(0);
  const trailing = useRef<number>(0);

  /**
   * The waveform behind the trim handles.
   *
   * ⛔ **Only for a pad carrying the producer's own file.** `explorer_waveform`
   * reads a path through the library containment check; a shipped pad's audio
   * is compiled into the binary and has no path at all. Drawing a *generic*
   * waveform for those would be a picture of a sample that is not the one on
   * the pad — the readout-that-lies failure this panel keeps guarding against —
   * so the handles run over a plain track instead and the panel says why.
   */
  useEffect(() => {
    const path = entry.path;
    if (path === null) return;
    let live = true;
    void invoke<{ peaks: Peaks }>('explorer_waveform', { path })
      .then((reply) => {
        if (live) setDrawn({ path, peaks: reply.peaks });
      })
      // ⚠ Swallowed to an empty waveform rather than surfaced: the store's
      // `error` slot is for a failed *edit*, and a sample whose picture could
      // not be drawn still edits perfectly well. The empty track and its note
      // are the honest report.
      .catch(() => {
        if (live) setDrawn({ path, peaks: [] });
      });
    return () => {
      live = false;
    };
  }, [entry.path]);

  // ⚠ Escape closes, and focus lands inside on open — this covers the panel
  // with the keyboard alone, which is the same bar the rest of the rail meets.
  //
  // ⚠ **The trailing audition is cancelled on the way out.** A pad that plays
  // itself 150 ms after the editor closed is a noise with nothing on screen to
  // explain it.
  useEffect(() => {
    dialogRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.clearTimeout(trailing.current);
    };
  }, [onClose]);

  /**
   * Change one field, and hear it.
   *
   * ⛔ **The audition is the point of the feature.** TASK-055A asks that these
   * *"re-audition live as they are changed"*; the plugin rebuilds and republishes
   * the kit on the same call, so triggering the lane straight after plays the
   * pad as it now is. Without this the producer turns a decay knob in silence.
   *
   * ⛔⛔ **Throttled, and that is not a nicety.** A range input fires on every
   * step it passes, so dragging Volume from 0 to −6 dB is a dozen events —
   * a dozen one-shots on top of each other, which is a machine-gun rather than
   * an audition. TASK-055's own entry names the number: *"live re-trigger
   * (≤ 150 ms throttle)"*.
   *
   * ⚠ **The edit itself is NOT throttled — only the sound is.** Dropping edits
   * would make the knob lag the pointer, which is the thing the optimistic write
   * in `setTweaks` exists to prevent. Every change is sent; at most one in
   * `AUDITION_MS` is heard.
   *
   * ⚠ **And there is a trailing audition**, so letting go of a slider always
   * plays the value you stopped on. Without it the last thing heard is wherever
   * the throttle happened to land, which is the one value the producer did not
   * choose.
   */
  const edit = (patch: Partial<PadTweaks>) => {
    void setTweaks(entry.lane, patch).then(() => {
      const now = Date.now();
      window.clearTimeout(trailing.current);
      if (now - heardAt.current >= AUDITION_MS) {
        heardAt.current = now;
        auditionLane(entry.lane);
        return;
      }
      trailing.current = window.setTimeout(() => {
        heardAt.current = Date.now();
        auditionLane(entry.lane);
      }, AUDITION_MS);
    });
  };

  const laneName = t(`lanes.${entry.lane}`);

  return (
    <div
      className="pad-editor"
      role="dialog"
      aria-label={t('padEditor.title', { lane: laneName })}
      tabIndex={-1}
      ref={dialogRef}
    >
      <header className="pad-editor__head">
        <h3>{t('padEditor.title', { lane: laneName })}</h3>
        <button
          type="button"
          className="btn-ghost"
          // ⛔ **Everything at once, in one command.** Resetting field by field
          // would rebuild the kit ten times and let the audio thread hear nine
          // states that were never on screen.
          onClick={() =>
            edit({
              gainDb: 0,
              pan: 0,
              semis: 0,
              cents: 0,
              normalize: false,
              trimStart: 0,
              trimEnd: 1,
              fadeInS: 0,
              fadeOutS: 0,
              adsr: { attackMs: 0, decayMs: 0, sustainDb: 0, releaseMs: 0 },
            })
          }
        >
          <RotateCcw size={12} aria-hidden="true" /> {t('padEditor.reset')}
        </button>
        <button
          type="button"
          className="btn-ghost"
          aria-label={t('padEditor.close')}
          onClick={onClose}
        >
          <X size={14} aria-hidden="true" />
        </button>
      </header>

      {/* ── The envelope (TASK-164) ─────────────────────────────────────── */}
      <figure className="pad-editor__env">
        <svg
          viewBox={`0 0 ${ENV_W} ${ENV_H}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={t('padEditor.envelopeShape', {
            attack: tweaks.adsr.attackMs.toFixed(0),
            decay: tweaks.adsr.decayMs.toFixed(0),
            sustain: tweaks.adsr.sustainDb.toFixed(1),
            release: tweaks.adsr.releaseMs.toFixed(0),
          })}
        >
          <path d={envelopePath(tweaks.adsr)} className="pad-editor__env-line" />
        </svg>
      </figure>

      <div className="pad-editor__dials">
        <Dial
          label={t('padEditor.attack')}
          value={tweaks.adsr.attackMs}
          min={0}
          max={ENV_SPAN_MS}
          step={1}
          unit="ms"
          onChange={(attackMs) => edit({ adsr: { ...tweaks.adsr, attackMs } })}
        />
        <Dial
          label={t('padEditor.decay')}
          value={tweaks.adsr.decayMs}
          min={0}
          max={ENV_SPAN_MS}
          step={1}
          unit="ms"
          onChange={(decayMs) => edit({ adsr: { ...tweaks.adsr, decayMs } })}
        />
        {/* ⚠ **dB, not a 0–1 ratio** — the reference reads `S −36.00 dB`, and a
            ratio control spends most of its travel in the top 6 dB. */}
        <Dial
          label={t('padEditor.sustain')}
          value={tweaks.adsr.sustainDb}
          min={MIN_DB}
          max={0}
          step={0.5}
          unit="dB"
          onChange={(sustainDb) => edit({ adsr: { ...tweaks.adsr, sustainDb } })}
        />
        <Dial
          label={t('padEditor.release')}
          value={tweaks.adsr.releaseMs}
          min={0}
          max={ENV_SPAN_MS}
          step={1}
          unit="ms"
          onChange={(releaseMs) => edit({ adsr: { ...tweaks.adsr, releaseMs } })}
        />
      </div>

      {/* ── Volume, pan, transpose (TASK-164) and tuning (TASK-055A) ────── */}
      <div className="pad-editor__dials">
        <Dial
          label={t('padEditor.volume')}
          value={tweaks.gainDb}
          min={MIN_DB}
          max={12}
          step={0.5}
          unit="dB"
          onChange={(gainDb) => edit({ gainDb })}
        />
        <Dial
          label={t('padEditor.pan')}
          value={tweaks.pan}
          min={-1}
          max={1}
          step={0.01}
          unit=""
          onChange={(pan) => edit({ pan })}
        />
        <Dial
          label={t('padEditor.transpose')}
          value={tweaks.semis}
          min={-24}
          max={24}
          step={1}
          unit="st"
          onChange={(semis) => edit({ semis })}
        />
        <Dial
          label={t('padEditor.cents')}
          value={tweaks.cents}
          min={-100}
          max={100}
          step={1}
          unit="¢"
          onChange={(cents) => edit({ cents })}
        />
      </div>

      <label className="pad-editor__toggle">
        <input
          type="checkbox"
          checked={tweaks.normalize}
          onChange={(event) => edit({ normalize: event.target.checked })}
        />
        {t('padEditor.normalize')}
      </label>

      {/* ── Trim and fades (TASK-055A) ──────────────────────────────────── */}
      <figure className="pad-editor__wave">
        <svg viewBox={`0 0 ${VIEW_W} ${VIEW_H}`} preserveAspectRatio="none" aria-hidden="true">
          {peaks.length > 0 && <path d={outlineOf(peaks)} className="pad-editor__wave-fill" />}
          {/* The trimmed-away ends, shaded rather than hidden: a producer has to
              see what they are cutting to decide where to cut it. */}
          <rect
            x={0}
            y={0}
            width={tweaks.trimStart * VIEW_W}
            height={VIEW_H}
            className="pad-editor__wave-cut"
          />
          <rect
            x={tweaks.trimEnd * VIEW_W}
            y={0}
            width={(1 - tweaks.trimEnd) * VIEW_W}
            height={VIEW_H}
            className="pad-editor__wave-cut"
          />
        </svg>
        {peaks.length === 0 && (
          // ⚠ Said, not left blank. A shipped pad's audio is compiled into the
          // binary and has no path to draw from — the controls below still work
          // on it, and a producer is owed the difference between "no waveform"
          // and "no sample".
          <figcaption className="pad-editor__note">{t('padEditor.noWaveform')}</figcaption>
        )}
      </figure>

      <div className="pad-editor__dials">
        <Dial
          label={t('padEditor.trimStart')}
          value={tweaks.trimStart}
          min={0}
          max={1}
          step={0.01}
          unit=""
          // ⛔ **The window is closed here as well as in Rust.** The plugin
          // clamps whatever arrives — an inverted slice is the shape that once
          // aborted the host — but a slider that lets you drag the start past
          // the end and then silently snaps it back is a control fighting its
          // own user.
          onChange={(trimStart) =>
            edit({ trimStart, trimEnd: Math.max(trimStart, tweaks.trimEnd) })
          }
        />
        <Dial
          label={t('padEditor.trimEnd')}
          value={tweaks.trimEnd}
          min={0}
          max={1}
          step={0.01}
          unit=""
          onChange={(trimEnd) =>
            edit({ trimEnd, trimStart: Math.min(trimEnd, tweaks.trimStart) })
          }
        />
        <Dial
          label={t('padEditor.fadeIn')}
          value={tweaks.fadeInS}
          min={0}
          max={2}
          step={0.01}
          unit="s"
          onChange={(fadeInS) => edit({ fadeInS })}
        />
        <Dial
          label={t('padEditor.fadeOut')}
          value={tweaks.fadeOutS}
          min={0}
          max={2}
          step={0.01}
          unit="s"
          onChange={(fadeOutS) => edit({ fadeOutS })}
        />
      </div>
    </div>
  );
}
