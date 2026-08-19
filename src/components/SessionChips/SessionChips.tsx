import { Link2, Unlink, Volume2, VolumeX, X } from 'lucide-react';
import {
  useCallback,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from 'react';
import { useTranslation } from 'react-i18next';

import { Combo } from '../Combo/Combo';
import { useSession, useActivePattern } from '../../state/session';
import { useSong } from '../../state/song';
import type { Scale } from '../../lib/ipc-types';
import {
  BPM_MAX,
  BPM_MIN,
  KEY_NAMES,
  SCALES,
  SWING_MAX,
  SWING_MIN,
  decimalOnly,
  digitsOnly,
  keyName,
  prettyKey,
} from './values';
import './SessionChips.css';

/**
 * The character index a click at `clientX` lands on inside a number field.
 *
 * ⛔⛔ **Because suppressing the press to stop drag-selection also removed
 * click-to-place-caret.** A review raised it and it shipped as a documented
 * "accepted cost": you could no longer click between the 4 and the 0 of a pinned
 * `140` and type. Measuring the text is what pays that back — the box is a drag
 * box first, but it is still a text field.
 *
 * ⚠ **Measured with the field's own font**, read off `getComputedStyle`, because
 * these chips are `--font-mono` at `--text-small` and a default-font measurement
 * lands the caret a character or two out on a four-character value.
 *
 * ⚠ **Honours `text-align`.** The swing box is right-aligned, so its text starts
 * at the *end* of the content box rather than the start; measuring from the left
 * would put every caret in the wrong place on that one field.
 */
let ruler: CanvasRenderingContext2D | null = null;

function caretFromX(box: HTMLInputElement, clientX: number): number | null {
  const text = box.value;
  if (!text) return 0;

  ruler ??= document.createElement('canvas').getContext('2d');
  const style = getComputedStyle(box);
  // ⚠ **`null`, not a guess.** No canvas — jsdom, a webview with it disabled,
  // a fingerprinting blocker — means no measurement to make, and the caller
  // leaves the caret entirely alone. Returning `text.length` here forced it to
  // the end of the field on every click, which is the regression this exists to
  // pay back rather than reintroduce.
  if (!ruler) return null;
  ruler.font = style.font || `${style.fontSize} ${style.fontFamily}`;

  const box_ = box.getBoundingClientRect();
  const padLeft = parseFloat(style.paddingLeft) || 0;
  const padRight = parseFloat(style.paddingRight) || 0;
  const inner = box_.width - padLeft - padRight;
  const width = ruler.measureText(text).width;
  // Where the text itself begins, which is not the content box's left edge when
  // the field is right-aligned.
  const from = style.textAlign === 'right' ? padLeft + Math.max(0, inner - width) : padLeft;
  const x = clientX - box_.left - from;

  let best = 0;
  let closest = Infinity;
  for (let at = 0; at <= text.length; at += 1) {
    const distance = Math.abs(ruler.measureText(text.slice(0, at)).width - x);
    if (distance < closest) {
      closest = distance;
      best = at;
    }
  }
  return best;
}

/**
 * Turn a number field into a drag box (2026-08-16).
 *
 * ⛔⛔ **Mike asked for the DAW idiom on both numbers** — *"ensure that this is a
 * drag up and down numeric box"*, then *"ensure that the Swing is the same type
 * of drag up and down as the BPM is"*. One hook rather than two copies: the
 * tempo and the swing differ only in what a step is worth and where it stops,
 * and two copies would be two places for the gesture to drift apart.
 *
 * ⛔ **It starts from the number you can SEE, pinned or not.** An unpinned box
 * shows the artist's value as a placeholder, and a drag that started from 0 — or
 * from some invented default — would jump the value the instant it was touched.
 * Grabbing 140 and pulling one step must give 141. Nothing to show means nothing
 * to grab, so a box reading `—` does not scrub at all rather than inventing a
 * start.
 *
 * ⛔ **`min`/`max` bound the drag, not just the typing.** *"Only have the swing
 * go up and down so much"* — swing runs 0.5 to 0.75, so a drag that ran on past
 * the ceiling would pin a number the engine refuses and leave the chip showing
 * a value nothing generates from.
 *
 * ⚠ **`preventDefault` on the press is what stops the caret and the text
 * selection** — a drag across a focused input otherwise highlights its own
 * digits. It deliberately does *not* stop `click`: the `<label>` still focuses
 * the box, which is how it stays typeable. `onClick` below is what tells the two
 * apart, and it must only fire for a drag that actually moved.
 *
 * ⚠ **It costs click-to-place-caret, and that is paid back rather than
 * accepted.** Suppressing the compatibility mouse events also removes the
 * browser's own caret positioning, which a review raised. `caretFromX` measures
 * the text and `pointerup` places the caret itself, so clicking between the 4
 * and the 0 of a pinned `140` still works — but only for a press that never
 * scrubbed, because after a real drag the click is swallowed anyway.
 *
 * ⚠ **Measured from where the drag STARTED** rather than by accumulating
 * per-move deltas, for the reason `RailResizer` gives — a dropped frame would
 * otherwise leave the value permanently offset from the cursor.
 *
 * ⚠ **Rounded to the step's own precision.** `0.54 + 3 * 0.01` is
 * `0.5700000000000001` in binary floating point, and that is what would be
 * pinned, saved and shown.
 *
 * ⚠ **`useCallback`, and not for the render cost.** `react-hooks/refs` refuses a
 * ref read from a plain helper built during render — it cannot tell that the
 * closure only ever runs from an event. Deferring them says so.
 */
function useDragBox(
  field: 'bpm' | 'swing',
  shown: number | null,
  { step, min, max }: { step: number; min: number; max: number },
) {
  const setPin = useSession((s) => s.setPin);
  /**
   * Where the drag started and the number it started from, and whether it ever
   * moved.
   *
   * ⚠ Refs rather than state, for the reason `RailResizer` gives: a render per
   * pointer move is the thing being avoided, and `null` between drags means a
   * press that never moved commits nothing.
   */
  const scrub = useRef<{ y: number; from: number; wrote: number | null } | null>(null);
  const scrubbed = useRef(false);

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLInputElement>) => {
      if (event.button !== 0 || shown === null) return;
      event.preventDefault();
      event.currentTarget.setPointerCapture(event.pointerId);
      // ⚠ `wrote: null`, not `shown` — "nothing written yet". Seeding it with the
      // starting number would make a drag that returns to its origin compare
      // equal and skip the write, which is exactly the one-way-at-the-origin
      // bug the `steps === 0` rule below exists to fix.
      scrub.current = { y: event.clientY, from: shown, wrote: null };
      // ⚠ **Armed here, not only cleared by the click.** A drag that ends off the
      // label may never produce a `click`, and a flag left set would swallow the
      // *next* press — the one that was a real click.
      scrubbed.current = false;
    },
    [shown],
  );

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLInputElement>) => {
      const start = scrub.current;
      if (!start || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
      // 3px per step: the tempo's whole musical range is then a comfortable
      // forearm rather than a mouse mat, and the swing's quarter is 75px.
      const steps = Math.round((start.y - event.clientY) / 3);
      // ⛔ **`steps === 0` is "back where I started", not "nothing happened".**
      // The value is absolute — `start.from + steps * step` — so returning early
      // on zero made the gesture one-way at the origin: pull the tempo from 140
      // to 141, drag back to exactly where you grabbed it, and 141 stayed
      // pinned. A press that has not yet moved still commits nothing, which is
      // what `scrubbed` distinguishes.
      if (steps === 0 && !scrubbed.current) return;
      scrubbed.current = true;
      const value = Number((start.from + steps * step).toFixed(step < 1 ? 2 : 0));
      const pinned = Math.min(max, Math.max(min, value));
      // ⛔ **Only when the number actually moves.** `setPin` publishes a fresh
      // `pins` object, which four components select by reference — including
      // `CenterStage`, the stage that hosts the roll and the grid. Pointer moves
      // arrive per animation frame while a step is 3px, so a drag slow enough to
      // land on an exact BPM produces more frames than values; and past either
      // end of the range *every* frame re-pins the same clamped number for as
      // long as the button is held. Swing's whole range is 75px of travel, so
      // that is not a corner case.
      if (pinned === start.wrote) return;
      start.wrote = pinned;
      setPin(field, pinned);
    },
    [field, max, min, setPin, step],
  );

  // ⚠ `pointercancel` as well as `pointerup`: a drag the host interrupts must
  // not leave the box scrubbing against a pointer that has gone.
  const onPointerUp = useCallback((event: PointerEvent<HTMLInputElement>) => {
    // ⛔ **Put the caret where the click landed** — see [caretFromX]. Only for a
    // press that never scrubbed: a finished drag has its click swallowed, and
    // moving the caret there would fight the gesture that just ended.
    if (!scrubbed.current) {
      const box = event.currentTarget;
      const at = caretFromX(box, event.clientX);
      // ⚠ After focus, not before: focusing an input moves the caret itself, so
      // setting the range first would be undone a moment later.
      box.focus();
      if (at !== null) box.setSelectionRange(at, at);
    }
    scrub.current = null;
  }, []);

  // ⛔ **Cancel also disarms the click-swallower.** A cancelled pointer produces
  // no trailing `click`, so `scrubbed` stayed true and the *next* real click on
  // the chip was swallowed — the label's focus-forwarding cancelled, and the
  // field silently refusing to take a caret until you clicked twice. `pointerup`
  // deliberately leaves it set, because there the click is still coming and is
  // the one that must be suppressed.
  const onPointerCancel = useCallback(() => {
    scrub.current = null;
    scrubbed.current = false;
  }, []);

  const onClick = useCallback((event: MouseEvent<HTMLLabelElement>) => {
    if (!scrubbed.current) return;
    scrubbed.current = false;
    event.preventDefault();
  }, []);

  // ⛔ **A drag-only control cannot be reached from a keyboard**, which is the
  // rule `RailResizer` states for the rail handle — and it is also a slider for
  // exactly that reason. Typing is a keyboard path here, but it *replaces* the
  // value: there was no way to ask for one more BPM without retyping all three
  // digits. Up/Down move one step, Page moves ten, and both clamp where the drag
  // clamps.
  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      const by =
        event.key === 'ArrowUp'
          ? 1
          : event.key === 'ArrowDown'
            ? -1
            : event.key === 'PageUp'
              ? 10
              : event.key === 'PageDown'
                ? -10
                : 0;
      if (by === 0 || shown === null) return;
      // ⚠ The browser would otherwise scroll the rail on Page, and move the
      // caret on Arrow — both fight the nudge.
      event.preventDefault();
      const value = Number((shown + by * step).toFixed(step < 1 ? 2 : 0));
      setPin(field, Math.min(max, Math.max(min, value)));
    },
    [field, max, min, setPin, shown, step],
  );

  return {
    /** For the chip, so a finished drag does not also read as a click. */
    chip: { onClick },
    /**
     * For the input itself.
     *
     * ⛔ **`role="spinbutton"`, because `aria-value*` is not allowed on a
     * textbox and was being ignored.** The first cut put the three attributes on
     * an `<input type="text">` and claimed the range was announced; ARIA
     * forbids them on the implicit `textbox` role, so every screen reader
     * dropped them and an axe pass would flag `aria-allowed-attr`. Caught by
     * review. `spinbutton` is what the control actually is now — a value with
     * bounds that Up/Down and Page move — and it is the same treatment
     * `RailResizer` gives the rail handle.
     *
     * ⚠ `aria-valuetext` as well as `aria-valuenow`, so an unpinned box
     * announces the artist's number rather than a bare figure with no context.
     */
    field: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel,
      onKeyDown,
      role: 'spinbutton',
      'aria-valuenow': shown ?? undefined,
      'aria-valuemin': min,
      'aria-valuemax': max,
    },
  };
}

