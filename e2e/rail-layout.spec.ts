import { expect, test } from '@playwright/test';
import { pickArtist } from './app';

/**
 * The right rail's panels never draw on top of each other.
 *
 * ⛔⛔ **A defect Mike screenshotted on 2026-08-06 and said was there "all the
 * time": two `.kit-hint` paragraphs overlapping into one unreadable smear.**
 * The handoff recorded it as "something above it is collapsing", and reading
 * the CSS found nothing — `.kit-hint` is margin, colour and font-size, every
 * container is normal flow, and `line-height` is a plain 1.5. It is only
 * findable by *measuring a real browser at a short viewport*.
 *
 * ▶ **What was actually happening**, measured at 1440x620: `.rail__section--grow`
 * was `flex: 1`, which is `1 1 0%` — **flex-basis zero**. With no leftover space
 * to distribute, the KIT section collapsed to **one pixel tall** while its
 * header and content kept their natural size, and `overflow: visible` let them
 * render straight out of the collapsed box and on top of STEMS. `kit.assignHint`
 * and `stems.hint` overlapped by 287x15 px.
 *
 * ⚠ **The gate has to be short, not narrow.** At 900px tall everything is
 * clean, which is why every existing spec missed it — Playwright's default
 * viewport is taller than the failure.
 */

/**
 * The box actually painted, clipped by every scrolling ancestor.
 *
 * ⛔ `getBoundingClientRect` alone is not enough and produced a false positive
 * while this was being diagnosed: an element scrolled out of an `overflow: auto`
 * parent still reports the rect it *would* have had, which reads as an overlap
 * with whatever really is drawn there.
 */
const VISIBLE_RECT = `(el) => {
  let r = el.getBoundingClientRect();
  let p = el.parentElement;
  while (p) {
    const cs = getComputedStyle(p);
    if (/auto|scroll|hidden/.test(cs.overflowY + cs.overflowX)) {
      const pr = p.getBoundingClientRect();
      const top = Math.max(r.top, pr.top), left = Math.max(r.left, pr.left);
      const bottom = Math.min(r.bottom, pr.bottom), right = Math.min(r.right, pr.right);
      if (bottom <= top || right <= left) return null;
      r = new DOMRect(left, top, right - left, bottom - top);
    }
    p = p.parentElement;
  }
  return { top: r.top, left: r.left, bottom: r.bottom, right: r.right };
}`;

// Short enough to collapse the grow section before the fix, and a size a
// producer really can end up at: a laptop with a DAW's own chrome around the
// plugin window leaves far less than 900px of height.
const SHORT = { width: 1440, height: 480 };

/**
 * ⛔ **The panels must have something in them, or the rail is short enough that
 * nothing collides and the gate passes on an empty screen.** Verified: without
 * this the reverted fix still passes — the STEMS panel is barely there until a
 * part exists, so the sections fit and the collapse never happens.
 */
async function generate(page: import('@playwright/test').Page) {
  // ⚠ The shared helper rather than a local copy: waiting for an option to be
  // *visible* does not hold for the portalled menu, and `pickArtist` also blurs,
  // which the app needs before a shortcut will fire.
  await pickArtist(page, 'uk');
  await page.getByRole('button', { name: 'Generate', exact: true }).click();
  await expect(page.getByRole('table', { name: 'Generated pattern' })).toBeVisible();
}

test('a short rail does not stack the KIT panel on top of STEMS', async ({ page }) => {
  await page.setViewportSize(SHORT);
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await generate(page);

  const overlaps = await page.evaluate(
    ([visibleRectSrc]) => {
      const visibleRect = eval(visibleRectSrc) as (
        el: Element,
      ) => { top: number; left: number; bottom: number; right: number } | null;

      const hints = Array.from(document.querySelectorAll('.rail--right .kit-hint'));
      const found: string[] = [];
      for (let i = 0; i < hints.length; i++) {
        for (let j = i + 1; j < hints.length; j++) {
          const a = visibleRect(hints[i]);
          const b = visibleRect(hints[j]);
          if (!a || !b) continue;
          const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
          const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
          if (ox > 2 && oy > 2) {
            found.push(
              `"${hints[i].textContent?.trim().slice(0, 30)}" over ` +
                `"${hints[j].textContent?.trim().slice(0, 30)}" by ${Math.round(ox)}x${Math.round(oy)}px`,
            );
          }
        }
      }
      return found;
    },
    [VISIBLE_RECT],
  );

  expect(
    overlaps,
    `panel text is drawn on top of other panel text: ${overlaps.join('; ')}`,
  ).toEqual([]);
});

