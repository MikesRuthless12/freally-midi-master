//! Fills and pattern length.
//!
//! Consensus formula #20: variation events land at phrase boundaries and the
//! densest bars are the ones that close a phrase. That is checkable as a
//! histogram — which bars get a fill, over many seeds — and a histogram is
//! what this file mostly is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::generators::grid;
use engine::pattern::{Articulation, Lane, LaneTrack, Note};
use engine::StyleModel;
use serde_json::{json, Value};

const SEEDS: u64 = 60;

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

fn model(drums: Value) -> StyleModel {
    serde_json::from_value(json!({
        "id": "test", "type": "genre", "name": "Test", "drums": drums,
    }))
    .expect("the test model must parse")
}

fn ctx(bars: u16) -> SessionContext {
    SessionContext {
        bars,
        ..Default::default()
    }
}

fn notes(lanes: &[LaneTrack], want: Lane) -> Vec<Note> {
    lanes
        .iter()
        .find(|l| l.lane == want)
        .map(|l| l.notes.clone())
        .unwrap_or_default()
}

/// How many fill notes land in each bar, summed over seeds.
///
/// Counts the snare *and* the clap: west-coast club turns its fills over on the
/// clap rather than the snare, and a histogram that only looked at one lane
/// reported that genre as having no fills at all.
fn histogram(model: &StyleModel, bars: u16) -> Vec<usize> {
    let context = ctx(bars);
    let bar_ticks = context.ticks_per_bar();
    let mut counts = vec![0; usize::from(bars)];

    for seed in 0..SEEDS {
        let lanes = generate(model, &context, seed);
        for lane in [Lane::Snare, Lane::Clap] {
            for note in notes(&lanes, lane) {
                if note.articulation == Some(Articulation::Roll) {
                    counts[(note.start_tick / bar_ticks) as usize] += 1;
                }
            }
        }
    }
    counts
}

#[test]
fn a_small_fill_lands_every_second_bar_and_nowhere_else() {
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    for (bar, count) in counts.iter().enumerate() {
        let position = bar + 1;
        if position % 2 == 0 {
            assert!(*count > 0, "bar {position} should carry a fill");
        } else {
            assert_eq!(*count, 0, "bar {position} should be plain");
        }
    }
}

#[test]
fn the_eighth_bar_gets_the_big_one() {
    // The phrase-boundary shape: every second bar varies, and the bar that
    // closes the eight-bar phrase is the densest of them.
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    let biggest = counts.iter().max().unwrap();
    assert_eq!(
        counts[7], *biggest,
        "bar 8 should be the densest: {counts:?}"
    );
    assert!(
        counts[7] > counts[1] * 3 / 2,
        "the big fill should be clearly bigger than a small one: {counts:?}"
    );
}

#[test]
fn a_different_cycle_moves_the_fills() {
    // Rage fills every four bars rather than every two, and the difference has
    // to show in the output or the parameter is decoration.
    let m = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 4, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    for (bar, count) in counts.iter().enumerate() {
        let position = bar + 1;
        if position % 4 == 0 {
            assert!(*count > 0, "bar {position} should carry a fill: {counts:?}");
        } else {
            assert_eq!(*count, 0, "bar {position} should be plain: {counts:?}");
        }
    }
}

#[test]
fn the_last_bar_fills_so_the_loop_leads_somewhere() {
    // `fillBeforeSection` — a pattern should end *into* whatever comes next
    // rather than stopping dead at the loop point. Three bars is the test:
    // bar 3 is not on the two-bar cycle, so only the flag can fill it.
    let with_flag = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": true }
    }));
    let without = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));

    assert!(
        histogram(&with_flag, 3)[2] > 0,
        "the flag should fill bar 3"
    );
    assert_eq!(histogram(&without, 3)[2], 0, "without it, bar 3 is plain");
}

#[test]
fn two_four_and_eight_bar_patterns_all_end_on_a_fill() {
    // The three lengths the UI offers (FR-003). Whatever the length, the last
    // bar leads out of it.
    let m = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": true }
    }));

    for bars in [2u16, 4, 8] {
        let counts = histogram(&m, bars);
        assert!(
            *counts.last().unwrap() > 0,
            "a {bars}-bar pattern should fill its last bar: {counts:?}"
        );
        // ...and every note still belongs to the pattern.
        let context = ctx(bars);
        for seed in 0..SEEDS {
            for track in generate(&m, &context, seed) {
                for note in &track.notes {
                    assert!(
                        note.start_tick < context.total_ticks(),
                        "{bars} bars, seed {seed}: a fill ran past the end"
                    );
                }
            }
        }
    }
}

