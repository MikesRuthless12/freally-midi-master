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

### Which folder a model goes in

⛔⛔ **Three folders, and the split between the first two is not cosmetic**
(2026-08-12). Mike: *"just put the producers only in there instead of having
artists and producers all in one folder, so that way i know who is who and so
does anyone else looking at my code"*, and then *"ensure that when you add
volumes within the code for the tasks in the roadmap that it splits it into
those 2 separate folders."*

| folder | `"type"` | what lives there |
|---|---|---|
| `data/artists/` | `"artist"` | someone who fronts the record |
| `data/producers/` | `"producer"` | someone who makes it |
| `data/genres/` | `"genre"` | the archetype an artist or producer extends |

**The folder and the `type` field must agree**, and
`every_model_sits_in_the_folder_its_type_names` in `engine/tests/dataset.rs`
fails the build when they do not. Two records of one fact is a drift risk taken
on deliberately: `files::scan` recurses and throws the path away before the
loader ever sees it, so the *folder* is invisible at runtime and the *field* is
invisible to a person reading the repo. Neither can be dropped, so they are made
to agree instead.

▶ **Which is which is not a judgement call — it is already written down.** Every
volume of `docs/style-research-extended*.md` splits each genre's entries under
`### ARTISTS` and `### PRODUCERS` headings. Encode from those sections; do not
re-decide. The one exception on record is the ten flagships that predate volume
1 and appear in no volume — Metro Boomin, Southside and Pi'erre Bourne were
filed as producers, the other seven as artists.

⚠ **Anything reading one folder is a bug waiting for the next volume.** The
split broke nothing at load time and silently halved two things that read
`data/artists` directly: `scripts/roster-ledger.mjs`, which would have gone on
describing 344 of 534 models and calling it the roster, and
`the_first_genre_an_artist_works_in_is_the_one_it_is_built_from`, which would
have stopped covering 190 of them. Both now read `STYLE_DIRS`/`styleDirs`.

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
**1250 models** — 802 artists and 448 producers; 321 flagship, 929 standard, 0 inherited, across 31 lanes.

*Generated by `scripts/roster-ledger.mjs`. Do not hand-edit.*

### atl-swag-rap — 15

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `fast-life-yungstaz` | Fast Life Yungstaz (F.L.Y.) | flagship | low | 6 |
| `ke-on-the-track` | K.E. on the Track | flagship | low | 5 |
| `roscoe-dash` | Roscoe Dash | flagship | medium | 5 |
| `sahbabii` | SahBabii | flagship | medium | 5 |
| `sk8star` | sk8star | flagship | high | 3 |
| `soulja-boy` | Soulja Boy | flagship | low | 6 |
| `teezus` | Teezus | flagship | high | 4 |
| `dj-burn-one` | DJ Burn One | standard | medium | 3 |
| `maaly-raw` | Maaly Raw | standard | medium | 2 |
| `quay-global` | Quay Global | standard | low | 2 |
| `rich-kidz` | Rich Kidz | standard | medium | 6 |
| `ricky-racks` | Ricky Racks | standard | medium | 2 |
| `two-9` | Two-9 | standard | low | 8 |
| `vybe-beatz` | Vybe Beatz | standard | medium | 2 |
| `yung-bans` | Yung Bans | standard | low | 4 |

### boom-bap — 124

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `9th-wonder` | 9th Wonder | flagship | medium | 5 |
| `a-tribe-called-quest` | A Tribe Called Quest | flagship | medium | 5 |
| `apollo-brown` | Apollo Brown | flagship | medium | 3 |
| `big-l` | Big L | flagship | medium | 3 |
| `big-pun` | Big Pun | flagship | medium | 4 |
| `busta-rhymes` | Busta Rhymes | flagship | medium | 4 |
| `camron` | Cam'ron | flagship | medium | 6 |
| `chance-the-rapper` | Chance the Rapper | flagship | medium | 4 |
| `childish-gambino` | Childish Gambino | flagship | medium | 4 |
| `clipse` | Clipse | flagship | medium | 5 |
| `common` | Common | flagship | medium | 4 |
| `da-beatminerz` | Da Beatminerz | flagship | medium | 5 |
| `daringer` | Daringer | flagship | medium | 3 |
| `de-la-soul` | De La Soul | flagship | medium | 6 |
| `dj-muggs` | DJ Muggs | flagship | medium | 4 |
| `dj-premier` | DJ Premier | flagship | high | 5 |
| `epmd` | EPMD | flagship | medium | 4 |
| `erick-sermon` | Erick Sermon | flagship | medium | 4 |
| `freddie-gibbs` | Freddie Gibbs | flagship | medium | 5 |
| `fugees` | Fugees | flagship | medium | 5 |
| `gang-starr` | Gang Starr | flagship | high | 4 |
| `ghostface-killah` | Ghostface Killah | flagship | medium | 5 |
| `gza` | GZA | flagship | medium | 4 |
| `havoc` | Havoc | flagship | medium | 4 |
| `irv-gotti` | Irv Gotti | flagship | medium | 4 |
| `j-cole` | J. Cole | flagship | medium | 5 |
| `j-dilla` | J Dilla | flagship | high | 5 |
| `jadakiss` | Jadakiss | flagship | medium | 4 |
| `jay-z` | JAY-Z | flagship | high | 6 |
| `joey-badass` | Joey Bada$$ | flagship | medium | 5 |
| `krs-one` | KRS-One | flagship | medium | 5 |
| `large-professor` | Large Professor | flagship | medium | 5 |
| `lauryn-hill` | Lauryn Hill | flagship | medium | 4 |
| `lil-kim` | Lil' Kim | flagship | medium | 5 |
| `mac-miller` | Mac Miller | flagship | medium | 5 |
| `madlib` | Madlib | flagship | medium | 5 |
| `method-man` | Method Man | flagship | medium | 5 |
| `mobb-deep` | Mobb Deep | flagship | high | 3 |
| `mop` | M.O.P. | flagship | medium | 5 |
| `mos-def` | Mos Def | flagship | medium | 5 |
| `nas` | Nas | flagship | high | 4 |
| `naughty-by-nature` | Naughty by Nature | flagship | medium | 4 |
| `notorious-big` | The Notorious B.I.G. | flagship | high | 5 |
| `ol-dirty-bastard` | Ol' Dirty Bastard | flagship | medium | 5 |
| `pete-rock` | Pete Rock | flagship | high | 4 |
| `prince-paul` | Prince Paul | flagship | medium | 3 |
| `puff-daddy` | Puff Daddy | flagship | medium | 5 |
| `pusha-t` | Pusha T | flagship | medium | 5 |
| `q-tip` | Q-Tip | flagship | medium | 5 |
| `raekwon` | Raekwon | flagship | medium | 5 |
| `redman` | Redman | flagship | medium | 4 |
| `rick-rubin` | Rick Rubin | flagship | medium | 4 |
| `roc-marciano` | Roc Marciano | flagship | medium | 4 |
| `rza` | RZA | flagship | high | 5 |
| `statik-selektah` | Statik Selektah | flagship | medium | 4 |
| `talib-kweli` | Talib Kweli | flagship | medium | 4 |
| `the-bomb-squad` | The Bomb Squad | flagship | medium | 6 |
| `the-game` | The Game | flagship | medium | 4 |
| `the-pharcyde` | The Pharcyde | flagship | medium | 4 |
| `the-roots` | The Roots | flagship | medium | 4 |
| `trackmasters` | Trackmasters | flagship | medium | 5 |
| `twista` | Twista | flagship | medium | 4 |
| `westside-gunn` | Westside Gunn | flagship | medium | 4 |
| `wu-tang-clan` | Wu-Tang Clan | flagship | high | 5 |
| `action-bronson` | Action Bronson | standard | medium | 4 |
| `ayatollah` | Ayatollah | standard | medium | 2 |
| `az` | AZ | standard | medium | 3 |
| `beanie-sigel` | Beanie Sigel | standard | medium | 5 |
| `benny-the-butcher` | Benny the Butcher | standard | medium | 5 |
| `bink` | Bink! | standard | medium | 4 |
| `black-sheep` | Black Sheep | standard | medium | 4 |
| `boldy-james` | Boldy James | standard | medium | 4 |
| `buckwild` | Buckwild | standard | medium | 4 |
| `camp-lo` | Camp Lo | standard | medium | 5 |
| `capone-n-noreaga` | Capone-N-Noreaga | standard | medium | 5 |
| `cassidy` | Cassidy | standard | medium | 4 |
| `chucky-thompson` | Chucky Thompson | standard | medium | 4 |
| `conway-the-machine` | Conway the Machine | standard | medium | 5 |
| `cormega` | Cormega | standard | medium | 3 |
| `crucial-conflict` | Crucial Conflict | standard | medium | 5 |
| `d-dot-angelettie` | Deric "D-Dot" Angelettie | standard | medium | 4 |
| `dame-grease` | Dame Grease | standard | medium | 4 |
| `dead-prez` | dead prez | standard | medium | 5 |
| `denaun-porter` | Denaun Porter | standard | medium | 5 |
| `diamond-d` | Diamond D | standard | medium | 4 |
| `digable-planets` | Digable Planets | standard | medium | 5 |
| `dmx` | DMX | standard | medium | 3 |
| `easy-mo-bee` | Easy Mo Bee | standard | medium | 4 |
| `eve` | Eve | standard | medium | 4 |
| `fabolous` | Fabolous | standard | medium | 5 |
| `fat-joe` | Fat Joe | standard | medium | 4 |
| `foxy-brown` | Foxy Brown | standard | medium | 5 |
| `freeway` | Freeway | standard | medium | 5 |
| `guru` | Guru | standard | medium | 4 |
| `hi-tek` | Hi-Tek | standard | high | 4 |
| `ja-rule` | Ja Rule | standard | medium | 4 |
| `jeru-the-damaja` | Jeru the Damaja | standard | medium | 4 |
| `jim-jones` | Jim Jones | standard | medium | 4 |
| `juelz-santana` | Juelz Santana | standard | medium | 5 |
| `kay-gee` | Kay Gee | standard | medium | 4 |
| `khrysis` | Khrysis | standard | medium | 4 |
| `lloyd-banks` | Lloyd Banks | standard | medium | 5 |
| `logic` | Logic | standard | medium | 5 |
| `lord-finesse` | Lord Finesse | standard | medium | 5 |
| `lupe-fiasco` | Lupe Fiasco | standard | medium | 4 |
| `mach-hommy` | Mach-Hommy | standard | medium | 4 |
| `marco-polo` | Marco Polo | standard | high | 4 |
| `mase` | Ma$e | standard | medium | 5 |
| `nashiem-myrick` | Nashiem Myrick | standard | medium | 3 |
| `nine` | Nine | standard | medium | 4 |
| `no-id` | No I.D. | standard | medium | 5 |
| `nottz` | Nottz | standard | medium | 4 |
| `obie-trice` | Obie Trice | standard | medium | 4 |
| `oc` | O.C. | standard | medium | 4 |
| `onyx` | Onyx | standard | medium | 4 |
| `rapsody` | Rapsody | standard | medium | 4 |
| `royce-da-5-9` | Royce da 5'9" | standard | medium | 6 |
| `salaam-remi` | Salaam Remi | standard | medium | 3 |
| `ski-beatz` | Ski Beatz | standard | medium | 4 |
| `stevie-j` | Stevie J | standard | medium | 3 |
| `styles-p` | Styles P | standard | medium | 5 |
| `the-heatmakerz` | The Heatmakerz | standard | high | 5 |
| `the-lox` | The LOX | standard | medium | 5 |
| `tony-yayo` | Tony Yayo | standard | low | 4 |

### cloud-rap — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `untiljapan` | UntilJapan | flagship | high | 3 |

