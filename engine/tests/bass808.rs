//! The 808 lane: legato, slides, register and the drill mute.
//!
//! The 808 is the one lane where a wrong length is as audible as a wrong note —
//! a gap between two notes is a hole in the record — so most of this is about
//! where notes *end*.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::generators::drums::{generate, generate_in};
use engine::generators::grid;
use engine::midi::pattern_to_smf;
use engine::pattern::{
    Articulation, Lane, LaneTrack, Note, Part, Pattern, Scale, SectionKind, PPQ,
};
use engine::StyleModel;
use serde_json::{json, Value};

const SEEDS: u64 = 100;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model, read and resolved **once per test binary**.
///
/// ⛔ **This used to re-read and re-resolve the entire dataset on every call.**
/// `files::scan` walks 609 JSON files and `resolve_all()` merges inheritance
/// across 590 models, and the callers below ask for it once per test — so the
/// same load ran over and over to answer questions about data that cannot have
/// changed. `plugin/tests/host_timeline.rs` measured the same mistake at
/// **1,300.91s → 1.70s** on one binary.
///
/// ⚠ **Cloned out rather than borrowed**, which keeps every call site unchanged:
/// a map copy is memory, and what this was costing was 609 file reads and 590
/// inheritance merges.
fn shipped() -> BTreeMap<String, StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, StyleModel>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            models
        })
        .clone()
}

fn shipped_model(id: &str) -> StyleModel {
    shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("`{id}` must ship"))
}

fn model(drums: Value) -> StyleModel {
    serde_json::from_value(json!({
        "id": "test", "type": "genre", "name": "Test", "drums": drums,
    }))
    .expect("the test model must parse")
}

fn ctx(bars: u16) -> SessionContext {
    SessionContext {
        bars,
        ..Default::default()
    }
}

fn notes(lanes: &[LaneTrack], want: Lane) -> Vec<Note> {
    lanes
        .iter()
        .find(|l| l.lane == want)
        .map(|l| l.notes.clone())
        .unwrap_or_default()
}

#[test]
fn the_808_rides_the_kick() {
    // Trap locks them 1:1 — "one instrument played twice".
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "halftime_3" },
        "bass808": { "role": "bassline", "register": [17, 31] }
    }));

    for seed in 0..SEEDS {
        let lanes = generate(&m, &ctx(2), seed);
        let kicks: Vec<u32> = notes(&lanes, Lane::Kick)
            .iter()
            .map(|n| n.start_tick)
            .collect();
        let bass: Vec<u32> = notes(&lanes, Lane::Sub)
            .iter()
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(bass, kicks, "seed {seed}");
    }
}

#[test]
fn a_looser_lock_follows_the_kick_less_often() {
    let share = |lock: f64| {
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": lock },
            "bass808": { "role": "bassline", "register": [17, 31] }
        }));
        let (mut bass, mut kick) = (0.0, 0.0);
        for seed in 0..SEEDS {
            let lanes = generate(&m, &ctx(4), seed);
            bass += notes(&lanes, Lane::Sub).len() as f64;
            kick += notes(&lanes, Lane::Kick).len() as f64;
        }
        bass / kick
    };

    assert!(
        (share(1.0) - 1.0).abs() < 0.01,
        "a full lock follows every kick"
    );
    let half = share(0.5);
    assert!((0.4..=0.6).contains(&half), "half a lock gave {half:.2}");
}

#[test]
fn the_808_is_legato_with_no_gaps_and_no_overlaps() {
    // Two rules at once: every note runs to the next (a gap is a hole), and
    // none overlaps it (mono, cut-self). The slide overlap is written by the
    // MIDI writer, not by the generator, so the pattern itself stays clean.
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "bass808": { "role": "bassline", "sustain": "legato", "register": [17, 31],
                     "slideProb": 0.6, "slidePositions": ["phrase_end", "bar_2"],
                     "slideIntervals": ["P5", "P8"] }
    }));

    for seed in 0..SEEDS {
        let context = ctx(4);
        let bass = notes(&generate(&m, &context, seed), Lane::Sub);
        for pair in bass.windows(2) {
            assert_eq!(
                pair[0].start_tick + pair[0].len_ticks,
                pair[1].start_tick,
                "seed {seed}: the line broke between {} and {}",
                pair[0].start_tick,
                pair[1].start_tick
            );
        }
        // ...and the last note runs to the end of the pattern.
        let last = bass.last().unwrap();
        assert_eq!(last.start_tick + last.len_ticks, context.total_ticks());
    }
}

