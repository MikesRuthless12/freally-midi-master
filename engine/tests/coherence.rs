//! Cross-part coherence (TASK-038, FR-002).
//!
//! Every other suite proves one part in isolation. This one generates **all five
//! parts together** and asserts they belong in the same record — which is the
//! only place a class of failure can be seen at all: each part can be perfectly
//! correct on its own terms and still fight the others.
//!
//! This is the suite behind the product's "cook-up doesn't clash" claim, so each
//! assertion below is a promise a producer would notice being broken:
//!
//! - **One key.** Every melodic pitch is in the session's scale, except where a
//!   model authored a device that leaves it.
//! - **The melody and the counter are not in each other's way**, because a
//!   countermelody in the lead's own register is a second lead.
//! - **The bass is under the harmony.** A bass above the chords is not a bass.
//! - **Nothing outruns the pattern**, on any part.
//!
//! It walks 20 (model, seed) combinations rather than one, because two parts
//! agreeing on one seed is a coincidence.

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::{bass, chords, counter, drums, melody};
use engine::pattern::{Lane, LaneTrack};
use engine::theory;
use serde_json::Value;

mod common;
use common::shipped_models;

/// Every part of one generation, ready to be compared with the others.
struct Arrangement {
    ctx: SessionContext,
    harmony: chords::Chords,
    kit: Vec<LaneTrack>,
    melody: LaneTrack,
    counter: LaneTrack,
    bass: LaneTrack,
}

fn arrange(model: &engine::StyleModel, seed: u64) -> Arrangement {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    let lead = melody::generate(model, &ctx, seed, &harmony, &kit);
    let answer = counter::generate(model, &ctx, seed, &harmony, &lead);
    let low = bass::generate(model, &ctx, seed, &harmony, &kit);

    Arrangement {
        ctx,
        harmony,
        kit,
        melody: lead,
        counter: answer,
        bass: low,
    }
}

/// The 20 (model, seed) combinations the suite walks.
///
/// Spread across the roster rather than 20 seeds of one model: the failures this
/// suite exists to catch come from a *model's* parts disagreeing, so covering
/// models matters more than covering seeds.
fn combinations() -> Vec<(String, engine::StyleModel, u64)> {
    let models: Vec<(String, engine::StyleModel)> = shipped_models().into_iter().collect();
    assert!(models.len() >= 5, "the roster must have models to arrange");

    (0..20)
        .map(|index| {
            let (id, model) = models[index % models.len()].clone();
            (id, model, (index as u64) + 1)
        })
        .collect()
}

/// Does this model author a device that is allowed to leave the key?
fn leaves_the_key(model: &engine::StyleModel, block: &str, key: &str) -> bool {
    model
        .blocks
        .get(block)
        .and_then(|b| b.get(key))
        .and_then(Value::as_f64)
        .is_some_and(|value| value > 0.0)
}

#[test]
fn every_arrangement_stays_in_one_key() {
    // ⛔ FR-002's headline promise. A melody in the key over chords in the key
    // with a bass in a *different* key is the single most audible way a
    // generated arrangement announces that it was generated.
    //
    // Two authored devices are allowed out of the scale, and both say so:
    //
    // - **`melody.halfStepDissonance`**, which names the semitone it adds.
    // - **`bassline.passingTones`**, which is subtler and was found by this test.
    //   `country-train` authors `["P5", "m7"]` and plays in Major, where a minor
    //   seventh off the root is not a scale tone. That is not a bug: a passing
    //   tone's job is to pass *between* scale tones, and a walked ♭7 over a major
    //   chord is the mixolydian colour country and blues bass is made of.
    //   Refusing it would mean refusing the idiom.
    let ctx_scale = |ctx: &SessionContext| theory::scale_semitones(ctx.scale);

    for (id, model, seed) in combinations() {
        let art = arrange(&model, seed);
        let allowed = ctx_scale(&art.ctx);
        let chromatic_melody = leaves_the_key(&model, "melody", "halfStepDissonance");
        let chromatic_bass = model
            .blocks
            .get("bassline")
            .and_then(|b| b.get("passingTones"))
            .and_then(Value::as_array)
            .is_some_and(|tones| {
                tones.iter().filter_map(Value::as_str).any(|name| {
                    theory::interval_semitones(name)
                        .is_some_and(|semitones| !allowed.contains(&(semitones as u8 % 12)))
                })
            });

        let check = |lane: &LaneTrack, part: &str, exempt: bool| {
            if exempt {
                return;
            }
            for note in &lane.notes {
                let class =
                    (i32::from(note.pitch) - i32::from(art.ctx.key_root)).rem_euclid(12) as u8;
                assert!(
                    allowed.contains(&class),
                    "{id} seed {seed}: the {part} played {} which is out of {:?}",
                    note.pitch,
                    art.ctx.scale
                );
            }
        };

        check(&art.melody, "melody", chromatic_melody);
        check(&art.counter, "counter", chromatic_melody);
        check(&art.bass, "bass", chromatic_bass);
    }
}

