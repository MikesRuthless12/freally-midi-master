//! Reading a Standard MIDI File back into a [`Pattern`] (TASK-040T).
//!
//! ⛔ **This exists so a producer's own MIDI can train a workflow.** Mike:
//! *"you should be able to drag in MIDI from the file explorer to train your
//! original artist/workflow."* The requirement that shapes the whole module is
//! the one beside it: **one measurement path, not two.** The fit reads a
//! `Pattern` and nothing else, so a file has to *become* a `Pattern` — a second
//! feature extractor for files would fit differently from the one for
//! generations, and a model trained from a folder would not match one trained
//! from the same music generated.
//!
//! ## What is recovered, and what is honestly not
//!
//! Notes, their timing rebased onto this engine's PPQ, velocities, the tempo and
//! the meter. **Not** the key or the scale: an SMF may carry a key signature and
//! most exported loops do not, and guessing one from the pitches would be a
//! measurement presented where there was none. The fit does not read either, so
//! the default costs nothing — but a caller that starts reading `scale` off an
//! imported pattern is reading an assumption.
//!
//! ⚠ **Drum lanes are recovered by General MIDI note**, the inverse of
//! [`crate::midi::gm_drum_note`]. A file whose drums were written against a
//! different map arrives on the wrong lanes, which is a real limitation and not
//! a bug we can fix from inside the file: GM is the only shared vocabulary there
//! is. A note on no known lane is dropped rather than guessed onto one.

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

use crate::generators::drums::LANE_ORDER;
use crate::midi::gm_drum_note;
use crate::pattern::{Lane, LaneTrack, Note, Part, Pattern, Scale, PPQ};

/// The longest file this will read, in notes.
///
/// ⛔ A bound rather than trust: the bytes come from a file a producer picked,
/// and a malformed or enormous one must not turn a training run into an
/// allocation the host cannot survive. Ten thousand notes is far past any loop
/// and far short of a problem.
const MAX_NOTES: usize = 10_000;

