import { useState } from 'react';
import { Check, Copy, Lock, Unlock } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import './SeedChip.css';

/**
 * The seed, shown and editable (US-004: "paste a seed, get the same beat").
 *
 * An editable input rather than a label with a copy button, because pasting is
 * half the feature — a seed you can copy but not paste is a receipt, not a way
 * back to a beat.
 *
 * The value is a string all the way down. A u64 seed does not survive a JSON
 * number, and `Number` would silently round the ones that matter.
 *
 * ⛔⛔ **The lock is not decoration — it is the fix for the defect Mike found in
 * Ableton.** This box holds two different things: a seed he chose, and the seed
 * the engine last picked and echoed back. They look identical, and while they
 * were treated identically every Generate after the first reproduced the same
 * beat. `session.ts::seedPinned` carries the distinction; this says out loud
 * which one is on screen, because a box that silently pinned itself after the
 * first Generate is what caused it.
 */
export function SeedChip() {
  const { t } = useTranslation();
  const seed = useSession((s) => s.seed);
  const setSeed = useSession((s) => s.setSeed);
  const pinned = useSession((s) => s.seedPinned);
  const setSeedPinned = useSession((s) => s.setSeedPinned);
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    if (!seed) return;
    try {
      await navigator.clipboard.writeText(seed);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // A denied clipboard is not worth an error banner: the value is on
      // screen and selectable, which is the fallback anyway.
    }
  };

  // What the box is currently *doing*, said in one string rather than left to
  // be inferred. This is the readout the defect needed.
  //
  // ⚠ **The lock button is back, because the reason it came out was fixed in
  // the same breath.** It was built, and removed when widening this chip pushed
  // `.stage__controls` leftward across the velocity lane — that row was an
  // absolutely positioned, right-aligned column floating over the editor, so
  // width turned into a dead zone over the caps. `.stage__bottom` is in flow
  // below the editor now (see `layout.css`, which says "width is free again"),
  // so the constraint is gone rather than worked around, and the mode gets a
  // control instead of only a tooltip and an Enter key nobody would find.
  const mode = pinned ? t('stage.seedLocked') : t('stage.seedRolls');

  return (
    <span className="chip chip--mono seed" data-pinned={pinned}>
      {/* The label is for the accessibility tree only. Drawn, it sat to the left
          of the number and took room the seed itself needs — a u64 is 20 digits
          and the box was showing 12 of them. `visually-hidden` rather than
          deleted, so the input keeps a real `<label>` association on top of its
          `aria-label`. */}
      <label className="seed__label visually-hidden" htmlFor="seed-input">
        {t('stage.seed')}
      </label>
      <input
        id="seed-input"
        className="seed__input"
        type="text"
        inputMode="numeric"
        spellCheck={false}
        placeholder={t('stage.seedAuto')}
        // ⚠ The mode rides on the accessible name as well as on the lock, so a
        // screen-reader user hears which way the box is set without having to
        // find the button and interrogate its pressed state.
        aria-label={`${t('stage.seedLabel')} — ${mode}`}
        title={mode}
        value={seed}
        onChange={(event) => setSeed(event.target.value)}
        // ⚠ **Enter pins too, alongside the lock.** Typing pins as you type,
        // but a seed the engine echoed back was never typed — so committing the
        // box is the keyboard route to keeping it, for somebody who is already
        // in the field. `setSeedPinned` refuses an empty box, so Enter in an
        // empty one does nothing rather than pinning "".
        onKeyDown={(event) => {
          if (event.key === 'Enter') setSeedPinned(true);
        }}
      />
      {/* ⛔ **Disabled with an empty box, because there is nothing to hold.**
          `setSeedPinned` refuses it anyway; this stops the control offering a
          press that would do nothing.

          ⚠ **The accessible name is the constant "lock the seed" and the state
          rides on `aria-pressed`** — the ARIA toggle pattern, and here it is
          also load-bearing for the tests: `e2e/arrangement.spec.ts` reaches the
          seed input by `getByLabel(/^Seed/)`, so a button whose name began
          "Seed…" would match that too and fail Playwright's strict mode. */}
      <button
        type="button"
        className="btn-ghost seed__lock"
        aria-pressed={pinned}
        disabled={!seed}
        onClick={() => setSeedPinned(!pinned)}
        aria-label={t('stage.seedLock')}
        title={mode}
      >
        {pinned ? (
          <Lock size={12} aria-hidden="true" />
        ) : (
          <Unlock size={12} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        className="btn-ghost seed__copy"
        onClick={copy}
        disabled={!seed}
        aria-label={t('stage.copySeed')}
        title={t('stage.copySeed')}
      >
        {copied ? (
          <Check size={12} aria-hidden="true" />
        ) : (
          <Copy size={12} aria-hidden="true" />
        )}
      </button>
      {/* Announced rather than drawn: the tick is the visual confirmation, and
          a screen reader gets the same news without a toast appearing. */}
      <span role="status" className="visually-hidden">
        {copied ? t('stage.seedCopied') : ''}
      </span>
    </span>
  );
}
