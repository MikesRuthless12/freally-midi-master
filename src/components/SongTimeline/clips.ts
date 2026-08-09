/**
 * Clips as objects (TASK-063B).
 *
 * ⛔ **Clip geometry is a separate axis from edited notes, and the two must not
 * be conflated.** The 2026-07-31 decision says so in as many words: an unedited
 * clip can be cloned, trimmed and moved *while staying in its cheap seed form*.
 * So everything here moves `Section`s and `PatternRef`s around and never once
 * touches a `Pattern`. The first trim materialising every clip it touched would
 * be the bug that decision exists to prevent.
 *
 * Pure functions over the song rather than methods on a store, so the rules —
 * a resize cannot invert a clip, a paste cannot land on top of what is already
 * there — are testable without mounting anything.
 */

import type { Part, PatternRef, Section, SectionKind, Song } from '../../lib/ipc-types';

/** Which clip: the section it belongs to, and the part row it sits on. */
export type ClipId = { sectionIndex: number; part: Part };

export function sameClip(a: ClipId, b: ClipId): boolean {
  return a.sectionIndex === b.sectionIndex && a.part === b.part;
}

export function isSelected(selection: ClipId[], clip: ClipId): boolean {
  return selection.some((s) => sameClip(s, clip));
}

/**
 * The shortest a section may be.
 *
 * One bar rather than one beat: a section is the unit the *structure* is made
 * of, and a zero- or part-bar section is a hole in the song rather than a short
 * one — `arrange.rs` refuses to build one and this refuses to edit one into
 * existence.
 */
const MIN_SECTION_BARS = 1;

/** Re-lay the sections end to end, so a length change moves what follows it. */
function retile(sections: Section[]): Section[] {
  let startBar = 0;
  return sections.map((section) => {
    const next = { ...section, startBar };
    startBar += section.bars;
    return next;
  });
}

/**
 * Sections in playing order with their bars recomputed.
 *
 * ⛔ Every operation below returns through this. The alternative — each one
 * fixing up `startBar` itself — is how a timeline ends up with a gap after a
 * delete and an overlap after a resize, which the ruler draws and the export
 * disagrees with.
 */
function withSections(song: Song, sections: Section[]): Song {
  return { ...song, sections: retile(sections) };
}

/** Resize a section from either edge (TASK-063B). */
export function resizeSection(song: Song, index: number, bars: number): Song {
  const section = song.sections[index];
  if (!section) return song;
  const clamped = Math.max(MIN_SECTION_BARS, Math.round(bars));
  if (clamped === section.bars) return song;
  return withSections(
    song,
    song.sections.map((s, i) => (i === index ? { ...s, bars: clamped } : s)),
  );
}

/**
 * Resize one clip — how many bars it loops on inside its section (TASK-142).
 *
 * ⛔ **Not the same gesture as resizing a section, and that is the whole point.**
 * A section's length is how long that part of the record runs and it moves every
 * row; a clip's length is how much of its loop this one row plays before it
 * repeats. Mike's finding was that the timeline only had the first of those, so
 * *"there is no clip resize"* — the clip always filled whatever its section was.
 *
 * ⛔ **Written on the reference rather than on the pattern**, because sections of
 * the same kind share one pattern: shortening the pattern would shorten every
 * verse in the song at once. `PatternRef.bars` carries it, and the engine reads
 * it in exactly one place — `SectionTiling::of` — so what plays and what exports
 * cannot disagree about it.
 *
 * ⚠ **Back to `null` at the clip's own length**, rather than storing the number.
 * `null` means "the whole clip", so a producer who drags a clip back out to full
 * length gets a song that is byte-identical to one that was never resized — and
 * a later edit that lengthens the pattern still fills the section.
 */
