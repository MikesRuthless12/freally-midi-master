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
import { invoke } from '../../lib/ipc';
import { useSession, armedClips, canDrive } from '../../state/session';
import { useSong } from '../../state/song';
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
 * The bottom bar — everything that is *not* transport (TASK-041T).
 *
 * ⛔⛔ **The rule that used to be written here is REVERSED and must not be
 * restored.** It read: *"inside a DAW the project's transport is the transport:
 * our Play would be a second one that cannot move the first, so it is
 * disabled."* That was right about the DAW's timeline and wrong about
 * auditioning — the plugin drives its own **preview** transport now (TASK-138),
 * and `plugin/src/shared.rs::set_running` carries the full reasoning.
 *
 * ⚠ **Play is still refused when it genuinely cannot work**, and that is the
 * part worth keeping: `playbackFailure` — no output device, a kit that failed to
 * decode, or a browser with no audio thread at all — disables the button and
 * wears its reason as a tooltip. Never merely styled that way, so keyboard and
 * screen-reader users are told rather than left clicking something inert.
 *
 * ▶ The controls themselves moved to `TransportControls`, at the top beside the
 * generator tabs, on Mike's instruction.
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
  const patterns = useSession((s) => s.patterns);
  const partsOff = useUi((s) => s.partsOff);
  const activeTab = useUi((s) => s.activeTab);
  const playhead = useSession((s) => s.playhead);

  // ⛔ **Normalised against the ARMED length, not the visible clip.**
  // `playhead` is a fraction of what the audio thread is playing, and
  // `Pattern::merge` sets the merged length to the MAX bars across the armed
  // parts. Reading the active tab's own `bars` meant 4-bar drums beside an
  // 8-bar melody counted 1.1 -> 5.1 across eight bars of real time, on the
  // Drums tab — the readout-that-lies failure, in the one control whose whole
  // job is to say where you are.
  const armed = armedClips(patterns, partsOff, activeTab);
  const bars = armed.reduce((most, clip) => Math.max(most, clip.bars), 0);
  const pattern = armed.length > 0;

  // Bars and beats from the fraction the audio thread publishes. 4/4 is the
  // only signature the generators write, which is why this can be arithmetic
  // rather than another field on the wire.
  const totalBeats = bars * 4;
  const beat = playhead * totalBeats;
  const position = pattern
    ? `${Math.floor(beat / 4) + 1}.${Math.floor(beat % 4) + 1}.${String(
        Math.floor((beat % 1) * 100),
      ).padStart(2, '0')}`
    : '1.1.00';

  return <span className="transport__position">{position}</span>;
}

/**
 * Play / Pause, Stop and Loop — at the top of the app, beside the tabs.
 *
 * ⛔⛔ **Moved out of the footer on Mike's instruction, 2026-08-06:** *"the
 * play/pause and stop buttons and loop button need to be moved to the top of the
 * app to the right of the generators tabs, so that way you can play the
 * generators from there."* They are the controls for the generator switches
 * beside them (TASK-127), so they belong in the same row as the thing they act
 * on rather than at the far end of the window.
 *
 * ⚠ **The rest of the footer stays** — the meter, the theme and view controls,
 * the rail toggle, Settings and About are not transport and were not asked for.
 */
export function TransportControls() {
  const { t } = useTranslation();
  const looping = useUi((s) => s.looping);
  const toggleLooping = useUi((s) => s.toggleLooping);

  const patterns = useSession((s) => s.patterns);
  const partsOff = useUi((s) => s.partsOff);
  const activeTab = useUi((s) => s.activeTab);
  // Song is not a part, so it has no slot — the arrangement is what is armed.
  const song = useSong((s) => s.song);
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
  // ⛔ **"Something is armed", not "the active tab holds a clip".** Gating on
  // `useActivePattern()` alone disabled Play, Pause and Stop *entirely* on the
  // Song tab — `TAB_PART.song` is `null` because a song is an arrangement of
  // the five rather than a part, so the lookup always answered `null` and Song
  // Mode had no transport at all. The arrangement is armed by `SongTimeline`
  // and there is no other control in the app, so the record could not be
  // auditioned. Before the five slots, `session.pattern` held whatever had been
  // generated regardless of the visible tab, which is why this worked then.
  // ⛔ **What is ARMED, not what is on screen.** See armedClips: the old
  // predicate was the pre-TASK-127 rule and disagreed with arming in both
  // directions — a dark button over an armed clip, and a live button over a
  // disarmed transport.
  const canPress = canDriveTransport && canDrive(patterns, partsOff, activeTab, song);
  // ⛔ **Stop is not gated on `playing`, and that is the whole difference
  // between it and Pause.** Pause holds the marker where it is; Stop returns it
  // to the beginning — so Stop has to stay reachable *from* a pause, which is
  // exactly the state `playing === false` describes. Gating it on `playing` left
  // a paused playhead with no way back to the start at all.
  const canStop = canPress && (playing || playhead > 0);

  return (
    <div className="transport__controls">
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
      {/* ⛔⛔ **Live now, and it was the last dead control in the app** — which
          is TASK-137's whole complaint. Mike, 2026-08-06: *"can you have the
          'Loop' button toggle off and on and either loop every time it plays to
          the end of the 4 or 8 bars or stop at the end of the 4 or 8 bars…and
          can you have it toggled a different background color?"*
          ⚠ **Not gated on `canDriveTransport`.** Play is a claim on the host's
          timeline and is refused inside a DAW; looping is a property of our own
          schedule over our own clip, so it is ours to answer anywhere — see
          `Shared::set_looping`, which deliberately lacks `set_running`'s
          standalone gate. */}
      <button
        type="button"
        className="btn-ghost btn-toggle"
        aria-label={t('transport.loop')}
        aria-pressed={looping}
        data-on={looping}
        title={t('transport.loop')}
        onClick={() => {
          toggleLooping();
          // The plugin is the authority; this keeps its copy in step. Ignored
          // in a browser, which has no schedule to loop.
          void invoke('transport_loop', { on: !looping }).catch(() => {});
        }}
      >
        <Repeat size={14} aria-hidden="true" />
      </button>
    </div>
  );
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

  return (
    <footer className="transport">
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
