//! What a DAW receives when a whole song is dragged out (US-011, FR-015).
//!
//! `midi_export.rs` makes this claim for one pattern. A song is a different
//! shape and has failure modes a pattern cannot have: a section's clip loops, so
//! the export has to *tile* it, and the transitions from TASK-066 live on the
//! section rather than in the notes — which means they exist only if this file
//! is what writes them out.
//!
//! ⛔ **The tiling gate is the one that matters.** A sixteen-bar verse over a
//! four-bar clip that is written once exports three bars of silence out of every
//! four, and every other assertion here still passes. The file opens, the track
//! names are right, the markers are in the right bars — and the song is mostly
//! empty.

use std::collections::BTreeMap;
use std::path::Path;

use engine::arrange;
use engine::context::SessionContext;
use engine::midi::song_to_smf;
use engine::pattern::{Part, Pattern, Section, SectionKind, Song};
use engine::StyleModel;
use midly::{Format, MetaMessage, MidiMessage, Smf, TrackEventKind};

fn shipped() -> BTreeMap<String, StyleModel> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data");
    let scan = engine::dataset::files::scan(&dir).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

fn model(id: &str) -> StyleModel {
    shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("no `{id}` in the shipped dataset"))
}

fn song(id: &str, seed: u64) -> Song {
    arrange::generate(&model(id), &SessionContext::default(), seed).expect("builds a song")
}

/// Note-ons per track, keyed by the track's name.
fn note_ons_by_track(bytes: &[u8]) -> BTreeMap<String, usize> {
    let smf = Smf::parse(bytes).expect("the export must parse");
    let mut out = BTreeMap::new();
    for track in &smf.tracks {
        let mut name = String::from("(unnamed)");
        let mut ons = 0usize;
        for event in track {
            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(raw)) => {
                    name = String::from_utf8_lossy(raw).into_owned();
                }
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { vel, .. },
                    ..
                } if vel.as_int() > 0 => ons += 1,
                _ => {}
            }
        }
        out.insert(name, ons);
    }
    out
}

fn markers(bytes: &[u8]) -> Vec<(u32, String)> {
    let smf = Smf::parse(bytes).expect("parses");
    let mut out = Vec::new();
    let mut tick = 0u32;
    for event in &smf.tracks[0] {
        tick += event.delta.as_int();
        if let TrackEventKind::Meta(MetaMessage::Marker(raw)) = event.kind {
            out.push((tick, String::from_utf8_lossy(raw).into_owned()));
        }
    }
    out
}

#[test]
fn a_song_exports_as_a_parallel_multi_track_file() {
    // Type-1, because US-011 promises "one multi-track SMF" — a producer drops
    // it in and gets a row per part rather than everything merged onto one.
    let bytes = song_to_smf(&song("trap", 7));
    let smf = Smf::parse(&bytes).expect("parses");
    assert_eq!(smf.header.format, Format::Parallel);
    assert!(
        smf.tracks.len() >= 2,
        "a conductor track and at least one part"
    );
}

#[test]
fn every_part_track_is_named_the_way_us_011_names_it() {
    // The producer opens a project that already has twenty tracks and has to
    // see which rows came from here.
    let bytes = song_to_smf(&song("trap", 7));
    let tracks = note_ons_by_track(&bytes);
    let named: Vec<&String> = tracks.keys().filter(|n| n.starts_with("FMM ")).collect();
    assert!(
        !named.is_empty(),
        "no track was named `FMM …`, got {:?}",
        tracks.keys().collect::<Vec<_>>()
    );
    for name in &named {
        assert!(
            [
                "FMM Drums",
                "FMM Chords",
                "FMM Melody",
                "FMM Counter",
                "FMM Bass"
            ]
            .contains(&name.as_str()),
            "unexpected track name `{name}`"
        );
    }
}

#[test]
fn the_conductor_track_marks_every_section() {
    // The arrangement surviving the export is what makes it an arrangement
    // rather than a long loop: the DAW's own ruler shows INTRO / VERSE / HOOK.
    let song = song("trap", 7);
    let bytes = song_to_smf(&song);
    let marks = markers(&bytes);

    assert_eq!(
        marks.len(),
        song.sections.len(),
        "one marker per section, got {marks:?}"
    );
    let ticks_per_bar = engine::pattern::PPQ * 4;
    for (section, (tick, text)) in song.sections.iter().zip(&marks) {
        assert_eq!(
            *tick,
            section.start_bar * ticks_per_bar,
            "marker `{text}` is not at its section's bar"
        );
        assert_eq!(text, &format!("{:?}", section.kind).to_uppercase());
    }
}