/// Read a type-0 or type-1 SMF as one pattern for `part`.
///
/// Every track is merged: a file exported per instrument still describes one
/// clip, and which track a note sat on says nothing about which lane it is.
pub fn smf_to_pattern(bytes: &[u8], part: Part, id: &str) -> Result<Pattern, String> {
    let smf = Smf::parse(bytes).map_err(|error| format!("not a MIDI file: {error}"))?;

    let file_ppq = match smf.header.timing {
        Timing::Metrical(ticks) => u32::from(ticks.as_int()).max(1),
        // ⛔ Refused rather than approximated. SMPTE timing is in frames per
        // second, so its ticks are wall-clock and this engine's are musical:
        // "rebasing" one onto the other needs a tempo the file may change
        // half-way through, and a silently wrong result is worse than a refusal
        // a producer can act on.
        Timing::Timecode(..) => {
            return Err("this file is timed in SMPTE frames rather than musical ticks".to_owned())
        }
    };

    // ⚠ **An `Option`, not a 120.0 sentinel.** The first cut compared against
    // 120.0 to mean "not set yet" — and 120 BPM is exactly 500,000 µs per
    // quarter, the single most common tempo any file declares. So a file whose
    // map went 120 → 140 read back as 140: the *last* tempo, which is the
    // outcome the rule below exists to avoid.
    let mut bpm: Option<f32> = None;
    let mut time_sig_num = 4_u8;
    let mut time_sig_den = 4_u8;
    let mut open: Vec<(u8, u32, u8, u8)> = Vec::new();
    let mut notes: Vec<(u8, Note)> = Vec::new();

    for track in &smf.tracks {
        let mut at: u64 = 0;
        for event in track {
            at += u64::from(event.delta.as_int());
            // Rebased as we go, so a long file cannot drift: integer maths on
            // the absolute tick rather than on each delta.
            let tick = rebase(at, file_ppq);

            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) => {
                    let us = us.as_int().max(1);
                    // ⚠ The **first** tempo wins. A file with a tempo map is
                    // describing something this pattern cannot hold — one clip,
                    // one tempo — and taking the last would silently describe
                    // the end of the file.
                    bpm.get_or_insert(60_000_000.0 / us as f32);
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, ..)) => {
                    time_sig_num = num.max(1);
                    time_sig_den = 1u8.checked_shl(u32::from(den_pow)).unwrap_or(4).max(1);
                }
                TrackEventKind::Midi { channel, message } => match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        open.push((channel.as_int(), tick, key.as_int(), vel.as_int()));
                    }
                    // ⚠ A note-on with velocity zero **is** a note-off, and a
                    // great many exporters write them that way. Treating them as
                    // onsets would double every note in the file.
                    MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                        close(&mut open, &mut notes, channel.as_int(), key.as_int(), tick);
                    }
                    _ => {}
                },
                _ => {}
            }

            if notes.len() > MAX_NOTES {
                return Err(format!("this file has more than {MAX_NOTES} notes"));
            }
        }

        // A note left hanging at the end of a track is given the length it had
        // when the track stopped, rather than being dropped: an exporter that
        // omits the final note-off is common, and dropping the last note of
        // every phrase would bias every measurement taken from the file.
        // ⚠ `at` is already the sum of every delta in this track — the loop
        // above accumulated it. Re-walking the events to add them up again
        // doubled the traversal of every imported file for a number in hand.
        let end = rebase(at, file_ppq);
        while let Some((channel, start, key, vel)) = open.pop() {
            notes.push((
                channel,
                Note {
                    start_tick: start,
                    len_ticks: end.saturating_sub(start).max(1),
                    pitch: key,
                    vel: vel.max(1),
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                },
            ));
        }
    }

    if notes.is_empty() {
        return Err("this file carries no notes".to_owned());
    }

    notes.sort_by_key(|(_, note)| (note.start_tick, note.pitch));
    let lanes = into_lanes(part, notes);
    if lanes.is_empty() {
        return Err("none of the notes in this file land on a lane this part can use".to_owned());
    }

    let last = lanes
        .iter()
        .flat_map(|track| track.notes.iter())
        // ⚠ Saturating: a file declaring `ppq: 1` with a huge delta can rebase
        // to a tick near `u32::MAX`, and a plain `+` would wrap to a small
        // number — a wrong bar count rather than a crash, but wrong silently.
        .map(|note| note.start_tick.saturating_add(note.len_ticks))
        .max()
        .unwrap_or(0);
    let per_bar = PPQ * u32::from(time_sig_num) * 4 / u32::from(time_sig_den).max(1);
    let bars = last
        .div_ceil(per_bar.max(1))
        .max(1)
        .min(u32::from(u16::MAX)) as u16;

    Ok(Pattern {
        id: id.to_owned(),
        part,
        artist_id: String::new(),
        seed: 0,
        song_seed: 0,
        bars,
        bpm: bpm.unwrap_or(120.0),
        time_sig_num,
        time_sig_den,
        // ⚠ Not recovered — see the module note. The fit reads neither.
        key_root: 0,
        scale: Scale::NaturalMinor,
        lanes,
        ppq: PPQ,
        mood: None,
        loop_region: None,
        clip_region: None,
    })
}

/// This engine's tick for a tick counted at the file's own resolution.
fn rebase(tick: u64, file_ppq: u32) -> u32 {
    let scaled = tick * u64::from(PPQ) / u64::from(file_ppq);
    scaled.min(u64::from(u32::MAX)) as u32
}

/// Close the most recent open note on this channel and pitch.
///
/// Most recent rather than first: two note-ons on one pitch before either off is
/// legal and is what a held-then-restruck note looks like, and pairing the off
/// with the *older* one would leave the newer hanging to the end of the track.
fn close(
    open: &mut Vec<(u8, u32, u8, u8)>,
    notes: &mut Vec<(u8, Note)>,
    channel: u8,
    key: u8,
    tick: u32,
) {
    let Some(at) = open
        .iter()
        .rposition(|(ch, _, k, _)| *ch == channel && *k == key)
    else {
        // A note-off with no note-on is an exporter's mistake, not ours.
        return;
    };
    let (_, start, pitch, vel) = open.remove(at);
    notes.push((
        channel,
        Note {
            start_tick: start,
            len_ticks: tick.saturating_sub(start).max(1),
            pitch,
            vel: vel.max(1),
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
        },
    ));
}

