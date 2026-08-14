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
use std::sync::OnceLock;

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::chords;
use engine::pattern::Scale;
use engine::theory;
use serde_json::Value;

mod common;
use common::{shape, variety_seeds, MIN_DISTINCT};

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
fn shipped_models() -> BTreeMap<String, engine::StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, engine::StyleModel>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            models
        })
        .clone()
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

/// The harmony a model generates for one seed, through the real path.
///
/// The context is built per seed like every other variety measurement, because
/// the key and the tempo are part of what a seed chooses — measuring a thousand
/// harmonies in one fixed key would understate the reachable space and, worse,
/// would not be what the product does.
fn harmony_for(model: &engine::StyleModel, seed: u64) -> engine::pattern::LaneTrack {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    chords::generate(model, &ctx, seed).track
}

/// The share of seeds that must reach a voicing no other seed reached.
///
/// ⛔ **Far lower than [`common::DISTINCT_SHARE`], and calibrated from a
/// measurement rather than guessed.** Harmony is the most *bounded* of the five
/// parts by construction: a model authors a handful of progression families,
/// caps them with `maxChords`, and picks a harmonic rhythm from two or three
/// options. A melody chooses a pitch and a position for every note; a
/// progression chooses a family and then a voicing of it. Demanding the melody's
/// 0.98 would be demanding that a vamp genre stop being a vamp genre — the same
/// mistake as authoring a chord family to satisfy a counter test.
///
/// Measured 2026-08-04 at 1,000 seeds, after the voicing fix below: the roster
/// spans 0.50–0.99, clustering near 0.65. **0.45 is therefore a regression
/// detector, not a target** — it fires when a model's reachable harmony roughly
/// halves, which is the change worth catching, and it does not fire on a genre
/// being what it is.
///
/// The absolute [`MIN_DISTINCT`] floor is what carries the real promise, and it
/// is **not** relaxed: whatever a model's share, it owes a producer 500
/// harmonies before it repeats one.
const HARMONY_DISTINCT_SHARE: f64 = 0.45;

/// Is this model's chord part a placeholder rather than a part?
///
/// ⛔ **Derived from the authored grammar, not a list of model ids** — the same
/// rule `melody.rs`'s `deliberately_narrow` follows, and for the same reason:
/// Phase 5 adds 500+ artists and a hand-kept list would rot on the first one
/// that inherits the property legitimately (`osamason` extends `rage` and
/// authors no chords of its own).
///
/// `impliedByRiff` is that property, and the dataset's own note for it says what
/// it means: *"rage states its harmony through the lead rather than a pad; the
/// chord part is still generated so the tab is not empty."* A model that states
/// its harmony in the melody has one chord to voice, and asking it for 500
/// distinct progressions is asking it to stop being that genre. Its **melody**
/// carries the variety instead, and `melody.rs` measures it there — rage reaches
/// 823/1000.
fn harmony_is_implied_by_the_riff(block: &Value) -> bool {
    block
        .get("impliedByRiff")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[test]
fn an_artist_can_reach_a_different_harmony_for_practically_every_seed() {
    // ⛔ **The gate chords did not have.** Every other generator has measured
    // its variety per model since Phase 2; harmony's only distinctness check was
    // `a_different_seed_reaches_a_different_harmony` below, which asserts that
    // 24 seeds produce *more than one* result. A model that reached exactly two
    // voicings and then repeated them forever passed that, and would have
    // shipped inside a 500-artist roster with nothing to catch it.
    let seeds = variety_seeds();
    let mut failures = Vec::new();
    let mut report = Vec::new();

    for (id, model, block) in harmonic_models() {
        let mut seen = BTreeSet::new();
        for seed in 1..=seeds {
            seen.insert(shape(&harmony_for(&model, seed)));
        }

        let share = seen.len() as f64 / seeds as f64;
        let implied = harmony_is_implied_by_the_riff(&block);
        report.push(format!(
            "{id}: {}/{seeds} distinct ({share:.3}){}",
            seen.len(),
            if implied {
                " — implied by the riff"
            } else {
                ""
            }
        ));

        // ⛔ The floor admits one exception, and it is authored rather than
        // granted: a model whose harmony is stated by its lead has a single
        // chord to voice, and its variety is measured on the melody instead.
        if !implied && seen.len() < MIN_DISTINCT {
            failures.push(format!(
                "{id}: only {} distinct harmonies in {seeds} seeds, and every model \
                 that authors a real chord part must reach {MIN_DISTINCT}",
                seen.len()
            ));
        }

        if !implied && share < HARMONY_DISTINCT_SHARE {
            failures.push(format!(
                "{id}: only {}/{seeds} harmonies were distinct ({share:.3}, needs \
                 {HARMONY_DISTINCT_SHARE}) — that model's grammar is saturating",
                seen.len()
            ));
        }
    }

    println!(
        "harmony variety over {seeds} seeds:\n  {}",
        report.join("\n  ")
    );
    assert!(failures.is_empty(), "{failures:#?}");
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
         part is still generated so the tab is not empty. **Now read by \
         `generators::melody`** (TASK-035), where it raises the chord-tone bias \
         to a floor — the lead is the only thing stating the changes, so it \
         cannot wander off them. It stays listed here because it is not the \
         *chords* generator that acts on it",
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
