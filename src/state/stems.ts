/**
 * Exporting the generated parts on their own (TASK-131F).
 *
 * Mike, 2026-08-05: *"i also need to be able to export the drums by themselves
 * as midi or audio stems"* and *"i want to be able to drag just one drum lane
 * out just like drum monkey, where i can just drag the hihats out to the daw or
 * just the snares, etc."*
 *
 * ⛔ **Its own store rather than a third exporter inside `song.ts`.** That file
 * already owns the whole-*song* export and its poll; this is the four- or
 * eight-bar loop on screen, which is a different document with a different
 * emptiness rule. What is shared is the *shape* of the poll, and that is
 * deliberately duplicated in a small form rather than abstracted — `runExport`'s
 * own comment records that two copies of the polling and timeout logic is how an
 * export ends up stuck reading "exporting…", and the way to avoid that is one
 * poll per document, each short enough to read.
 */

import { create } from 'zustand';

import { invoke } from '../lib/ipc';
import { reason } from './session';
import type { Pattern } from '../lib/ipc-types';

/** What `export_status` answers with. Mirrors `export::Status` in the plugin. */
type ExportStatus =
  | { state: 'idle' }
  | { state: 'running' }
  | { state: 'done'; path: string }
  | { state: 'cancelled' }
  | { state: 'failed'; reason: string };

/** How often the poll asks. Slow: a human is browsing for a folder. */
export const STEM_POLL_MS = 400;

export type StemFormat = 'midi' | 'audio';

type StemsState = {
  state: 'idle' | 'running' | 'done' | 'cancelled' | 'failed';
  /** The folder that was written, or the reason it was not. */
  message: string | null;
  /**
   * Whether to write one file per *lane* rather than one per part.
   *
   * The "drag just the hihats out" case. A session value rather than two more
   * buttons, because it is a preference about how every export comes out, not a
   * different action.
   */
  splitLanes: boolean;
  setSplitLanes: (split: boolean) => void;
  exportStems: (patterns: Pattern[], format: StemFormat) => Promise<void>;
};

export const useStems = create<StemsState>((set, get) => ({
  state: 'idle',
  message: null,
  splitLanes: false,

  setSplitLanes(splitLanes) {
    set({ splitLanes });
  },

  async exportStems(patterns, format) {
    if (get().state === 'running' || patterns.length === 0) return;
    set({ state: 'running', message: null });

    try {
      await invoke('export_pattern_stems', {
        patterns,
        audio: format === 'audio',
        lanes: get().splitLanes,
      });
    } catch (error) {
      // ⛔ **A refusal because one is already open falls through to the poll**,
      // for the reason `song.ts` gives at length: the plugin keeps one dialog
      // slot and only a poll drains it, so a page that stopped polling would
      // leave the producer refused with no dialog on screen and no way back.
      if (!reason(error).includes('already')) {
        set({ state: 'failed', message: reason(error) });
        return;
      }
    }

    const tick = async (): Promise<void> => {
      let status: ExportStatus;
      try {
        status = await invoke<ExportStatus>('export_status');
      } catch (error) {
        set({ state: 'failed', message: reason(error) });
        return;
      }
      if (status.state === 'running') {
        // ⛔ **No ceiling.** The plugin's dialog thread always publishes a
        // terminal status, so `running` genuinely means a dialog is open —
        // however long somebody spends making and renaming a folder. Giving up
        // early is what once left the chip idle while the slot stayed claimed.
        setTimeout(() => void tick(), STEM_POLL_MS);
        return;
      }
      set({
        state: status.state,
        message:
          status.state === 'done'
            ? status.path
            : status.state === 'failed'
              ? status.reason
              : null,
      });
    };
    await tick();
  },
}));
