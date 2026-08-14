//! Whether the line that came out of the melody band was sung (TASK-058G).
//!
//! ⛔⛔ **Mike was right and the first answer conflated two problems.** He asked
//! *"and leave the vocals alone"*, and then — correctly pushing back on a
//! too-absolute reply — *"you cannot exclude a vocal from an audio file based on
//! where the notes land?"*
//!
//! Separating vocal *audio* out of a mix needs ML. Deciding whether an extracted
//! *note line* came from a voice does not, and that is all this needs, because
//! the only question is whether the line is routed into Melody or left alone.
//! **Rejecting a line is far easier than removing it from audio.**
//!
//! ## The four discriminators, all arithmetic on measurements already taken
//!
//! - **Cents deviation.** A sequenced part sits on exact equal-temperament
//!   pitches; a sung line drifts ±20–50 cents. The strongest single signal, and
//!   [`super::pitched::Frame`] already carries it.
//! - **Grid alignment.** A programmed part lands within a few milliseconds of the
//!   sixteenth grid; a vocal is tens of milliseconds loose, phrase by phrase.
//! - **Pitch continuity.** Voices glide between notes; keys and plucks jump.
//! - **Vibrato.** 5–7 Hz modulation on sustained sung notes, which shows up as a
//!   regular oscillation in the cents contour of one held note.
//!
//! ⚠ **The fifth one in the roadmap is NOT here, and it is not an oversight.**
//! Stereo position — *"vocals sit centred; `L−R` cancels anything centred"* — is
//! the cheapest test of the five and it needs a stereo buffer.
//! `audio::import::decode` averages to mono inside the decoder, so by the time
//! any of this runs there is no side channel left to difference. Adding one means
//! a second decode path for every sample in the product, which is a larger change
//! than this task, and doing it badly (decoding twice) would double the cost of
//! every import to serve one test. **It is worth doing and it is not done.**
//!
//! ⚠⚠ **Where this degrades, and the page has to say so.** Hard Auto-Tune defeats
//! cents deviation, grid looseness **and** portamento at once — tuning software
//! exists precisely to make a voice behave like a sequenced instrument. On
//! heavily tuned modern trap vocals this approaches guessing, and the honest
//! thing is to route the line and let the producer delete it rather than to
//! silently drop a lead because it was sung well.
//!
//! ⛔ **The stem path sidesteps all of it, and is better.**
//! `audioTest/Cymatics - Anthem` arrives as four stems — Brass, Keys, Pad and
//! **Vocal** — and given stems there is nothing to classify: import the ones you
//! want and skip the vocal.

use super::grid::Grid;
use super::pitched::{runs, Frame};

/// Mean cents-off, above which the line is drifting rather than tuned.
///
/// ⚠ **18 cents.** A sequencer is exact and the tracker's own error on a clean
/// tone is about 5; a sung line runs 20–50. Eighteen sits above the noise and
/// below the signal.
const DRIFT_CENTS: f32 = 18.0;

/// How far off the sixteenth grid a line may sit, as a share of a sixteenth.
///
/// ⚠ A share rather than milliseconds, because 30 ms is a fifth of a sixteenth at
/// 90 BPM and a third of one at 150 — the same looseness means different things
/// at different tempos, and what the ear judges is the fraction.
const LOOSE_SHARE: f32 = 0.22;

/// What share of note changes must glide before the line is a gliding one.
const GLIDE_SHARE: f32 = 0.30;

/// The band a wobble has to be in to be vibrato, in hertz.
const VIBRATO_HZ: (f32, f32) = (4.0, 8.5);

/// ...and how deep, in cents. Below this it is the tracker, not the singer.
const VIBRATO_CENTS: f32 = 15.0;

/// ...and how long a note must be held to be able to carry one.
const VIBRATO_SECONDS: f32 = 0.3;

/// ...and what share of held notes must carry one.
///
/// ⚠ **A fifth, measured across four real stems.** The `Cymatics - Anthem` vocal
/// carries a 4–8 Hz wobble on **26%** of its held notes; the Keys stem on **0%**,
/// the Brass on 3% and the Pad on 10%. A fifth sits above the three instruments
/// and below the voice.
const VIBRATO_SHARE: f32 = 0.2;

