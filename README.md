<div align="center">

# Freally MIDI Master

**Artist-accurate MIDI. Drag it straight into your DAW.**

Drums, melodies, countermelodies, 808s, chords — and complete song arrangements —
generated in the style of specific artists, as original MIDI.
Free, offline, and 100% AI-free.

Windows · macOS · Linux

</div>

---

> **Status: in development.** [v0.1.0](https://github.com/MikesRuthless12/freally-midi-master/releases/tag/v0.1.0)
> is the first tagged build — the foundation, the shell and the CI spine, with
> the generators deliberately disabled rather than pretending to work.
>
> Phase 1 is under way: the drum engine now generates kicks, snares, hats, the
> roll vocabulary, the 808 line and fills across fifteen genre archetypes, and
> reproduces byte-identically from a seed. It is not wired to the UI yet, so
> the installable app still cannot generate — that lands with the next release.

## What it is

Most MIDI generators think in *genres*. "Trap" is not "Metro Boomin," and no
mainstream tool has heard of OsamaSon.

Freally MIDI Master thinks in **artists**. Type a name, hit Generate, and get
original patterns that carry that artist's actual signatures — the hat-roll grammar,
the 808 slide behaviour, the swing, the way their sections are laid out — then drag
the result into FL Studio, Ableton, Logic, Reaper, or anything else.

## How it works

- **Search an artist, not a genre.** Instant fuzzy autosuggest across a mainstream
  roster and an underground roster. Genres exist as a browse filter, not the unit of
  generation.
- **Six generators.** Drums · Melody · Countermelody · Bassline · Chords, plus Song
  Mode, which lays out a full arrangement with every part filled in.
- **Audition with your own sounds.** Import `.wav`/`.mp3` one-shots as drum pad
  voices *or* as pitched instruments with root-note detection.
- **Edit what you got.** Piano-roll editor and a pad-grid drum sequencer. Lock what
  you like, reroll the rest.
- **Drag out.** Standard MIDI files or rendered audio, straight into the DAW.
- **Reproducible.** Every generation has a seed. Copy it, paste it, get it back.

## The engine is rule-based, not trained

There is **no AI in this product**. No models, no training data, no inference, no
network calls during generation. The engine is deterministic procedural code reading
hand-authored style parameters derived from published research.

That is a legal architecture as much as a technical one: nothing here is copied MIDI,
sampled audio, or a transcription of anyone's record, and there is no feature that
recreates a specific song. Artist names are descriptive style references only —
see **[docs/legal/disclaimer.md](docs/legal/disclaimer.md)**.

## Privacy

No accounts. No telemetry. Nothing about you, your projects, or your output is ever
transmitted. Generation, playback, import, and export are entirely local.

Two outbound connections exist and are documented: a launch-time **update check**
that fetches one small version file and prompts before doing anything, and **crash
reports**, which are written locally, shown to you in full, and sent only if you
click to send them. Details in [EULA.md](EULA.md) § 5.

## Building from source

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20+,
and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/) for
your platform.

```bash
npm install
npm run tauri dev      # run the app
cargo test --workspace # engine + app tests
npm run build          # typecheck + build the frontend
```

Layout: `engine/` is a pure Rust library holding all musical logic — no Tauri types,
no network, no `unsafe` — so it can be tested headless and reused later.
`src-tauri/` is the desktop shell, `src/` the React UI, and `data/` the style dataset.

## Contributing

Style research and dataset additions are genuinely welcome — see
**[CONTRIBUTING.md](CONTRIBUTING.md)**. Note that this project is source-available,
not open source: read [LICENSE](LICENSE) before you fork.

## Licensing

Proprietary, source-available, All Rights Reserved. You may read the source, build
and run it locally, and submit contributions; you may not redistribute it or ship
derivatives. See [LICENSE](LICENSE) and [EULA.md](EULA.md).

**The music you make with it is yours,** with no royalty and no attribution
requirement.

---

<div align="center">

Built by [Havoc Software](https://github.com/MikesRuthless12) ·
[Report a bug](https://github.com/MikesRuthless12/freally-midi-master/issues)

</div>