### country-pop — 105

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `alan-jackson` | Alan Jackson | flagship | high | 3 |
| `carrie-underwood` | Carrie Underwood | flagship | high | 3 |
| `chicks` | The Chicks | flagship | high | 3 |
| `dann-huff` | Dann Huff | flagship | high | 3 |
| `david-garcia` | David Garcia | flagship | high | 3 |
| `ed-seay` | Ed Seay | flagship | high | 3 |
| `florida-georgia-line` | Florida Georgia Line | flagship | high | 3 |
| `garth-brooks` | Garth Brooks | flagship | high | 3 |
| `jason-aldean` | Jason Aldean | flagship | high | 3 |
| `jay-joyce` | Jay Joyce | flagship | high | 3 |
| `kacey-musgraves` | Kacey Musgraves | flagship | high | 3 |
| `kate-malone-and-devin-malone` | Kate Malone & Devin Malone | flagship | high | 4 |
| `martina-mcbride` | Martina McBride | flagship | high | 3 |
| `morgan-wallen` | Morgan Wallen | flagship | high | 3 |
| `nathan-chapman` | Nathan Chapman | flagship | high | 3 |
| `paul-worley` | Paul Worley | flagship | high | 3 |
| `sam-hunt` | Sam Hunt | flagship | high | 3 |
| `scott-hendricks` | Scott Hendricks | flagship | high | 3 |
| `tony-brown` | Tony Brown | flagship | high | 3 |
| `aaron-sterling` | Aaron Sterling | standard | medium | 3 |
| `allen-reynolds` | Allen Reynolds | standard | medium | 3 |
| `ashley-mcbryde` | Ashley McBryde | standard | medium | 3 |
| `bailey-zimmerman` | Bailey Zimmerman | standard | medium | 3 |
| `big-and-rich` | Big & Rich | standard | medium | 3 |
| `billy-currington` | Billy Currington | standard | medium | 3 |
| `blake-chancey` | Blake Chancey | standard | medium | 3 |
| `blake-shelton` | Blake Shelton | standard | medium | 3 |
| `brad-paisley` | Brad Paisley | standard | medium | 3 |
| `brent-mason` | Brent Mason | standard | medium | 3 |
| `brooks-and-dunn` | Brooks & Dunn | standard | medium | 3 |
| `brothers-osborne` | Brothers Osborne | standard | medium | 3 |
| `buddy-cannon` | Buddy Cannon | standard | medium | 3 |
| `busbee` | busbee | standard | medium | 3 |
| `byron-gallimore` | Byron Gallimore | standard | medium | 3 |
| `carly-pearce` | Carly Pearce | standard | medium | 3 |
| `chris-stapleton` | Chris Stapleton | standard | medium | 3 |
| `chris-young` | Chris Young | standard | medium | 3 |
| `chuck-ainlay` | Chuck Ainlay | standard | low | 3 |
| `cody-johnson` | Cody Johnson | standard | medium | 3 |
| `cole-swindell` | Cole Swindell | standard | medium | 3 |
| `dan-shay` | Dan + Shay | standard | medium | 3 |
| `darius-rucker` | Darius Rucker | standard | medium | 3 |
| `deana-carter` | Deana Carter | standard | medium | 3 |
| `diamond-rio` | Diamond Rio | standard | medium | 3 |
| `dierks-bentley` | Dierks Bentley | standard | medium | 3 |
| `don-cook` | Don Cook | standard | medium | 3 |
| `eddie-bayers` | Eddie Bayers | standard | medium | 3 |
| `emory-gordy-jr` | Emory Gordy Jr. | standard | medium | 3 |
| `eric-church` | Eric Church | standard | medium | 3 |
| `faith-hill` | Faith Hill | standard | medium | 3 |
| `frank-rogers` | Frank Rogers | standard | medium | 3 |
| `garth-fundis` | Garth Fundis | standard | medium | 3 |
| `gary-allan` | Gary Allan | standard | medium | 3 |
| `gretchen-wilson` | Gretchen Wilson | standard | medium | 3 |
| `hardy` | HARDY | standard | medium | 3 |
| `hunter-hayes` | Hunter Hayes | standard | medium | 3 |
| `james-stroud` | James Stroud | standard | medium | 3 |
| `jay-demarcus` | Jay DeMarcus | standard | medium | 3 |
| `jelly-roll` | Jelly Roll | standard | medium | 3 |
| `jesse-frasure` | Jesse Frasure | standard | medium | 3 |
| `jo-dee-messina` | Jo Dee Messina | standard | medium | 3 |
| `jordan-davis` | Jordan Davis | standard | low | 3 |
| `josh-turner` | Josh Turner | standard | medium | 3 |
| `justin-niebank` | Justin Niebank | standard | medium | 3 |
| `kane-brown` | Kane Brown | standard | medium | 3 |
| `keith-stegall` | Keith Stegall | standard | medium | 3 |
| `keith-urban` | Keith Urban | standard | medium | 3 |
| `kelsea-ballerini` | Kelsea Ballerini | standard | medium | 3 |
| `kenny-chesney` | Kenny Chesney | standard | medium | 3 |
| `lady-a` | Lady A | standard | medium | 3 |
| `lainey-wilson` | Lainey Wilson | standard | medium | 3 |
| `leann-rimes` | LeAnn Rimes | standard | medium | 3 |
| `little-big-town` | Little Big Town | standard | medium | 3 |
| `lonestar` | Lonestar | standard | medium | 3 |
| `luke-bryan` | Luke Bryan | standard | medium | 3 |
| `maren-morris` | Maren Morris | standard | medium | 3 |
| `mark-bright` | Mark Bright | standard | medium | 3 |
| `megan-moroney` | Megan Moroney | standard | medium | 3 |
| `michael-knox` | Michael Knox | standard | medium | 3 |
| `mikey-reaves` | Mikey Reaves | standard | medium | 3 |
| `miranda-lambert` | Miranda Lambert | standard | medium | 3 |
| `montgomery-gentry` | Montgomery Gentry | standard | medium | 3 |
| `old-dominion` | Old Dominion | standard | medium | 3 |
| `paul-franklin` | Paul Franklin | standard | medium | 3 |
| `peter-collins` | Peter Collins | standard | medium | 3 |
| `rascal-flatts` | Rascal Flatts | standard | medium | 3 |
| `rich-redmond` | Rich Redmond | standard | medium | 3 |
| `riley-green` | Riley Green | standard | medium | 3 |
| `rodney-atkins` | Rodney Atkins | standard | medium | 3 |
| `ross-copperman` | Ross Copperman | standard | medium | 3 |
| `sara-evans` | Sara Evans | standard | medium | 3 |
| `shaboozey` | Shaboozey | standard | medium | 3 |
| `shane-mcanally` | Shane McAnally | standard | medium | 3 |
| `shannon-forrest` | Shannon Forrest | standard | medium | 3 |
| `shedaisy` | SHeDAISY | standard | medium | 3 |
| `sugarland` | Sugarland | standard | medium | 3 |
| `terri-clark` | Terri Clark | standard | medium | 3 |
| `thomas-rhett` | Thomas Rhett | standard | medium | 3 |
| `tim-mcgraw` | Tim McGraw | standard | medium | 3 |
| `toby-keith` | Toby Keith | standard | medium | 3 |
| `trace-adkins` | Trace Adkins | standard | medium | 3 |
| `trisha-yearwood` | Trisha Yearwood | standard | medium | 3 |
| `zac-brown-band` | Zac Brown Band | standard | medium | 3 |
| `zach-crowell` | Zach Crowell | standard | medium | 3 |
| `zach-top` | Zach Top | standard | medium | 3 |

### country-shuffle — 21

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `dwight-yoakam` | Dwight Yoakam | flagship | high | 2 |
| `pete-anderson` | Pete Anderson | flagship | high | 2 |
| `asleep-at-the-wheel` | Asleep at the Wheel | standard | medium | 2 |
| `br549` | BR549 | standard | medium | 2 |
| `charley-crockett` | Charley Crockett | standard | medium | 3 |
| `clay-walker` | Clay Walker | standard | medium | 2 |
| `cody-jinks` | Cody Jinks | standard | medium | 3 |
| `colter-wall` | Colter Wall | standard | medium | 2 |
| `flaco-jimenez` | Flaco Jiménez | standard | medium | 2 |
| `junior-brown` | Junior Brown | standard | medium | 2 |
| `lloyd-maines` | Lloyd Maines | standard | medium | 2 |
| `mavericks` | The Mavericks | standard | medium | 3 |
| `midland` | Midland | standard | medium | 3 |
| `pam-tillis` | Pam Tillis | standard | medium | 2 |
| `ray-benson` | Ray Benson | standard | medium | 3 |
| `redd-volkaert` | Redd Volkaert | standard | medium | 3 |
| `sammy-kershaw` | Sammy Kershaw | standard | medium | 2 |
| `shooter-jennings` | Shooter Jennings | standard | medium | 2 |
| `sierra-ferrell` | Sierra Ferrell | standard | medium | 3 |
| `tracy-lawrence` | Tracy Lawrence | standard | medium | 2 |
| `wynonna` | Wynonna | standard | medium | 2 |

### country-train — 33

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `alison-krauss-and-union-station` | Alison Krauss & Union Station | flagship | medium | 4 |
| `bela-fleck` | Béla Fleck | flagship | medium | 3 |
| `del-mccoury-band` | Del McCoury Band | flagship | medium | 3 |
| `jerry-douglas` | Jerry Douglas | flagship | medium | 2 |
| `randy-scruggs` | Randy Scruggs | flagship | medium | 2 |
| `aaron-tippin` | Aaron Tippin | standard | medium | 2 |
| `billy-strings` | Billy Strings | standard | medium | 2 |
| `bryan-sutton` | Bryan Sutton | standard | medium | 2 |
| `confederate-railroad` | Confederate Railroad | standard | medium | 2 |
| `dan-tyminski` | Dan Tyminski | standard | medium | 2 |
| `harry-stinson` | Harry Stinson | standard | medium | 2 |
| `joe-diffie` | Joe Diffie | standard | medium | 2 |
| `john-michael-montgomery` | John Michael Montgomery | standard | medium | 3 |
| `little-texas` | Little Texas | standard | medium | 2 |
| `mark-chesnutt` | Mark Chesnutt | standard | medium | 2 |
| `marty-stuart` | Marty Stuart | standard | medium | 3 |
| `molly-tuttle-and-golden-highway` | Molly Tuttle & Golden Highway | standard | medium | 3 |
| `nickel-creek` | Nickel Creek | standard | medium | 2 |
| `old-crow-medicine-show` | Old Crow Medicine Show | standard | medium | 3 |
| `patty-loveless` | Patty Loveless | standard | medium | 2 |
| `paul-leim` | Paul Leim | standard | medium | 2 |
| `rhonda-vincent-and-the-rage` | Rhonda Vincent & The Rage | standard | medium | 2 |
| `richard-bennett` | Richard Bennett | standard | medium | 2 |
| `ricky-skaggs` | Ricky Skaggs | standard | medium | 3 |
| `ron-block` | Ron Block | standard | medium | 2 |
| `sam-bush` | Sam Bush | standard | medium | 2 |
| `sawyer-brown` | Sawyer Brown | standard | medium | 2 |
| `shenandoah` | Shenandoah | standard | low | 2 |
| `steeldrivers` | The SteelDrivers | standard | medium | 3 |
| `stuart-duncan` | Stuart Duncan | standard | medium | 2 |
| `tracy-byrd` | Tracy Byrd | standard | medium | 2 |
| `travis-tritt` | Travis Tritt | standard | medium | 2 |
| `turnpike-troubadours` | Turnpike Troubadours | standard | medium | 2 |