/// How many of the measurements have to fire.
///
/// ⛔ **Two, not one.** Each of these is individually defeatable: a synth with
/// portamento glides, a live keyboard player is loose against the grid, and a
/// detuned sample drifts. Requiring two means a line has to look sung in more than
/// one way — and a *sequenced* line that trips two of these is genuinely unusual.
///
/// ⛔⛔ **Except vibrato, which is sufficient alone, and that follows from the
/// roadmap's own warning rather than from a fixture.** Hard Auto-Tune defeats
/// cents deviation, grid looseness *and* portamento **at once** — that is what
/// the software is for. So on the exact case this feature was written for, a
/// produced modern vocal, three of the four signals are gone by construction and
/// requiring two of them means never firing at all. Vibrato survives tuning
/// (retune speed does not remove a 6 Hz wobble, and producers keep it
/// deliberately) and no sequenced part produces one by accident.
/// ▶ Measured across four real stems: the `Anthem` **Vocal** carries one on 26%
/// of its held notes; Keys 0%, Brass 3%, Pad 10%.
const NEEDED: usize = 2;

/// How much of the file the line must cover before it is judged at all.
///
/// ⛔⛔ **A fragmentary track cannot answer this question, and judging it anyway
/// deleted a synth lead.** Tracking a melody out of a *full mix* is the hard case
/// the roadmap names — leads, keys, guitars, brass and voices share that band, and
/// so do the drums — so the line that comes back is the moments the mix happened
/// to be clear. On `Starlight.wav` that is 28% of the file, and those fragments
/// measured 0.27 off the grid and 0.36 glided, which is two signals and a
/// rejection: the record's own lead, dropped, and the panel telling the producer a
/// *vocal* had been left alone in a track that has none.
///
/// ▶ **So a line this thin is not evidence of anything and the default is to keep
/// it.** Half. Measured: the Anthem Vocal covers 100% and the Pad 83%; the Keys
/// stem covers 40%, the Brass 27% and the Starlight mix 28%.
///
/// ⚠ Erring towards keeping is the right direction: a lead that should have been
/// dropped is one click to delete, and a lead that was dropped is invisible.
const JUDGEABLE_COVER: f32 = 0.5;

/// What the measurements said.
///
/// ⚠ **All four numbers are kept, not just the verdict.** The panel has to be
/// able to say *why* a line was left alone, which is this module's half of the
/// rule the whole feature is built on: never present a guess as a transcription.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub sung: bool,
    /// Whether the line was solid enough to be judged at all — see
    /// [`JUDGEABLE_COVER`].
    pub judged: bool,
    /// Mean distance from equal temperament, in cents.
    pub drift_cents: f32,
    /// Mean distance of a note's start from the sixteenth grid, as a share of one.
    pub off_grid: f32,
    /// Share of note changes the line glided through rather than jumped.
    pub glides: f32,
    /// Share of held notes carrying a 4–8 Hz wobble.
    pub vibrato: f32,
}