#[test]
fn the_root_comes_from_the_session_key_and_stays_in_the_register() {
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 3, "lockTo808": 1.0 },
        "bass808": { "role": "bassline", "register": [17, 31] }
    }));

    // C, F and A as key roots.
    for (key_root, expected) in [(0u8, 24u8), (5, 17), (9, 21)] {
        let context = SessionContext {
            key_root,
            bars: 2,
            ..Default::default()
        };
        for seed in 0..20 {
            for note in notes(&generate(&m, &context, seed), Lane::Sub) {
                assert_eq!(note.pitch, expected, "key {key_root}, seed {seed}");
                assert!((17..=31).contains(&note.pitch));
            }
        }
    }
}

#[test]
fn a_slide_names_a_target_inside_the_register() {
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 1.0,
                     "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                     "slideIntervals": ["P5", "P8", "M2"] }
    }));

    let mut slides = 0;
    for seed in 0..SEEDS {
        for note in notes(&generate(&m, &ctx(4), seed), Lane::Sub) {
            let Some(target) = note.slide_to_pitch else {
                continue;
            };
            slides += 1;
            // The register plus an octave of headroom above the note the slide
            // starts from — an octave glide has to be able to reach an octave.
            let ceiling = 31u8.max(note.pitch + 12);
            assert!(
                (17..=ceiling).contains(&target),
                "seed {seed}: {target} escaped [17, {ceiling}]"
            );
            assert_ne!(target, note.pitch, "a slide onto its own pitch is not one");

            // The *signed* interval, against exactly what the fixture
            // authored — up, or folded an octave down to stay in range.
            // Comparing residues mod 12 accepted five of the twelve semitone
            // classes, including a P4 and a m7 the model never asked for, so
            // rewriting P5 as a fourth in the interval table left this green.
            let raw = i16::from(target) - i16::from(note.pitch);
            assert!(
                [7, 12, 2, -5, -12, -10].contains(&raw),
                "seed {seed}: {raw} semitones is not P5, P8 or M2, up or folded down"
            );
        }
    }
    assert!(slides > 0, "a certain slide never happened");
}

#[test]
fn a_counter_riff_stays_where_it_slid_and_a_bassline_goes_home() {
    // The difference that separates drill's 808 from trap's: one carries its
    // own line, the other doubles the roots.
    let block = |role: &str| {
        json!({
            "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
            "bass808": { "role": role, "register": [17, 31], "slideProb": 1.0,
                         "slidePositions": ["bar_2"], "slideIntervals": ["P5"] }
        })
    };

    let mut riff_seeds_that_moved = 0;
    for seed in 0..SEEDS {
        let riff = notes(
            &generate(&model(block("counter_riff")), &ctx(4), seed),
            Lane::Sub,
        );
        let bassline = notes(
            &generate(&model(block("bassline")), &ctx(4), seed),
            Lane::Sub,
        );

        // A bassline only ever plays the root.
        let roots: Vec<u8> = bassline.iter().map(|n| n.pitch).collect();
        assert!(
            roots.iter().all(|p| *p == roots[0]),
            "seed {seed}: a bassline wandered: {roots:?}"
        );

        // ...and a counter-riff, sliding on every eligible note, must land
        // somewhere else and *stay* there: the note after a slide carries the
        // pitch it slid to. One moved note on one seed used to be enough.
        for pair in riff.windows(2) {
            if let Some(target) = pair[0].slide_to_pitch {
                assert_eq!(
                    pair[1].pitch, target,
                    "seed {seed}: the riff slid to {target} and went home anyway"
                );
            }
        }
        if riff.iter().any(|n| n.pitch != riff[0].pitch) {
            riff_seeds_that_moved += 1;
        }
    }
    assert!(
        riff_seeds_that_moved > SEEDS / 2,
        "a counter-riff should leave the root on most seeds, got {riff_seeds_that_moved}"
    );
}

