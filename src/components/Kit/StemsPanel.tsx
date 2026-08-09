import { useTranslation } from 'react-i18next';

import { DragRows } from './DragRows';
import { useSession } from '../../state/session';
import { useStems } from '../../state/stems';

/**
 * Exporting the generated parts on their own (TASK-131F).
 *
 * ⛔⛔ **In the right rail, and NOT in the stage toolbar — this is not a layout
 * preference, it is the trap this project has now been bitten by three times.**
 * `stage__controls` sits under `stage__body`, so every pixel that row grows is a
 * pixel the editor above it loses, and the velocity lane is what loses it. Put
 * here first, these three controls made a drag to velocity 96 land on 85 and
 * `e2e/piano-roll.spec.ts:380` caught it — the same assertion that reverted two
 * earlier attempts at styling that row.
 *
 * ⚠ **Both attempted fixes made it worse or did nothing**, and they are written
 * down so nobody tries them a fourth time: `flex-wrap: nowrap` did not help
 * because the row was not wrapping, and `overflow-x: auto` made it worse because
 * the scrollbar it adds is itself height. The row was removed from the equation
 * instead. Anything new that belongs near the pattern goes in this rail.
 */
export function StemsPanel() {
  const { t } = useTranslation();
  const patterns = useSession((s) => s.patterns);
  const exportStems = useStems((s) => s.exportStems);
  const state = useStems((s) => s.state);
  const message = useStems((s) => s.message);
  const splitLanes = useStems((s) => s.splitLanes);
  const setSplitLanes = useStems((s) => s.setSplitLanes);

  const generated = Object.values(patterns);
  const busy = state === 'running';
  const nothing = generated.length === 0;

  return (
    <>
      <p className="kit-hint">{t('stems.hint')}</p>

      <div className="stems">
        <button
          type="button"
          className="stems__action"
          disabled={busy || nothing}
          onClick={() => void exportStems(generated, 'midi')}
        >
          {t('stems.midi')}
        </button>
        <button
          type="button"
          className="stems__action"
          disabled={busy || nothing}
          onClick={() => void exportStems(generated, 'audio')}
        >
          {t('stems.audio')}
        </button>
      </div>

      {/* One file per *lane* rather than per part — "drag just the hihats out".
          A toggle rather than two more actions, because it is a preference about
          how every export comes out, not a different thing to do. */}
      <button
        type="button"
        className="stems__toggle"
        aria-pressed={splitLanes}
        disabled={busy}
        onClick={() => setSplitLanes(!splitLanes)}
      >
        {t('stems.perLane')}
      </button>

      {/* ⛔ **Below the Export buttons, not instead of them** (TASK-063C). The
          drag is the headline where there is a native drag source and renders
          nothing at all where there is not, and Export stays either way — it is
          the only route on macOS and Linux, and it is still the right one for
          "put these files in a folder I choose". */}
      <DragRows />

      {busy && <p className="kit-hint">{t('stems.choosing')}</p>}
      {/* ⚠ `cancelled` says nothing, deliberately: closing the folder picker is
          the ordinary way out of it, and reporting it would train people to
          ignore the one message that matters. */}
      {state === 'done' && message && <p className="stems__done">{message}</p>}
      {state === 'failed' && message && (
        <p className="kit-error" role="alert">
          {message}
        </p>
      )}
    </>
  );
}
