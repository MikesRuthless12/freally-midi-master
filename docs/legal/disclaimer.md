# Artist & Producer Name Disclaimer

*This is the canonical text. The About screen, the docs site, and the README all
quote from it — edit it here, not in three places.*

---

## Short form (About screen / UI)

> Artist and producer names in Freally MIDI Master are descriptive references to a
> musical style, nothing more. No affiliation, endorsement, or authorship is implied
> or claimed. Every pattern is generated procedurally from hand-authored style
> parameters — no MIDI, audio, or data is copied from any recording.

## Full form

### Names are used descriptively

Freally MIDI Master lets you search for an artist or producer and generate MIDI
*in the style of* their records. Those names appear solely as **descriptive
references to a recognisable musical style** — the same way a chord chart might be
labelled "bossa nova" or a drum lesson "a Bonham shuffle."

- No artist, producer, label, publisher, or rights-holder named in this software has
  endorsed, sponsored, approved, reviewed, or is affiliated with it in any way.
- No name is used as a brand, product name, or badge of origin for this software or
  for anything it generates.
- All trademarks and names are the property of their respective owners.

### What the style data actually is

Each style model in `data/` is a set of **numbers and rules** derived from published
research, music-theory analysis, interviews, public statistical datasets, and
listening notes. A model contains things like tempo ranges, swing percentages,
velocity distributions, note-density targets, hat-roll subdivision grammars, 808
slide conventions, scale and progression tendencies, and section-length templates.

A style model contains **no** copied MIDI, **no** sampled or embedded audio, **no**
transcription of any specific recording, and **no** melodic or lyrical content
extracted from any song.

### What the engine does

Generation is **100% procedural and rule-based**. The engine reads a style model,
seeds a deterministic pseudo-random generator, and constructs patterns from scratch.

- There is **no machine learning** anywhere in the product — no models, no training
  data, no inference, no neural networks of any kind.
- The engine was not trained on, and does not ingest, analyse, or reference, any
  commercial recordings, MIDI rips, or sample packs.
- There is **no feature that recreates a specific song**, and there never will be —
  this is a deliberate, permanent design decision, not a current limitation.
- Output is original by construction. A novelty guard additionally checks generated
  melodic material against a reference table of well-known hooks and rejects
  near-matches.

### Style is not copyrightable; a recording is

Copyright protects a **specific fixed work** — a particular recording and a
particular composition. It does not protect a *style*, a groove, a tempo, a drum
pattern convention, or the general feel of a genre or an era. Freally MIDI Master
operates entirely in the second category, and deliberately never touches the first.

This is a description of the software's design, **not legal advice**. You remain
responsible for the finished music you release — see `EULA.md` § 3.

### If you are a named artist, producer, or rights-holder

If you would prefer your name not appear in the roster, contact
**mythodikalone@gmail.com** and it will be removed. Removing a name does not require
a legal claim, a lawyer, or an explanation — just ask. Style models are data files;
removal is a one-line change and ships in the next dataset update.

### Attribution

Public datasets and research that contributed statistical parameters to the style
models are credited in `docs/credits.md` and in the About screen.
