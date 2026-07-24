//! The drum core against the genres that actually ship.
//!
//! The unit tests in `engine/src/generators/drums.rs` drive hand-built models
//! so each rule can be isolated. These drive `data/` — the claim being that the
//! grammar the research encoded into trap and UK drill comes back out of the
//! generator recognisably.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::context::SessionContext;
use engine::generators::drums::{generate, SnarePlacement};
use engine::generators::grid;
use engine::pattern::{Articulation, Lane, LaneTrack};
use engine::StyleModel;
use serde_json::Value;

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

fn model(id: &str) -> StyleModel {
    shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("`{id}` must ship"))
}

/// Four bars of 4/4 at the model's own tempo. Building the context *from* the
/// model is TASK-033; the drum grammar does not read the tempo, so only the
/// bar count matters here.
fn ctx(model: &StyleModel, bars: u16) -> SessionContext {
    let bpm = model
        .session
        .as_ref()
        .and_then(|s| s.bpm.as_ref())
        .map(|b| b.nominal() as f32)
        .unwrap_or(140.0);
    SessionContext {
        bpm,
        bars,
        ..Default::default()
    }
}

fn lane(lanes: &[LaneTrack], want: Lane) -> Option<&LaneTrack> {
    lanes.iter().find(|l| l.lane == want)
}

