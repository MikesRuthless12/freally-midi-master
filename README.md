<div align="center">

# Freally MIDI Master

**Artist-accurate MIDI, generated onto the track you are already working on.**

Drums, melodies, countermelodies, 808s, chords — and complete song arrangements —
generated in the style of specific artists, as original MIDI, **in your project's
own key and tempo**.
Free, offline, and 100% AI-free.

CLAP · VST3 · AU — Windows · macOS

</div>

---

> **Status: in development. It is a plugin.**
>
> Freally MIDI Master began as a desktop app; `v0.2.0` was that build. **As of
> 2026-07-29 the desktop app is retired** — it is no longer built or released.
> Nothing already installed stops working; it simply stops finding updates.
>
> **There are no downloads, and there will not be until `v1.0.0`.** No beta, no
> preview build, no early access — the desktop releases have been withdrawn and
> the plugin has not shipped one. A generator that half-works inside somebody's
> session is worse than one that is not there, so it ships when the QA matrix is
> green on every host and every OS. **Until then the only way to run it is to
> build it** — see [Building from source](#building-from-source), which is a
> supported path rather than a workaround.
>
> **It is a CLAP plugin, with VST3 and AU projected from it.** Not for the
> format's sake: a plugin is handed the host's tempo, time signature and
> playhead, so a generated pattern lands in the song you are actually writing
> instead of at whatever tempo the artist happens to be authored at.
>
> It has been loaded and generates in **Ableton Live 12** (VST3) and **FL Studio**
> (CLAP). Reaper, Bitwig and Logic are not yet tested, and Linux has no editor
> yet.
>
> The `engine` crate — the dataset, inheritance, the drum engine, the chords
> generator, humanize, the MIDI writer — carried across unchanged. That was the
> point of keeping it free of shell types from day one.
>
> **Documentation:** <https://mikesruthless12.github.io/freally-midi-master/> —
> what it does, how it follows the host, seeds, presets, shortcuts, privacy and
> building from source. Source for the site is in [`docs/`](docs/).

## What it is

Most MIDI generators think in *genres*. "Trap" is not "Metro Boomin," and no
mainstream tool has heard of OsamaSon.

Freally MIDI Master thinks in **artists**. Type a name, hit Generate, and get
original patterns that carry that artist's actual signatures — the hat-roll grammar,
the 808 slide behaviour, the swing, the way their sections are laid out — then drag
the result into FL Studio, Ableton, Logic, Reaper, or anything else.

## How it works

- **It follows your DAW.** The plugin reads the host's tempo and time signature
  every block, so a pattern is generated *for your song* — trap authored at 140
  comes out at 92 in a 92 BPM project. Pin a tempo yourself and yours wins; clear
  it and the project decides again.
- **The notes land on the track.** No file to drag, no folder to find — the
  plugin emits them where you inserted it.
- **And you can hear it straight away.** A preview kit is built in, so pressing
  Generate and playing your project makes a sound without wiring an instrument
  up first. One switch turns it off again and hands the notes to your own drum
  sampler, because MIDI-only is a way of working rather than a fallback.
- **Search an artist, not a genre.** Instant fuzzy autosuggest across a mainstream
  roster and an underground roster. Genres exist as a browse filter, not the unit of
  generation.
- **Five generators, plus Song Mode.** Drums · Melody · Countermelody · Bassline ·
  Chords — plus Song Mode, which is not a sixth generator but an arrangement that
  fills all five in. Drums is the one with an editor today; the other four are
  wired through the plugin's engine bridge and surface as the piano roll lands.
- **Audition with your own sounds.** Import `.wav`/`.mp3` one-shots as drum pad
  voices *or* as pitched instruments with root-note detection.
- **Edit what you got.** Piano-roll editor and a pad-grid drum sequencer. Lock what
  you like, reroll the rest.
- **Drag out.** Standard MIDI files or rendered audio, straight into the DAW.
- **Reproducible.** Every generation has a seed. Copy it, paste it, get it back.
- **Unlimited undo/redo.** `Ctrl`/`Cmd`+`Z` steps back through every change —
  artist, seed, bars, pins and each generation — with no depth limit, because an
  undo entry is the inputs rather than the notes.

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

Prerequisites: [Rust](https://rustup.rs) (the version in `rust-toolchain.toml`)
and [Node.js](https://nodejs.org) 20+. The desktop shell additionally needs the
[Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/).

```bash
npm install
npm run plugin:standalone   # the plugin in its own window, no DAW needed
npm run plugin:build        # release .clap, with the bundled-content gate
npm run plugin:install      # symlink it into the CLAP folder — once
npm run ci:local            # every gate, with CI's own environment
```

**`npm run build` must run before any cargo build**, because the plugin compiles
the built UI and the dataset into its binary — a plugin has no resource directory
to read them from. `npm run plugin:build` does both in order, and
`scripts/assert-plugin-bundled.mjs` refuses a binary missing either, so the
failure is a message rather than a blank window.

Layout: `engine/` is a pure Rust library holding all musical logic — no shell
types, no network, no `unsafe` — which is why it survived the pivot untouched.
`plugin/` is the CLAP plugin, `src/` the React UI shared by both shells,
`src-tauri/` the desktop shell, and `data/` the style dataset.

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

The plugin shows the agreement the first time it opens and will not generate,
play, export or save until it is accepted. Declining leaves it inert rather than
closing your DAW; reopen the agreement and accept, and it works immediately.

---

<div align="center">

Freally MIDI Master · By [Mike Weaver](https://github.com/MikesRuthless12) ·
[Report a bug](https://github.com/MikesRuthless12/freally-midi-master/issues)

</div>
