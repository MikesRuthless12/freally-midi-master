//! The bassline generator against the data that ships (TASK-037, FR-007).
//!
//! Same four questions as `melody.rs` and `counter.rs`, asked through `common`
//! so the parts cannot drift into different standards: is every authored
//! parameter read, is every authored string one a reader knows, does the part
//! stay in its register, and how many different basslines can an artist reach.
//!
//! Plus the one this part has of its own: **a model whose 808 already plays the
//! bassline must not generate a second one.**

use std::collections::BTreeSet;

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::{bass, chords, drums};
use engine::pattern::LaneTrack;
use engine::theory;
use serde_json::Value;

mod common;
use common::{
    authored_blocks, covers, leaf_paths, models_authoring, shape, variety_seeds, MIN_DISTINCT,
};

fn bass_models() -> Vec<(String, engine::StyleModel, Value)> {
    models_authoring("bassline")
}

/// Models that actually generate a bass — the 808 carries it for the rest.
fn generating_models() -> Vec<(String, engine::StyleModel, Value)> {
    bass_models()
        .into_iter()
        .filter(|(_, model, _)| !bass::eight_o_eight_is_the_bass(model))
        .collect()
}

fn bass_for(model: &engine::StyleModel, seed: u64) -> LaneTrack {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    bass::generate(model, &ctx, seed, &harmony, &kit)
}

/// Parameters under `bassline` that no generator reads, with the reason.
///
/// Empty: every authored parameter is currently read. Kept rather than deleted
/// so the place to record an exception already exists, exactly as in
/// `counter.rs`.
const NOT_NOTE_LEVEL: &[(&str, &str)] = &[];

/// The saturation bar for a bassline, which is *lower than the melody's* — and
/// the reason is the part, not a concession.
///
/// ⛔ **A bass under `mirror_kick` takes its onsets from the generated kick and
/// its pitches from the generated harmony.** Its reachable output is therefore
/// bounded by the variety of the two parts it locks to, and a bassline that broke
/// free of them to score better on this gate would have stopped doing the job a
/// bassline does. `ny-drill` lands at 0.955 for exactly that reason: a narrow
/// register, the kick's pattern, and the chord roots.
///
/// The absolute [`MIN_DISTINCT`] floor is *not* relaxed — every model still owes
/// a producer 500 different basslines, and they all clear it comfortably. This
/// only loosens the no-saturation check, which is the one that is asking a
/// derived part to behave like an independent one.
const BASS_DISTINCT_SHARE: f64 = 0.95;

#[test]
fn every_authored_bassline_parameter_is_read_or_documented_as_unread() {
    let known: Vec<&str> = bass::READ_KEYS
        .iter()
        .copied()
        .chain(NOT_NOTE_LEVEL.iter().map(|(key, _)| *key))
        .collect();

    let mut unread = Vec::new();
    for (id, model, _) in bass_models() {
        for (where_, block) in authored_blocks(&model, "bassline") {
            for path in leaf_paths(&block, "") {
                if !known.iter().any(|prefix| covers(prefix, &path)) {
                    unread.push(format!("{id}: {where_}.{path}"));
                }
            }
        }
    }
    unread.sort();
    unread.dedup();

    assert!(
        unread.is_empty(),
        "these parameters are authored and nothing reads them. Either read them \
         in generators::bass and add them to READ_KEYS, or add them to \
         NOT_NOTE_LEVEL with the reason: {unread:#?}"
    );
}

#[test]
fn the_unread_list_does_not_outlive_the_keys_it_excuses() {
    let authored: BTreeSet<String> = bass_models()
        .iter()
        .flat_map(|(_, _, block)| leaf_paths(block, ""))
        .collect();

    let stale: Vec<&str> = NOT_NOTE_LEVEL
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !authored.iter().any(|path| covers(key, path)))
        .collect();

    assert!(
        stale.is_empty(),
        "nothing authors these any more, so the excuse should go: {stale:#?}"
    );
}

