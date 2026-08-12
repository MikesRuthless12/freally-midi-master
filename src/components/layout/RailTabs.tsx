import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { RAIL_GROUPS, useUi, type RailId } from '../../state/ui';

/**
 * The vertical tab down the outer edge of a rail: what you get if you click it.
 *
 * ⛔⛔ **Mike asked for this shape by name**, 2026-08-11: *"you could have tabs
 * coming off the left or right side of the app itself like vertical tabs and
 * then you could click them and the other one or two or whatever would pop out
 * and switch places with one of the other ones"* — *"i think it would look a lot
 * more professional."*
 *
 * ▶ **What it replaces, and why his instinct was right.** Every panel used to be
 * an accordion stacked in the rail with its own header and its own scrollbar, so
 * the right rail showed five at once — KIT, STEMS, SESSION, PRESETS, PATTERN
 * LIBRARY — and STEMS got about 150px to draw five drag rows in.
 *
 * ⛔⛔ **ONLY THE GROUPS THAT ARE *NOT* SHOWING GET A TAB, AND THE LABEL SWITCHES
 * WHEN THEY SWAP.** Mike again: *"then switch the name on the vertical tab after
 * they switch."* So the tab is not a state readout — it is the *offer*. While
 * Kit and Stems are up, the right rail's tab reads SESSION · PRESETS · PATTERN
 * LIBRARY; press it, the panels swap, and it now reads KIT · STEMS.
 *
 * ⚠ **A tab per hidden group rather than a single toggle**, which is the same
 * thing today — both rails have exactly two groups — and stays correct if a
 * third is ever added, where a toggle would silently start skipping one.
 *
 * ⛔ **The strip is always drawn**, which is what makes the whole design safe:
 * there are no panel headers left to reopen from, so this is the only route
 * between groups and it can never be the thing that is missing.
 *
 * ⚠ **A plain button, not `role="tab"`.** A tablist promises a selected tab and
 * arrow-key movement between them; this offers the thing you have *not* got, and
 * the selected one is deliberately absent. Claiming the pattern would describe
 * something the control does not do.
 */
export function RailTabs({ rail }: { rail: RailId }) {
  const { t } = useTranslation();
  const at = useUi((s) => s.openGroups[rail]);
  const showSection = useUi((s) => s.showSection);

  return (
    <div className="railtabs" data-rail={rail} role="group" aria-label={t('view.panels')}>
      {RAIL_GROUPS[rail].map((group, index) =>
        index === at ? null : (
          <button
            key={index}
            type="button"
            className="railtabs__tab"
            // ⛔ **Which panels this tab brings, machine-readable.** The visible
            // label is translated and joined with `·`, so a test — or anything
            // else looking for "the way to the browser" — would have to match a
            // string that changes with the language. `e2e/app.ts::openPanel`
            // selects on `[data-sections~="explorer"]`, and that is the whole
            // reason this attribute exists: 44 specs broke the day the panels
            // went behind tabs, every one of them for the same reason.
            data-sections={group.join(' ')}
            // ⚠ Every panel it will bring, joined — so pressing it is never a
            // surprise. `·` rather than `/` because the names already contain
            // spaces and a slash reads as a path.
            onClick={() => showSection(group[0])}
          >
            {/* ⛔⛔ **An arrow saying which way the panel will come out**, and
                **on the side it comes out of** — Mike, 2026-08-11: *"a right
                arrow for the left side and a left arrow for the right side
                rails"*, then, seeing it: *"the right rail's chevron should be to
                the left of the text, but it's on the right."*

                ▶ So the two rails **mirror**: the left rail's arrow sits at the
                end of the tab nearest the stage, and the right rail's at its own
                end nearest the stage. Both therefore point *inwards*, in the
                direction the panel actually travels, and the pair reads as one
                symmetrical control rather than two that happen to look alike.

                ⚠ **Which end that is, is a DOM-order question here.** The tab is
                `writing-mode: vertical-rl` plus `rotate(180deg)`, so the label
                reads bottom-to-top and the flow is reversed: the **last** child
                lands at the *top* of the strip and the first at the bottom. The
                left rail wants its arrow last, the right rail first.

                ⚠ The icon is counter-rotated in CSS — the tab's own 180° would
                otherwise turn a right chevron into a left one. */}
            {rail === 'right' && (
              <span className="railtabs__arrow" aria-hidden="true">
                <ChevronLeft size={13} />
              </span>
            )}
            {group.map((id) => t(`sections.${id}`)).join(' · ')}
            {rail === 'left' && (
              <span className="railtabs__arrow" aria-hidden="true">
                <ChevronRight size={13} />
              </span>
            )}
          </button>
        ),
      )}
    </div>
  );
}
