import { invoke } from '../../lib/ipc';

/** Mirrors `BugReportContextDto` in `src-tauri/src/bugreport.rs`. */
export type BugReportContext = {
  appVersion: string;
  os: string;
  arch: string;
  diagnostics: string;
  /** The scrubbed crash text from the previous run, if the app crashed. */
  pendingCrash: string | null;
};

export type BugReportTarget = 'github' | 'gmail' | 'email';

export const bugReportContext = () => invoke<BugReportContext>('bug_report_context');

/**
 * Is a crash report waiting to be read? Cheap enough to ask on mount, and the
 * question anything else competing for the launch dialog slot must ask first —
 * a pending crash report outranks every other prompt.
 */
export const bugReportHasPendingCrash = () => invoke<boolean>('bug_report_has_pending_crash');

export const bugReportSubmit = (
  target: BugReportTarget,
  description: string,
  includeCrash: boolean,
) => invoke<void>('bug_report_submit', { target, description, includeCrash });

export const bugReportClearCrash = () => invoke<void>('bug_report_clear_crash');
