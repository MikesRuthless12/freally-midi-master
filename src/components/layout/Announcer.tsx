import { useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useSession } from '../../state/session';
import { keyName } from '../SessionChips/values';

/**
 * What a screen reader is told when a generation lands (TASK-095).
 *
 * ⛔⛔ **Pressing Generate produced nothing a screen reader could perceive.** The
 * grid redraws, the ripple plays and the chips fill in — all of it visual, and
 * the drum grid is a canvas besides. A producer using a reader pressed G and the
 * app went silent; whether it had worked, failed or was still thinking was
 * indistinguishable. This is the one gesture the whole product is built around,
 * so it is the one that most needed saying out loud.
 *
 * ⛔ **`polite`, never `assertive`.** Generation is something the producer asked
 * for and is expecting; interrupting whatever they are reading to announce a
 * success they predicted is the audible version of a modal. Failures are already
 * `role="alert"` where they are shown.
 *
 * ⚠ **It announces the RESULT, not the request.** "Generating…" is a state the
 * disabled button already carries via `aria-disabled`; what nothing said was
 * what came out. The sentence names the part, the length and the key, because
 * those are the three facts a producer checks on the chips before they play it.
 *
 * ⚠ **Rendered always, filled on change.** A live region has to be in the DOM
 * *before* the text arrives — one that mounts together with its message is
 * frequently not announced at all, which is the single most common way this
 * feature ships broken.
 */
/**
 * U+200B, as an escape rather than a literal.
 *
 * ⚠ `no-irregular-whitespace` refuses the character in source, and rightly — an
 * invisible one in a string is exactly the thing a reviewer cannot see. Named
 * here so what it is for is legible.
 */
export const ZERO_WIDTH = '\u200B';

export function Announcer() {
  const { t } = useTranslation();
  const patterns = useSession((s) => s.patterns);
  const generating = useSession((s) => s.generating);
  /**
   * The sentence, and a flip that makes an identical one a new text node.
   *
   * ⛔ **Setting state to the same string is a React bail-out** — no re-render,
   * no text-node change, and a live region whose text did not change is a live
   * region that says nothing. Two generations that land on the same bars, tempo
   * and key produce identical copy, so the second was silent. The zero-width
   * space is ignored by readers and makes the DOM differ.
   */
  const [{ text, flip }, setMessage] = useState({ text: '', flip: false });

  /**
   * The generation this has already spoken about.
   *
   * ⚠ Keyed on the *song seed and part set* rather than on object identity:
   * `patterns` is replaced on every store write, including ones that have
   * nothing to do with generating — a lane mute, an edit — and announcing "drums
   * generated" because somebody muted a hat is worse than saying nothing.
   */
  const spoken = useRef('');

  useEffect(() => {
    if (generating) return;
    const made = Object.values(patterns);
    if (made.length === 0) return;

    const generation = made
      .map((p) => `${p.part}:${p.seed}`)
      .sort()
      .join('|');
    if (generation === spoken.current) return;
    spoken.current = generation;

    // The newest set, described once however many parts it holds: five separate
    // announcements for one press is five interruptions.
    //
    // ⛔ **No count in the sentence, and that is a translation decision rather
    // than an omission.** Passing `count` to i18next turns on plural resolution,
    // so "1 part" and "5 parts" would need `_one`/`_other` forms in eighteen
    // catalogs — six of them in Arabic alone — and the repo's own convention
    // (`styles.kept`) avoids that entirely. The producer pressed Generate on a
    // tab and knows what they asked for; what they cannot see is what came out.
    const first = made[0];
    // ⛔ **`keyRoot` is a PITCH CLASS, not a note name.** Interpolating it raw
    // made the one sentence this component exists to speak say *"7 minor"* for
    // G minor. `keyName` is what the chips already use — a reader must not be
    // told something the screen is not saying.
    const key = [keyName(first.keyRoot), t(`scales.${first.scale}`, first.scale)]
      .filter(Boolean)
      .join(' ');
    const said = t('a11y.generated', { bars: first.bars, bpm: Math.round(first.bpm), key });
    setMessage((was) => ({ text: said, flip: !was.flip }));
  }, [patterns, generating, t]);

  return (
    <p className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
      {flip ? `${text}${ZERO_WIDTH}` : text}
    </p>
  );
}
