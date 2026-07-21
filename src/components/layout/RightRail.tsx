import { Section } from './Section';

/**
 * Right rail: kit over session readouts. The rail as a whole collapses below
 * 1440px and toggles with K; each panel inside also collapses on its own.
 */
export function RightRail() {
  return (
    <aside className="rail rail--right">
      <Section id="kit" grow>
        <div className="pads">
          {Array.from({ length: 8 }, (_, i) => (
            <button key={i} type="button" className="pad" disabled>
              {i + 1}
            </button>
          ))}
        </div>
        <div className="kit-drop">No kit yet. Drop your one-shots anywhere on this panel.</div>
      </Section>

      <Section id="session">
        <div className="readouts">
          <span className="chip chip--mono">
            BPM <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            Key <strong>—</strong>
          </span>
          <span className="chip chip--mono">
            Swing <strong>—</strong>
          </span>
        </div>
      </Section>
    </aside>
  );
}
