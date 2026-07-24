//! The hat engine: base subdivision, fill density, open hats, velocity contour.
//!
//! Hats are the busiest lane in the pattern and the one a listener uses to tell
//! two 140-BPM genres apart, so most of these are statistical: run 100 seeds and
//! assert the shape of the distribution rather than one lucky pattern.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::generators::grid;
use engine::pattern::{Articulation, Lane, LaneTrack, Note};
use engine::StyleModel;
use serde_json::{json, Value};

const SEEDS: u64 = 100;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

fn shipped() -> BTreeMap<String, StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

fn shipped_model(id: &str) -> StyleModel {
    shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("`{id}` must ship"))
}

/// A model with nothing but the hat block under test, so a failure points at
/// the parameter rather than at whatever else the genre does.
fn hat_model(hihat: Value) -> StyleModel {
    serde_json::from_value(json!({
        "id": "test", "type": "genre", "name": "Test",
        "drums": { "hihat": hihat },
    }))
    .expect("the test model must parse")
}

fn ctx(bars: u16) -> SessionContext {
    SessionContext {
        bars,
        ..Default::default()
    }
}

fn lane(lanes: &[LaneTrack], want: Lane) -> Option<&LaneTrack> {
    lanes.iter().find(|l| l.lane == want)
}

fn notes(lanes: &[LaneTrack], want: Lane) -> Vec<Note> {
    lane(lanes, want)
        .map(|l| l.notes.clone())
        .unwrap_or_default()
}

#[test]
fn an_eighth_note_base_plays_every_eighth_and_nothing_finer_without_fill() {
    let m = hat_model(json!({ "base": "8th", "fillDensity": 0.0 }));
    for seed in 0..SEEDS {
        let hats = notes(&generate(&m, &ctx(1), seed), Lane::ClosedHat);
        let ticks: Vec<u32> = hats.iter().map(|n| n.start_tick).collect();
        assert_eq!(
            ticks,
            vec![0, 480, 960, 1440, 1920, 2400, 2880, 3360],
            "seed {seed}"
        );
    }
}

#[test]
fn a_sixteenth_base_fills_the_bar() {
    let m = hat_model(json!({ "base": "16th", "fillDensity": 0.0 }));
    let hats = notes(&generate(&m, &ctx(1), 1), Lane::ClosedHat);
    assert_eq!(hats.len(), 16);
    for (i, note) in hats.iter().enumerate() {
        assert_eq!(note.start_tick, i as u32 * grid::SIXTEENTH);
    }
}

#[test]
fn fill_density_adds_sixteenths_between_the_base_hits() {
    // The base is guaranteed; the fill is what makes the stream breathe. Both
    // ends of the dial have to mean something.
    let count = |density: f64| {
        let m = hat_model(json!({ "base": "8th", "fillDensity": density }));
        let total: usize = (0..SEEDS)
            .map(|seed| notes(&generate(&m, &ctx(1), seed), Lane::ClosedHat).len())
            .sum();
        total as f64 / SEEDS as f64
    };

    let empty = count(0.0);
    let half = count(0.5);
    let full = count(1.0);

    assert_eq!(empty, 8.0, "no fill is the bare 8ths");
    assert_eq!(full, 16.0, "a full fill reaches every 16th");
    assert!(
        (11.0..=13.0).contains(&half),
        "half a fill should sit between them, got {half}"
    );
}

#[test]
fn the_base_hits_are_always_played_however_low_the_fill() {
    let m = hat_model(json!({ "base": "8th", "fillDensity": 0.0 }));
    for seed in 0..SEEDS {
        let ticks: Vec<u32> = notes(&generate(&m, &ctx(2), seed), Lane::ClosedHat)
            .iter()
            .map(|n| n.start_tick)
            .collect();
        for eighth in 0..8u32 {
            let tick = eighth * 480;
            assert!(ticks.contains(&tick), "seed {seed}: lost the 8th at {tick}");
        }
    }
}

