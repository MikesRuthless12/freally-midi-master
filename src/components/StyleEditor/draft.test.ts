import { describe, expect, it } from 'vitest';

import { BLANK, saysNothingOfItsOwn, type Draft } from './draft';

/**
 * The rule Mike asked for on 2026-08-16, in the one function that decides it.
 *
 * ⛔ *"the ticking a scale and pressing the save button (if you tick just a
 * scale without generating anything, then it shouldn't save anything)"*. The
 * other half of the rule — whether there was a beat on screen — is state the
 * dialog freezes when it opens, and is asserted at the call site rather than
 * here; this is the half that says what "anything" is.
 */
describe('saysNothingOfItsOwn', () => {
  /** What an untouched dialog holds when an artist is already selected. */
  const seeded: Draft = { ...BLANK, basedOn: 'trap', swing: 0.5, bpmMin: 130, bpmMax: 150 };
  const draft = (over: Partial<Draft>): Draft => ({ ...seeded, ...over });

  it('an untouched draft says nothing', () => {
    expect(saysNothingOfItsOwn(seeded, seeded)).toBe(true);
  });

  it('⛔ and it is measured against what the dialog opened with, not against BLANK', () => {
    // `trap` authors `swing.amount: 0.50` against `BLANK.swing`'s 0.54, and the
    // dialog seeds from the selected model — so measuring against `BLANK` said
    // "this has content" for every draft the producer had not touched at all.
    expect(seeded.swing).not.toBe(BLANK.swing);
    expect(saysNothingOfItsOwn(seeded, seeded)).toBe(true);
  });

  it('a name is a label on content, not content', () => {
    expect(saysNothingOfItsOwn(draft({ name: 'My Style' }), seeded)).toBe(true);
  });

  it('a different base is a different thing to restate, not something said over it', () => {
    expect(saysNothingOfItsOwn(draft({ basedOn: 'uk-drill' }), seeded)).toBe(true);
  });

  it('⛔ ticking a scale is still nothing — the list offered IS the base’s own', () => {
    expect(saysNothingOfItsOwn(draft({ scales: ['natural_minor'] }), seeded)).toBe(true);
    expect(
      saysNothingOfItsOwn(
        draft({ name: 'Mine', scales: ['natural_minor', 'phrygian'] }),
        seeded,
      ),
    ).toBe(true);
  });

  it.each([
    ['a moved tempo', { bpmMin: 90 } as Partial<Draft>],
    ['a moved swing', { swing: 0.62 }],
    ['a moved hat density', { hats: 0.9 }],
    ['a melody range', { melodyMin: 1 }],
    ['a snare placement', { snare: 'drill_3_4' as const }],
    ['a roll vocabulary', { rolls: ['32'] }],
    ['an 808 role', { bassRole: 'counter_riff' as const }],
    ['a slide amount', { slide: 0.8 }],
    ['a progression', { progressions: ['i-VI-VII'] }],
  ])('%s is content', (_label, over) => {
    expect(saysNothingOfItsOwn(draft(over), seeded)).toBe(false);
  });

  it('a field added to the draft counts as content unless it is excused by name', () => {
    // ⛔ The rule is stated as what is NOT content, so this holds for a control
    // nobody has written yet — which is the direction it has to fail in. A
    // slider a producer can move while the dialog says there is nothing to save
    // is the failure the other spelling would have shipped.
    const excused = new Set(['name', 'basedOn', 'scales']);
    for (const key of Object.keys(BLANK) as (keyof Draft)[]) {
      if (excused.has(key)) continue;
      const moved =
        typeof seeded[key] === 'number'
          ? { [key]: (seeded[key] as number) + 1 }
          : Array.isArray(seeded[key])
            ? { [key]: ['changed'] }
            : { [key]: 'changed' };
      expect(saysNothingOfItsOwn(draft(moved as Partial<Draft>), seeded), key).toBe(false);
    }
  });
});
