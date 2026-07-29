//! The melody generator against the data that ships (TASK-035, FR-005).
//!
//! The unit tests in `engine/src/generators/melody.rs` prove each device in
//! isolation. These prove the claims that only hold end to end: every authored
//! phrase structure and articulation is one the engine can read, every
//! generated pitch is in the key unless a model asked to leave it, every
//! parameter under `melody` is either read or listed here as deliberately not
//! — and how many *different* melodies an artist can actually reach.

use std::collections::BTreeSet;

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::{chords, drums, melody};
use engine::theory;
use serde_json::Value;

mod common;
use common::{
    authored_blocks, covers, leaf_paths, models_authoring, shape, variety_seeds, DISTINCT_SHARE,
    MIN_DISTINCT,
};

/// Every model that authors a melody, with its `melody` block.
fn melodic_models() -> Vec<(String, engine::StyleModel, Value)> {
    models_authoring("melody")
}

/// The melody a model generates for one seed, with its harmony and drums.
fn melody_for(model: &engine::StyleModel, seed: u64) -> engine::pattern::LaneTrack {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    melody::generate(model, &ctx, seed, &harmony, &kit)
}

/// Parameters under `melody` that no generator reads, each with its reason.
///
/// Every one of these is a real parameter with a real meaning; none of them has
/// a *note-level* meaning, which is the only kind this generator can act on.
const NOT_NOTE_LEVEL: &[(&str, &str)] = &[(
    "timbreHint",
    "which instrument the line should be played by — `keys`, `dark_bell`, \
     `supersaw_stab`, `detuned_piano`. It names a sound, and MIDI notes carry \
     no sound. The preview kit reads the drum equivalent already, and Phase 3's \
     pitched instrument slots (TASK-052/053) are what should read this one",
)];