#[test]
fn a_fill_takes_the_end_of_its_bar_and_leaves_the_backbeat_alone() {
    // A fill is a run-up, not a replacement for the bar. The backbeat that the
    // fill is leading away from has to survive it.
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 1, "bigEveryBars": 99, "fillBeforeSection": false }
    }));
    let context = ctx(4);
    let beat = grid::ticks_per_beat(&context);

    for seed in 0..SEEDS {
        let snares = notes(&generate(&m, &context, seed), Lane::Snare);
        for bar in 0..4u32 {
            let bar_start = bar * context.ticks_per_bar();
            let backbeat = snares.iter().any(|n| {
                n.start_tick == bar_start + beat && n.articulation != Some(Articulation::Roll)
            });
            assert!(
                backbeat,
                "seed {seed}, bar {bar}: the 2 was eaten by the fill"
            );
        }
    }
}

#[test]
fn a_fill_keeps_the_ghost_it_plays_over() {
    // Drill's and-of-4 ghost answers the backbeat and lives in exactly the beat
    // a fill lands on. Clearing it cost the genre half of them.
    let m = model(json!({
        "snare": {
            "placement": "halftime_3",
            "ghost": { "prob": 1.0, "pos": ["4&"], "vel": [0.45, 0.45] }
        },
        "fills": { "smallEveryBars": 1, "bigEveryBars": 99, "fillBeforeSection": false }
    }));
    let context = ctx(4);

    for seed in 0..SEEDS {
        let ghosts = notes(&generate(&m, &context, seed), Lane::Snare)
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Ghost))
            .count();
        assert_eq!(ghosts, 4, "seed {seed}: a fill ate a ghost");
    }
}

#[test]
fn a_model_with_no_fills_block_gets_none() {
    let m = model(json!({ "snare": { "placement": "backbeat_24" } }));
    for seed in 0..SEEDS {
        assert!(
            notes(&generate(&m, &ctx(8), seed), Lane::Snare)
                .iter()
                .all(|n| n.articulation != Some(Articulation::Roll)),
            "seed {seed}: an unasked-for fill"
        );
    }
}

#[test]
fn the_ladder_flag_decides_what_the_big_fill_is_made_of() {
    // Trap asks for the subdivision ladder; drill does not. The two must not
    // produce the same fill, or the flag means nothing.
    let ladder = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 8, "bigEveryBars": 8, "snareRollLadder": true },
        "snareRoll": { "ladder": ["4", "8", "16", "32"], "velocityRampRange": [1, 127] }
    }));
    let plain = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 8, "bigEveryBars": 8, "snareRollLadder": false }
    }));

    let context = ctx(8);
    let last_bar = |m: &StyleModel, seed: u64| -> Vec<u32> {
        let start = 7 * context.ticks_per_bar();
        notes(&generate(m, &context, seed), Lane::Snare)
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Roll) && n.start_tick >= start)
            .map(|n| n.start_tick - start)
            .collect()
    };

    // The ladder accelerates: its gaps get smaller. A plain roll does not.
    let rungs = last_bar(&ladder, 1);
    let flat = last_bar(&plain, 1);
    assert!(!rungs.is_empty() && !flat.is_empty());

    let gaps = |ticks: &[u32]| -> Vec<u32> { ticks.windows(2).map(|p| p[1] - p[0]).collect() };
    let ladder_gaps = gaps(&rungs);
    let flat_gaps = gaps(&flat);

    assert!(
        ladder_gaps.first() > ladder_gaps.last(),
        "the ladder should accelerate: {ladder_gaps:?}"
    );
    assert!(
        flat_gaps.iter().all(|g| *g == flat_gaps[0]),
        "a plain fill is one subdivision throughout: {flat_gaps:?}"
    );
}

#[test]
fn every_shipped_genre_fills_at_the_cycle_it_authors() {
    for (id, model) in shipped() {
        let Some(fills) = model.blocks.get("drums").and_then(|d| d.get("fills")) else {
            continue;
        };
        let small = fills
            .get("smallEveryBars")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize;

        let counts = histogram(&model, 8);
        for (bar, count) in counts.iter().enumerate() {
            let position = bar + 1;
            // The last bar always fills — `fillBeforeSection` defaults on.
            if position % small == 0 || position == 8 {
                assert!(*count > 0, "{id}: bar {position} should fill: {counts:?}");
            }
        }
    }
}
