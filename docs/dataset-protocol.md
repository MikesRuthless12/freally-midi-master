# Dataset Protocol

*How a style model gets written, checked and shipped. Phase 5 encodes 50 genre
archetypes and 500+ artists against this document; anything that disagrees with
it is a bug in one of the two.*

Read `docs/style-research.md` § Methodology first — its five hard rules are the
boundary this protocol works inside, and nothing here relaxes them.

---

## 1. What you are writing

A style model is **a set of numbers and rules describing tendencies**. It is not
a transcription, it contains no note data, and it never will. The schema says so
in its own description (`data/schema/artist-style.schema.json`), and the reason
is in `docs/legal/disclaimer.md`.

The line, stated once so it does not have to be re-derived per artist:

| Encode freely | Never encode |
|---|---|
| BPM ranges and centres of gravity | A specific song's melody, in any form |
| Swing amounts, microtiming offsets, velocity bands | Note data lifted from a MIDI file |
| Chord-progression *families* as Roman numerals | A named song's exact progression, presented as that song's |
| Drum grammars — densities, anchors, syncopation rates | Anything from a commercial MIDI pack |
| Register, contour, interval and density distributions | Anything from BitMidi/FreeMidi-class scrapes |
| Timbre *hints* — "detuned piano", "supersaw" | Samples, audio, or artist logos/images |

A Roman-numeral family like `i–VI–III–VII` is a commonplace building block and
unprotectable (Skidmore v. Led Zeppelin, 9th Cir. en banc 2020). A contour rule
that says "descends, 2-bar cell, repeats" is a tendency. Neither is a melody.

**Statistical sources are limited to the four in research ch. 5 § A** — GMD and
E-GMD (CC-BY 4.0, attribution required, see `docs/legal/attributions.md`),
McGill Billboard (CC0), Lakh aggregate statistics only, and Hooktheory trends
aggregates within its terms. MetaMIDI, GigaMIDI and MAESTRO are non-commercial
and must not be touched.

---

## 2. The encoding checklist

Eleven steps, in this order. Skipping one is how a model ends up plausible and
wrong.

### 1. Identify the lane

Which scene archetype does this artist actually work in? Research ch. 4 §
"Scene taxonomy" names eleven lanes for the underground; ch. 1–3 cover the
mainstream by genre. The lane is a musical claim, not a marketing one — Glokk40Spazz
is dark plugg out of Decatur, not Memphis, whatever the sonic influence says.

If the artist genuinely straddles two lanes, pick the one their **drums** come
from. Drums are what `extends` inherits most of, and what the roster's
distinctness test compares.

### 2. Pick `extends`

```json
"extends": ["dark-plugg"]
```

Ordered parents; later entries win over earlier ones, and the model itself wins
over all of them. Three rules:

- **A model may only extend what it is musically built from.** Extending `trap`
  because it is convenient makes every unstated field wrong, not absent.
- **The first entry is normally the artist's home genre**, and is normally also
  the first entry in `relatedGenres`.
- **Arrays replace, they do not append.** An artist that authors its own `modes`
  replaces its genre's rather than adding to them. That is deliberate — it keeps
  an artist's moods to the ones that artist actually does — but it means a
  partial `modes` array silently drops the rest.

### 3. Set the `session` block