#[test]
fn a_looping_clip_is_tiled_across_the_section_it_fills() {
    // ⛔ **The gate this file exists for.** A section is a clip on a loop: a
    // sixteen-bar verse over a four-bar pattern is that pattern four times.
    // Writing it once is silent in the bytes, passes every other assertion
    // here, and exports a song that is three-quarters empty.
    //
    // Built by hand rather than generated, so the expected count is arithmetic
    // rather than a number read back off the thing being tested.
    let generated = song("trap", 7);
    let (id, pattern) = generated
        .patterns
        .iter()
        .find(|(_, p)| p.part == Part::Drums && p.note_count() > 0)
        .expect("a drum pattern with notes");
    let clip_bars = pattern.bars;
    let per_clip = pattern.note_count();

    // One section, eight clips long, playing that one pattern.
    let repeats = 8u16;
    let one = Song {
        id: "tiling".into(),
        artist_id: "trap".into(),
        seed: 1,
        bpm: 140.0,
        key_root: 0,
        scale: pattern.scale,
        sections: vec![Section {
            kind: SectionKind::Verse,
            start_bar: 0,
            bars: clip_bars * repeats,
            patterns: BTreeMap::from([(
                Part::Drums,
                engine::pattern::PatternRef {
                    pattern_id: id.clone(),
                },
            )]),
            drop_out_beats: 0,
            decay: false,
            markers: vec![],
        }],
        time_sig_num: 4,
        time_sig_den: 4,
        patterns: BTreeMap::from([(id.clone(), pattern.clone())]),
        ppq: engine::pattern::PPQ,
    };

    let ons = note_ons_by_track(&song_to_smf(&one));
    let drums = ons.get("FMM Drums").copied().unwrap_or(0);

    // Slides split one note into two note-ons, so the per-clip count is taken
    // from the same export rather than from `note_count`, and only the
    // *multiple* is asserted.
    let single = Song {
        sections: vec![Section {
            bars: clip_bars,
            ..one.sections[0].clone()
        }],
        ..one.clone()
    };
    let once = note_ons_by_track(&song_to_smf(&single))
        .get("FMM Drums")
        .copied()
        .unwrap_or(0);

    assert!(once > 0, "the one-clip export wrote nothing");
    assert_eq!(
        drums,
        once * usize::from(repeats),
        "a {}-bar section over a {clip_bars}-bar clip wrote {drums} note-ons \
         where {repeats} repeats of {once} is {}; the clip is being placed once \
         instead of tiled (pattern holds {per_clip} notes)",
        clip_bars * repeats,
        once * usize::from(repeats)
    );
}

#[test]
fn a_drop_out_removes_the_notes_it_covers() {
    // TASK-066's drop-out is a field on the section, and a field the export
    // ignores is the "trim that only moves two marks on screen" failure this
    // codebase already wrote down once. So it is asserted on the bytes.
    let generated = song("trap", 7);
    let (id, pattern) = generated
        .patterns
        .iter()
        .find(|(_, p)| p.part == Part::Drums && p.note_count() > 0)
        .expect("a drum pattern with notes");

    let base = Song {
        id: "dropout".into(),
        artist_id: "trap".into(),
        seed: 1,
        bpm: 140.0,
        key_root: 0,
        scale: pattern.scale,
        sections: vec![Section {
            kind: SectionKind::Verse,
            start_bar: 0,
            bars: pattern.bars * 2,
            patterns: BTreeMap::from([(
                Part::Drums,
                engine::pattern::PatternRef {
                    pattern_id: id.clone(),
                },
            )]),
            drop_out_beats: 0,
            decay: false,
            markers: vec![],
        }],
        time_sig_num: 4,
        time_sig_den: 4,
        patterns: BTreeMap::from([(id.clone(), pattern.clone())]),
        ppq: engine::pattern::PPQ,
    };

    let full = note_ons_by_track(&song_to_smf(&base))
        .get("FMM Drums")
        .copied()
        .unwrap_or(0);

    let dropped = Song {
        sections: vec![Section {
            drop_out_beats: 4,
            ..base.sections[0].clone()
        }],
        ..base.clone()
    };
    let after = note_ons_by_track(&song_to_smf(&dropped))
        .get("FMM Drums")
        .copied()
        .unwrap_or(0);

    assert!(full > 0, "the control export wrote nothing");
    assert!(
        after < full,
        "a four-beat drop-out changed nothing: {after} note-ons against {full}"
    );
}