#[test]
fn every_authored_melody_parameter_is_read_or_documented_as_unread() {
    // The gate this project keeps needing: a parameter nobody reads is
    // decorative and nobody can tell by looking. It walks what the models
    // actually author rather than a hand-maintained list, so a new parameter
    // fails here on the day it is written.
    let known: Vec<&str> = melody::READ_KEYS
        .iter()
        .copied()
        .chain(NOT_NOTE_LEVEL.iter().map(|(key, _)| *key))
        .collect();

    // Every model's own `melody` *and* every mode's override of it — a mode can
    // change any block, so a dead parameter could otherwise hide in one.
    let mut unread = Vec::new();
    for (id, model, _) in melodic_models() {
        for (where_, block) in authored_blocks(&model, "melody") {
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
         in generators::melody and add them to READ_KEYS, or add them to \
         NOT_NOTE_LEVEL with the reason: {unread:#?}"
    );
}

#[test]
fn the_unread_list_does_not_outlive_the_keys_it_excuses() {
    // The other half of the gate. An excuse for a parameter nobody authors any
    // more is a comment that has stopped being true, and it would go on
    // silently excusing a *new* key that happened to reuse the name.
    let authored: BTreeSet<String> = melodic_models()
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
fn every_authored_phrase_structure_is_one_the_engine_can_read() {
    // A structure name the parser does not know falls back to a riff loop, so
    // the phrase shape the model asked for silently does not happen. Strings in
    // this dataset are droppable and every vocabulary needs this test.
    let mut unknown = Vec::new();
    for (id, _, block) in melodic_models() {
        if let Some(name) = block.get("structure").and_then(Value::as_str) {
            if !melody::can_read_structure(name) {
                unknown.push(format!("{id}: structure `{name}`"));
            }
        }
        if let Some(name) = block.get("articulation").and_then(Value::as_str) {
            if !melody::can_read_articulation(name) {
                unknown.push(format!("{id}: articulation `{name}`"));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "these names are authored and no reader knows them: {unknown:#?}"
    );
}

#[test]
fn every_shipped_melody_stays_in_its_key_and_register() {
    // FR-005 end to end, over the real dataset rather than a fixture. A model
    // authoring `halfStepDissonance` is allowed out of the scale — that device
    // says so in its name — so the assertion is scoped to those that do not.
    for (id, model, block) in melodic_models() {
        let dissonance = block
            .get("halfStepDissonance")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let register = block
            .get("register")
            .and_then(Value::as_array)
            .and_then(|values| Some((values[0].as_u64()? as u8, values[1].as_u64()? as u8)));

        for seed in 1..12u64 {
            let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), seed);
            let track = melody_for(&model, seed);
            let allowed = theory::scale_semitones(ctx.scale);

            for note in &track.notes {
                if let Some((low, high)) = register {
                    assert!(
                        note.pitch >= low && note.pitch <= high,
                        "{id} seed {seed}: {} is outside the authored register {low}..={high}",
                        note.pitch
                    );
                }

                if dissonance == 0.0 {
                    let class =
                        (i32::from(note.pitch) - i32::from(ctx.key_root)).rem_euclid(12) as u8;
                    assert!(
                        allowed.contains(&class),
                        "{id} seed {seed}: {} is out of the key and this model \
                         authors no halfStepDissonance",
                        note.pitch
                    );
                }
            }
        }
    }
}

/// Why this model's melody is *deliberately* narrow, if it is.
///
/// ⛔ **Derived from the authored grammar, not a list of model ids.** A small
/// reachable space is sometimes the genre rather than a failure — but naming the
/// models would rot: `osamason` extends `rage`, authors no melody of its own and
/// inherits the narrowness legitimately, and Phase 5 adds 500+ artists that
/// would each need adding by hand. The property is what is being excused, so the
/// property is what is tested.
///
/// `loopVerbatim` with a `motifPitchCount` means "two or three notes, repeated
/// identically" — the hypnotic repeat *is* the style, and a lead that reached a
/// thousand distinct shapes would not be that style.
fn deliberately_narrow(block: &Value) -> Option<&'static str> {
    let verbatim = block
        .get("loopVerbatim")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let motif = block.get("motifPitchCount").is_some();

    (verbatim && motif).then_some(
        "authors loopVerbatim with a motifPitchCount, so the line is a handful \
         of notes repeated exactly — the repeat is the style",
    )
}

#[test]
fn an_artist_can_reach_a_different_melody_for_practically_every_seed() {
    // ⛔ Mike's ask, as a number that can fail: "as many generations as we can
    // per artist — I don't care if it's 1,000 or 10,000." What makes that true
    // is not a stored count but a near-zero collision rate, so this measures
    // the share of seeds reaching note content no other seed reached.
    //
    // Failure names the model *and* the share, because "variety is low" is not
    // something anyone can act on.
    let seeds = variety_seeds();
    let mut failures = Vec::new();
    let mut report = Vec::new();

    for (id, model, block) in melodic_models() {
        let mut seen = BTreeSet::new();
        for seed in 1..=seeds {
            seen.insert(shape(&melody_for(&model, seed)));
        }

        let share = seen.len() as f64 / seeds as f64;
        let narrow = deliberately_narrow(&block);
        report.push(format!(
            "{id}: {}/{seeds} distinct ({share:.3}){}",
            seen.len(),
            if narrow.is_some() {
                " — narrow by design"
            } else {
                ""
            }
        ));

        // ⛔ The absolute floor admits no exceptions. A style may be narrow
        // enough to collide — rage's two-note hook is the genre — and it still
        // owes a producer 500 different melodies before it repeats itself.
        if seen.len() < MIN_DISTINCT {
            failures.push(format!(
                "{id}: only {} distinct melodies in {seeds} seeds, and every model \
                 must reach {MIN_DISTINCT}",
                seen.len()
            ));
        }

        // The no-saturation claim, which is what makes the output unbounded.
        // Waived only where the narrowness is the authored style.
        if narrow.is_none() && share < DISTINCT_SHARE {
            failures.push(format!(
                "{id}: only {}/{seeds} melodies were distinct ({share:.3}, needs \
                 {DISTINCT_SHARE}) — that model's grammar is saturating, which is \
                 a dataset failure rather than a generator one",
                seen.len()
            ));
        }
    }

    // Printed on success too: the numbers are the point, and a gate whose
    // output nobody can read is one nobody will maintain.
    println!(
        "melody variety over {seeds} seeds:\n  {}",
        report.join("\n  ")
    );

    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_narrow_exception_still_applies_to_something() {
    // Same rule as NOT_NOTE_LEVEL's second half: an excuse nothing needs any
    // more is dead code that would silently start excusing something else. If
    // no shipped model authors `loopVerbatim` with a `motifPitchCount`, the
    // exception — and the branch that honours it — should go.
    let excused: Vec<String> = melodic_models()
        .into_iter()
        .filter(|(_, _, block)| deliberately_narrow(block).is_some())
        .map(|(id, _, _)| id)
        .collect();

    assert!(
        !excused.is_empty(),
        "nothing authors loopVerbatim with a motifPitchCount any more, so the \
         narrow exception in the variety gate should be deleted"
    );
    println!("narrow by design: {}", excused.join(", "));
}
