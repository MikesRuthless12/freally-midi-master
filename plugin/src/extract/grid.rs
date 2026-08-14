//! A loop's own length, and the tempo that implies (TASK-058I).
//!
//! ⛔⛔ **The roadmap costed this as a standalone measurement and it is not one.**
//! Checked in the code on 2026-08-14: for MIDI there was nothing to remove —
//! `smf_read::assemble` already derives bars from the notes and
//! `smf_read::section_length` already *is* the autocorrelate-at-candidate-lags
//! measurement the entry asks for. For audio there was no drag-in path at all.
//! So this is the audio half, built with TASK-058F because it needs its onsets,
//! and it reuses `section_length`'s shape rather than being a second
//! period-finder.
//!
//! ## The one thing audio has that MIDI does not
//!
//! **A duration that is exactly a whole number of bars.** A sample-pack loop is
//! sold as "4 bars at 140" and the file is cut to it — `Starlight.wav` is
//! 15.966 s and its own MIDI arranges to 8 bars at 120 BPM, which is 16.000 s.
//! That is 0.2%. So the bar count and the tempo are *one* unknown, not two:
//! pick the bar count and the tempo follows from the length.
//!
//! ⚠ **Which leaves the octave problem, and nothing in the audio can solve it.**
//! Eight bars at 140 and four bars at 70 describe the same file, and every hit in
//! it lands on the grid either way. The session's tempo is the tie-break, which
//! is the same answer TASK-058E gave for MIDI — *"imports adopt the session tempo
//! and never claim one they did not read"* — applied to the one part of the
//! reading that is genuinely undecidable.
//!
//! ⚠ **And the producer keeps the override.** The bars chip is still theirs: a
//! producer who wants four when this says eight must not have to fight it.

use engine::pattern::{ticks_per_bar_of, PPQ};

use super::onsets::Onset;

/// The bar counts a dropped loop might be.
///
/// ⚠ **Includes 3, 6, 12 and 24** — not because of 3/4, which the meter handles,
/// but because a producer's loop is not always a power of two. A 12-bar blues and
/// a 6-bar turnaround are both ordinary and both would otherwise be read as the
/// nearest power of two at a tempo 33% wrong.
const BAR_CHOICES: &[u16] = &[1, 2, 3, 4, 6, 8, 12, 16, 24, 32];

/// The slowest and fastest a candidate may imply and still be considered.
///
/// ⚠ Wide on purpose. Half-time trap is written at 140 and felt at 70, and
/// drum'n'bass is 174 — a narrower window would refuse the file rather than
/// misread it, which is worse.
const MIN_BPM: f32 = 55.0;
const MAX_BPM: f32 = 210.0;

/// How finely a bar is divided when a hit is placed on it.
///
/// ⚠ **96, so both duplets and triplets exist.** 96 = 2⁵·3: a sixteenth is 6
/// divisions, a sixteenth triplet is 4, a thirty-second is 3. A power of two
/// alone would make Mike's *"triplets being heard for hihats"* unrepresentable,
/// and the hat-roll rule in [`super::drums`] is written around exactly that.
///
/// ⚠ **It is also the timing this can honestly claim.** At 140 BPM one division
/// is 18 ms, so a hit is placed within 9 ms of where it was played — which is
/// about the error the onset detector has anyway, and well inside the 20–60 ms a
/// swung sixteenth is displaced by. Quantising harder would flatten the groove;
/// not quantising at all would present the smoother's own few milliseconds of
/// delay as a performance.
pub const DIVISIONS_PER_BAR: u32 = 96;