/**
 * The session, shown and editable (FR-002).
 *
 * Every chip is empty until it is pinned, and empty means *the artist decides*
 * — the same contract the seed box has, and the same one `SessionOverrides`
 * has in the engine. The placeholder shows what the artist asks for, so the
 * difference between "140 because I said so" and "140 because trap says so" is
 * visible rather than inferred.
 *
 * Key and scale have no placeholder before a generation, because a seed picks
 * them: the artist offers a list, and which one it lands on is not knowable
 * until Generate is pressed. Once it has been, the chip says which it chose.
 */
export function SessionChips() {
  const { t } = useTranslation();
  const selectedId = useSession((s) => s.selectedId);
  const defaults = useSession((s) => s.defaults);
  const active = useActivePattern();
  const patterns = useSession((s) => s.patterns);
  const song = useSong((s) => s.song);
  // ⛔ **The readout must not blank when the *tab* has nothing.** It used to be
  // the active tab's clip alone, so clicking Melody before generating it — or
  // opening Song, which is not a part at all — emptied the key, scale and mood
  // chips even though the session plainly had all three. The five parts share a
  // seed, so any loaded one reports the session's key; the arrangement reports
  // it when no part is loaded. Falling back is truthful rather than a guess.
  const pattern =
    active ??
    Object.values(patterns)[0] ??
    (song ? { keyRoot: song.keyRoot, scale: song.scale, mood: null } : null);
  const pins = useSession((s) => s.pins);
  const setPin = useSession((s) => s.setPin);
  const hostTempo = useSession((s) => s.hostTempo);
  const autoSync = useSession((s) => s.autoSync);
  const setAutoSync = useSession((s) => s.setAutoSync);
  const mood = useSession((s) => s.mood);
  const setMood = useSession((s) => s.setMood);
  const complexity = useSession((s) => s.complexity);
  const setComplexity = useSession((s) => s.setComplexity);
  const heldLean = useSession((s) => s.lean);
  const base = useSession((s) => s.base);
  const setBase = useSession((s) => s.setBase);
  const roster = useSession((s) => s.roster);
  const audioEnabled = useSession((s) => s.audioEnabled);
  const setAudioEnabled = useSession((s) => s.setAudioEnabled);

  // ⛔ **Above the early return, because the drag boxes below are hooks.** These
  // read nothing but store values, so computing them for an empty panel costs a
  // subtraction — and the alternative is two hooks that cannot be called.
  // ⛔ Three states, not two (TASK-P15). The pin distinguishes "mine" from
  // "not mine"; `autoSync` distinguishes the two kinds of "not mine" — the
  // DAW's tempo and the artist's own. Before the toggle existed the artist's
  // was unreachable in a host, and a chip that showed the host's number while
  // generating at the artist's would be the readout lying either way.
  const synced = pins.bpm === null && hostTempo !== null && autoSync;
  // ⚠ **The artist's number and the shown number are different things**, and the
  // drag needs the second. The placeholder is what the box offers while nothing
  // is pinned; `shownTempo` is what a producer sees there either way, which is
  // what their finger is grabbing when they pull on it.
  const artistTempo = synced
    ? Math.round(hostTempo)
    : defaults
      ? Math.round(defaults.bpm)
      : null;
  const tempoPlaceholder = artistTempo === null ? '—' : String(artistTempo);
  const shownTempo = pins.bpm ?? artistTempo;
  const shownSwing = pins.swing ?? defaults?.swing.amount ?? null;

  // One step is one BPM; one step is a hundredth of swing. See `useDragBox`.
  const tempoDrag = useDragBox('bpm', shownTempo, { step: 1, min: BPM_MIN, max: BPM_MAX });
  const swingDrag = useDragBox('swing', shownSwing, {
    step: 0.01,
    min: SWING_MIN,
    max: SWING_MAX,
  });

  if (!selectedId) {
    return <p className="session__empty">{t('session.pickArtist')}</p>;
  }

  // Absent rather than empty for a style with no `modes` block, which is most
  // of them — the field is skipped on the wire when there is nothing to send.
  const moods = defaults?.moods ?? [];

  /**
   * The genres this artist is listed under, for the base chip (TASK-158C).
   *
   * ⛔ **`relatedGenres`, because that is exactly what `cross-filter.ts`
   * filters the rail on.** Offering a different list here would be a chip that
   * disagreed with the roster about which genres this artist works in, which is
   * the readout-that-lies failure the whole task is closing — arriving through
   * the fix.
   *
   * ⚠ **Named from the roster rather than shown as an id.** `boom-bap` is a key,
   * not a label; the genre's own entry is what carries the name a producer
   * reads. An id that resolves to nothing is dropped — the plugin already drops
   * dangling `relatedGenres` from the roster, so this only ever loses one the
   * rail is not offering either.
   *
   * ⛔ **Empty for a genre, and that is a decision rather than an accident of
   * the data.** 36 of the 56 shipped genres carry `relatedGenres` too, so
   * without this the chip would appear over Trap offering to generate "Trap, in
   * Drill". `resolve_over` would answer something for that, but the feature is
   * *"an artist generating in every genre they work in"* — a genre generating
   * in another genre is a control whose meaning nobody asked for and nobody
   * could predict.
   */
  const own = roster.find((entry) => entry.id === selectedId);
  const relatedGenres = (own?.type === 'genre' ? [] : (own?.relatedGenres ?? [])).flatMap(
    (id) => {
      const genre = roster.find((entry) => entry.id === id);
      return genre ? [{ id, name: genre.name }] : [];
    },
  );

  /** What the artist chose last time, for the "leave it to them" option. */
  const chose = (value: string | null) =>
    value === null ? t('session.artistChoice') : t('session.artistPicks', { value });

  // ⛔ **`complexity` decides which side is shown whenever it names one**, and
  // the remembered lean is consulted only while As Written is on. Reading the
  // memory unconditionally would let an undo — which restores `complexity`
  // without going through `setComplexity` — leave the knob on Simple over a
  // session that is about to generate Complex.
  const asWritten = complexity === 'authored';
  const lean = asWritten ? heldLean : complexity;

  return (
    <div className="readouts session">
      {/* ⚠ The tempo and the DAW switch share a line, and the switch takes the
          slack so the row ends flush with the chips below it. */}
      <div className="session__row">
        <label
          className="chip chip--mono session__chip"
          data-synced={synced || undefined}
          {...tempoDrag.chip}
        >
          <span className="session__label">{t('readouts.bpm')}</span>
          <input
            className="session__number"
            // Text, not `number`. A number input accepts `e`, `E`, `+`, `-` and
            // `.` for scientific notation — "1e5" is a legal value that arrives
            // as 100000 — and when it holds something the browser calls invalid
            // it reports an empty value, which reads here as "unpinned". Digits
            // are filtered on the way in instead, so nothing else can be typed.
            type="text"
            inputMode="numeric"
            // No `maxLength`: the browser applies it to the raw keystrokes,
            // *before* the filter below runs, so typing "12e5" fills up on "12e"
            // and the 5 is dropped entirely. Filter first, then limit — three
            // digits, because the ceiling is 999.
            value={pins.bpm ?? ''}
            placeholder={tempoPlaceholder}
            title={
              synced
                ? t('session.hostSynced')
                : hostTempo !== null && !autoSync
                  ? t('session.autoSyncOff')
                  : undefined
            }
            onChange={(e) => {
              const digits = digitsOnly(e.target.value).slice(0, 3);
              setPin('bpm', digits === '' ? null : Number(digits));
            }}
            // Clamped when the field is left rather than on each keystroke, or
            // typing "5" on the way to "50" would be corrected under the cursor.
            // Anything that generates takes focus away first, so the number shown
            // is always the number the engine will use.
            onBlur={(e) => {
              const digits = digitsOnly(e.target.value);
              if (digits === '') return;
              setPin('bpm', Math.min(BPM_MAX, Math.max(BPM_MIN, Number(digits))));
            }}
            // Drag it up and down to set the tempo — one step is one BPM.
            {...tempoDrag.field}
          />
          <Unpin field="bpm" pinned={pins.bpm !== null} />
        </label>

        {/* ⛔ Only shown inside a host, because there is nothing to sync to
            otherwise — the standalone has no project. A toggle that was present
            and inert would be a control that can only do nothing, which is the
            rule the factory-preset delete button follows too. */}
        {hostTempo !== null && (
          <button
            type="button"
            className="chip session__sync"
            role="switch"
            aria-checked={autoSync}
            title={autoSync ? t('session.hostSynced') : t('session.autoSyncOff')}
            onClick={() => setAutoSync(!autoSync)}
          >
            {autoSync ? (
              <Link2 size={12} aria-hidden="true" />
            ) : (
              <Unlink size={12} aria-hidden="true" />
            )}
            <span className="session__label">{t('session.autoSync')}</span>
          </button>
        )}
      </div>

      {/* ⚠ **Audio and As Written share the next line** — Mike, 2026-08-16:
          *"ensure that the As Written toggle is to the right of the Audio on/off
          button toggle"*. As Written takes the slack, so this row ends flush
          with the tempo row above it. */}
      <div className="session__row">
        {/* ⛔ Always offered, unlike the DAW-sync switch above it. That one is
            about a host that may not exist; this is about the plugin's own
            sound, which it makes in a DAW and in the standalone alike. MIDI-only
            is a first-class mode — it is what the plugin did before it had a
            sampler, and a producer routing into their own drums needs it in one
            click. */}
        <button
          type="button"
          className="chip session__sync"
          role="switch"
          aria-checked={audioEnabled}
          title={audioEnabled ? t('session.audioOn') : t('session.audioOff')}
          onClick={() => setAudioEnabled(!audioEnabled)}
        >
          {audioEnabled ? (
            <Volume2 size={12} aria-hidden="true" />
          ) : (
            <VolumeX size={12} aria-hidden="true" />
          )}
          <span className="session__label">{t('session.audio')}</span>
        </button>

        {/* ⚠ **The switch that takes the Simple/Complex one below away.** On is
            the model as authored; off hands the producer back the side they were
            last on, which is what `lean` is for. */}
        {/* ⚠ `data-lit` so the word lights when the switch is on, the same way
            the pair below lights its active side. Without it this box was the
            only one whose label stayed dim in both states — leaving a 26×14px
            knob as the sole indicator for the control that decides the default
            generation mode. */}
        <span
          className="chip chip--mono session__switchbox"
          data-lit={asWritten ? 'start' : undefined}
        >
          <span className="session__side">{t('session.complexity_authored')}</span>
          <button
            type="button"
            className="session__switch"
            role="switch"
            aria-checked={asWritten}
            aria-label={t('session.complexity_authored')}
            onClick={() => setComplexity(asWritten ? lean : 'authored')}
          >
            <span className="session__thumb" aria-hidden="true" />
          </button>
        </span>
      </div>

      {/* ⛔⛔ **Simple / Complex, over all four melodic generators at once**
          (TASK-125). Mike asked for one control that moves the chords, the
          melody, the countermelody and the bassline together between a plain
          reading of the style and a busy one.

          ⛔ **Three engine states, TWO switches** (2026-08-16). This was one
          three-button group — `Plain · As written · Busy` — and Mike asked for
          the shape it has now, by name: *"it should literally just be a toggle
          with the label Simple/Complex, and if you want it to write it as
          written, then there should be a separate toggle switch for As Written
          … and for the Simple/Complex toggle to be disabled"*, then *"an actual
          toggle switch button, not clicking the text"* and *"On/Off toggle
          switch"*. So the model as written is no longer a third position a
          producer has to notice is the middle one — it is a switch that visibly
          takes the other control away while it is on.

          ⛔ **`authored` is still the state the app opens in**, unchanged: it
          generates byte-for-byte what the app did before this existed, so every
          saved seed still rebuilds its own beat. Splitting the control moved
          where the producer clicks; it did not move the default.

          ⚠ **It leans; it does not override.** *"A rage vamp made busy is no
          longer rage"* — the engine only biases choices the model already
          offered, so a lane that authors one value is unmoved. That is why this
          is a two-position switch rather than a slider: a slider implies a
          continuum the models do not all have.

          ⚠ **The words sit either side of the knob rather than inside it.** The
          switch is the only thing that takes a click — that is the correction
          above, and it is why the labels are plain `<span>`s. `aria-label`
          carries the name the group used to have. */}
      <span
        className="chip chip--mono session__switchbox"
        data-lit={lean === 'complex' ? 'end' : 'start'}
        data-held={asWritten || undefined}
      >
        <span className="session__side">{t('session.complexity_simple')}</span>
        <button
          type="button"
          className="session__switch"
          role="switch"
          aria-checked={lean === 'complex'}
          aria-label={t('session.complexity')}
          // ⛔ **Disabled rather than hidden while As Written holds it.** A
          // control that vanished would leave the producer looking for the one
          // they just used; greyed out with the knob still on the side it was
          // on says what is holding it and what turning As Written off returns
          // them to.
          disabled={asWritten}
          title={t(`session.complexity_${lean}`)}
          onClick={() => setComplexity(lean === 'complex' ? 'simple' : 'complex')}
        >
          <span className="session__thumb" aria-hidden="true" />
        </button>
        <span className="session__side">{t('session.complexity_complex')}</span>
      </span>

      {/* ⛔⛔ **Not native `<select>`s** (TASK-057). A `<select>` popup inside
          WebView2 is drawn by the OS, against the *window* rather than the
          field and at OS scale — Mike screenshotted it. The scale chip below
          offers **41** scales, which is the list length that produced the
          screenshot; it was the strongest case in the app for this change and
          it was missing from the "six left" count entirely.
          ⚠ `<div>` rather than `<label>`: a `<label>` wrapping a combobox
          refocuses the input on every click inside it, including on the arrow
          whose whole job is to toggle the list. `Combo`'s `label` carries the
          accessible name the `<label>` used to give, so the name is unchanged. */}
      <div className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.key')}</span>
        <Combo
          label={t('readouts.key')}
          // ⛔ First, and it is the default: absence means "the artist chooses".
          // It is a real option rather than an empty field, because `Combo`'s
          // contract is that you cannot end up with nothing selected — and here
          // "nothing pinned" is itself a choice worth being able to make again.
          options={[
            { id: '', name: chose(pattern ? keyName(pattern.keyRoot) : null) },
            ...KEY_NAMES.map((name, pitchClass) => ({ id: String(pitchClass), name })),
          ]}
          value={pins.keyRoot === null ? '' : String(pins.keyRoot)}
          onChange={(id) => setPin('keyRoot', id === '' ? null : Number(id))}
        />
      </div>

      <div className="chip chip--mono session__chip">
        <span className="session__label">{t('readouts.scale')}</span>
        <Combo
          label={t('readouts.scale')}
          options={[
            { id: '', name: chose(pattern ? t(`scales.${pattern.scale}`) : null) },
            ...SCALES.map((scale) => ({ id: scale, name: t(`scales.${scale}`) })),
          ]}
          value={pins.scale ?? ''}
          onChange={(id) => setPin('scale', id === '' ? null : (id as Scale))}
        />
      </div>

      {/* ⛔⛔ **"Drake, but in R&B"** (TASK-158C). The roster lists an artist
          under every genre in their `relatedGenres` — 529 of 534 models name
          one they do not `extend` — and Generate has always answered the one
          they do. This chip is what makes the rail's claim true.

          ⛔ **A pin of its own rather than the genre combobox changing
          meaning.** That box and the roster box both write `selectedId`; making
          one of them mean something else when an artist is selected is a
          control whose behaviour depends on state you cannot see. "Any" here
          means the artist's own base, which is what "Any" means in every other
          chip in this row.

          ⚠ **Only where there is a choice**, on the same rule as the mood chip
          beside it: an artist who works in one genre has nothing to pick
          between, and a combobox with one option is a control that cannot do
          anything. */}
      {relatedGenres.length > 0 && (
        <div className="chip chip--mono session__chip">
          <span className="session__label">{t('readouts.base')}</span>
          <Combo
            label={t('readouts.base')}
            options={[
              { id: '', name: t('readouts.ownGenre') },
              ...relatedGenres.map((genre) => ({
                id: genre.id,
                name: genre.name,
              })),
            ]}
            value={base ?? ''}
            onChange={(id) => setBase(id === '' ? null : id)}
          />
        </div>
      )}

      {/* Only for a style that offers modes — eleven of the shipped genres
          author none, and a chip whose only option is "Any" is a control that
          cannot do anything. "Any" is a *pick from the seed* rather than "no
          mood", so a reroll can land on a different kind of record by the same
          artist; the chip then says which one it landed on, exactly as the
          key and scale chips do. */}
      {moods.length > 0 && (
        <div className="chip chip--mono session__chip">
          <span className="session__label">{t('readouts.mood')}</span>
          <Combo
            label={t('readouts.mood')}
            options={[
              { id: '', name: chose(pattern?.mood ?? null) },
              ...moods.map((name) => ({ id: name, name })),
            ]}
            value={mood ?? ''}
            onChange={(id) => setMood(id === '' ? null : id)}
          />
        </div>
      )}

      {/* ⚠ **Right-aligned, alone in the column** — Mike, 2026-08-16: *"ensure
          that the Swing and the number for the swing are right aligned … so that
          the text is all on the right side of it's container"*. */}
      <label className="chip chip--mono session__chip session__chip--end" {...swingDrag.chip}>
        <span className="session__label">{t('readouts.swing')}</span>
        <input
          className="session__number"
          // Text for the same reason as the tempo, plus one of its own: swing
          // is fractional, and a number input's locale handling turns "0,54"
          // into an empty value in half of Europe.
          type="text"
          inputMode="decimal"
          value={pins.swing ?? ''}
          placeholder={defaults ? defaults.swing.amount.toFixed(2) : '—'}
          onChange={(e) => {
            // Filtered then limited, for the same reason as the tempo above.
            const cleaned = decimalOnly(e.target.value).slice(0, 4);
            setPin('swing', cleaned === '' ? null : Number(cleaned));
          }}
          onBlur={(e) => {
            const cleaned = decimalOnly(e.target.value);
            if (cleaned === '') return;
            setPin('swing', Math.min(SWING_MAX, Math.max(SWING_MIN, Number(cleaned))));
          }}
          // ⛔ **The same gesture as the tempo, at a hundredth per step** — and
          // it stops at `SWING_MIN`/`SWING_MAX`, so the whole range is 75px of
          // travel. A step of 1 here would take a straight feel to the ceiling
          // on the first pixel.
          {...swingDrag.field}
        />
        <Unpin field="swing" pinned={pins.swing !== null} />
      </label>
    </div>
  );
}

