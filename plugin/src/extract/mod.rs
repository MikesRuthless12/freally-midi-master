//! Audio in, notes out (TASK-058D / TASK-058F / TASK-058G / TASK-058I).
//!
//! ⛔⛔ **The output type is [`engine::smf_read::SplitPart`], and that is the
//! design.** Mike, 2026-08-10: *"can you ensure that you can drag the audio/midi
//! of any sample into any of the generators and it will split them all the same
//! exact way?"* The only way to *ensure* that is for both roads to end in the
//! same value — so the page, the panel that names what was found, the import, the
//! muting and the undo step are one path with two front doors rather than two
//! features that agree today.
//!
//! ▶ Every part still carries its own `SplitReason`, and the audio ones are
//! separate variants for a reason that enum's own doc gives: a spectral
//! measurement must not wear a MIDI file's kind of certainty.
//!
//! ## The road, in order
//!
//! 1. [`onsets`] finds the hits and what each is made of.
//! 2. [`grid`] reads the tempo and the loop's own length off them (TASK-058I).
//! 3. [`drums`] classifies the hits, in two passes, onto real lanes.
//! 4. [`pitched`] low-passes at 250 Hz and pitch-tracks what survives — the
//!    bassline — then does the same in the lead register for the melody.
//! 5. [`vocal`] decides whether that melodic line was sung, and it is dropped if
//!    it was (TASK-058G).
//! 6. [`chroma`] names the progression and where it changes.
//!
//! ## What this does not do, said here rather than discovered later
//!
//! - ⛔ **No counter-melody.** Separating a second pitched line from the first in
//!   a mixed-down file is source separation, which needs the ML this project
//!   bans. The MIDI path produces `Part::Counter` because a file *keeps its
//!   voices apart*; a waveform does not. TASK-058H then switches that generator
//!   off rather than leaving it armed and empty, which is the honest outcome.
//! - ⛔ **No polyphonic transcription.** See [`chroma`]: the chords come back as
//!   a progression, spelled, not as the voicing that was played.
//! - ⚠ **A hit masked under a louder one on the same frame is gone**, and a
//!   heavily bus-compressed loop loses transients before any of this sees them.

pub mod bands;
pub mod chroma;
pub mod drums;
pub mod grid;
pub mod job;
pub mod onsets;
pub mod pitched;
pub mod vocal;

use serde::Serialize;

use engine::pattern::{LaneTrack, Part, Pattern, Scale, PPQ};
use engine::smf_read::{SplitPart, SplitReason};

use crate::audio::kit::DecodedAudio;

use grid::Grid;

/// The most notes one file may produce.
///
/// ⛔ A bound rather than trust, the same one `smf_read::MAX_NOTES` sets and for
/// the same reason: the bytes are a file a producer picked, and a pathological
/// one must not turn an import into an allocation the host cannot survive. Thirty
/// seconds of 32nd-note percussion across five lanes is a few thousand.
const MAX_NOTES: usize = 10_000;

/// What one audio file was found to contain.
///
/// ⚠ **Declared by hand in `src/state/explorer.ts`** rather than generated, the
/// way `HostSessionInfo` already is: `ts-rs` lives in the engine crate and this
/// is a plugin reply. `parts` is the generated [`SplitPart`], which is the half
/// that actually matters to keep in step.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSplit {
    pub parts: Vec<SplitPart>,
    /// The tempo the loop was read as.
    ///
    /// ⚠ Reported next to the parts so the page can *offer* it rather than adopt
    /// it: the session may be host-synced, and silently retuning a producer's
    /// project because they dragged in a 92 BPM loop is not an import.
    pub bpm: f32,
    /// A line in the lead register measured as sung, and left alone (TASK-058G).
    ///
    /// ⛔ **Reported rather than silently dropped.** Mike asked for *"and leave
    /// the vocals alone"*; a producer who drags in an a-cappella and gets an empty
    /// panel has been told the file is broken. This is what lets the page say what
    /// actually happened.
    pub vocal_left_alone: bool,
}