### crunk — 39

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `boosie-badazz` | Boosie Badazz | flagship | low | 6 |
| `david-banner` | David Banner | flagship | medium | 4 |
| `lil-jon` | Lil Jon | flagship | high | 6 |
| `ludacris` | Ludacris | flagship | medium | 5 |
| `mike-dean` | Mike Dean | flagship | high | 6 |
| `nelly` | Nelly | flagship | medium | 5 |
| `pastor-troy` | Pastor Troy | flagship | medium | 4 |
| `pimp-c` | Pimp C | flagship | high | 6 |
| `scarface` | Scarface | flagship | medium | 5 |
| `trick-daddy` | Trick Daddy | flagship | medium | 5 |
| `ugk` | UGK | flagship | high | 6 |
| `ying-yang-twins` | Ying Yang Twins | flagship | medium | 5 |
| `baby-bash` | Baby Bash | standard | medium | 5 |
| `bone-crusher` | Bone Crusher | standard | low | 4 |
| `chingy` | Chingy | standard | medium | 5 |
| `cool-and-dre` | Cool & Dre | standard | high | 6 |
| `crime-mob` | Crime Mob | standard | medium | 5 |
| `devin-the-dude` | Devin the Dude | standard | low | 5 |
| `field-mob` | Field Mob | standard | medium | 5 |
| `jay-e-epperson` | Jason "Jay E" Epperson | standard | low | 6 |
| `jazze-pha` | Jazze Pha | standard | medium | 5 |
| `jim-jonsin` | Jim Jonsin | standard | medium | 5 |
| `khia` | Khia | standard | low | 5 |
| `lil-scrappy` | Lil Scrappy | standard | low | 4 |
| `lroc` | LRoc | standard | medium | 5 |
| `mouse-on-tha-track` | Mouse on tha Track | standard | low | 6 |
| `murphy-lee` | Murphy Lee | standard | low | 5 |
| `no-joe` | N.O. Joe | standard | high | 6 |
| `petey-pablo` | Petey Pablo | standard | low | 4 |
| `polow-da-don` | Polow da Don | standard | low | 5 |
| `rich-boy` | Rich Boy | standard | low | 5 |
| `the-legendary-traxster` | The Legendary Traxster | standard | medium | 6 |
| `the-runners` | The Runners | standard | low | 6 |
| `trackboyz` | Trackboyz | standard | low | 6 |
| `trak-starz` | Trak Starz | standard | low | 6 |
| `trillville` | Trillville | standard | medium | 4 |
| `trina` | Trina | standard | low | 4 |
| `webbie` | Webbie | standard | low | 5 |
| `youngbloodz` | YoungBloodz | standard | medium | 5 |

### dance-pop — 75

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `darude` | Darude (with JS16) | flagship | high | 4 |
| `kygo` | Kygo | flagship | high | 3 |
| `2-unlimited` | 2 Unlimited | standard | medium | 4 |
| `ace-of-base` | Ace of Base | standard | medium | 4 |
| `afrojack` | Afrojack | standard | medium | 3 |
| `alan-walker` | Alan Walker | standard | medium | 3 |
| `alcazar` | Alcazar | standard | low | 3 |
| `alesso` | Alesso | standard | medium | 3 |
| `alice-deejay` | Alice Deejay | standard | medium | 4 |
| `aqua` | Aqua | standard | medium | 4 |
| `armin-van-buuren` | Armin van Buuren | standard | medium | 4 |
| `atb` | ATB | standard | medium | 3 |
| `avicii` | Avicii | standard | medium | 3 |
| `axwell` | Axwell | standard | medium | 3 |
| `basshunter` | Basshunter | standard | medium | 3 |
| `black-eyed-peas` | The Black Eyed Peas | standard | medium | 4 |
| `cascada` | Cascada | standard | medium | 3 |
| `chainsmokers` | The Chainsmokers | standard | medium | 4 |
| `cheat-codes` | Cheat Codes | standard | low | 3 |
| `clean-bandit` | Clean Bandit | standard | medium | 4 |
| `corona` | Corona | standard | medium | 3 |
| `culture-beat` | Culture Beat | standard | medium | 4 |
| `david-guetta` | David Guetta | standard | medium | 3 |
| `deadmau5` | Deadmau5 | standard | medium | 4 |
| `dimitri-vegas-and-like-mike` | Dimitri Vegas & Like Mike | standard | medium | 4 |
| `diplo` | Diplo (Major Lazer · Jack Ü · Silk City) | standard | medium | 4 |
| `disclosure` | Disclosure | standard | medium | 3 |
| `dj-snake` | DJ Snake | standard | medium | 3 |
| `don-diablo` | Don Diablo | standard | medium | 3 |
| `dr-alban` | Dr. Alban | standard | medium | 3 |
| `duke-dumont` | Duke Dumont | standard | medium | 3 |
| `eiffel-65` | Eiffel 65 | standard | medium | 5 |
| `ellie-goulding` | Ellie Goulding | standard | medium | 3 |
| `felix-jaehn` | Felix Jaehn | standard | medium | 3 |
| `flo-rida` | Flo Rida | standard | medium | 3 |
| `frank-farian` | Frank Farian | standard | medium | 4 |
| `galantis` | Galantis | standard | medium | 4 |
| `haddaway` | Haddaway | standard | medium | 3 |
| `hardwell` | Hardwell | standard | medium | 3 |
| `ian-van-dahl` | Ian Van Dahl | standard | low | 4 |
| `icona-pop` | Icona Pop | standard | medium | 4 |
| `jess-glynne` | Jess Glynne | standard | medium | 3 |
| `jonas-blue` | Jonas Blue | standard | medium | 3 |
| `juergen-wind-and-frank-quickmix-hassas` | Juergen Wind & Frank "Quickmix" Hassas | standard | medium | 5 |
| `kylie-minogue` | Kylie Minogue | standard | medium | 3 |
| `la-bouche` | La Bouche | standard | medium | 4 |
| `lmfao` | LMFAO | standard | medium | 4 |
| `lost-frequencies` | Lost Frequencies | standard | medium | 3 |
| `marshmello` | Marshmello | standard | medium | 3 |
| `martin-garrix` | Martin Garrix | standard | medium | 4 |
| `mo` | MØ | standard | medium | 3 |
| `nicky-romero` | Nicky Romero | standard | medium | 3 |
| `oliver-heldens` | Oliver Heldens | standard | medium | 4 |
| `pitbull` | Pitbull | standard | medium | 4 |
| `pronti-and-kalmani` | Pronti & Kalmani | standard | medium | 5 |
| `real-mccoy` | Real McCoy | standard | medium | 5 |
| `rednex` | Rednex | standard | low | 3 |
| `robin-schulz` | Robin Schulz | standard | medium | 3 |
| `robyn` | Robyn | standard | medium | 3 |
| `scatman-john` | Scatman John | standard | low | 3 |
| `sebastian-ingrosso` | Sebastian Ingrosso | standard | medium | 3 |
| `september` | September | standard | low | 3 |
| `sigala` | Sigala | standard | medium | 3 |
| `skrillex` | Skrillex | standard | medium | 3 |
| `snap` | Snap! | standard | medium | 4 |
| `soren-rasted-and-claus-norreen` | Søren Rasted & Claus Norreen | standard | medium | 4 |
| `steps` | Steps | standard | medium | 3 |
| `steve-angello` | Steve Angello | standard | medium | 3 |
| `swedish-house-mafia` | Swedish House Mafia | standard | medium | 3 |
| `technotronic` | Technotronic | standard | medium | 4 |
| `tiesto` | Tiësto | standard | medium | 3 |
| `vengaboys` | Vengaboys | standard | medium | 4 |
| `whigfield` | Whigfield | standard | low | 3 |
| `yanou-and-dj-manian` | Yanou & DJ Manian | standard | medium | 5 |
| `zedd` | Zedd | standard | medium | 3 |

### dark-plugg — 2

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `glokk40spazz` | Glokk40Spazz | flagship | high | 5 |
| `ohsxnta` | ohsxnta | standard | medium | 3 |

### dark-trap — 35

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `6ix9ine` | 6ix9ine | flagship | medium | 4 |
| `asap-ferg` | A$AP Ferg | flagship | medium | 4 |
| `asap-rocky` | A$AP Rocky | flagship | high | 5 |
| `bnyx` | BNYX | flagship | low | 4 |
| `budd-dwyer` | Budd Dwyer ($crim) | flagship | medium | 5 |
| `clams-casino` | Clams Casino | flagship | medium | 4 |
| `danny-brown` | Danny Brown | flagship | high | 4 |
| `denzel-curry` | Denzel Curry | flagship | medium | 4 |
| `flatbush-zombies` | Flatbush Zombies | flagship | medium | 4 |
| `ghostemane` | Ghostemane | flagship | medium | 4 |
| `jpegmafia` | JPEGMAFIA | flagship | medium | 4 |
| `ronny-j` | Ronny J | flagship | medium | 4 |
| `ski-mask-the-slump-god` | Ski Mask the Slump God | flagship | medium | 4 |
| `smokeasac` | Smokeasac | flagship | medium | 4 |
| `asap-mob` | A$AP Mob | standard | medium | 4 |
| `bighead` | Bighead | standard | low | 2 |
| `comethazine` | Comethazine | standard | medium | 2 |
| `crystal-caines` | Crystal Caines | standard | low | 3 |
| `erick-arc-elliott` | Erick Arc Elliott | standard | medium | 4 |
| `fnz` | FnZ | standard | low | 5 |
| `getter` | Getter | standard | low | 3 |
| `hector-delgado` | Hector Delgado | standard | medium | 3 |
| `iivi` | IIVI (George Astasio) | standard | medium | 4 |
| `lil-peep` | Lil Peep | standard | medium | 2 |
| `lil-pump` | Lil Pump | standard | medium | 2 |
| `paul-white` | Paul White | standard | medium | 3 |
| `rico-nasty` | Rico Nasty | standard | medium | 3 |
| `scarlxrd` | Scarlxrd | standard | medium | 2 |
| `smokepurpp` | Smokepurpp | standard | low | 2 |
| `sosmula` | SosMula | standard | low | 3 |
| `soundcloud-rap` | SoundCloud Rap (2015-2019 wave) | standard | medium | 4 |
| `suicideboys` | $uicideboy$ | standard | medium | 5 |
| `thraxx` | Thraxx | standard | medium | 3 |
| `trap-metal` | Trap Metal / Scream Rap | standard | medium | 4 |
| `zillakami` | ZillaKami | standard | medium | 3 |

### dungeon-family — 7

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `goodie-mob` | Goodie Mob | flagship | medium | 8 |
| `organized-noize` | Organized Noize | flagship | medium | 7 |
| `outkast` | OutKast | flagship | high | 7 |
| `big-boi` | Big Boi | standard | medium | 5 |
| `ceelo-green` | CeeLo Green | standard | medium | 6 |
| `earthtone-iii` | Earthtone III / Mr. DJ | standard | high | 6 |
| `killer-mike` | Killer Mike | standard | low | 4 |

### edm-rage — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `2hollis` | 2hollis | flagship | high | 4 |

### future-bass — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `flume` | Flume | standard | medium | 3 |

