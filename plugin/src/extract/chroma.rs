//! What the harmony is doing (TASK-058F).
//!
//! ⛔⛔ **Recognition, not transcription, and the difference is the promise.**
//! The roadmap is explicit and so is Build Philosophy §7: *"Chroma/HPCP features
//! against chord templates give the progression and where it changes, which is
//! exactly what the chords generator consumes … It does not give you every
//! voice."* Note-by-note transcription of a full mix is an ML problem, ML is
//! banned here, and **a feature that claims perfect polyphonic extraction and
//! delivers approximate notes is worse than one that says what it does.**
//!
//! So this answers two questions and refuses the third:
//!
//! - **Which chord** — a root and a quality, from twenty-four templates.
//! - **When it changes** — the harmonic rhythm, which is what an arrangement is.
//! - ⛔ **Not which voice played what.** The clip that comes out is the chord
//!   spelled plainly in one octave. A producer who wanted the pianist's voicing
//!   did not get it and is not told they did.
//!
//! ## Why a Goertzel bank and not an FFT
//!
//! The same answer [`super::bands`] and [`crate::audio::pitch`] both give: an FFT
//! is a dependency in a crate loaded into somebody else's DAW. And here it would
//! also be the wrong shape — an FFT gives linearly spaced bins and this needs
//! *semitones*, which are logarithmic, so the low octaves would land three notes
//! to a bin. Forty-eight Goertzel resonators tuned to the notes themselves is
//! both smaller and better resolved where it matters.

use engine::pattern::{Lane, LaneTrack, Note};

use super::bands;
use super::grid::{division_ticks, Grid};

/// The lowest and highest note the bank listens for.
///
/// C2 to B5. ⚠ **Not the whole keyboard**: below C2 is the bassline's register,
/// where a low-passed 808 would dominate every chroma bin with its own root, and
/// above B5 there is little but harmonics of what is already counted.
const LOW_NOTE: u8 = 36;
const HIGH_NOTE: u8 = 83;

/// The band the harmony is read from.
///
/// ⚠ Deliberately excludes the kick band and the hat band. Neither has a pitch,
/// and both are loud — a kick's energy at 55 Hz lands in the same chroma bin as
/// an A, which would make every trap loop an A minor.
const HARMONY_BAND: (Option<f32>, Option<f32>) = (Some(70.0), Some(2_500.0));

/// How long each chroma measurement is, and how far apart they are.
///
/// ⚠ A quarter of a second is about a beat at 240 BPM and half of one at 120 —
/// short enough to see a chord change on the beat, long enough that its lowest
/// note (65 Hz) has sixteen cycles in the window.
const FRAME_SECONDS: f32 = 0.25;
const HOP_SECONDS: f32 = 0.125;

/// The shortest a chord may be and still be written down.
///
/// ⛔ **Half a second.** Chord templates always answer *something*, so a
/// frame-by-frame reading flickers between relatives — C major and A minor share
/// two of three notes — and writing that out produces a progression that changes
/// eight times a bar and describes nothing. A chord a producer played is held.
const MIN_CHORD_SECONDS: f32 = 0.5;

/// How much the best template must beat the runner-up by.
///
/// ⛔ **Otherwise this labels noise.** A drum fill has energy in every bin, every
/// template scores about the same, and the winner is whichever rounding error was
/// largest. A tenth of a point of separation is the difference between "this is a
/// chord" and "twenty-four numbers happened to have a maximum".
const CHORD_MARGIN: f32 = 0.06;

/// The octave chords are written in.
///
/// ⚠ C3–B3. It is a *spelling* rather than a voicing — see the module note — so
/// it sits where a keyboard part usually does and does not collide with the
/// bassline's register.
const CHORD_OCTAVE: u8 = 48;

/// The intervals of the two qualities this recognises.
///
/// ⛔ **Major and minor only, and that is a limit worth naming.** Sevenths,
/// suspensions and extensions all share these three notes plus one more, so a
/// template bank that included them would win on the extra note about as often as
/// it lost on a passing tone — and the chords generator consumes roman numerals
/// and a harmonic rhythm, which these give. A ninth chord read as its triad is
/// the right chord; read as a different root it is not.
const QUALITIES: [(&[u8], bool); 2] = [(&[0, 4, 7], true), (&[0, 3, 7], false)];

/// One chord, and how long it was held.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chord {
    /// Pitch class of the root, `0..12` with 0 as C.
    pub root: u8,
    pub major: bool,
    pub from: f32,
    pub until: f32,
}

