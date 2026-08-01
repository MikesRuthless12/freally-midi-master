import {
  Info,
  Monitor,
  Moon,
  PanelRight,
  Pause,
  Play,
  Repeat,
  Settings2,
  Square,
  Sun,
} from 'lucide-react';
import { useUi } from '../../state/ui';
import { useSession } from '../../state/session';
import { ViewMenu } from './ViewMenu';
import { WindowSize } from './WindowSize';
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
 * Bottom transport (TASK-041T).
 *
 * ⛔ **Who owns Play depends on whether there is a host, and the difference is
 * not cosmetic.** Inside a DAW the project's transport is the transport: our
 * Play would be a second one that cannot move the first, so it is disabled and
 * wears the reason `playback_status` gives as its tooltip — never merely styled
 * that way, so keyboard and screen-reader users are told rather than left
 * clicking a control that does nothing. In the standalone there is no host and
 * these are the only transport controls there are, so Play and Pause work.
 *
 * Stop belongs here in both: the host owns *whether* time runs, and the plugin
 * owns *where in the pattern* it is. Pause holds that position; Stop returns it
 * to the beginning.
 */
/**
 * The bar/beat readout.
 *
 * ⛔ **Its own component so the playhead subscription stops here.** This is the
 * only thing in the transport bar that changes 30 times a second; when the bar
 * itself subscribed, every icon button, the theme toggle and the view menu
 * re-rendered with it — a few hundred wasted `t()` lookups a second to move
 * three digits.
 */
function Position() {
  const pattern = useSession((s) => s.pattern);
  const playhead = useSession((s) => s.playhead);

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

  return <span className="transport__position">{position}</span>;
}

export function TransportBar({
  onOpenSettings,
  onOpenAbout,
}: {
  onOpenSettings: () => void;
  onOpenAbout: () => void;
}) {
  const { t } = useTranslation();
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  const pattern = useSession((s) => s.pattern);
  const playing = useSession((s) => s.playing);
  // ⚠ Read as a boolean, never as the number. Subscribing to the raw playhead
  // here would re-render the whole footer 30 times a second, which is what
  // extracting `<Position />` was for — this only has to know whether the marker
  // has left the start.
  const parked = useSession((s) => s.playhead === 0);
  const playhead = parked ? 0 : 1;
  const canDriveTransport = useSession((s) => s.canDriveTransport());
  const playbackFailure = useSession((s) => s.playbackFailure);
  const play = useSession((s) => s.play);
  const pause = useSession((s) => s.pause);
  const stop = useSession((s) => s.stop);

  const unavailable = playbackFailure
    ? t('transport.unavailable', { reason: playbackFailure })
    : undefined;

  // ⛔ **One button, and one predicate behind it.** Play and Pause are two
  // labels for one piece of state, and the guard on both is the same question:
  // may this page drive the transport at all? It was briefly three guards — a
  // `disabled` derived from the failure string, a conditional `onClick`, and the
  // bridge's own refusal — which could each stop agreeing with the others
  // without anything failing loudly. The bridge's refusal stays, because that is
  // the trust boundary rather than a duplicate of this.
  const showPause = canDriveTransport && playing;
  // ⛔ **`pattern !== null` unconditionally.** Letting `playing` stand in for it
  // put Pause and Stop live with nothing generated: the standalone's backend
  // claims a running transport, so `playing` can be true before anything exists
  // to play. There is no state where driving a transport over an absent pattern
  // is meaningful.
  const canPress = canDriveTransport && pattern !== null;
  // ⛔ **Stop is not gated on `playing`, and that is the whole difference
  // between it and Pause.** Pause holds the marker where it is; Stop returns it
  // to the beginning — so Stop has to stay reachable *from* a pause, which is
  // exactly the state `playing === false` describes. Gating it on `playing` left
  // a paused playhead with no way back to the start at all.
  const canStop = canPress && (playing || playhead > 0);

  return (
    <footer className="transport">
      <button
        type="button"
        className="btn-ghost"
        aria-label={showPause ? t('transport.pause') : t('transport.play')}
        title={unavailable}
        disabled={!canPress}
        onClick={() => void (showPause ? pause() : play())}
      >
        {showPause ? (
          <Pause size={14} aria-hidden="true" />
        ) : (
          <Play size={14} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        className="btn-ghost"
        aria-label={t('transport.stop')}
        disabled={!canStop}
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

      <Position />

      <div className="transport__spacer" />

      <div className="meter" role="img" aria-label={t('transport.masterLevel')}>
        <div className="meter__fill" />
      </div>

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

      <WindowSize />

      {/* ⛔ Settings and About live here rather than on a title bar. The host
          owns the plugin's window — Ableton draws the frame, the title and the
          close button — so there is no chrome of ours to hang them off, and for
          a while there was no route to either at all. */}
      <button
        type="button"
        className="btn-ghost"
        data-testid="open-settings"
        onClick={onOpenSettings}
        aria-label={t('titlebar.settings')}
        title={t('titlebar.settings')}
      >
        <Settings2 size={14} aria-hidden="true" />
      </button>

      <button
        type="button"
        className="btn-ghost"
        data-testid="open-about"
        onClick={onOpenAbout}
        aria-label={t('titlebar.about')}
        title={t('titlebar.about')}
      >
        <Info size={14} aria-hidden="true" />
      </button>
    </footer>
  );
}
