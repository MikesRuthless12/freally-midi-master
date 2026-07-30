import { useEffect, useState } from 'react';
import { BugReportOverlay } from './components/BugReport/BugReport';
import { bugReportHasPendingCrash } from './components/BugReport/ipc';
import { CenterStage } from './components/layout/CenterStage';
import { Eula } from './components/Eula/Eula';
import { LeftRail } from './components/layout/LeftRail';
import { ResizeHandles } from './components/layout/ResizeHandles';
import { RightRail } from './components/layout/RightRail';
import { AboutModal } from './components/Settings/About';
import { SettingsModal } from './components/Settings/Settings';
import { TitleBar } from './components/layout/TitleBar';
import { TransportBar } from './components/layout/TransportBar';
import { UpdatePrompt } from './components/Updates/Updates';
import { subscribeToPlayhead, useSession } from './state/session';
import { isPlugin } from './lib/ipc-plugin';
import { isWide, useUi } from './state/ui';
import './components/layout/layout.css';

function Studio() {
  const [bugReportOpen, setBugReportOpen] = useState(false);
  // Undefined until the crash check answers. The update prompt must not mount
  // before then, or it could beat a pending crash report to the dialog slot.
  const [crashPending, setCrashPending] = useState<boolean | undefined>(undefined);
  const [updateDismissed, setUpdateDismissed] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const setWide = useUi((s) => s.setWide);
  const toggleRightRail = useUi((s) => s.toggleRightRail);
  const init = useSession((s) => s.init);
  const refreshHost = useSession((s) => s.refreshHost);

  // A crash left a report behind: the relaunched app opens it on its own, which
  // is the whole point of the crash loop. A pending crash takes the dialog slot
  // ahead of anything else that wants it at launch.
  useEffect(() => {
    bugReportHasPendingCrash()
      .then((pending) => {
        setCrashPending(pending);
        if (pending) setBugReportOpen(true);
      })
      .catch(() => {
        /* No backend (plain `vite dev`) — nothing to surface. */
        setCrashPending(false);
      });
  }, []);

  // The roster and the playback status, once per launch. Everything the rail
  // and the search bar draw comes from this, and the console line it logs is
  // what makes a build with a missing `data/` resource visible rather than
  // merely quiet.
  useEffect(() => {
    void init();
  }, [init]);

  // Follow the DAW's tempo. Polled rather than pushed: the plugin's bridge is
  // drained on the editor's event loop, and a host that changes tempo does not
  // notify anyone — it simply reports a different number on the next block.
  // Twice a second is far below anything a person notices as lag and far above
  // anything that costs a frame.
  //
  // Outside a plugin the command does not exist, `refreshHost` swallows that,
  // and the tempo stays null — which is exactly "no project to follow".
  useEffect(() => {
    void refreshHost();
    const timer = window.setInterval(() => void refreshHost(), 500);
    return () => window.clearInterval(timer);
  }, [refreshHost]);

  // Follow the playhead the audio thread publishes at 30 Hz. Returns a no-op
  // outside Tauri, where there is no event system behind it.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    void subscribeToPlayhead().then((unlisten) => {
      // The effect may have been torn down while the listener was being set
      // up; dropping it on the floor would leak a subscription per remount.
      if (cancelled) unlisten();
      else stop = unlisten;
    });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  // The Havoc standard: a pending crash report always wins the dialog slot,
  // and the update waits for the next launch.
  //
  // Gated on the crash check ALONE. Adding `!bugReportOpen` here looks like the
  // same rule but is a different one: it unmounts UpdatePrompt whenever the
  // user opens the bug dialog by hand, which cancels an in-flight check and
  // runs a second one on close — breaking the component's "one check per
  // launch" rule, and losing the prompt entirely if the network dropped in
  // between (the catch is deliberately silent). `hidden` keeps it out of the
  // way without remounting it.
  const updateMayShow = crashPending === false && !updateDismissed;

  // A resize listener rather than `matchMedia`, because a media query cannot
  // see the plugin's root zoom: it evaluates against the viewport, which stays
  // at the *window's* width while the page lays out at the full breakpoint
  // inside it. `isWide` measures the layout. The crossing check is kept so a
  // manual K toggle is not undone by an unrelated resize.
  useEffect(() => {
    let wasWide = isWide();
    const onResize = () => {
      const nowWide = isWide();
      if (nowWide === wasWide) return;
      wasWide = nowWide;
      setWide(nowWide);
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [setWide]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'k' && e.key !== 'K') return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      // Never steal the key from a text field.
      const el = e.target as HTMLElement | null;
      if (el?.matches?.('input, textarea, select, [contenteditable]')) return;
      e.preventDefault();
      toggleRightRail();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [toggleRightRail]);

  // Undo and redo (FMM-U01). Both the Windows/Linux and the macOS spellings,
  // plus Ctrl+Y, which is what a Windows producer's hands already do.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      if (e.altKey) return;

      const key = e.key.toLowerCase();
      if (key !== 'z' && key !== 'y') return;

      // ⛔ Inside a text field the browser's own undo owns this chord, and it
      // is undoing something more immediate than a session step. Taking it
      // would make the seed box unable to un-type a character.
      const el = e.target as HTMLElement | null;
      if (el?.matches?.('input, textarea, [contenteditable]')) return;

      e.preventDefault();
      const { undo, redo } = useSession.getState();
      if (key === 'y' || e.shiftKey) redo();
      else undo();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div
      className="studio"
      data-right-rail={rightRailOpen ? 'open' : 'closed'}
      data-shell={isPlugin() ? 'plugin' : 'desktop'}
    >
      {/* The host owns the plugin's window: Ableton draws the frame, the
          title and the close button, and a second set of them inside it is
          both redundant and a lie — our minimise and close cannot move a
          window we do not own. Settings and About move into the transport
          bar's overflow there instead. */}
      {!isPlugin() && (
        <TitleBar
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenAbout={() => setAboutOpen(true)}
        />
      )}
      <LeftRail />
      <CenterStage />
      {rightRailOpen && <RightRail />}
      <TransportBar onReportBug={() => setBugReportOpen(true)} />

      {bugReportOpen && <BugReportOverlay onClose={() => setBugReportOpen(false)} />}
      {updateMayShow && (
        <UpdatePrompt hidden={bugReportOpen} onDismiss={() => setUpdateDismissed(true)} />
      )}

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {aboutOpen && <AboutModal onClose={() => setAboutOpen(false)} />}

      <ResizeHandles />
    </div>
  );
}

/**
 * The app, behind its licence gate.
 *
 * [`Eula`] renders nothing but the agreement until it has been accepted, so the
 * studio below is never mounted, never fetches and never wires its shortcuts —
 * which is what "disable everything" has to mean on this side. The plugin
 * enforces the same thing at its RPC boundary regardless.
 */
function App() {
  return (
    <Eula>
      <Studio />
    </Eula>
  );
}

export default App;