### g-funk — 71

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `2pac` | 2Pac | flagship | high | 6 |
| `above-the-law` | Above the Law | flagship | high | 4 |
| `ant-banks` | Ant Banks | flagship | medium | 4 |
| `battlecat` | Battlecat | flagship | medium | 3 |
| `bone-thugs-n-harmony` | Bone Thugs-n-Harmony | flagship | medium | 6 |
| `chris-the-glove-taylor` | Chris "The Glove" Taylor | flagship | high | 4 |
| `cold-187um` | Cold 187um (Big Hutch) | flagship | high | 4 |
| `coolio` | Coolio | flagship | medium | 4 |
| `cypress-hill` | Cypress Hill | flagship | medium | 4 |
| `daz-dillinger` | Daz Dillinger | flagship | medium | 4 |
| `digital-underground` | Digital Underground | flagship | medium | 3 |
| `dj-pooh` | DJ Pooh | flagship | medium | 4 |
| `dj-quik` | DJ Quik | flagship | high | 4 |
| `dj-u-neek` | DJ U-Neek | flagship | medium | 5 |
| `e-40` | E-40 | flagship | medium | 5 |
| `eazy-e` | Eazy-E | flagship | medium | 5 |
| `fredwreck` | Fredwreck | flagship | low | 4 |
| `ice-cube` | Ice Cube | flagship | high | 5 |
| `johnny-j` | Johnny J | flagship | medium | 4 |
| `khayree` | Khayree | flagship | medium | 4 |
| `mc-eiht` | MC Eiht / Compton's Most Wanted | flagship | medium | 5 |
| `mike-elizondo` | Mike Elizondo | flagship | medium | 4 |
| `nate-dogg` | Nate Dogg | flagship | medium | 4 |
| `rick-rock` | Rick Rock | flagship | medium | 4 |
| `shock-g` | Shock G | flagship | medium | 4 |
| `sir-jinx` | Sir Jinx | flagship | medium | 4 |
| `snoop-dogg` | Snoop Dogg | flagship | high | 6 |
| `tha-dogg-pound` | Tha Dogg Pound | flagship | medium | 5 |
| `too-short` | Too Short | flagship | high | 4 |
| `warren-g` | Warren G | flagship | high | 4 |
| `westside-connection` | Westside Connection | flagship | high | 4 |
| `xzibit` | Xzibit | flagship | medium | 3 |
| `2nd-ii-none-amg-hi-c` | 2nd II None, AMG & Hi-C | standard | medium | 8 |
| `bad-azz` | Bad Azz | standard | low | 4 |
| `budda` | Bud'da | standard | high | 4 |
| `c-bo` | C-Bo | standard | medium | 4 |
| `celly-cel` | Celly Cel | standard | medium | 3 |
| `colin-wolfe-tony-green-rob-bacon` | Colin Wolfe / Tony Green / Rob "Fonksta" Bacon | standard | medium | 7 |
| `cyrus-esteban-and-franky-j` | Cyrus Esteban & Franky J | standard | medium | 5 |
| `del-the-funky-homosapien` | Del the Funky Homosapien | standard | medium | 5 |
| `dj-slip` | DJ Slip | standard | medium | 4 |
| `domino` | Domino | standard | medium | 4 |
| `doug-rasheed` | Doug Rasheed | standard | medium | 4 |
| `dru-down` | Dru Down | standard | medium | 3 |
| `e-a-ski-and-cmt` | E-A-Ski & CMT | standard | medium | 5 |
| `hank-thomas-sleepy-turner` | Henry "Hank" Thomas & Lamon "Sleepy" Turner | standard | medium | 6 |
| `king-t` | King T | standard | medium | 3 |
| `kokane` | Kokane | standard | medium | 5 |
| `kurupt` | Kurupt | standard | medium | 4 |
| `mac-mall` | Mac Mall | standard | medium | 3 |
| `mack-10` | Mack 10 | standard | medium | 5 |
| `mausberg` | Mausberg | standard | medium | 4 |
| `meech-wells` | Meech Wells | standard | medium | 4 |
| `mike-mosley-sam-bostic` | Mike Mosley & Sam Bostic | standard | medium | 5 |
| `rappin-4-tay` | Rappin' 4-Tay | standard | medium | 4 |
| `rbx` | RBX | standard | low | 3 |
| `rhythm-d` | Rhythm D | standard | medium | 4 |
| `shorty-b` | Shorty B | standard | medium | 4 |
| `soopafly` | Soopafly | standard | medium | 3 |
| `spice-1` | Spice 1 | standard | medium | 3 |
| `suga-free` | Suga Free | standard | medium | 5 |
| `tha-alkaholiks` | Tha Alkaholiks | standard | medium | 5 |
| `tha-eastsidaz` | Tha Eastsidaz | standard | medium | 6 |
| `the-boogie-men` | The Boogie Men | standard | medium | 5 |
| `the-click` | The Click | standard | medium | 5 |
| `the-dove-shack` | The Dove Shack | standard | medium | 5 |
| `the-lady-of-rage` | The Lady of Rage | standard | medium | 4 |
| `the-luniz` | The Luniz | standard | medium | 4 |
| `tone-capone` | Tone Capone | standard | medium | 4 |
| `wc` | WC | standard | medium | 5 |
| `yo-yo` | Yo-Yo | standard | low | 5 |

### houston-screw — 10

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `chamillionaire` | Chamillionaire | flagship | medium | 5 |
| `dj-screw` | DJ Screw | flagship | high | 7 |
| `mike-jones` | Mike Jones | flagship | high | 5 |
| `paul-wall` | Paul Wall | flagship | medium | 5 |
| `lil-flip` | Lil Flip | standard | low | 6 |
| `michael-5000-watts` | Michael "5000" Watts | standard | medium | 6 |
| `og-ron-c` | OG Ron C | standard | medium | 7 |
| `salih-williams` | Salih Williams | standard | high | 4 |
| `slim-thug` | Slim Thug | standard | low | 5 |
| `z-ro` | Z-Ro | standard | low | 7 |

### jerk — 4

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `fake-mink` | Fake Mink | flagship | high | 3 |
| `nettspend` | Nettspend | flagship | high | 2 |
| `xaviersobased` | xaviersobased | flagship | high | 4 |
| `bleood` | Bleood | standard | medium | 3 |

### liquid-dnb — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `rudimental` | Rudimental | standard | medium | 4 |

### memphis-rap — 42

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `8ball-and-mjg` | 8Ball & MJG | flagship | medium | 5 |
| `dj-paul` | DJ Paul | flagship | high | 4 |
| `dj-spanish-fly` | DJ Spanish Fly | flagship | medium | 4 |
| `gangsta-boo` | Gangsta Boo | flagship | medium | 5 |
| `glorilla` | GloRilla | flagship | high | 4 |
| `juicy-j` | Juicy J | flagship | high | 4 |
| `key-glock` | Key Glock | flagship | medium | 4 |
| `lord-infamous` | Lord Infamous | flagship | medium | 5 |
| `moneybagg-yo` | Moneybagg Yo | flagship | medium | 5 |
| `project-pat` | Project Pat | flagship | medium | 4 |
| `three-6-mafia` | Three 6 Mafia | flagship | high | 5 |
| `tommy-wright-iii` | Tommy Wright III | flagship | medium | 4 |
| `yo-gotti` | Yo Gotti | flagship | medium | 5 |
| `young-dolph` | Young Dolph | flagship | medium | 4 |
| `al-kapone` | Al Kapone | standard | medium | 5 |
| `bandplay` | BandPlay | standard | medium | 3 |
| `big30` | Big30 | standard | low | 3 |
| `blac-youngsta` | Blac Youngsta | standard | low | 5 |
| `blocboy-jb` | BlocBoy JB | standard | medium | 4 |
| `crunchy-black` | Crunchy Black | standard | low | 4 |
| `dj-squeeky` | DJ Squeeky | standard | low | 4 |
| `don-trip` | Don Trip | standard | low | 3 |
| `duke-deuce` | Duke Deuce | standard | medium | 4 |
| `finesse2tymes` | Finesse2tymes | standard | low | 3 |
| `frayser-boy` | Frayser Boy | standard | medium | 4 |
| `gangsta-pat` | Gangsta Pat | standard | low | 3 |
| `hitkidd` | Hitkidd | standard | medium | 3 |
| `hypnotize-camp-posse` | Hypnotize Camp Posse | standard | medium | 4 |
| `kia-shine` | Kia Shine | standard | low | 4 |
| `kingpin-skinny-pimp` | Kingpin Skinny Pimp | standard | low | 5 |
| `koopsta-knicca` | Koopsta Knicca | standard | low | 5 |
| `la-chat` | La Chat | standard | low | 4 |
| `lil-wyte` | Lil Wyte | standard | medium | 4 |
| `nle-choppa` | NLE Choppa | standard | medium | 3 |
| `playa-fly` | Playa Fly | standard | low | 5 |
| `pooh-shiesty` | Pooh Shiesty | standard | medium | 4 |
| `real-red` | Real Red | standard | low | 3 |
| `skywalker-og` | Skywalker OG | standard | low | 4 |
| `snootie-wild` | Snootie Wild | standard | low | 4 |
| `tear-da-club-up-thugs` | Tear Da Club Up Thugs | standard | medium | 4 |
| `tela` | Tela | standard | low | 5 |
| `yc-turnmeupyc` | YC (TurnMeUpYC) | standard | low | 4 |

### neo-soul — 90

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `amy-winehouse` | Amy Winehouse | flagship | high | 3 |
| `dangelo` | D'Angelo | flagship | high | 4 |
| `erykah-badu` | Erykah Badu | flagship | high | 4 |
| `pino-palladino` | Pino Palladino | flagship | high | 4 |
| `questlove` | Questlove | flagship | high | 3 |
| `soulquarians` | The Soulquarians (collective preset) | flagship | high | 4 |
| `adrian-younge` | Adrian Younge | standard | medium | 3 |
| `alex-isley` | Alex Isley | standard | low | 3 |
| `ali-shaheed-muhammad` | Ali Shaheed Muhammad | standard | medium | 4 |
| `aloe-blacc` | Aloe Blacc | standard | medium | 3 |
| `amel-larrieux` | Amel Larrieux / Groove Theory | standard | medium | 3 |
| `andra-day` | Andra Day | standard | medium | 3 |
| `andre-harris-and-vidal-davis` | Andre Harris & Vidal Davis (Dre & Vidal) | standard | medium | 4 |
| `angie-stone` | Angie Stone | standard | medium | 3 |
| `anthony-hamilton` | Anthony Hamilton | standard | medium | 3 |
| `ari-lennox` | Ari Lennox | standard | medium | 3 |
| `bilal` | Bilal | standard | medium | 3 |
| `bob-power` | Bob Power | standard | medium | 4 |
| `carvin-and-ivan-neo-soul-half` | Carvin & Ivan — neo-soul half (Carvin Haggins & Ivan Barias) | standard | medium | 3 |
| `chris-dave` | Chris Dave | standard | medium | 3 |
| `chrisette-michele` | Chrisette Michele | standard | low | 3 |
| `cleo-sol` | Cleo Sol | standard | medium | 3 |
| `cody-chesnutt` | Cody ChesnuTT | standard | low | 3 |
| `corinne-bailey-rae` | Corinne Bailey Rae | standard | medium | 3 |
| `cory-henry` | Cory Henry | standard | low | 3 |
| `danger-mouse` | Danger Mouse | standard | medium | 4 |
| `daniel-caesar` | Daniel Caesar | standard | medium | 4 |
| `derrick-hodge` | Derrick Hodge | standard | low | 3 |
| `dj-camper` | DJ Camper | standard | low | 3 |
| `dj-jazzy-jeff` | DJ Jazzy Jeff / A Touch of Jazz | standard | medium | 4 |
| `dwele` | Dwele | standard | medium | 3 |
| `emily-king` | Emily King | standard | low | 3 |
| `eric-roberson` | Eric Roberson | standard | medium | 3 |
| `fkj` | FKJ | standard | medium | 4 |
| `floetry` | Floetry / Marsha Ambrosius | standard | medium | 4 |
| `gabriel-roth` | Gabriel Roth (Bosco Mann) / Daptone | standard | medium | 4 |
| `goapele` | Goapele | standard | low | 3 |
| `her` | H.E.R. | standard | medium | 3 |
| `hiatus-kaiyote` | Hiatus Kaiyote | standard | medium | 4 |
| `indiaarie` | India.Arie | standard | medium | 3 |
| `inflo` | Inflo | standard | medium | 4 |
| `jack-splash` | Jack Splash | standard | low | 4 |
| `jaguar-wright` | Jaguar Wright | standard | low | 2 |
| `james-poyser` | James Poyser | standard | medium | 3 |
| `janelle-monae` | Janelle Monáe | standard | medium | 3 |
| `jazmine-sullivan` | Jazmine Sullivan | standard | medium | 3 |
| `jill-scott` | Jill Scott | standard | medium | 3 |
| `jorja-smith` | Jorja Smith | standard | medium | 3 |
| `karriem-riggins` | Karriem Riggins | standard | medium | 3 |
| `kaytranada` | Kaytranada | standard | medium | 3 |
| `keith-pelzer` | Keith Pelzer | standard | low | 4 |
| `kem` | Kem | standard | low | 3 |
| `lalah-hathaway` | Lalah Hathaway | standard | medium | 3 |
| `ledisi` | Ledisi | standard | medium | 3 |
| `leela-james` | Leela James | standard | low | 3 |
| `leon-bridges` | Leon Bridges | standard | medium | 3 |
| `leon-michels` | Leon Michels | standard | low | 3 |
| `les-nubians` | Les Nubians | standard | low | 3 |
| `lianne-la-havas` | Lianne La Havas | standard | medium | 3 |
| `lizz-wright` | Lizz Wright | standard | low | 3 |
| `lucy-pearl` | Lucy Pearl | standard | medium | 4 |
| `macy-gray` | Macy Gray | standard | medium | 3 |
| `masego` | Masego | standard | medium | 3 |
| `maxwell` | Maxwell | standard | medium | 3 |
| `melanie-fiona` | Melanie Fiona | standard | low | 3 |
| `meshell-ndegeocello` | Meshell Ndegeocello | standard | medium | 3 |
| `michael-kiwanuka` | Michael Kiwanuka | standard | medium | 3 |
| `moonchild` | Moonchild | standard | low | 3 |
| `musiq-soulchild` | Musiq Soulchild | standard | medium | 3 |
| `nao` | Nao | standard | low | 3 |
| `nate-smith` | Nate Smith | standard | low | 3 |
| `ndea-davenport` | N'Dea Davenport / The Brand New Heavies | standard | medium | 3 |
| `ommas-keith` | Om'Mas Keith | standard | low | 4 |
| `raheem-devaughn` | Raheem DeVaughn | standard | low | 3 |
| `rahsaan-patterson` | Rahsaan Patterson | standard | medium | 3 |
| `raphael-saadiq` | Raphael Saadiq | standard | medium | 3 |
| `rex-rideout` | Rex Rideout | standard | low | 4 |
| `robert-glasper` | Robert Glasper | standard | medium | 4 |
| `russell-elevado` | Russell Elevado | standard | high | 4 |
| `sault` | Sault | standard | medium | 3 |
| `sharon-jones-and-the-dap-kings` | Sharon Jones & the Dap-Kings | standard | medium | 4 |
| `snoh-aalegra` | Snoh Aalegra | standard | medium | 2 |
| `solange` | Solange | standard | medium | 3 |
| `steve-mckie` | Steve McKie | standard | low | 4 |
| `swagg-rcelious` | Swagg R'Celious | standard | low | 4 |
| `tom-elmhirst` | Tom Elmhirst | standard | low | 3 |
| `tom-misch` | Tom Misch | standard | medium | 3 |
| `tony-toni-tone` | Tony! Toni! Toné! | standard | medium | 3 |
| `van-hunt` | Van Hunt | standard | medium | 3 |
| `yebba` | Yebba | standard | medium | 3 |