/// The tempo, meter and length a file was read as.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub bpm: f32,
    pub bars: u16,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    /// Where beat one actually is, in seconds from the top of the file.
    ///
    /// ⛔⛔ **Without this the grid is right and every note is wrong.** A loop is
    /// cut with a little air in front of it, and the envelope smoothing adds a
    /// few milliseconds more — measured at **21 ms** on `Starlight.wav`, which is
    /// a fifth of a sixteenth at 120 BPM. Every hit in the file is displaced by
    /// that constant, so a grid fitted without solving for it scores the *correct*
    /// tempo at 0.21 (random is 0.25) and rejects it. Solving for the phase first
    /// takes the same file to 0.07.
    pub offset_seconds: f32,
    /// Whether the tempo came off the file or off the session.
    ///
    /// ⛔ **Reported, because a grid nobody measured is not evidence.** Onsets on
    /// a sustained pad are where the smoother happened to turn around, not where
    /// anything was struck — none of the candidate tempos beat random on any of
    /// the four Anthem stems. Falling back to the session's tempo is the honest
    /// answer there, and [`super::vocal`] has to know not to weigh "this line sits
    /// off the grid" when the grid was invented.
    pub measured: bool,
}

impl Grid {
    /// A grid at a tempo the caller already knows, with beat one at zero.
    ///
    /// ⚠ For the case where nothing has to be inferred — a producer who has
    /// overridden the reading, and the tests, which would otherwise each carry
    /// their own copy of "and the offset is nought and yes it was measured".
    pub const fn at(bpm: f32, bars: u16, time_sig_num: u8, time_sig_den: u8) -> Self {
        Self {
            bpm,
            bars,
            time_sig_num,
            time_sig_den,
            offset_seconds: 0.0,
            measured: true,
        }
    }

    /// How long one bar of this is.
    pub fn bar_seconds(&self) -> f32 {
        let beats = f32::from(self.time_sig_num) * 4.0 / f32::from(self.time_sig_den.max(1));
        beats * 60.0 / self.bpm.max(1.0)
    }

    /// How many sixteenths are in one bar of this meter.
    pub fn sixteenths_per_bar(&self) -> u32 {
        (u32::from(self.time_sig_num) * 16 / u32::from(self.time_sig_den.max(1))).max(1)
    }

    /// Where a moment sits in the loop, in bars from beat one.
    fn bars_at(&self, seconds: f32) -> f32 {
        ((seconds - self.offset_seconds) / self.bar_seconds()).max(0.0)
    }

    /// The engine tick a moment in seconds lands on, quantised to
    /// [`DIVISIONS_PER_BAR`].
    pub fn tick_of(&self, seconds: f32) -> u32 {
        let per_bar = ticks_per_bar_of(self.time_sig_num, self.time_sig_den);
        let division = per_bar / DIVISIONS_PER_BAR.max(1);
        let divisions = (self.bars_at(seconds) * DIVISIONS_PER_BAR as f32).round() as u32;
        divisions.saturating_mul(division.max(1))
    }

    /// Which sixteenth of the bar a moment falls on, `0..16` in 4/4.
    ///
    /// ⚠ The classifier's unit rather than the note's: *"snares on the 4 beat"*
    /// and *"16th-grid high-band"* are both statements about this number.
    pub fn sixteenth_of(&self, seconds: f32) -> u32 {
        let per_bar = self.sixteenths_per_bar();
        let index = (self.bars_at(seconds) * per_bar as f32).round() as u32;
        index % per_bar
    }

    /// How far a moment sits from the nearest sixteenth, `0..0.5` of one.
    ///
    /// ⚠ Shared with [`super::vocal`], which asks exactly this of a note's
    /// unquantised start.
    pub fn off_grid(&self, seconds: f32) -> f32 {
        let steps = self.bars_at(seconds) * self.sixteenths_per_bar() as f32;
        (steps - steps.round()).abs()
    }

    /// The whole clip, in ticks.
    pub fn total_ticks(&self) -> u32 {
        ticks_per_bar_of(self.time_sig_num, self.time_sig_den).saturating_mul(u32::from(self.bars))
    }
}

