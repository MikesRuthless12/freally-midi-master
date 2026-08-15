//! A line's pitch over time, and the notes that come out of it (TASK-058F).
//!
//! ⛔⛔ **The bass does not need transcription and that is the whole trick.**
//! Mike asked whether the bass could come from *"splitting the audio into all
//! midi then taking the lowest part"*. The routing idea is right and the road is
//! much shorter: **low-pass at ~250 Hz and what survives IS the bassline.** It is
//! then a *monophonic* pitch-tracking problem, which is solved — and solved in
//! this repo already, by [`crate::audio::pitch`].
//!
//! ⛔ **The kick removes itself.** It shares the band and it is a transient with
//! no stable period, so the NSDF's clarity peak never reaches its floor and the
//! frame comes back as nothing. Nothing had to be written to exclude it.
//!
//! ⛔ **The trick does not invert, and the roadmap is explicit about it:**
//! high-passing does not isolate a melody, because leads, keys, guitars, brass
//! and vocals all share that band. So the melody road is the same code pointed at
//! a wider band, it finds *the most periodic line in it*, and whether that line
//! is a sung one is [`super::vocal`]'s question rather than this module's.

use engine::pattern::{Lane, LaneTrack, Note};

use crate::audio::pitch::detect_in;

use super::bands;
use super::grid::{division_ticks, Grid};

/// How often the pitch is measured, in seconds.
///
/// ⚠ 20 ms. Fine enough to see a vibrato at 7 Hz (seven samples a cycle) and
/// coarse enough that a four-minute file is twelve thousand frames rather than
/// fifty.
///
/// ⚠ Public because [`super::vocal`] turns a frame count into a share of the
/// file, and doing that against its own idea of the hop is how the two would
/// drift.
pub const HOP_SECONDS: f32 = 0.02;

/// How much audio each measurement looks at.
///
/// ⛔ **It has to hold at least two periods of the lowest note being looked
/// for**, or the NSDF has no lag to find — 60 ms is three cycles of a 50 Hz
/// bass. Longer would smear one note into the next.
const WINDOW_SECONDS: f32 = 0.06;

/// Below this the frame is not a pitch.
///
/// The same floor [`crate::audio::pitch`] uses and for the same reason: a wrong
/// note is worse than none. Here it is also the thing that deletes the kick from
/// the bassline for free.
const MIN_CLARITY: f32 = 0.6;

/// How long a pitch must hold before it is a note.
///
/// ⚠ **Two frames, i.e. 40 ms.** One frame of a different note in the middle of a
/// held one is the tracker wobbling at a note boundary, not a grace note — and
/// writing it out would fill a bassline with 20 ms stabs nobody played.
const MIN_HOLD_FRAMES: usize = 2;

/// One measurement of where the line is.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub at: f32,
    /// The note it is nearest.
    pub note: u8,
    /// How far off that note it is, in cents. **The single strongest signal that
    /// a line was sung** — see [`super::vocal`].
    pub cents: i8,
    pub clarity: f32,
}

/// The band a part is tracked in, and the notes it can hold.
///
/// ⚠ **A type rather than four loose arguments**, because the two call sites
/// differ in every one of them and a transposed pair would be a bassline tracked
/// in the melody band with nothing to say so.
#[derive(Debug, Clone, Copy)]
pub struct Range {
    /// The filter band, in hertz.
    pub band: (Option<f32>, Option<f32>),
    /// The fundamentals worth looking for inside it.
    pub search: (f32, f32),
    /// What the resulting notes are.
    pub lane: Lane,
}

/// Under 250 Hz, which is the bassline and the kick, and the kick opts out.
pub const BASS: Range = Range {
    band: (None, Some(250.0)),
    search: (35.0, 250.0),
    lane: Lane::Bass,
};

/// The lead register. ⚠ Everything sings here — see the module note.
pub const MELODY: Range = Range {
    band: (Some(180.0), Some(2_500.0)),
    search: (120.0, 1_200.0),
    lane: Lane::Melody,
};