### nola-bounce — 15

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `b-g` | B.G. | flagship | medium | 7 |
| `juvenile` | Juvenile | flagship | high | 6 |
| `lil-wayne` | Lil Wayne | flagship | medium | 7 |
| `mannie-fresh` | Mannie Fresh | flagship | high | 5 |
| `master-p` | Master P | flagship | medium | 6 |
| `mystikal` | Mystikal | flagship | medium | 5 |
| `big-tymers` | Big Tymers | standard | medium | 5 |
| `c-murder` | C-Murder | standard | low | 6 |
| `choppa` | Choppa | standard | medium | 6 |
| `hot-boys` | Hot Boys | standard | medium | 5 |
| `klc-beats-by-the-pound` | KLC / Beats By the Pound | standard | medium | 8 |
| `mia-x` | Mia X | standard | low | 5 |
| `mo-b-dick` | Mo B. Dick | standard | medium | 6 |
| `silkk-the-shocker` | Silkk the Shocker | standard | low | 5 |
| `turk` | Turk | standard | low | 4 |

### ny-drill — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `pop-smoke` | Pop Smoke | flagship | high | 3 |

### plugg — 2

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `pierre-bourne` | Pi'erre Bourne | flagship | high | 4 |
| `sneak` | sneak | standard | low | 2 |

### pluggnb — 1

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `summrs` | Summrs | flagship | high | 2 |

### pop-2000s — 107

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `avril-lavigne` | Avril Lavigne | flagship | high | 3 |
| `backstreet-boys` | Backstreet Boys | flagship | high | 3 |
| `blink-182` | Blink-182 | flagship | high | 3 |
| `bloodshy-and-avant` | Bloodshy & Avant | flagship | high | 3 |
| `britney-spears` | Britney Spears | flagship | high | 3 |
| `cher` | Cher | flagship | high | 2 |
| `christina-aguilera` | Christina Aguilera | flagship | high | 3 |
| `dave-fortman` | Dave Fortman | flagship | high | 2 |
| `dido` | Dido | flagship | high | 2 |
| `eric-valentine` | Eric Valentine | flagship | high | 2 |
| `evanescence` | Evanescence | flagship | high | 2 |
| `fergie` | Fergie | flagship | high | 2 |
| `green-day` | Green Day | flagship | high | 2 |
| `gwen-stefani` | Gwen Stefani | flagship | high | 2 |
| `jerry-finn` | Jerry Finn | flagship | high | 2 |
| `justin-timberlake` | Justin Timberlake | flagship | high | 2 |
| `katy-perry` | Katy Perry | flagship | high | 3 |
| `kelly-clarkson` | Kelly Clarkson | flagship | high | 2 |
| `kesha` | Kesha | flagship | high | 2 |
| `mark-taylor-and-brian-rawling` | Mark Taylor & Brian Rawling (Metro) | flagship | high | 4 |
| `matrix` | The Matrix | flagship | high | 5 |
| `my-chemical-romance` | My Chemical Romance | flagship | high | 3 |
| `nsync` | NSYNC | flagship | high | 3 |
| `ricky-martin` | Ricky Martin | flagship | high | 2 |
| `rob-cavallo` | Rob Cavallo | flagship | high | 2 |
| `steve-kipner-and-david-frank` | Steve Kipner & David Frank | flagship | high | 3 |
| `third-eye-blind` | Third Eye Blind | flagship | high | 3 |
| `98-degrees` | 98 Degrees | standard | medium | 3 |
| `alanis-morissette` | Alanis Morissette | standard | medium | 3 |
| `all-saints` | All Saints | standard | medium | 2 |
| `anders-bagge-and-arnthor-birgisson` | Anders Bagge & Arnthor Birgisson (Murlyn) | standard | medium | 4 |
| `andreas-carlsson` | Andreas Carlsson | standard | medium | 3 |
| `ashlee-simpson` | Ashlee Simpson | standard | medium | 2 |
| `boyzone` | Boyzone | standard | medium | 2 |
| `brian-higgins` | Brian Higgins / Xenomania | standard | medium | 3 |
| `cathy-dennis` | Cathy Dennis | standard | medium | 3 |
| `celine-dion` | Celine Dion | standard | medium | 2 |
| `daniel-powter` | Daniel Powter | standard | medium | 2 |
| `david-bendeth` | David Bendeth | standard | medium | 2 |
| `david-foster` | David Foster | standard | medium | 2 |
| `denniz-pop` | Denniz PoP | standard | medium | 3 |
| `desmond-child` | Desmond Child | standard | medium | 2 |
| `enrique-iglesias` | Enrique Iglesias | standard | medium | 2 |
| `fall-out-boy` | Fall Out Boy | standard | medium | 3 |
| `fray` | The Fray | standard | medium | 2 |
| `girls-aloud` | Girls Aloud | standard | medium | 2 |
| `glen-ballard` | Glen Ballard | standard | medium | 3 |
| `goo-goo-dolls` | Goo Goo Dolls | standard | medium | 3 |
| `good-charlotte` | Good Charlotte | standard | medium | 2 |
| `greg-wells` | Greg Wells | standard | medium | 2 |
| `guy-chambers-and-steve-power` | Guy Chambers & Steve Power | standard | medium | 3 |
| `hilary-duff` | Hilary Duff | standard | medium | 3 |
| `jake-schulze` | Jake Schulze | standard | medium | 3 |
| `james-blunt` | James Blunt | standard | medium | 2 |
| `jason-mraz` | Jason Mraz | standard | medium | 2 |
| `jennifer-lopez` | Jennifer Lopez | standard | medium | 4 |
| `jesse-mccartney` | Jesse McCartney | standard | medium | 3 |
| `jessica-simpson` | Jessica Simpson | standard | medium | 2 |
| `john-mayer` | John Mayer | standard | medium | 2 |
| `john-shanks` | John Shanks | standard | medium | 2 |
| `jorgen-elofsson` | Jörgen Elofsson | standard | medium | 3 |
| `kara-dioguardi` | Kara DioGuardi | standard | medium | 3 |
| `kristian-lundin` | Kristian Lundin | standard | medium | 3 |
| `leona-lewis` | Leona Lewis | standard | medium | 2 |
| `lifehouse` | Lifehouse | standard | medium | 2 |
| `linda-perry` | Linda Perry | standard | medium | 2 |
| `mandy-moore` | Mandy Moore | standard | medium | 2 |
| `mariah-carey` | Mariah Carey | standard | medium | 2 |
| `maroon-5` | Maroon 5 | standard | medium | 3 |
| `matchbox-twenty` | Matchbox Twenty | standard | medium | 3 |
| `matt-squire` | Matt Squire | standard | medium | 2 |
| `michelle-branch` | Michelle Branch | standard | medium | 2 |
| `natalie-imbruglia` | Natalie Imbruglia | standard | medium | 2 |
| `natasha-bedingfield` | Natasha Bedingfield | standard | medium | 2 |
| `neal-avron` | Neal Avron | standard | medium | 3 |
| `nellee-hooper` | Nellee Hooper | standard | medium | 3 |
| `nelly-furtado` | Nelly Furtado | standard | medium | 2 |
| `nickelback` | Nickelback | standard | medium | 2 |
| `o-town` | O-Town | standard | medium | 2 |
| `onerepublic` | OneRepublic | standard | medium | 2 |
| `panic-at-the-disco` | Panic! at the Disco | standard | medium | 3 |
| `paramore` | Paramore | standard | medium | 2 |
| `per-magnusson-and-david-kreuger` | Per Magnusson & David Kreuger | standard | medium | 4 |
| `phil-thornalley` | Phil Thornalley | standard | medium | 3 |
| `pink` | Pink | standard | medium | 4 |
| `rami-yacoub` | Rami Yacoub | standard | medium | 3 |
| `richard-x` | Richard X | standard | medium | 2 |
| `rick-nowels` | Rick Nowels | standard | medium | 3 |
| `rick-parashar` | Rick Parashar | standard | medium | 2 |
| `robbie-williams` | Robbie Williams | standard | medium | 2 |
| `rollo-armstrong` | Rollo Armstrong | standard | medium | 3 |
| `s-club-7` | S Club 7 | standard | medium | 3 |
| `savage-garden` | Savage Garden | standard | medium | 2 |
| `shakira` | Shakira | standard | medium | 2 |
| `sheryl-crow` | Sheryl Crow | standard | medium | 3 |
| `simple-plan` | Simple Plan | standard | medium | 2 |
| `spice-girls` | Spice Girls | standard | medium | 2 |
| `sugababes` | Sugababes | standard | medium | 2 |
| `sum-41` | Sum 41 | standard | medium | 2 |
| `tatu` | t.A.T.u. | standard | medium | 2 |
| `train` | Train | standard | medium | 2 |
| `trevor-horn` | Trevor Horn | standard | medium | 2 |
| `vanessa-carlton` | Vanessa Carlton | standard | medium | 2 |
| `wayne-rodrigues-and-danielle-brisebois` | Wayne Rodrigues & Danielle Brisebois | standard | medium | 3 |
| `westlife` | Westlife | standard | medium | 2 |
| `whitney-houston` | Whitney Houston | standard | medium | 2 |
| `william-orbit` | William Orbit | standard | medium | 2 |

