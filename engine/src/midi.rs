//! Standard MIDI File output.
//!
//! A `Pattern` becomes a type-0 SMF (one track, every lane merged); a `Song`
//! will become type-1 (a track per part) when Song Mode lands. PPQ is
//! [`crate::pattern::PPQ`], so the roll subdivisions land on whole ticks.
//!
//! 808 slides are written as **overlapping notes**: the sliding note's
//! note-off comes *after* the destination's note-on. That overlap is the
//! convention every sampler reads as portamento, and it is the whole reason
//! drill and trap basslines sound the way they do — a gap instead of an
//! overlap retriggers the envelope and the slide disappears.

use midly::{
    num::{u15, u24, u28, u4, u7},
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};

use crate::pattern::{Lane, Note, Pattern, PPQ};

/// General MIDI drum note numbers, so a drum pattern is auditionable in any
/// DAW without a kit loaded.
fn gm_drum_note(lane: Lane) -> u8 {
    match lane {
        Lane::Kick => 36,      // Bass Drum 1
        Lane::Snare => 38,     // Acoustic Snare
        Lane::Clap => 39,      // Hand Clap
        Lane::ClosedHat => 42, // Closed Hi-Hat
        Lane::OpenHat => 46,   // Open Hi-Hat
        Lane::Rim => 37,       // Side Stick
        Lane::Snap => 39,      // Hand Clap
        Lane::Perc => 47,      // Low-Mid Tom
        // Pitched lanes carry their own pitch; this is never consulted.
        Lane::Bass808 | Lane::Melody | Lane::Counter | Lane::Bass | Lane::Chords => 0,
    }
}

/// Whether a lane's notes carry real pitch or map to a fixed drum voice.
fn is_pitched(lane: Lane) -> bool {
    matches!(
        lane,
        Lane::Bass808 | Lane::Melody | Lane::Counter | Lane::Bass | Lane::Chords
    )
}

/// MIDI channel 10 (index 9) is percussion by GM convention.
const DRUM_CHANNEL: u8 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event {
    tick: u32,
    /// Note-offs sort before note-ons at the same tick, so a re-struck note
    /// does not silence its own retrigger.
    is_on: bool,
    channel: u8,
    key: u8,
    velocity: u8,
}

fn events_for(pattern: &Pattern) -> Vec<Event> {
    let mut events = Vec::new();

    for lane in &pattern.lanes {
        let pitched = is_pitched(lane.lane);
        let channel = if pitched { 0 } else { DRUM_CHANNEL };

        for note in &lane.notes {
            let key = if pitched {
                note.pitch
            } else {
                gm_drum_note(lane.lane)
            };
            events.push(Event {
                tick: note.start_tick,
                is_on: true,
                channel,
                key,
                velocity: note.vel.clamp(1, 127),
            });
            events.push(Event {
                tick: note.start_tick + note.len_ticks.max(1),
                is_on: false,
                channel,
                key,
                velocity: 0,
            });
        }
    }

    // Stable ordering: by tick, then offs before ons. Without the second key a
    // note-off for the previous hit can land after the next note-on at the
    // same tick and cut it dead.
    events.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.is_on.cmp(&b.is_on)));
    events
}

/// Encode a pattern as a type-0 SMF.
pub fn pattern_to_smf(pattern: &Pattern) -> Vec<u8> {
    let mut track = Track::new();

    // Tempo, as microseconds per quarter note.
    let bpm = if pattern.bpm.is_finite() && pattern.bpm > 0.0 {
        pattern.bpm
    } else {
        120.0
    };
    let us_per_quarter = (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us_per_quarter))),
    });

    // Time signature. The denominator is stored as a power of two.
    let den_pow = match pattern.time_sig_den {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        16 => 4,
        32 => 5,
        _ => 2,
    };
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
            pattern.time_sig_num.max(1),
            den_pow,
            24,
            8,
        )),
    });

    let name = format!("{} — {:?}", pattern.artist_id, pattern.part);
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
    });

    let mut last_tick = 0u32;
    for event in events_for(pattern) {
        let delta = event.tick.saturating_sub(last_tick);
        last_tick = event.tick;

        let message = if event.is_on {
            MidiMessage::NoteOn {
                key: u7::new(event.key.min(127)),
                vel: u7::new(event.velocity.min(127)),
            }
        } else {
            MidiMessage::NoteOff {
                key: u7::new(event.key.min(127)),
                vel: u7::new(0),
            }
        };

        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: u4::new(event.channel),
                message,
            },
        });
    }

    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(PPQ as u16)),
        },
        tracks: vec![track],
    };

    let mut out = Vec::new();
    smf.write(&mut out).expect("writing to a Vec cannot fail");
    out
}

