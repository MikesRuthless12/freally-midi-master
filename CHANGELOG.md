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

[Unreleased]: https://github.com/MikesRuthless12/freally-midi-master/commits/main
