import { Bug, Monitor, Moon, PanelRight, Play, Repeat, Square, Sun } from 'lucide-react';
import { useUi } from '../../state/ui';
import { useSession } from '../../state/session';
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
 * Bottom transport.
 *
 * Play and stop are disabled until there is something to play and a device to
 * play it on — never merely styled that way, so keyboard and screen-reader
 * users are told rather than left clicking a control that does nothing. When a
 * machine has no audio output at all, the reason is on the button itself.
 */
export function TransportBar({ onReportBug }: { onReportBug: () => void }) {
  const { t } = useTranslation();
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  const pattern = useSession((s) => s.pattern);
  const playing = useSession((s) => s.playing);
  const playhead = useSession((s) => s.playhead);
  const playbackFailure = useSession((s) => s.playbackFailure);
  const deviceState = useSession((s) => s.deviceState);
  const play = useSession((s) => s.play);
  const stop = useSession((s) => s.stop);

  // A device that is gone or still being reopened cannot play anything, so the
  // button says so rather than sending a command into a stream that is being
  // rebuilt underneath it.
  const deviceDown = deviceState === 'recovering' || deviceState === 'failed';
  const canPlay = pattern !== null && playbackFailure === null && !deviceDown;
  const unavailable = playbackFailure
    ? t('transport.unavailable', { reason: playbackFailure })
    : undefined;

  // Bars and beats from the fraction the audio thread publishes. 4/4 is the
  // only signature the generators write, which is why this can be arithmetic
  // rather than another field on the wire.
  const totalBeats = (pattern?.bars ?? 0) * 4;
  const beat = playhead * totalBeats;
  const position = pattern
    ? `${Math.floor(beat / 4) + 1}.${Math.floor(beat % 4) + 1}.${String(
        Math.floor((beat % 1) * 100),
      ).padStart(2, '0')}`
    : '1.1.00';

  return (
    <footer className="transport">
      <button
        type="button"
        className="btn-ghost"
        aria-label={t('transport.play')}
        title={unavailable}
        disabled={!canPlay || playing}
        onClick={() => void play()}
      >
        <Play size={14} aria-hidden="true" />
      </button>
      <button
        type="button"
        className="btn-ghost"
        aria-label={t('transport.stop')}
        disabled={!playing}
        onClick={() => void stop()}
      >
        <Square size={14} aria-hidden="true" />
      </button>
      <button
        type="button"
        className="btn-ghost"
        aria-label={t('transport.loop')}
        aria-pressed
        disabled
        title={t('transport.loopAlways')}
      >
        <Repeat size={14} aria-hidden="true" />
      </button>

      <span className="transport__position">{position}</span>

      {/* The device going away and coming back (FR-014). `role="status"` and
          not `alert`: it is news about the hardware, not an error the user
          caused, and it must not interrupt whatever they are reading. */}
      {deviceState && (
        <span className={`transport__device transport__device--${deviceState}`} role="status">
          {t(`device.${deviceState}`)}
        </span>
      )}

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
