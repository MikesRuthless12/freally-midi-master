import { useState } from 'react';
import { Check, Copy } from 'lucide-react';
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
 */
export function SeedChip() {
  const { t } = useTranslation();
  const seed = useSession((s) => s.seed);
  const setSeed = useSession((s) => s.setSeed);
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

  return (
    <span className="chip chip--mono seed">
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
        aria-label={t('stage.seedLabel')}
        value={seed}
        onChange={(event) => setSeed(event.target.value)}
      />
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
