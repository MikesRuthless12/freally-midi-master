import { Section } from './Section';

import { KitPanel } from '../Kit/KitPanel';
import { StemsPanel } from '../Kit/StemsPanel';
import { PatternBrowser } from '../PatternLibrary/PatternBrowser';
import { Presets } from '../Presets/Presets';
import { SessionChips } from '../SessionChips/SessionChips';

/**
 * Right rail: kit over session readouts. The rail as a whole collapses below
 * 1440px and toggles with K; each panel inside also collapses on its own.
 */
export function RightRail() {
  return (
    <aside className="rail rail--right">
      <Section id="kit" grow>
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

      {/* ⛔ **Under the presets, and next to them on purpose.** A preset is a
          *starting point* — artist, seed, pins, no notes — and a saved pattern
          is the notes themselves; putting them in one panel would make the
          difference invisible at the moment a producer has to choose. */}
      <Section id="patterns">
        <PatternBrowser />
      </Section>
    </aside>
  );
}
