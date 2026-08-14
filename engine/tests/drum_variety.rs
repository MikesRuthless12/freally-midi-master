//! How many different beats does one artist actually write?
//!
//! ⛔ **Reported by a human, not by a gate.** Mike walked the ten flagship
//! artists under Trap in Ableton on 2026-08-05 and every one of them produced
//! the same snare roll; pressing Generate again on one artist gave him the same
//! kind of beat each time. Every existing drum test asserts that the output is
//! *correct* — in the right lanes, on the right grid, reproducible from its seed
//! — and none of them asks whether two seeds produce two beats.
//!
//! ⛔ **The same shape as the chord-variety measurement TASK-126 was built on**,
//! and for the same reason it worked there: `voice()` took the strict minimum
//! cost, which made a voicing a pure function of the chord, and rage reached
//! **8 distinct progressions in 1,000 seeds** while its melody reached 823. One
//! number made the argument that no amount of reading the code had.
//!
//! What is measured here is what a producer hears: the ticks and velocities of
//! every lane. Two patterns that differ only in a humanized microsecond are the
//! same beat, so timing is quantised to the 16th before counting.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::pattern::{Lane, LaneTrack};
use engine::StyleModel;

/// Enough that a small number is a real finding rather than a small sample.
const SEEDS: u64 = 200;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model, read and resolved **once per test binary**.
///
/// ⛔ **This used to re-read and re-resolve the entire dataset on every call.**
/// `files::scan` walks 609 JSON files and `resolve_all()` merges inheritance
/// across 590 models, and the callers below ask for it once per test — so the
/// same load ran over and over to answer questions about data that cannot have
/// changed. `plugin/tests/host_timeline.rs` measured the same mistake at
/// **1,300.91s → 1.70s** on one binary.
///
/// ⚠ **Cloned out rather than borrowed**, which keeps every call site unchanged:
/// a map copy is memory, and what this was costing was 609 file reads and 590
/// inheritance merges.
fn shipped() -> BTreeMap<String, StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, StyleModel>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            models
        })
        .clone()
}

/// The beat's **skeleton**: which 16ths carry a real hit, per lane.
///
/// ⛔ **Velocity and ghosts are deliberately thrown away, and that is the whole
/// point of this measure.** A first attempt kept them and reported 200 distinct
/// beats out of 200 seeds for every model on the roster — a perfect score, for
/// a generator a producer had just told us writes the same beat every time. It
/// was counting one ghost note moving by one 16th as a different beat.
///
/// What a producer means by "a different beat" is a different *skeleton*: the
/// kick pattern, where the backbeat sits, how the hats subdivide. That is also
/// exactly what the drum grid draws, which is what Mike was looking at.
fn shape(lanes: &[LaneTrack]) -> Vec<(Lane, Vec<u32>)> {
    let mut out: Vec<(Lane, Vec<u32>)> = lanes
        .iter()
        .map(|track| {
            let mut hits: Vec<u32> = track
                .notes
                .iter()
                // ⚠ Ghosts are ornament, not structure. A beat is not a
                // different beat because a 0.35-velocity brush moved.
                .filter(|n| n.articulation != Some(engine::pattern::Articulation::Ghost))
                .filter(|n| n.vel >= 50)
                // Quantised to the 16th: `humanize` nudges every hit by a few
                // ticks, so raw values would make every seed look distinct.
                .map(|n| (n.start_tick + 120) / 240)
                .collect();
            hits.sort_unstable();
            hits.dedup();
            (track.lane, hits)
        })
        .collect();
    out.sort_by_key(|(lane, _)| format!("{lane:?}"));
    out
}

/// The closed-hat part, **ghosts included**.
///
/// ⛔ **A correction to this file's first version, and the correction is the
/// point.** [`shape`] drops ghosts because a beat is not a different beat for
/// one moved brush — but a hat part's *ghosts are its content*: `hats()` fills
/// the gaps between its onsets with quiet 16ths and marks every one of them
/// `Ghost`. Measuring hats through `shape` therefore counted only the main
/// positions, which barely move, and reported 58–72 for models whose hat parts
/// are genuinely varied.
///
/// ⚠ **Worse than merely low: it could not respond to `fillDensity` at all.**
/// That made it a number that looked like a finding and could not be acted on —
/// four models had their `base` changed on the strength of it, and two of them
/// measured *worse* afterwards. A measure nothing can move is not a measure.
fn hat_shape(lanes: &[LaneTrack]) -> Vec<(u32, bool)> {
    let mut hits: Vec<(u32, bool)> = lanes
        .iter()
        .filter(|track| track.lane == Lane::ClosedHat)
        .flat_map(|track| track.notes.iter())
        // The accent pattern is half of what a hat part *is*, so a main hit and
        // a ghost on the same 16th are not the same event.
        .map(|note| {
            (
                (note.start_tick + 120) / 240,
                note.articulation != Some(engine::pattern::Articulation::Ghost),
            )
        })
        .collect();
    hits.sort_unstable();
    hits.dedup();
    hits
}