/// Note starts in one lane, as `(bar, tick within the bar)`.
fn positions(lanes: &[LaneTrack], want: Lane, ctx: &SessionContext) -> Vec<(u32, u32)> {
    let bar = ctx.ticks_per_bar();
    lane(lanes, want)
        .map(|l| {
            l.notes
                .iter()
                .map(|n| (n.start_tick / bar, n.start_tick % bar))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn trap_puts_its_snare_on_beat_three_and_nowhere_else() {
    // The AC for this task, and the single most recognisable fact about the
    // genre: a half-time feel programmed at double-time tempo.
    let trap = model("trap");
    let context = ctx(&trap, 4);
    let beat_three = grid::ticks_per_beat(&context) * 2;

    let lanes = generate(&trap, &context, 2024);
    let snare = lane(&lanes, Lane::Snare).expect("trap has a snare");

    let mains: Vec<u32> = snare
        .notes
        .iter()
        .filter(|n| {
            !matches!(
                n.articulation,
                Some(Articulation::Ghost) | Some(Articulation::Roll)
            )
        })
        .map(|n| n.start_tick % context.ticks_per_bar())
        .collect();

    assert_eq!(mains.len(), 4, "one snare per bar");
    assert!(
        mains.iter().all(|tick| *tick == beat_three),
        "trap's snare must sit on beat 3: {mains:?}"
    );
}

#[test]
fn traps_full_time_variant_is_the_rare_exception_the_model_asks_for() {
    // `fullTimeVariantProb: 0.1` — "only for uptempo crossovers". The variant
    // has to exist, and has to stay rare; if it never fired, the parameter
    // would be decoration, and if it fired often, trap would stop sounding
    // like trap.
    let trap = model("trap");
    let context = ctx(&trap, 2);
    let beat_three = grid::ticks_per_beat(&context) * 2;

    let mut full_time = 0u32;
    let seeds = 300u32;
    for seed in 0..seeds {
        let lanes = generate(&trap, &context, u64::from(seed));
        let snare = lane(&lanes, Lane::Snare).unwrap();
        let mains: Vec<u32> = snare
            .notes
            .iter()
            .filter(|n| {
                !matches!(
                    n.articulation,
                    Some(Articulation::Ghost) | Some(Articulation::Roll)
                )
            })
            .map(|n| n.start_tick % context.ticks_per_bar())
            .collect();

        // Whichever it picks, it commits for the whole pattern. Counted by
        // *where* the snares land rather than how many there are: a fill takes
        // the run-up at the end of a bar, so the count varies while the
        // placement does not.
        if mains.iter().all(|t| *t == beat_three) {
            continue;
        }
        assert!(
            mains.iter().any(|t| *t == grid::ticks_per_beat(&context)),
            "seed {seed}: a full-time variant plays the 2 as well as the 4"
        );
        full_time += 1;
    }

    let share = f64::from(full_time) / f64::from(seeds);
    assert!(
        (0.04..=0.18).contains(&share),
        "asked for a 10% full-time variant, got {share:.3}"
    );
}

#[test]
fn the_drill_two_bar_kick_form_reproduces_exactly() {
    // Research ch. 1 §2: "beat 1; and-of-2; beat 4 | bar 2: and-of-1; beat 3".
    // The model authors it as `fourBarGrammar`, and it is the genre's
    // fingerprint — it must be identical on every seed, not merely likely.
    let drill = model("uk-drill");
    let context = ctx(&drill, 4);
    let beat = grid::ticks_per_beat(&context);
    let eighth = beat / 2;

    let bar_one = vec![0, beat + eighth, beat * 3];
    let bar_two = vec![eighth, beat * 2];

    for seed in [0u64, 1, 7, 99, 12_345] {
        let lanes = generate(&drill, &context, seed);
        let kicks = positions(&lanes, Lane::Kick, &context);

        for bar in 0..4u32 {
            let in_bar: Vec<u32> = kicks
                .iter()
                .filter(|(b, _)| *b == bar)
                .map(|(_, tick)| *tick)
                .collect();
            let expected = if bar % 2 == 0 { &bar_one } else { &bar_two };
            assert_eq!(&in_bar, expected, "seed {seed}, bar {bar}");
        }
    }
}

#[test]
fn drills_kick_form_is_the_tresillo_it_is_described_as() {
    // Bar 1 of that grammar is 3-3-2 spelled in grid positions. Naming the
    // relationship keeps the two facts from drifting apart.
    let drill = model("uk-drill");
    let context = ctx(&drill, 2);
    let lanes = generate(&drill, &context, 3);

    let first_bar: Vec<u32> = positions(&lanes, Lane::Kick, &context)
        .iter()
        .filter(|(bar, _)| *bar == 0)
        .map(|(_, tick)| tick / grid::SIXTEENTH)
        .collect();

    assert_eq!(first_bar, vec![0, 6, 12]);
    assert!(first_bar.iter().all(|i| grid::is_tresillo(*i)));
}

#[test]
fn drill_answers_its_snare_with_a_ghost_on_the_and_of_four() {
    // The signature the research calls out, at the 40–50% velocity it states —
    // which is louder than the cross-genre ghost tier, because the model says
    // so and a specific number beats a general one.
    let drill = model("uk-drill");
    let context = ctx(&drill, 4);
    let and_of_four = grid::ticks_per_beat(&context) * 3 + grid::SIXTEENTH * 2;

    let mut bars_with_ghosts = 0;
    let mut total_bars = 0;

    for seed in 0..50u64 {
        let lanes = generate(&drill, &context, seed);
        let snare = lane(&lanes, Lane::Snare).unwrap();
        for note in &snare.notes {
            if note.articulation != Some(Articulation::Ghost) {
                continue;
            }
            // Drill nudges its snares off the grid on purpose, so the ghost is
            // near the and-of-4 rather than exactly on it.
            let within_bar = note.start_tick % context.ticks_per_bar();
            let offset = within_bar.abs_diff(and_of_four);
            assert!(
                offset < grid::SIXTEENTH,
                "seed {seed}: ghost at {within_bar} is not near the and-of-4 ({and_of_four})"
            );
            assert!(
                (50..=64).contains(&note.vel),
                "seed {seed}: ghost velocity {} is not the 40–50% the model states",
                note.vel
            );
            bars_with_ghosts += 1;
        }
        total_bars += 4;
    }

    let share = f64::from(bars_with_ghosts) / f64::from(total_bars);
    assert!(
        (0.6..=0.8).contains(&share),
        "the model asks for ghosts in 70% of bars, got {share:.3}"
    );
}

#[test]
fn drills_snare_sits_off_the_grid_on_purpose() {
    // `offGridMs: [2, 6]` — a displacement the genre is made of, applied by the
    // grammar rather than by the humanizer, so it survives hard quantization.
    let drill = model("uk-drill");
    let context = ctx(&drill, 2);
    let beat_three = grid::ticks_per_beat(&context) * 2;

    let mut displaced = 0;
    for seed in 0..40u64 {
        let lanes = generate(&drill, &context, seed);
        let snare = lane(&lanes, Lane::Snare).unwrap();
        let first = snare
            .notes
            .iter()
            .find(|n| n.articulation != Some(Articulation::Ghost))
            .unwrap();
        let offset = first.start_tick.abs_diff(beat_three);
        // 2–6 ms at drill tempo is 4–14 ticks: felt, not seen.
        assert!(
            offset <= 16,
            "seed {seed}: {offset} ticks is a flam, not a nudge"
        );
        if offset > 0 {
            displaced += 1;
        }
    }
    assert!(
        displaced > 30,
        "only {displaced} of 40 seeds nudged the snare — the parameter is inert"
    );
}

#[test]
fn trap_leaves_the_eighth_before_its_snare_empty() {
    // Research ch. 1 §1: avoid kicks immediately before the beat-3 snare. This
    // is the gap that makes the pattern breathe.
    let trap = model("trap");
    let context = ctx(&trap, 4);
    let eighth = grid::ticks_per_beat(&context) / 2;

    for seed in 0..80u64 {
        let lanes = generate(&trap, &context, seed);
        // Measured against the snares this pattern actually has, not against
        // beat 3: trap's full-time variant moves them, and a window nailed to
        // beat 3 would report a legal kick as a violation on those seeds.
        let snares: Vec<(u32, u32)> = lane(&lanes, Lane::Snare)
            .map(|l| {
                l.notes
                    .iter()
                    .filter(|n| {
                        !matches!(
                            n.articulation,
                            Some(Articulation::Ghost) | Some(Articulation::Roll)
                        )
                    })
                    .map(|n| {
                        (
                            n.start_tick / context.ticks_per_bar(),
                            n.start_tick % context.ticks_per_bar(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Measure the fixture before measuring against it: if `snares` came
        // back empty the loop below ran zero assertions and the rule went
        // unchecked. Mislabelling every main snare left this green.
        //
        // Per bar rather than a fixed count — trap's full-time variant plays
        // two a bar and a fill can take one, so the number varies while "every
        // bar has a snare the kick must leave room for" does not.
        for bar in 0..4u32 {
            assert!(
                snares.iter().any(|(b, _)| *b == bar),
                "seed {seed}, bar {bar}: no snare to measure the gap against"
            );
        }

        for (bar, tick) in positions(&lanes, Lane::Kick, &context) {
            for (snare_bar, snare) in &snares {
                if *snare_bar != bar {
                    continue;
                }
                assert!(
                    !(tick < *snare && snare - tick <= eighth),
                    "seed {seed}, bar {bar}: kick at {tick} is inside the 8th before the snare at {snare}"
                );
            }
        }
    }
}

#[test]
fn trap_and_drill_do_not_come_out_sounding_the_same() {
    // Both are half-time 140-zone genres; what separates their drums is where
    // the kicks land. Drill leans offbeat, trap anchors on the beat.
    let (trap, drill) = (model("trap"), model("uk-drill"));
    let context = ctx(&trap, 4);

    let offbeat_share = |m: &StyleModel| {
        let (mut offbeat, mut total) = (0.0, 0.0);
        for seed in 0..60u64 {
            for (_, tick) in positions(&generate(m, &context, seed), Lane::Kick, &context) {
                if grid::is_offbeat_eighth(tick / grid::SIXTEENTH) {
                    offbeat += 1.0;
                }
                total += 1.0;
            }
        }
        offbeat / total
    };

    let trap_share = offbeat_share(&trap);
    let drill_share = offbeat_share(&drill);
    assert!(
        drill_share > trap_share + 0.1,
        "drill should lean offbeat harder than trap: {drill_share:.2} vs {trap_share:.2}"
    );
}

#[test]
fn every_shipped_model_states_a_snare_placement_the_engine_knows() {
    // A placement the parser does not recognise falls back to a backbeat, which
    // would quietly turn a half-time genre into a full-time one. Nothing else
    // in the suite would notice.
    let mut checked = 0;
    for (id, model) in shipped() {
        let Some(placement) = model
            .blocks
            .get("drums")
            .and_then(|d| d.get("snare"))
            .and_then(|s| s.get("placement"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        checked += 1;
        assert!(
            SnarePlacement::parse(placement).is_some(),
            "{id}: `{placement}` is not a placement the engine knows"
        );
    }
    assert!(
        checked > 0,
        "no model declared a placement — nothing checked"
    );
}

#[test]
fn every_grid_position_in_the_dataset_resolves_to_a_tick() {
    // The same class of failure as an unknown placement: a position the parser
    // rejects is silently skipped, so the rule it belongs to stops applying
    // while the model still looks like it says something.
    let context = SessionContext::default();
    let mut checked = 0;

    /// Every string in a JSON array must be a grid position. Returns how many
    /// it looked at, so an empty array cannot pass for a checked one.
    fn check(id: &str, where_: &str, value: &Value, ctx: &SessionContext) -> u32 {
        let mut seen = 0;
        for position in value.as_array().into_iter().flatten() {
            let Some(text) = position.as_str() else {
                continue;
            };
            assert!(
                grid::position_ticks(text, ctx).is_some(),
                "{id}: `{text}` in {where_} is not a grid position"
            );
            seen += 1;
        }
        seen
    }

    for (id, model) in shipped() {
        let Some(drums) = model.blocks.get("drums") else {
            continue;
        };

        if let Some(kick) = drums.get("kick") {
            if let Some(anchors) = kick.get("anchors") {
                checked += check(&id, "kick.anchors", anchors, &context);
            }
            if let Some(secondary) = kick.get("secondaryAnchor").and_then(Value::as_str) {
                assert!(
                    grid::position_ticks(secondary, &context).is_some(),
                    "{id}: `{secondary}` in kick.secondaryAnchor is not a grid position"
                );
                checked += 1;
            }
            for row in kick
                .get("fourBarGrammar")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                checked += check(&id, "kick.fourBarGrammar", row, &context);
            }
        }

        if let Some(pos) = drums.pointer("/snare/ghost/pos") {
            checked += check(&id, "snare.ghost.pos", pos, &context);
        }

        // Open-hat positions carry one symbolic form the grid notation does not
        // express: rage's `"1_pre"` is "just before the downbeat", which lands
        // in the *previous* bar (research ch. 1 §3). The hat engine (TASK-019)
        // owns what that resolves to; what is checkable now is that the base
        // position is real, so a typo cannot hide inside the suffix.
        for position in drums
            .pointer("/hihat/openHat/pos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let base = position.strip_suffix("_pre").unwrap_or(position);
            assert!(
                grid::position_ticks(base, &context).is_some(),
                "{id}: `{position}` in hihat.openHat.pos is not a position"
            );
            checked += 1;
        }
    }

    assert!(checked > 0, "no positions were checked");
}

#[test]
fn every_shipped_model_generates_a_playable_pattern() {
    let mut generated = 0;
    for (id, model) in shipped() {
        let context = ctx(&model, 4);
        let total = context.total_ticks();

        for seed in 0..20u64 {
            let lanes = generate(&model, &context, seed);
            assert!(
                !lanes.is_empty(),
                "{id} seed {seed}: generated nothing at all"
            );
            for track in &lanes {
                assert!(!track.notes.is_empty(), "{id}: empty {:?} lane", track.lane);
                for note in &track.notes {
                    assert!(note.start_tick < total, "{id}: note past the pattern");
                    assert!(note.vel >= 1 && note.vel <= 127, "{id}: bad velocity");
                    assert!(note.len_ticks > 0, "{id}: zero-length note");
                }
            }
            generated += 1;
        }
    }
    assert!(generated > 0, "no models were generated");
}
