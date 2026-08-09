//! Fitting a style model to the generations a producer kept (TASK-040T).
//!
//! ⛔⛔ **"Trained" means fitted, and there is no machine learning anywhere in
//! it.** This engine is offline by design — no network, no ML, and
//! `scripts/check-denylist.mjs` enforces it. Training here is **parameter
//! fitting**: measure the features of the kept generations, and write those
//! distributions back as a model's authored ranges and weights. A [`StyleModel`]
//! *is* those numbers, so the output is an ordinary model the existing
//! generators already know how to read — no new code path, no second kind of
//! artist. Saying this plainly because "train" invites someone to reach for a
//! dependency this product's offline guarantee does not permit.
//!
//! ## The failure mode this module is mostly about
//!
//! ⛔ **Fitting narrows a grammar, and a narrowed grammar repeats itself.** If
//! every kept generation happened to have five onsets a bar, the naive fit
//! writes `densityPerBar: 5` and the new artist produces one beat forever. *"I
//! trained my own artist and now everything sounds identical"* would be this
//! feature working exactly as specified and being useless. So two rules are
//! built in rather than left to the caller:
//!
//! - **Ranges never collapse to a point** ([`MIN_DENSITY_SPREAD`],
//!   [`MIN_REGISTER_SPREAD`]). Unanimity across thirty samples is weak evidence
//!   that the thirty-first would agree, and a range of zero width is a constant
//!   pretending to be a distribution.
//! - **Weights are fitted, not argmaxed** ([`smooth`]). An 18/8/4 split becomes
//!   roughly 18/8/4, never "always the first one" — dropping the tail is what
//!   turns a grammar into a template.
//!
//! Neither is a substitute for the real gate: a fitted model faces the same
//! per-part variety floor a shipped one does, and [`verify_variety`] is what
//! says so before the producer is handed something that repeats.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::pattern::{Lane, Part, Pattern};

/// The fewest kept generations a fit will run on.
///
/// ⛔ **Thirty, and it is not arbitrary.** The fit estimates three-way weight
/// splits (`intervals`, `contour`) and min/max ranges; below about thirty a
/// single kept outlier sets a range's edge, so the model encodes the seed's
/// opinion rather than the producer's taste. Mike asked for a floor —
/// *"you should have a minimum number of generations that you HAVE to train
/// on"* — and this is it, in one place, with the reason beside it so raising it
/// is a deliberate change rather than a quiet one.
pub const MIN_KEPT: usize = 30;

/// The narrowest a fitted onsets-per-bar range may be.
pub const MIN_DENSITY_SPREAD: f64 = 2.0;

/// The narrowest a fitted register may be, in semitones.
///
/// An octave: a melody confined to less than one cannot move, whatever the
/// kept set happened to do.
pub const MIN_REGISTER_SPREAD: u8 = 12;

/// Why a fit was refused, in words the producer can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FitError {
    /// Fewer than [`MIN_KEPT`] were kept. Carries the count so the UI can show
    /// `18 / 30` rather than greying a button out with no explanation.
    NotEnough { kept: usize, needed: usize },
    /// Kept generations that are not all the same part, or none at all.
    NothingToMeasure(String),
}

impl std::fmt::Display for FitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEnough { kept, needed } => write!(
                f,
                "{kept} of {needed} kept — keep more generations before training"
            ),
            Self::NothingToMeasure(why) => write!(f, "{why}"),
        }
    }
}

/// What one kept generation contributes to the fit.
///
/// Measured from a [`Pattern`] and nothing else, which is the constraint that
/// makes a MIDI file a training source too: anything that produces a `Pattern`
/// is measurable by exactly this code, so a model fitted from imported files
/// cannot drift from one fitted from generations.
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    pub part: Part,
    /// Note onsets per bar, across every lane.
    pub onsets_per_bar: f64,
    /// The lowest and highest pitch, for a pitched part.
    pub register: Option<(u8, u8)>,
    /// Melodic interval mix: steps (1–2), thirds (3–4), leaps (5+).
    pub intervals: [usize; 3],
    /// How many of those intervals were an octave or more.
    pub octave_jumps: usize,
    /// Which shape the line made: riff, descend, arch.
    pub contour: [usize; 3],
    /// Closed-hat onsets per bar. `None` off the drums.
    pub hat_per_bar: Option<f64>,
    /// Kick onsets per bar. `None` off the drums.
    pub kick_per_bar: Option<f64>,
}