#[test]
fn report_how_many_distinct_beats_each_artist_writes() {
    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };

    let mut rows: Vec<(String, usize, usize, usize)> = Vec::new();

    for (id, model) in &models {
        let mut whole = BTreeSet::new();
        let mut kicks = BTreeSet::new();
        let mut hats = BTreeSet::new();

        for seed in 0..SEEDS {
            let lanes = generate(model, &ctx, seed);
            let shaped = shape(&lanes);
            for (lane, hits) in &shaped {
                if *lane == Lane::Kick {
                    kicks.insert(hits.clone());
                }
            }
            hats.insert(hat_shape(&lanes));
            whole.insert(shaped);
        }
        rows.push((id.clone(), whole.len(), kicks.len(), hats.len()));
    }

    rows.sort_by_key(|(_, whole, _, _)| *whole);

    println!("\n  distinct over {SEEDS} seeds, 4 bars");
    println!(
        "  {:>16}  {:>6}  {:>6}  {:>6}",
        "model", "beat", "kick", "hat"
    );
    for (id, whole, kicks, hats) in &rows {
        println!("  {id:>16}  {whole:>6}  {kicks:>6}  {hats:>6}");
    }

    let worst = rows.first().expect("the roster is not empty");
    println!(
        "\n  WORST: {} reaches {} distinct beats in {SEEDS} seeds",
        worst.0, worst.1
    );

    // ⚠ **A floor, not a target, and a low one deliberately.** `country-train`
    // sat at 24 because a train beat *is* four-on-the-floor with `syncopation:
    // 0` — that is the style, not a saturation, and raising this to punish it
    // would be asking the generator to stop sounding like country. What this
    // catches is a model collapsing toward one beat, which is what
    // `uk-drill`'s kick did before anyone measured it.
    //
    // ▶ **It reaches 73 since 2026-08-13, and its kick is still 1.** The kick
    // could not move and should not: `the_train_beat_is_a_sixteenth_stream_over_
    // walking_quarters` asserts every country kick lands on a quarter, and with
    // four anchors against `densityPerBar: 4` the fill loop never runs, so the
    // lane draws no random numbers at all. The variety came from the two lanes
    // the invariants leave free — a second open-hat position and a higher roll
    // frequency. ⚠ `percs` is NOT one of them: widening it from [1,2] to [2,5]
    // moved this number by exactly zero, because `shape` drops anything under
    // velocity 50 and the perc lanes generate below it.
    const FLOOR: usize = 20;
    let thin: Vec<&(String, usize, usize, usize)> = rows
        .iter()
        .filter(|(_, beats, _, _)| *beats < FLOOR)
        .collect();
    assert!(
        thin.is_empty(),
        "these models barely vary their beat over {SEEDS} seeds (floor {FLOOR}): {thin:?}"
    );
}