/// Did a voice play this line?
///
/// `seconds` is the length of the file the line was tracked out of, which is what
/// [`JUDGEABLE_COVER`] is a share of.
pub fn assess(frames: &[Frame], grid: &Grid, seconds: f32) -> Verdict {
    let held = runs(frames);
    let cover = if seconds > 0.0 {
        frames.len() as f32 * super::pitched::HOP_SECONDS / seconds
    } else {
        0.0
    };
    if held.is_empty() || cover < JUDGEABLE_COVER {
        return Verdict {
            sung: false,
            judged: false,
            drift_cents: 0.0,
            off_grid: 0.0,
            glides: 0.0,
            vibrato: 0.0,
        };
    }

    let drift_cents = mean(frames.iter().map(|frame| f32::from(frame.cents).abs()));
    let off_grid = mean(held.iter().map(|run| grid.off_grid(frames[run.start].at)));
    let glides = glide_share(frames, &held);
    let vibrato = mean(
        held.iter()
            .map(|run| f32::from(wobbles(&frames[run.clone()]))),
    );

    // ⛔⛔ **The grid test only counts when there IS a grid.** `grid::fit` falls
    // back to the session's tempo when nothing in the file beat random, and every
    // line then sits "off the grid" by about a quarter of a sixteenth — because
    // the grid is arbitrary, not because the line is loose. Counting it anyway
    // rejected a *pad* as a vocal on the strength of a tempo nobody measured.
    let wobbling = vibrato >= VIBRATO_SHARE;
    let fired = usize::from(drift_cents >= DRIFT_CENTS)
        + usize::from(grid.measured && off_grid >= LOOSE_SHARE)
        + usize::from(glides >= GLIDE_SHARE)
        + usize::from(wobbling);

    Verdict {
        // ⛔ Vibrato on its own is enough — see [`NEEDED`].
        sung: wobbling || fired >= NEEDED,
        judged: true,
        drift_cents,
        off_grid,
        glides,
        vibrato,
    }
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let mut count = 0_usize;
    let mut total = 0.0_f32;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// What share of the changes between notes were glided through.
///
/// ⚠ **Measured on the frames the runs do NOT cover.** A held note is a run; the
/// frames between two runs are the transition, and a voice spends them on the
/// pitches in between while a keyboard spends none there at all.
///
/// ⛔ **The gap is addressed, not searched for.** `runs` hands back index ranges
/// into `frames`, so the transition between two runs *is* `frames[a.end..b.start]`
/// — three or four frames. The first cut filtered the whole track per note change,
/// which on a one-minute melody is up to three thousand runs against three
/// thousand frames: four and a half million visits to inspect what is normally
/// one frame.
fn glide_share(frames: &[Frame], held: &[std::ops::Range<usize>]) -> f32 {
    if held.len() < 2 {
        return 0.0;
    }
    let mut glided = 0_usize;
    let mut changes = 0_usize;
    for pair in held.windows(2) {
        let from = frames[pair[0].end - 1];
        let to = frames[pair[1].start];
        // Only a real change of note.
        if from.note == to.note {
            continue;
        }
        let (low, high) = (from.note.min(to.note), from.note.max(to.note));
        // ⚠ An interval of one semitone has no room to glide *through* anything,
        // so it can neither prove nor disprove portamento — and it is not counted
        // as a change either, or a chromatic line would read as one that never
        // glides.
        if high - low < 2 {
            continue;
        }
        changes += 1;
        let between = frames[pair[0].end..pair[1].start]
            .iter()
            .filter(|frame| frame.note > low && frame.note < high)
            .count();
        if between > 0 {
            glided += 1;
        }
    }
    if changes == 0 {
        0.0
    } else {
        glided as f32 / changes as f32
    }
}

/// Does this held note carry a 4–8 Hz wobble deep enough to be vibrato?
///
/// ⛔ **Counted rather than transformed.** The roadmap says *"found by an FFT of
/// the pitch contour"* — but a vibrato is a *regular* oscillation, so counting the
/// times the contour turns around and dividing by twice the duration is the same
/// number without the dependency this crate spends whole modules avoiding.
fn wobbles(run: &[Frame]) -> bool {
    let seconds = run[run.len() - 1].at - run[0].at;
    if seconds < VIBRATO_SECONDS || run.len() < 5 {
        return false;
    }
    let contour: Vec<f32> = run.iter().map(|frame| f32::from(frame.cents)).collect();
    let low = contour.iter().copied().fold(f32::INFINITY, f32::min);
    let high = contour.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if high - low < VIBRATO_CENTS {
        return false;
    }

    // ⚠ Turns of the contour, ignoring the flat steps a quantised cents value
    // produces — an `i8` of cents changes in whole units, so a slow glide is a
    // staircase and every tread would otherwise be a turn.
    let mut turns = 0_usize;
    let mut rising: Option<bool> = None;
    for pair in contour.windows(2) {
        let delta = pair[1] - pair[0];
        if delta.abs() < 1.0 {
            continue;
        }
        let now = delta > 0.0;
        if rising.is_some_and(|was| was != now) {
            turns += 1;
        }
        rising = Some(now);
    }

    let hz = turns as f32 / (2.0 * seconds);
    (VIBRATO_HZ.0..=VIBRATO_HZ.1).contains(&hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: Grid = Grid::at(120.0, 2, 4, 4);

    /// The length of a file these frames would fill completely.
    ///
    /// ⚠ Every test below hands in a line with no gaps in it, so its own span
    /// *is* the file — which makes [`JUDGEABLE_COVER`] a pass rather than the
    /// thing under test. `a_fragmentary_line_is_not_judged_at_all` is where that
    /// guard is exercised on purpose.
    fn covered(frames: &[Frame]) -> f32 {
        frames.len() as f32 * crate::extract::pitched::HOP_SECONDS
    }

    /// A line of `(note, cents)` steps, one frame every 20 ms from `start`.
    fn line(start: f32, steps: &[(u8, i8)]) -> Vec<Frame> {
        steps
            .iter()
            .enumerate()
            .map(|(at, (note, cents))| Frame {
                at: start + at as f32 * 0.02,
                note: *note,
                cents: *cents,
                clarity: 0.9,
            })
            .collect()
    }

    /// A sequenced part: on the grid, dead in tune, jumping between notes.
    fn sequenced() -> Vec<Frame> {
        let sixteenth = GRID.bar_seconds() / 16.0;
        let mut out = Vec::new();
        for (index, note) in [72_u8, 74, 77, 74].iter().enumerate() {
            out.extend(line(
                index as f32 * sixteenth * 4.0,
                &[(*note, 1), (*note, -2), (*note, 0), (*note, 1), (*note, -1)],
            ));
        }
        out
    }

    #[test]
    fn a_sequenced_line_is_kept() {
        let verdict = assess(&sequenced(), &GRID, covered(&sequenced()));
        assert!(!verdict.sung, "{verdict:?}");
        assert!(verdict.drift_cents < DRIFT_CENTS, "{verdict:?}");
    }

    #[test]
    fn a_line_that_drifts_and_sits_late_is_left_alone() {
        // ⛔ Two of the four: 30 cents of drift and a third of a sixteenth late.
        // Either alone is defeatable — a detuned sample drifts, a live keyboard
        // is loose — which is why it takes two.
        let sixteenth = GRID.bar_seconds() / 16.0;
        let mut out = Vec::new();
        for (index, note) in [72_u8, 74, 77, 74].iter().enumerate() {
            out.extend(line(
                index as f32 * sixteenth * 4.0 + sixteenth * 0.33,
                &[
                    (*note, 30),
                    (*note, -34),
                    (*note, 28),
                    (*note, -30),
                    (*note, 33),
                ],
            ));
        }
        let verdict = assess(&out, &GRID, covered(&out));
        assert!(verdict.sung, "{verdict:?}");
        assert!(verdict.drift_cents > DRIFT_CENTS, "{verdict:?}");
        assert!(verdict.off_grid > LOOSE_SHARE, "{verdict:?}");
    }

    #[test]
    fn one_signal_on_its_own_is_not_enough() {
        // ⚠ A synth with portamento glides and is otherwise a machine. Rejecting
        // it would delete a lead the producer wanted.
        let sixteenth = GRID.bar_seconds() / 16.0;
        let mut out = line(0.0, &[(72, 0), (72, 1), (72, -1), (72, 0), (72, 1)]);
        // The glide up.
        out.extend(line(0.1, &[(73, 0), (74, 1), (75, 0)]));
        out.extend(line(
            sixteenth * 4.0,
            &[(77, 0), (77, 1), (77, -1), (77, 0)],
        ));
        let verdict = assess(&out, &GRID, covered(&out));
        assert!(!verdict.sung, "{verdict:?}");
    }

    #[test]
    fn a_held_note_with_a_six_hertz_wobble_in_it_reads_as_vibrato() {
        // ⛔ 6 Hz at ±25 cents over one second, sampled every 20 ms.
        let contour: Vec<(u8, i8)> = (0..50)
            .map(|i| {
                let t = i as f32 * 0.02;
                let cents = 25.0 * (2.0 * std::f32::consts::PI * 6.0 * t).sin();
                (72_u8, cents.round() as i8)
            })
            .collect();
        let verdict = assess(&line(0.0, &contour), &GRID, covered(&line(0.0, &contour)));
        assert!(verdict.vibrato >= 0.5, "{verdict:?}");
    }

    #[test]
    fn a_steady_held_note_is_not_vibrato() {
        let steady: Vec<(u8, i8)> = (0..50).map(|_| (72_u8, 2)).collect();
        assert_eq!(
            assess(&line(0.0, &steady), &GRID, covered(&line(0.0, &steady))).vibrato,
            0.0
        );
    }

    #[test]
    fn nothing_to_measure_is_answered_with_nothing_rather_than_a_verdict() {
        let verdict = assess(&[], &GRID, 1.0);
        assert!(!verdict.sung);
        assert!(!verdict.judged);
        assert_eq!(verdict.drift_cents, 0.0);
    }

    #[test]
    fn a_fragmentary_line_is_not_judged_at_all() {
        // ⛔⛔ **The guard that stops a synth lead being deleted as a vocal.** A
        // melody tracked out of a full mix is the moments the mix happened to be
        // clear — 28% of `Starlight.wav` — and those fragments measured two of the
        // four signals. Judging them rejected the record's own lead and told the
        // producer a vocal had been left alone in a track with none.
        let sixteenth = GRID.bar_seconds() / 16.0;
        let mut out = Vec::new();
        for (index, note) in [72_u8, 74, 77, 74].iter().enumerate() {
            out.extend(line(
                index as f32 * sixteenth * 4.0 + sixteenth * 0.33,
                &[(*note, 30), (*note, -34), (*note, 28), (*note, -30)],
            ));
        }
        // The same line that is rejected above, now covering a fifth of the file.
        let verdict = assess(&out, &GRID, covered(&out) * 5.0);
        assert!(!verdict.judged, "{verdict:?}");
        assert!(!verdict.sung, "{verdict:?}");
    }
}
