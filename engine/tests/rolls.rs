//! The roll vocabulary against the genres that ship.
//!
//! FR-003's requirement is not "rolls exist" but that every archetype reaches
//! at least two of them, so these run across seeds and look at what the
//! generator actually produced rather than at what the model asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::generators::grid;
use engine::generators::rolls::RollPosition;
use engine::pattern::{Articulation, Lane, LaneTrack, Note};
use engine::StyleModel;
use serde_json::Value;

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

fn ctx(bars: u16) -> SessionContext {
    SessionContext {
        bars,
        ..Default::default()
    }
}

fn hats(lanes: &[LaneTrack]) -> Vec<Note> {
    lanes
        .iter()
        .find(|l| l.lane == Lane::ClosedHat)
        .map(|l| l.notes.clone())
        .unwrap_or_default()
}

/// The subdivisions the roll notes in this pattern were written at.
///
/// Read off the output rather than the model: the gap between consecutive notes
/// of a roll *is* its subdivision, so this measures what a listener would hear.
///
/// Grouped into runs, taking the **minimum** gap in each. A roll with a hole
/// cut in it — trap authors `insertGaps` — leaves two notes twice the
/// subdivision apart, and counting that gap as a subdivision of its own
/// reported an 8th-note triplet the model never authored.
fn roll_subdivisions(notes: &[Note]) -> BTreeSet<u32> {
    let rolls: Vec<&Note> = notes
        .iter()
        .filter(|n| n.articulation == Some(Articulation::Roll))
        .collect();

    let beat = grid::ticks_per_beat(&SessionContext::default());
    let mut found = BTreeSet::new();
    let mut run: Option<u32> = None;

    for pair in rolls.windows(2) {
        let gap = pair[1].start_tick.saturating_sub(pair[0].start_tick);
        // A gap of a beat or more is the space between two separate rolls,
        // not a subdivision inside one.
        if gap == 0 || gap >= beat {
            found.extend(run.take());
            continue;
        }
        run = Some(run.map_or(gap, |smallest: u32| smallest.min(gap)));
    }
    found.extend(run);
    found
}

#[test]
fn every_roll_is_written_at_a_subdivision_its_model_authored() {
    // The claim that is both true and able to fail. The original — "a roll is
    // finer than a 16th" — restated the helper's own filter, so writing every
    // roll four times coarser than authored left it green.
    //
    // Comparing against the densest moment of the surrounding stream was too
    // strict in the other direction: `_defaults` fills 16ths into an 8th base,
    // so two stream notes can already sit a 16th apart. What must hold is that
    // the engine only ever writes a subdivision the model asked for.
    let mut checked = 0;

    for (id, model) in shipped() {
        let Some(rolls) = model
            .blocks
            .get("drums")
            .and_then(|d| d.pointer("/hihat/rolls"))
        else {
            continue;
        };

        let authored: BTreeSet<u32> = rolls
            .get("vocab")
            .and_then(|v| v.get("values").or(Some(v)))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(grid::note_value_ticks)
                    .collect()
            })
            .unwrap_or_default();
        assert!(!authored.is_empty(), "{id}: an empty roll vocabulary");

        for seed in 0..SEEDS {
            let notes = hats(&generate(&model, &ctx(4), seed));
            for subdivision in roll_subdivisions(&notes) {
                assert!(
                    authored.contains(&subdivision),
                    "{id} seed {seed}: rolled at {subdivision} ticks, which is not in {authored:?}"
                );
                checked += 1;
            }
        }
    }

    assert!(checked > 0, "no roll subdivision was checked");
}

#[test]
fn roll_notes_are_marked_so_the_grid_and_the_sampler_can_see_them() {
    // The drum grid renders roll cells subdivided (US-006) and the humanizer
    // leaves their velocities to the ramp. Both read the articulation.
    let models = shipped();
    let trap = models.get("trap").expect("trap must ship");

    let mut rolled = 0;
    for seed in 0..SEEDS {
        for note in hats(&generate(trap, &ctx(4), seed)) {
            if note.articulation == Some(Articulation::Roll) {
                rolled += 1;
                assert!(note.vel >= 1 && note.vel <= 127);
                assert!(note.len_ticks > 0);
            }
        }
    }
    assert!(
        rolled > 0,
        "trap authors rolls at 0.8 a bar and produced none"
    );
}

