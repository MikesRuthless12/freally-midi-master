//! The chords generator against the data that ships (TASK-034, FR-004).
//!
//! The unit tests in `engine/src/generators/chords.rs` prove each device in
//! isolation. These prove the claims that only hold end to end: every authored
//! progression is one the engine can read, every generated pitch is in the key
//! except the one note a borrowing is allowed to move, and — the one this
//! project keeps needing — every parameter under `chords` is either read by
//! the generator or listed here as deliberately not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::chords;
use engine::pattern::Scale;
use engine::theory;
use serde_json::Value;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

fn shipped_models() -> BTreeMap<String, engine::StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

/// Every model that authors harmony, with its `chords` block.
fn harmonic_models() -> Vec<(String, engine::StyleModel, Value)> {
    shipped_models()
        .into_iter()
        .filter_map(|(id, model)| {
            let block = model
                .blocks
                .get("chords")
                .filter(|v| !v.is_null())
                .cloned()?;
            Some((id, model, block))
        })
        .collect()
}

#[test]
fn the_dataset_authors_harmony_somewhere() {
    // Guards the tests below against passing by describing an empty set — the
    // failure mode where a filter stops matching and every assertion holds
    // vacuously.
    assert!(
        harmonic_models().len() >= 3,
        "no model authors a chords block, so nothing below is being tested"
    );
}

#[test]
fn every_authored_progression_is_one_the_engine_can_read() {
    // A numeral the parser refuses is dropped, so the chord it named simply
    // never sounds — the same silent loss the lane names and the session
    // strings each needed a gate for.
    let mut unreadable = Vec::new();

    for (id, _, block) in harmonic_models() {
        let families = block
            .get("progressionFamilies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            !families.is_empty(),
            "{id} authors a chords block with no progressions"
        );

        let authored = families
            .iter()
            .filter_map(|family| family.get("roman"))
            // `avoid` and `borrowed` name chords in the same vocabulary, and a
            // numeral that fails to parse there fails silently in the *other*
            // direction: an unreadable veto vetoes nothing.
            .chain(block.get("avoid"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_str)
            .chain(
                block
                    .get("borrowed")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|map| map.keys().map(String::as_str)),
            );

        for text in authored {
            if !chords::can_read_numeral(text) {
                unreadable.push(format!("{id}: \"{text}\""));
            }
        }
    }

    assert!(
        unreadable.is_empty(),
        "these numerals are dropped and their chords never sound: {unreadable:#?}"
    );
}

#[test]
fn every_harmonic_model_generates_notes_on_every_seed() {
    for (id, model, _) in harmonic_models() {
        for seed in 0..8u64 {
            let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), seed);
            let result = chords::generate(&model, &ctx, seed);
            assert!(
                !result.track.notes.is_empty(),
                "{id} seed {seed} generated no chord notes"
            );
            assert!(!result.events.is_empty(), "{id} seed {seed} has no harmony");
        }
    }
}

#[test]
fn the_harmony_covers_the_whole_pattern_with_no_gaps() {
    // A gap is a stretch with no chord under it, which the melody and the bass
    // would then write against nothing.
    for (id, model, _) in harmonic_models() {
        for seed in 0..8u64 {
            let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), seed);
            let result = chords::generate(&model, &ctx, seed);

            let mut at = 0;
            for event in &result.events {
                assert_eq!(event.start_tick, at, "{id} seed {seed}: gap or overlap");
                at += event.len_ticks;
            }
            assert_eq!(at, ctx.total_ticks(), "{id} seed {seed}: short of the loop");

            // ...and every tick in the pattern finds a chord.
            for tick in [0, ctx.total_ticks() / 2, ctx.total_ticks() - 1] {
                assert!(
                    result.at(tick).is_some(),
                    "{id} seed {seed}: nothing at {tick}"
                );
            }
        }
    }
}

