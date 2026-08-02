//! What a DAW receives (TASK-026, FR-015).
//!
//! `midi.rs`'s own tests build tiny patterns by hand to pin one rule each. This
//! file makes the other claim: that the file a user actually gets — generated
//! from a shipped model, humanized, then written — parses back with its notes
//! intact and its session described correctly.
//!
//! The two are not the same test. Hand-built input cannot produce the note
//! densities, roll subdivisions and slide targets the generators do, and every
//! failure mode here (a hanging note, a hit past the end, a tempo that does not
//! match the session) is silent in the bytes and obvious in a DAW.

use std::collections::BTreeMap;
use std::path::Path;

use engine::context::{Humanize, SessionContext, Swing, SwingGrid};
use engine::generators::drums::generate;
use engine::humanize::humanize;
use engine::midi::pattern_to_smf;
use engine::pattern::{Lane, Part, Pattern, Scale, PPQ};
use engine::StyleModel;
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};

fn shipped() -> BTreeMap<String, StyleModel> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data");
    let scan = engine::dataset::files::scan(&dir).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

/// A session with feel in it, so the export is tested against notes that sit
/// off the grid rather than on tidy tick boundaries.
fn context(bars: u16) -> SessionContext {
    SessionContext {
        bpm: 143.0,
        time_sig_num: 4,
        time_sig_den: 4,
        // F# minor — three sharps, and not the zero that a forgotten field
        // would also produce.
        key_root: 6,
        scale: Scale::NaturalMinor,
        swing: Swing {
            grid: SwingGrid::Sixteenth,
            amount: 0.56,
        },
        bars,
        half_time: false,
        humanize: Humanize {
            quantize_strength: 0.85,
            velocity_var: 0.15,
            timing_jitter_ms: [(Lane::Kick, 2.0), (Lane::ClosedHat, 3.0)]
                .into_iter()
                .collect(),
        },
    }
}

fn render(model: &StyleModel, seed: u64, bars: u16) -> Pattern {
    let ctx = context(bars);
    let mut lanes = generate(model, &ctx, seed);
    humanize(&mut lanes, &ctx, seed);

    Pattern {
        loop_region: None,
        clip_region: None,
        id: format!("{}-{seed}", model.id),
        part: Part::Drums,
        artist_id: model.id.clone(),
        seed,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
        mood: None,
    }
}

/// Every model in the roster, so a new one cannot arrive untested.
fn every_model() -> Vec<(String, StyleModel)> {
    let models: Vec<_> = shipped()
        .into_iter()
        .filter(|(id, _)| !id.starts_with('_'))
        .collect();
    assert!(
        models.len() >= 15,
        "the roster should be the whole shipped set, got {}",
        models.len()
    );
    models
}

#[test]
fn no_generated_note_is_ever_left_hanging() {
    // A note-on with no note-off sounds forever in the DAW, and an unpaired
    // note-off silences whatever it lands on. Neither shows up in the bytes,
    // and both are what an interleaving bug produces — so this walks the real
    // stream and counts voices per (channel, key).
    for (id, model) in every_model() {
        for seed in [1u64, 7, 2024] {
            let bytes = pattern_to_smf(&render(&model, seed, 4));
            let smf = Smf::parse(&bytes).unwrap_or_else(|e| panic!("{id} seed {seed}: {e}"));

            let mut sounding: BTreeMap<(u8, u8), i32> = BTreeMap::new();
            for event in smf.tracks[0].iter() {
                let TrackEventKind::Midi { channel, message } = event.kind else {
                    continue;
                };
                let (key, delta) = match message {
                    // Velocity 0 is the running-status note-off every writer is
                    // allowed to use; counting it as an on would hide a leak.
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => (key, 1),
                    MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => (key, -1),
                    _ => continue,
                };
                let count = sounding
                    .entry((channel.as_int(), key.as_int()))
                    .or_insert(0);
                *count += delta;
                assert!(
                    *count >= 0,
                    "{id} seed {seed}: a note-off with nothing to release on key {}",
                    key.as_int()
                );
            }

            let stuck: Vec<_> = sounding.iter().filter(|(_, v)| **v != 0).collect();
            assert!(
                stuck.is_empty(),
                "{id} seed {seed}: notes never released: {stuck:?}"
            );
        }
    }
}

#[test]
fn every_drum_hit_reaches_the_file_on_the_percussion_channel() {
    // Exact, not "at least": a drum lane cannot slide, so its note count and
    // its note-on count are the same number. Anything that silently dropped a
    // lane on the way out would still produce a playable file.
    for (id, model) in every_model() {
        let pattern = render(&model, 7, 4);
        let expected: usize = pattern
            .lanes
            .iter()
            .filter(|track| {
                !matches!(
                    track.lane,
                    Lane::Bass808 | Lane::Melody | Lane::Counter | Lane::Bass | Lane::Chords
                )
            })
            .map(|track| track.notes.len())
            .sum();
        assert!(expected > 0, "{id} generated no drums to export");

        let bytes = pattern_to_smf(&pattern);
        let smf = Smf::parse(&bytes).unwrap();
        let ons = smf.tracks[0]
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    TrackEventKind::Midi {
                        channel,
                        message: MidiMessage::NoteOn { vel, .. },
                    } if channel.as_int() == 9 && vel.as_int() > 0
                )
            })
            .count();
        assert_eq!(
            ons, expected,
            "{id}: drum hits lost or invented on the way out"
        );
    }
}

