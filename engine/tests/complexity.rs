//! The Simple / Complex switch (TASK-125).
//!
//! ⛔⛔ **The claim this file exists to hold is a NEGATIVE one**: the switch
//! scales *within* what a model authored and never overrides it. Mike's rule, as
//! the roadmap states it: *"Complex must not mean 'wrong for the style': a rage
//! vamp made busy is no longer rage, so the switch scales within each model's
//! authored ranges rather than overriding them."*
//!
//! So the tests below come in pairs — one that the switch moves something when
//! the model left room, and one that it moves nothing when the model did not.
//! The second is the harder half and the reason the first is safe.
//!
//! ⚠ **`Authored` byte-identity is held by the rest of the suite rather than
//! here.** Every existing test builds a `SessionContext::default()`, which is
//! `Authored`, so `golden.rs` and the snapshots are already the proof that the
//! default setting moved nothing. Re-asserting it here would add a case that
//! cannot fail.

use engine::context::{Complexity, SessionContext};
use engine::generators::{chords, counter, melody};
use engine::pattern::Part;
use engine::{parts, StyleModel};

mod common;
use common::shipped_models;

fn ctx(complexity: Complexity) -> SessionContext {
    SessionContext {
        bars: 4,
        complexity,
        ..Default::default()
    }
}

fn model(id: &str) -> StyleModel {
    shipped_models()
        .get(id)
        .cloned()
        .unwrap_or_else(|| panic!("`{id}` must ship"))
}

/// Notes in one part, over enough seeds for an average to mean something.
fn notes_over_seeds(model: &StyleModel, part: Part, complexity: Complexity) -> usize {
    let context = ctx(complexity);
    (0..120u64)
        .map(|seed| {
            parts::render(model, &context, parts::Seeds::shared(seed), part)
                .iter()
                .map(|lane| lane.notes.len())
                .sum::<usize>()
        })
        .sum()
}

#[test]
fn a_busier_reading_writes_more_notes_than_a_plainer_one() {
    // The feature, measured on the part every model authors a *range* for.
    // ⚠ Averaged over 120 seeds rather than asserted per seed: the switch leans
    // the draw, it does not clamp it, so any single seed may go either way —
    // which is the point. A per-seed assertion would be asking for a different
    // feature and would fail on the first unlucky number.
    for id in ["trap", "boom-bap", "afro-house"] {
        let m = model(id);
        let simple = notes_over_seeds(&m, Part::Melody, Complexity::Simple);
        let authored = notes_over_seeds(&m, Part::Melody, Complexity::Authored);
        let complex = notes_over_seeds(&m, Part::Melody, Complexity::Complex);

        assert!(
            complex > simple,
            "{id}: complex wrote {complex} melody notes against simple's {simple}"
        );
        assert!(
            (simple..=complex).contains(&authored),
            "{id}: the model as written ({authored}) should sit between {simple} and {complex}"
        );
    }
}

#[test]
fn a_model_that_authored_one_value_is_unmoved_at_every_setting() {
    // ⛔⛔ **The rule the whole feature rests on.** A model that says
    // `densityPerBar: 5` has been specific, and a switch that overrode it would
    // be a different model rather than a busier reading of this one — *"a rage
    // vamp made busy is no longer rage"*.
    let exact: StyleModel = serde_json::from_value(serde_json::json!({
        "id": "exact",
        "type": "genre",
        "name": "Exact",
        "session": { "keys": { "values": ["Cm"] }, "scales": { "values": ["natural_minor"] } },
        "chords": {
            "progressionFamilies": [{ "roman": ["i", "VI"], "weight": 1 }],
            "harmonicRhythm": "vamp",
            "voicing": { "register": [48, 72] }
        },
        "melody": { "register": [60, 84], "densityPerBar": 5 },
        "countermelody": { "densityRatio": 0.5, "styles": "answer_lick" }
    }))
    .expect("the model must parse");

    for part in [Part::Chords, Part::Melody, Part::Counter] {
        let plain = parts::render(
            &exact,
            &ctx(Complexity::Simple),
            parts::Seeds::shared(7),
            part,
        );
        let busy = parts::render(
            &exact,
            &ctx(Complexity::Complex),
            parts::Seeds::shared(7),
            part,
        );
        assert_eq!(
            plain, busy,
            "{part:?} moved on a model that authored one value for everything"
        );
    }
}