### pop-2020s — 89

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `aaron-dessner` | Aaron Dessner | flagship | high | 4 |
| `adele` | Adele | flagship | high | 4 |
| `ag-cook` | A. G. Cook | flagship | high | 4 |
| `ariana-grande` | Ariana Grande | flagship | high | 4 |
| `billie-eilish` | Billie Eilish | flagship | high | 4 |
| `chappell-roan` | Chappell Roan | flagship | high | 4 |
| `dua-lipa` | Dua Lipa | flagship | high | 4 |
| `dylan-brady` | Dylan Brady | flagship | high | 4 |
| `ed-sheeran` | Ed Sheeran | flagship | high | 4 |
| `greg-kurstin` | Greg Kurstin | flagship | high | 4 |
| `lorde` | Lorde | flagship | high | 4 |
| `olivia-rodrigo` | Olivia Rodrigo | flagship | high | 4 |
| `sophie` | SOPHIE | flagship | high | 4 |
| `stuart-price` | Stuart Price | flagship | high | 4 |
| `taylor-swift` | Taylor Swift | flagship | high | 4 |
| `amy-allen` | Amy Allen | standard | medium | 4 |
| `andrew-watt` | Andrew Watt | standard | medium | 4 |
| `anne-marie` | Anne-Marie | standard | medium | 4 |
| `ariel-rechtshaid` | Ariel Rechtshaid | standard | medium | 4 |
| `bebe-rexha` | Bebe Rexha | standard | medium | 4 |
| `benson-boone` | Benson Boone | standard | low | 4 |
| `blackpink` | Blackpink | standard | medium | 4 |
| `blake-slatkin` | Blake Slatkin | standard | medium | 4 |
| `bloodpop` | BloodPop | standard | medium | 4 |
| `bruno-mars` | Bruno Mars | standard | medium | 4 |
| `bts` | BTS | standard | medium | 4 |
| `camila-cabello` | Camila Cabello | standard | medium | 4 |
| `carly-rae-jepsen` | Carly Rae Jepsen | standard | medium | 4 |
| `caroline-polachek` | Caroline Polachek | standard | medium | 4 |
| `cashmere-cat` | Cashmere Cat | standard | medium | 4 |
| `charli-xcx` | Charli XCX | standard | medium | 4 |
| `daheala` | DaHeala | standard | medium | 4 |
| `danny-l-harle` | Danny L Harle | standard | medium | 4 |
| `demi-lovato` | Demi Lovato | standard | medium | 4 |
| `djo` | Djo | standard | low | 4 |
| `emily-warren` | Emily Warren | standard | medium | 4 |
| `fraser-t-smith` | Fraser T Smith | standard | medium | 4 |
| `glass-animals` | Glass Animals | standard | medium | 4 |
| `gracie-abrams` | Gracie Abrams | standard | medium | 4 |
| `halsey` | Halsey | standard | medium | 4 |
| `harry-styles` | Harry Styles | standard | medium | 4 |
| `hozier` | Hozier | standard | medium | 4 |
| `illangelo` | Illangelo | standard | medium | 4 |
| `ilya-salmanzadeh` | Ilya Salmanzadeh | standard | medium | 4 |
| `imagine-dragons` | Imagine Dragons | standard | medium | 4 |
| `jeff-bhasker` | Jeff Bhasker | standard | medium | 4 |
| `joel-little` | Joel Little | standard | medium | 4 |
| `johnny-mcdaid` | Johnny McDaid | standard | medium | 4 |
| `jon-bellion` | Jon Bellion | standard | medium | 4 |
| `jonas-brothers` | Jonas Brothers | standard | medium | 4 |
| `julia-michaels` | Julia Michaels | standard | medium | 4 |
| `julian-bunetta` | Julian Bunetta | standard | medium | 4 |
| `justin-bieber` | Justin Bieber | standard | medium | 4 |
| `kid-harpoon` | Kid Harpoon | standard | medium | 4 |
| `kim-petras` | Kim Petras | standard | medium | 4 |
| `lana-del-rey` | Lana Del Rey | standard | medium | 4 |
| `little-mix` | Little Mix | standard | medium | 4 |
| `lizzo` | Lizzo | standard | medium | 4 |
| `marina` | Marina | standard | medium | 4 |
| `mark-ronson` | Mark Ronson | standard | medium | 4 |
| `meghan-trainor` | Meghan Trainor | standard | medium | 4 |
| `miley-cyrus` | Miley Cyrus | standard | medium | 4 |
| `monsters-and-strangerz` | The Monsters & Strangerz | standard | medium | 4 |
| `newjeans` | NewJeans | standard | medium | 4 |
| `noah-kahan` | Noah Kahan | standard | medium | 4 |
| `omer-fedi` | Omer Fedi | standard | medium | 4 |
| `one-direction` | One Direction | standard | medium | 4 |
| `paul-epworth` | Paul Epworth | standard | medium | 4 |
| `post-malone` | Post Malone | standard | medium | 4 |
| `ricky-reed` | Ricky Reed | standard | medium | 4 |
| `rina-sawayama` | Rina Sawayama | standard | medium | 4 |
| `rostam` | Rostam | standard | medium | 4 |
| `ryan-tedder` | Ryan Tedder | standard | medium | 4 |
| `sabrina-carpenter` | Sabrina Carpenter | standard | medium | 4 |
| `sam-smith` | Sam Smith | standard | medium | 4 |
| `selena-gomez` | Selena Gomez | standard | medium | 4 |
| `shawn-mendes` | Shawn Mendes | standard | medium | 4 |
| `sia` | Sia | standard | medium | 4 |
| `steve-mac` | Steve Mac | standard | medium | 4 |
| `stray-kids` | Stray Kids | standard | medium | 4 |
| `tate-mcrae` | Tate McRae | standard | medium | 4 |
| `teddy-swims` | Teddy Swims | standard | medium | 4 |
| `tommy-brown` | Tommy Brown | standard | medium | 4 |
| `tove-lo` | Tove Lo | standard | medium | 4 |
| `troye-sivan` | Troye Sivan | standard | medium | 4 |
| `twenty-one-pilots` | Twenty One Pilots | standard | medium | 4 |
| `twice` | Twice | standard | medium | 4 |
| `tyler-johnson` | Tyler Johnson | standard | medium | 4 |
| `zara-larsson` | Zara Larsson | standard | medium | 4 |

### rage — 5

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `homixide-gang` | Homixide Gang | flagship | high | 5 |
| `nine-vicious` | Nine Vicious | flagship | high | 3 |
| `osamason` | OsamaSon | flagship | high | 3 |
| `slayr` | Slayr | flagship | high | 2 |
| `apollored1` | ApolloRed1 | standard | medium | 4 |

### ringtone-club-rap — 16

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `d4l` | D4L | flagship | medium | 4 |
| `dem-franchize-boyz` | Dem Franchize Boyz | flagship | medium | 5 |
| `dj-unk` | DJ Unk | flagship | medium | 5 |
| `mr-collipark` | Mr. Collipark | flagship | medium | 6 |
| `t-pain` | T-Pain | flagship | medium | 7 |
| `yung-joc` | Yung Joc | flagship | medium | 4 |
| `bubba-sparxxx` | Bubba Sparxxx | standard | medium | 4 |
| `dj-montay` | DJ Montay | standard | high | 4 |
| `huey` | Huey | standard | low | 4 |
| `hurricane-chris` | Hurricane Chris | standard | low | 4 |
| `j-kwon` | J-Kwon | standard | medium | 4 |
| `k-rab` | K-Rab | standard | medium | 5 |
| `mims` | Mims | standard | medium | 5 |
| `nitti` | Nitti | standard | medium | 4 |
| `shop-boyz` | Shop Boyz | standard | medium | 4 |
| `vic` | V.I.C. | standard | low | 5 |

### rnb-2000s — 110

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `aaliyah` | Aaliyah | flagship | high | 4 |
| `ashanti` | Ashanti | flagship | high | 4 |
| `blackstreet` | Blackstreet | flagship | high | 3 |
| `brandy` | Brandy | flagship | high | 4 |
| `bryan-michael-cox` | Bryan-Michael Cox | flagship | high | 3 |
| `destinys-child` | Destiny's Child | flagship | high | 3 |
| `mario` | Mario | flagship | high | 3 |
| `mary-j-blige` | Mary J. Blige | flagship | high | 4 |
| `ne-yo` | Ne-Yo | flagship | high | 4 |
| `rich-harrison` | Rich Harrison | flagship | high | 3 |
| `rodney-darkchild-jerkins` | Rodney "Darkchild" Jerkins | flagship | high | 3 |
| `sisqo` | Sisqó | flagship | high | 4 |
| `stargate` | Stargate (Mikkel S. Eriksen & Tor Erik Hermansen) | flagship | high | 4 |
| `teddy-riley` | Teddy Riley | flagship | high | 3 |
| `tlc` | TLC | flagship | high | 4 |
| `toni-braxton` | Toni Braxton | flagship | high | 3 |
| `tricky-stewart` | Tricky Stewart | flagship | high | 3 |
| `112` | 112 | standard | medium | 4 |
| `3lw` | 3LW | standard | low | 4 |
| `adina-howard` | Adina Howard | standard | medium | 3 |
| `alicia-keys` | Alicia Keys | standard | medium | 4 |
| `allure` | Allure | standard | low | 3 |
| `amerie` | Amerie | standard | medium | 4 |
| `anthony-dent` | Anthony Dent | standard | low | 3 |
| `avant` | Avant | standard | medium | 3 |
| `az-yet` | Az Yet | standard | low | 3 |
| `b2k` | B2K | standard | low | 3 |
| `babyface` | Babyface (Kenneth Edmonds) | standard | medium | 4 |
| `blaque` | Blaque | standard | low | 3 |
| `bobby-valentino` | Bobby Valentino | standard | medium | 3 |
| `boyz-ii-men` | Boyz II Men | standard | medium | 3 |
| `brian-alexander-morgan` | Brian Alexander Morgan | standard | medium | 3 |
| `carvin-and-ivan` | Carvin & Ivan (Carvin Haggins & Ivan Barias) | standard | low | 4 |
| `case` | Case | standard | low | 3 |
| `cassie` | Cassie | standard | medium | 4 |
| `changing-faces` | Changing Faces | standard | low | 3 |
| `chante-moore` | Chanté Moore | standard | low | 3 |
| `christina-milian` | Christina Milian | standard | low | 3 |
| `ciara` | Ciara | standard | medium | 4 |
| `cory-rooney` | Cory Rooney | standard | low | 3 |
| `dallas-austin` | Dallas Austin | standard | medium | 3 |
| `danity-kane` | Danity Kane | standard | low | 3 |
| `danja` | Danja (Floyd Nathaniel Hills) | standard | medium | 3 |
| `day26` | Day26 | standard | low | 3 |
| `deborah-cox` | Deborah Cox | standard | medium | 3 |
| `devante-swing` | DeVanté Swing (Donald DeGrate) | standard | medium | 3 |
| `donell-jones` | Donell Jones | standard | medium | 3 |
| `dru-hill` | Dru Hill | standard | medium | 3 |
| `en-vogue` | En Vogue | standard | medium | 3 |
| `eric-hudson` | Eric Hudson | standard | medium | 3 |
| `faith-evans` | Faith Evans | standard | medium | 3 |
| `ginuwine` | Ginuwine | standard | medium | 3 |
| `h-town` | H-Town | standard | medium | 4 |
| `immature` | Immature / IMx | standard | low | 4 |
| `jagged-edge` | Jagged Edge | standard | medium | 3 |
| `jermaine-dupri` | Jermaine Dupri | standard | high | 3 |
| `jimmy-jam-and-terry-lewis` | Jimmy Jam & Terry Lewis | standard | medium | 4 |
| `jodeci` | Jodeci | standard | medium | 3 |
| `joe` | Joe | standard | low | 3 |
| `john-legend` | John Legend | standard | medium | 4 |
| `jon-b` | Jon B. | standard | low | 3 |
| `jr-rotem` | JR Rotem | standard | medium | 3 |
| `kelis` | Kelis | standard | medium | 4 |
| `kelly-price` | Kelly Price | standard | medium | 3 |
| `keri-hilson` | Keri Hilson | standard | medium | 3 |
| `keyshia-cole` | Keyshia Cole | standard | medium | 3 |
| `kwame` | Kwamé (Kwamé Holland / K-1 Million) | standard | low | 3 |
| `lloyd` | Lloyd | standard | medium | 3 |
| `los-da-mystro` | Los Da Mystro (Carlos McKinney) | standard | medium | 3 |
| `mario-winans` | Mario Winans | standard | low | 3 |
| `mark-batson` | Mark Batson | standard | medium | 3 |
| `marques-houston` | Marques Houston | standard | low | 3 |
| `mike-city` | Mike City (Michael Flowers) | standard | medium | 3 |
| `missy-elliott` | Missy Elliott | standard | medium | 3 |
| `monica` | Monica | standard | medium | 3 |
| `montell-jordan` | Montell Jordan | standard | medium | 3 |
| `mya` | Mýa | standard | medium | 3 |
| `next` | Next | standard | medium | 4 |
| `nivea` | Nivea | standard | low | 3 |
| `omarion` | Omarion | standard | medium | 4 |
| `pretty-ricky` | Pretty Ricky | standard | medium | 3 |
| `ray-j` | Ray J | standard | low | 3 |
| `rihanna` | Rihanna | standard | medium | 4 |
| `ron-fair` | Ron Fair | standard | medium | 3 |
| `ryan-leslie` | Ryan Leslie | standard | medium | 3 |
| `salt-n-pepa` | Salt-N-Pepa | standard | medium | 3 |
| `sammie` | Sammie | standard | low | 3 |
| `sean-garrett` | Sean Garrett | standard | medium | 3 |
| `shai` | Shai | standard | medium | 3 |
| `shanice` | Shanice | standard | low | 3 |
| `shekspere` | She'kspere (Kevin "She'kspere" Briggs) | standard | medium | 3 |
| `silk` | Silk | standard | low | 3 |
| `soulshock-and-karlin` | Soulshock & Karlin | standard | medium | 4 |
| `static-major` | Static Major (Stephen Garrett) | standard | medium | 3 |
| `steve-stone-huff` | Steve "Stone" Huff | standard | medium | 3 |
| `swv` | SWV | standard | medium | 3 |
| `tamia` | Tamia | standard | low | 3 |
| `tank` | Tank | standard | medium | 3 |
| `teairra-mari` | Teairra Marí | standard | low | 3 |
| `the-dream` | The-Dream (Terius Nash) | standard | medium | 4 |
| `tim-and-bob` | Tim & Bob (Tim Kelley & Bob Robinson) | standard | medium | 4 |
| `total` | Total | standard | medium | 4 |
| `troy-taylor` | Troy Taylor | standard | low | 3 |
| `truth-hurts` | Truth Hurts | standard | medium | 3 |
| `tyrese` | Tyrese | standard | medium | 3 |
| `underdogs` | The Underdogs (Harvey Mason Jr. & Damon Thomas) | standard | medium | 4 |
| `walter-afanasieff` | Walter Afanasieff | standard | medium | 3 |
| `warryn-campbell` | Warryn Campbell | standard | low | 3 |
| `wyclef-jean-and-jerry-wonda-duplessis` | Wyclef Jean & Jerry "Wonda" Duplessis | standard | medium | 4 |
| `xscape` | Xscape | standard | medium | 4 |