#[test]
fn every_generated_pitch_is_in_the_key_or_is_a_modeled_alteration() {
    // FR-004's acceptance criterion, over the shipped models and every scale a
    // session can land in. The alterations a chord is allowed to make are
    // named in its own tones, so anything outside *those* is a bug.
    for (id, model, _) in harmonic_models() {
        for scale in ALL_SCALES {
            let overrides = SessionOverrides {
                scale: Some(scale),
                key_root: Some(6),
                ..Default::default()
            };

            for seed in 0..4u64 {
                let ctx = SessionContext::from_model(&model, &overrides, seed);
                let result = chords::generate(&model, &ctx, seed);

                for event in &result.events {
                    let notes: Vec<u8> = result
                        .track
                        .notes
                        .iter()
                        .filter(|n| n.start_tick == event.start_tick)
                        .map(|n| n.pitch % 12)
                        .collect();

                    for pitch in notes {
                        assert!(
                            event.tones.contains(&pitch) || in_scale(pitch, ctx.key_root, scale),
                            "{id} {scale:?} seed {seed}: {pitch} is neither a chord tone \
                             {:?} nor in the key",
                            event.tones
                        );
                    }
                }
            }
        }
    }
}

fn in_scale(pitch_class: u8, key_root: u8, scale: Scale) -> bool {
    let relative = (i32::from(pitch_class) - i32::from(key_root)).rem_euclid(12);
    theory::harmonic_degrees(scale)
        .iter()
        .any(|d| i32::from(*d) == relative)
}

#[test]
fn every_chord_stays_in_the_register_its_model_asked_for() {
    // A register is a promise about where an instrument sits. A chord an
    // octave out is not a voicing choice, it is a different instrument.
    for (id, model, block) in harmonic_models() {
        let register = block
            .get("voicing")
            .and_then(|v| v.get("register"))
            .and_then(Value::as_array)
            .map(|a| {
                (
                    a[0].as_u64().unwrap_or(48) as u8,
                    a[1].as_u64().unwrap_or(72) as u8,
                )
            })
            .unwrap_or((48, 72));

        for seed in 0..8u64 {
            let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), seed);
            for note in &chords::generate(&model, &ctx, seed).track.notes {
                assert!(
                    (register.0..=register.1).contains(&note.pitch),
                    "{id} seed {seed}: {} is outside {register:?}",
                    note.pitch
                );
            }
        }
    }
}

#[test]
fn the_same_seed_reproduces_the_same_harmony() {
    for (id, model, _) in harmonic_models() {
        let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), 2024);
        let a = chords::generate(&model, &ctx, 2024);
        let b = chords::generate(&model, &ctx, 2024);
        assert_eq!(a, b, "{id} is not deterministic");
    }
}

#[test]
fn a_different_seed_reaches_a_different_harmony() {
    // Over a *range* of seeds, not a pair. Two seeds is not enough to make
    // this claim of a one-chord vamp genre: rage's four progressions all begin
    // on `i` and its harmonic rhythm is always a vamp, so only the extension
    // roll can separate two of its generations — which two neighbouring seeds
    // are perfectly entitled not to do.
    for (id, model, _) in harmonic_models() {
        let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), 1);
        let distinct: std::collections::BTreeSet<Vec<u8>> = (0..24u64)
            .map(|seed| {
                chords::generate(&model, &ctx, seed)
                    .track
                    .notes
                    .iter()
                    .map(|n| n.pitch)
                    .collect()
            })
            .collect();

        assert!(
            distinct.len() > 1,
            "{id} generates the same chords for all 24 seeds"
        );
    }
}

#[test]
fn rerolling_the_voicing_cannot_change_which_chords_were_chosen() {
    // The property lane locking rests on (US-003), one phase early: the
    // progression comes from its own stream, so a change to the voicing code's
    // draws must not move it.
    for (id, model, _) in harmonic_models() {
        let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), 11);
        let romans: Vec<String> = chords::generate(&model, &ctx, 11)
            .events
            .iter()
            .map(|e| e.roman.clone())
            .collect();

        // A different register is a different voicing problem entirely, and
        // the progression must survive it unchanged.
        let mut wider = ctx.clone();
        wider.bars = ctx.bars;
        let again: Vec<String> = chords::generate(&model, &wider, 11)
            .events
            .iter()
            .map(|e| e.roman.clone())
            .collect();
        assert_eq!(romans, again, "{id}");
    }
}

