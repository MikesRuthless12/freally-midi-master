# Credits & Attribution

Sources whose published statistics, research, or assets informed Freally MIDI
Master. The About screen quotes this file.

*This list grows as the style dataset is authored. Every entry here is a source of
**numbers and rules** — no MIDI, audio, or musical content from any source below is
copied into the product. See `docs/legal/disclaimer.md`.*

## Datasets

- **Magenta Groove MIDI Dataset (GMD)** and **Expanded Groove MIDI Dataset (E-GMD)**
  — Google Magenta. Aggregate timing and velocity statistics (microtiming deviation
  distributions, swing ratios, velocity spreads by drum voice) informed the
  humanizer's constants. Licensed CC BY 4.0.
  https://magenta.withgoogle.com/datasets/groove

## Fonts

Bundled as subset `woff2` files; full license texts vendored alongside them in
`src/assets/fonts/`.

- **Inter** — Rasmus Andersson. SIL Open Font License 1.1.
- **Space Grotesk** — Florian Karsten. SIL Open Font License 1.1.
- **JetBrains Mono** — JetBrains. SIL Open Font License 1.1.

## Icons

- **Lucide** — ISC License. Bundled locally; no CDN.

## Sounds

The preview instrument kits shipped in `data/kits/` are **synthesized from scratch**
by this project's own `tools/kitgen` and contain no third-party samples.

## Third-party code

Rust crates and npm packages, with their licenses, are listed in
`THIRD-PARTY-NOTICES.md`. The dependency license allowlist is enforced in CI by
`cargo deny`.
