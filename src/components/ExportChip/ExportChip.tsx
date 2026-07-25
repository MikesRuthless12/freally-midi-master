import { useEffect, useRef, useState } from 'react';
import { FolderOpen, Upload } from 'lucide-react';

import { invoke, isTauri } from '../../lib/ipc';
import { useSession } from '../../state/session';
import './ExportChip.css';
import { useTranslation } from 'react-i18next';

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
type StartDrag = typeof import('@crabnebula/tauri-plugin-drag').startDrag;

export function ExportChip() {
  const { t } = useTranslation();
  const [capability, setCapability] = useState<Capability | null>(null);
  // Nothing generated means nothing to hand over. The chips say so rather than
  // starting a drag that carries an empty file.
  const pattern = useSession((s) => s.pattern);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });

  useEffect(() => {
    invoke<Capability>('drag_capability')
      .then(setCapability)
      .catch(() => setCapability(null));
  }, []);

  /**
   * Write the file first — a drag needs something real on disk.
   *
   * The *generated* pattern, not the drag spike: what lands in the DAW has to
   * be what is on screen. Exporting the pattern rather than regenerating from
   * its seed matters too — the seed reproduces the notes, but not any edit the
   * user has made to them.
   */
  const prepare = async (): Promise<ExportResult> => {
    const pattern = useSession.getState().pattern;
    if (!pattern) throw new Error(t('export.nothingToExport'));
    const result = await invoke<ExportResult>('export_midi', { pattern });
    // Verify before handing the path to the OS: a drag whose source does not
    // exist is silently ignored by the drop target, which looks exactly like
    // "this platform cannot drag".
    await invoke<number>('drag_source_ready', { path: result.path });
    return result;
  };

  /**
   * The prepared file and the drag plugin, readied *before* the gesture.
   *
   * `dragstart` cannot wait: doing the work inside it meant two IPC round-trips
   * (synthesise a 4-bar pattern, atomic file write, canonicalise, stat) plus a
   * dynamic import before `startDrag` was even called. By then the user has
   * usually released the button, so the OS drag begins with nothing held and
   * the drop never reaches the DAW — indistinguishable from "this platform
   * cannot drag", which is precisely the distinction the spike exists to make.
   *
   * Started on pointer-down, which fires before `dragstart`, so the await inside
   * the handler is normally already settled.
   */
  const readied = useRef<Promise<{ file: ExportResult; startDrag: StartDrag }> | null>(null);

  // Readying happens on pointer-down and is cached, so a new generation has to
  // throw the cache away — otherwise the chip keeps handing the DAW the beat
  // before last.
  useEffect(() => {
    readied.current = null;
  }, [pattern]);

  const ready = () => {
    readied.current ??= (async () => {
      const [file, plugin] = await Promise.all([
        prepare(),
        import('@crabnebula/tauri-plugin-drag'),
      ]);
      return { file, startDrag: plugin.startDrag };
    })();
    return readied.current;
  };

  const onPointerDown = () => {
    if (!isTauri()) return;
    // Failures surface in onDragStart, which awaits the same promise.
    ready().catch(() => {});
  };

  const onDragStart = async (e: React.DragEvent) => {
    e.preventDefault();
    if (!isTauri()) {
      setStatus({ kind: 'error', message: t('export.dragDesktopOnly') });
      return;
    }
    try {
      setStatus({ kind: 'working' });
      const { file, startDrag } = await ready();
      // The result comes from the OS, not from `startDrag` resolving. Reporting
      // success as soon as the call returns claims the file landed when all
      // that happened is that a drag was started — which would have made the
      // DAW matrix a record of nothing.
      await startDrag({ item: [file.path], icon: '' }, (payload) => {
        setStatus(
          payload?.result === 'Dropped'
            ? {
                kind: 'ok',
                message: t('export.dropped'),
              }
            : { kind: 'idle' },
        );
      });
    } catch (err) {
      readied.current = null; // let the next attempt rebuild it
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  };

  const onExport = async () => {
    if (!isTauri()) {
      setStatus({ kind: 'error', message: t('export.exportDesktopOnly') });
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
      // No folder argument: Rust remembers what the picker returned, so the
      // destination cannot be aimed by anything running in the WebView.
      const written = await invoke<ExportResult>('export_to_folder', {
        source: file.path,
      });
      setStatus({ kind: 'ok', message: t('export.exportedTo', { path: written.path }) });
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
          draggable={pattern !== null}
          disabled={pattern === null}
          onPointerDown={onPointerDown}
          onDragStart={onDragStart}
          title={
            pattern === null
              ? t('export.nothingToExport')
              : (capability?.note ?? t('export.dragTitle'))
          }
        >
          <Upload size={14} aria-hidden="true" />
          {t('export.drag')}
        </button>
      )}

      <button
        type="button"
        className="btn-ghost"
        onClick={onExport}
        disabled={pattern === null}
        title={pattern === null ? t('export.nothingToExport') : t('export.exportTitle')}
      >
        <FolderOpen size={14} aria-hidden="true" />
        {t('export.export')}
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