/// Separate a decoded audio file into the generators its parts belong to.
///
/// `session_bpm` breaks the octave tie in the tempo reading and nothing else —
/// see [`grid::fit`].
pub fn split(
    audio: &DecodedAudio,
    id: &str,
    session_bpm: f32,
    time_sig_num: u8,
    time_sig_den: u8,
) -> Result<AudioSplit, String> {
    let rate = audio.sample_rate;
    if rate == 0 || audio.samples.is_empty() {
        return Err("that file decoded to no audio at all".to_owned());
    }
    let seconds = audio.samples.len() as f32 / rate as f32;

    let hits = onsets::detect(&onsets::analyse(&audio.samples, rate));
    let grid = grid::fit(&hits, seconds, session_bpm, time_sig_num, time_sig_den);

    let mut parts: Vec<SplitPart> = Vec::new();

    // ⛔ **Only when the hits are actually percussion.** Every note start is an
    // onset, so a pad file has plenty of them — and without this a held string
    // chord arrives as a drum pattern of "perc" hits, which is the silent
    // mis-file this whole feature is written to avoid.
    if drums::percussive(&hits) {
        let mut lanes: Vec<LaneTrack> = Vec::new();
        for (lane, note) in drums::classify(&hits, &grid) {
            match lanes.iter_mut().find(|track| track.lane == lane) {
                Some(track) => track.notes.push(note),
                None => lanes.push(LaneTrack {
                    lane,
                    notes: vec![note],
                }),
            }
        }
        if let Some(part) = finish(id, Part::Drums, lanes, &grid, SplitReason::PercussiveBand) {
            parts.push(part);
        }
    }

    if let Some(track) = pitched::part(&audio.samples, rate, &grid, pitched::BASS) {
        if let Some(part) = finish(id, Part::Bass, vec![track], &grid, SplitReason::LowBand) {
            parts.push(part);
        }
    }

    // ⛔⛔ **The melody is tracked, then judged, and only then kept.** Mike: *"and
    // leave the vocals alone"* — and the line that comes out of the lead band is
    // as likely to be the singer as the lead, because leads, keys, guitars, brass
    // and voices all share that register. See [`vocal`] for what is measured and
    // for where it degrades.
    let lead = pitched::track(&audio.samples, rate, pitched::MELODY);
    let verdict = vocal::assess(&lead, &grid, seconds);
    if !verdict.sung {
        let notes = pitched::notes(&lead, &grid);
        if !notes.is_empty() {
            let track = LaneTrack {
                lane: pitched::MELODY.lane,
                notes,
            };
            if let Some(part) = finish(
                id,
                Part::Melody,
                vec![track],
                &grid,
                SplitReason::MelodicBand,
            ) {
                parts.push(part);
            }
        }
    }

    if let Some(track) = chroma::part(&audio.samples, rate, &grid) {
        if let Some(part) = finish(
            id,
            Part::Chords,
            vec![track],
            &grid,
            SplitReason::ChromaTemplate,
        ) {
            parts.push(part);
        }
    }

    if parts.is_empty() && !verdict.sung {
        return Err("nothing in that file could be read as notes".to_owned());
    }

    Ok(AudioSplit {
        parts,
        bpm: grid.bpm,
        vocal_left_alone: verdict.sung,
    })
}

