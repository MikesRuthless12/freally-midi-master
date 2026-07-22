import { useEffect, useState } from 'react';
import { FolderOpen, Upload } from 'lucide-react';

import { invoke, isTauri } from '../../lib/ipc';
import './ExportChip.css';

/**
 * Drag a generated `.mid` into the DAW, or export it to a folder.
 *
 * Both routes are here on purpose. The export route is not a consolation
 * prize: on a platform where native drag proves unreliable it becomes the
 * default and the chip relabels, rather than a feature quietly failing (PRD
 * § 15 Q1, TASK-013).
 */

type ExportResult = { path: string; bytes: number };
type Capability = {
  platform: string;
  dragSupported: boolean;
  isWayland: boolean;
  note: string | null;
};

type Status = { kind: 'idle' | 'working' | 'ok' | 'error'; message?: string };

export function ExportChip() {
  const [capability, setCapability] = useState<Capability | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });

  useEffect(() => {
    invoke<Capability>('drag_capability')
      .then(setCapability)
      .catch(() => setCapability(null));
  }, []);

  /** Write the file first — a drag needs something real on disk. */
  const prepare = async (): Promise<ExportResult> => {
    const result = await invoke<ExportResult>('export_spike_midi');
    // Verify before handing the path to the OS: a drag whose source does not
    // exist is silently ignored by the drop target, which looks exactly like
    // "this platform cannot drag".
    await invoke<number>('drag_source_ready', { path: result.path });
    return result;
  };

  const onDragStart = async (e: React.DragEvent) => {
    e.preventDefault();
    if (!isTauri()) {
      setStatus({ kind: 'error', message: 'Drag only works in the desktop app.' });
      return;
    }
    try {
      setStatus({ kind: 'working' });
      const file = await prepare();
      const { startDrag } = await import('@crabnebula/tauri-plugin-drag');
      await startDrag({ item: [file.path], icon: '' });
      setStatus({ kind: 'ok', message: 'Landed. Drag it in.' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  const onExport = async () => {
    if (!isTauri()) {
      setStatus({ kind: 'error', message: 'Export only works in the desktop app.' });
      return;
    }
    try {
      setStatus({ kind: 'working' });
      const file = await prepare();
      const folder = await invoke<string | null>('pick_export_folder');
      if (!folder) {
        setStatus({ kind: 'idle' });
        return;
      }
      const written = await invoke<ExportResult>('export_to_folder', {
        source: file.path,
        folder,
      });
      setStatus({ kind: 'ok', message: `Exported to ${written.path}` });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  // On Wayland drag is unverified, so lead with Export rather than offering a
  // gesture that may silently do nothing.
  const dragFirst = !capability?.isWayland;

  return (
    <div className="exportchip">
      {dragFirst && (
        <button
          type="button"
          className="btn-ghost"
          draggable
          onDragStart={onDragStart}
          title={capability?.note ?? 'Drag the generated MIDI into your DAW'}
        >
          <Upload size={14} aria-hidden="true" />
          Drag MIDI
        </button>
      )}

      <button
        type="button"
        className="btn-ghost"
        onClick={onExport}
        title="Write the MIDI to a folder and reveal it"
      >
        <FolderOpen size={14} aria-hidden="true" />
        Export
      </button>

      {status.kind !== 'idle' && status.message && (
        <span
          className={`exportchip__status exportchip__status--${status.kind}`}
          role={status.kind === 'error' ? 'alert' : 'status'}
        >
          {status.message}
        </span>
      )}
    </div>
  );
}
