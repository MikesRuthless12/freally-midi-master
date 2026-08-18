//! Modes against the data that ships (TASK-040V).
//!
//! The unit tests in `engine/src/dataset/modes.rs` prove the mechanism. These
//! prove it against the real roster, and answer the question Mike keeps asking:
//! **how many different generations can I actually get — per mood, not just per
//! genre.**
//!
//! Three properties are asserted here that only hold end to end:
//!
//! 1. Every authored mode is well formed, and its overrides land where they say.
//! 2. **A mood only works on a model whose lineage offers it.** A trap mood is
//!    not applicable to liquid dnb, and that is enforced rather than documented.
//! 3. Every `(model, mood)` pair clears the same variety floor a plain model
//!    does — because a mood that collapses the grammar is a mood that gives a
//!    producer one beat.

use std::collections::BTreeSet;

use engine::context::{SessionContext, SessionOverrides};
use engine::dataset::modes;
use engine::generators::{chords, drums, melody};
use engine::theory;

mod common;
use common::{shape, shipped_models, variety_seeds, MIN_DISTINCT};
use rayon::prelude::*;

/// Every shipped model that offers at least one mode, with its modes.
fn models_with_modes() -> Vec<(String, engine::StyleModel, Vec<modes::Mode>)> {
    shipped_models()
        .into_iter()
        .filter_map(|(id, model)| {
            let offered = modes::modes_of(&model);
            (!offered.is_empty()).then_some((id, model, offered))
        })
        .collect()
}

fn melody_for(model: &engine::StyleModel, seed: u64) -> engine::pattern::LaneTrack {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    melody::generate(model, &ctx, seed, &harmony, &kit)
}

/// Everything a mode can move, shaped for comparison.
///
/// ⛔⛔ **The gate is named "do not generate the same THING", and it was
/// comparing the melody alone** (2026-08-17, Mike's call). `justin-niebank` is
/// the model that found it: his two moods are `three-mic ribbon kit` and `full
/// booth kit`, and his research entry distinguishes them **only** by the drum
/// mic list — `Keys: n/a`, and the melody/chords line is a signal chain with no
/// note content in it. His modes therefore override `session.humanize` and
/// `drums.*` and nothing else, and `melody::generate` reads neither: the kit is
/// inert input unless `melody.mirrorSnareDisplacement` is authored, which four
/// models in the roster do and the country-pop lineage is not among them.
///
/// ▶ So the two moods really do differ — tom lanes present against absent,
/// `smallEveryBars` 4 against 16, their own velocity tiers and perc sets — and
/// the assertion could not see any of it. Widening the measurement to what the
/// test already claims to measure is the honest fix; the alternatives were to
/// invent a melodic difference his entry does not support, or to delete a mode
/// and throw away the researched per-song kit decision.
///
/// ⚠ **`melody_for` is deliberately left alone.** Its other two callers ask
/// melody-specific questions — that a melody stays in its key and register, and
/// that a mood reaches enough distinct melodies — and folding the kit into those
/// would let a busy drum track answer a question about the tune.
///
/// ⚠ **This does not lower the bar.** A mode pair that moves nothing any
/// generator reads still fails, which is the decoration this gate exists to
/// catch; `justin-niebank` is the only model in the roster whose modes never
/// touch `melody` or `chords`, so nothing else changes verdict here.
fn everything_for(model: &engine::StyleModel, seed: u64) -> Vec<Vec<(u32, u32, u8, u8)>> {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    let tune = melody::generate(model, &ctx, seed, &harmony, &kit);

    // ⚠ `Chords` is a plan plus its notes; the notes are the part a producer
    // hears, and `ChordEvent` differences that never reach a note are the
    // harmony generator's own business rather than this gate's.
    let mut shaped: Vec<Vec<(u32, u32, u8, u8)>> = kit.iter().map(shape).collect();
    shaped.push(shape(&harmony.track));
    shaped.push(shape(&tune));
    shaped
}