#[test]
fn beats_and_offbeat_eighths_are_louder_than_the_sixteenths_between_them() {
    // Research ch. 1 §1: mains 80–100%, ghosts 40–60%. The split is positional,
    // so it must hold on every seed rather than on average.
    let m = hat_model(json!({
        "base": "16th",
        "fillDensity": 1.0,
        "velocities": { "main": [0.8, 1.0], "ghost": [0.4, 0.6] }
    }));

    for seed in 0..SEEDS {
        let hats = notes(&generate(&m, &ctx(1), seed), Lane::ClosedHat);

        // Split by **position**, not by the articulation the generator wrote.
        // Partitioning on the label was a test that could not fail: inverting
        // the rule flipped the labels and the velocities together, so "notes
        // marked main are louder than notes marked ghost" stayed true while
        // the accents had moved off the beat entirely.
        let (mains, ghosts): (Vec<&Note>, Vec<&Note>) = hats.iter().partition(|n| {
            let index = n.start_tick / grid::SIXTEENTH;
            grid::is_downbeat(index) || grid::is_offbeat_eighth(index)
        });

        assert_eq!(mains.len(), 8, "seed {seed}: beats and &s");
        assert_eq!(ghosts.len(), 8, "seed {seed}: the e/a 16ths");

        let quietest_main = mains.iter().map(|n| n.vel).min().unwrap();
        let loudest_ghost = ghosts.iter().map(|n| n.vel).max().unwrap();
        assert!(
            quietest_main > loudest_ghost,
            "seed {seed}: the beat should be the loud one — {quietest_main} vs {loudest_ghost}"
        );

        // The bands are the ones the model authored, as fractions of 127...
        assert!(mains.iter().all(|n| (101..=127).contains(&n.vel)));
        assert!(ghosts.iter().all(|n| (50..=77).contains(&n.vel)));
        // ...and the articulation agrees with the position, so the sampler and
        // the grid read the same thing the velocity says.
        assert!(mains.iter().all(|n| n.articulation.is_none()));
        assert!(ghosts
            .iter()
            .all(|n| n.articulation == Some(Articulation::Ghost)));
    }
}

#[test]
fn a_tresillo_base_follows_its_authored_grouping() {
    // Drill's hats sit on 3-3-2 (research ch. 1 §2). The grouping sums to half
    // a bar, so it lands twice: 16ths 0, 3, 6, 8, 11, 14.
    let m = hat_model(json!({
        "base": "tresillo",
        "tresilloGrouping": [3, 3, 2],
        "fillDensity": 0.0
    }));
    let hats = notes(&generate(&m, &ctx(1), 1), Lane::ClosedHat);
    let indices: Vec<u32> = hats
        .iter()
        .map(|n| n.start_tick / grid::SIXTEENTH)
        .collect();
    assert_eq!(indices, vec![0, 3, 6, 8, 11, 14]);
}

#[test]
fn a_degenerate_grouping_falls_back_rather_than_spinning() {
    // A grouping of zeros would never advance the cursor. It must produce the
    // 3-3-2 the name means, not hang the generator.
    let m = hat_model(json!({
        "base": "tresillo",
        "tresilloGrouping": [0, 0],
        "fillDensity": 0.0
    }));
    let hats = notes(&generate(&m, &ctx(1), 1), Lane::ClosedHat);
    assert_eq!(hats.len(), 6);
}

#[test]
fn a_non_continuous_stream_leaves_negative_space() {
    // Rage: "fast base but SPARSE — bursts, not continuous streams". A
    // continuous 16th stream at the same density would be twice as busy, and
    // rage would stop being rage.
    let sparse = hat_model(json!({
        "base": "16th", "continuous": false, "fillDensity": 0.3
    }));
    let dense = hat_model(json!({
        "base": "16th", "continuous": true, "fillDensity": 0.3
    }));

    let average = |m: &StyleModel| {
        let total: usize = (0..SEEDS)
            .map(|seed| notes(&generate(m, &ctx(2), seed), Lane::ClosedHat).len())
            .sum();
        total as f64 / SEEDS as f64
    };

    let sparse_count = average(&sparse);
    let dense_count = average(&dense);
    assert!(
        sparse_count < dense_count / 2.0,
        "sparse {sparse_count} should be far below continuous {dense_count}"
    );
    assert!(sparse_count > 0.0, "sparse is not silent");
}

