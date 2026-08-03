//! The song as one playable clip (TASK-072).
//!
//! ⛔ **The gate that matters here is the cross-check against the export.**
//! `Song::flatten` and `midi::song_to_smf` lay the same arrangement out on the
//! same timeline, and until they shared [`SectionTiling`] they were two walks
//! over the same fields — free to disagree, so that what a producer heard was
//! not what they exported. The tiling arithmetic has been wrong twice already
//! and both times it shipped, which is exactly why this is asserted rather than
//! assumed.
//!
//! The individual properties are checked too, because a cross-check between two
//! things that are both wrong the same way passes.

use std::collections::BTreeMap;
use std::path::Path;

use engine::arrange;
use engine::context::SessionContext;
use engine::midi::song_to_smf;
use engine::pattern::{Lane, Note, Part, Pattern, PatternRef, Scale, Section, SectionKind, Song};
use engine::pattern::{DECAY_FLOOR, PPQ};
use engine::StyleModel;
use midly::{MidiMessage, Smf, TrackEventKind};

fn shipped() -> BTreeMap<String, StyleModel> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data");
    let scan = engine::dataset::files::scan(&dir).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

fn song(id: &str, seed: u64) -> Song {
    let model = shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("no `{id}` in the shipped dataset"));
    arrange::generate(&model, &SessionContext::default(), seed).expect("builds a song")
}

