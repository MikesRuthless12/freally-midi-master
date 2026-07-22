import { Bug, Monitor, Moon, PanelRight, Play, Repeat, Square, Sun } from 'lucide-react';
import { useUi } from '../../state/ui';
import { ExportChip } from '../ExportChip/ExportChip';
import { ViewMenu } from './ViewMenu';
import type { ThemePreference } from '../../state/theme';
import { useTranslation } from 'react-i18next';

/** Icons only — labels come from the catalog, keyed by preference. */
const THEMES: { value: ThemePreference; Icon: typeof Sun }[] = [
  { value: 'system', Icon: Monitor },
  { value: 'dark', Icon: Moon },
  { value: 'light', Icon: Sun },
];

function ThemeToggle() {
  const { t } = useTranslation();
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);

  return (
    <div
      role="group"
      aria-label={t('theme.group')}
      style={{ display: 'flex', gap: 'var(--space-1)' }}
    >
      {THEMES.map(({ value, Icon }) => {
        const label = t(`theme.${value}`);
        return (
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
        );
      })}
    </div>
  );
}

/**
 * Bottom transport. Playback and export are inert until the audio engine and
 * MIDI writer exist; the controls are `disabled` so they cannot lie about it.
 */
export function TransportBar({ onReportBug }: { onReportBug: () => void }) {
  const { t } = useTranslation();
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  return (
    <footer className="transport">
      <button type="button" className="btn-ghost" aria-label={t('transport.play')} disabled>
        <Play size={14} aria-hidden="true" />
      </button>
      <button type="button" className="btn-ghost" aria-label={t('transport.stop')} disabled>
        <Square size={14} aria-hidden="true" />
      </button>
      <button type="button" className="btn-ghost" aria-label={t('transport.loop')} disabled>
        <Repeat size={14} aria-hidden="true" />
      </button>

      <span className="transport__position">1.1.00</span>

      <div className="transport__spacer" />

      <div className="meter" role="img" aria-label={t('transport.masterLevel')}>
        <div className="meter__fill" />
      </div>

      <ExportChip />

      <ThemeToggle />

      <ViewMenu />

      <button
        type="button"
        className="btn-ghost"
        aria-pressed={rightRailOpen}
        aria-label={t('transport.toggleRightRail')}
        title={t('transport.toggleRightRail')}
        onClick={toggleRightRail}
      >
        <PanelRight size={14} aria-hidden="true" />
      </button>

      <button
        type="button"
        className="btn-ghost"
        onClick={onReportBug}
        aria-label={t('transport.reportBug')}
        title={t('transport.reportBug')}
      >
        <Bug size={14} aria-hidden="true" />
      </button>
    </footer>
  );
}
