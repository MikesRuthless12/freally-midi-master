import { useEffect, useState } from 'react';
import { CenterStage } from './components/layout/CenterStage';
import { Eula } from './components/Eula/Eula';
import { LeftRail } from './components/layout/LeftRail';
import { RightRail } from './components/layout/RightRail';
import { AboutModal } from './components/Settings/About';
import { SettingsModal } from './components/Settings/Settings';
import { TransportBar } from './components/layout/TransportBar';
import { isTypingTarget } from './lib/keyboard';
import { subscribeToPlayhead, useSession } from './state/session';
import { isWide, useUi } from './state/ui';
import './components/layout/layout.css';

function Studio() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const setWide = useUi((s) => s.setWide);
  const toggleRightRail = useUi((s) => s.toggleRightRail);
  const init = useSession((s) => s.init);
  const refreshHost = useSession((s) => s.refreshHost);

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
  // outside the plugin, where there is no audio thread behind it.
  useEffect(() => subscribeToPlayhead(), []);

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
      if (isTypingTarget(e.target)) return;
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
      //
      // ⛔ This used to spell the selector out and *omitted `select`*, so undo
      // was stolen from a dropdown while `K` above was not. One predicate now.
      if (isTypingTarget(e.target)) return;

      // ⛔ **The Song tab owns a different document, and this stack is not its
      // stack.** `useSession`'s history holds the pattern and the session
      // chips; the arrangement lives in `useSong` and has no history yet. Left
      // unguarded, Ctrl+Z on the Song tab silently stepped *the session* back —
      // a seed keystroke, a pin, or a piano-roll edit made on another tab —
      // while the arrangement stayed exactly as it was. That is worse than
      // doing nothing: the producer sees no change here and finds the damage
      // later, somewhere else.
      //
      // Doing nothing is the honest behaviour until `useSong` has a stack of
      // its own. See the handoff.
      //
      // ⛔ **`preventDefault` first, then return** — returning early without it
      // was itself a bug, and a confusing one: the chord fell through to the
      // *browser's* undo, which reverted the last text field edited even though
      // focus had long since moved to the timeline. The seed box emptied itself
      // when the producer pressed Ctrl+Z over an arrangement.
      e.preventDefault();
      if (useUi.getState().activeTab === 'song') return;

      const { undo, redo } = useSession.getState();
      if (key === 'y' || e.shiftKey) redo();
      else undo();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // ⛔ No title bar, in any shell. The host owns the plugin's window — Ableton
  // draws the frame, the title and the close button — and a second set of them
  // inside it is both redundant and a lie, since our minimise and close cannot
  // move a window we do not own. Settings and About live in the transport bar.
  return (
    <div className="studio" data-right-rail={rightRailOpen ? 'open' : 'closed'}>
      <LeftRail />
      <CenterStage />
      {rightRailOpen && <RightRail />}
      <TransportBar
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenAbout={() => setAboutOpen(true)}
      />

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {aboutOpen && <AboutModal onClose={() => setAboutOpen(false)} />}
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