/// Every note-on tick in the exported file, across every track.
fn exported_on_ticks(bytes: &[u8]) -> Vec<u32> {
    let smf = Smf::parse(bytes).expect("the export must parse");
    let mut out = Vec::new();
    for track in &smf.tracks {
        let mut at = 0u32;
        for event in track {
            at = at.saturating_add(event.delta.as_int());
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { vel, .. },
                ..
            } = event.kind
            {
                if vel.as_int() > 0 {
                    out.push(at);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Every note-on tick in the flattened clip.
fn played_on_ticks(pattern: &Pattern) -> Vec<u32> {
    let mut out: Vec<u32> = pattern
        .lanes
        .iter()
        .flat_map(|lane| lane.notes.iter().map(|note| note.start_tick))
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn what_plays_is_what_exports() {
    // ⛔ The whole reason `SectionTiling` exists. A slide writes *two* MIDI
    // notes for one `Note`, so the counts legitimately differ — but every tick
    // the file plays a note at must be a tick the schedule plays a note at, and
    // the span they cover must match.
    for id in ["trap", "osamason", "uk-drill", "boom-bap"] {
        for seed in [7u64, 4_242] {
            let song = song(id, seed);
            let played = played_on_ticks(&song.flatten());
            let exported = exported_on_ticks(&song_to_smf(&song));

            assert!(!played.is_empty(), "{id}/{seed}: the song plays nothing");
            assert_eq!(
                played.first(),
                exported.first(),
                "{id}/{seed}: the first note is at a different tick"
            );
            assert_eq!(
                played.last(),
                exported.last(),
                "{id}/{seed}: the last note is at a different tick"
            );

            // Every played tick is an exported tick. Not the reverse: a slide's
            // destination note is written to the file and is part of the same
            // `Note` in the schedule.
            let in_file: std::collections::BTreeSet<u32> = exported.iter().copied().collect();
            for tick in &played {
                assert!(
                    in_file.contains(tick),
                    "{id}/{seed}: the schedule plays a note at {tick} the file does not"
                );
            }
        }
    }
}

#[test]
fn a_looping_clip_is_tiled_across_the_section_that_plays_it() {
    // The same claim `song_export.rs` makes for the file, made for playback: a
    // sixteen-bar verse over a four-bar clip is that clip four times. Played
    // once instead, three bars in four are silent — and every other assertion
    // here still passes.
    let clip = one_bar_kick("a");
    let song = two_sections(clip, 8, 4);
    let flat = song.flatten();

    let ticks = played_on_ticks(&flat);
    // 8 bars + 4 bars of a one-bar clip carrying one note.
    assert_eq!(ticks.len(), 12, "the clip was not tiled: {ticks:?}");
    assert_eq!(ticks.first(), Some(&0));
    assert_eq!(ticks.last(), Some(&(11 * PPQ * 4)));
}

#[test]
fn the_flattened_clip_is_as_long_as_the_song() {
    // `Schedule::progress` divides by the clip's own length, so a flattened
    // song that reported one section's length would run the marker to the right
    // edge in the intro and pin it there for the rest of the record.
    for seed in [1u64, 2, 3] {
        let song = song("trap", seed);
        assert_eq!(u32::from(song.flatten().bars), song.total_bars());
    }
}

#[test]
fn a_drop_out_is_silent_in_what_plays_as_well_as_in_what_exports() {
    // ⛔ TASK-066's transitions are `Section` fields, and this module's own rule
    // is that a field the export honours and playback ignores is the same
    // failure as a field the export ignores. The drop-out is the audible one:
    // it is the beat or two of nothing that makes the hook land.
    let clip = one_bar_kick("a");
    let mut song = two_sections(clip, 4, 4);
    song.sections[0].drop_out_beats = 8; // two whole bars of a 4/4 section

    let ticks = played_on_ticks(&song.flatten());
    let bar = PPQ * 4;
    // The first section keeps bars 0 and 1 and loses 2 and 3.
    assert_eq!(ticks[0], 0);
    assert_eq!(ticks[1], bar);
    assert_eq!(ticks[2], 4 * bar, "the drop-out still sounded: {ticks:?}");
}

#[test]
fn a_decaying_section_gets_quieter_across_its_length_when_played() {
    // ⚠ The trap this test's export twin was written to avoid: a section
    // exactly *one clip long* is the shape the shipped data produces, and the
    // ramp used to be measured in whole clip repeats — zero throughout, so
    // every outro played and exported dead flat while the timeline drew a fade.
    // ⛔ **One clip exactly as long as the section**, which is the shape the
    // shipped data produces — `_defaults` authors a 4-bar outro and the bars
    // chip defaults to 4 — and the one a per-repeat ramp cannot fade at all,
    // because there is only ever repeat zero. A fixture built from several
    // repeats would ramp fine under the bug.
    let mut clip = one_bar_kick("a");
    clip.bars = 4;
    clip.lanes[0].notes = (0..4)
        .map(|bar| Note {
            start_tick: bar * PPQ * 4,
            len_ticks: PPQ / 4,
            pitch: 36,
            vel: 120,
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
        })
        .collect();

    let mut song = two_sections(clip, 1, 4);
    song.sections[1].decay = true;

    let flat = song.flatten();
    let notes: Vec<&Note> = flat
        .lanes
        .iter()
        .flat_map(|lane| lane.notes.iter())
        .filter(|note| note.start_tick >= PPQ * 4)
        .collect();
    assert!(notes.len() >= 2, "not enough notes to see a ramp");

    let first = notes.first().expect("a first note").vel;
    let last = notes.last().expect("a last note").vel;
    assert!(
        last < first,
        "the outro played flat: {first} then {last} — the ramp is per-repeat again"
    );
    assert!(
        f32::from(last) >= f32::from(first) * DECAY_FLOOR * 0.9,
        "the outro reached silence before its bars ran out: {first} then {last}"
    );
}

#[test]
fn a_section_span_covers_exactly_the_bars_it_occupies() {
    // What the loop-section toggle sets as the schedule's loop region. Off by a
    // bar in either direction and the loop either clips the section's last bar
    // or plays the top of the next one.
    let song = song("trap", 11);
    let bar = song.ticks_per_bar();
    for (index, section) in song.sections.iter().enumerate() {
        let span = song.section_span(index).expect("every section has a span");
        assert_eq!(span.from_tick, section.start_bar * bar);
        assert_eq!(
            span.to_tick,
            (section.start_bar + u32::from(section.bars)) * bar
        );
    }
    assert_eq!(song.section_span(song.sections.len()), None);
}

#[test]
fn a_dangling_reference_is_silence_rather_than_a_panic() {
    // A `Song` reaches `flatten` from the webview and from a restored project,
    // so a reference the store does not hold is reachable. `dangling_refs` is
    // what reports it; this must not be where it is discovered.
    let clip = one_bar_kick("a");
    let mut song = two_sections(clip, 4, 4);
    song.patterns.remove("a");

    let flat = song.flatten();
    assert!(flat.lanes.iter().all(|lane| lane.notes.is_empty()));
    assert_eq!(song.dangling_refs(), vec!["a".to_owned()]);
}

// ---------------------------------------------------------------------------
// Fixtures: one note per bar, so a tick is readable as a bar number.
// ---------------------------------------------------------------------------

fn one_bar_kick(id: &str) -> Pattern {
    Pattern {
        id: id.to_owned(),
        part: Part::Drums,
        artist_id: "fixture".into(),
        seed: 1,
        bars: 1,
        bpm: 140.0,
        time_sig_num: 4,
        time_sig_den: 4,
        key_root: 0,
        scale: Scale::NaturalMinor,
        lanes: vec![engine::pattern::LaneTrack {
            lane: Lane::Kick,
            notes: vec![Note {
                start_tick: 0,
                len_ticks: PPQ / 4,
                pitch: 36,
                vel: 120,
                model_vel: None,
                slide_to_pitch: None,
                articulation: None,
            }],
        }],
        ppq: PPQ,
        mood: None,
        loop_region: None,
        clip_region: None,
    }
}

fn two_sections(clip: Pattern, first_bars: u16, second_bars: u16) -> Song {
    let refs = || {
        BTreeMap::from([(
            Part::Drums,
            PatternRef {
                pattern_id: "a".into(),
            },
        )])
    };
    Song {
        id: "fixture".into(),
        artist_id: "fixture".into(),
        seed: 1,
        bpm: 140.0,
        key_root: 0,
        scale: Scale::NaturalMinor,
        sections: vec![
            Section {
                kind: SectionKind::Intro,
                start_bar: 0,
                bars: first_bars,
                patterns: refs(),
                drop_out_beats: 0,
                decay: false,
                markers: vec![],
            },
            Section {
                kind: SectionKind::Hook,
                start_bar: u32::from(first_bars),
                bars: second_bars,
                patterns: refs(),
                drop_out_beats: 0,
                decay: false,
                markers: vec![],
            },
        ],
        time_sig_num: 4,
        time_sig_den: 4,
        patterns: BTreeMap::from([("a".to_owned(), clip)]),
        ppq: PPQ,
    }
}
