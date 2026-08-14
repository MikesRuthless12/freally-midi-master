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
> It does pass both automated plugin validators — **`pluginval` at strictness
> level 5** against the VST3 and **`clap-validator`** against the CLAP — which
> checks the host contract (state save/restore, transport fuzzing, bus layouts)
> without anybody watching. That is not the same as a producer having used it.
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
- **Search an artist, not a genre.** One type-to-autocomplete box finds anyone —
  across a mainstream roster and an underground one, through aliases and past
  typos. Artists and genres sit in the same list, each row saying which it is, so
  you never have to know in advance whether "UK Drill" is a person or a style.
  Stop typing halfway and click away and it takes the best match, so you cannot
  end up with nothing chosen. What the selection *does* — its era, its genres, the
  tempo range and key it tends toward, its moods by name, and **which parts it
  writes and which it does not** — appears directly underneath, before you press
  Generate. An artist who does not write countermelodies says so, rather than
  answering that tab with silence. Genres exist as a browse filter, not the unit
  of generation.
- **Every artist has moods, not one sound.** Pick a mood beside the artist and
  stay there, or leave it on *Any* and let each press walk their range —
  boom bap's *dusty / jazzy / hard*, phonk's *cowbell / memphis / brazilian*,
  trap's *dark / bounce / melodic / minimal*. One artist writes more than one kind
  of record, and the seed alone could never cross between them.
- **Build a style of your own, and train it on what you keep.** Start from any
  artist or genre, adjust the tempo range, swing, hat density, melody density and
  scales, and save it — it appears in the roster marked *Yours* and generates,
  locks, re-rolls and exports exactly like a shipped one, because it is one. Star
  the takes you like and Train fits a style to them. **No machine learning
  anywhere in it**: it measures your generations and writes the numbers back, and
  a style that would only repeat itself is refused rather than saved. Your own
  `.mid` files can train it too.
- **Nothing copies your samples without asking.** A style can keep the one-shots
  you assigned so they survive you moving the originals — and because that is a
  second copy on your drive, it tells you how many files and how many megabytes
  first, unticked.
- **Five generators, plus Song Mode.** Drums · Melody · Countermelody · Bassline ·
  Chords — plus Song Mode, which is not a sixth generator but an arrangement that
  fills all five in. Drums has a pad grid; the other four have a piano roll.
- **A whole song, and you arrange it before it leaves.** Song Mode samples one of
  the artist's own forms, builds a clip per part per section, and draws it on a
  timeline you can edit: resize, clone, delete, lock what you like, re-roll a
  section without touching the rest, and open any clip in its own editor. It
  plays, so the marker is a position through the record rather than through one
  loop.
- **It makes a sound, without arming anything.** A sampler behind the
  generators, so every part is audible before it leaves the plugin — drums,
  melody, countermelody, bassline and chords. The plugin has its own preview
  transport, so auditioning a beat does not mean rolling the whole project;
  starting your DAW's transport takes it straight back.
- **Audition with your own sounds.** Drop your own one-shot on any drum lane or
  on melody, countermelody, bassline or chords — WAV, AIFF, FLAC, MP3, M4A or
  OGG. The synthesized kit that ships in the binary is the default, not the
  ceiling. A sample recorded at a different rate from the kit around it is
  converted once, properly filtered, when you load it — rather than stretched to
  fit on every note.
- **A file browser that behaves like one.** Your library folders sit at the top,
  their subfolders indent underneath, files below those — up to **eight folders
  as tabs**, and it all comes back next time you open the app, project or no
  project. Arrow keys walk it: `→` opens a folder, `←` shuts it, and on a file
  those same keys play it forwards and backwards. **Star anything** — sample,
  one-shot or MIDI — and it joins a list; click a starred name and the tree opens
  its way down to it, or opens Explorer or Finder if that folder is no longer one
  of your eight.
- **It holds a real sample library.** A folder with two thousand files in it
  opens as fast as one with four, because only the rows you can see are drawn.
  **Type to filter** and the tree narrows to what matches, keeping the folders
  that lead to it — and it says which folders it searched, because it can only
  look inside the ones you have opened. Unplug the drive a library folder lives
  on and its tab says so instead of sitting there refusing to open.
- **Hear a `.mid` before you decide what to do with it.** Press Play on a MIDI
  file and it sounds, through a plain built-in instrument rather than whichever
  artist you happen to have selected — so the same file sounds the same tomorrow.
  It never touches your project's transport.
