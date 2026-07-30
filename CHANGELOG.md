# Changelog

All notable changes to Freally MIDI Master.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **The release job extracts the tagged version's section from this file** and
> uses it as the updater's release notes. Match the heading format exactly —
> `## [0.1.0] - YYYY-MM-DD`. A missing section means every user sees a generic
> note instead of what actually changed.

## [Unreleased]

## [0.4.0] - 2026-07-29

### Added — unlimited undo/redo, and the licence gate

- **Unlimited undo/redo** (FMM-U01). Every session change — artist, seed, bars,
  pins, auto-sync and each generation — steps back with `Ctrl`/`Cmd`+`Z` and
  forward with `Ctrl`/`Cmd`+`Shift`+`Z` or `Ctrl`+`Y`. No depth limit: an entry
  is a handful of scalars plus a *shared* `Pattern` reference, because a pattern
  is derived from its seed rather than stored, so a hundred steps across one
  generation cost one pattern. A run of edits to one control inside 600 ms
  collapses into a single step; two generations never merge, however fast the
  reroll. Recorded by a store subscription rather than per-action calls, so a
  future action cannot forget to register — the same argument the session save
  already made. Armed *after* the project restore, so `Ctrl`+`Z` cannot step
  behind the session the host handed back onto an empty plugin.
- **First-run licence gate.** The agreement is compiled in from
  `EULA.md` and shown before anything else; nothing generates, plays, exports or
  saves until it is accepted. **Agree stays disabled until the text has been
  scrolled to the end**, because "you cannot use it until you read it" is the
  requirement and a live button asks nobody to read anything. ⛔ Enforced at the
  plugin's RPC boundary, not in the UI — a page that was reloaded, bypassed or
  driven from devtools still cannot generate — and by an *allowlist*, so a new
  command has to be added deliberately to be reachable before acceptance.
  **Decline leaves the plugin inert rather than closing the DAW**, which a plugin
  must never do; the agreement can be reopened and accepted at any time, and
  everything works immediately after. Acceptance is stored per user, beside the
  presets — never in the project file, which would ask a collaborator to
  re-accept because they opened your song.
- **The standalone carries the app icon on Windows**, with product, company and
  copyright strings, via a new `plugin/build.rs`. A missing resource compiler
  warns rather than failing the build.
- **A documentation site** under `docs/`: what the
  plugin does, how it follows the host, seeds, presets, shortcuts, privacy and
  building from source, with search and a short changelog.

### Fixed

- **The Windows standalone opened a blank window** (TASK-P16). `baseview`'s
  `open_blocking` pumps with `GetMessageW(&mut msg, hwnd, …)` — a non-NULL
  `hwnd`, which retrieves messages only for that window and its children and
  **never retrieves thread messages at all**. WebView2 is COM/STA and delivers
  its completions as exactly those, through a COM-owned message-only window, so
  the custom-protocol handler was never dispatched, navigation never completed
  and the page stayed on `about:blank`. The vendored adapter now drains the queue
  from `on_frame`, **off unless the process opts in** — `plugin/src/bin/standalone.rs`
  is the only caller and a DAW never runs it, so a host's queue is never touched.
  ⛔ It skips messages belonging to the editor window and its children:
  dispatching those re-enters baseview's window procedure while it already holds
  a `RefCell` borrow, which panics inside an `extern "system"` frame and aborts
  the process. Verified by photographing the window, not by a green build.

### Changed

- **No downloads before `v1.0.0`.** The `v0.1.0` and `v0.2.0` desktop releases
  were withdrawn from GitHub (converted to drafts; assets intact), and the
  documentation site offers no download link. Building from source is the only
  supported way to run it until 1.0.

## [0.3.0] - 2026-07-29

### Added — melodic generation, and moods that multiply it

- **Melody generator** (TASK-035, FR-005). Phrase structures (riff loop,
  question/answer, call/response, long arc), chord-tone bias on strong beats,
  colour tones, interval and contour distributions, octave jumps, end variation,
  and the per-genre devices: rage's two-to-three-note staccato motif, drill's
  snare-mirrored onsets and doubled voicing, straight-eighth bars and deliberate
  silence. Pitches are chosen as **scale degrees** and only then made into MIDI
  notes, so staying in the key is structural rather than filtered for.