/// How many bars this pattern actually spans, never zero.
fn bars(pattern: &Pattern) -> f64 {
    f64::from(pattern.bars.max(1))
}

/// Measure one generation.
pub fn measure(pattern: &Pattern) -> Features {
    let bars = bars(pattern);
    let onsets: usize = pattern.lanes.iter().map(|lane| lane.notes.len()).sum();

    // ⚠ Drums are measured per lane and pitched parts as one line. A drum
    // pattern's "intervals" would be the distance between a kick and a hat,
    // which is a number about General MIDI rather than about music.
    let drums = pattern.part == Part::Drums;

    let mut register = None;
    let mut intervals = [0usize; 3];
    let mut octave_jumps = 0usize;
    let mut contour = [0usize; 3];

    if !drums {
        // ⛔ **One pitch per onset, the highest.** Two notes on the same tick are
        // a stack, not a move: measured as an interval they read as a leap the
        // player never made, and a part with any stacking at all would fit as
        // leap-heavy. The top voice is what a listener follows as the line, so
        // that is what the contour and the interval mix are measured from.
        let mut by_onset: BTreeMap<u32, u8> = BTreeMap::new();
        for note in pattern.lanes.iter().flat_map(|lane| lane.notes.iter()) {
            by_onset
                .entry(note.start_tick)
                .and_modify(|top| *top = (*top).max(note.pitch))
                .or_insert(note.pitch);
        }
        let pitches: Vec<(u32, u8)> = by_onset.into_iter().collect();

        if let (Some(low), Some(high)) = (
            pitches.iter().map(|(_, p)| *p).min(),
            pitches.iter().map(|(_, p)| *p).max(),
        ) {
            register = Some((low, high));
        }

        for pair in pitches.windows(2) {
            let step = i32::from(pair[1].1).abs_diff(i32::from(pair[0].1));
            match step {
                0..=2 => intervals[0] += 1,
                3..=4 => intervals[1] += 1,
                _ => intervals[2] += 1,
            }
            if step >= 12 {
                octave_jumps += 1;
            }
        }

        contour[shape_of(&pitches)] += 1;
    }

    let per_bar = |lane: Lane| {
        pattern
            .lanes
            .iter()
            .find(|track| track.lane == lane)
            .map(|track| track.notes.len() as f64 / bars)
    };

    Features {
        part: pattern.part,
        onsets_per_bar: onsets as f64 / bars,
        register,
        intervals,
        octave_jumps,
        contour,
        hat_per_bar: drums.then(|| per_bar(Lane::ClosedHat).unwrap_or(0.0)),
        kick_per_bar: drums.then(|| per_bar(Lane::Kick).unwrap_or(0.0)),
    }
}

/// Which of riff / descend / arch this line is, as an index into `contour`.
///
/// Deliberately crude, and the crudeness is the point: these three are the only
/// shapes `melody.contour` offers, so a classifier with more opinions than the
/// generator has parameters would be measuring something nothing can act on.
fn shape_of(pitches: &[(u32, u8)]) -> usize {
    if pitches.len() < 3 {
        return 0;
    }

    let first = f64::from(pitches[0].1);
    let last = f64::from(pitches[pitches.len() - 1].1);
    let peak = pitches
        .iter()
        .enumerate()
        .max_by_key(|(_, (_, pitch))| *pitch)
        .map(|(at, _)| at)
        .unwrap_or(0);

    let inner = peak > 0 && peak < pitches.len() - 1;
    if last < first - 2.0 {
        1 // descend
    } else if inner {
        2 // arch
    } else {
        0 // riff
    }
}

