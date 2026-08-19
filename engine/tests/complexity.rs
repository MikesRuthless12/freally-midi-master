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
use engine::dataset::StrSpec;
use engine::generators::{chords, counter, melody, read};
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
    //
    // ⚠ Read through `StrSpec::options` — the same type `string_spec_leaning`
    // reads this parameter through — rather than by matching the JSON by hand.
    // A second parser here could disagree with the one under test and the test
    // would be asserting against its own reading rather than the engine's.
    let authored: Vec<String> = model("boom-bap")
        .blocks
        .get("chords")
        .and_then(|c| c.get("harmonicRhythm"))
        .and_then(|v| serde_json::from_value::<StrSpec>(v.clone()).ok())
        .map(|spec| spec.options())
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

/// The lean table names real parameters, and nothing leans without a row.
///
/// ⛔⛔ **TASK-170's whole point is that this list is reviewable, so it has to be
/// kept honest.** `read::LEANING` is now the only place that says which authored
/// parameters the Simple/Complex switch reaches and in which direction — the
/// lean used to be opt-in at each call site in four different spellings. A table
/// nothing checks would rot into a comment: a row could name a key no model
/// authors, or a generator could stop reading one, and the switch would quietly
/// reach less than the table claims.
#[test]
fn every_leaning_parameter_is_one_the_dataset_actually_authors() {
    let models = shipped_models();
    assert!(
        !models.is_empty(),
        "no models, so this would pass vacuously"
    );

    for (block, key, _) in read::LEANING {
        let authored = models
            .values()
            .any(|model| model.blocks.get(*block).and_then(|b| b.get(*key)).is_some());
        assert!(
            authored,
            "`read::LEANING` leans {block}.{key}, which no shipped model authors — \
             either the row is stale or the parameter was renamed"
        );
    }
}

/// A parameter with no row does not lean, and that is by construction.
///
/// ⚠ This is the safety the table buys: `read::leaning` answers `Authored` for
/// anything it does not find, so a call site cannot lean a parameter the table
/// does not declare. Deleting a row therefore shows up as generation changing —
/// which `golden.rs` catches — rather than as a switch that silently does less.
#[test]
fn a_parameter_absent_from_the_table_does_not_lean() {
    for complexity in [Complexity::Simple, Complexity::Complex] {
        assert_eq!(
            read::leaning(complexity, "melody", "registerOffset"),
            Complexity::Authored,
            "an unlisted parameter must not lean"
        );
    }

    // ...and a listed one does, in the direction the row states.
    assert_eq!(
        read::leaning(Complexity::Complex, "melody", "densityPerBar"),
        Complexity::Complex
    );
    assert_eq!(
        read::leaning(Complexity::Complex, "chords", "chordDurationBeats"),
        Complexity::Complex.inverted(),
        "a shorter chord cell is the busier one, so that row is inverted"
    );
}