/**
 * Hand a field back to the artist.
 *
 * Only rendered for the two number fields: a select already has an option that
 * means "the artist's", and a second control for the same thing beside it
 * would be one more place for the two to disagree.
 */
function Unpin({ field, pinned }: { field: 'bpm' | 'swing'; pinned: boolean }) {
  const { t } = useTranslation();
  const setPin = useSession((s) => s.setPin);
  if (!pinned) return null;

  return (
    <button
      type="button"
      className="btn-ghost session__unpin"
      onClick={() => setPin(field, null)}
      aria-label={t('session.clearPin')}
      title={t('session.clearPin')}
    >
      <X size={12} aria-hidden="true" />
    </button>
  );
}

/**
 * Keep the pinned session, or adopt the new artist's (FR-002).
 *
 * It sits by the Generate button rather than beside the chips, because the
 * right rail collapses under 1440px and behind K — a prompt nobody can see
 * would leave the last artist's tempo quietly attached to this one. It does
 * not block: the artist has already changed, and browsing a roster must not
 * cost a dialog per click. Keeping is the default the PRD states, so ignoring
 * it entirely loses nothing.
 */
export function SessionSwitchPrompt() {
  const { t } = useTranslation();
  const pending = useSession((s) => s.pendingArtist);
  const pins = useSession((s) => s.pins);
  const defaults = useSession((s) => s.defaults);
  const keepPins = useSession((s) => s.keepPins);
  const adoptDefaults = useSession((s) => s.adoptDefaults);

  if (!pending) return null;

  // Only the pinned rows: an unpinned field is not in dispute, and listing it
  // would make the switch look bigger than it is.
  const rows: { label: string; mine: string; theirs: string }[] = [];
  if (pins.bpm !== null) {
    rows.push({
      label: t('readouts.bpm'),
      mine: String(pins.bpm),
      theirs: defaults ? String(Math.round(defaults.bpm)) : '—',
    });
  }
  if (pins.keyRoot !== null) {
    rows.push({
      label: t('readouts.key'),
      mine: keyName(pins.keyRoot) ?? '—',
      theirs: defaults?.keys.length ? defaults.keys.map(prettyKey).join(' / ') : '—',
    });
  }
  if (pins.scale !== null) {
    rows.push({
      label: t('readouts.scale'),
      mine: t(`scales.${pins.scale}`),
      theirs: defaults?.scales.length
        ? defaults.scales.map((scale) => t(`scales.${scale}`)).join(' / ')
        : '—',
    });
  }
  if (pins.swing !== null) {
    rows.push({
      label: t('readouts.swing'),
      mine: pins.swing.toFixed(2),
      theirs: defaults ? defaults.swing.amount.toFixed(2) : '—',
    });
  }

  return (
    <div className="switch-prompt" role="status">
      <p className="switch-prompt__body">{t('session.switchBody', { name: pending.name })}</p>
      <ul className="switch-prompt__rows">
        {rows.map((row) => (
          <li key={row.label}>
            <span className="switch-prompt__field">{row.label}</span>
            <strong>{row.mine}</strong>
            <span aria-hidden="true">→</span>
            <span>{row.theirs}</span>
          </li>
        ))}
      </ul>
      <div className="switch-prompt__actions">
        <button type="button" className="btn-ghost" onClick={keepPins}>
          {t('session.keep')}
        </button>
        <button type="button" className="btn-ghost" onClick={adoptDefaults}>
          {t('session.adopt', { name: pending.name })}
        </button>
      </div>
    </div>
  );
}