#[test]
fn a_roll_never_doubles_the_stream_it_replaced() {
    // The window belongs to the roll. If the base hats survived underneath,
    // every roll would trigger two samples on the same tick.
    let mut checked = 0;
    for (id, model) in shipped() {
        for seed in 0..SEEDS {
            let notes = hats(&generate(&model, &ctx(4), seed));
            checked += notes.len();
            let mut seen = BTreeSet::new();
            for note in &notes {
                assert!(
                    seen.insert(note.start_tick),
                    "{id} seed {seed}: two closed hats on tick {}",
                    note.start_tick
                );
            }
        }
    }
    assert!(checked > 0, "no hat notes were examined at all");
}

#[test]
fn rolls_stay_inside_the_pattern() {
    let context = ctx(4);
    let total = context.total_ticks();
    let mut checked = 0;
    for (id, model) in shipped() {
        for seed in 0..SEEDS {
            for note in hats(&generate(&model, &context, seed)) {
                checked += 1;
                assert!(
                    note.start_tick < total,
                    "{id} seed {seed}: a roll ran past the pattern at {}",
                    note.start_tick
                );
            }
        }
    }
    assert!(checked > 0, "no hat notes were examined at all");
}

#[test]
fn rage_rolls_in_bursts_rather_than_filling_the_beat() {
    // "Bursts/short rolls, not continuous streams" (research ch. 1 §3), which
    // the model spells `burstOnly: true`.
    let models = shipped();
    let rage = models.get("rage").expect("rage must ship");
    let trap = models.get("trap").expect("trap must ship");

    // A run is notes that are *contiguous in time*, not merely next to each
    // other in the list. Rage's stream is sparse, so two bursts three beats
    // apart sit side by side in the note list with nothing between them —
    // counting those as one six-note roll measured the gaps, not the bursts.
    let longest_run = |model: &StyleModel| {
        let mut longest = 0;
        for seed in 0..SEEDS {
            let rolls: Vec<Note> = hats(&generate(model, &ctx(4), seed))
                .into_iter()
                .filter(|n| n.articulation == Some(Articulation::Roll))
                .collect();

            let mut run = if rolls.is_empty() { 0 } else { 1 };
            longest = longest.max(run);
            for pair in rolls.windows(2) {
                let gap = pair[1].start_tick.saturating_sub(pair[0].start_tick);
                run = if gap > 0 && gap <= grid::SIXTEENTH {
                    run + 1
                } else {
                    1
                };
                longest = longest.max(run);
            }
        }
        longest
    };

    let rage_run = longest_run(rage);
    let trap_run = longest_run(trap);
    assert!(rage_run <= 3, "rage bursts should be short, got {rage_run}");
    assert!(
        trap_run > rage_run,
        "trap fills the beat ({trap_run}) where rage bursts ({rage_run})"
    );
}

#[test]
fn every_roll_vocabulary_entry_in_the_dataset_is_a_note_value() {
    // The same silent-failure class as the lane names and grid positions: a
    // vocabulary entry the parser rejects is skipped, so the genre quietly
    // loses that roll type while the model still lists it.
    let mut checked = 0;

    for (id, model) in shipped() {
        let Some(rolls) = model
            .blocks
            .get("drums")
            .and_then(|d| d.pointer("/hihat/rolls"))
        else {
            continue;
        };

        // `vocab` is authored either as a plain array or as a weighted spec.
        let values = rolls
            .get("vocab")
            .and_then(|v| v.get("values").or(Some(v)))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(!values.is_empty(), "{id}: an empty roll vocabulary");

        for value in values {
            let text = value.as_str().unwrap_or_default();
            assert!(
                grid::note_value_ticks(text).is_some(),
                "{id}: `{text}` is not a note value, so that roll type never fires"
            );
            checked += 1;
        }

        for position in rolls
            .get("positions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            assert!(
                RollPosition::parse(position).is_some(),
                "{id}: `{position}` is not a roll position the engine knows"
            );
            checked += 1;
        }
    }

    assert!(checked > 0, "no roll vocabulary was checked");
}

#[test]
fn the_snare_roll_ladders_in_the_dataset_are_note_values_too() {
    let mut checked = 0;
    for (id, model) in shipped() {
        let Some(ladder) = model
            .blocks
            .get("drums")
            .and_then(|d| d.pointer("/snareRoll/ladder"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for rung in ladder.iter().filter_map(Value::as_str) {
            assert!(
                grid::note_value_ticks(rung).is_some(),
                "{id}: `{rung}` is not a note value"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no ladder was checked");
}

#[test]
fn rolls_are_reproducible_across_the_whole_pattern() {
    let models = shipped();
    let trap = models.get("trap").unwrap();
    let a = generate(trap, &ctx(8), 31_337);
    let b = generate(trap, &ctx(8), 31_337);
    assert_eq!(a, b);
}