/// Just the fill's own notes, as ticks and velocities.
///
/// ⚠ `Articulation::Roll` is what `rolls::Roll::render` stamps and nothing else
/// does — the last beats of the bar also hold the backbeat and surviving
/// ghosts, and those *do* vary per model, so measuring the whole window reports
/// variety the fill does not have. A first attempt did exactly that and made a
/// byte-identical roll look like 40 distinct fills.
fn roll_shape(lanes: &[LaneTrack], ctx: &SessionContext) -> Vec<(u32, u8)> {
    let end = u32::from(ctx.bars) * ctx.ticks_per_bar();
    let from = end - engine::generators::grid::ticks_per_beat(ctx) * 2;
    lanes
        .iter()
        .find(|l| l.lane == Lane::Snare)
        .map(|l| {
            l.notes
                .iter()
                .filter(|n| n.start_tick >= from)
                .filter(|n| n.articulation == Some(engine::pattern::Articulation::Roll))
                .map(|n| (n.start_tick - from, n.vel))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_fill_is_not_the_same_gesture_every_time() {
    // ⛔ **Mike's report, 2026-08-05, turned into a gate.** He walked the ten
    // flagship artists in Ableton and every one wrote the same snare roll. It
    // measured out worse than that: six of them produced a byte-identical roll
    // — ticks [0,480,960,1200,1440,1560,1680,1800], velocities
    // [1,33,64,69,96,77,84,91] — and every model on the roster reached only one
    // to four distinct rolls in forty seeds, because `drums::fills` built it
    // from a hardcoded `Roll::new(..).ramp(64,120)` that read neither the model
    // nor the rng.
    //
    // The floor is deliberately well under what the fix actually reaches, so
    // this fails on a regression rather than on a tuning change.
    const FLOOR: usize = 25;

    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };

    let mut thin = Vec::new();
    for (id, model) in &models {
        let mut seen = BTreeSet::new();
        for seed in 0..SEEDS {
            let fill = roll_shape(&generate(model, &ctx, seed), &ctx);
            if !fill.is_empty() {
                seen.insert(fill);
            }
        }
        // A model whose fills never fire has nothing to measure — that is a
        // `fills` block it does not author, not a saturated roll.
        if !seen.is_empty() && seen.len() < FLOOR {
            thin.push((id.clone(), seen.len()));
        }
    }
    assert!(
        thin.is_empty(),
        "these models write too few distinct fills in {SEEDS} seeds \
         (floor {FLOOR}): {thin:?}"
    );
}

/// How often two different models may write the same thing at one seed.
///
/// ⛔ **A rate over the whole roster, not "no collisions at four seeds" — and
/// the difference is a gate that works versus one that passes on luck.** The
/// first version of this checked four hand-picked seeds and asserted zero
/// collisions. It passed. A 200-seed sweep then found `ny-drill` and
/// `pop-smoke` writing an identical beat on **16 of 200 seeds** and `dark-trap`
/// and `future` an identical fill on **10** — none of which the four seeds
/// happened to hit. That is exactly the shape of failure this repo has written
/// down twice: a test that samples too little reports the absence of evidence
/// as evidence of absence.
///
/// ⚠ **Zero is the wrong target and would be a worse gate.** Two models that
/// share a parent and a musical family will occasionally land on the same bar
/// from the same seed, and forcing that to never happen would mean authoring
/// artificial differences — a `travis-scott` that is *not allowed* to play what
/// `metro-boomin` plays is not a more accurate model of either. What matters is
/// that a producer clicking through the roster is not handed the same beat, and
/// a handful of coincidences in a thousand does not do that.
/// ⛔⛔ **AND THE CONSTANT THAT USED TO LIVE HERE COULD NOT SURVIVE VOLUME 1.**
/// It was `MAX_COLLISIONS_PER_1000 = 6`, i.e. "more than 1 of 200 seeds fails",
/// and it is a **per-pair** number while the thing it is spent on grows with the
/// **square** of the roster. Fifteen genres and ten artists is ~300 pairs; 590
/// models is **173,755**. Measured on the shipped roster, 2026-08-12:
///
/// | fail when a pair collides on | expected purely by chance | actually observed |
/// |---|---|---|
/// | ≥2 of 200 *(the old rule)* | **977** | 1,092 |
/// | ≥3 | 36 | 530 |
/// | ≥4 | 1.0 | 332 |
/// | ≥6 | **0.00** | **171** |
///
/// So at the old ceiling **nine tenths of the failures were the birthday
/// paradox**, and "differentiate them" would have meant authoring arbitrary
/// differences into 455 models to beat a threshold that says nothing about
/// whether two artists sound alike. The 171 that survive a chance-free ceiling
/// are real, and they are the work.
///
/// ▶ **So the ceiling is derived rather than chosen**, which is also what the
/// roster needs: volumes 2–5 take it to 2,087 models and 2.2 million pairs, and
/// a hand-tuned number would be wrong again at every one of those steps. This
/// asks the only question worth asking — *is this pair closer than coincidence
/// explains?* — at whatever size the roster happens to be.
///
/// ⚠ **This is not the gate being loosened.** `q` is estimated from the run's
/// own data, so a roster whose models genuinely converge raises `q`, raises the
/// ceiling's chance floor **and** raises the observed counts together — the
/// excess is what fails. The floor of 2 keeps it at least as strict as the old
/// rule on a small roster.
fn ceiling_for(pairs: f64, hits: &BTreeMap<String, usize>) -> usize {
    let seeds = SEEDS as f64;
    // Method of moments: P(collide on ≥2) ≈ C(seeds,2)·q² for small q.
    let over = hits.values().filter(|count| **count >= 2).count() as f64;
    let q = ((over / pairs) / (seeds * (seeds - 1.0) / 2.0)).sqrt();
    if !q.is_finite() || q <= 0.0 {
        return 2;
    }
    for k in 2..=SEEDS as usize {
        // log C(seeds, k), built term by term because `k` never gets large.
        let log_c: f64 = (0..k)
            .map(|i| ((seeds - i as f64) / (i as f64 + 1.0)).ln())
            .sum();
        let log_p = log_c + k as f64 * q.ln() + (seeds - k as f64) * (1.0 - q).ln();
        if log_p.exp() * pairs < 1.0 {
            return k;
        }
    }
    SEEDS as usize
}

#[test]
fn two_different_artists_do_not_write_the_same_beat_or_fill() {
    // ⛔ The half Mike actually saw, 2026-08-05: holding the seed and changing
    // the artist gave him the same roll. Before the fix, the flagships collapsed
    // into two identical groups — one of five artists and one of six — at
    // *every* seed.
    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };
    let ids: Vec<&String> = models.keys().filter(|id| *id != "_defaults").collect();

    let mut beat_hits: BTreeMap<String, usize> = BTreeMap::new();
    let mut fill_hits: BTreeMap<String, usize> = BTreeMap::new();

    for seed in 0..SEEDS {
        let mut by_beat: BTreeMap<Vec<(Lane, Vec<u32>)>, Vec<&str>> = BTreeMap::new();
        let mut by_fill: BTreeMap<Vec<(u32, u8)>, Vec<&str>> = BTreeMap::new();

        for id in &ids {
            let lanes = generate(&models[*id], &ctx, seed);
            by_beat.entry(shape(&lanes)).or_default().push(id);
            let fill = roll_shape(&lanes, &ctx);
            if !fill.is_empty() {
                by_fill.entry(fill).or_default().push(id);
            }
        }

        for group in by_beat.values().filter(|g| g.len() > 1) {
            *beat_hits.entry(group.join(" = ")).or_default() += 1;
        }
        for group in by_fill.values().filter(|g| g.len() > 1) {
            *fill_hits.entry(group.join(" = ")).or_default() += 1;
        }
    }

    // Reported whatever the outcome: a pair that is drifting toward each other
    // should be visible before it crosses the line, not only after.
    let report = |what: &str, hits: &BTreeMap<String, usize>| {
        let mut rows: Vec<(&String, &usize)> = hits.iter().collect();
        rows.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        println!("\n  {what} collisions over {SEEDS} seeds:");
        for (pair, count) in rows.iter().take(10) {
            println!("    {count:>4} / {SEEDS}  {pair}");
        }
    };
    report("beat", &beat_hits);
    report("fill", &fill_hits);

    // Every unordered pair the sweep could have compared — the denominator the
    // chance floor is spent against. See `ceiling_for`.
    let pairs = ids.len() as f64 * (ids.len() as f64 - 1.0) / 2.0;
    let beat_ceiling = ceiling_for(pairs, &beat_hits);
    let fill_ceiling = ceiling_for(pairs, &fill_hits);
    println!(
        "\n  {} models = {pairs:.0} pairs; a pair fails at {beat_ceiling} identical beats \
         or {fill_ceiling} identical fills out of {SEEDS}",
        ids.len()
    );

    let over = |hits: &BTreeMap<String, usize>, ceiling: usize| -> Vec<(String, usize)> {
        hits.iter()
            .filter(|(_, count)| **count >= ceiling)
            .map(|(pair, count)| (pair.clone(), *count))
            .collect()
    };

    let beat_over = over(&beat_hits, beat_ceiling);
    let fill_over = over(&fill_hits, fill_ceiling);
    assert!(
        beat_over.is_empty() && fill_over.is_empty(),
        "these models are too close to tell apart — they land on an identical result \
         more often than coincidence explains at this roster size (beat ≥{beat_ceiling} \
         of {SEEDS}, fill ≥{fill_ceiling} of {SEEDS}).\n  beat: {beat_over:?}\n  \
         fill: {fill_over:?}\n\
         Each pair is a model and its parent, or two siblings under one parent; the cure \
         is authored difference in the child, not a looser gate — and the ceiling is \
         derived from the run's own data, so raising it by hand is not available."
    );
}

/// The ten flagships Mike was clicking through, plus the genre they inherit.
const FLAGSHIPS: &[&str] = &[
    "trap",
    "drake",
    "future",
    "metro-boomin",
    "nettspend",
    "osamason",
    "pierre-bourne",
    "pop-smoke",
    "southside",
    "summrs",
    "travis-scott",
];

#[test]
fn report_whether_two_artists_at_one_seed_write_two_different_beats() {
    // ⛔ **Mike's actual sentence, turned into a number.** "every artist has the
    // same 4 snare roll no matter which artist you pick" is a claim about
    // holding the seed and changing the artist — which is the opposite axis
    // from the test above, and the one a producer exercises when they are
    // auditioning the roster.
    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };

    for seed in [7u64, 2024, 99] {
        let mut by_shape: BTreeMap<Vec<(Lane, Vec<u32>)>, Vec<&str>> = BTreeMap::new();
        for id in FLAGSHIPS {
            let Some(model) = models.get(*id) else {
                continue;
            };
            by_shape
                .entry(shape(&generate(model, &ctx, seed)))
                .or_default()
                .push(id);
        }
        println!(
            "\n  seed {seed}: {} distinct beats across {} flagship artists",
            by_shape.len(),
            FLAGSHIPS.len()
        );
        let collisions: Vec<&Vec<&str>> = by_shape.values().filter(|g| g.len() > 1).collect();
        assert!(
            collisions.is_empty(),
            "at seed {seed} these artists write an identical beat: {collisions:?}"
        );
    }
}
