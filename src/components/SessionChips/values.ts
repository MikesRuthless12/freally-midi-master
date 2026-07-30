import type { Scale } from '../../lib/ipc-types';

/**
 * The tempo range the BPM chip accepts — Ableton Live's, which is the one most
 * producers already know.
 *
 * Deliberately a DAW's range rather than a bespoke one. The plugin is a guest
 * in someone else's project and has to be able to say whatever that project
 * says: inventing a narrower limit would mean a session the host is running at
 * is a tempo this app refuses to display. It is also exactly what
 * `plugin/src/host.rs` already trusts from the host, so the number you can
 * type and the number the DAW can report are the same range.
 *
 * (Live is 20–999, FL 10–522, Logic 5–990, Reaper 1–960. Live's is the widest
 * commonly-cited one that still rejects a typo like 1200.)
 */
export const BPM_MIN = 20;
export const BPM_MAX = 999;

/** Swing, straight to fully triplet. Matches the engine's clamp exactly. */
export const SWING_MIN = 0.5;
export const SWING_MAX = 0.75;

/**
 * Keep only the digits.
 *
 * `<input type="number">` looks like it does this and does not: it accepts
 * `e`, `E`, `+`, `-` and `.` for scientific notation, so "1e5" is a legal
 * value that arrives as 100000 — and a number input holding text the browser
 * considers invalid reports its value as the empty string, which reads here as
 * "unpinned". The chip is therefore a text input with this on the way in.
 */
export function digitsOnly(text: string): string {
  return text.replace(/\D/g, '');
}

/** Keep digits and a single decimal point — for swing, which is fractional. */
export function decimalOnly(text: string): string {
  const cleaned = text.replace(/[^\d.]/g, '');
  const [whole, ...rest] = cleaned.split('.');
  return rest.length > 0 ? `${whole}.${rest.join('')}` : whole;
}

/**
 * Every scale the engine can generate in, in the order `engine::pattern::Scale`
 * declares them.
 *
 * Hand-written because a TypeScript union is a compile-time thing with no
 * runtime form to iterate. `values.test.ts` reads the generated `Scale` union
 * back out of `ipc-types.ts` and requires this list to match it exactly, so a
 * scale added in Rust fails here rather than becoming one the chip cannot
 * offer.
 */
export const SCALES: readonly Scale[] = [
  'natural_minor',
  'harmonic_minor',
  'phrygian',
  'phrygian_dominant',
  'dorian',
  'major',
  'mixolydian',
  'lydian',
  'aeolian',
  'minor_pentatonic',
  'major_pentatonic',
  'blues',
];

/**
 * Pitch-class names, 0 = C, spelled with sharps.
 *
 * Note names are the same twelve symbols in every locale this app ships, so
 * they are not catalog strings. The models author the ASCII forms (`"F#"`);
 * these are the typographic ones, and only ever displayed — what crosses the
 * IPC boundary is the pitch class itself.
 */
export const KEY_NAMES = [
  'C',
  'C♯',
  'D',
  'D♯',
  'E',
  'F',
  'F♯',
  'G',
  'G♯',
  'A',
  'A♯',
  'B',
] as const;

/** The typographic name for a pitch class, or `null` if it is not one. */
export function keyName(pitchClass: number | null | undefined): string | null {
  if (pitchClass == null) return null;
  return KEY_NAMES[pitchClass] ?? null;
}

/**
 * A model authors keys as ASCII (`"F#"`, `"Bb"`); the chips show them as the
 * app spells them, so the artist's options and the pinned value do not look
 * like two different notations for the same note.
 */
export function prettyKey(authored: string): string {
  return authored.replace('#', '♯').replace(/([A-G])b/, '$1♭');
}