### trap — 102

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `2-chainz` | 2 Chainz | flagship | medium | 5 |
| `atl-jacob` | ATL Jacob | flagship | medium | 3 |
| `bangladesh` | Bangladesh | flagship | high | 3 |
| `big-sean` | Big Sean | flagship | medium | 3 |
| `cardi-b` | Cardi B | flagship | high | 3 |
| `city-girls` | City Girls | flagship | medium | 3 |
| `dababy` | DaBaby | flagship | high | 4 |
| `dj-khaled` | DJ Khaled | flagship | medium | 3 |
| `dj-toomp` | DJ Toomp | flagship | high | 3 |
| `doechii` | Doechii | flagship | medium | 3 |
| `don-toliver` | Don Toliver | flagship | medium | 3 |
| `drake` | Drake | flagship | high | 4 |
| `drumma-boy` | Drumma Boy | flagship | high | 3 |
| `frank-dukes` | Frank Dukes | flagship | high | 3 |
| `future` | Future | flagship | high | 3 |
| `gucci-mane` | Gucci Mane | flagship | medium | 6 |
| `jack-harlow` | Jack Harlow | flagship | medium | 3 |
| `jeezy` | Jeezy | flagship | medium | 5 |
| `kenny-beats` | Kenny Beats | flagship | medium | 3 |
| `kodak-black` | Kodak Black | flagship | medium | 5 |
| `latto` | Latto | flagship | medium | 4 |
| `lil-yachty` | Lil Yachty | flagship | medium | 4 |
| `megan-thee-stallion` | Megan Thee Stallion | flagship | medium | 4 |
| `metro-boomin` | Metro Boomin | flagship | high | 3 |
| `migos` | Migos | flagship | high | 6 |
| `offset` | Offset | flagship | medium | 4 |
| `quavo` | Quavo | flagship | medium | 4 |
| `rae-sremmurd` | Rae Sremmurd | flagship | medium | 3 |
| `roddy-ricch` | Roddy Ricch | flagship | medium | 3 |
| `shawty-redd` | Shawty Redd | flagship | medium | 3 |
| `sonny-digital` | Sonny Digital | flagship | medium | 3 |
| `southside` | Southside | flagship | high | 3 |
| `ti` | T.I. | flagship | high | 5 |
| `travis-scott` | Travis Scott | flagship | high | 4 |
| `trippie-redd` | Trippie Redd | flagship | medium | 4 |
| `turbo` | Turbo | flagship | medium | 3 |
| `waka-flocka-flame` | Waka Flocka Flame | flagship | medium | 6 |
| `24kgoldn` | 24kGoldn | standard | medium | 4 |
| `30-roc` | 30 Roc | standard | medium | 3 |
| `b-o-b` | B.o.B | standard | medium | 5 |
| `bia` | BIA | standard | medium | 3 |
| `bossman-dlow` | BossMan Dlow | standard | medium | 3 |
| `buddah-bless` | Buddah Bless | standard | medium | 3 |
| `cardo` | Cardo | standard | medium | 3 |
| `charlie-handsome` | Charlie Handsome | standard | high | 2 |
| `chasethemoney` | ChaseTheMoney | standard | medium | 3 |
| `coi-leray` | Coi Leray | standard | medium | 3 |
| `da-doman` | D.A. Doman | standard | high | 3 |
| `deko` | Deko | standard | medium | 3 |
| `desiigner` | Desiigner | standard | medium | 4 |
| `dj-durel` | DJ Durel | standard | medium | 3 |
| `dj-spinz` | DJ Spinz | standard | medium | 3 |
| `dun-deal` | Dun Deal | standard | medium | 3 |
| `dy-krazy` | DY Krazy | standard | low | 4 |
| `erica-banks` | Erica Banks | standard | low | 2 |
| `est-gee` | EST Gee | standard | medium | 4 |
| `fki-1st` | FKi 1st | standard | medium | 4 |
| `flo-milli` | Flo Milli | standard | medium | 3 |
| `foreign-teck` | Foreign Teck | standard | medium | 4 |
| `hitmaka` | Hitmaka | standard | medium | 3 |
| `honorable-cnote` | Honorable C.N.O.T.E. | standard | medium | 4 |
| `hunxho` | Hunxho | standard | low | 3 |
| `ilovemakonnen` | iLoveMakonnen | standard | medium | 5 |
| `j-white-did-it` | J. White Did It | standard | high | 4 |
| `jetsonmade` | JetsonMade | standard | high | 3 |
| `jt` | JT | standard | medium | 3 |
| `k-camp` | K Camp | standard | medium | 5 |
| `lil-keed` | Lil Keed | standard | medium | 4 |
| `lil-mosey` | Lil Mosey | standard | medium | 3 |
| `lil-poppa` | Lil Poppa | standard | medium | 3 |
| `lil-tjay` | Lil Tjay | standard | medium | 3 |
| `luh-tyler` | Luh Tyler | standard | low | 3 |
| `nard-and-b` | Nard & B | standard | medium | 4 |
| `nardo-wick` | Nardo Wick | standard | medium | 3 |
| `nineteen85` | Nineteen85 | standard | medium | 3 |
| `og-parker` | OG Parker | standard | medium | 3 |
| `oj-da-juiceman` | OJ da Juiceman | standard | medium | 5 |
| `oz` | OZ | standard | medium | 3 |
| `pyrex-whippa` | Pyrex Whippa | standard | low | 3 |
| `real-boston-richey` | Real Boston Richey | standard | low | 3 |
| `rich-homie-quan` | Rich Homie Quan | standard | medium | 5 |
| `rich-the-kid` | Rich the Kid | standard | medium | 4 |
| `roget-chahayed` | Rogét Chahayed | standard | high | 3 |
| `rylo-rodriguez` | Rylo Rodriguez | standard | medium | 3 |
| `sexyy-red` | Sexyy Red | standard | medium | 3 |
| `shawty-lo` | Shawty Lo | standard | medium | 4 |
| `sheck-wes` | Sheck Wes | standard | medium | 3 |
| `slim-jxmmi` | Slim Jxmmi | standard | medium | 3 |
| `swae-lee` | Swae Lee | standard | medium | 3 |
| `t-minus` | T-Minus | standard | medium | 3 |
| `take-a-daytrip` | Take a Daytrip | standard | high | 4 |
| `takeoff` | Takeoff | standard | medium | 3 |
| `tntxd` | TnTXD | standard | medium | 3 |
| `toosii` | Toosii | standard | medium | 3 |
| `travis-porter` | Travis Porter | standard | medium | 5 |
| `trinidad-james` | Trinidad James | standard | medium | 4 |
| `vinylz` | Vinylz | standard | medium | 2 |
| `wondagurl` | WondaGurl | standard | high | 3 |
| `yfn-lucci` | YFN Lucci | standard | medium | 4 |
| `yung-la` | Yung L.A. | standard | medium | 5 |
| `yung-lan` | Yung Lan | standard | medium | 3 |
| `yung-miami` | Yung Miami | standard | low | 3 |