#[test]
fn the_roster_actually_authors_modes() {
    // Guard against this whole file passing vacuously. If nothing offers a mode,
    // every test below is green and proves nothing.
    let with = models_with_modes();
    assert!(
        !with.is_empty(),
        "no shipped model offers a mode, so the mode gates are vacuous"
    );
    println!(
        "models offering modes: {}",
        with.iter()
            .map(|(id, _, offered)| format!("{id}({})", offered.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn every_authored_mode_is_named_once_and_weighted_sanely() {
    for (id, _, offered) in models_with_modes() {
        let mut names: Vec<&str> = offered.iter().map(|m| m.name.as_str()).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            before,
            "{id} authors a duplicate mode name, so one of them is unreachable"
        );

        for mode in &offered {
            assert!(
                !mode.name.trim().is_empty(),
                "{id} authors a mode with no name"
            );
            assert!(
                mode.weight >= 0.0,
                "{id}'s `{}` has a negative weight",
                mode.name
            );
        }

        assert!(
            offered.iter().any(|mode| mode.weight > 0.0),
            "{id} weights every mode zero, so the seed can never pick one"
        );
    }
}

#[test]
fn an_artist_inherits_its_genres_moods_unless_it_authors_its_own() {
    // ⛔ TASK-040U's authenticity rule, and it falls out of `extends` rather than
    // being enforced anywhere. `travis-scott` authors no modes and must offer
    // trap's; `metro-boomin` authors its own and must offer exactly those.
    let models = shipped_models();

    let trap: Vec<String> = modes::modes_of(&models["trap"])
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(
        !trap.is_empty(),
        "trap must author modes for this test to mean anything"
    );

    let inherited: Vec<String> = modes::modes_of(&models["travis-scott"])
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(
        inherited, trap,
        "travis-scott authors no modes and must inherit trap's whole list"
    );

    let own: Vec<String> = modes::modes_of(&models["metro-boomin"])
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert!(
        !own.is_empty(),
        "metro-boomin should author its own modes as the roadmap's worked example"
    );
}

#[test]
fn an_artist_narrows_its_genres_scales_rather_than_being_offered_all_of_them() {
    // ⛔ Mike: *"per artist, you should only be able to pick the scales/moods
    // those artists/producers actually use in their real life day-to-day
    // productions — not be allowed to pick whatever you want."*
    //
    // The same `extends` rule as moods, on the scale axis: an artist's authored
    // `session.scales` **replaces** its genre's rather than adding to it, so a
    // producer is offered what that artist actually records in. `metro-boomin`
    // drops `minor_pentatonic` from trap's four, and that has to survive
    // inheritance — if arrays merged instead of replacing, an artist could never
    // narrow anything.
    let models = shipped_models();

    let scales_of = |id: &str| -> Vec<String> {
        models[id]
            .session
            .as_ref()
            .and_then(|session| session.scales.clone())
            .and_then(|spec| serde_json::to_value(spec).ok())
            .and_then(|value| {
                value
                    .get("values")
                    .or(Some(&value))
                    .and_then(serde_json::Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
            })
            .unwrap_or_default()
    };

    let genre = scales_of("trap");
    let artist = scales_of("metro-boomin");

    assert!(
        !genre.is_empty() && !artist.is_empty(),
        "both must author scales"
    );
    assert!(
        artist.len() < genre.len(),
        "metro-boomin should narrow trap's {genre:?}, got {artist:?}"
    );
    assert!(
        artist.iter().all(|scale| genre.contains(scale)),
        "an artist may narrow its genre's scales, never reach outside them: {artist:?} vs {genre:?}"
    );
}

#[test]
fn a_mood_only_works_on_a_model_whose_lineage_offers_it() {
    // ⛔ Mike: moods must "only work if they are actually used in that specific
    // genre". A mood name is not a global vocabulary — asking a model for one it
    // does not offer is an error, never a quiet fallback to something else.
    let models = shipped_models();

    // Something with no modes at all cannot be given one.
    for (id, model) in &models {
        if modes::modes_of(model).is_empty() {
            assert!(
                modes::apply(model, "dark").is_err(),
                "{id} offers no modes but accepted `dark`"
            );
        }
    }

    // And a name one model offers is not thereby valid on another.
    for (id, _, offered) in models_with_modes() {
        for (other_id, other, _) in models_with_modes() {
            if other_id == id {
                continue;
            }
            for mode in &offered {
                if !modes::offers(&other, &mode.name) {
                    assert!(
                        modes::apply(&other, &mode.name).is_err(),
                        "{other_id} accepted `{}`, which only {id} offers",
                        mode.name
                    );
                }
            }
        }
    }
}

#[test]
fn every_mood_still_generates_in_its_key_and_register() {
    // A mode can move a register or a density, so it can also move them
    // somewhere the invariants no longer hold. Applying one must not cost the
    // guarantees a plain model has.
    for (id, model, offered) in models_with_modes() {
        for mode in &offered {
            let applied = modes::apply(&model, &mode.name)
                .unwrap_or_else(|error| panic!("{id}/{}: {error}", mode.name));

            let register = applied
                .blocks
                .get("melody")
                .and_then(|melody| melody.get("register"))
                .and_then(serde_json::Value::as_array)
                .and_then(|values| Some((values[0].as_u64()? as u8, values[1].as_u64()? as u8)));
            let dissonance = applied
                .blocks
                .get("melody")
                .and_then(|melody| melody.get("halfStepDissonance"))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);

            for seed in 1..10u64 {
                let ctx = SessionContext::from_model(&applied, &SessionOverrides::default(), seed);
                let track = melody_for(&applied, seed);
                let allowed = theory::scale_semitones(ctx.scale);

                for note in &track.notes {
                    if let Some((low, high)) = register {
                        assert!(
                            note.pitch >= low && note.pitch <= high,
                            "{id}/{} seed {seed}: {} is outside {low}..={high}",
                            mode.name,
                            note.pitch
                        );
                    }
                    if dissonance == 0.0 {
                        let class =
                            (i32::from(note.pitch) - i32::from(ctx.key_root)).rem_euclid(12) as u8;
                        assert!(
                            allowed.contains(&class),
                            "{id}/{} seed {seed}: {} is out of the key",
                            mode.name,
                            note.pitch
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_mood_of_every_genre_reaches_the_variety_floor() {
    // ⛔ **The measurement Mike asked for: moods, not just genres.** A mood is a
    // narrower grammar than the model it overrides, so it is exactly where the
    // reachable output could collapse — and a mood that gives a producer fifty
    // beats is a mood they will use once.
    //
    // Failure names the model *and* the mood, because "variety is low" is not
    // something anyone can act on.
    let seeds = variety_seeds();

    // ⛔⛔ **Parallel because this one test was 89 minutes of a four-and-a-half
    // hour suite** (measured 2026-08-17, over 1,304 models). Every iteration is
    // independent — a mood is applied to a clone, generated at its own seeds, and
    // its counts never touch another mood's — so the only thing the serial shape
    // bought was ordering, and that is restored by sorting below.
    //
    // ⚠ **Collected, not pushed.** Two shared `Vec`s would need a lock per model
    // and would serialise most of what this buys; `flat_map` hands each rayon
    // thread its own rows and joins them at the end.
    let rows: Vec<(String, Option<String>)> = models_with_modes()
        .par_iter()
        .flat_map(|(id, model, offered)| {
            offered
                .par_iter()
                .map(|mode| {
                    let applied =
                        modes::apply(model, &mode.name).expect("a listed mode must apply");

                    let mut seen = BTreeSet::new();
                    for seed in 1..=seeds {
                        seen.insert(shape(&melody_for(&applied, seed)));
                    }

                    let share = seen.len() as f64 / seeds as f64;
                    let line = format!(
                        "{id}/{}: {}/{seeds} distinct ({share:.3})",
                        mode.name,
                        seen.len()
                    );
                    let failure = (seen.len() < MIN_DISTINCT).then(|| {
                        format!(
                            "{id}/{}: only {} distinct melodies in {seeds} seeds, and every mood must reach {MIN_DISTINCT}",
                            mode.name,
                            seen.len()
                        )
                    });
                    (line, failure)
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // ⚠ Sorted, because rayon finishes in whatever order the threads land and a
    // report that reshuffles between runs cannot be diffed.
    let mut report: Vec<&str> = rows.iter().map(|(line, _)| line.as_str()).collect();
    report.sort_unstable();
    let mut failures: Vec<&str> = rows
        .iter()
        .filter_map(|(_, failure)| failure.as_deref())
        .collect();
    failures.sort_unstable();

    println!(
        "melody variety per mood over {seeds} seeds:\n  {}",
        report.join("\n  ")
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn two_moods_of_one_artist_do_not_generate_the_same_thing() {
    // The point of modes: "a dark 140 trap beat and a melodic 128 one are both
    // Metro, and no seed will cross between them." If two moods produced the
    // same notes for the same seed, the mood control would be decoration.
    for (id, model, offered) in models_with_modes() {
        if offered.len() < 2 {
            continue;
        }

        for pair in offered.windows(2) {
            let first = modes::apply(&model, &pair[0].name).unwrap();
            let second = modes::apply(&model, &pair[1].name).unwrap();

            // ⚠ Every lane a mode can move, not the melody alone — see
            // [`everything_for`] for the model that found the gap.
            let differs = (1..40u64)
                .any(|seed| everything_for(&first, seed) != everything_for(&second, seed));
            assert!(
                differs,
                "{id}: `{}` and `{}` generate identically for every seed in 1..40",
                pair[0].name, pair[1].name
            );
        }
    }
}
