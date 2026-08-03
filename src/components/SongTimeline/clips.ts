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

import type { Part, Section, SectionKind, Song } from '../../lib/ipc-types';

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
  /** The pattern each copied clip pointed at, by part. */
  clips: { part: Part; patternId: string }[];
};

export function copyClips(song: Song, clips: ClipId[]): Clipboard | null {
  const entries = clips.flatMap((clip) => {
    const reference = song.sections[clip.sectionIndex]?.patterns[clip.part];
    return reference ? [{ part: clip.part, patternId: reference.patternId }] : [];
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
    if (!song.patterns[clip.patternId]) continue;
    patterns[clip.part] = { patternId: clip.patternId };
    pasted += 1;
  }
  if (pasted === 0) return song;

  return withSections(
    song,
    song.sections.map((s, i) => (i === sectionIndex ? { ...s, patterns } : s)),
  );
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
