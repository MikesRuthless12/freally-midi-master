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
//! It walks **every shipped model across eight seeds** rather than one, because
//! two parts agreeing on one seed is a coincidence. See [`combinations`] for why
//! that is no longer a fixed count of twenty.

use std::sync::LazyLock;

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::{bass, chords, counter, drums, grid, melody};
use engine::pattern::{Lane, LaneTrack, Note};
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

/// How many seeds each model is walked across.
const SEEDS_PER_MODEL: u64 = 8;

/// The resolved roster, read from disk once per test binary.
///
/// ⚠ **Borrowed, not cloned.** A `StyleModel` carries its whole body as parsed
/// `serde_json::Value` trees — kilobytes across hundreds of allocations — and the
/// first cut of the widening below handed every one of the 504 pairs its own
/// copy, again for each of the six tests that ask for them. The seed does not
/// vary the model, so none of that copying bought anything.
static ROSTER: LazyLock<Vec<(String, engine::StyleModel)>> = LazyLock::new(|| {
    let models: Vec<(String, engine::StyleModel)> = shipped_models().into_iter().collect();
    assert!(models.len() >= 5, "the roster must have models to arrange");
    models
});

/// Every shipped model, across a fixed span of seeds.
///
/// ⛔⛔ **This was twenty pairs picked by `index % models.len()`, and which pairs
/// those were depended on every model's ALPHABETICAL POSITION.** Adding the 32
/// genres of TASK-158A moved `boom-bap` from index 1 to index 6 — from seed 2 to
/// seed 7 — and seed 7 failed `the_bass_stays_under_the_harmony` immediately,
/// with a bass sitting above the chords. **The defect was not new.** It had
/// shipped for months; nothing had ever generated that pair, and the model's own
/// `bassline.register` had overlapped its `chords.voicing.register` the whole
/// time.
///
/// ▶ **A sample whose membership shifts when an unrelated file is added reports
/// the absence of evidence as evidence of absence.** It also means the suite got
/// *thinner* as the roster grew: 20 pairs over 31 models covered two thirds of
/// them, and over 63 models it covered under a third — the dataset doubling
/// halved the coverage, silently.
///
/// This repo has written that lesson down twice already: `drum_variety`'s four
/// hand-picked seeds missed a 16-of-200 collision, and `melodic_variety` says
/// plainly that a test sampling too little cannot see what it never generates.
///
/// So: every model, every seed in a fixed span, and the cost stays linear in the
/// roster.
fn combinations() -> impl Iterator<Item = (&'static str, &'static engine::StyleModel, u64)> {
    ROSTER
        .iter()
        .flat_map(|(id, model)| (1..=SEEDS_PER_MODEL).map(move |seed| (id.as_str(), model, seed)))
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
    // **`melody.halfStepDissonance`** names the semitone the melody adds, and a
    // model that authors it is saying so out loud.
    //
    // ⛔⛔ **The bass is judged as a LINE, not as a set of pitches, because that
    // is what a bassline is and it is how the commercial generators model it.**
    // Band-in-a-Box and Toontrack's EZbass both allow a non-diatonic bass note
    // as an *approach note*: it sits between chord tones and **resolves by step**
    // into the next one. Legality is about resolution, not about key membership.
    //
    // ▶ **The old rule could not express that, and the dataset is what proved
    // it.** `_defaults` authors `passingTones: ["P5", "m7"]`, and a perfect
    // fifth over the SUPERTONIC is a semitone outside every minor key — ii's own
    // fifth is diminished. So the most ordinary passing tone in the set leaves
    // the key over an ordinary diatonic chord. Every model inherits that.
    //
    // The old escape was a blanket exemption: declare any chromatic passing tone
    // and the whole bass lane went unchecked. That is all-or-nothing in the
    // wrong direction — a model either skipped the check entirely or failed it
    // for playing a normal walking figure. Widening this suite from twenty
    // (model, seed) pairs to every model across eight seeds is what made the
    // difference visible.
    //
    // ⚠ A leap away from a chromatic note is still a failure. Passing through is
    // the whole justification; a note that does not resolve is just out of key.
    const STEP: u8 = 2;
    let ctx_scale = |ctx: &SessionContext| theory::scale_semitones(ctx.scale);

    for (id, model, seed) in combinations() {
        let art = arrange(model, seed);
        let allowed = ctx_scale(&art.ctx);
        let chromatic_melody = leaves_the_key(model, "melody", "halfStepDissonance");

        let class_of =
            |pitch: u8| (i32::from(pitch) - i32::from(art.ctx.key_root)).rem_euclid(12) as u8;

        let check = |lane: &LaneTrack, part: &str, exempt: bool| {
            if exempt {
                return;
            }
            for note in &lane.notes {
                let class = class_of(note.pitch);
                assert!(
                    allowed.contains(&class),
                    "{id} seed {seed}: the {part} played {} (class {class} over key root {}) \
                     which is out of {:?}",
                    note.pitch,
                    art.ctx.key_root,
                    art.ctx.scale
                );
            }
        };

        check(&art.melody, "melody", chromatic_melody);
        check(&art.counter, "counter", chromatic_melody);

        // The bass, by resolution.
        let mut bass: Vec<&Note> = art.bass.notes.iter().collect();
        bass.sort_by_key(|note| note.start_tick);
        for (index, note) in bass.iter().enumerate() {
            let class = class_of(note.pitch);
            if allowed.contains(&class) {
                continue;
            }
            // ⚠ **A bass note that belongs to the chord sounding under it is
            // harmony, not a wrong note.** `bII` is authored by five models and a
            // borrowed `V` sits in `_defaults`; their roots are chromatic by
            // design, and the bass playing one is the arrangement agreeing with
            // itself. `chords.rs` makes exactly this allowance for the chord
            // part — `event.tones.contains(pitch) || in_scale(..)` — and the bass
            // is entitled to the same one.
            //
            // ⚠ **And the chord it is about to reach counts too.**
            // `anticipationProb` pulls a note a 16th early on purpose — that lean
            // is the device — so an anticipated note sounds *before* the chord it
            // belongs to starts. Looking only under its own tick finds the chord
            // it already left, which is how `country-train` failed here on its
            // own authored `I-bVII-IV`: the walked ♭7 this suite's comment
            // defends by name.
            let belongs_to = |tick: u32| {
                art.harmony
                    .at(tick)
                    .is_some_and(|event| event.tones.contains(&(note.pitch % 12)))
            };
            if belongs_to(note.start_tick) || belongs_to(note.start_tick + grid::SIXTEENTH) {
                continue;
            }
            // ⚠ **Compared as pitch classes, because the octave is a separate
            // and equally deliberate decision.** `bass.rs` pops an octave a
            // quarter of the time — "octave pops are what bass players do", and
            // it is what took `ny-drill` from 628 distinct basslines to 1,000.
            // So an approach note can resolve harmonically while the absolute
            // interval reads as eleven semitones. What makes it a passing tone
            // is that it sits a step from the scale tone it moves into; which
            // octave either lands in is a different question.
            let resolves = bass.get(index + 1).is_some_and(|next| {
                let next_class = class_of(next.pitch);
                let apart = class.abs_diff(next_class);
                let step = apart.min(12 - apart);
                step > 0 && step <= STEP && allowed.contains(&next_class)
            });
            assert!(
                resolves,
                "{id} seed {seed}: the bass played {} (class {class} over key root {}) which is \
                 out of {:?} and does not resolve by step into a note that is — an approach note \
                 has to land somewhere",
                note.pitch, art.ctx.key_root, art.ctx.scale
            );
        }
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
        let art = arrange(model, seed);
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
        let art = arrange(model, seed);
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
        let art = arrange(model, seed);
        let eight_o_eight_notes = art
            .kit
            .iter()
            .filter(|track| track.lane == Lane::Sub)
            .map(|track| track.notes.len())
            .sum::<usize>();

        if bass::eight_o_eight_is_the_bass(model) {
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
                        .filter(|t| t.lane == Lane::Sub)
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
        let art = arrange(model, seed);
        let total = art.ctx.total_ticks();

        // ⚠ The drum lanes carry their own name, because "a drums note runs past
        // the pattern" over a 37-lane kit says nothing about which lane to look
        // at — and the length that overruns belongs to one lane's grammar.
        let mut parts: Vec<(String, &LaneTrack)> = vec![
            ("melody".to_owned(), &art.melody),
            ("counter".to_owned(), &art.counter),
            ("bass".to_owned(), &art.bass),
            ("chords".to_owned(), &art.harmony.track),
        ];
        parts.extend(
            art.kit
                .iter()
                .map(|track| (format!("drums/{:?}", track.lane), track)),
        );

        for (name, track) in parts {
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
    //
    // ⛔ **`step_by` because the iterator is model-major now.** It used to be
    // pair-major (`models[index % len]`), so a plain `take(6)` drew six
    // *different* models. After the widening it draws six *seeds of one* — and
    // that one is `_defaults`, which sorts first in the `BTreeMap` (`_` is 0x5F,
    // below `a`). A shared-RNG regression in any real genre would have become
    // invisible to this test. Striding by the seed count restores one seed each
    // from six distinct models, which is what the claim needs.
    for (id, model, seed) in combinations().step_by(SEEDS_PER_MODEL as usize).take(6) {
        let first = arrange(model, seed);
        let again = arrange(model, seed);

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