export function resizeClip(song: Song, clip: ClipId, bars: number): Song {
  const section = song.sections[clip.sectionIndex];
  const reference = section?.patterns[clip.part];
  if (!section || !reference) return song;

  const pattern = song.patterns[reference.patternId];
  const full = pattern?.bars ?? 0;
  // ⛔ **Floored at one bar and capped at the clip's own length.** Zero would
  // ask the engine to lay the clip down once per tick — its own guard refuses
  // that, and a UI that can send it is a UI relying on a guard. Longer than the
  // pattern is not a longer loop: there are no notes out there, so it would read
  // as the clip having gone quiet.
  const wanted = Math.max(1, Math.round(bars));
  const clamped = full > 0 ? Math.min(full, wanted) : wanted;
  const next = full > 0 && clamped === full ? null : clamped;
  if ((reference.bars ?? null) === next) return song;

  return withSections(
    song,
    song.sections.map((s, index) =>
      index === clip.sectionIndex
        ? {
            ...s,
            patterns: {
              ...s.patterns,
              [clip.part]: { ...reference, bars: next },
            },
          }
        : s,
    ),
  );
}

/**
 * How many bars this clip loops on — its resize, or the pattern's own length.
 *
 * ⚠ **One reader, so the timeline and the engine agree.** `SectionTiling::of`
 * applies the identical rule in Rust; two spellings of "how long is this clip"
 * is how a drawn width comes to disagree with a played one.
 */
export function clipBars(song: Song, clip: ClipId): number {
  const reference = song.sections[clip.sectionIndex]?.patterns[clip.part];
  if (!reference) return 0;
  const full = song.patterns[reference.patternId]?.bars ?? 0;
  // ⚠ No `?? null` dance: `undefined > 0` is already false. The one in
  // `resizeClip` above does earn its place — there `next` is `number | null` and
  // the comparison is against a real null.
  const own = reference.bars ?? 0;
  return own > 0 ? own : full;
}

/**
 * Duplicate a section, placing the copy directly after it (TASK-063B).
 *
 * The clone shares its `PatternRef`s rather than copying the patterns: two
 * sections playing the same clip is exactly what a clone *is*, and it is what
 * `arrange.rs` already produces for two verses. Copying the notes would double
 * the document to express something the format already says in one word.
 */
export function cloneSection(song: Song, index: number): Song {
  const section = song.sections[index];
  if (!section) return song;
  const copy: Section = { ...section, patterns: { ...section.patterns } };
  const sections = [...song.sections];
  sections.splice(index + 1, 0, copy);
  return withSections(song, sections);
}

/**
 * Remove clips from their sections, leaving the sections in place (TASK-063B).
 *
 * "Delete a MIDI part and re-add it" — deleting the drums out of a verse is not
 * deleting the verse, and a delete that removed the section would make the two
 * gestures impossible to tell apart.
 */
export function deleteClips(song: Song, clips: ClipId[]): Song {
  const sections = song.sections.map((section, index) => {
    const parts = clips.filter((c) => c.sectionIndex === index).map((c) => c.part);
    if (parts.length === 0) return section;
    const patterns = { ...section.patterns };
    for (const part of parts) delete patterns[part];
    return { ...section, patterns };
  });
  return withSections(song, sections);
}

/** What a cut or copy put on the clipboard. */
export type Clipboard = {
  /**
   * The whole reference each copied clip held, by part.
   *
   * ⛔ **The reference, not just its `patternId`.** This carried the id alone
   * until TASK-142 gave a `PatternRef` a second field — and then every copy,
   * cut, paste and drag-move silently discarded the producer's clip resize,
   * because `move` is composed from copy + delete + paste. Resize a clip to loop
   * on two bars, drag it one section along, and it came back at the pattern's
   * full length. Carrying the reference whole means the next field added to it
   * cannot go missing the same way.
   */
  clips: { part: Part; reference: PatternRef }[];
};

export function copyClips(song: Song, clips: ClipId[]): Clipboard | null {
  const entries = clips.flatMap((clip) => {
    const reference = song.sections[clip.sectionIndex]?.patterns[clip.part];
    return reference ? [{ part: clip.part, reference: { ...reference } }] : [];
  });
  return entries.length > 0 ? { clips: entries } : null;
}

/**
 * Paste onto a section (TASK-063B).
 *
 * ⛔ **Refuses to paste a clip whose pattern the song no longer holds.** A
 * paste that wrote a dangling `PatternRef` would draw an empty row and export
 * silence, and the id it named would be one the producer deleted three actions
 * ago — undebuggable from the UI. The store is the authority on what exists.
 */
