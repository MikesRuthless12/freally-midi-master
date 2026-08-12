import type { ReactNode } from 'react';
import { groupOf, useUi, type SectionId } from '../../state/ui';
import { useTranslation } from 'react-i18next';

/**
 * One panel in a rail slot.
 *
 * ⛔⛔ **This was an accordion and it is not one any more** (2026-08-11). Mike:
 * *"only leave 2 open at a time that both take up half the space … the other
 * one's would be hidden until you clicked something."* The header used to be a
 * collapse toggle with a chevron, and every panel in the rail rendered one — so
 * the right rail was five headers and five scrollbars deep. `RailTabs` is the
 * control now; a closed panel is not drawn at all rather than drawn collapsed.
 *
 * ⚠ **The header stays, minus the chevron — and minus the `×` as well.** With
 * two panels stacked you still have to be able to tell which is which, so the
 * label earns its place. A dismiss control does not: under groups a rail always
 * shows exactly one group, so "close this one" would either empty a slot nothing
 * could fill or silently switch the whole group, and neither is what an `×`
 * promises. See the comment on the `<h2>` below, and `.rail__title` in
 * `layout.css` — the wrapper and close-button styles were deleted with it.
 *
 * ⚠ **`grow` is gone with the accordion.** Slots are equal by construction:
 * `.rail__section` is `flex: 1 1 0`, so two panels are half the rail each
 * whatever is in them. A panel that could claim more would be the thing that
 * made the other one a sliver, which is the layout this replaced.
 */
export function Section({ id, children }: { id: SectionId; children: ReactNode }) {
  const { t } = useTranslation();
  const open = useUi((s) => s.sections[id]);
  const { rail, at } = groupOf(id);
  const leaving = useUi((s) => s.leaving[rail]) === at;

  // ⛔ Unmounted rather than hidden, exactly as the collapsed content was: the
  // roster, the browser and the pattern library all cost real work to render,
  // and a panel nobody can see must not be paying for it.
  //
  // ⛔⛔ **…but NOT while it is still sliding out.** Mike, 2026-08-11: *"you can
  // actually see it hiding and can visibly see the other one starting to slide
  // out."* React drops a panel the frame it leaves the open group, so without
  // this there is nothing on screen to animate away — the old set would blink
  // out and only the new one could move. `showSection` records the departing
  // group and clears it on a timer; this is what reads that.
  if (!open && !leaving) return null;

  return (
    <section
      className="rail__section"
      data-section={id}
      data-open={String(open)}
      // ⚠ The CSS keys both the direction and the delay off this: `out` runs
      // immediately, `in` waits for `out` to finish. One attribute, so the two
      // halves of the swap cannot be given different clocks.
      data-phase={leaving ? 'out' : 'in'}
      // ⚠ **Taken out of the flow while it leaves**, so the arriving group lays
      // out at its full size straight away and the two overlap instead of the
      // rail briefly holding five panels.
      data-rail={rail}
    >
      {/* ⚠ **A label, with no control on it at all.** The first cut put a `×`
          here to dismiss the panel, which made sense while a rail could show any
          two — a panel could then simply leave. Under groups there is nowhere
          for it to go: a rail always shows exactly one group, so "close this
          one" would either empty a slot nothing could fill or silently switch
          the whole group, and neither is what an `×` promises. Switching is the
          tab strip's job and it is two centimetres away. */}
      <h2 className="rail__title">{t(`sections.${id}`)}</h2>

      <div id={`section-${id}`} className="rail__content">
        {children}
      </div>
    </section>
  );
}