#[test]
fn drill_slides_two_to_three_times_every_four_bars() {
    // The authored statistic, and the genre marker: `slidesPer4Bars: [2, 3]`.
    let drill = shipped_model("uk-drill");
    let context = ctx(4);

    let counts: Vec<usize> = (0..SEEDS)
        .map(|seed| {
            notes(&generate(&drill, &context, seed), Lane::Sub)
                .iter()
                .filter(|n| n.slide_to_pitch.is_some())
                .count()
        })
        .collect();

    // The ceiling is absolute — three is what the model asks for and a fourth
    // would be a different genre.
    for (seed, count) in counts.iter().enumerate() {
        assert!(*count <= 3, "seed {seed}: {count} slides in four bars");
    }

    // The floor is not, and pretending otherwise would be a test that lies
    // about the engine: drill locks its 808 to only 60% of the kicks, so on a
    // few seeds every position a slide could have taken is simply not played.
    // What has to hold is that the marker arrives on almost every pattern and
    // averages inside the authored band.
    let average = counts.iter().sum::<usize>() as f64 / SEEDS as f64;
    assert!(
        (1.5..=3.0).contains(&average),
        "drill should average two to three slides a phrase, got {average:.2}"
    );

    let silent = counts.iter().filter(|c| **c == 0).count();
    assert!(
        silent <= SEEDS as usize / 10,
        "{silent} of {SEEDS} patterns had no slide at all — the marker is missing"
    );
}

#[test]
fn drills_808_stops_under_the_snare() {
    // "Mutes at snare hits" — the gap that makes drill sound like drill.
    let drill = shipped_model("uk-drill");
    let context = ctx(4);

    for seed in 0..SEEDS {
        let lanes = generate(&drill, &context, seed);
        let snares: Vec<u32> = notes(&lanes, Lane::Snare)
            .iter()
            .filter(|n| n.articulation != Some(Articulation::Ghost))
            .map(|n| n.start_tick)
            .collect();

        for note in notes(&lanes, Lane::Sub) {
            let end = note.start_tick + note.len_ticks;
            for snare in &snares {
                assert!(
                    !(note.start_tick < *snare && end > *snare),
                    "seed {seed}: an 808 rang through the snare at {snare}"
                );
            }
        }
    }
}

#[test]
fn trap_lets_its_808_ring_through_because_it_does_not_ask_for_the_mute() {
    // The control for the test above: without `muteUnderSnare` the line is
    // continuous, so the drill assertion is measuring the parameter and not
    // something that happens to be true everywhere.
    let trap = shipped_model("trap");
    let context = ctx(4);

    let mut rang_through = false;
    for seed in 0..SEEDS {
        let lanes = generate(&trap, &context, seed);
        let snares: Vec<u32> = notes(&lanes, Lane::Snare)
            .iter()
            .filter(|n| n.articulation != Some(Articulation::Ghost))
            .map(|n| n.start_tick)
            .collect();
        for note in notes(&lanes, Lane::Sub) {
            let end = note.start_tick + note.len_ticks;
            if snares.iter().any(|s| note.start_tick < *s && end > *s) {
                rang_through = true;
            }
        }
    }
    assert!(rang_through, "trap's 808 should sustain across the snare");
}

