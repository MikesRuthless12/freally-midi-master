import { invoke } from '../../lib/ipc';
import type { Scale } from '../../lib/ipc-types';

/**
 * Which pitch classes are in the key, for row tinting and folding (TASK-041B).
 *
 * ⛔ **Asked of the engine, never restated here.** There are forty-one scales
 * and `engine::theory::scale_semitones` is the one table that defines them. A
 * TypeScript copy would drift, and the symptom of drift is the roll tinting the
 * wrong rows and `Fold to scale` hiding notes that *are* in the key — which
 * reads as the generator writing out of key rather than as the view being
 * wrong.
 *
 * ⛔ **Cached per scale, because the renderer needs this synchronously.** The
 * draw runs at frame rate and cannot await anything; the cache is filled once
 * per scale and the roll draws untinted until it lands, which is one frame at
 * worst and honest in the meantime — no tint is better than the wrong tint.
 */

const cache = new Map<Scale, ReadonlySet<number>>();

/** In-flight requests, so a scale is never asked for twice at once. */
const pending = new Map<Scale, Promise<void>>();

/** The pitch classes of `scale` in the key of `keyRoot`, or `null` if not known yet. */
export function scaleClasses(scale: Scale, keyRoot: number): ReadonlySet<number> | null {
  const semitones = cache.get(scale);
  if (semitones === undefined) return null;
  const out = new Set<number>();
  for (const semitone of semitones) out.add((semitone + keyRoot) % 12);
  return out;
}

/**
 * Make sure `scale` is in the cache, fetching it if not.
 *
 * Resolves either way: a failed fetch leaves the scale uncached, and the roll
 * simply does not tint. A scale nobody can look up is not worth an error banner
 * over the Generate button.
 */
export async function loadScale(scale: Scale): Promise<void> {
  if (cache.has(scale)) return;
  const existing = pending.get(scale);
  if (existing) return existing;

  const request = invoke<number[]>('scale_pitches', { scale })
    .then((semitones) => {
      if (Array.isArray(semitones) && semitones.length > 0) {
        cache.set(scale, new Set(semitones));
      }
    })
    .catch(() => {
      // No backend behind this page, or a scale the bridge refused. Untinted.
    })
    .finally(() => {
      pending.delete(scale);
    });

  pending.set(scale, request);
  return request;
}
