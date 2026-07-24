//! The humanizer against the data that ships and across many seeds.
//!
//! The unit tests in `engine/src/humanize.rs` prove each knob in isolation.
//! These prove the two claims that only hold end to end: the swing table
//! matches the MPC scale the research documents, and the models in `data/` say
//! things the engine can actually act on.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use engine::context::{Humanize, SessionContext, Swing, SwingGrid};
use engine::humanize::{humanize, ramp, swing_tick, VelocityTiers};
use engine::pattern::{Articulation, Lane, LaneTrack, Note, PPQ};
use engine::rng;
use serde_json::Value;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model, resolved.
fn shipped_models() -> BTreeMap<String, engine::StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let registry = engine::dataset::registry_from(scan.files);
    let (models, errors) = registry.resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

fn note(start: u32, len: u32, vel: u8) -> Note {
    Note {
        start_tick: start,
        len_ticks: len,
        pitch: 36,
        vel,
        slide_to_pitch: None,
        articulation: None,
    }
}

fn sixteenth(amount: f32) -> Swing {
    Swing {
        grid: SwingGrid::Sixteenth,
        amount,
    }
}

#[test]
fn swing_offsets_match_the_mpc_scale() {
    // The scale as the hardware states it, in ticks at PPQ 960: the second 16th
    // of each pair is delayed by `(amount - 50%)` of the 480-tick pair.
    //
    // 66% is the number the MPC prints; 2/3 is the exact triplet, and the two
    // are one tick apart at this resolution — which is the whole reason PPQ is
    // 960 rather than 480.
    let cases = [
        (0.50, 240, "straight"),
        (0.54, 259, "subtle groove"),
        (0.58, 278, "classic MPC boom bap"),
        (0.62, 298, "heavy shuffle"),
        (0.66, 317, "MPC's 66%"),
        (0.6667, 320, "the exact triplet"),
    ];

    for (amount, expected, name) in cases {
        let got = swing_tick(240, sixteenth(amount));
        assert_eq!(got, expected, "{name} ({amount}) put the offbeat at {got}");
        // ...and the downbeat either side of it never moves.
        assert_eq!(swing_tick(0, sixteenth(amount)), 0);
        assert_eq!(swing_tick(480, sixteenth(amount)), 480);
    }
}

#[test]
fn the_swing_delay_is_proportional_to_the_amount() {
    // Not just ordered — evenly spaced, which is what makes the percentage on
    // the dial mean something.
    let delay = |amount: f32| swing_tick(240, sixteenth(amount)) as i32 - 240;
    let steps: Vec<i32> = [0.50, 0.54, 0.58, 0.62, 0.66]
        .iter()
        .map(|a| delay(*a))
        .collect();

    for pair in steps.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (18..=20).contains(&gap),
            "4% of a 480-tick pair is 19.2 ticks, got {gap} in {steps:?}"
        );
    }
}

#[test]
fn velocity_distributions_stay_in_their_tiers_across_a_hundred_seeds() {
    let tiers = VelocityTiers::default();

    for seed in 0..100u64 {
        let mut stream = rng::stream(seed, "test/velocity");
        let mut ghosts = Vec::new();
        let mut mains = Vec::new();
        let mut accents = Vec::new();

        for _ in 0..64 {
            ghosts.push(tiers.pick(Some(Articulation::Ghost), &mut stream));
            mains.push(tiers.pick(None, &mut stream));
            accents.push(tiers.pick(Some(Articulation::Accent), &mut stream));
        }

        let span = |v: &[u8]| (*v.iter().min().unwrap(), *v.iter().max().unwrap());
        let (ghost_lo, ghost_hi) = span(&ghosts);
        let (main_lo, main_hi) = span(&mains);
        let (accent_lo, accent_hi) = span(&accents);

        // The tiers never overlap: the loudest ghost in 64 draws is still
        // quieter than the quietest main. A ghost note that can read as a main
        // hit is not a ghost note.
        assert!(ghost_hi < main_lo, "seed {seed}: {ghost_hi} >= {main_lo}");
        assert!(main_hi < accent_lo, "seed {seed}: {main_hi} >= {accent_lo}");

        // And each tier actually uses its band rather than collapsing onto one
        // value — flat velocity is the thing humanization exists to prevent.
        assert!(ghost_hi > ghost_lo, "seed {seed}: ghosts are flat");
        assert!(main_hi > main_lo, "seed {seed}: mains are flat");
        assert!(accent_hi > accent_lo, "seed {seed}: accents are flat");

        assert!(ghost_lo >= tiers.ghost.lo && ghost_hi <= tiers.ghost.hi);
        assert!(main_lo >= tiers.main.lo && main_hi <= tiers.main.hi);
        assert!(accent_lo >= tiers.accent.lo && accent_hi <= tiers.accent.hi);
    }
}