- **Countermelody generator** (TASK-036, FR-006). Octave echo, bell echo,
  arpeggio, answer lick and sustained pad, placed in the melody's gaps by
  construction rather than by filtering afterwards.
- **Moods** (TASK-040V, engine half). A model may author named `modes` — trap
  ships dark, bounce, melodic and minimal — each a partial override merged into
  the model *before* generation, so every generator honours a mood without
  knowing moods exist. Moods inherit through `extends`, so an artist offers only
  the moods its own lineage does.
- **Presets the plugin owns** (TASK-P13, session half). Six factory presets
  compiled into the binary, user presets in the platform's per-user data
  directory, and a panel in the right rail. No factory preset pins a tempo — that
  would override your DAW on load.
- **The Linux editor** (TASK-P12). The plugin's window now opens on Linux over
  X11 and WebKitGTK, verified by photographing it under Xvfb rather than by a
  green build.

### Fixed

- **A repeated melody no longer clashes with the chords it repeats over.** A riff
  now follows the progression, keeping its own contour and rhythm.
- **The countermelody is no longer silent about half the time.** An octave echo
  is delayed, because an octave copy at zero delay is a doubling.
- **Sustained pads voice more than the chord root**, and pick their octave.
- **`echoOffset: "1/8"` is read.** The note-value parser knew `"8th"` and `"16T"`
  and silently ignored the third spelling the dataset uses.
- **The seed box shows a whole seed.** It was 12 characters wide against a
  20-digit `u64`, so a long seed was cut off — and a seed you cannot read is one
  you cannot type back in.

### Changed — Freally MIDI Master is becoming a plugin

Decided 2026-07-28. Not for the format's sake: a plugin is handed the host's
tempo, time signature and playhead, so a generated pattern lands in the song you
are actually writing rather than at whatever tempo the artist is authored at.
`docs/product-roadmap.md` carries the decision, what survived and what did not.

- **New `plugin/` crate** on `nih-plug`, exporting **CLAP**. VST3 and AU are
  projected from it by `clap-wrapper` at packaging time.
- **The `engine` crate is unchanged**, which was the point of keeping it free of
  shell types. No FFI and no C++.
- **Host tempo sync**, with precedence **user pin > host > model**. Trap
  authored at 140 generates at 92 inside a 92 BPM project; a pinned tempo beats
  the host; a host that has not reported yet leaves the model its own value.
- **Notes are emitted onto the host's track**, replacing drag-out.
- **The session is saved with the project.** Artist, seed, pins, bars and the
  window size go through the host's own state calls — there is no settings file
  and no path to find, and a session belongs to a *song* rather than to a
  machine. The notes are not saved; the inputs that make them are, because the
  engine is deterministic and a project file should not carry regenerable notes.
- **The window has three scales** (small, medium, large) on a button in the
  transport bar. The layout stays 1440x900 at all of them — the window is drawn
  smaller, rather than shown less of — so the kit and session panels never
  disappear just because the window shrank.
- **Verified in Ableton Live 12 (VST3) and FL Studio (CLAP).** FL is the first
  host to load the `.clap` itself rather than the projection.
- **Releases now carry the plugin**, as a per-platform zip holding both the
  `.clap` and the `.vst3`, validated by `clap-validator` before the draft is
  published and refused by `verify-downloads.yml` if either format is missing.

### Removed — the desktop app is retired

Freally MIDI Master ships as a **plugin** now. Releases from here on carry the
CLAP and the VST3 and nothing else; the Windows, macOS and Linux installers are
no longer built.

**If you are on v0.2.0:** nothing you have installed stops working, and nothing
is uninstalled. Your copy will simply stop finding updates, because there will
not be any — the update channel goes quiet rather than breaking. To carry on,
install the plugin from this release and load it in your DAW, which is where
the tempo sync, the host key and the notes-on-the-track live. That is the whole
reason for the move: the desktop app could not know what song you were writing.

### Known issue

- **A corrupt project file can abort the host.** `nih-plug`'s CLAP state loader
  reads a length prefix straight into an allocation with no sanity check, so
  malformed state aborts the process rather than failing to load. It is upstream's
  bug, the maintained fork carries it identically, and it needs a patched fork to
  fix. `clap-validator`'s `state-invalid-random` is excluded by name until then.
- **The UI carried across.** `src/lib/ipc.ts` was always the one seam and gained
  a third branch; the React app, the 18 locale catalogs and the design tokens
  are the same ones the desktop app shipped.