/// How far off the sixteenth grid a candidate's hits may average and still be
/// considered a fit, as a share of a sixteenth.
///
/// ⛔⛔ **An absolute threshold, not "within some factor of the best", and the
/// difference is a bug this had.** A relative cutoff systematically prefers the
/// *slowest* candidate: half the tempo is twice the bar, so the same five
/// milliseconds of detection error is half as far off the grid — a two-bar loop
/// at 140 scored 0.17 while reading the same file as one bar at 70 scored 0.08,
/// and the correct answer was thrown out for being 1.5× worse than a reading with
/// a coarser ruler.
///
/// ⚠ **And measured against SIXTEENTHS rather than [`DIVISIONS_PER_BAR`].** Tempo
/// is decided by where the beats are. At 140 BPM a sixteenth is 107 ms, so the
/// detector's own ~5 ms error is 5% of one — against a ninety-sixth it would be
/// 28%, which is most of the way to the 25% that randomly-placed hits score.
///
/// ⚠ **0.12, measured on `Starlight.wav`.** Its true reading — 8 bars at 120 —
/// scores **0.068** and the best wrong one scores **0.135**; the four Anthem
/// stems, which are sustained instruments with no readable pulse, score 0.22–0.25
/// at every candidate. This sits between the three groups.
const ALIGNED_ENOUGH: f32 = 0.12;

/// What share of the hits the tempo is fitted to, strongest first.
///
/// ⛔⛔ **Half, and it is the difference between reading the tempo and not.** A
/// real loop's onset list is not its score: hat tails, room and the shoulders of
/// loud transients all clear the detection threshold, and they land wherever they
/// land. On `Starlight.wav` the correct tempo scores **0.161** over all 208 hits
/// and **0.068** over the loudest 104 — the difference between "rejected as
/// unaligned" and "the clear winner". The quiet half is not noise in the audio; it
/// is the part of the audio that is not the beat.
const FITTED_SHARE: f32 = 0.5;

/// Read the tempo and length of a loop `seconds` long from where its hits are.
///
/// `session_bpm` breaks the octave tie and nothing else.
pub fn fit(onsets: &[Onset], seconds: f32, session_bpm: f32, num: u8, den: u8) -> Grid {
    // ⛔⛔ **The meter is narrowed here, because it arrives from two untrusted
    // places.** `explorer_audio_split` reads it off bridge JSON, and `pins`
    // carries it in a **shared project file** — so `255/1` is a value this
    // function really can be handed. See [`division_ticks`] for what it used to
    // cost; this closes the door on the way in as well as at the sill.
    // ⚠ 32 and a power-of-two denominator up to 32 covers every meter written
    // down; `.max(1)` alone was never a range check.
    let (num, den) = (
        num.clamp(1, 32),
        den.clamp(1, 32).next_power_of_two().min(32),
    );
    let beats_per_bar = f32::from(num) * 4.0 / f32::from(den);
    let session_bpm = if session_bpm.is_finite() && session_bpm > 0.0 {
        session_bpm
    } else {
        120.0
    };

    // ⚠ **A file with nothing readable in it still has to answer**, and the honest
    // answer is the session's own tempo with the length rounded to bars at it —
    // nothing was measured, so `measured` says so and nothing is claimed.
    let adopt_session = || {
        let bar_seconds = beats_per_bar * 60.0 / session_bpm;
        Grid {
            bpm: session_bpm,
            bars: (seconds / bar_seconds)
                .round()
                .clamp(1.0, f32::from(u16::MAX)) as u16,
            time_sig_num: num,
            time_sig_den: den,
            offset_seconds: 0.0,
            measured: false,
        }
    };
    // ⚠ Load-bearing: `strengths[…]` below indexes into what this guarantees is
    // not empty.
    if onsets.len() < 2 {
        return adopt_session();
    }

    // ⛔ Fitted to the loudest half — see [`FITTED_SHARE`].
    let mut strengths: Vec<f32> = onsets.iter().map(|onset| onset.strength).collect();
    strengths.sort_by(f32::total_cmp);
    let floor = strengths[((strengths.len() as f32) * (1.0 - FITTED_SHARE)) as usize];
    let strong: Vec<&Onset> = onsets
        .iter()
        .filter(|onset| onset.strength >= floor)
        .collect();

    let sixteenths_per_bar = u32::from(num) * 16 / u32::from(den);
    // ⛔ Among everything that fits the hits, the one nearest the session — in
    // *ratio*, so 70 and 280 are equally far from 140 and neither is favoured by
    // being the smaller number.
    // ⚠ `BAR_CHOICES` is walked in order and `min_by` keeps the first minimum, so
    // an exact tie goes to the shorter reading — which is the same answer the
    // collapse below would reach anyway.
    let Some((bars, bpm, offset)) = BAR_CHOICES
        .iter()
        .copied()
        .filter_map(|bars| {
            // The whole point: the length and the bar count give the tempo.
            let bpm = f32::from(bars) * beats_per_bar * 60.0 / seconds.max(1.0e-3);
            if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
                return None;
            }
            let (offset, score) =
                misalignment(&strong, seconds / f32::from(bars), sixteenths_per_bar);
            (score <= ALIGNED_ENOUGH).then_some((bars, bpm, offset))
        })
        .min_by(|a, b| {
            let distance = |bpm: f32| (bpm / session_bpm).ln().abs();
            distance(a.1).total_cmp(&distance(b.1))
        })
    else {
        return adopt_session();
    };

    Grid {
        bpm,
        bars: collapse(onsets, bars, seconds / f32::from(bars), offset),
        time_sig_num: num,
        time_sig_den: den,
        offset_seconds: offset,
        measured: true,
    }
}

