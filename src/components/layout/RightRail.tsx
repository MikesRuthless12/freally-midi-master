import { RailTabs } from './RailTabs';
import { SWAP_STYLE } from '../../state/ui';
import { Section } from './Section';

import { KitPanel } from '../Kit/KitPanel';
import { StemsPanel } from '../Kit/StemsPanel';
import { PatternBrowser } from '../PatternLibrary/PatternBrowser';
import { Presets } from '../Presets/Presets';
import { SessionChips } from '../SessionChips/SessionChips';

/**
 * Right rail: two panels at a time, with the rest on the tab strip.
 *
 * ⛔⛔ **This used to draw five accordions at once** — KIT, STEMS, SESSION,
 * PRESETS, PATTERN LIBRARY, each with its own header and its own scrollbar, so
 * STEMS had about 150px to show five drag rows in. Mike, 2026-08-11: *"only
 * leave 2 open at a time that both take up half the space … the other one's
 * would be hidden to the right until you clicked something."* `RailTabs` is the
 * way back to the other three, and it is always drawn.
 *
 * ⚠ **The panels are still listed here in full**, not filtered: `Section`
 * renders nothing when its panel is closed, so which two are showing stays one
 * fact in one place — the store — rather than a list this file also has an
 * opinion about.
 *
 * The rail as a whole still collapses below 1440px and toggles with K.
 */
export function RightRail() {
  return (
    <aside className="rail rail--right" style={SWAP_STYLE}>
      <div className="rail__panels">
        <Section id="kit">
          {/* ⛔ This panel used to be eight hardcoded `disabled` buttons and a
              static "No kit yet", rendered while a twelve-pad kit was loaded and
              audibly playing (TASK-136). It is `KitPanel`'s job now, and every
              word it shows comes from `kit_state`. */}
          <KitPanel />
        </Section>

        {/* ⛔ Here rather than in the stage toolbar. `stage__controls` sits under
            `stage__body`, so a control there costs the velocity lane height and
            fails `e2e/piano-roll.spec.ts` — see `StemsPanel` for the full note. */}
        <Section id="stems">
          <StemsPanel />
        </Section>

        <Section id="session">
          <SessionChips />
        </Section>

        <Section id="presets">
          <Presets />
        </Section>

        {/* ⛔ **Next to the presets on the strip, on purpose.** A preset is a
            *starting point* — artist, seed, pins, no notes — and a saved pattern
            is the notes themselves; putting them in one panel would make the
            difference invisible at the moment a producer has to choose. */}
        <Section id="patterns">
          <PatternBrowser />
        </Section>
      </div>

      <RailTabs rail="right" />
    </aside>
  );
}