/// Measure the pitch of one band, frame by frame.
pub fn track(samples: &[f32], rate: u32, range: Range) -> Vec<Frame> {
    let (low_hz, high_hz) = range.band;
    let filtered = bands::band(samples, rate, low_hz, high_hz);

    // ⛔ **Nothing is tracked in a band the file barely uses.** Filtering always
    // returns *something*: a lead at 523 Hz is only 24 dB down after a 250 Hz
    // low-pass, and the NSDF will happily answer with one of its subharmonics —
    // so a melody-only file produced a confident bassline an octave and a fifth
    // below the tune. The band has to be present before it is read.
    if !carries(samples, &filtered) {
        return Vec::new();
    }

    // ⛔⛔ **Decimated first, and without it this is unaffordable.** The NSDF is
    // `window · lags` and both scale with the sample rate: a 60 ms bass frame at
    // 48 kHz is 2,880 samples against 1,370 lags — four million operations for
    // one twentieth of a second of one line. At 2 kHz the same frame is 120
    // against 57, and the band had nothing above 250 Hz in it to lose.
    //
    // ⚠ **Eight samples per period at the top of the search, not three.** The lag
    // is an integer index and the peak between two of them is recovered by the
    // parabola in `detect_in` — but a parabola through three points a *third of a
    // period* apart is fitting a curve to almost nothing. Eight leaves the
    // interpolation about five cents of error, which is a tenth of the fifty it
    // takes to name the wrong note.
    let (min_hz, max_hz) = range.search;
    let factor = ((rate as f32 / (max_hz * 8.0)).floor() as usize).max(1);
    let (small, small_rate) = bands::decimate(&filtered, rate, factor);

    let hop = ((small_rate as f32 * HOP_SECONDS).round() as usize).max(1);
    let window = ((small_rate as f32 * WINDOW_SECONDS).round() as usize).max(4);

    let mut out: Vec<Frame> = Vec::new();
    let mut at = 0_usize;
    while at + window <= small.len() {
        if let Some(root) = detect_in(&small[at..at + window], small_rate, min_hz, max_hz) {
            if root.clarity >= MIN_CLARITY {
                out.push(Frame {
                    at: at as f32 / small_rate as f32,
                    note: root.note,
                    cents: root.cents,
                    clarity: root.clarity,
                });
            }
        }
        at += hop;
    }
    out
}

/// How far below the whole file a band may sit and still be read as a part.
///
/// ⚠ **−20 dB.** In any real mix the bass band is one of the loudest things
/// there is; a tenth of the file's level is far below anything a producer would
/// call "the bassline" and far above the skirt of a 4-pole filter, which is 24 dB
/// down only one octave past its corner.
const BAND_PRESENT: f32 = 0.1;

/// Is this band actually a part of the file, or only the filter's skirt?
fn carries(samples: &[f32], filtered: &[f32]) -> bool {
    let rms = |buffer: &[f32]| {
        if buffer.is_empty() {
            return 0.0;
        }
        let sum: f64 = buffer.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (sum / buffer.len() as f64).sqrt() as f32
    };
    let whole = rms(samples);
    whole > 0.0 && rms(filtered) / whole >= BAND_PRESENT
}

/// The stretches of the track where one note was held.
///
/// ⚠ **Segmented by the pitch itself rather than by the drum onsets**, and that
/// is deliberate: a bassline slides into its next note as often as it restrikes,
/// and a 240-slide has no transient for an onset detector to find. Where the note
/// changes and *stays* changed is where the note changed.
///
/// ⚠ **Shared with [`super::vocal`] rather than segmented twice.** That module
/// asks how far each note's *unquantised* start sits from the grid, which is a
/// question about exactly these boundaries — and a second segmenter would answer
/// it about slightly different ones.
/// ⚠ **Index ranges rather than slices**, so a caller can also see what lies
/// *between* two runs: [`super::vocal`]'s portamento test wants exactly the
/// frames a note glided through, and given only the sub-slices it had to rescan
/// the whole track per note change — up to four and a half million frame visits
/// on a one-minute melody, for a gap that is usually one frame wide.
pub fn runs(frames: &[Frame]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    let mut from = 0_usize;
    for at in 1..=frames.len() {
        let ended = at == frames.len()
            || frames[at].note != frames[at - 1].note
            // A gap in the track — the line stopped — ends the note.
            || frames[at].at - frames[at - 1].at > HOP_SECONDS * 2.5;
        if ended {
            if at - from >= MIN_HOLD_FRAMES {
                out.push(from..at);
            }
            from = at;
        }
    }
    out
}

/// Fold a pitch track into notes on a grid.
pub fn notes(frames: &[Frame], grid: &Grid) -> Vec<Note> {
    let division = division_ticks(grid.time_sig_num, grid.time_sig_den);
    let mut out: Vec<Note> = Vec::new();

    for run in runs(frames).into_iter().map(|range| &frames[range]) {
        let start = grid.tick_of(run[0].at);
        let end = grid.tick_of(run[run.len() - 1].at + HOP_SECONDS);
        // ⚠ Velocity from clarity, which is the only loudness-adjacent number a
        // pitch tracker has. A confidently-tracked note was a clearly-played one.
        let clarity = run.iter().map(|frame| frame.clarity).sum::<f32>() / run.len() as f32;
        out.push(Note {
            start_tick: start,
            len_ticks: end.saturating_sub(start).max(division),
            pitch: run[0].note,
            vel: (60.0 + clarity.clamp(0.0, 1.0) * 60.0).round() as u8,
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
            reversed: false,
        });
    }

    // ⛔ **Notes that landed on the same division are folded, not both kept.** A
    // wobble across a note boundary can produce two 40 ms "notes" that quantise
    // to the same tick, and a lane with two notes starting on one tick plays them
    // both — a chord in a part whose whole premise is that it is monophonic.
    out.dedup_by(|later, held| {
        let same = later.start_tick == held.start_tick;
        if same {
            held.len_ticks = held.len_ticks.max(later.len_ticks);
        }
        same
    });
    out
}