/// Where beat one is, and how far the hits sit from the grid once it is there.
///
/// ⛔⛔ **The phase is solved for, not assumed to be zero.** A loop has air in
/// front of it and the analysis adds a few milliseconds of its own, so every hit
/// in the file is displaced by one constant. Measuring against a grid nailed to
/// `t = 0` therefore scores the *correct* tempo as though the hits were random —
/// on `Starlight.wav`, 0.212 against a random 0.25.
///
/// ▶ **Found the way any shared phase is found: as a circular mean.** Each hit's
/// position within its sixteenth is an angle; the mean *vector* of those angles
/// points at the offset they share, and averaging them as plain numbers would
/// answer 0.5 for a set clustered on both sides of zero.
///
/// ⚠ The returned distance is in sixteenths, so it means the same thing at every
/// tempo: 0 is dead on the grid and 0.5 is as far off as it is possible to be.
/// Hits scattered at random average 0.25, which is what makes
/// [`ALIGNED_ENOUGH`] a threshold rather than a tuning.
fn misalignment(onsets: &[&Onset], bar_seconds: f32, sixteenths_per_bar: u32) -> (f32, f32) {
    if bar_seconds <= 0.0 || sixteenths_per_bar == 0 || onsets.is_empty() {
        return (0.0, 0.5);
    }
    let sixteenth = bar_seconds / sixteenths_per_bar as f32;
    let steps_of = |onset: &Onset| onset.at / sixteenth;

    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    for onset in onsets {
        let angle = steps_of(onset).fract() * std::f32::consts::TAU;
        x += angle.cos();
        y += angle.sin();
    }
    // In sixteenths, wrapped to ±half of one: a hit 0.9 of a sixteenth late is a
    // hit 0.1 of one early, and the second reading is the one that is true.
    let phase = y.atan2(x) / std::f32::consts::TAU;

    let total: f32 = onsets
        .iter()
        .map(|onset| {
            let steps = steps_of(onset) - phase;
            (steps - steps.round()).abs()
        })
        .sum();
    (phase * sixteenth, total / onsets.len() as f32)
}