#[test]
fn every_shipped_model_declares_humanization_the_engine_can_act_on() {
    // The failure this exists for: `timingJitterMs` is authored with string
    // keys, so a lane name that does not exist — "hats", "hihat", "808" — is
    // valid JSON, passes the schema, and is then silently dropped on the way
    // into the engine. The model would look humanized and play robotic.
    let mut checked = 0;

    for (id, model) in shipped_models() {
        let Some(session) = &model.session else {
            continue;
        };
        let Some(humanize) = &session.humanize else {
            continue;
        };
        checked += 1;

        if let Some(strength) = humanize.quantize_strength {
            assert!(
                (0.0..=1.0).contains(&strength),
                "{id}: quantizeStrength {strength} is not a fraction"
            );
        }
        if let Some(var) = humanize.velocity_var {
            assert!(
                (0.0..=1.0).contains(&var),
                "{id}: velocityVar {var} is not a fraction"
            );
        }

        for (lane, ms) in &humanize.timing_jitter_ms {
            let parsed: Result<Lane, _> = serde_json::from_value(Value::String(lane.clone()));
            assert!(
                parsed.is_ok(),
                "{id}: `{lane}` is not a lane the engine knows, so its jitter would be ignored"
            );
            assert!(
                *ms >= 0.0 && *ms < 200.0,
                "{id}: {lane} jitter of {ms} ms is not a human hand"
            );
        }
    }

    assert!(
        checked > 0,
        "no model declared a humanize block — this test checked nothing"
    );
}

#[test]
fn every_shipped_models_swing_is_a_swing_the_engine_applies() {
    let mut checked = 0;

    for (id, model) in shipped_models() {
        let Some(swing) = model.session.as_ref().and_then(|s| s.swing.as_ref()) else {
            continue;
        };
        checked += 1;

        assert!(
            (0.5..=0.75).contains(&swing.amount),
            "{id}: swing {} is outside the MPC scale",
            swing.amount
        );
        assert!(
            matches!(swing.grid.as_str(), "8th" | "16th"),
            "{id}: swing grid `{}` is neither 8th nor 16th",
            swing.grid
        );
    }

    assert!(checked > 0, "no model declared swing — nothing was checked");
}

#[test]
fn the_shipped_velocity_tiers_keep_ghosts_ghosts() {
    // Whatever a genre reshapes, the ordering has to survive: a model whose
    // ghost band overlaps its main band has lost the distinction the whole
    // articulation system rests on.
    for (id, model) in shipped_models() {
        let tiers = VelocityTiers::from_json(model.blocks.get("drums"));
        assert!(
            tiers.ghost.hi < tiers.main.lo,
            "{id}: ghost band {:?} overlaps main {:?}",
            tiers.ghost,
            tiers.main
        );
        assert!(
            tiers.main.hi <= tiers.accent.lo + 12,
            "{id}: main band {:?} swallows accents {:?}",
            tiers.main,
            tiers.accent
        );
    }
}

#[test]
fn trap_is_humanized_by_velocity_and_not_by_timing() {
    // Research ch. 1 §1: "Swing: 50% straight default; humanization via velocity
    // variance, not timing." That is a claim about the shipped trap model, and
    // it is checkable — the notes must come back on the grid while the
    // velocities move.
    let models = shipped_models();
    let trap = models.get("trap").expect("trap must ship");
    let session = trap.session.as_ref().unwrap();

    let swing = session.swing.as_ref().unwrap();
    assert_eq!(swing.amount, 0.5, "trap is straight");

    let humanize_spec = session.humanize.as_ref().unwrap();
    let mut ctx = SessionContext {
        swing: sixteenth(swing.amount as f32),
        ..Default::default()
    };
    ctx.humanize = Humanize {
        quantize_strength: humanize_spec.quantize_strength.unwrap_or(0.92) as f32,
        velocity_var: humanize_spec.velocity_var.unwrap_or(0.12) as f32,
        timing_jitter_ms: humanize_spec
            .timing_jitter_ms
            .iter()
            .filter_map(|(lane, ms)| {
                serde_json::from_value::<Lane>(Value::String(lane.clone()))
                    .ok()
                    .map(|lane| (lane, *ms as f32))
            })
            .collect(),
    };

    let grid: Vec<u32> = (0..16).map(|i| i * 240).collect();
    let hats = || LaneTrack {
        lane: Lane::ClosedHat,
        notes: grid.iter().map(|t| note(*t, 60, 100)).collect(),
    };

    let drift = |ctx: &SessionContext, seed: u64| {
        let mut lanes = vec![hats()];
        humanize(&mut lanes, ctx, seed);
        let worst = lanes[0]
            .notes
            .iter()
            .zip(&grid)
            .map(|(n, g)| (n.start_tick as i64 - *g as i64).unsigned_abs())
            .max()
            .unwrap();
        (
            worst,
            lanes[0].notes.iter().map(|n| n.vel).collect::<Vec<u8>>(),
        )
    };

    // Trap's own numbers: 3 ms of hand at 92% quantize. Not literally zero —
    // a hit can land one tick out — but one tick is 0.45 ms at 140 BPM, which
    // is under the threshold of hearing and under any DAW's display grid.
    let (worst, velocities) = drift(&ctx, 1234);
    assert!(
        worst <= 1,
        "trap's drums drifted {worst} ticks ({:.2} ms)",
        worst as f32 * ctx.ms_per_tick()
    );
    assert!(
        velocities.iter().any(|v| *v != 100),
        "trap's life comes from velocity: {velocities:?}"
    );

    // The contrast, so "on the grid" is a measurement and not a tautology: the
    // same jitter at a neo-soul quantize strength moves the hats audibly.
    let mut loose = ctx.clone();
    loose.humanize.quantize_strength = 0.3;
    loose
        .humanize
        .timing_jitter_ms
        .insert(Lane::ClosedHat, 25.0);
    let (loose_worst, _) = drift(&loose, 1234);
    assert!(
        loose_worst > 20,
        "a loosely quantized session should be audibly off the grid, got {loose_worst} ticks"
    );
}