/// Wrap a set of lanes as one part, clipped to the loop the grid found.
///
/// ⛔⛔ **The clip is what makes `grid`'s collapse safe.** When four identical
/// bars are read as a one-bar loop, everything after bar one is the same notes
/// again — and a `Pattern` whose `bars` says one while its lanes hold four is
/// exactly the defect the locked-lane fix recorded: the grid draws a clean bar
/// while `to_midi`, the stems and the host all get the other three.
fn finish(
    id: &str,
    part: Part,
    lanes: Vec<LaneTrack>,
    grid: &Grid,
    reason: SplitReason,
) -> Option<SplitPart> {
    let limit = grid.total_ticks();
    // ⚠ **A budget spelled out, not a counter captured by a `map` closure.** The
    // first cut hid `MAX_NOTES` inside a `take_while` whose counter accumulated
    // across lanes — it worked, and a reader had to reason about closure capture
    // to see that the cap was for the whole part rather than for each lane.
    let mut budget = MAX_NOTES;
    let mut lanes = lanes;
    for track in &mut lanes {
        track.notes.retain(|note| note.start_tick < limit);
        track.notes.truncate(budget);
        budget -= track.notes.len();
        // ⚠ And the tail is trimmed rather than left hanging past the loop: a
        // note that starts inside the clip and ends outside it is a note the roll
        // draws off the end of itself.
        for note in &mut track.notes {
            note.len_ticks = note.len_ticks.min(limit - note.start_tick).max(1);
        }
    }
    lanes.retain(|track| !track.notes.is_empty());

    let notes = lanes.iter().map(|track| track.notes.len() as u32).sum();
    if notes == 0 {
        return None;
    }

    Some(SplitPart {
        part,
        pattern: Pattern {
            id: id.to_owned(),
            part,
            // ⚠ **No artist and no seed**, exactly as an imported `.mid` leaves
            // them: a number invented here would name a seed that reproduces
            // nothing.
            artist_id: String::new(),
            seed: 0,
            song_seed: 0,
            bars: grid.bars,
            bpm: grid.bpm,
            time_sig_num: grid.time_sig_num,
            time_sig_den: grid.time_sig_den,
            // ⚠ Not recovered, the same way `smf_read::assemble` does not recover
            // it. A key could be guessed off the chroma, and a guessed key the fit
            // does not read would be a measurement presented where there was none.
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes,
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        },
        reason,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::onsets::fixtures::{kick, noise, tone};
    use super::*;

    const RATE: u32 = 48_000;

    fn decoded(samples: Vec<f32>) -> DecodedAudio {
        DecodedAudio {
            samples,
            sample_rate: RATE,
        }
    }

    /// Two bars of a plain trap beat at 140: kicks, a backbeat and eighth hats.
    fn drum_loop() -> DecodedAudio {
        let bar = 4.0 * 60.0 / 140.0;
        let seconds = bar * 2.0;
        let mut out = vec![0.0_f32; (RATE as f32 * seconds) as usize];
        let at = |beats: f32| ((beats / 4.0) * bar * RATE as f32) as usize;

        let mut hats = vec![0.0_f32; out.len()];
        for bar_index in 0..2 {
            let offset = bar_index as f32 * 4.0;
            for beat in [0.0, 1.5, 2.5] {
                kick(&mut out, at(offset + beat), RATE, 0.09, 0.9);
            }
            for beat in [1.0, 3.0] {
                // A snare: body, plus the crack that lands in the hat bands.
                tone(&mut out, at(offset + beat), RATE, 210.0, 0.09, 0.5);
                noise(&mut hats, at(offset + beat), RATE, 0.07, 0.5, 0x5_1AB);
            }
            for eighth in 0..8 {
                noise(
                    &mut hats,
                    at(offset + eighth as f32 * 0.5),
                    RATE,
                    0.02,
                    0.18,
                    0x4A7,
                );
            }
        }
        let shaped = bands::band(&hats, RATE, Some(3_000.0), None);
        for (index, sample) in shaped.iter().enumerate() {
            out[index] += sample;
        }
        decoded(out)
    }

    #[test]
    fn a_drum_loop_comes_back_as_a_drum_part() {
        let split = split(&drum_loop(), "loop", 140.0, 4, 4).expect("a drum loop is readable");
        let drums = split
            .parts
            .iter()
            .find(|part| part.part == Part::Drums)
            .unwrap_or_else(|| panic!("no drum part in {:#?}", split.parts));
        assert_eq!(drums.reason, SplitReason::PercussiveBand);
        assert!(
            (split.bpm - 140.0).abs() < 2.0,
            "the tempo came back as {}",
            split.bpm
        );
        let lanes: Vec<_> = drums.pattern.lanes.iter().map(|track| track.lane).collect();
        assert!(lanes.contains(&engine::pattern::Lane::Kick), "{lanes:?}");
    }

    #[test]
    fn a_held_chord_is_not_a_drum_pattern() {
        // ⛔ Every note start is an onset. Without the percussion guard a pad
        // arrives as a grid full of "perc", which is the silent mis-file this
        // whole feature exists to avoid.
        let n = (RATE as f32 * 2.0) as usize;
        let mut out = vec![0.0_f32; n];
        for note in [48_u8, 52, 55] {
            let hz = 440.0 * 2f32.powf((f32::from(note) - 69.0) / 12.0);
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / RATE as f32;
                *sample += 0.3 * (2.0 * std::f32::consts::PI * hz * t).sin();
            }
        }
        let split = split(&decoded(out), "pad", 120.0, 4, 4).expect("a chord is readable");
        assert!(
            !split.parts.iter().any(|part| part.part == Part::Drums),
            "{:#?}",
            split.parts
        );
        assert!(split.parts.iter().any(|part| part.part == Part::Chords));
    }

    #[test]
    fn every_part_is_clipped_to_the_loop_the_grid_found() {
        // ⛔⛔ Four identical bars are read as one, and a pattern whose `bars`
        // says one while its lanes hold four sends three bars nobody can see to
        // the host, to `to_midi` and to the stems.
        let split = split(&drum_loop(), "loop", 140.0, 4, 4).expect("readable");
        for part in &split.parts {
            let limit = u32::from(part.pattern.bars)
                * engine::pattern::ticks_per_bar_of(
                    part.pattern.time_sig_num,
                    part.pattern.time_sig_den,
                );
            for lane in &part.pattern.lanes {
                for note in &lane.notes {
                    assert!(
                        note.start_tick + note.len_ticks <= limit,
                        "a note runs to {} in a clip {limit} long",
                        note.start_tick + note.len_ticks
                    );
                }
            }
        }
    }

    #[test]
    fn an_import_has_no_artist_and_no_seed_and_says_so() {
        // ⚠ The same promise `smf_to_song` makes. A number invented here would
        // name a seed that reproduces nothing.
        let split = split(&drum_loop(), "loop", 140.0, 4, 4).expect("readable");
        for part in &split.parts {
            assert_eq!(part.pattern.artist_id, "");
            assert_eq!(part.pattern.seed, 0);
        }
    }

    #[test]
    fn silence_is_refused_with_a_reason_rather_than_returned_empty() {
        let error = split(&decoded(vec![0.0; RATE as usize]), "quiet", 120.0, 4, 4)
            .expect_err("silence has no notes in it");
        assert!(error.contains("nothing"), "{error}");
    }
}