/// Counts to weights, with one imaginary observation per category.
///
/// ⛔ **This is what stops a fit becoming a template.** A category observed zero
/// times out of thirty is not impossible — it is unobserved, and those are
/// different claims. Add-one is the standard answer to that and it is
/// explainable in a sentence, which matters more here than sophistication: a
/// producer whose thirty kept beats all stepped should still occasionally hear a
/// leap, or they have trained an artist that can only ever say one thing.
fn smooth(counts: [usize; 3]) -> [f64; 3] {
    [
        counts[0] as f64 + 1.0,
        counts[1] as f64 + 1.0,
        counts[2] as f64 + 1.0,
    ]
}

/// A [min, max] pair from observations, never narrower than `least`.
fn spread(values: &[f64], least: f64) -> (f64, f64) {
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !low.is_finite() || !high.is_finite() {
        return (0.0, least);
    }

    let short = least - (high - low);
    if short <= 0.0 {
        return (low, high);
    }

    // Widened symmetrically, so the fit does not drift upward every time it has
    // to widen — and floored at zero, because a negative density is not a thing
    // a generator can be asked for.
    //
    // ⛔ **The floor must not eat the width.** Clamping the low edge and leaving
    // the high one where it was silently delivered *less* than `least`: thirty
    // kept takes with no kick at all gave `[0, 1]` — width 1 against a promised
    // 2 — which is the collapse this function exists to prevent, arriving
    // through its own guard. Whatever the floor takes off the bottom is given
    // back at the top.
    let wanted_low = low - short / 2.0;
    let new_low = wanted_low.max(0.0);
    (new_low, new_low + least)
}

/// Round a probability to two places, so an authored model reads like one.
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Fit a model to the kept generations.
///
/// `base` is the genre or artist the new model extends — one only, because a
/// workflow trained across two genres would be a third genre nobody authored.
/// `moods` are the ones it was trained in, recorded rather than applied: they
/// are provenance, and the model inherits its base's `modes` unchanged.
pub fn fit(
    id: &str,
    name: &str,
    base: &str,
    moods: &[String],
    kept: &[Pattern],
) -> Result<Value, FitError> {
    if kept.len() < MIN_KEPT {
        return Err(FitError::NotEnough {
            kept: kept.len(),
            needed: MIN_KEPT,
        });
    }

    let measured: Vec<Features> = kept.iter().map(measure).collect();

    let mut blocks: Map<String, Value> = Map::new();
    for part in [Part::Drums, Part::Melody, Part::Counter, Part::Bass] {
        let of_part: Vec<&Features> = measured.iter().filter(|f| f.part == part).collect();

        // ⛔⛔ **The floor is per part, and the total is not enough.** It used to
        // test `kept.len()` alone, so 29 kept drum takes and *one* kept melody
        // reached thirty and fitted a melody block whose density, register and
        // contour all came from that single take — which is precisely the "a
        // single kept outlier sets a range's edge" failure [`MIN_KEPT`] exists
        // to prevent, walking straight through the guard named after it.
        //
        // ⚠ A part below the floor is **skipped, not fatal**: thirty kept drum
        // takes should give a drum style rather than a refusal.
        if of_part.len() < MIN_KEPT {
            continue;
        }
        if let Some((key, block)) = block_for(part, &of_part) {
            blocks.insert(key, block);
        }
    }

    if blocks.is_empty() {
        let counts = kept_by_part(kept)
            .into_iter()
            .map(|(part, count)| format!("{part:?} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(FitError::NothingToMeasure(format!(
            "no single part has {MIN_KEPT} kept generations to learn from — keep more of \
             one part rather than a few of each ({counts})"
        )));
    }

    let mut model = json!({
        "id": id,
        "type": "artist",
        "name": name,
        "extends": [base],
        // ⛔ **Provenance, and it is not decoration.** A model fitted from 31
        // generations is a different object from one fitted from 400, and the
        // producer is entitled to know which they are generating from.
        "notes": training_note(kept.len(), moods),
    });

    if let Value::Object(map) = &mut model {
        for (key, block) in blocks {
            map.insert(key, block);
        }
    }
    Ok(model)
}

/// The sentence a trained model carries about its own training.
fn training_note(kept: usize, moods: &[String]) -> String {
    let moods = if moods.is_empty() {
        "no particular mood".to_owned()
    } else {
        moods.join(", ")
    };
    format!("Trained on {kept} kept generations in {moods}.")
}

