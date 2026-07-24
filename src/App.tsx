import { useEffect, useState } from 'react';
import { BugReportOverlay } from './components/BugReport/BugReport';
import { bugReportHasPendingCrash } from './components/BugReport/ipc';
import { CenterStage } from './components/layout/CenterStage';
import { LeftRail } from './components/layout/LeftRail';
import { ResizeHandles } from './components/layout/ResizeHandles';
import { RightRail } from './components/layout/RightRail';
import { AboutModal } from './components/Settings/About';
import { SettingsModal } from './components/Settings/Settings';
import { TitleBar } from './components/layout/TitleBar';
import { TransportBar } from './components/layout/TransportBar';
import { UpdatePrompt } from './components/Updates/Updates';
import { loadRoster } from './lib/roster';
import { useUi, WIDE_BREAKPOINT } from './state/ui';
import './components/layout/layout.css';

function App() {
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

  // The roster, once per launch. Nothing renders from it yet — the rail and the
  // search bar are TASK-028 — but the load is what proves the dataset shipped,
  // and it puts the model count in the console where a build with a missing
  // `data/` resource is visible rather than merely quiet.
  useEffect(() => {
    loadRoster().catch((e: unknown) => {
      console.error('dataset: the roster could not be loaded', e);
    });
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

  // The right rail follows the breakpoint, but only when it is actually
  // crossed — so a manual K toggle is not undone by an unrelated resize.
  useEffect(() => {
    const mq = window.matchMedia(`(min-width: ${WIDE_BREAKPOINT}px)`);
    const onChange = (e: MediaQueryListEvent) => setWide(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
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

  return (
    <div className="studio" data-right-rail={rightRailOpen ? 'open' : 'closed'}>
      <TitleBar
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenAbout={() => setAboutOpen(true)}
      />
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

export default App;
