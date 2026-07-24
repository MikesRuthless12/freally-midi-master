//! The edges: meters other than 4/4, patterns of one bar, and the ids the
//! roster is supposed to hide.
//!
//! Every other suite drives 4/4 at four bars, because that is what the models
//! are written for. These drive the corners — and the first one they found was
//! real: a 2-and-4 backbeat in 3/4 wrote its "beat 4" at tick 2880, which is
//! the downbeat of the *next* bar, and in the final bar it escaped the pattern
//! altogether. `SnarePlacement::hits` now drops beats the meter does not have,
//! the same rule `grid::position_ticks` applies to authored positions.
use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::StyleModel;
use serde_json::json;

fn m(drums: serde_json::Value) -> StyleModel {
    serde_json::from_value(json!({"id":"t","type":"genre","name":"T","drums":drums})).unwrap()
}

#[test]
fn every_meter_keeps_its_notes_inside_the_pattern() {
    let model = m(json!({
        "kick": {"anchors":["1"],"densityPerBar":4,"syncopation":0.6,"lockTo808":1.0},
        "snare": {"placement":"backbeat_24","ghost":{"prob":0.5,"pos":["4&"]}},
        "hihat": {"base":"16th","fillDensity":0.5,
                  "rolls":{"vocab":["16","32"],"positions":["phrase_end","pre_snare"],"freqPerBar":1.0},
                  "openHat":{"prob":0.5,"pos":["2&"],"perBar":1}},
        "fills": {"smallEveryBars":1,"bigEveryBars":2},
        "bass808": {"role":"bassline","register":[17,31],"slideProb":1.0,
                    "slidePositions":["phrase_end"],"slideIntervals":["P5","P8"]}
    }));

    for (num, den) in [(4u8, 4u8), (3, 4), (6, 8), (5, 4), (7, 8)] {
        for bars in [1u16, 2, 3, 4, 8] {
            for seed in 0..12u64 {
                let ctx = SessionContext {
                    time_sig_num: num,
                    time_sig_den: den,
                    bars,
                    ..Default::default()
                };
                let total = ctx.total_ticks();
                let lanes = generate(&model, &ctx, seed);
                for track in &lanes {
                    let mut prev = 0;
                    for n in &track.notes {
                        assert!(
                            n.start_tick < total,
                            "{num}/{den} {bars}bar seed {seed}: {:?} at {} >= total {total}",
                            track.lane,
                            n.start_tick
                        );
                        assert!(n.len_ticks > 0, "{num}/{den}: zero-length note");
                        assert!(
                            n.vel >= 1 && n.vel <= 127,
                            "{num}/{den}: velocity {}",
                            n.vel
                        );
                        assert!(
                            n.start_tick >= prev,
                            "{num}/{den}: {:?} out of order",
                            track.lane
                        );
                        prev = n.start_tick;
                    }
                }
            }
        }
    }
}

#[test]
fn a_model_whose_id_merely_contains_an_underscore_stays_in_the_roster() {
    let files = vec![
        (
            std::path::PathBuf::from("a.json"),
            r#"{"id":"_defaults","type":"genre","name":"D"}"#.to_string(),
        ),
        (
            std::path::PathBuf::from("b.json"),
            r#"{"id":"lo_fi","type":"genre","name":"Lo Fi"}"#.to_string(),
        ),
        (
            std::path::PathBuf::from("c.json"),
            r#"{"id":"_private","type":"genre","name":"P"}"#.to_string(),
        ),
    ];
    let loaded = engine::dataset::load("t", files);
    let ids: Vec<&str> = loaded
        .summary
        .entries
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, vec!["lo_fi"], "only a LEADING underscore is internal");
}

/// A counter-riff 808 must stay in the register it was given.
///
/// It did not: the slide ceiling was measured from the *running* pitch, so
/// each slide raised the note and its own ceiling together and the line
/// ratcheted — uk-drill walked 24 → 31 → 38 → 50 → 60 → 70, three and a half
/// octaves above its authored ceiling of 28, on 28% of its notes.
#[test]
fn a_counter_riff_808_never_ratchets_out_of_its_register() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data");
    let scan = engine::dataset::files::scan(&dir).unwrap();
    let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();

    for id in ["uk-drill", "ny-drill", "trap", "phonk", "plugg"] {
        let model = models.get(id).unwrap();
        let register = model.blocks["drums"]["bass808"]
            .get("register")
            .and_then(|r| r.as_array())
            .map(|a| (a[0].as_u64().unwrap() as u8, a[1].as_u64().unwrap() as u8))
            .unwrap();
        // An octave of headroom above the **root** is legal — that is what
        // makes an octave glide reachable, and it is a fixed bound. The bug
        // was that the bound moved with the line, so what this test really
        // asserts is that a *constant* ceiling holds however many slides
        // happen. Computed here with plain arithmetic rather than by calling
        // the engine's own helper.
        let (lo, hi) = register;
        let root = lo + ((12 - lo % 12) % 12); // key C is the default
        let ceiling = hi.max(root + 12);

        for bars in [4u16, 8, 16] {
            let ctx = SessionContext {
                bars,
                ..Default::default()
            };
            for seed in 0..200u64 {
                for track in generate(model, &ctx, seed) {
                    if track.lane != engine::pattern::Lane::Bass808 {
                        continue;
                    }
                    for n in &track.notes {
                        assert!(
                            n.pitch <= ceiling,
                            "{id} {bars}bar seed {seed}: 808 at {} is above the {ceiling} ceiling",
                            n.pitch
                        );
                        if let Some(target) = n.slide_to_pitch {
                            assert!(
                                target <= ceiling,
                                "{id} seed {seed}: slide target {target} is above {ceiling}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Two notes on one key at one tick is the collision the SMF note-off pairing
/// cannot survive — the second off is orphaned and the hit doubles.
///
/// Fills used to write straight over the ghost notes `clear_for_fill`
/// deliberately keeps, because both sit on the 16th grid. Eleven of the
/// fifteen genres produced these.
#[test]
fn no_lane_ever_carries_two_notes_on_the_same_tick() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data");
    let scan = engine::dataset::files::scan(&dir).unwrap();
    let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();

    for (id, model) in &models {
        for bars in [4u16, 8] {
            let ctx = SessionContext {
                bars,
                ..Default::default()
            };
            for seed in 0..50u64 {
                for track in generate(model, &ctx, seed) {
                    let mut seen = std::collections::BTreeSet::new();
                    for n in &track.notes {
                        assert!(
                            seen.insert(n.start_tick),
                            "{id} {bars}bar seed {seed}: two {:?} notes on tick {}",
                            track.lane,
                            n.start_tick
                        );
                    }
                }
            }
        }
    }
}

/// A velocity outside 1..=127 is not a MIDI velocity, whatever a model says.
#[test]
fn an_out_of_range_velocity_tier_is_clamped_rather_than_shipped() {
    let m = m(json!({
        "velocityTiers": { "main": [130, 140], "ghost": [0, 0], "accent": [200, 250] },
        "kick": {"anchors":["1"],"densityPerBar":4},
        "snare": {"placement":"train_16ths"}
    }));
    let ctx = SessionContext {
        bars: 2,
        ..Default::default()
    };
    for seed in 0..20u64 {
        for track in generate(&m, &ctx, seed) {
            for n in &track.notes {
                assert!(
                    n.vel >= 1 && n.vel <= 127,
                    "seed {seed}: {:?} velocity {} is not a MIDI velocity",
                    track.lane,
                    n.vel
                );
            }
        }
    }
}