/// The lanes a set of notes becomes, for this part.
fn into_lanes(part: Part, notes: Vec<(u8, Note)>) -> Vec<LaneTrack> {
    if part != Part::Drums {
        // One line. The pitched parts each write a single lane, so a file read
        // as a melody is a melody however many channels it arrived on.
        let lane = match part {
            Part::Melody => Lane::Melody,
            Part::Counter => Lane::Counter,
            Part::Bass => Lane::Bass,
            _ => Lane::Chords,
        };
        return vec![LaneTrack {
            lane,
            notes: notes.into_iter().map(|(_, note)| note).collect(),
        }];
    }

    // ⚠ The inverse of `gm_drum_note`, built once from the lane list rather than
    // restated as a second table — two tables mapping the same thing is how one
    // of them starts being wrong.
    let mut tracks: Vec<LaneTrack> = Vec::new();
    for (_, note) in notes {
        let Some(lane) = LANE_ORDER
            .iter()
            .copied()
            .find(|lane| gm_drum_note(*lane) == note.pitch)
        else {
            // Dropped rather than guessed onto a lane. A note on no known GM
            // drum is something this map cannot name, and putting it on the
            // nearest one would invent percussion the producer never played.
            continue;
        };

        match tracks.iter_mut().find(|track| track.lane == lane) {
            Some(track) => track.notes.push(note),
            None => tracks.push(LaneTrack {
                lane,
                notes: vec![note],
            }),
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::pattern_to_smf;

    /// A pattern, written and read back.
    fn round_trip(pattern: &Pattern) -> Pattern {
        let bytes = pattern_to_smf(pattern);
        smf_to_pattern(&bytes, pattern.part, "read").expect("what we wrote must read")
    }

    fn clip(part: Part, lane: Lane, pitches: &[(u32, u8)]) -> Pattern {
        Pattern {
            id: "written".into(),
            part,
            artist_id: "trap".into(),
            seed: 7,
            song_seed: 7,
            bars: 4,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane,
                notes: pitches
                    .iter()
                    .map(|(tick, pitch)| Note {
                        start_tick: *tick,
                        len_ticks: PPQ / 4,
                        pitch: *pitch,
                        vel: 100,
                        model_vel: None,
                        slide_to_pitch: None,
                        articulation: None,
                    })
                    .collect(),
            }],
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    #[test]
    fn what_the_writer_wrote_is_what_the_reader_reads() {
        // ⛔ The strongest available check on both halves: the exporter is
        // already gated against a real SMF parser, so agreeing with it is
        // agreeing with the format.
        let written = clip(Part::Melody, Lane::Melody, &[(0, 60), (480, 63), (960, 67)]);
        let read = round_trip(&written);

        assert_eq!(read.lanes.len(), 1);
        let got: Vec<(u32, u8)> = read.lanes[0]
            .notes
            .iter()
            .map(|n| (n.start_tick, n.pitch))
            .collect();
        assert_eq!(got, vec![(0, 60), (480, 63), (960, 67)]);
        assert_eq!(read.bpm.round(), 140.0);
        assert_eq!(read.ppq, PPQ);
    }

    #[test]
    fn a_drum_file_comes_back_on_the_lanes_it_was_written_from() {
        // ⛔ The writer replaces each note's pitch with its lane's GM note, so a
        // two-lane pattern is the only way to test the inverse — one lane
        // carrying two "pitches" would be written as two hits of that one lane,
        // which is what the exporter is *for*.
        let mut written = clip(Part::Drums, Lane::Kick, &[(0, 0), (960, 0)]);
        written.lanes.push(LaneTrack {
            lane: Lane::Snare,
            notes: vec![Note {
                start_tick: 480,
                len_ticks: PPQ / 4,
                pitch: 0,
                vel: 100,
                model_vel: None,
                slide_to_pitch: None,
                articulation: None,
            }],
        });

        let read = round_trip(&written);
        let lanes: Vec<Lane> = read.lanes.iter().map(|track| track.lane).collect();
        assert!(lanes.contains(&Lane::Kick), "{lanes:?}");
        assert!(lanes.contains(&Lane::Snare), "{lanes:?}");
    }

    #[test]
    fn a_note_on_no_known_drum_is_dropped_rather_than_guessed_onto_a_lane() {
        // Putting it on the nearest lane would invent percussion the producer
        // never played, and the fit would then measure it. Pitch 3 is below
        // every GM drum note, and read as drums it maps to nothing.
        let stray = clip(Part::Melody, Lane::Melody, &[(0, 3)]);
        let bytes = pattern_to_smf(&stray);

        let error = smf_to_pattern(&bytes, Part::Drums, "read").unwrap_err();
        assert!(error.contains("lane"), "{error}");
    }

    #[test]
    fn an_empty_file_is_refused_rather_than_counted_as_a_kept_generation() {
        // ⛔ Otherwise the thirty-generation floor could be cleared with thirty
        // empty files, which is the failure the floor exists to prevent.
        let empty = clip(Part::Melody, Lane::Melody, &[]);
        let bytes = pattern_to_smf(&empty);
        let error = smf_to_pattern(&bytes, Part::Melody, "read").unwrap_err();
        assert!(error.contains("no notes"), "{error}");
    }

    #[test]
    fn rubbish_is_refused_by_name_rather_than_panicking() {
        let error = smf_to_pattern(b"this is not a midi file", Part::Melody, "read").unwrap_err();
        assert!(error.contains("MIDI"), "{error}");
    }

    #[test]
    fn a_file_at_another_resolution_is_rebased_rather_than_played_at_the_wrong_speed() {
        // A 96-PPQ file — the old hardware default, and still what a great many
        // exports use — has a quarter note at tick 96, not 960.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"MThd");
        bytes.extend(6u32.to_be_bytes());
        bytes.extend(0u16.to_be_bytes()); // format 0
        bytes.extend(1u16.to_be_bytes()); // one track
        bytes.extend(96u16.to_be_bytes()); // 96 ticks per quarter

        let mut track: Vec<u8> = Vec::new();
        track.extend([0x00, 0x90, 60, 100]); // note on at 0
        track.extend([96, 0x80, 60, 0]); // note off one quarter later
        track.extend([0x00, 0xFF, 0x2F, 0x00]); // end of track

        bytes.extend(b"MTrk");
        bytes.extend((track.len() as u32).to_be_bytes());
        bytes.extend(track);

        let read = smf_to_pattern(&bytes, Part::Melody, "read").expect("a 96-PPQ file must read");
        assert_eq!(read.lanes[0].notes[0].start_tick, 0);
        assert_eq!(
            read.lanes[0].notes[0].len_ticks, PPQ,
            "a quarter note at 96 PPQ is a quarter note at ours"
        );
    }

    #[test]
    fn a_note_on_with_zero_velocity_closes_a_note_rather_than_opening_one() {
        // ⛔ A great many exporters write note-offs this way. Treating them as
        // onsets would double every note in the file, and every density the fit
        // measured from it.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"MThd");
        bytes.extend(6u32.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend(1u16.to_be_bytes());
        bytes.extend(960u16.to_be_bytes());

        let mut track: Vec<u8> = Vec::new();
        track.extend([0x00, 0x90, 60, 100]);
        track.extend([0x60, 0x90, 60, 0]); // note-on, velocity 0
        track.extend([0x00, 0xFF, 0x2F, 0x00]);

        bytes.extend(b"MTrk");
        bytes.extend((track.len() as u32).to_be_bytes());
        bytes.extend(track);

        let read = smf_to_pattern(&bytes, Part::Melody, "read").expect("it must read");
        assert_eq!(read.lanes[0].notes.len(), 1, "the file has one note in it");
    }
}
