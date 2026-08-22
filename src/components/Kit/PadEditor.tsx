import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RotateCcw, X } from 'lucide-react';

import { invoke } from '../../lib/ipc';
import { useKit, type Adsr, type KitLane, type PadTweaks } from '../../state/kit';
import { outlineOf, VIEW_H, VIEW_W } from '../Explorer/waveform';
import { auditionLane } from '../DrumGrid/audition';
import {
  ENV_H,
  ENV_SPAN_MS,
  ENV_W,
  MIN_DB,
  dbOf,
  envelopePath,
  envelopePoints,
  msOf,
  type EnvelopeCorners,
} from './padEnvelope';
// The one spelling of scientific pitch notation this app has. Reused rather
// than repeated: the piano roll's gutter and this readout must name the same
// note, and MIDI 60 is C4 here for the reasons that file records.
import { pitchName } from '../PianoRoll/geometry';

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
 * The clarity below which a detected root is shown as doubtful (TASK-052).
 *
 * ⚠ **Above `pitch.rs`'s own `MIN_CLARITY` of 0.6**, which is the floor for
 * answering at all. Between the two the detector was willing to name a note and
 * not confident about it, and that gap is exactly what a producer is owed a
 * marker for — below 0.6 they are shown nothing, which is already honest.
 */
const UNSURE_BELOW = 0.75;

/** How big a handle is, in the envelope's own drawing units. */
const GRIP_R = 5;

/**
 * The four envelope handles: which `Adsr` field each carries, and what a drag
 * of it means (TASK-055).
 *
 * ⛔ **A table rather than four near-identical blocks.** Naming the field once
 * per stage is what lets the grip, its readout, its drag and its double-click
 * reset all derive from one entry — four copies differing in one field each is
 * where the third one quietly stops matching its own dial.
 *
 * ⛔⛔ **The drag rule is IN the table**, and the first cut of this left it out:
 * it lived in a separate four-arm `switch` keyed by the same four literals,
 * with nothing making the compiler check the two lists against each other. The
 * piece that list did not cover — which earlier stage's corner to subtract from
 * `x` — is exactly the piece most likely to drift, and a fifth handle would
 * have drawn a grip whose drag silently did nothing.
 *
 * ⛔ **Each stage drags in the axis it lives on.** Attack, decay and release are
 * *durations* and move horizontally; sustain is a *level* and moves vertically,
 * which is why `drag` takes both and each entry ignores the one it does not
 * use. Letting either move both ways would mean a producer adjusting a level
 * also shortened a stage they were not touching.
 *
 * ⚠ **A duration is measured from its own corner, not from the left edge** —
 * decay begins where attack ends, release after the sustain leg — without which
 * every stage would read longer than it is. That is the whole of what `at` is
 * passed for.
 */
const STAGES = [
  {
    key: 'a' as const,
    label: 'padEditor.attack',
    field: 'attackMs' as const,
    drag: (x: number) => msOf(x),
  },
  {
    key: 'd' as const,
    label: 'padEditor.decay',
    field: 'decayMs' as const,
    drag: (x: number, _y: number, at: EnvelopeCorners) => msOf(x - at.a.x),
  },
  {
    key: 's' as const,
    label: 'padEditor.sustain',
    field: 'sustainDb' as const,
    drag: (_x: number, y: number) => dbOf(y),
  },
  {
    key: 'r' as const,
    label: 'padEditor.release',
    field: 'releaseMs' as const,
    drag: (x: number, _y: number, at: EnvelopeCorners) => msOf(x - at.s.x),
  },
] satisfies readonly {
  key: string;
  label: string;
  field: keyof Adsr;
  drag: (x: number, y: number, at: EnvelopeCorners) => number;
}[];