#[test]
fn every_authored_string_vocabulary_is_one_the_engine_can_read() {
    // Three vocabularies here, and every one of them is silently droppable: an
    // unknown rhythm falls back to mirroring the kick, an unparseable cell
    // degree shortens the figure, and an unknown passing-tone name is dropped
    // from the list. All three change what plays with nothing reporting it.
    let mut unknown = Vec::new();

    for (id, _, block) in bass_models() {
        if let Some(name) = block.get("rhythm").and_then(Value::as_str) {
            if !bass::can_read_rhythm(name) {
                unknown.push(format!("{id}: rhythm `{name}`"));
            }
        }

        for tone in block
            .get("passingTones")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default()
        {
            if theory::interval_semitones(tone).is_none() {
                unknown.push(format!("{id}: passingTone `{tone}`"));
            }
        }

        for shape in block
            .get("cellShapes")
            .and_then(Value::as_array)
            .map(|shapes| {
                shapes
                    .iter()
                    .filter_map(Value::as_array)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        {
            for degree in shape.iter().filter_map(Value::as_str) {
                if !bass::can_read_cell_degree(degree) {
                    unknown.push(format!("{id}: cellShape degree `{degree}`"));
                }
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "these names are authored and no reader knows them: {unknown:#?}"
    );
}

#[test]
fn a_model_whose_808_is_the_bassline_generates_no_second_bass() {
    // ⛔ FR-007's unification, over the real roster. Most trap artists author
    // `bass808.role = "bassline"`, so the 808 *is* the bass part; a second one
    // in the same octave is a muddier low end rather than a fuller one.
    let deferring: Vec<String> = bass_models()
        .into_iter()
        .filter(|(_, model, _)| bass::eight_o_eight_is_the_bass(model))
        .map(|(id, model, _)| {
            for seed in 1..8u64 {
                assert!(
                    bass_for(&model, seed).notes.is_empty(),
                    "{id} seed {seed}: the 808 is the bassline and the bass generated anyway"
                );
            }
            id
        })
        .collect();

    assert!(
        !deferring.is_empty(),
        "no shipped model authors bass808.role = bassline, so this gate is vacuous"
    );
    println!("808 carries the bass for: {}", deferring.join(", "));
}

#[test]
fn every_generated_bass_stays_inside_its_authored_register() {
    for (id, model, block) in generating_models() {
        let register = block
            .get("register")
            .and_then(Value::as_array)
            .and_then(|values| Some((values[0].as_u64()? as u8, values[1].as_u64()? as u8)));
        let Some((low, high)) = register else {
            continue;
        };

        for seed in 1..12u64 {
            for note in &bass_for(&model, seed).notes {
                assert!(
                    note.pitch >= low && note.pitch <= high,
                    "{id} seed {seed}: {} is outside {low}..={high}",
                    note.pitch
                );
                if let Some(target) = note.slide_to_pitch {
                    assert!(
                        target >= low && target <= high,
                        "{id} seed {seed}: a glide targets {target}, outside {low}..={high}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_artist_can_reach_a_different_bassline_for_practically_every_seed() {
    let seeds = variety_seeds();
    let mut failures = Vec::new();
    let mut report = Vec::new();

    for (id, model, _) in generating_models() {
        let mut seen = BTreeSet::new();
        for seed in 1..=seeds {
            seen.insert(shape(&bass_for(&model, seed)));
        }

        let share = seen.len() as f64 / seeds as f64;
        report.push(format!(
            "{id}: {}/{seeds} distinct ({share:.3})",
            seen.len()
        ));

        if seen.len() < MIN_DISTINCT {
            failures.push(format!(
                "{id}: only {} distinct basslines in {seeds} seeds, and every model \
                 must reach {MIN_DISTINCT}",
                seen.len()
            ));
        }
        if share < BASS_DISTINCT_SHARE {
            failures.push(format!(
                "{id}: only {}/{seeds} basslines were distinct ({share:.3}, needs \
                 {BASS_DISTINCT_SHARE}) — that model's grammar is saturating",
                seen.len()
            ));
        }
    }

    println!(
        "bass variety over {seeds} seeds:\n  {}",
        report.join("\n  ")
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