/// One lane's worth of a pitched part, or `None` if the file has none of it.
pub fn part(samples: &[f32], rate: u32, grid: &Grid, range: Range) -> Option<LaneTrack> {
    let notes = notes(&track(samples, rate, range), grid);
    (!notes.is_empty()).then_some(LaneTrack {
        lane: range.lane,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    const GRID: Grid = Grid::at(120.0, 1, 4, 4);

    /// A buffer of `seconds`, with `notes` played in order as sines.
    fn played(midi: &[u8], seconds: f32) -> Vec<f32> {
        let total = (RATE as f32 * seconds) as usize;
        let each = total / midi.len().max(1);
        let mut out = vec![0.0_f32; total];
        for (index, note) in midi.iter().enumerate() {
            let hz = 440.0 * 2f32.powf((f32::from(*note) - 69.0) / 12.0);
            for i in 0..each {
                let t = i as f32 / RATE as f32;
                out[index * each + i] = (2.0 * std::f32::consts::PI * hz * t).sin() * 0.8;
            }
        }
        out
    }

    #[test]
    fn a_bassline_comes_back_as_the_notes_it_played() {
        let buffer = played(&[36, 41, 39, 36], 2.0);
        let notes = notes(&track(&buffer, RATE, BASS), &GRID);
        let pitches: Vec<u8> = notes.iter().map(|note| note.pitch).collect();
        assert_eq!(pitches, vec![36, 41, 39, 36], "{notes:#?}");
    }

    #[test]
    fn a_kick_in_the_bass_band_removes_itself_and_an_808_does_not() {
        // ⛔⛔ **Both halves, because they are the same band.** The roadmap's claim
        // is that the kick opts out on its own — it is a transient sweeping an
        // octave and a half, so no window of it holds a pitch. What must *not*
        // follow from that is a rule that also throws away the 808, which is a
        // held low note and is exactly what the producer wanted.
        let mut kicks = vec![0.0_f32; RATE as usize];
        super::super::onsets::fixtures::kick(&mut kicks, 0, RATE, 0.06, 0.9);
        super::super::onsets::fixtures::kick(&mut kicks, RATE as usize / 2, RATE, 0.06, 0.9);
        let found = notes(&track(&kicks, RATE, BASS), &GRID);
        assert!(found.is_empty(), "two kicks became a bassline: {found:#?}");

        let held = played(&[33, 33], 1.0);
        assert!(!notes(&track(&held, RATE, BASS), &GRID).is_empty());
    }

    #[test]
    fn a_file_with_nothing_pitched_in_it_yields_no_part() {
        // ⚠ `None` rather than an empty lane. TASK-058H switches a generator OFF
        // when the split produced nothing for it, and it cannot tell "no bass"
        // from "a bass with no notes" if this hands back a track either way.
        let mut state = 0xD1CE_u32;
        let noise: Vec<f32> = (0..RATE)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 9) as f32 / f32::from(u16::MAX) / 128.0 - 0.5
            })
            .collect();
        assert!(part(&noise, RATE, &GRID, BASS).is_none());
    }

    #[test]
    fn the_melody_band_finds_a_lead_where_the_bass_band_finds_nothing() {
        // ⛔ The trick does not invert — the bands are what separate these, not
        // the register alone.
        let lead = played(&[72, 74, 76, 74], 2.0);
        assert!(part(&lead, RATE, &GRID, MELODY).is_some());
        assert!(part(&lead, RATE, &GRID, BASS).is_none());
    }

    #[test]
    fn two_notes_that_quantise_together_are_one_note_rather_than_a_chord() {
        // ⛔ A monophonic part with two notes on the same tick plays them both.
        let frames: Vec<Frame> = (0..6)
            .map(|i| Frame {
                at: i as f32 * 0.002,
                // A wobble at the boundary: three frames of one note, three of
                // its neighbour, all inside one division.
                note: if i < 3 { 40 } else { 41 },
                cents: 0,
                clarity: 0.9,
            })
            .collect();
        let out = notes(&frames, &GRID);
        assert_eq!(out.len(), 1, "{out:#?}");
    }
}
