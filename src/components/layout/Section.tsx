import { ChevronDown } from 'lucide-react';
import type { ReactNode } from 'react';
import { SECTION_LABELS, useUi, type SectionId } from '../../state/ui';

/**
 * A rail panel the user can collapse. The header is a real button with
 * `aria-expanded`, so the control is reachable by keyboard and announced
 * properly; collapsed content is unmounted rather than hidden with CSS so it
 * costs nothing once the grid and roster are real.
 */
export function Section({
  id,
  children,
  grow = false,
}: {
  id: SectionId;
  children: ReactNode;
  grow?: boolean;
}) {
  const open = useUi((s) => s.sections[id]);
  const toggleSection = useUi((s) => s.toggleSection);

  return (
    <div
      className={`rail__section${grow && open ? ' rail__section--grow' : ''}`}
      data-section={id}
      data-open={open}
    >
      <button
        type="button"
        className="rail__toggle"
        aria-expanded={open}
        aria-controls={`section-${id}`}
        onClick={() => toggleSection(id)}
      >
        <ChevronDown
          className="rail__chevron"
          size={14}
          aria-hidden="true"
          data-rotated={!open}
        />
        {SECTION_LABELS[id]}
      </button>

      {open && (
        <div id={`section-${id}`} className="rail__content">
          {children}
        </div>
      )}
    </div>
  );
}
