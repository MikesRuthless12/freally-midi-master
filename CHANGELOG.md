# Changelog

All notable changes to Freally MIDI Master.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **The release job extracts the tagged version's section from this file** and
> uses it as the updater's release notes. Match the heading format exactly —
> `## [0.1.0] - YYYY-MM-DD`. A missing section means every user sees a generic
> note instead of what actually changed.

## [Unreleased]

### Added

- The style dataset is bundled with the app and loaded at startup: every model
  is parsed, inheritance-resolved and validated before the first frame, and the
  roster is served to the UI by the new `roster_summary` and `resolve_model`
  commands. An invalid model is skipped and reported rather than taken as a
  reason to refuse to start.
- The humanizer: MPC swing (50% straight to 66% triplet), velocity tiers for
  accents, main hits and ghost notes, per-lane timing jitter in milliseconds,
  and a quantize strength that decides how much of that jitter survives. Swing
  warps the whole timeline, so rolls written at finer resolutions travel with
  the beat they belong to.
- The drum generator core: the kick grammar (anchors, density, syncopation,
  tresillo lean, the gap before the snare, explicit multi-bar forms) and snare
  placement — half-time on 3, the 2-and-4 backbeat, drill's two-bar 3-then-4,
  and the country train beat — with ghost snares and a layered clap. Trap comes
  out with its snare on beat 3; UK drill's authored two-bar kick form
  reproduces exactly on every seed.
- The hat engine: base subdivision (8ths, 16ths or a tresillo grouping), fill
  density, open hats that close the hat underneath them, a pitch-bent second
  layer and the swell across a loop. Beats and offbeat 8ths carry the accent;
  the 16ths between them fill in quietly.
- The roll vocabulary: subdivision-switch hat rolls (16th through 64th,
  including the triplet grids), rolls placed at phrase ends, before the snare
  and before the downbeat, velocity ramps in both directions, bursts, gaps and
  offset clusters — plus snare-roll ladders with build-and-stop and dual-layer
  variants, the 8-bar riser and the stutter cluster.
- The 808 lane: it rides the kick at the share the model locks them to, sustains
  legato from one note to the next, takes its root from the session key, and
  slides by the intervals the model lists — written as the overlapping notes a
  sampler reads as portamento. UK drill's 808 stops under the snare; trap's
  rings through it.
- Fills at phrase boundaries: a small variation every two bars, a bigger one
  every eight, and a fill on the last bar so a loop leads somewhere instead of
  stopping dead. Fills take the end of their bar and leave the backbeat — and
  the ghost notes — intact.
- Twelve more genre archetypes: Chicago and NY drill, plugg and pluggnb, jerk,
  phonk, west-coast club, boom bap, 2000s R&B, liquid drum & bass, the country
  train beat and 2000s pop — fifteen in all, each with a test asserting the
  grammar that makes it that genre. Models can now say their 808 is staccato
  rather than legato, that they have no 808 at all, and that their fills turn
  over on the clap.

- Golden determinism snapshots: a fixed seed, model and session now produce
  byte-identical pattern JSON and MIDI, pinned by committed snapshots. This is
  what makes the seed chip's promise — paste a seed, get the same beat — a
  guarantee rather than an intention.

### Changed

- An 808 slide may now reach an octave above the note it starts from rather
  than being folded back inside the model's register. An octave glide — the
  phonk signature — previously landed on its own root and was discarded.
- Inheritance resolution no longer copies the whole accumulated model at each
  step of a chain, which brought a 1,000-model load from 330 ms to 219 ms —
  inside the 300 ms startup budget.

## [0.1.0] - 2026-07-22

First tagged build: the Phase 0 foundation. The Studio shell, the pure
generation engine, the style-model dataset and the full CI spine are in place;
the generators themselves arrive in Phase 1, so the transport and Generate are
deliberately disabled rather than pretending to work.

### Added

- Tauri v2 + React + TypeScript shell on a Cargo workspace, with the pure
  `engine` crate (no Tauri types, no network, no `unsafe`).
- Studio layout: left rail, six generator tabs, grid stage, right rail and
  transport, with every panel independently collapsible and the state persisted.
- Dark and light themes, contrast-verified against WCAG 2.1 AA in both.
- Engine core: `Pattern`/`Note`/`Lane`/`Song`, `SessionContext`, and seeded
  ChaCha8 RNG with per-domain stream derivation so rerolling one part leaves
  every other part byte-identical.
- Style dataset: JSON Schema, inheritance deep-merge with cycle detection,
  semantic lints, and the first three genre archetypes — trap, uk-drill, rage.
- `datasetc` CLI — validate, lint, stats, coverage.
- Crash reporter per the Havoc standard: opt-in, scrubbed, never transmitted
  without a click.
- Three-OS CI, supply-chain gates, and the AI/network dependency denylist.
- Playwright E2E against `vite dev` with IPC mocked at a single seam.
- Borderless window with its own minimise / maximise / close controls, a centred
  title, and drag-to-resize on all eight edges.
- Settings and About, reachable from the title bar, with a system-tray option
  (minimise-to-tray and close-to-tray, both off by default).
- Bug reporter and the Havoc-standard updater.
- **Eighteen languages** — English plus Arabic, Chinese (Simplified), Dutch,
  French, German, Hindi, Indonesian, Italian, Japanese, Korean, Polish,
  Portuguese (Brazil), Russian, Spanish, Turkish, Ukrainian and Vietnamese.
  Switching is instant, persists, and Arabic mirrors the whole layout.
- **Noto throughout**, bundled: 546 faces covering CJK, Arabic, Hebrew, the
  Indic scripts, Thai, Khmer, Georgian, Armenian, Ethiopic and more, so no
  language falls back to whatever the machine happens to have. Nothing is
  fetched at runtime — the app still makes no network request except the
  update check.
- CI captures the running app on all three OSes, and the Settings modal in every
  language, as downloadable artifacts. The macOS capture is partial and the job
  says so (see Live-To-Do).

### Known limitations

- The generators, playback and audio export are not implemented yet; their
  controls are disabled rather than inert.
- Native drag-out is built but **unverified against real DAWs** — that is the
  Phase 0 decision gate and it needs a human.
- The tray menu (Show / Quit) is not translated.
- Installers are unsigned: expect SmartScreen on Windows and Gatekeeper on
  macOS. See the release notes for the per-platform steps.

[Unreleased]: https://github.com/MikesRuthless12/freally-midi-master/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MikesRuthless12/freally-midi-master/releases/tag/v0.1.0