/// The progression a file plays, or nothing if it has no harmony to read.
pub fn progression(samples: &[f32], rate: u32) -> Vec<Chord> {
    let (low_hz, high_hz) = HARMONY_BAND;

    // ⚠ Decimated to about 6 kHz, where the highest note in the bank (988 Hz) and
    // its first harmonic still fit. The Goertzel bank is `notes · window`, and
    // both halves shrink with the rate.
    //
    // ⛔ **One walk, because the band-limited buffer had no other reader.** This
    // was `band(...)` into a full-length `Vec` and then `decimate(&that, …)`,
    // which allocated 11.5 MB on a sixty-second 48 kHz file purely to hand it to
    // the next line — on the editor thread of somebody's DAW. `band_decimate`
    // runs the same two cascades in the same order over the same samples, so it
    // is the same answer bit for bit; only the intermediate is gone.
    let factor = ((rate as f32 / 6_000.0).floor() as usize).max(1);
    let (small, small_rate) = bands::band_decimate(samples, rate, low_hz, high_hz, factor);

    let window = ((small_rate as f32 * FRAME_SECONDS).round() as usize).max(16);
    let hop = ((small_rate as f32 * HOP_SECONDS).round() as usize).max(1);
    if small.len() < window {
        return Vec::new();
    }

    // ⛔⛔ **Both tables are built ONCE, and the first cut built them per bin per
    // frame.** The window is 1,500 samples and there are 48 notes: recomputing
    // the Hann term inside `goertzel` was **34.5 million `cos` calls** over a
    // 60-second file for 1,500 distinct values, and re-deriving `440·2^((n−69)/12)`
    // and `2·cos(w)` per frame was another 23,000 `powf`s for 48 constants.
    // ⚠ The arithmetic is identical; only where it happens changed.
    let hann: Vec<f32> = (0..window)
        .map(|at| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * at as f32 / window as f32).cos())
        .collect();
    let coefficients: Vec<(usize, f32)> = (LOW_NOTE..=HIGH_NOTE)
        .map(|note| {
            let hz = crate::midi_audition::midi_hz(note);
            let w = 2.0 * std::f32::consts::PI * hz / small_rate as f32;
            (usize::from(note % 12), 2.0 * w.cos())
        })
        .collect();

    // ⚠ One scratch buffer for the windowed frame, reused, rather than a fresh
    // allocation per frame.
    let mut windowed = vec![0.0_f32; window];
    let mut labelled: Vec<(f32, Option<(u8, bool)>)> = Vec::new();
    let mut at = 0_usize;
    while at + window <= small.len() {
        for (slot, (sample, shape)) in windowed
            .iter_mut()
            .zip(small[at..at + window].iter().zip(&hann))
        {
            *slot = sample * shape;
        }
        let chroma = chroma_of(&windowed, &coefficients);
        labelled.push((at as f32 / small_rate as f32, best_chord(&chroma)));
        at += hop;
    }

    segments(&labelled)
}

/// The twelve pitch-class energies of one **already-windowed** frame,
/// normalised to sum to one.
fn chroma_of(windowed: &[f32], coefficients: &[(usize, f32)]) -> [f32; 12] {
    let mut out = [0.0_f32; 12];
    for (bin, coefficient) in coefficients.iter().copied() {
        out[bin] += goertzel(windowed, coefficient);
    }
    let total: f32 = out.iter().sum();
    if total > f32::MIN_POSITIVE {
        for bin in &mut out {
            *bin /= total;
        }
    }
    out
}

/// The energy of one frequency in one frame, from its Goertzel coefficient.
///
/// ⚠ **The frame arrives Hann-windowed** — see [`progression`], which applies it
/// once for all forty-eight bins. A *rectangular* window would leak a strong note
/// across five or six neighbouring bins, and five neighbouring semitones is most
/// of a chord, so an un-windowed bank finds every triad in every sustained note.
fn goertzel(windowed: &[f32], coefficient: f32) -> f32 {
    let (mut previous, mut older) = (0.0_f32, 0.0_f32);
    for sample in windowed {
        let value = sample + coefficient * previous - older;
        older = previous;
        previous = value;
    }
    (previous * previous + older * older - coefficient * previous * older).max(0.0)
}