#[test]
fn open_hats_land_where_the_model_says_and_close_the_hat_underneath() {
    // One hi-hat cannot be open and shut at the same instant. If the closed hit
    // survived, playback would trigger both samples on the same tick.
    let m = hat_model(json!({
        "base": "16th",
        "fillDensity": 0.0,
        "openHat": { "prob": 1.0, "pos": ["2&"], "perBar": 1 }
    }));

    for seed in 0..SEEDS {
        let lanes = generate(&m, &ctx(1), seed);
        let open = notes(&lanes, Lane::OpenHat);
        let closed = notes(&lanes, Lane::ClosedHat);

        assert_eq!(open.len(), 1, "seed {seed}");
        assert_eq!(open[0].start_tick, 1440);
        assert!(
            !closed.iter().any(|n| n.start_tick == 1440),
            "seed {seed}: the closed hat under the open one survived"
        );
    }
}

#[test]
fn an_open_hat_probability_of_zero_opens_none() {
    let m = hat_model(json!({
        "base": "8th",
        "openHat": { "prob": 0.0, "pos": ["2&", "4&"], "perBar": [1, 2] }
    }));
    for seed in 0..SEEDS {
        assert!(
            notes(&generate(&m, &ctx(2), seed), Lane::OpenHat).is_empty(),
            "seed {seed}"
        );
    }
}

#[test]
fn a_pre_downbeat_open_hat_lands_just_before_the_beat() {
    // Rage's `"3_pre"`: one 16th before beat 3.
    let m = hat_model(json!({
        "base": "16th",
        "fillDensity": 0.0,
        "openHat": { "prob": 1.0, "pos": ["3_pre"], "perBar": 1 }
    }));
    let open = notes(&generate(&m, &ctx(1), 5), Lane::OpenHat);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].start_tick, 1920 - grid::SIXTEENTH);
}

#[test]
fn a_pre_downbeat_open_hat_on_bar_one_is_dropped_not_wrapped() {
    // `"1_pre"` is in the *previous* bar. In the first bar there is none, and
    // wrapping it would put an accent at the very end of the pattern — the one
    // place a lead-in must not be.
    let m = hat_model(json!({
        "base": "16th",
        "fillDensity": 0.0,
        "openHat": { "prob": 1.0, "pos": ["1_pre"], "perBar": 1 }
    }));

    let one_bar = notes(&generate(&m, &ctx(1), 5), Lane::OpenHat);
    assert!(one_bar.is_empty(), "nothing precedes the first bar");

    let two_bars = notes(&generate(&m, &ctx(2), 5), Lane::OpenHat);
    assert_eq!(two_bars.len(), 1, "the second bar has one");
    assert_eq!(two_bars[0].start_tick, 3840 - grid::SIXTEENTH);
}

#[test]
fn the_pitch_bent_layer_only_appears_when_the_model_asks_for_it() {
    // The flag rides on `Note.pitch`, which our sampler reads and the SMF
    // writer replaces with the lane's GM voice — GM has one closed hat, so a
    // repitched layer is a detail of playback, not of the exported file.
    let plain = hat_model(json!({ "base": "16th", "pitchBendProb": 0.0 }));
    let bent = hat_model(json!({ "base": "16th", "pitchBendProb": 1.0 }));
    let gm_closed_hat = 42;

    for seed in 0..SEEDS {
        assert!(
            notes(&generate(&plain, &ctx(2), seed), Lane::ClosedHat)
                .iter()
                .all(|n| n.pitch == gm_closed_hat),
            "seed {seed}: an unasked-for bend"
        );
    }

    // Per seed, not summed: at `pitchBendProb: 1.0` the bend is certain, and a
    // total over 100 seeds was satisfied by a 3% chance — the stated certainty
    // could be reduced thirtyfold and still pass.
    for seed in 0..SEEDS {
        assert!(
            notes(&generate(&bent, &ctx(2), seed), Lane::ClosedHat)
                .iter()
                .any(|n| n.pitch != gm_closed_hat),
            "seed {seed}: a certain bend did not happen"
        );
    }
}

#[test]
fn the_swell_rises_across_the_loop() {
    // "Optional hat swell — a gradual velocity rise across the loop." It scales
    // what is there rather than overwriting it, so the main/ghost contour has
    // to survive the gesture.
    let m = hat_model(json!({
        "base": "8th",
        "fillDensity": 0.0,
        "velocities": { "main": [1.0, 1.0], "ghost": [0.5, 0.5] },
        "swellProb": 1.0
    }));
    let hats = notes(&generate(&m, &ctx(4), 3), Lane::ClosedHat);

    let velocities: Vec<u8> = hats.iter().map(|n| n.vel).collect();
    let first = *velocities.first().unwrap();
    let last = *velocities.last().unwrap();
    assert!(last > first, "the swell should rise: {first} -> {last}");

    // ...rising the whole way, not flat until the last note. Nothing
    // constrained the middle before, so a swell that did nothing for 95% of
    // the loop and then stepped passed as a gradual rise.
    for pair in velocities.windows(2) {
        assert!(pair[1] >= pair[0], "the swell dipped: {velocities:?}");
    }
    let midpoint = velocities[velocities.len() / 2];
    assert!(
        midpoint > first && midpoint < last,
        "the swell is a step, not a rise: {velocities:?}"
    );
}