### Added

- **Session chips** — BPM, key, scale and swing, editable in the right rail.
  Empty means the artist decides; a value means you do. When running in a host
  the tempo chip follows the DAW and says so.
- **The chords generator** (FR-004): progression families, diatonic
  third-stacking so every pitch is in the key by construction, borrowed chords,
  sus and the drill middle-note drop, close and open voicings, and syncopated
  3–5 beat cells.
- **`scripts/assert-plugin-bundled.mjs`** — refuses a plugin binary whose UI or
  dataset failed to embed, because that failure otherwise presents as a blank
  window with no error.
- **`npm run plugin:standalone`** — the plugin in its own window, no DAW needed.
  **`npm run plugin:install`** symlinks it into the CLAP folder so a rebuild is
  live without copying.

### Fixed

- **`vst3-sys` is GPLv3** and nih-plug's VST3 export links it — which would have
  put this proprietary product in breach. Caught by `cargo deny`. VST3 now comes
  from `clap-wrapper` (MIT) instead. Steinberg's own VST3 SDK went MIT in
  November 2025; nih-plug does not use it.
- **The generation error message has never been visible.** `.stage__error` had
  no CSS rule and sat behind the FX layer, which is `position: absolute;
  inset: 0` over the whole stage.
- **A pinned tempo is clamped** to Ableton Live's 20–999 at the IPC edge, and
  the BPM box accepts digits only — `<input type="number">` accepts `e`, `E`,
  `+` and `-`, so "1e5" was a legal tempo.
- `.github/workflows/{ci,release}.yml` were failing `format:check` on `main`.

## [0.2.0] - 2026-07-25

Phase 1: the app makes beats. Search an artist, press Generate, hear it, and
drag it into a DAW.

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
- Generation and export are reachable from the app: `generate_pattern` runs a
  style model through the drum generator and the humanizer and hands back the
  pattern, and `export_midi` writes it into the session directory as a type-0
  MIDI file. A request may pin the tempo, key, scale, swing, bar count or
  half-time feel; anything it leaves alone is the model's own choice, and
  omitting the seed picks a fresh one that reproduces the result exactly.
- Ten flagship artist models over the genre bases — Metro Boomin, Southside,
  Pierre Bourne, OsamaSon, Nettspend, Summrs, Pop Smoke, Travis Scott, Future
  and Drake — each with aliases to search by, and a test that every one of them
  generates something its parent genre does not.
- **The app makes beats.** Search an artist, press Generate, and the pattern
  appears in the drum grid; press Play and you hear it; drag it into a DAW or
  export it to a folder. Search is fuzzy and forgiving — "osa" finds OsamaSon,
  "drizzy" finds Drake, and a typo still lands — with the keyboard alone:
  ↑↓ to move, Enter to take, Esc to close.
- Playback: a real-time audio engine with a synthesized preview kit, sample-
  accurate sequencing, looping, a playhead that follows along, and a limiter so
  a dense pattern never cracks the output. A machine with no sound card still
  generates and exports, and says why playback is unavailable rather than
  failing silently on click.
- The seed is shown after every generation and can be pasted back to get the
  same beat again.
- A generation ripple sweeps the grid while a pattern is built, so a beat
  arrives rather than blinking into place, igniting brightest where the notes
  actually land. Turning on the system's reduced-motion setting replaces it with
  a short crossfade — immediately, without a restart.
- Settings now says when a style model was skipped at startup, with the file
  and the reason. A skipped model is a missing artist, and until now only the
  console ever mentioned it.
- Unplugging an audio interface mid-session no longer leaves the app silently
  deaf. It says the device is gone, reopens one by itself as soon as there is
  one to open — retrying for as long as it takes — and says when it is back.
  Playback does not restart on its own; pressing play works again, which was
  the thing that stopped working.
- A **Reduce motion** setting under Appearance, for machines whose system
  offers no such preference, or anyone who wants everything else animated and
  this one thing still. It takes effect the moment it is ticked.

### Changed

- Exported MIDI now carries a key signature. It was the one session field the
  file did not describe, so a clip landed in a project without saying what key
  its 808 was in. A mode is written as its parallel major or minor, which is as
  much as the format can say. **The golden `.mid` snapshots were regenerated for
  this**: six bytes per file, the new meta event and nothing else — the pattern
  JSON is untouched.

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