export function pasteClips(song: Song, clipboard: Clipboard, sectionIndex: number): Song {
  const section = song.sections[sectionIndex];
  if (!section) return song;

  const patterns = { ...section.patterns };
  let pasted = 0;
  for (const clip of clipboard.clips) {
    if (!song.patterns[clip.reference.patternId]) continue;
    // Copied whole, so the clip's own loop length lands with it.
    patterns[clip.part] = { ...clip.reference };
    pasted += 1;
  }
  if (pasted === 0) return song;

  return withSections(
    song,
    song.sections.map((s, i) => (i === sectionIndex ? { ...s, patterns } : s)),
  );
}

/**
 * Pick clips up and put them down on another section (TASK-130).
 *
 * ⛔⛔ **The one DAW verb the timeline did not have.** Mike, 2026-08-06: *"you
 * should be able to rearrange or drag them and move them, delete them, copy and
 * paste them, clone them, etc. like you would in a real DAW."* Every other verb
 * in that list was already here — this was the gap, and without it rearranging
 * meant copy, paste, then go back and delete the original.
 *
 * ⚠ **Composed from [`deleteClips`] and [`pasteClips`] rather than written
 * out.** A move *is* those two, and re-implementing the walk would be a fourth
 * place that has to know a clip is a `PatternRef` under a part key — which is
 * exactly how `pasteClips` came to need its dangling-reference guard. Composing
 * means this inherits that guard for free.
 *
 * ⚠ **The copy is taken before the delete**, and the delete before the paste.
 * `deleteClips` clears a part inside its section and never removes the section
 * itself, so section indices do not move underneath any of the three — which is
 * what makes the composition safe rather than merely tidy.
 *
 * A move onto the section the clips already sit in returns the song unchanged,
 * so a drag that goes nowhere records no undo step.
 *
 * ## ⛔⛔ Every clip keeps its own offset, and getting this wrong destroyed one
 *
 * `to` is where the clip **the producer grabbed** lands; `from` is the section
 * it came from. Everything else in the selection moves by the same distance.
 * That is what a DAW does, and it is also the only shape that is safe: the first
 * cut dropped the *whole* selection onto `to`, and since a section holds at most
 * one clip per part, two selected drums clips collapsed into one. `deleteClips`
 * had already removed both, so the loser was **gone from the arrangement with
 * nothing on screen saying so** — undo was the only way back. Selecting two
 * whole sections and dragging them is enough to hit it, because selecting a
 * section selects every part in it.
 *
 * ⚠ **The distance is clamped so nothing falls off either end**, rather than
 * dropping the clips that would. A drag that would take the selection past the
 * last section moves it as far as it goes — which is what the producer can see
 * happening, and it cannot silently lose a clip.
 */
export function moveClips(song: Song, clips: ClipId[], to: number, from: number): Song {
  // ⚠ A no-op distance returns the song itself, so a drag that goes nowhere
  // records no undo step — the property the old identity check gave.
  const shift = moveShift(song, clips, to, from);
  if (shift === 0) return song;

  // ⚠ Grouped by destination and pasted per group, which is what keeps this
  // composed from `pasteClips` — and therefore keeps its refusal to write a
  // reference the song no longer holds — rather than walking the sections a
  // fourth time here.
  const byTarget = new Map<number, Clipboard>();
  for (const clip of clips) {
    const reference = song.sections[clip.sectionIndex]?.patterns[clip.part];
    if (!reference) continue;
    const target = clip.sectionIndex + shift;
    const already = byTarget.get(target);
    const entry = already ?? { clips: [] };
    // The reference whole, so a drag-move carries the clip resize with it.
    entry.clips.push({ part: clip.part, reference: { ...reference } });
    if (!already) byTarget.set(target, entry);
  }
  // ⚠ Nothing to move — every named clip was already gone. Returning the song
  // itself keeps this off the undo stack.
  if (byTarget.size === 0) return song;

  // ⛔ Deleted only once every destination is known, and all of them at once, so
  // a clip landing on a section another selected clip is leaving does not read
  // the vacated slot as occupied.
  let next = deleteClips(song, clips);
  for (const [target, clipboard] of byTarget) next = pasteClips(next, clipboard, target);
  return next;
}