#[test]
fn a_generated_slide_exports_as_two_overlapping_notes() {
    // FR-015 / US-009: the file a DAW opens has to carry the glide. The writer
    // owns the convention; this checks a *generated* pattern reaches it.
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 1.0,
                     "slidePositions": ["bar_2"], "slideIntervals": ["P5"] }
    }));
    let context = ctx(4);

    let lanes: Vec<LaneTrack> = generate(&m, &context, 3)
        .into_iter()
        .filter(|l| l.lane == Lane::Sub)
        .collect();
    assert!(
        lanes[0].notes.iter().any(|n| n.slide_to_pitch.is_some()),
        "the fixture needs a slide in it"
    );

    let pattern = Pattern {
        loop_region: None,
        clip_region: None,
        id: "test".into(),
        part: Part::Drums,
        artist_id: "test".into(),
        seed: 3,
        song_seed: 3,
        bars: context.bars,
        bpm: context.bpm,
        time_sig_num: context.time_sig_num,
        time_sig_den: context.time_sig_den,
        key_root: context.key_root,
        scale: Scale::NaturalMinor,
        lanes,
        ppq: PPQ,

        mood: None,
    };

    let bytes = pattern_to_smf(&pattern);
    assert_eq!(&bytes[0..4], b"MThd");
    // The overlap is what a sampler reads as portamento: somewhere in the file
    // a second note-on arrives before the first note-off.
    let parsed = midly::Smf::parse(&bytes).expect("our own output must parse");
    let mut open = 0;
    let mut overlapped = false;
    for track in &parsed.tracks {
        for event in track {
            if let midly::TrackEventKind::Midi { message, .. } = event.kind {
                match message {
                    midly::MidiMessage::NoteOn { vel, .. } if vel.as_int() > 0 => {
                        open += 1;
                        if open > 1 {
                            overlapped = true;
                        }
                    }
                    midly::MidiMessage::NoteOff { .. } => open -= 1,
                    midly::MidiMessage::NoteOn { .. } => open -= 1,
                    _ => {}
                }
            }
        }
    }
    assert!(overlapped, "the slide's overlap did not survive export");
}

#[test]
fn every_shipped_model_with_an_808_produces_a_playable_line() {
    for (id, model) in shipped() {
        // `null` is how a model says it has no 808 — boom-bap, country and
        // R&B are real kits. The test's idea of "has an 808" has to be the
        // engine's, or it asks a drummer for a sub-bass line.
        let has_808 = model
            .blocks
            .get("drums")
            .and_then(|d| d.get("bass808"))
            .is_some_and(|value| !value.is_null());
        if !has_808 {
            continue;
        }
        // A model that says `staccato` must get staccato notes. Asserting
        // `Legato` outright would have been a test that only passed because
        // every model at the time happened to want it.
        let staccato = model
            .blocks
            .get("drums")
            .and_then(|d| d.pointer("/bass808/sustain"))
            .and_then(Value::as_str)
            == Some("staccato");
        let expected = if staccato {
            Articulation::Staccato
        } else {
            Articulation::Legato
        };

        let context = ctx(4);
        for seed in 0..20u64 {
            let bass = notes(&generate(&model, &context, seed), Lane::Sub);
            assert!(!bass.is_empty(), "{id} seed {seed}: no 808 at all");
            for note in &bass {
                assert!(note.len_ticks > 0, "{id}: a zero-length 808");
                assert!(note.vel >= 1 && note.vel <= 127);
                assert!(
                    note.start_tick + note.len_ticks <= context.total_ticks(),
                    "{id} seed {seed}: an 808 ran past the pattern"
                );
                assert_eq!(note.articulation, Some(expected), "{id}");
                if staccato {
                    // The "Light 808": a bounce, not a sustain.
                    assert!(
                        note.len_ticks <= grid::SIXTEENTH,
                        "{id}: a staccato 808 of {} ticks is a sustain",
                        note.len_ticks
                    );
                }
            }
        }
    }
}