/// Shorten a loop that is literally the same few bars several times over.
///
/// ⛔⛔ **Only when collapsing loses NOTHING, and that is a deliberate departure
/// from `section_length`.** That function takes the longest period where *half*
/// the blocks repeat, because it is cutting a song into sections and a hook that
/// differs from the verse is still a section boundary. This is cutting a
/// **loop**, and Mike's own note on the MIDI side is the rule: *"some trap drums
/// vary every 4 bars for a verse"* — an A-A-A-B file collapsed to A on the
/// strength of three matches out of four silently deletes the fill. So every
/// block has to match, or the file keeps its own length.
///
/// ⛔⛔ **The blocks must be IDENTICAL, not merely alike, and 0.9 was not enough.**
/// This used to accept a nine-tenths match, borrowed from `SECTION_MATCH` — and
/// on a four-bar loop with ten hits a block plus one crash in bar four, ten
/// shared out of eleven is **0.909**, so it collapsed and `finish` then deleted
/// the crash. Which is the sentence above, with the numbers filled in.
/// ▶ A tolerance is right for *sections* of a song, where a hook that differs
/// from the verse is still a section. It is wrong for a loop, where the whole
/// question is whether anything would be lost.
///
/// ⛔⛔ **And the blocks are cut in OFFSET-RELATIVE DIVISIONS, not in raw
/// seconds.** Slicing on time from `offset` leaves the interval `[0, offset)` in
/// no block at all — so on any real file the first bar's downbeat, which sits a
/// few milliseconds *before* the measured phase, was dropped from the reference
/// while the same hit in every later bar landed at the far end of the previous
/// block as division 96 instead of 0. A four-bar loop of one repeated bar then
/// failed to collapse about half the time, depending on which way the first hit
/// jittered. ⚠ **No synthetic fixture could see it**: they are perfectly aligned,
/// so `offset` is zero and the hole has no width.
///
/// ⚠ Shortest-first, so eight bars of one repeated bar come back as one bar
/// rather than as two.
fn collapse(onsets: &[Onset], bars: u16, bar_seconds: f32, offset: f32) -> u16 {
    if bar_seconds <= 0.0 {
        return bars;
    }
    // Every hit as a division from beat one, wrapped into the loop. `rem_euclid`
    // rather than `%`, so a hit before the offset wraps to the end of the loop
    // instead of coming back negative.
    let whole = i64::from(bars) * i64::from(DIVISIONS_PER_BAR);
    let divisions: Vec<i64> = onsets
        .iter()
        .map(|onset| {
            (((onset.at - offset) / bar_seconds * DIVISIONS_PER_BAR as f32).round() as i64)
                .rem_euclid(whole.max(1))
        })
        .collect();

    for period in BAR_CHOICES.iter().copied() {
        if period >= bars || !bars.is_multiple_of(period) {
            continue;
        }
        let per_block = i64::from(period) * i64::from(DIVISIONS_PER_BAR);
        let signature = |block: i64| -> Vec<i64> {
            let mut out: Vec<i64> = divisions
                .iter()
                .filter(|at| **at / per_block == block)
                .map(|at| at % per_block)
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        };

        let first = signature(0);
        if first.is_empty() {
            continue;
        }
        if (1..i64::from(bars / period)).all(|block| signature(block) == first) {
            return period;
        }
    }
    bars
}

/// One division of a bar, in engine ticks.
///
/// Exposed so a note's length can be expressed in the same unit its start is.
///
/// ⛔⛔ **Bounded ABOVE by a quarter note, and that bound is what stops a bridge
/// message aborting the DAW.** `drums::classify` writes
/// `len.clamp(division, QUARTER)`, and `Ord::clamp` **panics** when `min > max` —
/// so a meter big enough to make one ninety-sixth of a bar longer than a quarter
/// note panics the worker thread. `panic = "abort"` is release-only, so in the
/// shipped plugin that is not a dead worker: it is the **host process gone**,
/// with every unsaved project in it.
///
/// ▶ **And it is reachable twice over.** `explorer_audio_split` reads
/// `timeSigNum`/`timeSigDen` straight off bridge JSON, which this project's own
/// rule says is untrusted — and `pins.timeSigNum` rides in a **shared project
/// file**, so a producer opening somebody else's project could carry `25/1` with
/// no way to see it and abort their DAW on the first drum loop they read.
/// `fit` narrows the meter as well; this is the backstop, at the one function
/// every consumer of a division goes through.
///
/// ⚠ A division longer than a quarter is not a subdivision of anything, so
/// clamping loses nothing real.
pub fn division_ticks(num: u8, den: u8) -> u32 {
    (ticks_per_bar_of(num, den) / DIVISIONS_PER_BAR.max(1)).clamp(1, QUARTER)
}

