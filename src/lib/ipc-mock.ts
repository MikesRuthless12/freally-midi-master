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
  }),
  settings_set: (args) => (args as { settings: unknown } | undefined)?.settings,

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
