import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import "./BugReport.css";

/** Mirrors `BugReportContextDto` in `src-tauri/src/bugreport.rs`. */
export type BugReportContext = {
  appVersion: string;
  os: string;
  arch: string;
  diagnostics: string;
  /** The scrubbed crash text from the previous run, if the app crashed. */
  pendingCrash: string | null;
};

type Target = "github" | "gmail" | "email";

export const bugReportContext = () => invoke<BugReportContext>("bug_report_context");

/**
 * Is a crash report waiting to be read? Cheap enough to ask on mount, and the
 * question anything else competing for the launch dialog slot must ask first —
 * a pending crash report outranks every other prompt.
 */
export const bugReportHasPendingCrash = () => invoke<boolean>("bug_report_has_pending_crash");

const bugReportSubmit = (target: Target, description: string, includeCrash: boolean) =>
  invoke<void>("bug_report_submit", { target, description, includeCrash });

const bugReportClearCrash = () => invoke<void>("bug_report_clear_crash");

/**
 * Report a bug — opt-in and anonymous. Shows the user the EXACT report (app/OS
 * + a scrubbed crash from the last run, if any), then lets them submit it via a
 * pre-filled GitHub issue, a Gmail compose window, or their own mail client.
 * The subject is `[Freally MIDI Master] <what went wrong>` so a report is
 * instantly attributable. No server, no shipped credentials — no report is
 * transmitted until the user clicks Send in their own client.
 */
export function BugReportOverlay({ onClose }: { onClose: () => void }) {
  const [ctx, setCtx] = useState<BugReportContext | null>(null);
  const [description, setDescription] = useState("");
  const [includeCrash, setIncludeCrash] = useState(true);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    bugReportContext()
      .then(setCtx)
      .catch((err) => setError(String(err)));
  };

  useEffect(load, []);

  // Mirrors `compose_body(.., BodyStyle::Plain)` in `bugreport.rs`. The GitHub
  // target sends the same content as Markdown (`###` headings, fenced
  // diagnostics); only the syntax differs, never the information.
  const preview = useMemo(() => {
    if (!ctx) return "";
    const parts = [
      "WHAT HAPPENED",
      description.trim() || "(no description provided)",
      "",
      "ANONYMOUS DIAGNOSTICS (no personal data)",
      "From: Freally MIDI Master",
      ctx.diagnostics.trimEnd(),
    ];
    if (includeCrash && ctx.pendingCrash) {
      parts.push("", "--- crash excerpt ---", ctx.pendingCrash.trimEnd());
    }
    return parts.join("\n");
  }, [ctx, description, includeCrash]);

  const submit = (target: Target) => {
    setError(null);
    bugReportSubmit(target, description, includeCrash && !!ctx?.pendingCrash).catch((err) =>
      setError(String(err)),
    );
  };

  const copy = () => {
    navigator.clipboard
      .writeText(preview)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => setError("Could not copy the report to the clipboard."));
  };

  const dismissCrash = () => {
    bugReportClearCrash()
      .then(load)
      .catch((err) => setError(String(err)));
  };

  return (
    <div className="bugreport-backdrop" onClick={onClose}>
      <div
        className="bugreport-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Report a bug"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="bugreport-header">
          <h2>Report a bug</h2>
          <button type="button" className="bugreport-close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        <p className="bugreport-intro">
          Reports are anonymous and never sent automatically. Read the exact text below, then choose
          how to send it — a GitHub issue, a Gmail draft, or your own mail client. You click Send.
        </p>

        {ctx?.pendingCrash && (
          <p className="bugreport-crash-notice">
            Freally MIDI Master closed unexpectedly last time. The crash details below were saved on
            this machine only.
          </p>
        )}

        <label className="bugreport-field">
          What were you doing when it went wrong?
          <textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            rows={3}
            placeholder="Optional, but it helps a lot."
          />
        </label>

        {ctx?.pendingCrash && (
          <label className="bugreport-check">
            <input
              type="checkbox"
              checked={includeCrash}
              onChange={(event) => setIncludeCrash(event.target.checked)}
            />
            Include the crash details
          </label>
        )}

        <span className="bugreport-label">Exactly what will be sent</span>
        <pre className="bugreport-preview">{preview}</pre>

        <div className="bugreport-actions">
          <button type="button" className="bugreport-primary" onClick={() => submit("github")}>
            Open GitHub issue
          </button>
          <button
            type="button"
            className="bugreport-primary"
            onClick={() => submit("gmail")}
            title="Opens Gmail's compose window in your browser, pre-filled."
          >
            Compose in Gmail
          </button>
          <button
            type="button"
            className="bugreport-primary"
            onClick={() => submit("email")}
            title="Opens your operating system's default mail client, pre-filled."
          >
            Send email
          </button>
          <button type="button" onClick={copy}>
            {copied ? "Copied" : "Copy report"}
          </button>
          {ctx?.pendingCrash && (
            <button type="button" className="bugreport-danger" onClick={dismissCrash}>
              Dismiss crash
            </button>
          )}
        </div>

        {error && (
          <p role="alert" className="bugreport-error">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
