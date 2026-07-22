# Changelog

All notable changes to Freally MIDI Master.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **The release job extracts the tagged version's section from this file** and
> uses it as the updater's release notes. Match the heading format exactly —
> `## [0.1.0] - YYYY-MM-DD`. A missing section means every user sees a generic
> note instead of what actually changed.

## [Unreleased]

Nothing yet.

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