#[test]
fn hats_are_reproducible_and_independent_of_the_other_lanes() {
    let m = hat_model(json!({
        "base": "16th", "fillDensity": 0.5,
        "openHat": { "prob": 0.5, "pos": ["2&", "4&"], "perBar": [1, 2] }
    }));
    let a = notes(&generate(&m, &ctx(4), 77), Lane::ClosedHat);
    let b = notes(&generate(&m, &ctx(4), 77), Lane::ClosedHat);
    let c = notes(&generate(&m, &ctx(4), 78), Lane::ClosedHat);
    assert_eq!(a, b);
    assert_ne!(a, c);

    // A different snare grammar must not move a single hat.
    let with_snare: StyleModel = serde_json::from_value(json!({
        "id": "test", "type": "genre", "name": "Test",
        "drums": {
            "hihat": { "base": "16th", "fillDensity": 0.5,
                       "openHat": { "prob": 0.5, "pos": ["2&", "4&"], "perBar": [1, 2] } },
            "snare": { "placement": "train_16ths" }
        }
    }))
    .unwrap();
    assert_eq!(
        a,
        notes(&generate(&with_snare, &ctx(4), 77), Lane::ClosedHat)
    );
}

#[test]
fn every_shipped_genre_produces_hats_that_stay_inside_the_pattern() {
    for (id, model) in shipped() {
        if model
            .blocks
            .get("drums")
            .and_then(|d| d.get("hihat"))
            .is_none()
        {
            continue;
        }
        let context = ctx(4);
        let total = context.total_ticks();

        for seed in 0..SEEDS {
            let lanes = generate(&model, &context, seed);
            // A model that declares a hat block must produce hats. Without
            // this, deleting the hi-hat lane outright left the test named
            // "produces hats" green — it only ever asserted *inside* the loop.
            assert!(
                !notes(&lanes, Lane::ClosedHat).is_empty(),
                "{id} seed {seed}: declares a hihat block and produced no hats"
            );
            for want in [Lane::ClosedHat, Lane::OpenHat] {
                for note in notes(&lanes, want) {
                    assert!(
                        note.start_tick < total,
                        "{id} seed {seed}: {want:?} at {} is past the pattern",
                        note.start_tick
                    );
                    assert!(note.vel >= 1 && note.vel <= 127, "{id}: bad velocity");
                }
            }
        }
    }
}

#[test]
fn trap_and_rage_hats_are_the_opposite_densities_the_research_describes() {
    // Trap's interest comes from a filled stream; rage's from the space where
    // one is not. If these ever converge, both genres are wrong.
    let context = ctx(4);
    let average = |model: &StyleModel| {
        let total: usize = (0..SEEDS)
            .map(|seed| notes(&generate(model, &context, seed), Lane::ClosedHat).len())
            .sum();
        total as f64 / SEEDS as f64
    };

    let trap = average(&shipped_model("trap"));
    let rage = average(&shipped_model("rage"));
    assert!(
        trap > rage * 1.5,
        "trap's hats ({trap:.1}) should be far busier than rage's ({rage:.1})"
    );
}

#[test]
fn drill_hats_sit_on_the_tresillo_the_model_authors() {
    let drill = shipped_model("uk-drill");
    let context = ctx(2);
    let hats = notes(&generate(&drill, &context, 11), Lane::ClosedHat);

    // With `fillDensity` the stream is not only the skeleton, but the skeleton
    // must always be in there.
    let ticks: Vec<u32> = hats.iter().map(|n| n.start_tick).collect();
    for index in [0, 3, 6, 8, 11, 14] {
        let tick = index * grid::SIXTEENTH;
        assert!(ticks.contains(&tick), "drill lost the 3-3-2 hit at {tick}");
    }
}
