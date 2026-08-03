//! Standard MIDI File output.
//!
//! A `Pattern` becomes a type-0 SMF (one track, every lane merged); a `Song`
//! becomes type-1 — a conductor track carrying the tempo map and the section
//! markers, then one track per part. PPQ is [`crate::pattern::PPQ`], so the roll
//! subdivisions land on whole ticks.
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

use crate::pattern::{Lane, Note, Part, Pattern, Scale, ScaleCharacter, Section, Song, PPQ};

/// General MIDI drum note numbers, so a drum pattern is auditionable in any
/// DAW without a kit loaded.
///
/// **Every drum lane must map to a distinct note.** Two lanes sharing one key
/// on one channel is not a cosmetic clash: their note-ons and note-offs
/// interleave, so one lane's off silences the other's note and the DAW drops
/// whichever it cannot pair. Trap models layer a snap against a clap routinely
/// (`trap.json` lists both), so the two lanes really do coexist.
pub fn gm_drum_note(lane: Lane) -> u8 {
    match lane {
        Lane::Kick => 36,      // Bass Drum 1
        Lane::Snare => 38,     // Acoustic Snare
        Lane::Clap => 39,      // Hand Clap
        Lane::ClosedHat => 42, // Closed Hi-Hat
        Lane::OpenHat => 46,   // Open Hi-Hat
        Lane::Rim => 37,       // Side Stick
        // Claves, not a second Hand Clap: GM has no finger snap, and 39 is
        // already the clap. A sharp, dry transient is the closest voice, and
        // being audibly separate from the clap is the point.
        Lane::Snap => 75, // Claves
        Lane::Perc => 47, // Low-Mid Tom
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

/// The SMF key signature for a session: accidentals, and whether it is minor.
///
/// Positive counts sharps, negative counts flats — the file format has no way
/// to say more than that. Two consequences worth knowing rather than
/// rediscovering in a DAW:
///
/// - **A mode is written as its parallel major or minor.** There is no way to
///   spell Dorian or Mixolydian in a key signature, so the flag reports the
///   scale's third and the count is the ordinary signature of the tonic. A
///   piano roll then highlights a key that is close rather than exact, which is
///   the best the format allows.
/// - **The spelling is not the model's.** A [`Pattern`] carries `key_root` as a
///   pitch class, so `"Ebm"` and `"D#m"` have already become the same 3 by the
///   time they arrive here. Six accidentals goes to sharps for that reason —
///   there is nothing left to prefer flats by.
fn key_signature(key_root: u8, scale: Scale) -> (i8, bool) {
    // ⛔ **Deferred to `theory::scale_character`, which is the one place that
    // answers "is this scale dark or bright".** This was a hand-written match
    // over every scale — fine at twelve, a drift hazard at forty-one — and then
    // briefly its own interval rule, which promptly disagreed with the character
    // table about the major blues scale: it carries *both* thirds, so "has a
    // minor third" called it minor while the table called it bright. Two rules
    // for one question is how a roll ends up tinting a scale bright over an
    // export that says minor. There is one rule now, and
    // `the_character_decides_the_signature` is what holds them together.
    //
    // Neutral is the only case left to decide here, and the third decides it:
    // the symmetric scales have no dark/bright opinion, so the format gets the
    // nearest thing it can express.
    let degrees = crate::theory::scale_semitones(scale);
    let minor = match crate::theory::scale_character(scale) {
        ScaleCharacter::Dark => true,
        ScaleCharacter::Bright => false,
        ScaleCharacter::Neutral => !degrees.contains(&4),
    };

    // A minor key borrows its signature from the major a minor third above.
    //
    // Reduced *before* the third is added: `key_root` is a `u8` off the wire on
    // the song path, and 253 or above overflowed the addition.
    let key_root = key_root % 12;
    let major_pc = if minor { (key_root + 3) % 12 } else { key_root };

    // Each step clockwise round the circle of fifths adds one sharp, and a
    // fifth is seven semitones.
    let sharps = (u16::from(major_pc) * 7 % 12) as i8;
    // 7 through 11 sharps are the same keys as 5 through 1 flats, spelled with
    // fewer accidentals — C# major is written as D♭.
    if sharps > 6 {
        (sharps - 12, minor)
    } else {
        (sharps, minor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event {
    tick: u32,
    /// Note-offs sort before note-ons at the same tick, so a re-struck note
    /// does not silence its own retrigger.
    is_on: bool,
    channel: u8,
    key: u8,
    velocity: u8,
    /// The tick the note this event belongs to *started* at — the same value on
    /// the on and its off.
    ///
    /// ⛔ **This is what keeps a pair together when one end is filtered.** The
    /// song export drops notes that start past the end of a section, and
    /// deciding that per *event* dropped the on and kept the off: an unmatched
    /// note-off then landed inside the next section, on the same channel and
    /// key, where a DAW pairs it with that section's own note and cuts it dead.
    /// Judging both ends by the onset is what makes the filter operate on
    /// notes rather than on halves of them.
    origin: u32,
}

/// How far a slide's two notes overlap: a 32nd note.
///
/// Long enough that no sampler reads a gap and retriggers the envelope, short
/// enough that the origin pitch is not still sounding well into the
/// destination.
const SLIDE_OVERLAP_TICKS: u32 = PPQ / 8;

/// Push a note's on and off, both stamped with `origin` — the tick the note the
/// pair belongs to started at. For a slide's second note that is still the
/// *original* note's onset, so the whole gesture is kept or dropped together.
fn push_note(
    events: &mut Vec<Event>,
    channel: u8,
    key: u8,
    velocity: u8,
    on: u32,
    off: u32,
    origin: u32,
) {
    events.push(Event {
        tick: on,
        is_on: true,
        channel,
        key,
        velocity,
        origin,
    });
    events.push(Event {
        tick: off,
        is_on: false,
        channel,
        key,
        velocity: 0,
        origin,
    });
}

fn events_for(pattern: &Pattern) -> Vec<Event> {
    let mut events = Vec::new();

    for lane in &pattern.lanes {
        let pitched = is_pitched(lane.lane);
        let channel = if pitched { 0 } else { DRUM_CHANNEL };

        for note in &lane.notes {
            // The clip's own start and end (TASK-041E). A trimmed clip has to
            // *export* trimmed, or the markers are a boundary the producer can
            // see and the file does not have.
            if !pattern.within_clip(note) {
                continue;
            }
            let key = if pitched {
                note.pitch
            } else {
                gm_drum_note(lane.lane)
            };
            let len = note.len_ticks.max(1);
            let velocity = note.vel.clamp(1, 127);

            // Only a pitched lane can slide: a drum lane's key *is* its voice,
            // so "sliding" one would just be a different drum. A slide onto the
            // note's own pitch is a no-op, and emitting it would put two notes
            // on one key — the collision the note-off pairing cannot survive.
            let destination = note
                .slide_to_pitch
                .filter(|d| pitched && *d != key && len >= 4);

            match destination {
                // Two overlapping notes, per this module's header: the
                // destination's note-on lands while the origin is still held,
                // and the origin's note-off follows it. Both stay inside the
                // note's own span, so a slide never lengthens the pattern.
                // ⛔ Saturating throughout: these ticks are untrusted on the song
                // path — `song_smf` takes a whole `Song` from the webview — and
                // `voice.rs` already writes down why that matters here.
                Some(destination) => {
                    let slide_at = note.start_tick.saturating_add(len / 2);
                    let overlap = SLIDE_OVERLAP_TICKS.clamp(1, len / 4);
                    push_note(
                        &mut events,
                        channel,
                        key,
                        velocity,
                        note.start_tick,
                        slide_at.saturating_add(overlap),
                        note.start_tick,
                    );
                    push_note(
                        &mut events,
                        channel,
                        destination,
                        velocity,
                        slide_at,
                        note.start_tick.saturating_add(len),
                        note.start_tick,
                    );
                }
                None => push_note(
                    &mut events,
                    channel,
                    key,
                    velocity,
                    note.start_tick,
                    note.start_tick.saturating_add(len),
                    note.start_tick,
                ),
            }
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

    // Key signature. Drums do not care, but the 808 lane is pitched and the
    // clip is dropped into a project that already has a key — a DAW that reads
    // this transposes and highlights against the right one.
    let (sharps, minor) = key_signature(pattern.key_root, pattern.scale);
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)),
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

/// How quiet a decaying section gets by its last bar, as a share of the
/// velocity it started at.
///
/// Not zero: an outro that reaches silence before it ends leaves the producer
/// dragging in bars of nothing, and every DAW draws that as the clip being
/// broken rather than as a fade.
const DECAY_FLOOR: f32 = 0.35;

/// The tracks a song is written as, in the order they appear in the file.
///
/// Fixed rather than "whichever parts turned up", so two songs from the same
/// artist import into a DAW with their tracks in the same rows.
const TRACK_ORDER: [Part; 5] = [
    Part::Drums,
    Part::Chords,
    Part::Melody,
    Part::Counter,
    Part::Bass,
];

fn part_track_name(part: Part) -> &'static str {
    // US-011 names these exactly: "FMM Drums", ... — the producer sees which
    // rows came from this plugin in a project that already has twenty.
    match part {
        Part::Drums => "FMM Drums",
        Part::Chords => "FMM Chords",
        Part::Melody => "FMM Melody",
        Part::Counter => "FMM Counter",
        Part::Bass => "FMM Bass",
    }
}

fn section_marker(section: &Section) -> String {
    format!("{:?}", section.kind).to_uppercase()
}

/// Every event one part contributes across the whole song.
///
/// ⛔ **A section plays its clip on a loop, so the clip is tiled rather than
/// placed once.** A sixteen-bar verse over a four-bar pattern is that pattern
/// four times; writing it once would export three bars of silence out of every
/// four, which is the bug that makes an exported song sound like it is missing
/// most of itself.
fn song_events_for(song: &Song, part: Part, ticks_per_bar: u32) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    let beat_ticks = (ticks_per_bar / u32::from(song.time_sig_num.max(1))).max(1);

    for section in &song.sections {
        let Some(reference) = section.patterns.get(&part) else {
            continue;
        };
        let Some(pattern) = song.pattern(reference) else {
            continue;
        };

        // ⛔ **Saturating, because these ticks are untrusted.** A `Song` reaches
        // here from the webview — a project file somebody else saved, or
        // devtools — exactly as the notes in `voice.rs` do, and that file says
        // the same thing: this workspace sets `panic = "abort"`, so an
        // overflow-checked build turns an arithmetic wrap into the *host*
        // process dying. In the shipped release profile it wraps silently
        // instead and writes the notes at the wrong ticks, which is a corrupted
        // export nobody can explain.
        let section_start = section.start_bar.saturating_mul(ticks_per_bar);
        let section_len = u32::from(section.bars).saturating_mul(ticks_per_bar);
        let clip_len = u32::from(pattern.bars).saturating_mul(ticks_per_bar).max(1);

        // The drop-out (TASK-066): the last beats of the section are silent so
        // whatever follows lands. Measured from the section's end rather than
        // the clip's, which is why it cannot live in the pattern.
        let sounding = section_len
            .saturating_sub(u32::from(section.drop_out_beats).saturating_mul(beat_ticks));

        // Computed once and reused for every repeat: `events_for` allocates,
        // walks every note and sorts, and it is a pure function of the pattern —
        // so recomputing it per tile was doing that work up to four times per
        // section for an identical answer, which the outer sort then discarded
        // anyway.
        let clip_events = events_for(pattern);

        // ⛔ **A counted loop, not `while offset < sounding`.** With the
        // increment written as `offset += clip_len` the loop is a state machine
        // over `k * clip_len mod 2^32`: release builds have no overflow checks,
        // so for a long enough section over a long enough clip `offset` wraps
        // past `sounding` and cycles forever without ever exceeding it. That
        // spins the thread the host draws its editor from, with no crash and no
        // way out but killing the DAW. A repeat count cannot do that whatever
        // the arithmetic does.
        let repeats = sounding.div_ceil(clip_len);
        for repeat in 0..repeats {
            let offset = repeat.saturating_mul(clip_len);
            for event in clip_events.iter().copied() {
                let tick = section_start
                    .saturating_add(offset)
                    .saturating_add(event.tick);
                // A note whose *start* falls outside the sounding span is not
                // written at all; one that started inside and rings past the
                // end is allowed to finish, exactly as a held note does at a
                // clip boundary.
                //
                // ⛔ Judged on `origin` — the onset of the note this event
                // belongs to — and therefore applied to the note-off as well as
                // the note-on. Testing `event.tick` and gating on `is_on`
                // dropped the on and kept the off, and the orphan landed in the
                // next section where a DAW paired it with that section's note.
                if offset.saturating_add(event.origin) >= sounding {
                    continue;
                }
                let velocity = if section.decay && event.is_on {
                    // ⛔ **Measured across the section, not per whole clip
                    // repeat.** `offset / clip_len` is zero for the whole of a
                    // section exactly one clip long — which is what the shipped
                    // data produces, since `_defaults` authors a 4-bar outro and
                    // the bars chip defaults to 4 — so every decaying outro
                    // exported dead flat while the timeline drew the badge. A
                    // position within the section ramps continuously and reaches
                    // the floor at the end whatever the clip length is.
                    let position = offset.saturating_add(event.tick) as f32;
                    let through = (position / section_len.max(1) as f32).clamp(0.0, 1.0);
                    let scale = 1.0 - (1.0 - DECAY_FLOOR) * through;
                    ((f32::from(event.velocity) * scale).round() as u8).clamp(1, 127)
                } else {
                    event.velocity
                };
                events.push(Event {
                    tick,
                    velocity,
                    ..event
                });
            }
        }
    }

    events.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.is_on.cmp(&b.is_on)));
    events
}

/// Encode a whole song as a type-1 SMF: a conductor track, then one per part.
///
/// This is what US-011's "one drag lays the whole song on the DAW timeline"
/// means, and the reason [`Section::drop_out_beats`] and [`Section::decay`] are
/// fields rather than decoration — they are read here, so a transition the
/// timeline draws is a transition the file contains.
pub fn song_to_smf(song: &Song) -> Vec<u8> {
    let ticks_per_bar = song.ticks_per_bar();

    // ── The conductor track: tempo, meter, key, and where the sections are.
    let mut conductor = Track::new();
    let bpm = if song.bpm.is_finite() && song.bpm > 0.0 {
        song.bpm
    } else {
        120.0
    };
    let us_per_quarter = (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us_per_quarter))),
    });

    let den_pow = match song.time_sig_den {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        16 => 4,
        32 => 5,
        _ => 2,
    };
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
            song.time_sig_num.max(1),
            den_pow,
            24,
            8,
        )),
    });

    let (sharps, minor) = key_signature(song.key_root, song.scale);
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)),
    });

    let title = format!("{} — song", song.artist_id);
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(title.as_bytes())),
    });

    // Markers are how the arrangement survives the export: a producer opening
    // the file sees INTRO / VERSE / HOOK on the DAW's own ruler rather than one
    // undifferentiated run of bars.
    let mut last_tick = 0u32;
    let mut markers: Vec<(u32, String)> = song
        .sections
        .iter()
        .map(|section| {
            (
                section.start_bar.saturating_mul(ticks_per_bar),
                section_marker(section),
            )
        })
        .collect();
    markers.extend(song.sections.iter().flat_map(|s| {
        s.markers
            .iter()
            .map(|m| (s.start_bar.saturating_mul(ticks_per_bar), m.clone()))
    }));
    // Stable, so a section's own kind marker stays ahead of any custom marker
    // sharing its tick.
    markers.sort_by_key(|(tick, _)| *tick);
    for (tick, text) in &markers {
        conductor.push(TrackEvent {
            delta: u28::new(tick.saturating_sub(last_tick)),
            kind: TrackEventKind::Meta(MetaMessage::Marker(text.as_bytes())),
        });
        last_tick = *tick;
    }
    conductor.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let mut tracks = vec![conductor];

    // ── One track per part that plays anywhere in the song.
    for part in TRACK_ORDER {
        let events = song_events_for(song, part, ticks_per_bar);
        if events.is_empty() {
            continue;
        }

        let mut track = Track::new();
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(part_track_name(part).as_bytes())),
        });

        let mut last_tick = 0u32;
        for event in events {
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
        tracks.push(track);
    }

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::new(PPQ as u16)),
        },
        tracks,
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
                model_vel: None,
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
            model_vel: None,
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
                model_vel: None,
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
        loop_region: None,
        clip_region: None,
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
        mood: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{LaneTrack, Part, Scale};

    fn tiny(lane: Lane, notes: Vec<Note>) -> Pattern {
        Pattern {
            loop_region: None,
            clip_region: None,
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
            mood: None,
        }
    }

    fn note(start: u32, len: u32, pitch: u8) -> Note {
        Note {
            model_vel: None,
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

    fn written_key_signature(key_root: u8, scale: Scale) -> (i8, bool) {
        let mut p = tiny(Lane::Bass808, vec![note(0, 240, 36)]);
        p.key_root = key_root;
        p.scale = scale;
        let bytes = pattern_to_smf(&p);
        Smf::parse(&bytes).unwrap().tracks[0]
            .iter()
            .find_map(|e| match e.kind {
                TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor)) => {
                    Some((sharps, minor))
                }
                _ => None,
            })
            .expect("a key signature must be written")
    }

    #[test]
    fn the_key_signature_reaches_the_file_and_names_the_session_key() {
        // The three spellings the dataset actually authors, as pitch classes.
        assert_eq!(
            written_key_signature(6, Scale::NaturalMinor),
            (3, true),
            "F# minor is three sharps"
        );
        assert_eq!(
            written_key_signature(7, Scale::Major),
            (1, false),
            "G major is one sharp"
        );
        assert_eq!(
            written_key_signature(10, Scale::NaturalMinor),
            (-5, true),
            "B♭ minor is five flats"
        );
        assert_eq!(written_key_signature(0, Scale::Major), (0, false));
    }

    #[test]
    fn every_key_stays_inside_the_circle_and_names_its_own_tonic() {
        // Both halves matter: an accidental count outside ±7 is not a legal key
        // signature at all, and a count that does not lead back to the tonic is
        // a legal signature for the wrong key — which no parser would flag.
        for key_root in 0..12u8 {
            for (scale, minor_expected) in [(Scale::Major, false), (Scale::NaturalMinor, true)] {
                let (sharps, minor) = key_signature(key_root, scale);
                assert_eq!(minor, minor_expected, "{key_root} {scale:?}");
                assert!((-7..=7).contains(&sharps), "{key_root} {scale:?}: {sharps}");

                // Walk the circle back: seven semitones per step, and a minor
                // key sits a minor third below the major it borrowed from.
                let major_pc = (i16::from(sharps) * 7).rem_euclid(12) as u8;
                let tonic = if minor { (major_pc + 9) % 12 } else { major_pc };
                assert_eq!(
                    tonic, key_root,
                    "{sharps} sharps says {tonic}, not {key_root}"
                );
            }
        }
    }

    #[test]
    fn a_mode_is_written_as_the_parallel_key_of_its_third() {
        // SMF cannot spell a mode, so the flag reports the third and the count
        // is the tonic's own signature. D dorian is written as D minor rather
        // than as the C major it is diatonic to — the tonic is what the clip is
        // actually centred on.
        assert_eq!(written_key_signature(2, Scale::Dorian), (-1, true));
        assert_eq!(written_key_signature(7, Scale::Mixolydian), (1, false));
        assert_eq!(written_key_signature(9, Scale::MinorPentatonic), (0, true));
        // Phrygian dominant is the exception argued in `key_signature`: a major
        // third, written minor, because its ♭2 and ♭6 land there.
        assert_eq!(written_key_signature(4, Scale::PhrygianDominant), (1, true));
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
    fn every_drum_lane_maps_to_a_distinct_gm_note() {
        // Two lanes on one key + one channel is not a cosmetic clash: their
        // note-offs pair against the wrong note-ons, so one lane silences the
        // other. Clap and Snap both sat on 39 and trap models use both.
        use std::collections::BTreeMap;
        let drums = [
            Lane::Kick,
            Lane::Snare,
            Lane::Clap,
            Lane::ClosedHat,
            Lane::OpenHat,
            Lane::Rim,
            Lane::Snap,
            Lane::Perc,
        ];
        let mut by_note: BTreeMap<u8, Vec<Lane>> = BTreeMap::new();
        for lane in drums {
            by_note.entry(gm_drum_note(lane)).or_default().push(lane);
        }
        let clashes: Vec<_> = by_note.iter().filter(|(_, v)| v.len() > 1).collect();
        assert!(clashes.is_empty(), "lanes sharing a GM note: {clashes:?}");
    }

    #[test]
    fn a_clap_and_a_snap_survive_each_other_in_one_pattern() {
        // The end-to-end shape of the collision: overlapping hits in the two
        // lanes must produce four independently pairable events.
        let mut p = tiny(Lane::Clap, vec![note(0, 480, 0)]);
        p.lanes.push(LaneTrack {
            lane: Lane::Snap,
            notes: vec![note(240, 480, 0)],
        });

        let events = events_for(&p);
        let clap = gm_drum_note(Lane::Clap);
        let snap = gm_drum_note(Lane::Snap);
        assert_ne!(clap, snap);

        // Each key gets exactly one on and one off, in that order.
        for key in [clap, snap] {
            let for_key: Vec<bool> = events
                .iter()
                .filter(|e| e.key == key)
                .map(|e| e.is_on)
                .collect();
            assert_eq!(
                for_key,
                vec![true, false],
                "key {key} is not cleanly paired"
            );
        }
    }

    #[test]
    fn a_slide_is_written_as_two_overlapping_notes() {
        // The module header promises this encoding and nothing used to emit it:
        // slide_to_pitch was dropped on the floor, so every 808 glide exported
        // as a flat retrigger.
        let slide = Note {
            model_vel: None,
            start_tick: 0,
            len_ticks: 960,
            pitch: 33,
            vel: 100,
            slide_to_pitch: Some(40),
            articulation: None,
        };
        let events = events_for(&tiny(Lane::Bass808, vec![slide]));

        let on = |key: u8| {
            events
                .iter()
                .find(|e| e.key == key && e.is_on)
                .unwrap()
                .tick
        };
        let off = |key: u8| {
            events
                .iter()
                .find(|e| e.key == key && !e.is_on)
                .unwrap()
                .tick
        };

        assert_eq!(events.len(), 4, "origin and destination, on and off each");
        // The overlap IS the portamento: the destination starts before the
        // origin ends. A gap here retriggers the envelope and the glide is gone.
        assert!(
            on(40) < off(33),
            "destination must start before the origin ends: on {} vs off {}",
            on(40),
            off(33)
        );
        assert!(on(33) < on(40), "the origin sounds first");
        assert!(off(33) < off(40), "the origin releases first");
        // A slide must not stretch the note beyond its own span.
        assert_eq!(off(40), 960);
    }

    #[test]
    fn a_slide_onto_the_same_pitch_stays_a_single_note() {
        // Otherwise it emits two notes on one key, which is the collision the
        // note-off pairing cannot survive.
        let flat = Note {
            model_vel: None,
            start_tick: 0,
            len_ticks: 960,
            pitch: 33,
            vel: 100,
            slide_to_pitch: Some(33),
            articulation: None,
        };
        assert_eq!(events_for(&tiny(Lane::Bass808, vec![flat])).len(), 2);
    }

    #[test]
    fn a_drum_lane_ignores_a_slide_target() {
        // A drum lane's key is its voice, so sliding one would just be a
        // different drum.
        let hit = Note {
            model_vel: None,
            start_tick: 0,
            len_ticks: 480,
            pitch: 36,
            vel: 100,
            slide_to_pitch: Some(60),
            articulation: None,
        };
        let events = events_for(&tiny(Lane::Kick, vec![hit]));
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.key == gm_drum_note(Lane::Kick)));
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