#[test]
fn a_full_pattern_survives_a_hundred_seeds() {
    let ctx = SessionContext {
        swing: sixteenth(0.58),
        humanize: Humanize {
            quantize_strength: 0.6,
            velocity_var: 0.2,
            timing_jitter_ms: [
                (Lane::Kick, 2.0),
                (Lane::Snare, 4.0),
                (Lane::ClosedHat, 6.0),
                (Lane::Bass808, 2.0),
            ]
            .into_iter()
            .collect(),
        },
        bars: 4,
        ..Default::default()
    };
    let total = ctx.total_ticks();

    for seed in 0..100u64 {
        let mut lanes = vec![
            LaneTrack {
                lane: Lane::Kick,
                notes: (0..8).map(|i| note(i * 480, 120, 96)).collect(),
            },
            LaneTrack {
                lane: Lane::Snare,
                notes: (0..4).map(|i| note(960 + i * 1920, 120, 110)).collect(),
            },
            LaneTrack {
                lane: Lane::ClosedHat,
                notes: (0..64).map(|i| note(i * 240, 90, 80)).collect(),
            },
            LaneTrack {
                lane: Lane::Bass808,
                notes: (0..8).map(|i| note(i * 480, 480, 100)).collect(),
            },
        ];
        let before: Vec<usize> = lanes.iter().map(|l| l.notes.len()).collect();

        humanize(&mut lanes, &ctx, seed);

        for (track, count) in lanes.iter().zip(&before) {
            assert_eq!(track.notes.len(), *count, "seed {seed}: notes went missing");

            let mut previous = 0;
            for note in &track.notes {
                assert!(
                    note.vel >= 1 && note.vel <= 127,
                    "seed {seed}: bad velocity"
                );
                assert!(note.len_ticks >= 1, "seed {seed}: zero-length note");
                assert!(
                    note.start_tick >= previous,
                    "seed {seed}: {:?} came back out of order",
                    track.lane
                );
                // A humanized note may sit a hair past the last bar line — that
                // is a late hit, not a bug — but it must not run away.
                assert!(
                    note.start_tick < total + PPQ,
                    "seed {seed}: note at {} escaped the pattern",
                    note.start_tick
                );
                previous = note.start_tick;
            }
        }
    }
}

#[test]
fn a_hat_roll_ramps_into_its_target_beat() {
    // Research ch. 1 §1: rolls ramp 50→100% into the target beat. The ramp is
    // applied before humanization, and must still be climbing after it.
    let ctx = SessionContext {
        humanize: Humanize {
            quantize_strength: 0.9,
            velocity_var: 0.05,
            timing_jitter_ms: BTreeMap::new(),
        },
        ..Default::default()
    };

    let mut roll: Vec<Note> = (0..8).map(|i| note(1680 + i * 30, 30, 100)).collect();
    ramp(&mut roll, 64, 127);
    let mut lanes = vec![LaneTrack {
        lane: Lane::ClosedHat,
        notes: roll,
    }];
    humanize(&mut lanes, &ctx, 77);

    let velocities: Vec<u8> = lanes[0].notes.iter().map(|n| n.vel).collect();
    assert!(
        velocities.last().unwrap() > velocities.first().unwrap(),
        "the roll should arrive louder than it started: {velocities:?}"
    );
    // Small velocity variance must not turn the ramp into noise.
    let midpoint = velocities[4];
    assert!(
        midpoint > velocities[0] && midpoint < *velocities.last().unwrap(),
        "the middle of the ramp lost its shape: {velocities:?}"
    );
}

#[test]
fn the_defaults_file_is_the_source_of_the_engines_tier_constants() {
    // `VelocityTiers::default()` and `data/_defaults.json` state the same
    // research constants in two places. They are allowed to — the engine must
    // work with no dataset at all — but they are not allowed to disagree.
    let text = fs::read_to_string(data_dir().join("_defaults.json")).unwrap();
    let defaults: Value = serde_json::from_str(&text).unwrap();
    let authored = VelocityTiers::from_json(defaults.get("drums"));

    assert_eq!(
        authored,
        VelocityTiers::default(),
        "data/_defaults.json and VelocityTiers::default() have drifted"
    );
}
