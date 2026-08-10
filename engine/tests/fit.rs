//! Training a workflow from kept generations (TASK-040T).
//!
//! ⛔ **The interesting tests here are the ones that assert the fit does *not*
//! learn too well.** Fitting narrows a grammar, and a narrowed grammar repeats
//! itself: the naive version of this feature writes back exactly what it saw and
//! hands the producer an artist that plays one beat forever. So the rules under
//! test are the constraints — ranges that refuse to collapse, weights that keep
//! their tail — and the gate is the same per-part variety floor a shipped model
//! faces, because only generating can say whether the constraints worked.

use engine::context::{SessionContext, SessionOverrides};
use engine::fit::{self, FitError, MIN_KEPT};
use engine::generators::{chords, drums, melody};
use engine::pattern::{Part, Pattern};

mod common;
use common::shipped_models;

/// `n` generations of one part from one model, at consecutive seeds.
fn kept(model_id: &str, part: Part, n: usize) -> Vec<Pattern> {
    let models = shipped_models();
    let model = &models[model_id];

    (1..=n as u64)
        .map(|seed| {
            let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
            let harmony = chords::generate(model, &ctx, seed);
            let kit = drums::generate(model, &ctx, seed);
            let lanes = match part {
                Part::Drums => kit.clone(),
                _ => vec![melody::generate(model, &ctx, seed, &harmony, &kit)],
            };

            Pattern {
                id: format!("{model_id}-{seed}"),
                part,
                artist_id: model_id.to_owned(),
                seed,
                song_seed: seed,
                bars: 4,
                bpm: 140.0,
                time_sig_num: 4,
                time_sig_den: 4,
                key_root: ctx.key_root,
                scale: ctx.scale,
                lanes,
                ppq: 960,
                mood: None,
                loop_region: None,
                clip_region: None,
            }
        })
        .collect()
}

#[test]
fn training_below_the_floor_is_refused_with_the_count_rather_than_a_grey_button() {
    // ⛔ Mike asked for a minimum and the number is `MIN_KEPT`. The failure has
    // to carry the count: `18 / 30 kept` is something a producer can act on and
    // a disabled button with no explanation is not.
    let short = kept("trap", Part::Melody, MIN_KEPT - 12);
    let error = fit::fit("mine", "Mine", "trap", &[], &short).unwrap_err();

    assert_eq!(
        error,
        FitError::NotEnough {
            kept: MIN_KEPT - 12,
            needed: MIN_KEPT
        }
    );
    assert!(error.to_string().contains(&format!("{}", MIN_KEPT - 12)));
    assert!(error.to_string().contains(&MIN_KEPT.to_string()));
}

#[test]
fn a_fitted_model_extends_its_base_and_says_what_it_was_trained_on() {
    let moods = vec!["dark".to_owned(), "melodic".to_owned()];
    let model = fit::fit(
        "my-trap",
        "My Trap",
        "trap",
        &moods,
        &kept("trap", Part::Melody, 40),
    )
    .expect("forty kept must be enough");

    assert_eq!(model["id"], "my-trap");
    assert_eq!(model["extends"], serde_json::json!(["trap"]));

    // ⛔ Provenance: a model fitted from 40 is a different object from one
    // fitted from 400, and the producer is entitled to know which.
    let notes = model["notes"].as_str().unwrap();
    assert!(notes.contains("40"), "{notes}");
    assert!(
        notes.contains("dark") && notes.contains("melodic"),
        "{notes}"
    );
}

#[test]
fn a_unanimous_kept_set_still_produces_a_range_rather_than_a_constant() {
    // ⛔⛔ **The failure this whole module is about.** Thirty identical
    // generations are the strongest possible signal to a naive fit, and the
    // naive answer — `densityPerBar: 5` — is an artist that plays one beat
    // forever. Unanimity across thirty is weak evidence about the thirty-first.
    let one = kept("trap", Part::Melody, 1).remove(0);
    let same: Vec<Pattern> = (0..MIN_KEPT).map(|_| one.clone()).collect();

    let model = fit::fit("mine", "Mine", "trap", &[], &same).expect("thirty is the floor");
    let density = model["melody"]["densityPerBar"].as_array().unwrap();
    let low = density[0].as_f64().unwrap();
    let high = density[1].as_f64().unwrap();

    assert!(
        high - low >= fit::MIN_DENSITY_SPREAD - 1.0,
        "a unanimous set collapsed to {low}..{high}"
    );
    assert!(low >= 0.0, "a negative density is not a thing to ask for");
}