type Stage = (typeof STAGES)[number]['key'];

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

  // ── The envelope's handles and its A/B (TASK-055) ──────────────────────
  const envRef = useRef<SVGSVGElement>(null);
  /** Which handle the pointer has hold of, or `null`. */
  const [dragging, setDragging] = useState<Stage | null>(null);
  /**
   * The shape parked as B, or `null` before anything has been stored.
   *
   * ⚠ **Component state, deliberately not persisted.** B is a scratchpad for
   * the next two minutes; a project that reopened holding a comparison nobody
   * remembers making is clutter with no way to clear it.
   */
  const [held, setHeld] = useState<Adsr | null>(null);
  const [comparing, setComparing] = useState(false);
  /** Whichever of the two the graph is drawing right now. */
  const shownAdsr = comparing && held !== null ? held : tweaks.adsr;
  /**
   * The live envelope and the grabbed handle's box, for the drag to read.
   *
   * ⛔ **Refs rather than deps.** The drag listeners live on `window` for the
   * length of one gesture; depending on `tweaks.adsr` would tear them down and
   * re-add them on every move, because the move's own `edit` replaces it. And
   * the SVG cannot move while a modal editor is open with a pointer captured,
   * so its box is measured once at `pointerdown` instead of forcing a layout
   * flush at pointer rate.
   */
  const latest = useRef(tweaks.adsr);
  const grabbed = useRef<DOMRect | null>(null);
  // ⚠ **In an effect, not in the render body**, which the `react-hooks/refs`
  // rule refuses outright. This keeps the ref in step with edits made from
  // *outside* a drag — the dials below, Reset, Keep B; a drag itself writes the
  // value it just sent, so it never has to wait for this.
  useEffect(() => {
    latest.current = tweaks.adsr;
  }, [tweaks.adsr]);

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

  /**
   * Drag a handle (TASK-055).
   *
   * ⛔ **On `window`, not on the circle.** A 5-unit circle is a few pixels on
   * screen; a drag that only worked while the pointer stayed inside it would be
   * a control nobody could use. Pointer capture keeps the events coming, and
   * this listens where they arrive.
   *
   * ⛔ **Each stage is dragged in the axis it lives on.** Attack, decay and
   * release are *durations* and move horizontally; sustain is a *level* and
   * moves vertically. Letting either move both ways would mean a producer
   * adjusting a level also shortened a stage they were not touching.
   *
   * ⛔ **Measured against the stage's own origin, not the left edge.** Decay
   * starts where attack ends and release starts after the sustain leg, so a
   * handle dragged to an x has to have the stages before it subtracted or every
   * one of them would read as longer than it is.
   */
  useEffect(() => {
    if (dragging === null) return;
    const stage = STAGES.find((one) => one.key === dragging);
    if (stage === undefined) return;

    const move = (event: PointerEvent) => {
      // ⛔ **The box is measured once per DRAG, not once per move.**
      // `getBoundingClientRect` forces a synchronous layout, and a pointer
      // fires up to a thousand times a second; the SVG cannot move or resize
      // while a modal editor is open and a pointer is captured, so re-measuring
      // is a layout flush per frame for a number that cannot have changed.
      const box = grabbed.current;
      if (box === null || box.width === 0 || box.height === 0) return;
      // The SVG scales to the panel, so pointer pixels become drawing units.
      const x = ((event.clientX - box.left) / box.width) * ENV_W;
      const y = ((event.clientY - box.top) / box.height) * ENV_H;

      // ⛔ **The drag rule comes off the STAGES table**, so there is no second
      // four-way list to keep in step with it — see the table for why.
      const adsr = latest.current;
      const next = stage.drag(x, y, envelopePoints(adsr));
      // ⚠ **Skipped when nothing moved.** `msOf`/`dbOf` clamp, so dragging past
      // either end of a stage emits a stream of identical values — and each one
      // would otherwise be a full kit rebuild on the plugin side and a whole
      // panel re-render on this one.
      if (adsr[stage.field] === next) return;
      const moved = { ...adsr, [stage.field]: next };
      // ⛔ **Written here as well as in the effect above.** The effect lands a
      // commit later, so two pointer events inside one frame would both compute
      // their origin from the same stale corner — a decay dragged fast would
      // lag the pointer by however many events the frame swallowed.
      latest.current = moved;
      edit({ adsr: moved });
    };
    const drop = () => setDragging(null);

    window.addEventListener('pointermove', move);
    // ⚠ `pointercancel` too: a touch drag interrupted by the system fires that
    // and never `pointerup`, which would leave the handle stuck to the pointer.
    window.addEventListener('pointerup', drop);
    window.addEventListener('pointercancel', drop);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', drop);
      window.removeEventListener('pointercancel', drop);
    };
    // ⛔ **`dragging` alone, and the live envelope is read through a ref.**
    // Depending on `tweaks.adsr` meant every move — whose own `edit` replaces
    // it — tore down and re-added three window listeners, at pointer rate.
    // ⚠ `edit` is redefined on every render and is not in the deps for the same
    // reason; it closes over `setTweaks` and `entry.lane`, both of which are
    // stable for as long as this editor is open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragging]);

  const laneName = t(`lanes.${entry.lane}`);
  /** Every corner, once — each grip below reads its own out of this. */
  const corners = envelopePoints(tweaks.adsr);

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

      {/* ── The envelope (TASK-164), with handles on it (TASK-055) ─────── */}
      <figure className="pad-editor__env">
        <svg
          ref={envRef}
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
          <path d={envelopePath(shownAdsr)} className="pad-editor__env-line" />
          {/* ⛔ **The handles sit on the line because they are computed from the
              same points it is** — see `envelopePoints`. Two spellings of the
              arithmetic is how a handle ends up a few pixels off the curve it
              is supposed to move. ⚠ Computed ONCE for all four, not once per
              handle — the record carries every corner and each grip reads one.

              ⚠ **Drawn last, so they are above the line and take the pointer.**
              ⚠ **Not rendered while B is showing**: B is a comparison, and a
              handle that moved the value you were comparing against would make
              the A/B meaningless. */}
          {!comparing &&
            STAGES.map((stage) => (
              <circle
                key={stage.key}
                className="pad-editor__env-grip"
                data-stage={stage.key}
                cx={corners[stage.key].x}
                cy={corners[stage.key].y}
                r={GRIP_R}
                // ⛔ **A pointer affordance, not a control** — `aria-hidden`
                // and not focusable. The first cut gave these `role="slider"`
                // and `tabIndex={0}` with no key handling, so a keyboard user
                // could tab to four things announced as sliders that arrow keys
                // did nothing to, which is worse than not reaching them at all.
                // ⚠ **Nothing is lost:** the four labelled `Dial`s below are
                // `<input type="range">` over the same values — draggable,
                // focusable and arrow-key operable for free — so the keyboard
                // path to every stage already exists and is the better one.
                aria-hidden="true"
                onPointerDown={(event) => {
                  // ⚠ Capture, so a drag that leaves the small circle keeps
                  // being delivered — which is most drags.
                  event.currentTarget.setPointerCapture(event.pointerId);
                  // ⚠ Measured here, once, for the whole drag — see `grabbed`.
                  grabbed.current = envRef.current?.getBoundingClientRect() ?? null;
                  setDragging(stage.key);
                }}
                // ⚠ **No `onPointerUp` of its own.** A captured pointer's
                // `pointerup` still bubbles to `window`, where the drag effect
                // is already listening for exactly the length of the gesture,
                // and implicit capture is released by that event anyway — so a
                // handler here would only call `setDragging(null)` twice.
                //
                // ⛔ **Double-click resets THIS stage, not the pad**
                // (TASK-055). The header's Reset clears everything; a producer
                // who has dialled in three stages and wants the fourth back
                // has no other way to get it without retyping the other three.
                onDoubleClick={() => edit({ adsr: { ...tweaks.adsr, [stage.field]: 0 } })}
              />
            ))}
        </svg>
      </figure>

      {/* ⛔ **A/B: hold one shape while you try another** (TASK-055). B is this
          editor's own state and is deliberately NOT persisted — it is a
          scratchpad for the next two minutes, and a project file that reopened
          holding a comparison nobody remembers making is clutter. */}
      <div className="pad-editor__ab">
        <button
          type="button"
          className="btn-ghost"
          onClick={() => setHeld(tweaks.adsr)}
          disabled={comparing}
        >
          {t('padEditor.storeB')}
        </button>
        <button
          type="button"
          className="btn-ghost"
          disabled={held === null}
          aria-pressed={comparing}
          onClick={() => setComparing((was) => !was)}
        >
          {comparing ? t('padEditor.showA') : t('padEditor.showB')}
        </button>
        <button
          type="button"
          className="btn-ghost"
          disabled={held === null}
          // ⛔ **Keeping B is what makes the comparison worth making.** Without
          // it a producer can hear that B was better and has no way to arrive
          // at it except by remembering four numbers.
          onClick={() => {
            if (held === null) return;
            setHeld(tweaks.adsr);
            setComparing(false);
            edit({ adsr: held });
          }}
        >
          {t('padEditor.keepB')}
        </button>
      </div>

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
        {/* ⛔ **What the sample was measured to be in** (TASK-052), beside the
            two dials that correct it. The kit roots the pad here, so a producer
            seeing a note they disagree with knows exactly which control moves
            it — and `clarity` is shown as a doubt marker rather than a number,
            because 0.61 means nothing to anybody and *"unsure"* does. */}
        {entry.root !== null && (
          <p className="pad-editor__root">
            {t('padEditor.detectedRoot', {
              note: pitchName(entry.root.note),
              cents: entry.root.cents > 0 ? `+${entry.root.cents}` : String(entry.root.cents),
            })}
            {entry.root.clarity < UNSURE_BELOW && ` ${t('padEditor.rootUnsure')}`}
          </p>
        )}
        {/* ⛔ **Said, not silently done** (TASK-053A). A producer who draws a
            whole-note chord and hears it stop early is owed the reason: this
            sample has no steady state to loop, so the note ends when the file
            does. ⚠ Only the `false` case earns a sentence — `true` is what they
            already expect, and `null` is a lane where holding means nothing. */}
        {entry.holds === false && (
          <p className="pad-editor__root">{t('padEditor.cannotHold')}</p>
        )}
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
          {peaks.length > 0 && (
            // ⛔⛔ **Mirrored when the pad plays backwards.**
            // `oneshot::load` flips the buffer at decode, so the trim window is
            // measured against the REVERSED audio — while `explorer_waveform`
            // reads the file off disk forwards. Without this, dragging Start to
            // 0.5 shaded the left half of the picture and the engine cut the
            // right half of what was drawn.
            //
            // ⚠ **A transform on the path, not a second request.** The peaks are
            // the same numbers either way round; asking the plugin for a
            // reversed waveform would be a second answer to a question the page
            // can already answer.
            <path
              d={outlineOf(entry.reversed ? [...peaks].reverse() : peaks)}
              className="pad-editor__wave-fill"
            />
          )}
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
