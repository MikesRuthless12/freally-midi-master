import { useEffect, useState } from "react";
import {
  BugReportOverlay,
  bugReportHasPendingCrash,
} from "./components/BugReport/BugReport";
import { CenterStage } from "./components/layout/CenterStage";
import { LeftRail } from "./components/layout/LeftRail";
import { RightRail } from "./components/layout/RightRail";
import { TransportBar } from "./components/layout/TransportBar";
import { useUi, WIDE_BREAKPOINT } from "./state/ui";
import "./components/layout/layout.css";

function App() {
  const [bugReportOpen, setBugReportOpen] = useState(false);
  const rightRailOpen = useUi((s) => s.rightRailOpen);
  const setWide = useUi((s) => s.setWide);
  const toggleRightRail = useUi((s) => s.toggleRightRail);

  // A crash left a report behind: the relaunched app opens it on its own, which
  // is the whole point of the crash loop. A pending crash takes the dialog slot
  // ahead of anything else that wants it at launch.
  useEffect(() => {
    bugReportHasPendingCrash()
      .then((pending) => {
        if (pending) setBugReportOpen(true);
      })
      .catch(() => {
        /* No backend (plain `vite dev`) — nothing to surface. */
      });
  }, []);

  // The right rail follows the breakpoint, but only when it is actually
  // crossed — so a manual K toggle is not undone by an unrelated resize.
  useEffect(() => {
    const mq = window.matchMedia(`(min-width: ${WIDE_BREAKPOINT}px)`);
    const onChange = (e: MediaQueryListEvent) => setWide(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [setWide]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "k" && e.key !== "K") return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      // Never steal the key from a text field.
      const el = e.target as HTMLElement | null;
      if (el?.matches?.("input, textarea, select, [contenteditable]")) return;
      e.preventDefault();
      toggleRightRail();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleRightRail]);

  return (
    <div className="studio" data-right-rail={rightRailOpen ? "open" : "closed"}>
      <LeftRail />
      <CenterStage />
      {rightRailOpen && <RightRail />}
      <TransportBar onReportBug={() => setBugReportOpen(true)} />

      {bugReportOpen && <BugReportOverlay onClose={() => setBugReportOpen(false)} />}
    </div>
  );
}

export default App;