#[test]
fn one_part_cannot_be_fitted_from_a_handful_because_another_made_up_the_count() {
    // ⛔⛔ **The floor used to be counted across every part together.** Twenty-nine
    // kept drum takes and *one* kept melody reached thirty, and the fit then
    // wrote a melody block whose density, register and contour all came from
    // that single take — the exact "a single kept outlier sets a range's edge"
    // failure `MIN_KEPT` is named for, walking through its own guard.
    let mut mixed = kept("trap", Part::Drums, MIN_KEPT - 1);
    mixed.extend(kept("trap", Part::Melody, 1));
    assert!(
        mixed.len() >= MIN_KEPT,
        "the premise: the total clears the floor"
    );

    let error = fit::fit("mine", "Mine", "trap", &[], &mixed).unwrap_err();
    assert!(
        error.to_string().contains("one part"),
        "it must say which way to fix it: {error}"
    );

    // ⚠ And a part that *does* clear the floor still fits, with the short one
    // simply absent — thirty kept drum takes are a drum style, not a refusal.
    let mut enough = kept("trap", Part::Drums, MIN_KEPT);
    enough.extend(kept("trap", Part::Melody, 2));
    let model = fit::fit("mine", "Mine", "trap", &[], &enough).expect("the drums clear it");
    assert!(model.get("drums").is_some(), "{model}");
    assert!(
        model.get("melody").is_none(),
        "two takes must not become a melody grammar: {model}"
    );
}

#[test]
fn a_range_clamped_at_its_floor_is_still_as_wide_as_it_promised() {
    // ⛔ **The guard used to eat its own width.** Widening symmetrically and
    // then clamping the low edge at zero delivered `[0, 1]` for a kept set with
    // no kick at all — width 1 against a promised 2 — so the anti-collapse rule
    // failed in exactly the case it exists for.
    let silent: Vec<Pattern> = kept("trap", Part::Drums, MIN_KEPT)
        .into_iter()
        .map(|mut pattern| {
            pattern.lanes.clear();
            pattern
        })
        .collect();

    let model = fit::fit("mine", "Mine", "trap", &[], &silent).expect("thirty is the floor");
    let kick = model["drums"]["kick"]["densityPerBar"].as_array().unwrap();
    let low = kick[0].as_f64().unwrap();
    let high = kick[1].as_f64().unwrap();

    assert!(low >= 0.0, "a negative density is not a thing to ask for");
    assert!(
        high - low >= fit::MIN_DENSITY_SPREAD - 0.5,
        "the floor ate the width: {low}..{high}"
    );
}

#[test]
fn a_register_narrower_than_an_octave_is_widened_to_one() {
    // A melody confined to less than an octave cannot move, whatever the kept
    // set happened to do.
    let model = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Melody, 40)).unwrap();
    let register = model["melody"]["register"].as_array().unwrap();
    let low = register[0].as_u64().unwrap();
    let high = register[1].as_u64().unwrap();

    assert!(high > low, "{low}..{high}");
    assert!(
        high - low >= u64::from(fit::MIN_REGISTER_SPREAD),
        "{low}..{high} is narrower than an octave"
    );
    assert!(high <= 127, "a fitted register must stay inside MIDI");
}

#[test]
fn every_contour_keeps_a_weight_even_when_it_was_never_observed() {
    // ⛔ **Weights are fitted, not argmaxed.** A shape observed zero times is
    // unobserved, not impossible — and dropping the tail is what turns a grammar
    // into a template. A kept set of forty trap melodies is overwhelmingly one
    // shape, so this is the case the add-one smoothing exists for rather than a
    // contrived one.
    let model = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Melody, 40)).unwrap();

    for key in ["riff", "descend", "arch"] {
        let weight = model["melody"]["contour"][key]
            .as_f64()
            .unwrap_or_else(|| panic!("`{key}` is missing from the fitted contour"));
        assert!(weight > 0.0, "`{key}` was dropped entirely");
    }
}

#[test]
fn the_weights_follow_what_was_kept_rather_than_being_flat() {
    // The other direction: smoothing must not wash the measurement out. Whatever
    // the kept set says, the fitted weights have to *say something* — three
    // equal thirds would mean the measurement never reached the model.
    let model = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Melody, 40)).unwrap();
    let shapes = ["riff", "descend", "arch"].map(|key| {
        model["melody"]["contour"][key]
            .as_f64()
            .expect("every contour weight is written")
    });

    let flat = shapes.iter().all(|w| (w - 1.0 / 3.0).abs() < 0.02);
    assert!(
        !flat,
        "the contour fit is indistinguishable from nothing: {shapes:?}"
    );
    for weight in shapes {
        assert!(weight > 0.0, "a shape was dropped: {shapes:?}");
    }
}