BPM min/max/**mode**, `halfTime`, keys, scales, swing, humanize.

`mode` is the centre of gravity and matters as much as the bounds. A range of
130–160 with a mode of 152 is a different artist from the same range with a mode
of 136.

Swing lives on the MPC scale: `0.50` straight, `0.54` subtle (modern hip-hop and
R&B), `0.58` classic MPC boom bap, `0.62` heavy shuffle (neo-soul), `0.667` full
triplet. The lint rejects anything outside 0.50–0.75 because outside that range
it is almost always a typo.

### 4–8. The part deltas: drums, chords, melody, countermelody, bassline

**Author deltas, not copies.** Every field you restate at the same value as the
parent is a field that will not track the parent when the parent is corrected.
Write only what makes this artist *not* their genre.

The five questions worth answering explicitly, because they are what listeners
actually hear:

- **Drums** — what is the kick doing off the grid, and is the 808 a bassline or
  its own counter-riff? `bass808.role` is the single highest-leverage field in
  the schema.
- **Chords** — how many, how often do they change, and how coloured? `extensions`
  (triad/seventh/ninth) is what separates pluggnb from plugg in one line.

  ⛔ **A numeral names a degree, never a chord quality.** The parser accepts an
  optional `b`/`#`, roman letters in one case, and an optional `dim` — so `i`,
  `VI`, `bII` and `vii°` are numerals and **`i7`, `iv7` and `IIImaj7` are not**.
  Research ch. 2 writes families as `i7–iv7–VII7–IIImaj7` because that is how a
  musician writes them; the *model* writes `["i","iv","VII","III"]` and puts the
  sevenths in `extensions`. A numeral the parser cannot read is **dropped
  silently** — the chord simply never sounds — which is why
  `engine/tests/chords.rs` fails the build on one. Ask it, do not eyeball it.
- **Melody** — register, phrase length, density, contour, and `timbreHint`.
- **Countermelody** — `densityRatio` and whether it answers or sustains.
- **Bassline** — does it follow the roots, or is it independent?

### 9. Arrangement

`sectionBars`, `structures`, `transitions`. This is the block most often left
inherited, and it is the one that makes Song Mode sound like the artist rather
than like the app. A pop model whose chorus does not land inside 60 seconds is
not a pop model.

### 10. Aliases, tier, confidence, sources

**Aliases are how the artist gets found**, and they must include the spellings
producers actually type — Discord channel names, nicknames, and the two or three
obvious misspellings. `"ye"` for Kanye West, `"osama son"` for OsamaSon,
`"x"` for XXXTENTACION. Aliases are lowercase and never duplicated across models;
see § 5.

`confidence` is `high` / `medium` / `low`, and **a `low` model must say why in
`notes`**. `sources` cites where the parameters came from, in the compressed form
the shipped models already use — `"style-research ch.4 OsamaSon"`.

### 11. Two checks: the compiler, then the ear

```sh
npm run dataset:validate     # schema + lints + inheritance + cross-references
npm run dataset:stats        # counts, and who is missing sources
npm run dataset:coverage     # what each model declares vs inherits
```

Then the **invariant listen test**: generate on several seeds and ask whether it
is recognisably this artist and not their genre. The suite asserts the mechanical
half of that (`engine/tests/genre_invariants.rs`), but a model can pass every
assertion and still be a name with numbers attached. That is what the ear is for.

---

## 3. Quality bars per tier

`tier` is a claim about depth, and the roster's ranking uses it. Three values:

| Tier | Blocks it must declare | Distinctness bar | Sources |
|---|---|---|---|
| **flagship** | `session` + all five parts + `arrangement`, and normally `modes` | Blind A/B against its genre parent **and** against the nearest flagship in the same lane must be audibly different | Named research entry, `confidence: high` |
| **standard** | `session` + `drums` + at least two melodic blocks | Differs from its genre parent on every seed | Named research entry or a documented convention |
| **inherited** | `session` + `drums` deltas, plus aliases | Differs from its genre parent on every seed | May cite the era-genre archetype it sits on |

Every tier, without exception:

- **Passes `datasetc validate`.** This is the CI gate; there is no local override.
- **Generates distinct drums from every other shipped model.** Enforced by
  `no_two_genres_produce_the_same_drums`, which compares every pair in the roster
  — so an inherited-tier model whose only overrides are BPM and keys will fail,
  because neither reaches the drum generator. Give it at least one drum delta
  that is true of the artist.
- **Differs from its parent on all 20 seeds**, enforced by
  `every_flagship_artist_sounds_unlike_the_genre_it_extends` (which, despite the
  name, checks every artist).
- **Cites sources.** `datasetc stats` lists unsourced models by name.

Genre archetypes carry one further bar: **each needs a signature invariant test**
in `engine/tests/genre_invariants.rs`, and the guard test
`every_genre_in_the_roster_has_an_invariant_test` fails the build if you add a
genre without one. The test asserts the thing that makes it that genre — the
claim, sourced, checked over 100 seeds, because a genre is a distribution rather
than a pattern.

---

## 4. Batch workflow

Dataset work is batched because the checks are cheap per batch and expensive per
file. One batch is 15–25 models in a single lane or era.

1. **Read the research for the whole batch first.** Encoding artists one at a
   time produces a batch that agrees with itself only by accident; reading the
   lane first is what makes the deltas relative to each other.
2. **Write the models.** Deltas only, per § 2.
3. **`npm run dataset:validate`.** Fix everything before moving on — a batch with
   one failure is a batch nobody can check the rest of.
4. **`cargo test -p engine`** — distinctness, invariants, and the generator
   suites. This is where "two artists in the same lane came out identical" shows
   up.
5. **`npm run dataset:coverage`** and read the column for the batch. A row of
   `○` where you expected `●` means the deltas did not land where you thought.
6. **Listen.** Flagship tier: every model. Standard: every model. Inherited:
   a 10% sample per sub-batch, chosen across lanes rather than alphabetically.
7. **Record the batch in the roster ledger** (§ 6) and commit as one change.

---

## 5. Aliases and search

The roster is searched with `src/lib/fuzzy.ts`, over names and aliases, and
ranked with tier as a weight. Three rules that stop a 500-model roster from
becoming unsearchable:

- **No alias may resolve to two models.** Collisions are a defect, not a
  ranking problem — if `"x"` matches both XXXTENTACION and an artist literally
  named X, one of them must give it up.
- **Aliases are lowercase, punctuation-free, and never a substring the fuzzy
  matcher would find anyway.** `"metro"` earns its place because the model is
  named "Metro Boomin"; `"metro boomin"` does not.
- **Spell it how the channel spells it.** The single most common way a search
  fails is that the roster carries the label-correct spelling and the user typed
  the Discord one.

---

## 6. Roster ledger

The ledger is the record of what was encoded against which archetype — the thing
that makes "is this artist already in?" answerable without listing 500 files. It
lives in Appendix A of this document, one row per artist: id, display name,
tier, confidence and alias count, **grouped by lane** rather than alphabetically,
because lane balance is what is worth seeing at a glance. A roster with 200 trap
artists and three country ones has a hole in it, and an alphabetical list hides
that completely.

<!-- ROSTER-LEDGER:BEGIN -->
**10 artists** — 10 flagship, 0 standard, 0 inherited, across 6 lanes.

*Generated by `scripts/roster-ledger.mjs`. Do not hand-edit.*

### jerk — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `nettspend` | Nettspend | flagship | high | 2 |

### ny-drill — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `pop-smoke` | Pop Smoke | flagship | high | 3 |

### plugg — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `pierre-bourne` | Pi'erre Bourne | flagship | high | 4 |

### pluggnb — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `summrs` | Summrs | flagship | high | 2 |

### rage — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `osamason` | OsamaSon | flagship | high | 3 |

### trap — 5

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `drake` | Drake | flagship | high | 4 |
| `future` | Future | flagship | high | 3 |
| `metro-boomin` | Metro Boomin | flagship | high | 3 |
| `southside` | Southside | flagship | high | 3 |
| `travis-scott` | Travis Scott | flagship | high | 4 |

<!-- ROSTER-LEDGER:END -->

---

## 7. Regenerating the ledger

```sh
node scripts/roster-ledger.mjs         # rewrite Appendix A in place
node scripts/roster-ledger.mjs --check # fail if it is stale (the CI gate)
```

The ledger is derived, so it is regenerated rather than maintained. `--check` is
what stops it drifting from `data/`.