test('a collapsed panel clips its own content rather than spilling', async ({ page }) => {
  // ⛔ The direct statement of the rule, so a future flex change that squashes a
  // section is caught even if the two hints happen not to land on each other.
  // A section may legitimately be short; what it may never do is paint outside
  // itself, because the next panel is there.
  await page.setViewportSize(SHORT);
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
  await generate(page);

  const spills = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.rail--right .rail__section'))
      .map((s) => {
        const box = s.getBoundingClientRect();
        const content = s.querySelector('.rail__content');
        if (!content) return null;
        const inner = content.getBoundingClientRect();
        // Sub-pixel rounding is not a spill; a header's worth of text is.
        const past = Math.round(inner.bottom - box.bottom);
        return past > 2 ? `${s.getAttribute('data-section')} spills ${past}px` : null;
      })
      .filter(Boolean),
  );

  expect(spills).toEqual([]);
});

/**
 * The right rail stays on screen at a small window, and the stage can get out
 * of its way.
 *
 * ⛔⛔ **Mike, 2026-08-12**: *"if you resize to too small of a size, the right
 * rail is never visible"* — reported while asking for a collapsible centre,
 * which is the fix he had in mind. The cause was narrower: `.studio` sized both
 * rails as **fixed** tracks either side of a `1fr` stage, so below about 600px
 * the stage was already at zero and the grid was *wider than the window*. The
 * overflow carried the right rail off the right-hand edge — it was never being
 * crowded out, it simply could not shrink.
 *
 * ⚠ **Measured against the viewport, not against a class.** "Is it visible" is a
 * question about pixels: the rail was mounted, styled and present in the DOM the
 * whole time it was off-screen, so every assertion that reads the DOM would have
 * passed while Mike was looking at a window without it.
 */
const SMALL = { width: 520, height: 700 };

test('the right rail stays inside a small window instead of overflowing it', async ({
  page,
}) => {
  await page.setViewportSize(SMALL);
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  // ⚠ **Opened by hand, because `WIDE_BREAKPOINT` closed it on the way in.**
  // That is the *other* half of Mike's report and it is worth stating here: at
  // this size the rail is not merely off-screen, it is not mounted. K is what
  // brings it back, and the overflow is what used to happen next.
  await page.keyboard.press('k');

  const rail = page.locator('.rail--right');
  const box = await rail.boundingBox();
  expect(box, 'the right rail should be laid out at all').not.toBeNull();
  expect(
    box!.x + box!.width,
    `the rail ends at ${box!.x + box!.width}px in a ${SMALL.width}px window`,
  ).toBeLessThanOrEqual(SMALL.width + 1);
});

test('collapsing the stage gives the window to the rails', async ({ page }) => {
  // ⛔ *"if you have a generation that you like, that the center can be collapsed
  // so that the 'right rail' will always show … like the whole generation
  // part."* The stage is what you have finished with once a take is settled;
  // STEMS is what you still need.
  await page.setViewportSize(SMALL);
  await page.goto('/');
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();

  // ⚠ The menu stays open after a click on purpose — it is a list of toggles —
  // so it is opened once and the same row is used twice.
  await page.getByRole('button', { name: /View/i }).click();
  const stage = page.getByRole('menuitemcheckbox', { name: 'Stage' });
  await stage.click();

  // The generator tabs are gone with the stage, and the right rail is *there*:
  // at this width the breakpoint had closed it, so collapsing the stage has to
  // bring it back or the producer is left looking at a hole.
  await expect(page.getByRole('tablist', { name: 'Generator' })).toHaveCount(0);
  const wide = (await page.locator('.rail--right').boundingBox())!;
  expect(wide.x + wide.width).toBeLessThanOrEqual(SMALL.width + 1);

  // And it comes back — a collapse nobody can undo is a panel you have lost.
  await stage.click();
  await expect(page.getByRole('tablist', { name: 'Generator' })).toBeVisible();
});
