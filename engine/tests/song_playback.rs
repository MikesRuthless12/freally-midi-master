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

use engine::arrange;
use engine::context::SessionContext;
use engine::midi::song_to_smf;
use engine::pattern::{Lane, Note, Part, Pattern, PatternRef, Scale, Section, SectionKind, Song};
use engine::pattern::{DECAY_FLOOR, PPQ};

mod common;
use common::shipped_models;
use midly::{MidiMessage, Smf, TrackEventKind};

fn song(id: &str, seed: u64) -> Song {
    let model = shipped_models()
        .remove(id)
        .unwrap_or_else(|| panic!("no `{id}` in the shipped dataset"));
    arrange::generate(&model, &SessionContext::default(), seed).expect("builds a song")
}

/// Every note-on in the exported file as (tick, velocity), across every track.
fn exported_on_ticks(bytes: &[u8]) -> Vec<(u32, u8)> {
    let smf = Smf::parse(bytes).expect("the export must parse");
    let mut out: Vec<(u32, u8)> = Vec::new();
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
                    out.push((at, vel.as_int()));
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Every note-on in the flattened clip as (tick, velocity).
fn played_on_ticks(pattern: &Pattern) -> Vec<(u32, u8)> {
    let mut out: Vec<(u32, u8)> = pattern
        .lanes
        .iter()
        .flat_map(|lane| lane.notes.iter().map(|note| (note.start_tick, note.vel)))
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
            let in_file: std::collections::BTreeSet<(u32, u8)> = exported.iter().copied().collect();
            for tick in &played {
                assert!(
                    in_file.contains(tick),
                    "{id}/{seed}: the schedule plays {tick:?} and the file does not — \n                     a tick or a velocity the export and the transport disagree on"
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
    assert_eq!(ticks.first().map(|(tick, _)| *tick), Some(0));
    assert_eq!(ticks.last().map(|(tick, _)| *tick), Some(11 * PPQ * 4));
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
    assert_eq!(ticks[0].0, 0);
    assert_eq!(ticks[1].0, bar);
    assert_eq!(ticks[2].0, 4 * bar, "the drop-out still sounded: {ticks:?}");
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

#[test]
fn a_resized_clip_loops_on_its_own_length_in_what_plays_and_in_what_exports() {
    // ⛔⛔ **TASK-142's clip resize, checked on BOTH sides of the seam.** The
    // whole reason `PatternRef::bars` is read through `SectionTiling` is that
    // the exporter and the transport are two walks over these fields, and this
    // module's own header records that they have disagreed twice and shipped
    // both times. A resize that retiled the playback and not the file would be
    // the third.
    let mut clip = one_bar_kick("a");
    clip.bars = 4;
    // Four kicks, one per bar, so a tick reads as a bar number.
    clip.lanes[0].notes = (0..4)
        .map(|bar| Note {
            start_tick: bar * clip.ticks_per_bar(),
            len_ticks: PPQ / 4,
            pitch: 36,
            vel: 120,
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
        })
        .collect();

    // One eight-bar section over a four-bar clip: two repeats, eight kicks.
    let mut song = two_sections(clip, 8, 0);
    song.sections.truncate(1);
    assert_eq!(
        kick_ticks(&song).len(),
        8,
        "the clip's own length, untouched"
    );

    // ...and asked to loop on two bars instead, it lays down four repeats of
    // the clip's *first two* bars — eight kicks again, but at different ticks.
    let resized = kick_ticks(&resize_drums(&song, 2));
    assert_eq!(resized.len(), 8);
    let bar = song.ticks_per_bar();
    assert_eq!(
        resized,
        vec![0, bar, 2 * bar, 3 * bar, 4 * bar, 5 * bar, 6 * bar, 7 * bar],
        "a two-bar loop repeats every two bars"
    );

    // ⛔ And the exported file agrees, note for note.
    for bars in [None, Some(2), Some(1)] {
        let song = match bars {
            None => song.clone(),
            Some(bars) => resize_drums(&song, bars),
        };
        assert_eq!(
            kick_ticks(&song),
            exported_kick_ticks(&song),
            "what plays and what exports disagree at clipBars {bars:?}"
        );
    }
}

#[test]
fn a_resized_clip_does_not_ring_into_its_own_next_repeat() {
    // ⛔⛔ **The orphan note-off, reopened by making the loop shrinkable.**
    // `sounds` keeps or drops a note by its *onset* — right, because the two
    // halves of one note must go together — but it says nothing about length.
    // So a note longer than a shortened loop kept its full length: repeat 0's
    // long note was still sounding when repeat 1 re-struck the same pitch on the
    // same channel, and a DAW pairs the stale off with the live note and cuts it
    // dead. `SectionTiling::sounds`'s own note says the design exists to prevent
    // exactly this.
    let mut clip = one_bar_kick("a");
    clip.bars = 4;
    // ⚠ **Notes LONGER than the loop they will be resized to**, or there is
    // nothing to trim and the test cannot fail: two-bar notes at bars 0 and 2,
    // which sit end to end in the clip's own four bars and overhang a one-bar
    // loop by a whole bar each.
    let bar = clip.ticks_per_bar();
    clip.lanes[0].notes = [0, 2]
        .into_iter()
        .map(|index| Note {
            start_tick: index * bar,
            len_ticks: bar * 2,
            pitch: 36,
            vel: 120,
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
        })
        .collect();

    let mut song = two_sections(clip, 8, 0);
    song.sections.truncate(1);
    let resized = resize_drums(&song, 1);

    // Nothing may still be sounding when the next repeat starts.
    let flat = resized.flatten();
    for lane in &flat.lanes {
        for note in &lane.notes {
            assert!(
                note.len_ticks <= bar,
                "a note held {} ticks rings past a {bar}-tick loop",
                note.len_ticks
            );
        }
    }

    // ⛔ And the exported file agrees: every note-on is closed before the next
    // one on the same key. Read as a running count, which is what a DAW does.
    let bytes = song_to_smf(&resized);
    let smf = Smf::parse(&bytes).expect("a parseable file");
    for track in &smf.tracks {
        let mut sounding = 0i32;
        for event in track {
            if let TrackEventKind::Midi { message, .. } = event.kind {
                match message {
                    MidiMessage::NoteOn { key, vel } if key.as_int() == 36 && vel.as_int() > 0 => {
                        sounding += 1;
                        assert!(
                            sounding <= 1,
                            "two overlapping note-ons on one key — the previous \
                             repeat never closed"
                        );
                    }
                    MidiMessage::NoteOff { key, .. } if key.as_int() == 36 => sounding -= 1,
                    MidiMessage::NoteOn { key, .. } if key.as_int() == 36 => sounding -= 1,
                    _ => {}
                }
            }
        }
        assert_eq!(
            sounding, 0,
            "a note was left hanging at the end of the track"
        );
    }
}

#[test]
fn a_clip_resized_to_nothing_is_refused_rather_than_laid_down_forever() {
    // ⚠ `repeats` is `sounding.div_ceil(clip_len)`. A zero would not divide by
    // zero — `clip_len` is floored at 1 — it would lay the clip down once per
    // *tick*, which for an eight-bar section is 30,720 copies of every note,
    // built synchronously on the thread the host draws its window from.
    let mut clip = one_bar_kick("a");
    clip.bars = 4;
    let mut song = two_sections(clip, 8, 0);
    song.sections.truncate(1);

    assert_eq!(
        kick_ticks(&resize_drums(&song, 0)),
        kick_ticks(&song),
        "a zero-bar resize falls back to the clip's own length"
    );
}

/// The same song with its drum row looping on `bars` instead.
fn resize_drums(song: &Song, bars: u16) -> Song {
    let mut out = song.clone();
    for section in &mut out.sections {
        if let Some(reference) = section.patterns.get_mut(&Part::Drums) {
            reference.bars = Some(bars);
        }
    }
    out
}

/// Every kick onset in the flattened song, in order.
fn kick_ticks(song: &Song) -> Vec<u32> {
    song.flatten()
        .lanes
        .iter()
        .filter(|lane| lane.lane == Lane::Kick)
        .flat_map(|lane| lane.notes.iter().map(|note| note.start_tick))
        .collect()
}

/// The same, read back out of the exported MIDI file.
fn exported_kick_ticks(song: &Song) -> Vec<u32> {
    let bytes = song_to_smf(song);
    let smf = Smf::parse(&bytes).expect("a parseable file");
    let mut out = Vec::new();
    for track in &smf.tracks {
        let mut at = 0u32;
        for event in track {
            at += event.delta.as_int();
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { key, vel },
                ..
            } = event.kind
            {
                if key.as_int() == 36 && vel.as_int() > 0 {
                    out.push(at);
                }
            }
        }
    }
    out.sort_unstable();
    out
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
        song_seed: 1,
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
                bars: None,
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

#[test]
fn a_single_part_flatten_keeps_that_part_rather_than_a_stand_in() {
    // ⛔ **The file name and the file must not contradict each other.**
    // `Pattern` has no honest value for "the whole song", so a whole-song
    // flatten uses a stand-in — but `pattern_to_smf` writes its track name from
    // this field, and the stems path feeds a *single-part* flatten straight to
    // it. Without carrying the part through, every melodic stem landed on disk
    // as `FMM Melody.mid` with a track called `trap — Drums` inside it.
    let song = song("trap", 5);
    for part in [Part::Drums, Part::Chords, Part::Melody, Part::Counter] {
        let flat = song.flatten_parts(Some(&[part]));
        if flat.note_count() == 0 {
            continue;
        }
        assert_eq!(
            flat.part, part,
            "a {part:?} stem reported itself as another part"
        );
    }

    // And the whole-song flatten still uses the stand-in, because there is no
    // honest alternative — it never reaches the writer.
    assert_eq!(song.flatten().part, Part::Drums);
}

#[test]
fn a_stem_and_its_track_in_the_multi_track_file_agree() {
    // ⛔ **The strongest form of this module's claim, and the one the tick-set
    // cross-check above cannot make.** `song_to_smf` tiles a part into a track;
    // `pattern_to_smf(flatten_parts([part]))` is the same part as a stem file.
    // They are two different routes to the same notes and they must produce the
    // same bytes' worth of note-ons — otherwise a producer who drags the
    // multi-track file gets one performance and one who drags the stems gets
    // another.
    //
    // It caught a real divergence: a slide's destination note-on is emitted at
    // `start + len / 2`, and the song exporter sampled the decay ramp *there*
    // while the flatten baked it in at the note's onset. Every shipped model,
    // 1–8 notes per song, quieter in the multi-track file than in the stem.
    let mut clip = one_bar_kick("a");
    clip.bars = 4;
    clip.lanes = vec![engine::pattern::LaneTrack {
        lane: Lane::Sub,
        notes: (0..4)
            .map(|bar| Note {
                start_tick: bar * PPQ * 4,
                // Long, so the slide's halfway point is far from the onset and
                // the two sampling points cannot coincide by luck.
                len_ticks: PPQ * 2,
                pitch: 30,
                vel: 120,
                model_vel: None,
                slide_to_pitch: Some(37),
                articulation: None,
            })
            .collect(),
    }];

    let mut song = two_sections(clip, 1, 4);
    song.sections[1].decay = true;

    let whole = exported_on_ticks(&song_to_smf(&song));
    let stem = exported_on_ticks(&engine::midi::pattern_to_smf(
        &song.flatten_parts(Some(&[Part::Drums])),
    ));

    assert!(!stem.is_empty(), "the stem carries no notes at all");
    assert_eq!(
        whole, stem,
        "the multi-track file and the stem disagree — same part, same song, two \
         different performances"
    );
}
