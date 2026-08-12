import { useEffect, useState } from 'react';
import { CenterStage } from './components/layout/CenterStage';
import { Eula } from './components/Eula/Eula';
import { LeftRail } from './components/layout/LeftRail';
import { RightRail } from './components/layout/RightRail';
import { AboutModal } from './components/Settings/About';
import { SettingsModal } from './components/Settings/Settings';
import { ShortcutsModal } from './components/Shortcuts/Shortcuts';
import { TransportBar } from './components/layout/TransportBar';
import { isTypingTarget } from './lib/keyboard';
import { useDrag } from './state/drag';
import { subscribeToPreview, useExplorer } from './state/explorer';
import { subscribeToPadBlink } from './state/padBlink';
import { useSong } from './state/song';
import { canDrive, subscribeToPlayhead, TAB_PART, useSession } from './state/session';
import { GENERATOR_TABS, isWide, useUi } from './state/ui';
import './components/layout/layout.css';

function Studio() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const railWidth = useExplorer((s) => s.railWidth);
  const setWide = useUi((s) => s.setWide);
  const toggleRightRail = useUi((s) => s.toggleRightRail);
  const init = useSession((s) => s.init);
  const refreshHost = useSession((s) => s.refreshHost);
  const loadDragCapability = useDrag((s) => s.loadCapability);

  // The roster and the playback status, once per launch. Everything the rail
  // and the search bar draw comes from this, and the console line it logs is
  // what makes a build with a missing `data/` resource visible rather than
  // merely quiet.
  useEffect(() => {
    void init();
  }, [init]);

  // Whether this build has a native drag source (TASK-063C). Once per launch,
  // because it is a fact about what was compiled in rather than about the
  // session. ⛔ Asked rather than inferred from the platform name: the day a
  // macOS build ships without `NSFilePromiseProvider`, a page that guessed
  // would offer a handle that drops nothing.
  useEffect(() => {
    void loadDragCapability();
  }, [loadDragCapability]);

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

  // ⛔ **Light each pad as its lane fires** (Mike, 2026-08-11). Riding the same
  // playhead rather than a second poll, and writing straight to the DOM rather
  // than through state — `state/padBlink.ts` gives both reasons. It reads the
  // store rather than the bridge, so unlike the two subscriptions around it this
  // one is live in the browser too, wherever a playhead is being written.
  useEffect(() => subscribeToPadBlink(), []);

  // The same, for the sample the browser is auditioning (TASK-132). A separate
  // subscription rather than a branch inside the one above, because the two
  // read different atomics at different rates and neither should be able to
  // stall the other — and because this one also drives `Preview::collect`, the
  // editor-thread half of the audition buffer handoff.
  useEffect(() => subscribeToPreview(), []);

  // The browser rail's width (TASK-132).
  //
  // ⛔ **A custom property rather than a class**, because the width is a
  // continuous drag: a class per step is not expressible, and re-writing
  // `grid-template-columns` from JS would put the whole track list in an inline
  // style where the stylesheet could no longer own the other two columns. The
  // stage is `minmax(0, 1fr)`, so this is also what makes the centre shrink as
  // the browser widens — Mike asked for both halves.
  //
  // ⛔⛔ **On the ROOT, not as an inline style on `.studio`, and that is the
  // whole reason this is an effect.** `RailResizer` writes the same property
  // while a drag is in flight — so the store is not touched sixty times a second
  // and the tree is not reconciled — and an inline style here would have *won*
  // over the root during every one of those frames, pinning the rail at whatever
  // the store last committed. One property, one place, two writers that cannot
  // fight.
  useEffect(() => {
    document.documentElement.style.setProperty('--rail-left-width', `${railWidth}px`);
  }, [railWidth]);

  // A resize listener rather than `matchMedia`, because a media query cannot
  // see the plugin's root zoom: it evaluates against the viewport, which stays
  // at the *window's* width while the page lays out at the full breakpoint
  // inside it. `isWide` measures the layout.
  //
  // ⚠ **The crossing check is `setWide`'s own now**, and it had to move: this
  // was not the only caller — `WindowFit.tsx::applyZoom` calls it too, without
  // a guard, on every resize. Two callers of one rule is one caller too many,
  // and the one that forgot it reopened the rail under the producer.
  useEffect(() => {
    const onResize = () => setWide(isWide());
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

  // Space plays and pauses (TASK-131I).
  //
  // ⛔ **Added because the shortcuts panel could not honestly list it.**
  // `catalog.test.ts` refuses to document a key no handler listens for, and
  // Space was the first thing it caught: play and pause were wired to the
  // transport buttons alone, in a tool whose entire audience presses Space to
  // hear something.
  //
  // ⚠ **`event.code`, not `event.key`.** `key` for the space bar is `' '`,
  // which is easy to compare wrongly and differs under some IMEs; `code` is the
  // physical key and is stable everywhere.
  //
  // ⚠ **The guard is the same one the transport button uses.** Pressing Space
  // when the host owns the transport must do nothing rather than fight the DAW
  // for it — `canDriveTransport` is the one predicate that answers that, and
  // duplicating the rule here is how the two would drift.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== 'Space') return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      // ⛔ Never steal it from a text field — and never from a focused button
      // either, where Space is the browser's own "activate", so intercepting it
      // would double-fire whatever the producer had tabbed to.
      if (isTypingTarget(e.target)) return;
      if (e.target instanceof HTMLElement && e.target.closest('button, a, [role="button"]')) {
        return;
      }
      const session = useSession.getState();
      const ui = useUi.getState();
      // ⛔ **The SAME predicate the button uses, not half of it.** This checked
      // only `canDriveTransport()`, so Space with nothing generated set
      // `running` over an empty schedule — the app then reported playing
      // forever, Play rendered as a disabled Pause, Stop went disabled with it,
      // and only a second Space got out. That is the second bullet in
      // `armedClips`' own doc, arriving through the keyboard instead.
      if (!session.canDriveTransport()) return;
      if (!canDrive(session.patterns, ui.partsOff, ui.activeTab, useSong.getState().song)) {
        return;
      }
      e.preventDefault();
      void (session.playing ? session.pause() : session.play());
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // ⛔ **R rerolls, and it is Generate rather than a second code path**
  // (TASK-044). The lock is applied by `generate` on the way in, so "reroll the
  // unlocked lanes" *is* what Generate does once anything is locked — giving R
  // a generator of its own would be two answers to one question, and they would
  // drift the first time either changed.
  //
  // ⚠ **Nothing happens on the Song tab**, where R already means "reroll this
  // section" and `SongTimeline` owns it. A window-level binding that fired too
  // would reroll the section *and* the pattern behind it on one keypress.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const key = e.key.toLowerCase();
      if (key !== 'r' && key !== 'g') return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (isTypingTarget(e.target)) return;
      // ⚠ `TAB_PART.song` is `null`, so this one check covers the Song tab
      // too — where `SongTimeline` owns R for "reroll this section".
      const part = TAB_PART[useUi.getState().activeTab];
      if (part === null) return;
      e.preventDefault();

      // ⛔ **`Shift+G` is Generate All, and it is the only one of the three
      // that does something different.** `G` and `R` are deliberately the same
      // action: once anything is locked, "generate" *is* "reroll the unlocked
      // lanes" (TASK-044), and giving them separate handlers would be two
      // answers to one question. Both keys exist because both are already in a
      // producer's hands.
      if (key === 'g' && e.shiftKey) {
        void useSession.getState().generateAll();
        return;
      }
      void useSession.getState().generate(part);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // ⛔ **1–6 pick a generator** (TASK-046, FR-018). Six tabs, six digits, in
  // the order they are drawn — `GENERATOR_TABS` is the list both read, so a
  // seventh generator lands here on the day it is added rather than being a
  // tab the keyboard cannot reach.
  //
  // ⚠ **No modifier**, unlike the tuplet chords in the drum grid: those are
  // `Ctrl+3`…`Ctrl+9` precisely so a bare digit stays free for this.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
      if (isTypingTarget(e.target)) return;
      const index = Number(e.key) - 1;
      if (!Number.isInteger(index) || index < 0 || index >= GENERATOR_TABS.length) return;
      e.preventDefault();
      useUi.getState().setActiveTab(GENERATOR_TABS[index]);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // The keyboard-shortcuts panel (TASK-131I).
  //
  // ⚠ **`?` and F1, because the two audiences press different things.** `?` is
  // what every editor uses and what a producer already knows; F1 is what a
  // Windows user reaches for and costs nothing to also accept. `?` is Shift+/
  // on most layouts, so the modifier is *not* excluded here the way it is for
  // `K` above — requiring an unshifted `?` would make the panel unreachable on
  // a US keyboard.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const asked = e.key === '?' || e.key === 'F1';
      if (!asked) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (isTypingTarget(e.target)) return;
      e.preventDefault();
      setShortcutsOpen((open) => !open);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

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

      // ⛔ **`preventDefault` before anything else.** Returning early without it
      // was a bug once and a confusing one: the chord fell through to the
      // *browser's* undo, which reverted the last text field edited even though
      // focus had long since moved to the timeline — the seed box emptied
      // itself when the producer pressed Ctrl+Z over an arrangement.
      e.preventDefault();

      // ⛔ **No tab check, and its removal is the fix rather than an omission.**
      // This used to return here on the Song tab, because the arrangement had
      // no undo stack and stepping the *session* back instead was worse than
      // doing nothing. The arrangement is now part of the same snapshot
      // (`history.ts`: `Snapshot.song`), so there is one stack, one Ctrl+Z, and
      // no question about which document a keypress is about — which is exactly
      // why it was put there rather than into a second stack of its own.
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
        onOpenShortcuts={() => setShortcutsOpen(true)}
      />

      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {aboutOpen && <AboutModal onClose={() => setAboutOpen(false)} />}
      {shortcutsOpen && <ShortcutsModal onClose={() => setShortcutsOpen(false)} />}
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