- **Drop a MIDI file in and it works out what is in it.** Drag a `.mid` onto a
  generator and its notes land there; drop it on the Song tab and the whole file
  arrives as an arrangement you can take parts out of, cell by cell, without
  overwriting anything until you choose to. A layered file separates into bass,
  melody, countermelody, chords and drums — and **each part tells you why it was
  routed there**, so a wrong guess is one click to redirect rather than something
  you find out later.
- **Eight pads across the top, and everything on their face.** Each is a drum
  lane: its name, what is on it, and a dot — green if you can hear it, red if you
  cannot. Press the pad to mute it, press Play in its middle to hear that sound
  alone, drag a sample onto it, or clear it back to the built-in. Every pad's name
  is a picker over all thirty-seven lanes, **two pads may share one so you can
  layer a snare**, and the layout is remembered *per artist* — a style you built
  comes back exactly as you left it, with the sounds you gave it.
- **Re-roll a pad from the folder you are browsing.** A shuffle on any lane pulls
  a new sample from the folder open in the browser, matched to what that lane is —
  a snare gets snares — and one re-roll does the whole kit. Locked pads are left
  alone. **Name a kit and it is there next time**, in any song: save, load,
  duplicate or delete it, and it stores the paths rather than copying anybody's
  audio around.
- **Edit what you got.** Piano-roll editor and a pad-grid drum sequencer, with
  solo, mute and per-lane audition on every drum row — click a lane's name to
  hear that pad on its own. **Lock what you like and reroll the rest**: a locked
  lane comes back note for note however many times you press Generate.
- **Any drum row opens into a pitch lane.** Trap lives on 808s that move, and a
  drum grid that draws them as on/off cannot say what they are playing. Open a
  row and it becomes seven — the lane's root, three semitones either side — and
  a hit is something you drag to a pitch rather than a value the grid threw
  away. Drag past the edge and the window follows, so the reach is the whole of
  MIDI while the row stays seven tall. **Every lane, not just the 808**: on the
  808 the rows are notes, and everywhere else they are the sample transposed, so
  a pitched hat roll or a tuned tom fill is the same gesture.
- **It will not hand you somebody else's hook.** Every melody and countermelody
  is screened against a table of well-known contours before you hear it — on the
  Melody tab and inside every section of a Song Mode arrangement — and a take
  that matches is thrown away and drawn again. The table holds one-way
  fingerprints and no note data at all — transposing a hook does not hide it from
  the screen, and nothing anybody else wrote is in this repository.
- **Keep the ones you like.** Name a pattern and it is there next time, in any
  song and any DAW — saved as *notes*, with no kit, so you can load it and put
  whatever sounds you like underneath. And every take of the session is kept:
  step back through them and the whole setup comes with it — artist, mood, seed,
  bars and pins, with the tempo and key that were actually used. **And it
  outlives the session**: click the take counter and every generation you have
  ever made is there, grouped by artist, each one saying how long and how fast it
  was and when you made it. Pick one and it comes back — the beat from Tuesday
  night whose seed you never wrote down.
- **Export.** The whole arranged song as one multi-track Standard MIDI file, or
  one file per part into a folder, through your platform's own Save As.
- **Drag it out.** Pick a part up and drop it straight onto a DAW track, as MIDI
  or as audio — or turn on **Per lane** and drag just the hats out. The whole
  arrangement drags too. Files arrive named the way you would name them
  (`trap - Snare - 140 BPM - C# Minor`), and the clip's own notes ride on the
  cursor so you can see what you picked up. Click a part's **MIDI** or **Audio**
  chip and you get a menu of every instrument playing in it — drag just the hats,
  or **All Tracks** to take every lane out at once as separate files. Hold
  **Ctrl** as you drop to stack them instead of laying them end to end.
  ⚠ **Windows is the one a human has actually dropped into Ableton.** An HTML5
  drag inside a plugin's webview is not an operating-system file drag, so each
  platform needs its own native drag source; macOS (`NSDraggingSession`) and
  Linux (GTK, `text/uri-list`) are written and switched on, but **neither has
  been dropped into a real DAW yet.** Export works everywhere regardless.