#[test]
fn a_decaying_section_gets_quieter_and_never_reaches_silence() {
    // An outro that fades to zero before it ends leaves the producer dragging
    // in empty bars, which reads as the clip being broken rather than as a
    // fade — so the floor is asserted alongside the fall.
    let generated = song("trap", 7);
    let (id, pattern) = generated
        .patterns
        .iter()
        .find(|(_, p)| p.part == Part::Drums && p.note_count() > 4)
        .expect("a drum pattern with notes");

    let make = |decay: bool| Song {
        id: "decay".into(),
        artist_id: "trap".into(),
        seed: 1,
        bpm: 140.0,
        key_root: 0,
        scale: pattern.scale,
        sections: vec![Section {
            kind: SectionKind::Outro,
            start_bar: 0,
            bars: pattern.bars * 4,
            patterns: BTreeMap::from([(
                Part::Drums,
                engine::pattern::PatternRef {
                    pattern_id: id.clone(),
                },
            )]),
            drop_out_beats: 0,
            decay,
            markers: vec![],
        }],
        time_sig_num: 4,
        time_sig_den: 4,
        patterns: BTreeMap::from([(id.clone(), pattern.clone())]),
        ppq: engine::pattern::PPQ,
    };

    let velocities = |bytes: &[u8]| -> Vec<u8> {
        let smf = Smf::parse(bytes).expect("parses");
        smf.tracks
            .iter()
            .flat_map(|t| t.iter())
            .filter_map(|e| match e.kind {
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { vel, .. },
                    ..
                } if vel.as_int() > 0 => Some(vel.as_int()),
                _ => None,
            })
            .collect()
    };

    let flat = velocities(&song_to_smf(&make(false)));
    let faded = velocities(&song_to_smf(&make(true)));

    assert_eq!(
        flat.len(),
        faded.len(),
        "a decay must not delete notes, only quieten them"
    );
    assert!(!faded.is_empty(), "nothing was written");
    assert!(faded.iter().all(|v| *v >= 1), "a decay silenced a note");

    let head = faded.len() / 4;
    let mean = |slice: &[u8]| slice.iter().map(|v| f64::from(*v)).sum::<f64>() / slice.len() as f64;
    assert!(
        mean(&faded[faded.len() - head..]) < mean(&faded[..head]),
        "the end of the outro is no quieter than its start"
    );
    assert!(
        mean(&faded) < mean(&flat),
        "the decaying export is no quieter than the flat one"
    );
}

#[test]
fn every_shipped_model_exports_a_song_that_parses() {
    // The end-to-end claim: pick any artist, generate, drag. A model whose song
    // writes a file no DAW can open is a broken drag for that one artist, which
    // no per-part test would see.
    for (id, model) in shipped() {
        if id.starts_with('_') {
            continue;
        }
        let song = arrange::generate(&model, &SessionContext::default(), 2026)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let bytes = song_to_smf(&song);
        let smf = Smf::parse(&bytes).unwrap_or_else(|e| panic!("{id}: export will not parse: {e}"));
        assert!(smf.tracks.len() >= 2, "{id}: no part tracks");

        let ons: usize = note_ons_by_track(&bytes).values().sum();
        assert!(ons > 0, "{id}: the exported song is silent");
    }
}

#[test]
fn a_song_built_to_wrap_the_tiling_counter_terminates() {
    // ⛔ **The exploit /security-review found, asserted at the engine rather
    // than at the bridge.** The tiling loop was `while offset < sounding` with
    // `offset += clip_len`. Release builds carry no overflow checks, so `offset`
    // walks `k * clip_len mod 2^32` — and for these values that orbit steps over
    // `sounding` without ever reaching it, so the loop never ends. It spins the
    // thread the host draws its editor from: no crash, no error, no way out but
    // killing the DAW.
    //
    // The bridge refuses a song like this now, but the engine must not depend on
    // that: `song_to_smf` is a public function and the next caller will not know.
    // The loop is counted rather than conditional, so it terminates whatever the
    // arithmetic does — which is what this asserts by simply returning.
    let generated = song("trap", 7);
    let (id, pattern) = generated
        .patterns
        .iter()
        .find(|(_, p)| p.part == Part::Drums)
        .expect("a drum pattern");

    // The exact values from the finding: a 69/4 meter, a 64,824-bar section and
    // a 16,384-bar clip.
    let clip = Pattern {
        bars: 16_384,
        lanes: Vec::new(),
        ..pattern.clone()
    };
    let hostile = Song {
        id: "wrap".into(),
        artist_id: "x".into(),
        seed: 0,
        bpm: 120.0,
        key_root: 0,
        scale: clip.scale,
        sections: vec![Section {
            kind: SectionKind::Verse,
            start_bar: 0,
            bars: 64_824,
            patterns: BTreeMap::from([(
                Part::Drums,
                engine::pattern::PatternRef {
                    pattern_id: id.clone(),
                },
            )]),
            drop_out_beats: 0,
            decay: false,
            markers: vec![],
        }],
        time_sig_num: 69,
        time_sig_den: 4,
        patterns: BTreeMap::from([(id.clone(), clip)]),
        ppq: engine::pattern::PPQ,
    };

    // Reaching the next line at all is the assertion.
    let bytes = song_to_smf(&hostile);
    assert!(Smf::parse(&bytes).is_ok(), "the export must still parse");
}

#[test]
fn a_key_root_past_the_octave_does_not_overflow_the_signature() {
    // `key_root` is a `u8` off the wire on the song path, and the minor-key
    // branch adds 3 to it before reducing — so 253 and above overflowed.
    let generated = song("trap", 7);
    let hostile = Song {
        key_root: 255,
        ..generated
    };
    assert!(Smf::parse(&song_to_smf(&hostile)).is_ok());
}