#[test]
fn the_interval_mix_is_measured_and_deliberately_not_written_back() {
    // ⛔⛔ **This is the one place the obvious fit is wrong, and the numbers are
    // why.** `melody.intervals` weights the generator's *proposal* for the next
    // note; the chord-tone constraint and the octave jump then act on top of it,
    // so the mix that comes *out* is not the mix that went in. Measured on the
    // shipped trap model: it authors `step: 0.62` and its own output steps about
    // 0.33 of the time, because the nearest chord tone is usually a third or
    // more away.
    //
    // ⛔ **Writing the realised mix back into the authored one is therefore a
    // feedback loop, not a fit** — the model proposes more leaps, the constraint
    // widens them further, and the next training round measures wider still. A
    // producer who trains twice would find their artist drifting away from the
    // music they kept, in the direction they never asked for. So the feature is
    // measured (it is real, and the panel can show it) and **not authored**.
    let models = shipped_models();
    let authored = models["trap"].blocks["melody"]["intervals"]["step"]
        .as_f64()
        .expect("trap authors an interval mix");

    let measured: Vec<engine::fit::Features> = kept("trap", Part::Melody, 40)
        .iter()
        .map(fit::measure)
        .collect();
    let totals = measured.iter().fold([0usize; 3], |mut acc, f| {
        for (slot, count) in acc.iter_mut().zip(f.intervals) {
            *slot += count;
        }
        acc
    });
    let all: usize = totals.iter().sum();
    let realised = totals[0] as f64 / all as f64;

    assert!(
        (authored - realised).abs() > 0.15,
        "the premise of this test is that the two differ; authored {authored}, realised {realised}"
    );

    // And the fit does not carry the difference into the model.
    let model = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Melody, 40)).unwrap();
    assert!(
        model["melody"].get("intervals").is_none(),
        "the realised interval mix must not be authored back: {}",
        model["melody"]
    );
    assert!(
        model["melody"].get("octaveJumpProb").is_none(),
        "and neither must the octave-jump rate, which has the same shape"
    );
}

#[test]
fn a_drum_fit_writes_the_kick_and_the_hats_and_nothing_it_cannot_measure() {
    let model = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Drums, 40)).unwrap();

    let kick = model["drums"]["kick"]["densityPerBar"].as_array().unwrap();
    assert!(kick[1].as_f64().unwrap() >= kick[0].as_f64().unwrap());

    let fill = model["drums"]["hihat"]["fillDensity"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&fill), "fillDensity was {fill}");

    // ⛔ No melody block from a drum-only kept set. Writing an empty one would
    // *replace* the base's through `extends` — arrays and objects merge, and a
    // block authored as nothing is still authored — so a trained artist would
    // lose the melody grammar it inherited.
    assert!(model.get("melody").is_none(), "{model}");
}

#[test]
fn a_fitted_model_resolves_and_still_reaches_the_variety_floor() {
    // ⛔⛔ **The gate the constraints exist to pass.** Ranges and smoothing make
    // a collapse unlikely; only generating says whether it happened. Run at 300
    // seeds against a floor of 150 rather than the shipped 1,000/500 — the ratio
    // is what the claim rests on, and the shipped numbers cost a minute per part
    // in a suite that already measures them for thirty models.
    let raw = fit::fit("mine", "Mine", "trap", &[], &kept("trap", Part::Melody, 40)).unwrap();

    let mut registry = engine::Registry::new();
    for (id, model) in shipped_models() {
        let value = serde_json::to_value(&model).expect("a shipped model must serialise");
        registry
            .insert(std::path::Path::new(&format!("{id}.json")), value)
            .expect("and insert");
    }
    registry
        .insert(std::path::Path::new("mine.json"), raw)
        .expect("the fitted model must insert");

    let resolved = registry
        .resolve("mine")
        .expect("and resolve against its base");
    let distinct = fit::verify_variety(&resolved, Part::Melody, 300, 150)
        .unwrap_or_else(|error| panic!("a fitted model must not repeat itself: {error}"));
    println!("fitted model: {distinct}/300 distinct melodies");
}

#[test]
fn a_model_that_can_only_say_one_thing_is_refused_by_name() {
    // The other half of the gate, and it has to be provable rather than assumed:
    // a model narrowed until it repeats must be *reported*, not saved. Built by
    // hand, because the constraints above are meant to stop a fit reaching here.
    let mut registry = engine::Registry::new();
    for (id, model) in shipped_models() {
        let value = serde_json::to_value(&model).unwrap();
        registry
            .insert(std::path::Path::new(&format!("{id}.json")), value)
            .unwrap();
    }
    registry
        .insert(
            std::path::Path::new("narrow.json"),
            serde_json::json!({
                "id": "narrow",
                "type": "artist",
                "name": "Narrow",
                "extends": ["trap"],
                "melody": {
                    "densityPerBar": [1, 1],
                    "register": [60, 61],
                    "structure": "loop_verbatim",
                    "phraseBars": 1
                }
            }),
        )
        .unwrap();

    let resolved = registry.resolve("narrow").unwrap();
    let error = fit::verify_variety(&resolved, Part::Melody, 200, 150).unwrap_err();
    assert!(error.contains("keep more"), "{error}");
}

#[test]
fn a_kept_set_is_counted_per_part_so_the_ui_can_say_where_it_is_short() {
    let mut all = kept("trap", Part::Melody, 4);
    all.extend(kept("trap", Part::Drums, 7));

    let counts = fit::kept_by_part(&all);
    assert_eq!(counts.get(&Part::Melody), Some(&4));
    assert_eq!(counts.get(&Part::Drums), Some(&7));
    assert_eq!(counts.get(&Part::Bass), None);
}