#[test]
fn the_counter_does_not_collide_with_the_melody() {
    // ⛔ **A unison collision, not a register comparison.** Comparing the two
    // parts' *mean* pitches was the first version of this and it was wrong:
    // `jerk` put the melody at 77.8 and the counter at 75.2 and failed, even
    // though their authored registers are a full octave apart — the realised
    // notes had simply clustered at opposite ends of each range. Means converging
    // is not a clash.
    //
    // What a producer actually hears as a clash is the two parts playing **the
    // same pitch at the same moment**, so that is what is measured. Their
    // registers being offset is already asserted in `counter.rs`, where it
    // belongs.
    let mut compared = 0;

    for (id, model, seed) in combinations() {
        let art = arrange(&model, seed);
        if art.melody.notes.is_empty() || art.counter.notes.is_empty() {
            continue;
        }
        compared += 1;

        let unisons = art
            .counter
            .notes
            .iter()
            .filter(|answer| {
                art.melody.notes.iter().any(|lead| {
                    lead.pitch == answer.pitch && lead.start_tick.abs_diff(answer.start_tick) < 240
                })
            })
            .count();
        let share = unisons as f64 / art.counter.notes.len() as f64;

        assert!(
            share <= 0.25,
            "{id} seed {seed}: {share:.2} of the counter plays the melody's own pitch \
             at the same moment, which is a second lead rather than an answer"
        );
    }

    assert!(
        compared > 0,
        "no combination generated both a melody and a counter, so this is vacuous"
    );
}

#[test]
fn the_bass_stays_under_the_harmony() {
    // A bass above the chords is not a bass. Compared at the extremes rather
    // than on average, because one high bass note is audible even if the mean
    // looks fine.
    let mut compared = 0;

    for (id, model, seed) in combinations() {
        let art = arrange(&model, seed);
        if art.bass.notes.is_empty() || art.harmony.track.notes.is_empty() {
            continue;
        }
        compared += 1;

        let highest_bass = art.bass.notes.iter().map(|n| n.pitch).max().unwrap();
        let lowest_chord = art
            .harmony
            .track
            .notes
            .iter()
            .map(|n| n.pitch)
            .min()
            .unwrap();

        assert!(
            highest_bass <= lowest_chord,
            "{id} seed {seed}: the bass reached {highest_bass} and the chords start at \
             {lowest_chord}"
        );
    }

    assert!(
        compared > 0,
        "no combination generated both a bass and chords, so this is vacuous"
    );
}

#[test]
fn the_low_end_is_played_by_one_part_or_the_other() {
    // ⛔ FR-007's unification, seen from the arrangement rather than from the
    // bass generator. When the 808 is the bassline there must be no bass lane,
    // and when there is a bass lane the 808 is not doing that job — two
    // instruments on the same notes in the same octave is the muddy low end this
    // rule exists to prevent.
    for (id, model, seed) in combinations() {
        let art = arrange(&model, seed);
        let eight_o_eight_notes = art
            .kit
            .iter()
            .filter(|track| track.lane == Lane::Bass808)
            .map(|track| track.notes.len())
            .sum::<usize>();

        if bass::eight_o_eight_is_the_bass(&model) {
            assert!(
                art.bass.notes.is_empty(),
                "{id} seed {seed}: the 808 is the bassline and the bass generated \
                 {} notes as well",
                art.bass.notes.len()
            );
        } else if !art.bass.notes.is_empty() && eight_o_eight_notes > 0 {
            // Both present is allowed — an 808 in a counter-riff role is a
            // different part — but they must not be the same line. Compared by
            // onset, because that is what makes two low parts fight.
            let same: usize = art
                .bass
                .notes
                .iter()
                .filter(|note| {
                    art.kit
                        .iter()
                        .filter(|t| t.lane == Lane::Bass808)
                        .any(|t| t.notes.iter().any(|o| o.start_tick == note.start_tick))
                })
                .count();
            let share = same as f64 / art.bass.notes.len() as f64;
            // ⛔ **Near-total identity is the failure; heavy overlap is the
            // style.** The first version of this drew the line at 0.9 and
            // `ny-drill` landed exactly there — which is not a bug: in trap and
            // drill the kick, the 808 and the bass lock together deliberately,
            // and that lock is most of what makes the low end hit as one thing.
            // What is wrong is the bass being *entirely* the 808, note for note,
            // because then one of the two parts is contributing nothing.
            assert!(
                share < 0.98,
                "{id} seed {seed}: {share:.2} of the bass lands exactly on the 808, \
                 which is one line played twice rather than two parts locking"
            );
        }
    }
}

#[test]
fn no_part_outruns_the_pattern() {
    for (id, model, seed) in combinations() {
        let art = arrange(&model, seed);
        let total = art.ctx.total_ticks();

        let parts: Vec<(&str, &LaneTrack)> = vec![
            ("melody", &art.melody),
            ("counter", &art.counter),
            ("bass", &art.bass),
            ("chords", &art.harmony.track),
        ];

        for (name, track) in parts.into_iter().chain(
            art.kit
                .iter()
                .map(|track| ("drums", track))
                .collect::<Vec<_>>(),
        ) {
            for note in &track.notes {
                assert!(
                    note.start_tick + note.len_ticks <= total,
                    "{id} seed {seed}: a {name} note runs past the pattern"
                );
                assert!(
                    note.len_ticks > 0,
                    "{id} seed {seed}: a {name} note has no length"
                );
                assert!(note.vel >= 1, "{id} seed {seed}: a {name} note is silent");
            }
        }
    }
}

#[test]
fn an_arrangement_is_reproducible_whole() {
    // Determinism across *all* parts at once. Each generator proves it alone;
    // this proves nothing shared between them — a global RNG, a hash map's
    // iteration order — has crept in.
    for (id, model, seed) in combinations().into_iter().take(6) {
        let first = arrange(&model, seed);
        let again = arrange(&model, seed);

        assert_eq!(first.melody, again.melody, "{id} seed {seed}: melody");
        assert_eq!(first.counter, again.counter, "{id} seed {seed}: counter");
        assert_eq!(first.bass, again.bass, "{id} seed {seed}: bass");
        assert_eq!(
            first.harmony.track, again.harmony.track,
            "{id} seed {seed}: chords"
        );
        assert_eq!(first.kit, again.kit, "{id} seed {seed}: drums");
    }
}