#[test]
fn nothing_sounds_past_the_end_of_the_clip() {
    // A clip dropped on a timeline loops at the bar. A note-on after the last
    // barline plays over the top of the next repeat, which is audible and is
    // the kind of thing a fill at a phrase boundary can cause.
    for (id, model) in every_model() {
        for bars in [4u16, 8] {
            let pattern = render(&model, 11, bars);
            let end = u32::from(bars) * PPQ * 4;

            let bytes = pattern_to_smf(&pattern);
            let smf = Smf::parse(&bytes).unwrap();
            let mut tick = 0u32;
            for event in smf.tracks[0].iter() {
                tick += event.delta.as_int();
                if let TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { vel, .. },
                    ..
                } = event.kind
                {
                    if vel.as_int() > 0 {
                        assert!(tick < end, "{id} {bars} bars: a hit at {tick}, past {end}");
                    }
                }
            }
        }
    }
}

#[test]
fn the_session_the_user_set_is_the_session_the_daw_reads() {
    // The roadmap's own acceptance for this task: the file opens with the right
    // BPM and the right key. Both are meta events nothing else in the suite
    // reads back out of a generated file.
    let models = shipped();
    let trap = models.get("trap").expect("trap must ship");
    let bytes = pattern_to_smf(&render(trap, 7, 4));
    let smf = Smf::parse(&bytes).unwrap();

    let mut tempo = None;
    let mut key = None;
    let mut name = None;
    for event in smf.tracks[0].iter() {
        match event.kind {
            TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempo = Some(t.as_int()),
            TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)) => {
                key = Some((sharps, minor))
            }
            TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                name = Some(String::from_utf8_lossy(n).into_owned())
            }
            _ => {}
        }
    }

    // 143 BPM: 60_000_000 / 143 = 419580.4, and a DAW reading this back must
    // land on 143 again rather than 142.9.
    let tempo = tempo.expect("a tempo must be written");
    let round_trip = 60_000_000.0 / f64::from(tempo);
    assert!(
        (round_trip - 143.0).abs() < 0.01,
        "the tempo reads back as {round_trip}, not 143"
    );
    assert_eq!(key, Some((3, true)), "F# minor is three sharps, minor");
    assert_eq!(name.as_deref(), Some("trap — Drums"));
}

/// TASK-041E: a clip in a meter other than 4/4 must *export* in it.
///
/// ⛔ The meta event is asserted, not `ticks_per_bar`. A DAW opening the file
/// reads the meta event and nothing else — a clip whose arithmetic was in 6/8
/// while its header said 4/4 would look right here and open wrong there, which
/// is the whole failure this task exists to close.
#[test]
fn a_six_eight_clip_says_six_eight_in_the_file() {
    let models = shipped();
    let model = models.get("trap").expect("trap must be in the roster");

    let ctx = SessionContext {
        time_sig_num: 6,
        time_sig_den: 8,
        ..context(4)
    };
    let mut lanes = generate(model, &ctx, 7);
    humanize(&mut lanes, &ctx, 7);
    let pattern = Pattern {
        loop_region: None,
        clip_region: None,
        id: "six-eight".into(),
        part: Part::Drums,
        artist_id: model.id.clone(),
        seed: 7,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
        mood: None,
    };

    let bytes = pattern_to_smf(&pattern);
    let smf = Smf::parse(&bytes).expect("and parse back");

    let mut meter = None;
    for event in smf.tracks[0].iter() {
        if let TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, _, _)) = event.kind {
            meter = Some((num, den_pow));
        }
    }
    // The denominator is a power of two in the file: 8 is 2³.
    assert_eq!(meter, Some((6, 3)), "the file must say 6/8");

    // And a 6/8 bar is three quarter notes, not six — so a four-bar clip is
    // twelve quarters long rather than twenty-four.
    assert_eq!(ctx.ticks_per_bar(), PPQ * 3);
}

/// TASK-041E: the clip's own start and end have to reach the *file*.
///
/// ⛔ The markers were draggable, saved with the project and read by nothing —
/// a producer trimmed a clip to its first bar, and the export still wrote four.
/// A boundary that only moves two marks on screen is worse than no boundary.
#[test]
fn a_trimmed_clip_exports_trimmed() {
    let models = shipped();
    let trap = models.get("trap").expect("trap must ship");
    let whole = render(trap, 7, 4);
    let bar = PPQ * 4;

    let mut trimmed = whole.clone();
    trimmed.clip_region = Some(engine::pattern::Region {
        from_tick: 0,
        to_tick: bar,
    });

    let count = |pattern: &Pattern| {
        Smf::parse(&pattern_to_smf(pattern))
            .expect("parses")
            .tracks
            .iter()
            .flatten()
            .filter(|event| {
                matches!(
                    event.kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::NoteOn { .. },
                        ..
                    }
                )
            })
            .count()
    };

    let before = count(&whole);
    let after = count(&trimmed);
    assert!(before > 0, "the fixture must have notes to trim");
    assert!(
        after < before,
        "trimming to one bar of four must drop notes: {after} of {before}"
    );
    assert!(
        whole
            .lanes
            .iter()
            .flat_map(|l| l.notes.iter())
            .any(|n| n.start_tick >= bar),
        "and the fixture must actually have notes past bar 1"
    );
}
