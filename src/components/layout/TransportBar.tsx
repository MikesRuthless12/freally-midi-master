import { Bug, Monitor, Moon, PanelRight, Play, Repeat, Square, Sun } from 'lucide-react';
import { useUi } from '../../state/ui';
import { ExportChip } from '../ExportChip/ExportChip';
import { ViewMenu } from './ViewMenu';
import type { ThemePreference } from '../../state/theme';

const THEMES: { value: ThemePreference; label: string; Icon: typeof Sun }[] = [
  { value: 'system', label: 'Match system theme', Icon: Monitor },
  { value: 'dark', label: 'Dark theme', Icon: Moon },
  { value: 'light', label: 'Light theme', Icon: Sun },
];

function ThemeToggle() {
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);

  return (
    <div role="group" aria-label="Theme" style={{ display: 'flex', gap: 'var(--space-1)' }}>
      {THEMES.map(({ value, label, Icon }) => (
        <button
          key={value}
          type="button"
          className="btn-ghost"
          aria-pressed={theme === value}
          aria-label={label}
          title={label}
          onClick={() => setTheme(value)}
        >
          <Icon size={14} aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}

/**
 * Bottom transport. Playback and export are inert until the audio engine and
 * MIDI writer exist; the controls are `disabled` so they cannot lie about it.
 */
export function TransportBar({ onReportBug }: { onReportBug: () => void }) {
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  return (
    <footer className="transport">
      <button type="button" className="btn-ghost" aria-label="Play" disabled>
        <Play size={14} aria-hidden="true" />
      </button>
      <button type="button" className="btn-ghost" aria-label="Stop" disabled>
        <Square size={14} aria-hidden="true" />
      </button>
      <button type="button" className="btn-ghost" aria-label="Loop" disabled>
        <Repeat size={14} aria-hidden="true" />
      </button>

      <span className="transport__position">1.1.00</span>

      <div className="transport__spacer" />

      <div className="meter" role="img" aria-label="Master level: silent">
        <div className="meter__fill" />
      </div>

      <ExportChip />

      <ThemeToggle />

      <ViewMenu />

      <button
        type="button"
        className="btn-ghost"
        aria-pressed={rightRailOpen}
        aria-label="Toggle right rail (K)"
        title="Toggle right rail (K)"
        onClick={toggleRightRail}
      >
        <PanelRight size={14} aria-hidden="true" />
      </button>

      <button
        type="button"
        className="btn-ghost"
        onClick={onReportBug}
        aria-label="Report a bug"
        title="Report a bug"
      >
        <Bug size={14} aria-hidden="true" />
      </button>
    </footer>
  );
}
