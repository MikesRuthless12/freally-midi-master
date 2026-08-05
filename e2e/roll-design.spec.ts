import { expect, test, type Page } from '@playwright/test';

/**
 * The roll's visual design pass (TASK-041F).
 *
 * ⛔ **These check the *rendered* result, in both themes, rather than a
 * screenshot.** A pixel baseline would have to be generated on one machine and
 * compared on another — CI runs a different OS from every developer here, so
 * font hinting and canvas rasterisation differ and the gate would fail for
 * reasons that have nothing to do with the design. What matters is checkable
 * without it: that the roll actually paints in both themes, that the colours it
 * paints with clear WCAG 2.1 AA where the token file promises they do, that the
 * drag affordance appears on hover and not before, and that nothing in the
 * editor animates when the OS asks it not to.
 */

/** WCAG 2.1's relative luminance, from an `rgb(...)` string. */
function ratio(a: string, b: string): number {
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const luminance = (color: string) => {
    const [r, g, blue] = color.match(/\d+(\.\d+)?/g)!.map(Number);
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(blue);
  };
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

async function openRoll(page: Page) {
  await page.goto('/');
  const search = page.getByLabel('Search an artist');
  await search.fill('trap');
  await search.press('Enter');
  await page.getByRole('tab', { name: 'Melody' }).click();
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.locator('[data-testid="roll-notes"] li').first()).toBeAttached();
}

/** The roll's own tokens, resolved to what the browser will actually paint. */
async function tokens(page: Page) {
  return page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    const read = (name: string) => {
      const probe = document.createElement('span');
      probe.style.color = style.getPropertyValue(name).trim();
      document.body.append(probe);
      const resolved = getComputedStyle(probe).color;
      probe.remove();
      return resolved;
    };
    return {
      surface: read('--color-surface'),
      surface2: read('--color-surface-2'),
      primary: read('--color-primary'),
      signal: read('--color-signal'),
      text: read('--color-text'),
      text3: read('--color-text-3'),
    };
  });
}

for (const theme of ['dark', 'light'] as const) {
  test(`the roll paints, and clears AA, in the ${theme} theme`, async ({ page }) => {
    await openRoll(page);
    await page.evaluate((wanted) => {
      document.documentElement.dataset.theme = wanted;
    }, theme);

    const t = await tokens(page);

    // The note bodies and the playhead are the two things a producer reads at a
    // glance, and both are painted on the roll's own surface.
    expect(ratio(t.primary, t.surface), 'note bodies against the roll').toBeGreaterThanOrEqual(
      3,
    );
    expect(ratio(t.signal, t.surface), 'the playhead against the roll').toBeGreaterThanOrEqual(
      3,
    );
    // ⛔ The note outline is whichever of the two reads on that note's own body
    // — `readableOn` in `PianoRoll.tsx` — because neither one clears 3:1 on
    // both themes' note colours. So the claim to check is that *one of them*
    // does, on both fills, which is exactly what that function picks.
    for (const [fill, name] of [
      [t.primary, 'an unselected note'],
      [t.signal, 'a selected note'],
    ] as const) {
      const best = Math.max(ratio(t.text, fill), ratio(t.surface, fill));
      expect(best, `the outline on ${name}`).toBeGreaterThanOrEqual(3);
    }
    // The gutter's key names are ordinary small text.
    expect(ratio(t.text, t.surface2), 'the gutter labels').toBeGreaterThanOrEqual(4.5);
    // `--color-text-3` is documented as the muted tier: 3:1, large text only.
    expect(ratio(t.text3, t.surface2), 'the muted tier').toBeGreaterThanOrEqual(3);

    // And the canvas really did paint something rather than staying blank —
    // which every ratio above would still pass on.
    const painted = await page.locator('.roll__canvas').evaluate((canvas) => {
      const context = (canvas as HTMLCanvasElement).getContext('2d');
      const data = context!.getImageData(0, 0, (canvas as HTMLCanvasElement).width, 40).data;
      return new Set(data).size;
    });
    expect(painted, 'the roll canvas is not one flat colour').toBeGreaterThan(2);
  });
}

test('the drag affordance appears on hover, not before', async ({ page }) => {
  // ⛔ FR-010's own point: a roll that drew a grip on every note at all times has
  // a hundred grips, and the notes stop being what the eye finds first.
  await openRoll(page);
  const canvas = page.locator('.roll__canvas');
  const box = await canvas.boundingBox();
  if (box === null) throw new Error('the roll canvas has no box');

  const read = async (name: string) => Number(await canvas.getAttribute(name));
  const gutter = await read('data-gutter');
  const zoom = await read('data-zoom');
  const ppq = await read('data-ppq');
  const rowHeight = await read('data-row-height');
  const topPitch = await read('data-top-pitch');

  const first = await page
    .locator('[data-testid="roll-notes"] li')
    .first()
    .evaluate((li) => ({
      tick: Number(li.getAttribute('data-tick')),
      pitch: Number(li.getAttribute('data-pitch')),
      len: Number(li.getAttribute('data-len')),
    }));

  const xOf = (tick: number) => box.x + gutter + (tick / ppq) * zoom;
  const y = box.y + (topPitch - first.pitch) * rowHeight + rowHeight / 2;

  // Empty canvas: the crosshair, which says "draw here".
  await page.mouse.move(box.x + gutter + 4, box.y + rowHeight / 2);
  await expect(canvas).toHaveCSS('cursor', 'crosshair');

  // The middle of a note: move.
  await page.mouse.move(xOf(first.tick + first.len / 2), y);
  await expect(canvas).toHaveCSS('cursor', 'move');

  // Its right edge: resize. The affordance the roadmap asks to appear on hover.
  await page.mouse.move(xOf(first.tick + first.len) - 2, y);
  await expect(canvas).toHaveCSS('cursor', 'ew-resize');
});

test('nothing in the editor animates under reduced motion', async ({ page }) => {
  // The same rule `GenFx` already follows (FR-017). Asserted over every element
  // the roll owns rather than a chosen few, because the failure is always the
  // one rule somebody added afterwards.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openRoll(page);

  const moving = await page.evaluate(() => {
    const roll = document.querySelector('.roll');
    if (roll === null) return ['no roll'];
    const offenders: string[] = [];
    for (const element of [roll, ...roll.querySelectorAll('*')]) {
      const style = getComputedStyle(element);
      // ⛔ A millisecond, not zero. `tokens.css` uses the standard technique of
      // 0.01ms rather than 0 so that `transitionend` still fires and nothing
      // waiting on it hangs — so "no motion" is "nothing anyone can perceive",
      // and a threshold of exactly zero would flag every element in the app.
      const durations = [style.transitionDuration, style.animationDuration]
        .join(',')
        .split(',')
        .map((value) => parseFloat(value) || 0);
      if (durations.some((value) => value > 0.001))
        offenders.push(element.className.toString());
    }
    return offenders;
  });

  expect(moving, 'these still animate with reduced motion asked for').toEqual([]);
});