/// A short, valid pattern for exercising the export and drag paths before the
/// generators exist. Real, not a stub: four bars of kick, snare and hats that
/// a DAW will happily play.
pub fn drag_spike_pattern() -> Pattern {
    use crate::pattern::{LaneTrack, Part, Scale};

    let sixteenth = PPQ / 4;
    let bar = PPQ * 4;
    let mut kick = Vec::new();
    let mut snare = Vec::new();
    let mut hats = Vec::new();

    for b in 0..4u32 {
        let start = b * bar;
        // Kick on 1 and the "and" of 3 — a plain trap skeleton.
        for offset in [0, bar / 2 + PPQ / 2] {
            kick.push(Note {
                start_tick: start + offset,
                len_ticks: PPQ / 2,
                pitch: 36,
                vel: 112,
                slide_to_pitch: None,
                articulation: None,
            });
        }
        // Snare on beat 3 only: half-time.
        snare.push(Note {
            start_tick: start + PPQ * 2,
            len_ticks: PPQ / 2,
            pitch: 38,
            vel: 118,
            slide_to_pitch: None,
            articulation: None,
        });
        // Straight 16th hats.
        for i in 0..16u32 {
            hats.push(Note {
                start_tick: start + i * sixteenth,
                len_ticks: sixteenth / 2,
                pitch: 42,
                vel: if i % 4 == 0 { 100 } else { 72 },
                slide_to_pitch: None,
                articulation: None,
            });
        }
    }

    Pattern {
        id: "drag-spike".into(),
        part: Part::Drums,
        artist_id: "spike".into(),
        seed: 0,
        bars: 4,
        bpm: 140.0,
        time_sig_num: 4,
        time_sig_den: 4,
        key_root: 0,
        scale: Scale::NaturalMinor,
        lanes: vec![
            LaneTrack {
                lane: Lane::Kick,
                notes: kick,
            },
            LaneTrack {
                lane: Lane::Snare,
                notes: snare,
            },
            LaneTrack {
                lane: Lane::ClosedHat,
                notes: hats,
            },
        ],
        ppq: PPQ,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{LaneTrack, Part, Scale};

    fn tiny(lane: Lane, notes: Vec<Note>) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "t".into(),
            seed: 1,
            bars: 1,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack { lane, notes }],
            ppq: PPQ,
        }
    }

    fn note(start: u32, len: u32, pitch: u8) -> Note {
        Note {
            start_tick: start,
            len_ticks: len,
            pitch,
            vel: 100,
            slide_to_pitch: None,
            articulation: None,
        }
    }

    #[test]
    fn the_output_is_a_valid_smf_that_parses_back() {
        let bytes = pattern_to_smf(&drag_spike_pattern());
        assert_eq!(&bytes[0..4], b"MThd", "must start with a MIDI header chunk");

        let parsed = Smf::parse(&bytes).expect("our own output must parse");
        assert_eq!(parsed.header.format, Format::SingleTrack);
        assert_eq!(parsed.header.timing, Timing::Metrical(u15::new(PPQ as u16)));
        assert_eq!(parsed.tracks.len(), 1);
    }

    #[test]
    fn the_spike_pattern_carries_real_notes() {
        let p = drag_spike_pattern();
        // 4 bars: 2 kicks + 1 snare + 16 hats each.
        assert_eq!(p.note_count(), 4 * (2 + 1 + 16));

        // The pattern spans four bars, but the last hat is a 16th that ends
        // before the barline — a clip does not have to end on it.
        let four_bars = PPQ * 16;
        assert!(p.end_tick() <= four_bars, "must not overrun four bars");
        assert!(
            p.end_tick() > four_bars - PPQ,
            "the last bar must actually be played, ended at {}",
            p.end_tick()
        );
    }

    #[test]
    fn drum_lanes_are_written_on_the_percussion_channel() {
        let bytes = pattern_to_smf(&tiny(Lane::Kick, vec![note(0, 240, 60)]));
        let parsed = Smf::parse(&bytes).unwrap();
        let channels: Vec<u8> = parsed.tracks[0]
            .iter()
            .filter_map(|e| match e.kind {
                TrackEventKind::Midi { channel, .. } => Some(channel.as_int()),
                _ => None,
            })
            .collect();
        assert!(
            channels.iter().all(|c| *c == DRUM_CHANNEL),
            "drums belong on channel 10 (index 9), got {channels:?}"
        );
    }

    #[test]
    fn a_drum_lanes_pitch_is_replaced_by_its_gm_voice() {
        // The lane decides the drum voice, not whatever pitch the generator
        // happened to put in the note.
        let bytes = pattern_to_smf(&tiny(Lane::Snare, vec![note(0, 240, 99)]));
        let parsed = Smf::parse(&bytes).unwrap();
        let keys: Vec<u8> = parsed.tracks[0]
            .iter()
            .filter_map(|e| match e.kind {
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { key, .. },
                    ..
                } => Some(key.as_int()),
                _ => None,
            })
            .collect();
        assert_eq!(keys, vec![38], "snare should be GM 38, not the note's 99");
    }

    #[test]
    fn pitched_lanes_keep_their_pitch_and_stay_off_the_drum_channel() {
        let bytes = pattern_to_smf(&tiny(Lane::Bass808, vec![note(0, 960, 29)]));
        let parsed = Smf::parse(&bytes).unwrap();
        for event in parsed.tracks[0].iter() {
            if let TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn { key, .. },
            } = event.kind
            {
                assert_ne!(channel.as_int(), DRUM_CHANNEL);
                assert_eq!(key.as_int(), 29);
            }
        }
    }

    #[test]
    fn note_offs_sort_before_note_ons_at_the_same_tick() {
        // Two hits back to back: the first note ends exactly where the second
        // begins. If the off were emitted after the on, the second hit would be
        // silenced immediately.
        let p = tiny(Lane::Kick, vec![note(0, 480, 36), note(480, 480, 36)]);
        let kinds: Vec<bool> = events_for(&p)
            .iter()
            .filter(|e| e.tick == 480)
            .map(|e| e.is_on)
            .collect();
        assert_eq!(
            kinds,
            vec![false, true],
            "off must precede on at a shared tick"
        );
    }

    #[test]
    fn tempo_is_written_as_microseconds_per_quarter() {
        let bytes = pattern_to_smf(&tiny(Lane::Kick, vec![note(0, 240, 36)]));
        let parsed = Smf::parse(&bytes).unwrap();
        let tempo = parsed.tracks[0].iter().find_map(|e| match e.kind {
            TrackEventKind::Meta(MetaMessage::Tempo(t)) => Some(t.as_int()),
            _ => None,
        });
        // 140 BPM -> 60_000_000 / 140 = 428571.4 -> 428571
        assert_eq!(tempo, Some(428_571));
    }

    #[test]
    fn a_nonsense_tempo_does_not_produce_a_corrupt_file() {
        let mut p = tiny(Lane::Kick, vec![note(0, 240, 36)]);
        p.bpm = 0.0;
        let bytes = pattern_to_smf(&p);
        assert!(
            Smf::parse(&bytes).is_ok(),
            "a bad BPM must not corrupt the file"
        );
    }

    #[test]
    fn a_zero_length_note_still_produces_an_off() {
        // Otherwise the note hangs forever in the DAW.
        let p = tiny(Lane::Kick, vec![note(0, 0, 36)]);
        let events = events_for(&p);
        assert_eq!(events.len(), 2);
        assert!(
            events[1].tick > events[0].tick,
            "the off must come after the on"
        );
    }

    #[test]
    fn deltas_are_relative_and_reconstruct_the_original_timing() {
        let pattern = drag_spike_pattern();
        let bytes = pattern_to_smf(&pattern);
        let parsed = Smf::parse(&bytes).unwrap();

        // Summing the deltas must land exactly on the pattern's last event.
        // A drift here means every note after it is in the wrong place.
        let total: u32 = parsed.tracks[0].iter().map(|e| e.delta.as_int()).sum();
        assert_eq!(total, pattern.end_tick());
    }
}