/// The best-fitting triad, or `None` when nothing fits clearly enough.
fn best_chord(chroma: &[f32; 12]) -> Option<(u8, bool)> {
    let mut scored: Vec<((u8, bool), f32)> = Vec::with_capacity(24);
    for root in 0..12_u8 {
        for (intervals, major) in QUALITIES {
            let score: f32 = intervals
                .iter()
                .map(|step| chroma[usize::from((root + step) % 12)])
                .sum();
            scored.push(((root, major), score));
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, top) = scored[0];
    // ⛔ Compared against the best chord that does not share this root, not
    // against the literal runner-up: C major and C minor differ by one note, so
    // the second place is nearly always the parallel and the margin would never
    // be met by anything.
    let rival = scored
        .iter()
        .find(|((root, _), _)| *root != best.0)
        .map(|(_, score)| *score)
        .unwrap_or(0.0);
    (top - rival >= CHORD_MARGIN).then_some(best)
}

/// Fold a frame-by-frame labelling into held chords.
fn segments(labelled: &[(f32, Option<(u8, bool)>)]) -> Vec<Chord> {
    let mut out: Vec<Chord> = Vec::new();
    let mut run: Option<(u8, bool, f32)> = None;

    for (at, label) in labelled.iter().copied() {
        // Still the same chord: the run carries on and nothing is written.
        if let (Some((root, major, _)), Some(next)) = (run, label) {
            if (root, major) == next {
                continue;
            }
        }
        if let Some((root, major, from)) = run {
            push(&mut out, root, major, from, at);
        }
        run = label.map(|(root, major)| (root, major, at));
    }
    if let Some((root, major, from)) = run {
        let end = labelled
            .last()
            .map(|(at, _)| *at + HOP_SECONDS)
            .unwrap_or(from);
        push(&mut out, root, major, from, end);
    }
    out
}

/// Keep a run if it was held long enough to be a chord.
fn push(out: &mut Vec<Chord>, root: u8, major: bool, from: f32, until: f32) {
    if until - from >= MIN_CHORD_SECONDS {
        out.push(Chord {
            root,
            major,
            from,
            until,
        });
    }
}

/// The progression as a clip, or `None` if there was none.
pub fn part(samples: &[f32], rate: u32, grid: &Grid) -> Option<LaneTrack> {
    let chords = progression(samples, rate);
    if chords.is_empty() {
        return None;
    }
    let division = division_ticks(grid.time_sig_num, grid.time_sig_den);

    let notes: Vec<Note> = chords
        .iter()
        .flat_map(|chord| {
            let start = grid.tick_of(chord.from);
            let len = grid
                .tick_of(chord.until)
                .saturating_sub(start)
                .max(division);
            let intervals: &[u8] = if chord.major {
                QUALITIES[0].0
            } else {
                QUALITIES[1].0
            };
            intervals.iter().map(move |step| Note {
                start_tick: start,
                len_ticks: len,
                // ⚠ Stacked upward from the root rather than folded back into one
                // octave: folding turns A minor into C-E-A, which is a first
                // inversion nobody played and reads as a different chord on the
                // roll.
                pitch: CHORD_OCTAVE + chord.root + step,
                vel: 88,
                model_vel: None,
                slide_to_pitch: None,
                articulation: None,
                reversed: false,
            })
        })
        .collect();

    Some(LaneTrack {
        lane: Lane::Chords,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A held triad, as three sines with a couple of harmonics each.
    fn triad(root: u8, major: bool, seconds: f32) -> Vec<f32> {
        let n = (RATE as f32 * seconds) as usize;
        let intervals: &[u8] = if major { &[0, 4, 7] } else { &[0, 3, 7] };
        let mut out = vec![0.0_f32; n];
        for step in intervals {
            let note = root + step;
            for harmonic in 1..=3_u32 {
                let hz = 440.0 * 2f32.powf((f32::from(note) - 69.0) / 12.0) * harmonic as f32;
                if hz > 2_000.0 {
                    continue;
                }
                let level = 0.4 / harmonic as f32;
                for (i, sample) in out.iter_mut().enumerate() {
                    let t = i as f32 / RATE as f32;
                    *sample += level * (2.0 * std::f32::consts::PI * hz * t).sin();
                }
            }
        }
        out
    }

    #[test]
    fn a_held_major_triad_is_read_as_that_triad() {
        // C3 major.
        let found = progression(&triad(48, true, 2.0), RATE);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].root, 0);
        assert!(found[0].major);
    }

    #[test]
    fn a_minor_triad_is_not_read_as_its_relative_major() {
        // ⛔ A minor and C major share two notes of three; a bank that scores by
        // overlap alone answers whichever it happened to try first.
        let found = progression(&triad(45, false, 2.0), RATE);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].root, 9, "A minor is root 9, got {}", found[0].root);
        assert!(!found[0].major);
    }

    #[test]
    fn a_change_is_found_where_it_happens() {
        let mut buffer = triad(48, true, 1.5);
        buffer.extend(triad(53, true, 1.5));
        let found = progression(&buffer, RATE);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_eq!((found[0].root, found[1].root), (0, 5));
        assert!(
            (found[1].from - 1.5).abs() < 0.3,
            "the change was found at {}",
            found[1].from
        );
    }

    #[test]
    fn noise_is_not_a_chord() {
        // ⛔ Twenty-four templates always have a maximum. Without the margin, a
        // drum fill is a progression.
        let mut state = 0xC40D_u32;
        let noise: Vec<f32> = (0..RATE * 2)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 9) as f32 / f32::from(u16::MAX) / 128.0 - 0.5
            })
            .collect();
        assert!(progression(&noise, RATE).is_empty());
    }

    #[test]
    fn a_chord_that_is_not_held_is_not_written_down() {
        // ⚠ Two hundred milliseconds of something is a passing tone, not a chord.
        let mut buffer = triad(48, true, 0.3);
        buffer.extend(vec![0.0_f32; RATE as usize]);
        assert!(progression(&buffer, RATE).is_empty());
    }

    #[test]
    fn the_clip_spells_the_chord_rather_than_voicing_it() {
        // ⛔ The module's own promise: a root, a third and a fifth in one octave,
        // and no claim that this is what the pianist played.
        let grid = Grid::at(120.0, 2, 4, 4);
        let track = part(&triad(48, true, 2.0), RATE, &grid).expect("a chord");
        assert_eq!(track.lane, Lane::Chords);
        assert_eq!(track.notes.len(), 3);
        let pitches: Vec<u8> = track.notes.iter().map(|note| note.pitch).collect();
        assert_eq!(pitches, vec![48, 52, 55]);
    }
}