/// The authored block one part's measurements become.
fn block_for(part: Part, of_part: &[&Features]) -> Option<(String, Value)> {
    let density: Vec<f64> = of_part.iter().map(|f| f.onsets_per_bar).collect();
    let (low, high) = spread(&density, MIN_DENSITY_SPREAD);

    match part {
        Part::Drums => {
            let hats: Vec<f64> = of_part.iter().filter_map(|f| f.hat_per_bar).collect();
            let kicks: Vec<f64> = of_part.iter().filter_map(|f| f.kick_per_bar).collect();
            if hats.is_empty() && kicks.is_empty() {
                return None;
            }

            let (kick_low, kick_high) = spread(&kicks, MIN_DENSITY_SPREAD);
            // `fillDensity` is a share of the sixteenths between the base hits,
            // and sixteen is how many a 4/4 bar has. Clamped rather than
            // trusted: a kept set of half-time patterns can report more hat
            // onsets per bar than the grid it will be written back onto.
            let fill = round2((mean(&hats) / 16.0).clamp(0.0, 1.0));

            Some((
                "drums".to_owned(),
                json!({
                    "kick": { "densityPerBar": [kick_low.round(), kick_high.round()] },
                    "hihat": { "fillDensity": fill }
                }),
            ))
        }
        Part::Melody | Part::Counter | Part::Bass => {
            let shapes = share(smooth(totals(of_part, |f| f.contour)));

            let lows: Vec<u8> = of_part
                .iter()
                .filter_map(|f| f.register.map(|r| r.0))
                .collect();
            let highs: Vec<u8> = of_part
                .iter()
                .filter_map(|f| f.register.map(|r| r.1))
                .collect();
            let register = register_of(&lows, &highs);

            // ⛔⛔ **The interval mix and the octave-jump rate are measured and
            // deliberately not authored back.** `melody.intervals` weights the
            // generator's *proposal* for the next note; the chord-tone
            // constraint acts on top of it, so what comes out is not what went
            // in. Measured on the shipped trap model: it authors `step: 0.62`
            // and its own output steps about 0.33 of the time, because the
            // nearest chord tone is usually a third or more away.
            //
            // Writing the realised mix into the authored one is therefore a
            // **feedback loop rather than a fit** — the model proposes more
            // leaps, the constraint widens them further, and the next training
            // round measures wider still, so a producer who trains twice finds
            // their artist drifting in a direction they never asked for. The
            // numbers stay in [`Features`], where a panel can show them
            // honestly; they do not reach the model. `the_interval_mix_is_
            // measured_and_deliberately_not_written_back` pins both halves.
            //
            // ⚠ Density, register and contour do **not** have this shape: each
            // is the same quantity going in as coming out, so measuring the
            // output and authoring it is a fit rather than a loop.

            let mut block = json!({
                "densityPerBar": [low.round(), high.round()],
                "contour": {
                    "riff": round2(shapes[0]),
                    "descend": round2(shapes[1]),
                    "arch": round2(shapes[2])
                }
            });

            if let (Some(register), Value::Object(map)) = (register, &mut block) {
                map.insert("register".to_owned(), json!([register.0, register.1]));
            }

            let key = match part {
                Part::Melody => "melody",
                Part::Counter => "countermelody",
                _ => "bassline",
            };
            Some((key.to_owned(), block))
        }
        // Harmony is not fitted, and the reason is that a `Pattern` does not
        // carry the progression it was written against — only the notes that
        // came out. Measuring "progression families actually used" needs the
        // chord plan beside the clip, and inventing one from the pitches would
        // be a guess presented as a measurement.
        Part::Chords => None,
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn totals(of_part: &[&Features], pick: impl Fn(&Features) -> [usize; 3]) -> [usize; 3] {
    of_part.iter().fold([0; 3], |mut acc, f| {
        let counts = pick(f);
        for (slot, count) in acc.iter_mut().zip(counts) {
            *slot += count;
        }
        acc
    })
}

fn share(weights: [f64; 3]) -> [f64; 3] {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return [1.0 / 3.0; 3];
    }
    [weights[0] / total, weights[1] / total, weights[2] / total]
}

/// The register a set of observed extremes becomes, never narrower than an
/// octave and always inside MIDI.
fn register_of(lows: &[u8], highs: &[u8]) -> Option<(u8, u8)> {
    let low = *lows.iter().min()?;
    let high = *highs.iter().max()?;

    let short = MIN_REGISTER_SPREAD.saturating_sub(high - low);
    if short == 0 {
        return Some((low, high));
    }

    // ⛔ **The same rule as `spread`, and the same trap at the other end.**
    // Clamping the top at 127 while leaving the bottom where it was delivered
    // less than an octave: a line confined to MIDI 125 gave `119..127`, eight
    // semitones against a promised twelve. The ceiling gives back downwards.
    let half = short / 2 + short % 2;
    let high = high.saturating_add(half).min(127);
    let low = high.saturating_sub(MIN_REGISTER_SPREAD.max(high.saturating_sub(low)));
    Some((low, high))
}

/// Does this fitted model still offer a producer more than one beat?
///
/// ⛔ **The gate the whole module exists to pass.** Ranges and smoothing make a
/// collapse unlikely; only generating says whether it happened. A fit that
/// cannot clear the floor is reported back as something the producer can act on
/// — keep more, or keep more varied ones — rather than saved as a model that
/// repeats itself.
///
/// `seeds` and `floor` are the caller's so a test can measure honestly at a
/// smaller scale; the shipped values are 1,000 and 500, the same numbers every
/// shipped model is held to.
pub fn verify_variety(
    model: &crate::StyleModel,
    part: Part,
    seeds: u64,
    floor: usize,
) -> Result<usize, String> {
    use crate::context::{SessionContext, SessionOverrides};
    use crate::generators::{bass, chords, counter, drums, melody};
    use std::collections::BTreeSet;

    let mut seen: BTreeSet<Vec<(u32, u32, u8, u8)>> = BTreeSet::new();
    for seed in 1..=seeds {
        let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
        let harmony = chords::generate(model, &ctx, seed);
        let kit = drums::generate(model, &ctx, seed);

        let shape: Vec<(u32, u32, u8, u8)> = match part {
            Part::Drums => kit
                .iter()
                .flat_map(|track| track.notes.iter())
                .map(|n| (n.start_tick, n.len_ticks, n.pitch, n.vel))
                .collect(),
            Part::Melody => note_shape(&melody::generate(model, &ctx, seed, &harmony, &kit)),
            Part::Counter => {
                let lead = melody::generate(model, &ctx, seed, &harmony, &kit);
                note_shape(&counter::generate(model, &ctx, seed, &harmony, &lead))
            }
            Part::Bass => note_shape(&bass::generate(model, &ctx, seed, &harmony, &kit)),
            Part::Chords => note_shape(&harmony.track),
        };
        seen.insert(shape);

        // ⛔ **Stop the moment the floor is cleared.** This is exact rather than
        // an approximation — the question is "does it reach `floor`", and once
        // it has, every further seed is work whose answer cannot change. It
        // matters because this runs on the thread the host draws its window
        // from: a healthy model now costs a few hundred generations instead of
        // the full sweep, and only a model that is *about to be refused* pays
        // for all of them.
        if seen.len() >= floor {
            return Ok(seen.len());
        }
    }

    if seen.len() < floor {
        return Err(format!(
            "only {} different patterns in {seeds} seeds — too few or too similar \
             generations to learn from, so keep more, or keep more varied ones",
            seen.len()
        ));
    }
    Ok(seen.len())
}

fn note_shape(track: &crate::pattern::LaneTrack) -> Vec<(u32, u32, u8, u8)> {
    track
        .notes
        .iter()
        .map(|n| (n.start_tick, n.len_ticks, n.pitch, n.vel))
        .collect()
}

/// How many of each part were kept, for the UI's `18 / 30` readout.
pub fn kept_by_part(kept: &[Pattern]) -> BTreeMap<Part, usize> {
    let mut counts = BTreeMap::new();
    for pattern in kept {
        *counts.entry(pattern.part).or_insert(0) += 1;
    }
    counts
}