/**
 * How far {@link moveClips} would actually shift, clamped to the arrangement.
 *
 * ⛔ **Exported because the selection has to follow the clips**, and the store
 * cannot work the distance out for itself without restating the clamp — which
 * is how the selection ring ends up on cells the clips are not in.
 */
export function moveShift(song: Song, clips: ClipId[], to: number, from: number): number {
  if (!song.sections[to] || clips.length === 0) return 0;
  const last = song.sections.length - 1;
  const lowest = Math.min(...clips.map((clip) => clip.sectionIndex));
  const highest = Math.max(...clips.map((clip) => clip.sectionIndex));
  return Math.max(-lowest, Math.min(last - highest, to - from));
}

/**
 * Which section covers `bar`, or `null` past either end (TASK-130).
 *
 * The drop half of a clip drag: the pointer lands on an x, `xToBar` turns that
 * into a bar, and this says whose bar it is. Here rather than in the component
 * because "the sections tile end to end" is this file's invariant, and a
 * hand-rolled scan in the view would be a second reader of it.
 */
export function sectionAtBar(song: Song, bar: number): number | null {
  if (bar < 0) return null;
  const index = song.sections.findIndex(
    (section) => bar >= section.startBar && bar < section.startBar + section.bars,
  );
  return index === -1 ? null : index;
}

/**
 * Every section kind the engine can produce, in running order.
 *
 * ⛔ Mirrors `SectionKind` in `engine/src/pattern.rs`, and the locale gate
 * iterates this to demand a `song.kind.*` label for each — so adding a kind to
 * the engine and forgetting its name fails in CI rather than rendering the key
 * itself on the section header, which is the failure that once put
 * "settings.language" on a tab.
 */
export const SECTION_KINDS = [
  'intro',
  'verse',
  'preChorus',
  'hook',
  'bridge',
  'outro',
] as const;

// ⛔ **The compile-time half of that claim.** The locale gate only iterates this
// array, so it catches a kind added *here* without a label — and would say
// nothing about a kind added to the Rust enum and forgotten here, which renders
// `song.kind.preChorus` verbatim on the section header. This line is what makes
// the comment above true: a new `SectionKind` fails `tsc` until it is listed.
type CoversSectionKind = SectionKind extends (typeof SECTION_KINDS)[number] ? true : never;
const SECTION_KINDS_COMPLETE: CoversSectionKind = true;
void SECTION_KINDS_COMPLETE;

/**
 * Every part row the timeline draws, in a fixed order.
 *
 * ⛔ Mirrors `PART_ORDER` in `engine/src/pattern.rs`, which the multi-track SMF
 * export also reads — a timeline whose rows run one way and an exported file
 * whose tracks run another looks wrong in the DAW and is nobody's obvious bug.
 */
// ⛔ `as const`, not `: Part[]`. Annotated as `Part[]` the completeness check
// below is *vacuous* — `(typeof PART_ROWS)[number]` widens straight back to
// `Part`, so `Part extends Part` holds however few entries the array has. The
// literal tuple is what makes a missing row a compile error.
export const PART_ROWS = ['drums', 'chords', 'melody', 'counter', 'bass'] as const;

// The same completeness check: a `Part` added to the engine and missed here
// silently drops a row from the arrangement rather than failing.
type CoversPart = Part extends (typeof PART_ROWS)[number] ? true : never;
const PART_ROWS_COMPLETE: CoversPart = true;
void PART_ROWS_COMPLETE;

/** Does any section carry this part? Rows nothing plays on are not drawn. */
export function partsInUse(song: Song): Part[] {
  return PART_ROWS.filter((part) => song.sections.some((s) => s.patterns[part]));
}

/** The song's length in bars, from the sections themselves. */
export function totalBars(song: Song): number {
  return song.sections.reduce((sum, s) => sum + s.bars, 0);
}