#[test]
fn every_slide_interval_in_the_dataset_is_one_the_engine_knows() {
    // The silent-failure class again: an interval name the table rejects is
    // skipped, so the model lists a slide that can never happen.
    let mut checked = 0;
    for (id, model) in shipped() {
        let Some(intervals) = model
            .blocks
            .get("drums")
            .and_then(|d| d.pointer("/bass808/slideIntervals"))
            .filter(|value| !value.is_null())
        else {
            continue;
        };
        let values = intervals
            .get("values")
            .or(Some(intervals))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(!values.is_empty(), "{id}: an empty slide vocabulary");

        for value in values.iter().filter_map(Value::as_str) {
            assert!(
                engine::theory::interval_semitones(value).is_some(),
                "{id}: `{value}` is not an interval the engine knows"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no slide intervals were checked");
}

// ---------------------------------------------------------------------------
// The glide's shape: `portamentoMs` (105 models) and `slideOverlap` (3), both
// authored long before anything could read them.
// ---------------------------------------------------------------------------

#[test]
fn an_authored_glide_time_reaches_every_sliding_note_and_only_those() {
    // `drums.bass808.portamentoMs` — the largest entry the authored-key register
    // ever held. A slide with no time on it is one the sampler plays at its own
    // constant rate, which is the defect.
    let model = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "backbeat_24" },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 1.0,
                     "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                     "portamentoMs": 90, "slideIntervals": ["P5"] },
    }));
    let context = ctx(4);

    let mut slid = 0;
    for seed in 0..20u64 {
        for note in notes(&generate(&model, &context, seed), Lane::Sub) {
            match note.slide_to_pitch {
                Some(_) => {
                    slid += 1;
                    assert_eq!(note.slide_ms, Some(90), "a slide with no glide time");
                }
                // ⚠ Stamped on the sliding notes only. A flat note carrying a
                // portamento would serialize into every saved project and every
                // golden for 105 models to say nothing at all.
                None => assert_eq!(note.slide_ms, None, "a flat note carries a glide"),
            }
        }
    }
    assert!(slid > 0, "no slide was ever written");
}

#[test]
fn the_glide_time_is_one_setting_for_the_take_rather_than_a_draw_per_note() {
    // ⚠ Portamento is a knob on the instrument: a producer sets it and plays the
    // line. A per-note draw would also move 105 models' rng position on every
    // slide, which is a change to their *pitches* made by a key about tone.
    let model = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "backbeat_24" },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 1.0,
                     "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                     "portamentoMs": [40, 200], "slideIntervals": ["P5", "m7"] },
    }));
    let context = ctx(8);

    for seed in 0..20u64 {
        let times: Vec<Option<u16>> = notes(&generate(&model, &context, seed), Lane::Sub)
            .iter()
            .filter(|n| n.slide_to_pitch.is_some())
            .map(|n| n.slide_ms)
            .collect();
        if times.len() < 2 {
            continue;
        }
        assert!(
            times.windows(2).all(|pair| pair[0] == pair[1]),
            "seed {seed}: the glide time changed mid-take: {times:?}"
        );
    }
}

#[test]
fn the_glide_time_moves_no_note_of_the_line() {
    // ⛔ The rule a sub-kick velocity broke once, applied to the tone keys: they
    // draw from `drums/bass808/tone`, so adding one to a model rewrites nothing
    // it plays.
    let with_glide = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "backbeat_24" },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 0.6,
                     "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                     "portamentoMs": [40, 200], "slideIntervals": ["P5", "m7"] },
    }));
    let bare = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "backbeat_24" },
        "bass808": { "role": "bassline", "register": [17, 31], "slideProb": 0.6,
                     "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                     "slideIntervals": ["P5", "m7"] },
    }));
    let context = ctx(4);

    for seed in 0..20u64 {
        let glided = notes(&generate(&with_glide, &context, seed), Lane::Sub);
        let plain = notes(&generate(&bare, &context, seed), Lane::Sub);
        let strip = |notes: Vec<Note>| -> Vec<Note> {
            notes
                .into_iter()
                .map(|mut n| {
                    n.slide_ms = None;
                    n
                })
                .collect()
        };
        assert_eq!(strip(glided), plain, "seed {seed}: the line itself moved");
    }
}