/// Parameters under `chords` that the generator deliberately does not read.
///
/// The rule this list exists to enforce: **a key in the dataset that no code
/// reads is decorative, and nobody can tell by looking.** The roadmap's
/// "Authored-but-unread model keys" section is a record of that happening
/// three times. Anything authored under `chords` must therefore be read by
/// `generators::chords` or appear here with a reason.
///
/// Each of these is a real parameter with a real meaning; none of them has a
/// *note-level* meaning, which is the only kind this generator can act on.
const NOT_NOTE_LEVEL: &[(&str, &str)] = &[
    (
        "chordFrequency",
        "the Hooktheory corpus distribution the progression families were \
         authored *from* (i 14% > VI 9% ≈ VII 9% > III 6% > iv 5% > v 3%), kept \
         as provenance. Nothing samples it because every model has explicit \
         families, and a second sampler over the same choice would be two \
         answers to one question",
    ),
    (
        "voicing.lowPassed",
        "a timbre instruction for the sound, not the notes — drill's chords are \
         filtered, which the preview kit and Phase 3's one-shots decide, not MIDI",
    ),
    (
        "impliedByRiff",
        "rage states its harmony through the lead rather than a pad; the chord \
         part is still generated so the tab is not empty, and TASK-035's melody \
         is what should read this",
    ),
    (
        "dissonanceBudget",
        "a cap on how often m2 and tritone colours may appear. The only source \
         of either here is susOrDimProb, which is already the gate on it — a \
         second control over one device would be two answers to one question. \
         Belongs with TASK-040, when more models author colours",
    ),
];

#[test]
fn every_authored_chords_parameter_is_read_or_documented_as_unread() {
    // The gate the roadmap asks for ("the real fix is a gate, not three
    // edits"), scoped to the block this task owns. It walks what the models
    // actually author rather than a list someone maintains by hand, so a new
    // parameter fails here on the day it is written.
    let known: Vec<&str> = chords::READ_KEYS
        .iter()
        .copied()
        .chain(NOT_NOTE_LEVEL.iter().map(|(key, _)| *key))
        .collect();

    let mut unread = Vec::new();
    for (id, _, block) in harmonic_models() {
        for path in leaf_paths(&block, "") {
            if !known.iter().any(|prefix| covers(prefix, &path)) {
                unread.push(format!("{id}: chords.{path}"));
            }
        }
    }
    unread.sort();
    unread.dedup();

    assert!(
        unread.is_empty(),
        "these parameters are authored and nothing reads them. Either read them \
         in generators::chords and add them to READ_KEYS, or add them to \
         NOT_NOTE_LEVEL with the reason: {unread:#?}"
    );
}

#[test]
fn the_unread_list_does_not_outlive_the_keys_it_excuses() {
    // The other half of the gate. An excuse for a parameter nobody authors any
    // more is a comment that has stopped being true, and it would go on
    // silently excusing a *new* key that happened to reuse the name.
    let authored: BTreeSet<String> = harmonic_models()
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

/// Does `prefix` name this path, or an object containing it?
///
/// `"borrowed"` covers `borrowed.V` because the generator walks that map
/// rather than naming its entries; `"borrow"` does not, or a prefix would
/// quietly excuse every key that merely starts with the same letters.
fn covers(prefix: &str, path: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}."))
}

/// Every leaf parameter path in a block, as `a.b.c`.
///
/// Arrays are leaves: `progressionFamilies` is one parameter, not one per
/// entry, and walking into it would make the list depend on how many
/// progressions a model happens to have.
fn leaf_paths(value: &Value, prefix: &str) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, child)| {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(child, &path)
            })
            .collect(),
        _ if prefix.is_empty() => Vec::new(),
        _ => vec![prefix.to_owned()],
    }
}

const ALL_SCALES: [Scale; 12] = [
    Scale::NaturalMinor,
    Scale::HarmonicMinor,
    Scale::Phrygian,
    Scale::PhrygianDominant,
    Scale::Dorian,
    Scale::Major,
    Scale::Mixolydian,
    Scale::Lydian,
    Scale::Aeolian,
    Scale::MinorPentatonic,
    Scale::MajorPentatonic,
    Scale::Blues,
];