- **Reproducible.** Every generation has a seed. Copy it, paste it, get it back.
- **Unlimited undo/redo.** `Ctrl`/`Cmd`+`Z` steps back through every change —
  artist, seed, bars, pins and each generation — with no depth limit, because an
  undo entry is the inputs rather than the notes.

## The road to 1.0

One build per rung, each landing a whole capability. Nothing ships half-done.

| | |
|---|---|
| `v0.3.0` | **Plugin pivot** — CLAP, VST3 and AU; loads in Ableton and FL Studio *(now)* |
| `v0.4.0` | **Unlimited undo** *(landed)* |
| `v0.5.0` | **It makes a sound** — sampler, your own one-shots, a transport of its own |
| `v0.6.0` | **Your sample library** — explorer, preview, drag-to-pad, reverse |
| `v0.7.0` | **The arrangement, like a DAW** — clips drawn from their notes, drag-out per part |
| `v0.8.0` | **50 genres, 500 artists** — the full roster, underground beside mainstream |
| `v0.9.0` | **Polish and hardening** — accessibility, HiDPI, crash safety, cold start |
| `v1.0.0` | **Downloads open** — every format, all three platforms, the full QA matrix |

⛔ **There is nothing to download until 1.0.** No beta, no preview build. A plugin
that half-works inside somebody's session is worse than one that is not there.

## The engine is rule-based, not trained

There is **no AI in this product**. No models, no training data, no inference, no
network calls during generation. The engine is deterministic procedural code reading
hand-authored style parameters derived from published research.

⛔ **This holds for "train your own style" too, and the word is worth being exact
about.** Training here is **parameter fitting**: the app measures the takes you
kept — how many onsets a bar, what register, what shape the line made — and
writes those ranges back as an ordinary style model, the same kind of file the
shipped artists are. There is no model, nothing is learned from anybody else's
music, and nothing leaves your machine. A build with an AI or an HTTP client
anywhere in its dependency graph fails CI: `scripts/check-denylist.mjs` walks
every resolved crate and every production npm package on every push.

That is a legal architecture as much as a technical one: nothing here is copied MIDI,
sampled audio, or a transcription of anyone's record, and there is no feature that
recreates a specific song. Artist names are descriptive style references only —
see **[docs/legal/disclaimer.md](docs/legal/disclaimer.md)**.

## Privacy

No accounts. No telemetry. Nothing about you, your projects, or your output is ever
transmitted. Generation, playback, import, and export are entirely local.

**The plugin makes no outbound connections at all.** The two that used to exist — a
launch-time update check and an opt-in crash report — belonged to the desktop shell
and were removed with it; a plugin is installed and updated by whoever installs
plugins, and the host owns that. `scripts/check-denylist.mjs` gates this rather than
asserting it: no HTTP client is linked into the binary at all, so the allowlist that
used to carry `reqwest` and `hyper` is now empty. Details in [EULA.md](EULA.md) § 5.

## Building from source

Prerequisites: [Rust](https://rustup.rs) (the version in `rust-toolchain.toml`)
and [Node.js](https://nodejs.org) 20+. On Linux the editor additionally needs
WebKitGTK and the X11 development headers — `.github/workflows/ci.yml` lists the
exact `apt` packages the runners install.

```bash
npm install
npm run plugin:standalone   # the plugin in its own window, no DAW needed
npm run plugin:build        # release .clap, with the bundled-content gate
npm run plugin:install      # symlink it into the CLAP folder — once
npm run ci:local            # every gate, with CI's own environment
```

The release standalone is `target/release/standalone.exe` and can simply be
double-clicked: it supplies its own audio period size rather than needing a flag,
and opens no console window behind itself. If it ever dies, it appends a stack
trace to `%APPDATA%\Freally MIDI Master\standalone-crash.log` — and
`NIH_LOG=some\file.log` brings the whole log back without a rebuild.

**`npm run build` must run before any cargo build**, because the plugin compiles
the built UI and the dataset into its binary — a plugin has no resource directory
to read them from. `npm run plugin:build` does both in order, and
`scripts/assert-plugin-bundled.mjs` refuses a binary missing either, so the
failure is a message rather than a blank window.

Layout: `engine/` is a pure Rust library holding all musical logic — no shell
types, no network, no `unsafe` — which is why it survived the pivot untouched.
`plugin/` is the CLAP plugin, `src/` the React UI it embeds, and `data/` the style
dataset.

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