#[test]
fn trap_exports_the_wider_overlap_it_authors() {
    // `drums.bass808.slideOverlap: "1/16"` — trap authors it, authors no
    // `portamentoMs`, and the writer answered for it with a constant 32nd. The
    // two keys are independent and this is the half the `.mid` can carry.
    let trap = shipped_model("trap");
    let context = ctx(4);

    let mut checked = 0;
    for seed in 0..40u64 {
        for note in notes(&generate(&trap, &context, seed), Lane::Sub) {
            if note.slide_to_pitch.is_some() {
                assert_eq!(
                    note.slide_overlap_ticks,
                    grid::note_value_ticks("1/16").map(|t| t as u16),
                    "trap's authored overlap did not reach the note"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "trap never slid");
}

#[test]
fn drills_loop_leads_back_into_itself() {
    // `arrangement.melodic808SlideAtLoopEnd` — `uk-drill` is the one author, and
    // the whole point of the key is that the four bars do not stop dead.
    let drill = shipped_model("uk-drill");
    let context = ctx(4);

    let mut landed = 0;
    let mut slid = 0;
    for seed in 0..SEEDS {
        let line = notes(&generate(&drill, &context, seed), Lane::Sub);
        if line.len() < 2 || !line.iter().any(|n| n.slide_to_pitch.is_some()) {
            continue;
        }
        slid += 1;
        let last = line.last().expect("checked");
        if last.slide_to_pitch.is_some() {
            landed += 1;
        }
    }

    assert!(slid > 0, "drill never slid at all");
    // ⚠ Not every one: the last note may already be on the pitch the loop opens
    // on, and a slide onto a note's own pitch is not a slide. What must not
    // happen is the gesture landing anywhere *but* the loop's end.
    assert!(
        landed * 4 >= slid * 3,
        "only {landed} of {slid} sliding patterns slid at the loop end"
    );
}

#[test]
fn the_turnaround_never_buys_a_fourth_slide() {
    // ⛔ `slidesPer4Bars: [2, 3]`, and the ceiling test right above calls it
    // absolute. This key names *where* the gesture goes, so a pattern that was
    // already at three has to still be at three.
    let drill = shipped_model("uk-drill");
    let context = ctx(4);

    for seed in 0..SEEDS {
        let count = notes(&generate(&drill, &context, seed), Lane::Sub)
            .iter()
            .filter(|n| n.slide_to_pitch.is_some())
            .count();
        assert!(count <= 3, "seed {seed}: the turnaround made it {count}");
    }
}

#[test]
fn a_cross_stick_verse_still_stops_the_808_for_the_backbeat() {
    // ⛔⛔ **The defect a review caught before it shipped.** `crossStickVerses`
    // was first written at the placement site, so in a verse the backbeat went
    // straight onto `Lane::Rim` — and `bass808` reads the hits it must not ring
    // through out of `kit.notes(Lane::Snare)`. The list came back without a
    // backbeat in it and the 808 sustained over the one hit drill and trap-soul
    // are defined by stopping for, in a lane that never mentions the key.
    //
    // It is a finishing pass now, run after the 808 is placed. This is what says
    // so if it ever moves back.
    let m = model(json!({
        "kick": { "anchors": ["1"], "densityPerBar": 4, "lockTo808": 1.0 },
        "snare": { "placement": "backbeat_24", "crossStickVerses": true },
        "bass808": { "role": "bassline", "sustain": "legato", "register": [24, 36],
                     "muteUnderSnare": true },
    }));

    let mut verses = 0;
    for seed in 0..SEEDS {
        let lanes = generate_in(&m, &ctx(4), seed, Some(SectionKind::Verse));
        let rims = notes(&lanes, Lane::Rim);
        let subs = notes(&lanes, Lane::Sub);
        if rims.is_empty() || subs.is_empty() {
            continue;
        }
        verses += 1;
        // The backbeat did move to the rim — otherwise this proves nothing.
        assert!(
            notes(&lanes, Lane::Snare)
                .iter()
                .all(|n| n.articulation.is_some()),
            "seed {seed}: the backbeat is still on the drum"
        );
        for sub in &subs {
            let end = sub.start_tick + sub.len_ticks;
            for rim in &rims {
                assert!(
                    !(sub.start_tick < rim.start_tick && end > rim.start_tick),
                    "seed {seed}: an 808 rang through the cross-stick at {}",
                    rim.start_tick
                );
            }
        }
    }
    assert!(verses > 0, "no verse had both an 808 and a cross-stick");
}
