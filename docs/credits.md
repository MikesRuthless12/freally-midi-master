# Credits & Attribution

Sources whose published statistics, research, or assets informed Freally MIDI
Master. The About screen quotes this file.

*This list grows as the style dataset is authored. Every entry here is a source of
**numbers and rules** — no MIDI, audio, or musical content from any source below is
copied into the product. See `docs/legal/disclaimer.md`.*

## Datasets

⛔⛔ **NONE, TODAY — AND THIS SECTION USED TO SAY OTHERWISE** (corrected
2026-08-22, TASK-091). It credited the Magenta Groove MIDI Dataset for having
*"informed the humanizer's constants"*, and the About screen quoted that as
*"Timing and velocity statistics derived from the Magenta Groove MIDI Dataset
(CC BY 4.0)"* in all eighteen languages. **Neither is true yet.**
`engine/src/humanize.rs` is hand-authored from the published technique research
below — its own header says *"research ch. 1 cross-genre constants"* — there is
no `data/humanize/`, and `tools/groovestats` has never been written.

▶ **GMD is a *permitted* source, not a used one.** `docs/style-research.md`
lists it as commercial-safe under CC-BY 4.0 and describes what to extract in the
future tense: *"extract per-genre swing ratios, per-instrument timing offsets,
velocity distributions, ghost-note probabilities; ship derived statistics with
attribution."* That is **TASK-078**, which is unstarted.

⚠ **Removing the credit is the correct direction, not a shortcut.** CC-BY asks
for attribution when you *use* the work; attributing something you do not use is
a false statement about where the product's numbers came from — and precise
provenance is the whole of this product's legal position. ▶ **When TASK-078
lands, this section comes back**, with the attribution CC-BY then requires.

## Published technique research

The drum grammars in `data/genres/` — kick placement, snare and ghost-note
conventions, hat subdivisions, roll vocabularies, swing settings and 808
behaviour — are encoded from documented production practice. These are the
publications behind those numbers. **Rules and statistics only; nothing musical
is copied from any of them.**

- **Roger Linn on swing and groove** — Attack Magazine. The MPC swing scale
  (50% straight, 54, 58, 62, 66% triplet) that the humanizer implements.
  https://www.attackmagazine.com/features/interview/roger-linn-swing-groove-magic-mpc-timing/
- **Attack Magazine** — Beat Dissected (west-coast hip-hop), *10 Snare Rolls for
  the Drop*.
- **MusicRadar** — mixed-resolution trap hi-hat programming, the six jungle/DnB
  grooves, snare-roll build-ups, realistic banjo programming.
- **audeobox** — MPC drum programming, trap and drill walkthroughs.
- **EDMProd** — the drums guide (fill conventions), liquid DnB, phonk.
- **Splice** — Memphis rap; **BVKER** — phonk.
- **BRL Theory** — J Dilla's microtiming analyses, from which the "drunk"
  quantize-strength and swing-drift figures come.
- **Drumeo** — a drummer's guide to country, for the train beat and two-beat
  patterns.
- **ujam**, **MusicTech**, **MasterClass**, **LANDR**, **emastered**,
  **Native Instruments**, **Melodigging**, **Amped Studio**, **Soundation**,
  **kickdrum.io**, **Noisegate**, **POW MAG**, **Gearspace** — genre-specific
  technique articles cited per model in each file's `sources` field.

Every model in `data/genres/` names its own sources; `datasetc stats` reports
any model that cites none.

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

Rust crates and npm packages are pinned in `Cargo.lock` and
`package-lock.json`. The dependency licence allowlist is enforced in CI by
`cargo deny`, which fails the build on any licence outside it — see
`deny.toml`.
