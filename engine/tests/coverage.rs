//! What a model covers, said before Generate is pressed (TASK-158D).
//!
//! ⛔⛔ **The failure this closes is silence with no warning.** `bass.rs`,
//! `chords.rs`, `melody.rs` and `counter.rs` each open with a `let … else {
//! return empty }` on their own block, so a model that authored no `melody`
//! answers Generate on the Melody tab with an empty track. That is correct
//! behaviour — an artist who does not write melodies should not have one
//! invented for them — but until the detail pane said so, the only way to find
//! out was to press the button and stare at an empty roll.
//!
//! Mike's TASK-158D: *"The detail pane should tell a producer what they are
//! about to get — genres, moods, tempo range, what the model does and does
//! **not** cover — rather than making them press Generate to find out."*
//!
//! ⚠ **Asserted against `generate` itself, not against the block list.**
//! `parts_of` reading `blocks` and this test reading `blocks` would prove only
//! that one function agrees with itself. Every claim below is checked by
//! generating and counting the notes, which is the thing the producer sees.

mod common;

use engine::context::{parts_of, SessionContext, SessionDefaults, SessionOverrides};
use engine::generators::{bass, chords, counter, drums, melody};
use engine::pattern::Part;

/// Generate every part for `model` on one seed and report which wrote notes.
fn parts_with_notes(model: &engine::StyleModel, seed: u64) -> Vec<Part> {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    let lead = melody::generate(model, &ctx, seed, &harmony, &kit);
    let answer = counter::generate(model, &ctx, seed, &harmony, &lead);
    let low = bass::generate(model, &ctx, seed, &harmony, &kit);

    let mut parts = Vec::new();
    if kit.iter().any(|lane| !lane.notes.is_empty()) {
        parts.push(Part::Drums);
    }
    if !harmony.track.notes.is_empty() {
        parts.push(Part::Chords);
    }
    if !lead.notes.is_empty() {
        parts.push(Part::Melody);
    }
    if !answer.notes.is_empty() {
        parts.push(Part::Counter);
    }
    // ⛔ **The 808 lane counts.** For a model whose `bass808.role` is
    // `"bassline"` the bass generator returns empty on purpose and the sub comes
    // out of the kit — see `parts_of`.
    if !low.notes.is_empty() || bass::eight_o_eight_is_the_bass(model) {
        parts.push(Part::Bass);
    }
    parts
}

#[test]
fn every_shipped_model_covers_what_it_says_it_covers() {
    // ⛔ **One direction only, and deliberately.** A part `parts_of` names must
    // produce notes — that is the promise the pane makes, and breaking it is the
    // readout-that-lies failure arriving through the fix for it.
    //
    // ⚠ The converse is **not** asserted: a part can produce notes on one seed
    // and not another (a fill lane that rolled empty, a melody whose plan came
    // out sparse), so "produced nothing on seed 1" is not evidence of a claim
    // being wrong. This test would be flaky in exactly the way the variety
    // gates are careful not to be.
    let models = common::shipped_models();
    let mut broken: Vec<String> = Vec::new();

    for (id, model) in &models {
        let claimed = parts_of(model);
        let produced = parts_with_notes(model, 1);
        for part in claimed {
            if !produced.contains(&part) {
                broken.push(format!("{id} claims {part:?} and generated nothing"));
            }
        }
    }

    assert!(
        broken.is_empty(),
        "{} models promise a part they do not write:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

#[test]
fn a_model_that_authors_no_melody_does_not_claim_one() {
    // The whole point, stated on the smallest possible model: no blocks at all.
    // Drums still come out — `drums.rs` reads its block as an `Option` and falls
    // back to a real kit — and nothing else does.
    let bare: engine::StyleModel = serde_json::from_value(serde_json::json!({
        "id": "bare",
        "type": "genre",
        "name": "Bare",
    }))
    .expect("a model with no blocks is legal");

    assert_eq!(parts_of(&bare), vec![Part::Drums]);
}

#[test]
fn an_808_that_plays_the_bassline_counts_as_bass() {
    // ⛔⛔ **`bass.rs` returns empty for such a model on purpose** — FR-007's
    // "unify with the 808 lane", because doubling a sub is a production mistake
    // rather than a fuller sound. But the producer *does* get a bassline; it
    // comes out of the kit's `bass808` lane. Reporting "no bass" here would be
    // wrong about most of trap.
    let trap: engine::StyleModel = serde_json::from_value(serde_json::json!({
        "id": "eight-oh-eight",
        "type": "genre",
        "name": "808",
        "drums": { "bass808": { "role": "bassline" } },
    }))
    .expect("a drums-only model is legal");

    assert!(parts_of(&trap).contains(&Part::Bass));
}

#[test]
fn a_countermelody_with_no_melody_to_answer_is_not_claimed() {
    // `counter.rs` returns empty when the melody did: *"a counter answers a
    // melody. With nothing to answer it would just be a second melody."*
    let orphan: engine::StyleModel = serde_json::from_value(serde_json::json!({
        "id": "orphan-counter",
        "type": "genre",
        "name": "Orphan",
        "countermelody": { "role": "call-response" },
    }))
    .expect("the block shape is free-form");

    assert!(!parts_of(&orphan).contains(&Part::Counter));
}

#[test]
fn the_tempo_range_is_the_models_own_and_not_the_engines() {
    // ⚠ A model that said nothing about tempo has not said it works from 40 to
    // 300. The nominal twice is the honest summary of silence; a legal range
    // would be an invention.
    let silent: engine::StyleModel = serde_json::from_value(serde_json::json!({
        "id": "no-tempo",
        "type": "genre",
        "name": "No tempo",
    }))
    .expect("a model need not author a session block");
    let defaults = SessionDefaults::of(&silent);
    assert_eq!(defaults.bpm_min, defaults.bpm);
    assert_eq!(defaults.bpm_max, defaults.bpm);

    let spread: engine::StyleModel = serde_json::from_value(serde_json::json!({
        "id": "wide",
        "type": "genre",
        "name": "Wide",
        "session": { "bpm": { "min": 68.0, "max": 96.0, "mode": 82.0 } },
    }))
    .expect("a bpm spec is min/max/mode");
    let defaults = SessionDefaults::of(&spread);
    assert_eq!(defaults.bpm, 82.0);
    assert_eq!(defaults.bpm_min, 68.0);
    assert_eq!(defaults.bpm_max, 96.0);
}
