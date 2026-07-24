/**
 * Canned IPC responses for running the UI without a Rust backend.
 *
 * Used by Playwright and by `vite dev` in a plain browser. This is a test
 * fixture, not a second implementation: it returns the smallest response that
 * lets the UI render, and an unknown command is a loud failure rather than a
 * silent `undefined` — a mock that quietly answers everything hides exactly the
 * bugs E2E exists to catch.
 */

import type { InvokeArgs } from '@tauri-apps/api/core';
import type { RosterSummary } from './ipc-types';

type Handler = (args?: InvokeArgs) => unknown;

const handlers: Record<string, Handler> = {
  // Exactly the shape `app_info` returns in src-tauri/src/lib.rs — no more, no
  // fewer. It used to omit `arch` and invent two fields the command has never
  // returned, so the About pane rendered "mock / undefined" here and correctly
  // in the real app: a fixture that disagrees with the DTO tests the fixture.
  app_info: () => ({
    version: '0.0.0-mock',
    platform: 'mock',
    arch: 'mock',
  }),

  // No crash happened in a browser, so the report overlay stays shut.
  bug_report_has_pending_crash: () => false,

  bug_report_context: () => ({
    appVersion: '0.0.0-mock',
    os: 'mock',
    arch: 'mock',
    diagnostics: 'From: Freally MIDI Master\nApp: 0.0.0-mock\nOS: mock / mock',
    pendingCrash: null,
  }),

  bug_report_preview: (args) => {
    const a = args as { description?: string } | undefined;
    return `WHAT HAPPENED
${a?.description?.trim() || '(no description provided)'}

ANONYMOUS DIAGNOSTICS (no personal data)
From: Freally MIDI Master`;
  },

  bug_report_submit: () => undefined,
  bug_report_clear_crash: () => undefined,

  // Settings, so the panel renders with real defaults in a browser.
  settings_get: () => ({
    minimizeToTray: false,
    closeToTray: false,
    showTrayIcon: true,
    theme: 'system',
    // Empty = never chosen, matching Settings::default() in Rust.
    language: '',
  }),
  settings_set: (args) => (args as { settings: unknown } | undefined)?.settings,

  // The roster, as the real command returns it: two genres and one artist over
  // one of them, which is enough shape for search and the tier badges without
  // pretending to be the shipped dataset.
  // Typed against the generated DTO on purpose: `tsc` then fails if the Rust
  // struct gains or renames a field and this fixture does not follow. An
  // untyped mock that disagrees with the real command tests the fixture — this
  // repo has shipped that bug before (see `app_info` above).
  roster_summary: (): RosterSummary => ({
    datasetVersion: '0.0.0-mock',
    entries: [
      {
        id: 'trap',
        name: 'Trap',
        aliases: [],
        type: 'genre',
        tier: 'standard',
        genres: ['trap'],
        era: '2010s',
      },
      {
        id: 'uk-drill',
        name: 'UK Drill',
        aliases: ['drill'],
        type: 'genre',
        tier: 'standard',
        genres: ['drill'],
        era: '2018-',
      },
      {
        id: 'mock-artist',
        name: 'Mock Artist',
        aliases: ['mock'],
        type: 'artist',
        tier: 'flagship',
        genres: ['trap'],
        era: null,
      },
    ],
    problems: [],
  }),

  // Export / drag. Without these the ExportChip's catch-all would swallow a
  // missing-handler error and render as if everything were fine.
  drag_capability: () => ({
    platform: 'mock',
    dragSupported: false,
    isWayland: false,
    note: 'Drag-out needs the desktop app.',
  }),
};

export async function mockInvoke<T>(command: string, args?: InvokeArgs): Promise<T> {
  const handler = handlers[command];
  if (!handler) {
    throw new Error(
      `ipc-mock has no handler for "${command}". Add one in src/lib/ipc-mock.ts — ` +
        `silently returning undefined would hide the bug this test exists to catch.`,
    );
  }
  return handler(args) as T;
}