/// A quarter note, which is what an unmeasurable length falls back to.
pub const QUARTER: u32 = PPQ;

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits at these positions in the bar, over `bars` bars at `bpm`.
    fn hits(bpm: f32, bars: u16, sixteenths: &[u32]) -> (Vec<Onset>, f32) {
        let bar_seconds = 4.0 * 60.0 / bpm;
        let mut out = Vec::new();
        for bar in 0..bars {
            for step in sixteenths {
                out.push(Onset {
                    at: f32::from(bar) * bar_seconds + *step as f32 * bar_seconds / 16.0,
                    low: 1.0,
                    body: 0.0,
                    presence: 0.0,
                    air: 0.0,
                    decay: 0.05,
                    strength: 1.0,
                });
            }
        }
        (out, f32::from(bars) * bar_seconds)
    }

    #[test]
    fn the_length_and_the_hits_give_the_tempo() {
        // Four bars at 140 is 6.857 s. Nothing but the file says so.
        let (onsets, seconds) = hits(140.0, 4, &[0, 4, 8, 12]);
        let grid = fit(&onsets, seconds, 140.0, 4, 4);
        assert!((grid.bpm - 140.0).abs() < 0.5, "{}", grid.bpm);
        assert_eq!(grid.bars, 1, "four identical bars are one bar of loop");
    }

    #[test]
    fn a_fill_in_the_last_bar_keeps_the_file_its_own_length() {
        // ⛔⛔ Mike, on the MIDI side: *"some trap drums vary every 4 bars for a
        // verse."* Collapsing A-A-A-B to A deletes the fill, silently.
        let bar_seconds = 4.0 * 60.0 / 140.0;
        let (mut onsets, seconds) = hits(140.0, 4, &[0, 4, 8, 12]);
        onsets.push(Onset {
            at: 3.0 * bar_seconds + bar_seconds * 14.0 / 16.0,
            low: 0.0,
            body: 1.0,
            presence: 0.0,
            air: 0.0,
            decay: 0.05,
            strength: 1.0,
        });
        onsets.sort_by(|a, b| a.at.total_cmp(&b.at));

        let grid = fit(&onsets, seconds, 140.0, 4, 4);
        assert_eq!(grid.bars, 4, "the bar with the fill in it has to survive");
    }

    #[test]
    fn the_session_breaks_the_octave_tie_and_nothing_else_can() {
        // ⛔ Eight bars at 140 and four at 70 are the same file, and every hit
        // lands on the grid either way. There is nothing in the audio to prefer
        // one, so the session decides — the same answer TASK-058E gave for MIDI.
        let (onsets, seconds) = hits(140.0, 8, &[0, 4, 8, 12]);
        assert!((fit(&onsets, seconds, 140.0, 4, 4).bpm - 140.0).abs() < 1.0);
        assert!((fit(&onsets, seconds, 70.0, 4, 4).bpm - 70.0).abs() < 1.0);
    }

    #[test]
    fn a_loop_with_no_hits_in_it_claims_nothing() {
        // ⚠ Nothing was measured, so nothing is claimed: the session's tempo, and
        // the length rounded to bars at it.
        let grid = fit(&[], 6.857, 140.0, 4, 4);
        assert_eq!(grid.bpm, 140.0);
        assert_eq!(grid.bars, 4);
    }

    #[test]
    fn a_note_lands_on_the_division_it_was_played_on() {
        let grid = Grid::at(120.0, 2, 4, 4);
        assert_eq!(grid.bar_seconds(), 2.0);
        assert_eq!(grid.tick_of(0.0), 0);
        // The third beat of the second bar: one bar plus half of one.
        assert_eq!(grid.tick_of(3.0), ticks_per_bar_of(4, 4) * 3 / 2);
        // ⚠ Six milliseconds off a division still lands on it.
        assert_eq!(grid.tick_of(3.006), ticks_per_bar_of(4, 4) * 3 / 2);
        assert_eq!(grid.sixteenth_of(3.0), 8);
    }

    #[test]
    fn a_triplet_is_representable_and_a_grid_of_powers_of_two_would_lose_it() {
        // ⛔ Mike's hat rule is *"triplets being heard for hihats"*, so the grid
        // has to be able to hold one at all.
        let grid = Grid::at(120.0, 1, 4, 4);
        // The middle of an eighth-note triplet in beat one: a third of a beat.
        let third = grid.bar_seconds() / 4.0 / 3.0;
        assert_eq!(grid.tick_of(third), PPQ / 3);
    }
}