#[test]
fn a_vamp_stays_a_vamp_however_busy_the_producer_asks_for() {
    // ⛔ The sentence in the roadmap, as a test: a lane whose *only* authored
    // harmonic rhythm is a vamp holds one chord for the whole clip at every
    // setting, because there is nothing busier for the lean to reach.
    //
    // ⚠ **Built here rather than taken from the roster, and the first cut of
    // this test got that wrong.** It used `rage` on the assumption that rage is
    // the vamp model; rage authors **three** rhythms — `vamp` 5, `2_bars_per_chord`
    // 2, `1_per_bar` 1 — so reaching a chord a bar at Complex is the feature
    // working, not a bug. A model authoring one rhythm is the only thing that
    // tests the claim.
    let vamp: StyleModel = serde_json::from_value(serde_json::json!({
        "id": "one-chord",
        "type": "genre",
        "name": "One Chord",
        "session": { "keys": { "values": ["Cm"] }, "scales": { "values": ["natural_minor"] } },
        "chords": {
            "progressionFamilies": [{ "roman": ["i"], "weight": 1 }],
            "harmonicRhythm": "vamp",
            "voicing": { "register": [48, 72] }
        }
    }))
    .expect("the model must parse");

    let context = ctx(Complexity::Complex);
    for seed in 0..40u64 {
        let events = chords::generate(&vamp, &context, seed).events;
        assert_eq!(
            events.len(),
            1,
            "seed {seed}: a one-rhythm vamp reached {} chord events at Complex — the \
             switch may not add changes the model does not author",
            events.len()
        );
    }
}

#[test]
fn the_switch_only_reaches_choices_the_model_listed() {
    // ⛔ **A value the model never authored must stay unreachable at every
    // setting**, which is what makes `string_spec_leaning` a lean rather than an
    // override. `boom-bap` does not author `syncopated_cell`, so no amount of
    // "busier" may produce the cell lengths that only it can make.
    let authored: Vec<String> = model("boom-bap")
        .blocks
        .get("chords")
        .and_then(|c| c.get("harmonicRhythm"))
        .map(|value| match value {
            serde_json::Value::String(one) => vec![one.clone()],
            serde_json::Value::Object(map) => map
                .get("values")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    assert!(
        !authored.is_empty() && !authored.iter().any(|name| name == "syncopated_cell"),
        "this case needs a model that authors a harmonic rhythm and not the cell; \
         boom-bap now authors {authored:?}"
    );

    // A syncopated cell is the only rhythm that produces chord lengths off the
    // bar, so its absence is measurable rather than a claim about the parameter.
    let bap = model("boom-bap");
    let context = ctx(Complexity::Complex);
    let bar = context.ticks_per_bar();
    for seed in 0..40u64 {
        for event in chords::generate(&bap, &context, seed).events {
            assert!(
                event.start_tick.is_multiple_of(bar),
                "seed {seed}: a chord started at {} — off the bar, which only the \
                 syncopated cell this model never authored can do",
                event.start_tick
            );
        }
    }
}

#[test]
fn the_counter_leans_only_where_a_range_was_authored() {
    // ⚠ `number_leaning`'s rule: a `[min, max]` leans and an exact number does
    // not. `techno` authors `densityRatio: [0.45, 0.8]`, so it must move; the
    // hand-built model above authors `0.5` and must not.
    let techno = model("techno");
    let leaned: Vec<usize> = [Complexity::Simple, Complexity::Complex]
        .iter()
        .map(|complexity| {
            let context = ctx(*complexity);
            (0..80u64)
                .map(|seed| {
                    let harmony = chords::generate(&techno, &context, seed);
                    let kit = engine::generators::drums::generate(&techno, &context, seed);
                    let lead = melody::generate(&techno, &context, seed, &harmony, &kit);
                    counter::generate(&techno, &context, seed, &harmony, &lead)
                        .notes
                        .len()
                })
                .sum()
        })
        .collect();

    assert!(
        leaned[1] > leaned[0],
        "techno's counter wrote {} notes at Complex against {} at Simple, and its \
         densityRatio is an authored range",
        leaned[1],
        leaned[0]
    );
}