### trap-soul — 70

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `frank-ocean` | Frank Ocean | flagship | high | 5 |
| `070-shake` | 070 Shake | standard | low | 4 |
| `6lack` | 6LACK | standard | medium | 4 |
| `alina-baraz` | Alina Baraz | standard | low | 3 |
| `amaarae` | Amaarae | standard | low | 4 |
| `arin-ray` | Arin Ray | standard | low | 3 |
| `august-alsina` | August Alsina | standard | low | 4 |
| `bizness-boi` | Bizness Boi | standard | low | 3 |
| `bj-the-chicago-kid` | BJ the Chicago Kid | standard | low | 4 |
| `blood-orange` | Blood Orange (Dev Hynes) | standard | medium | 4 |
| `bongo-bytheway` | Bongo ByTheWay | standard | low | 3 |
| `buddy-ross` | Buddy Ross | standard | low | 3 |
| `carter-lang` | Carter Lang | standard | low | 3 |
| `chloe` | Chlöe | standard | low | 3 |
| `coco-jones` | Coco Jones | standard | low | 3 |
| `dmile-trap-soul-window` | D'Mile (Dernst Emile II) — 2019+ trap-soul-era window | standard | medium | 4 |
| `dpat` | Dpat | standard | low | 3 |
| `dvsn` | dvsn | standard | medium | 3 |
| `ella-mai` | Ella Mai | standard | medium | 3 |
| `eric-bellinger` | Eric Bellinger | standard | low | 4 |
| `fisticuffs` | Fisticuffs (Mac Robinson & Brian Warfield) | standard | medium | 4 |
| `fridayy` | Fridayy | standard | low | 3 |
| `gallant` | Gallant | standard | low | 4 |
| `giveon` | Giveon | standard | medium | 4 |
| `halle` | Halle | standard | low | 3 |
| `happy-perez` | Happy Perez | standard | medium | 3 |
| `jacquees` | Jacquees | standard | medium | 3 |
| `jahaan-sweet` | Jahaan Sweet | standard | low | 3 |
| `james-blake` | James Blake | standard | medium | 4 |
| `jeff-ellis` | Jeff Ellis | standard | medium | 3 |
| `jeff-gitelman` | Jeff Gitelman | standard | low | 3 |
| `jeremih` | Jeremih | standard | medium | 3 |
| `jhene-aiko` | Jhené Aiko | standard | medium | 4 |
| `kali-uchis` | Kali Uchis | standard | medium | 4 |
| `kehlani` | Kehlani | standard | medium | 4 |
| `khalid` | Khalid | standard | medium | 4 |
| `kiana-lede` | Kiana Ledé | standard | low | 4 |
| `leon-thomas` | Leon Thomas | standard | low | 4 |
| `lucky-daye` | Lucky Daye | standard | medium | 4 |
| `mahalia` | Mahalia | standard | low | 4 |
| `majid-jordan` | Majid Jordan | standard | medium | 4 |
| `malay` | Malay (James Ho) | standard | medium | 3 |
| `michael-uzowuru` | Michael Uzowuru | standard | low | 3 |
| `miguel` | Miguel | standard | medium | 4 |
| `monte-booker` | Monte Booker | standard | low | 3 |
| `muni-long` | Muni Long | standard | low | 3 |
| `normani` | Normani | standard | low | 3 |
| `pop-and-oak` | Pop & Oak (Pop Wansel & Oak Felder) | standard | low | 4 |
| `queen-naija` | Queen Naija | standard | low | 4 |
| `ravyn-lenae` | Ravyn Lenae | standard | low | 4 |
| `ro-james` | Ro James | standard | low | 4 |
| `rob-bisel` | Rob Bisel | standard | low | 3 |
| `roy-woods` | Roy Woods | standard | low | 3 |
| `sabrina-claudio` | Sabrina Claudio | standard | low | 3 |
| `sampha` | Sampha | standard | medium | 4 |
| `sango` | Sango | standard | medium | 4 |
| `sevyn-streeter` | Sevyn Streeter | standard | low | 4 |
| `sonder` | Sonder | standard | low | 3 |
| `syd` | Syd / The Internet | standard | medium | 4 |
| `syk-sense` | Syk Sense | standard | low | 3 |
| `tems` | Tems | standard | low | 4 |
| `teyana-taylor` | Teyana Taylor | standard | low | 4 |
| `thankgod4cody` | ThankGod4Cody | standard | low | 3 |
| `tinashe` | Tinashe | standard | medium | 4 |
| `tone-stith` | Tone Stith | standard | low | 4 |
| `tory-lanez` | Tory Lanez | standard | medium | 4 |
| `trey-songz-trap-soul-window` | Trey Songz — 2011+ trap-soul window | standard | medium | 4 |
| `vegyn` | Vegyn (Joe Thornalley) | standard | low | 3 |
| `victoria-monet` | Victoria Monét | standard | medium | 3 |
| `yung-bleu` | Yung Bleu | standard | low | 4 |

### west-coast-club — 55

| id | name | tier | confidence | aliases |
|---|---|---|---|---|
| `anderson-paak` | Anderson .Paak | flagship | high | 5 |
| `dj-dahi` | DJ Dahi | flagship | medium | 3 |
| `dj-khalil` | DJ Khalil | flagship | medium | 3 |
| `kendrick-lamar` | Kendrick Lamar | flagship | high | 7 |
| `nipsey-hussle` | Nipsey Hussle | flagship | medium | 3 |
| `ron-ron-the-producer` | Ron-RonTheProducer | flagship | medium | 5 |
| `schoolboy-q` | ScHoolboy Q | flagship | medium | 5 |
| `shoreline-mafia` | Shoreline Mafia | flagship | high | 5 |
| `sounwave` | Sounwave | flagship | high | 2 |
| `terrace-martin` | Terrace Martin | flagship | high | 2 |
| `thundercat` | Thundercat | flagship | medium | 2 |
| `ty-dolla-sign` | Ty Dolla $ign | flagship | medium | 4 |
| `tyga` | Tyga | flagship | medium | 3 |
| `vince-staples` | Vince Staples | flagship | medium | 4 |
| `yg` | YG | flagship | high | 4 |
| `03-greedo` | 03 Greedo | standard | medium | 4 |
| `1500-or-nothin` | 1500 or Nothin' | standard | medium | 4 |
| `ab-soul` | Ab-Soul | standard | medium | 4 |
| `audio-push` | Audio Push | standard | low | 4 |
| `azchike` | AzChike | standard | medium | 2 |
| `baby-keem` | Baby Keem | standard | medium | 4 |
| `blueface` | Blueface | standard | medium | 3 |
| `blxst` | Blxst | standard | high | 4 |
| `buddy` | Buddy | standard | low | 2 |
| `casey-veggies` | Casey Veggies | standard | medium | 3 |
| `dem-jointz` | Dem Jointz | standard | medium | 2 |
| `digi-phonics` | Digi+Phonics | standard | medium | 6 |
| `dj-fu` | DJ Fu | standard | medium | 2 |
| `dom-kennedy` | Dom Kennedy | standard | medium | 3 |
| `duval-timothy` | Duval Timothy | standard | medium | 1 |
| `fenix-flexin` | Fenix Flexin | standard | medium | 2 |
| `focus` | Focus... | standard | medium | 3 |
| `g-eazy` | G-Eazy | standard | medium | 3 |
| `g-perico` | G Perico | standard | medium | 4 |
| `isaiah-rashad` | Isaiah Rashad | standard | medium | 3 |
| `jay-rock` | Jay Rock | standard | medium | 3 |
| `kal-banx` | Kal Banx | standard | medium | 2 |
| `kamaiyah` | Kamaiyah | standard | medium | 3 |
| `kid-ink` | Kid Ink | standard | medium | 2 |
| `knxwledge` | Knxwledge | standard | medium | 3 |
| `larry-june` | Larry June | standard | medium | 3 |
| `mike-and-keys` | Mike & Keys | standard | medium | 4 |
| `mozzy` | Mozzy | standard | medium | 4 |
| `new-boyz` | New Boyz | standard | low | 4 |
| `nez-and-rio` | Nez & Rio | standard | medium | 4 |
| `ohgeesy` | OhGeesy | standard | medium | 3 |
| `ot-genasis` | O.T. Genasis | standard | medium | 3 |
| `problem` | Problem | standard | low | 3 |
| `rahki` | Rahki | standard | medium | 2 |
| `saweetie` | Saweetie | standard | medium | 2 |
| `scoop-deville` | Scoop DeVille | standard | medium | 3 |
| `sir` | SiR | standard | medium | 3 |
| `steve-lacy` | Steve Lacy | standard | medium | 3 |
| `taz-arnold` | Taz Arnold | standard | low | 3 |
| `teeflii` | TeeFlii | standard | low | 3 |

<!-- ROSTER-LEDGER:END -->

---

## 7. Regenerating the ledger

```sh
node scripts/roster-ledger.mjs         # rewrite Appendix A in place
node scripts/roster-ledger.mjs --check # fail if it is stale (the CI gate)
```

The ledger is derived, so it is regenerated rather than maintained. `--check` is
what stops it drifting from `data/`.

---

## 8. The novelty table

`data/novelty/hooks.hash` is the reference table the novelty guard (FR-011,
TASK-039) screens every generated melody against. This section is the documented
process the task asks for: what the file is, how it is rebuilt, and why its
inputs are not in this repo.

### What is in the file, and what is deliberately not

**Hashes, and nothing else.** Each line is one `0x`-prefixed 64-bit fingerprint
of a *contour* — a run of `(interval, onset-gap)` steps — and there is no way
back from one to a note. Nothing in the file names a pitch, a key, a tempo or a
title. That is the whole point: the table has to ship inside a product that must
never carry somebody else's notes.

**A fingerprint is transposition- and articulation-blind by construction.** The
interval side is the semitone step between consecutive notes, so a hook moved to
another key fingerprints identically. The rhythm side is the *gap between
onsets*, not the note's length, so the same line played staccato and legato
fingerprints identically — and a humanised 478-tick eighth quantises onto the
same rung as an exact 480, which is what lets the guard run before or after the
humanizer and get the same answer.

Each melody contributes hashes at **both** widths the guard looks up: `n = 8`,
the screen every take must pass, and `n = 12`, the loosened screen it falls back
to after four refusals. A lookup at one width can only ever find a hash written
at that width.

### The contour listing format

One melody per file, `*.contour`, so an n-gram can never straddle two of them
and fingerprint a phrase nobody wrote. A file is whitespace-separated tokens:

```
<interval>:<note value>
```

- **`<interval>`** — semitones from the previous note, signed (`+2`, `-5`, `0`).
  Clamped to ±24, because a two-octave leap and a three-octave one are the same
  event to a contour.
- **`<note value>`** — the gap to the next note, spelled the way the rest of the
  dataset spells a note value: `4`, `8`, `8th`, `16`, `16T`, `1/8`. It is read by
  `grid::note_value_ticks`, not by a second parser.
- `#` starts a comment. Newlines are whitespace — break a listing across lines
  by phrase, it makes no difference to the output.

A melody of *k* notes is *k − 1* tokens. ⚠ **The ladder has no dotted values**,
so a dotted quarter is written as the rung it quantises to (`2`). That coarsens
the screen, which errs towards catching more than it should rather than less —
the safe direction for a guard whose bad outcome is letting a known hook
through.

### Rebuilding the table

```sh
cargo run -p datasetc -- novelty <contours>/ > data/novelty/hooks.hash
```

The command prints one line per listing with its step count, then the totals, to
stderr; the table itself goes to stdout. It **fails rather than skips** on a
listing it cannot read, because a mistyped note value would otherwise cost that
melody its entry in silence.

Then re-add the file's header comment — the command emits hashes only — or copy
it from the previous version.

### ⛔ Why the contour listings are not in this repo

FR-011's rule is "hashes, not note data", and a contour listing *is* note data
with the key filed off: transposition-invariant, but still a melody. So the
listings are an **input**, authored at research time from public melodic-contour
listings, and never committed. What is committed is the irreversible output.

That has a cost worth naming: rebuilding the table means re-deriving the
contours, not re-running a script over checked-in files. The list below is what
makes that possible.

### What the shipped table was built from

A **public-domain starter set** — seven traditional and classical melodies.
It exists so the guard is a working mechanism today rather than a switch waiting
to be wired, and it is expected to grow as the research-time listings are
encoded.

| Melody | Steps | Status |
|---|---|---|
| Frère Jacques | 31 | traditional, public domain |
| Ode to Joy — Beethoven, Symphony No. 9 | 29 | public domain |
| Twinkle, Twinkle, Little Star (*Ah! vous dirai-je, maman*) | 13 | traditional, public domain |
| Mary Had a Little Lamb | 12 | traditional, public domain |
| London Bridge Is Falling Down | 12 | traditional, public domain |
| Jingle Bells — J. L. Pierpont, 1857 | 10 | public domain |
| Für Elise — Beethoven, WoO 59 (opening figure) | 8 | public domain |

104 distinct hashes across both widths. A melody shorter than 12 steps
contributes at the tight width only, and one of exactly 8 — Für Elise's opening
figure — contributes a single hash; that is correct rather than a shortfall.

### The gates on it

- `the_bundled_table_parses_and_is_not_empty` (`engine/src/novelty.rs`) — the
  table is compiled in with `include_str!`, and a malformed one degrades to an
  empty table rather than panicking inside a DAW. This test is what makes that
  degraded path unreachable.
- `the_shipped_roster_walks_past_the_bundled_table` (`engine/tests/novelty.rs`)
  — every shipped model, both screened parts, five seeds. ⛔ A failure here is
  **news, not necessarily a bug**: it means a shipped artist's melody collides
  with a reference contour, and without the test every generation of that artist
  would silently be a retry.
- `screening_costs_less_than_the_five_millisecond_budget` — FR-011's stated
  overhead.

### Changing the fingerprint

`SCHEME` in `engine/src/novelty.rs` is mixed into every hash. **Bump it whenever
a step's meaning changes** — the ladder, the clamp, the choice of onset gap over
note length — and rebuild the table. Without that the two schemes share a number
space, and a stale table goes on matching things it never described.
