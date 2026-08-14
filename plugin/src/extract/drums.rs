//! Which drum each hit is (TASK-058F, TASK-058H).
//!
//! ⛔⛔ **THE ARCHITECTURE IS MIKE'S OWN**, 2026-08-10:
//!
//! > *"can't there be like something that has a condition that when it's met like
//! > triplets being heard for hihats, snares on the 4 beat, or multiple snare
//! > rolls, and percussions sounds going up and down, etc. that it splits it into
//! > the proper channels and routes the proper default instruments to them based
//! > on what conditions have been met?"*
//!
//! Two passes, and the split between them is the whole design:
//!
//! - **Pass 1 proposes from the spectrum.** Kick under 120 Hz; snare and clap
//!   broadband; hats up top, closed or open by how long they ring; anything with
//!   a body and no crack is percussion.
//! - **Pass 2 disambiguates with music**, because pass 1 physically cannot. A
//!   snare and a clap are near-identical spectrally, and a hat roll is just more
//!   hat hits — those are not facts about the sound, they are facts about *where
//!   in the bar it happened*.
//!
//! ⛔⛔ **PASS 2 MAY ONLY REASSIGN WHAT PASS 1 DETECTED.** It never invents a hit
//! no onset supports. That is the line between a detector and a generator, and
//! crossing it would make an import into a generation wearing an import's name.
//!
//! ⚠ **What this cannot do**, stated here rather than discovered: a quiet hit
//! masked under a loud one on the same frame is gone — recovering it is source
//! separation. Heavily bus-compressed loops smear their transients and lose
//! onsets before this ever sees them. And if the classifier cannot separate a rim
//! from a snap it picks one; TASK-058H's rule is that it must not invent a lane
//! to fill the grid.

use engine::midi::gm_drum_note;
use engine::pattern::{Lane, Note};

use super::grid::{division_ticks, Grid, QUARTER};
use super::onsets::Onset;

/// How much of a hit must be under 120 Hz for it to be the kick.
///
/// ⚠ Half. A kick is not *only* low — a real one has a click at 3 kHz that is
/// most of how you hear it on a laptop — but half its energy under 120 Hz is
/// something no other drum in a kit does.
const KICK_LOW_SHARE: f32 = 0.5;

/// ...and how much must be above 2 kHz for it to be a hat or a cymbal.
///
/// ⚠ **0.55 rather than something higher**, because real hats are recorded with
/// a room in them and a room has a low end. What matters is that the majority is
/// up top, which a snare's never is.
const HAT_HIGH_SHARE: f32 = 0.55;

/// How much body a broadband hit needs before it is a snare rather than a hat.
///
/// ⛔ **This is what separates a clap from an open hat**, which otherwise look
/// alike: both are wide, both ring. A clap has 150–400 Hz in it because it is
/// hands hitting; a hat has almost none.
const SNARE_BODY_SHARE: f32 = 0.18;

/// ...and how much crack, because a tom has body and no crack at all.
const SNARE_HIGH_SHARE: f32 = 0.15;

/// How long a hat may ring and still be a closed one, in seconds.
///
/// ⚠ Measured against the envelope's own floor: the 2–8 kHz band is smoothed at
/// 200 Hz, so the shortest decay it can report is about 5 ms. A closed hat lands
/// at 20–40 ms and an open one does not reach −12 dB for 150 ms or more.
const CLOSED_HAT_DECAY: f32 = 0.09;

/// How many hits at an off-duple spacing make a roll.
///
/// ⚠ Three. Two hats a triplet apart is an accident of where the grid fell;
/// three in a row is a player.
const ROLL_RUN: usize = 3;

/// How many pitched hits in a row make a fill.
const FILL_RUN: usize = 3;

/// The dynamic range velocity is spread over, in decibels.
///
/// ⚠ **30 dB, referenced to the loudest hit in the file rather than to full
/// scale.** A loop mastered four decibels quieter must produce the same
/// velocities, or the same beat imported twice would play differently depending
/// on how the producer's sample pack was rendered.
const VELOCITY_RANGE_DB: f32 = 30.0;

/// The quietest a detected hit is written at.
///
/// ⚠ Not 1. A hit the detector found is a hit that was audible, and a velocity of
/// 1 through a sampler is silence — which would be the readout-that-lies failure
/// on the pad grid: a dot showing where nothing sounds.
const MIN_VELOCITY: u8 = 28;

/// What pass 1 thinks a hit is, before the bar gets a say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proposal {
    Kick,
    /// A snare or a clap. **Pass 1 cannot tell these apart and does not try** —
    /// see the module note.
    Broadband,
    ClosedHat,
    OpenHat,
    /// Body, no crack: a tom, a conga, a woodblock.
    Pitched,
    /// Detected, and none of the above described it.
    Other,
}

/// How few hits a file may have and still be a drum loop.
///
/// ⚠ Four. Two bars of a kick on one and three is four hits, which is the
/// sparsest thing anyone would call a beat.
const MIN_HITS: usize = 4;

/// ...and what share of them must read as a kit.
const KIT_SHARE: f32 = 0.5;

/// The longest a kit's hits may typically ring, in seconds.
///
/// ⛔⛔ **The spectral test alone was not enough and four real stems proved it.**
/// Almost anything with an attack has some 150–400 Hz body and some 2–8 kHz
/// crack, so `Broadband` fires on a piano, a brass stab and a syllable — the
/// Anthem Keys stem came back as **662 drum notes** and the *Vocal* stem as 621.
///
/// ▶ **What actually separates a kit from an instrument is that a kit stops.**
/// Measured as the median time to fall 12 dB: `Starlight.wav`, a trap loop, is
/// **0.105 s**; the Keys, Pad, Brass and Vocal stems are all **0.500** — which is
/// the cap, meaning none of them had decayed at all. A quarter of a second sits
/// between them with room on both sides.
///
/// ⚠ It is a *median*, so a loop full of open hats and a crash still reads as
/// drums as long as most of it is short.
const KIT_MAX_MEDIAN_DECAY: f32 = 0.25;

/// Is this a drum loop at all?
///
/// ⛔⛔ **Every note start is an onset, so without this a pad is a drum
/// pattern.** A held string chord produces a perfectly good onset for each note
/// it enters on, none of which is percussion — and pass 1 has an `Other` arm, so
/// they would all arrive on `Perc` and the producer would be looking at a grid of
/// hits for a file with no drums in it. That is the silent mis-file the whole
/// feature is written to avoid, and it is the audio twin of `looks_like_a_kit`
/// on the MIDI side.
///
/// ▶ **Asked of the proposals rather than of the audio**, so there is one
/// spectral rule and not two: a file is a drum loop when half of what is in it
/// looks like a drum.
pub fn percussive(onsets: &[Onset]) -> bool {
    if onsets.len() < MIN_HITS {
        return false;
    }
    // ⛔ **The decay test first, because it is the one that works on real
    // material** — see [`KIT_MAX_MEDIAN_DECAY`].
    let mut decays: Vec<f32> = onsets.iter().map(|onset| onset.decay).collect();
    decays.sort_by(f32::total_cmp);
    if decays[decays.len() / 2] > KIT_MAX_MEDIAN_DECAY {
        return false;
    }

    let kit = onsets
        .iter()
        .filter(|onset| {
            matches!(
                propose(onset),
                Proposal::Kick | Proposal::Broadband | Proposal::ClosedHat | Proposal::OpenHat
            )
        })
        .count();
    kit as f32 / onsets.len() as f32 >= KIT_SHARE
}

/// Every hit, on the lane it belongs to.
///
/// ⚠ Returns lanes rather than notes so the caller can ask *"which lanes did this
/// file actually use"* — which is what TASK-058H's muting is.
pub fn classify(onsets: &[Onset], grid: &Grid) -> Vec<(Lane, Note)> {
    let mut proposals: Vec<Proposal> = onsets.iter().map(propose).collect();

    // ⚠ **Derived once.** `backbeat_lane` used to call `backbeat_steps` itself,
    // which allocates — so a file with five hundred snare hits allocated a
    // two-element `Vec` five hundred times for a list that depends only on the
    // meter.
    let steps = backbeat_steps(grid);
    backbeat(onsets, grid, &steps, &mut proposals);
    let rolling = rolls(onsets, grid, &proposals);
    let fills = fill_order(onsets, &proposals);

    let reference = onsets
        .iter()
        .map(Onset::level)
        .fold(0.0_f32, f32::max)
        .max(f32::MIN_POSITIVE);
    let division = division_ticks(grid.time_sig_num, grid.time_sig_den);

    onsets
        .iter()
        .enumerate()
        .map(|(at, onset)| {
            let lane = match proposals[at] {
                Proposal::Kick => Lane::Kick,
                Proposal::Broadband => backbeat_lane(grid.sixteenth_of(onset.at), &steps),
                // ⛔ **A roll is one lane**, whatever each hit's own decay said.
                // Mike: *"triplets being heard for hihats … it splits it into the
                // proper channels"* — a roll that alternates closed and open
                // because two of its hits rang 10 ms longer is not what anybody
                // played.
                Proposal::OpenHat if rolling[at] => Lane::ClosedHat,
                Proposal::ClosedHat => Lane::ClosedHat,
                Proposal::OpenHat => Lane::OpenHat,
                Proposal::Pitched => fills[at].unwrap_or(Lane::Perc),
                Proposal::Other => Lane::Perc,
            };
            (
                lane,
                Note {
                    start_tick: grid.tick_of(onset.at),
                    // ⚠ From the hit's own decay, so an open hat draws longer than
                    // a closed one — bounded below by a division (a zero-length
                    // note is invisible in the roll) and above by a quarter (a
                    // ride's tail is not a held note).
                    len_ticks: ((onset.decay * grid.bpm.max(1.0) / 60.0 * QUARTER as f32) as u32)
                        .clamp(division, QUARTER),
                    pitch: gm_drum_note(lane),
                    vel: velocity(onset.level(), reference),
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                    reversed: false,
                },
            )
        })
        .collect()
}

/// Pass 1: what the spectrum alone says.
fn propose(onset: &Onset) -> Proposal {
    if onset.low_share() >= KICK_LOW_SHARE {
        return Proposal::Kick;
    }
    if onset.body_share() >= SNARE_BODY_SHARE && onset.high_share() >= SNARE_HIGH_SHARE {
        return Proposal::Broadband;
    }
    if onset.high_share() >= HAT_HIGH_SHARE {
        return if onset.decay <= CLOSED_HAT_DECAY {
            Proposal::ClosedHat
        } else {
            Proposal::OpenHat
        };
    }
    if onset.body_share() >= SNARE_BODY_SHARE {
        return Proposal::Pitched;
    }
    Proposal::Other
}

/// Which sixteenths of the bar the backbeat is on.
///
/// 4 and 12 in 4/4 — beats two and four. ⚠ Derived from the meter rather than
/// written down, because "the 4 beat" in 6/8 is not sixteenth 12.
fn backbeat_steps(grid: &Grid) -> Vec<u32> {
    let per_bar = u32::from(grid.time_sig_num) * 16 / u32::from(grid.time_sig_den.max(1));
    let per_beat = (per_bar / u32::from(grid.time_sig_num).max(1)).max(1);
    (0..u32::from(grid.time_sig_num))
        .filter(|beat| beat % 2 == 1)
        .map(|beat| beat * per_beat)
        .collect()
}

/// Pass 2, rule 1 — anything landing on two and four is the backbeat.
///
/// ⛔ Mike: *"snares on the 4 beat"*, and *"broadband hits consistently on 2 and
/// 4 → the backbeat lane, whatever the spectrum guessed."* So a hit the spectrum
/// filed as percussion, sitting on beat two of every bar, is promoted: the bar
/// outranks the band.
///
/// ⚠ **"Consistently", which is why this counts first.** One perc hit that
/// happens to fall on beat four is not a snare. It has to be a pattern —
/// something on the backbeat in most of the bars — before the rule fires at all.
fn backbeat(onsets: &[Onset], grid: &Grid, steps: &[u32], proposals: &mut [Proposal]) {
    if steps.is_empty() {
        return;
    }
    let on_backbeat = |onset: &Onset| steps.contains(&grid.sixteenth_of(onset.at));

    let hits = onsets.iter().filter(|onset| on_backbeat(onset)).count();
    // Two thirds of the bars carrying one, which is what "consistently" means for
    // a backbeat and what a stray coincidence cannot reach.
    if (hits as f32) < f32::from(grid.bars) * 1.3 {
        return;
    }

    for (at, onset) in onsets.iter().enumerate() {
        if on_backbeat(onset) && matches!(proposals[at], Proposal::Pitched | Proposal::Other) {
            proposals[at] = Proposal::Broadband;
        }
    }
}

/// Which snare lane a broadband hit lands on.
///
/// ⛔ **Three lanes, because this kit has three and they mean different things.**
/// `Snare` is the backbeat. `OffSnare` is *"a second snare voice on the
/// off-beats"* — its own doc. `GhostSnare` is *"the quiet answering snare on the
/// e/a slots"*, which is a statement about velocity as much as position.
///
/// ⚠ **A clap is not separated out, and that is honesty rather than an
/// oversight.** A clap and a snare on the backbeat are spectrally the same event,
/// and TASK-058H's rule for exactly this case is that the classifier *"picks one
/// and says so"* rather than inventing a distinction. The producer moves the
/// sample; the notes are right either way.
fn backbeat_lane(sixteenth: u32, steps: &[u32]) -> Lane {
    if steps.contains(&sixteenth) {
        return Lane::Snare;
    }
    // The e and a of a beat — the odd sixteenths — are where a ghost lives.
    if sixteenth % 2 == 1 {
        return Lane::GhostSnare;
    }
    Lane::OffSnare
}

/// Pass 2, rule 2 — high-band hits at a spacing the duple grid does not have.
///
/// ⛔ Mike: *"triplets being heard for hihats"*, and the roadmap: *"high-band hits
/// at triplet spacing inside one beat → a hat roll, one lane."*
///
/// ⚠ **Measured on the SPACING, not on the count.** Six hats in a beat could be
/// sixteenth-triplets or thirty-seconds, and those are different performances;
/// what says which is whether the gaps divide the beat into three. A run of at
/// least [`ROLL_RUN`] hits whose *every gap* is not a whole number of sixteenths
/// is a roll.
///
/// ⚠ **It does NOT require the gaps to be equal to each other**, and the
/// difference is worth naming: a roll that accelerates is still a roll, and
/// demanding evenness would drop exactly the fills a producer plays by hand.
fn rolls(onsets: &[Onset], grid: &Grid, proposals: &[Proposal]) -> Vec<bool> {
    let mut rolling = vec![false; onsets.len()];
    let beat_seconds = grid.bar_seconds() / f32::from(grid.time_sig_num.max(1));
    if beat_seconds <= 0.0 {
        return rolling;
    }

    let high: Vec<usize> = (0..onsets.len())
        .filter(|at| matches!(proposals[*at], Proposal::ClosedHat | Proposal::OpenHat))
        .collect();

    // ⚠ **A run is a contiguous window of `high`, so it is two indices rather
    // than a `Vec`.** The first cut collected the members and needed a closure
    // taking both pieces of state as arguments purely to dodge a borrow.
    let mut from: Option<usize> = None;
    // `at` is an index *into `high`*, and the run being closed is `high[from..=at]`.
    let close = |from: &mut Option<usize>, at: usize, rolling: &mut Vec<bool>| {
        let Some(start) = from.take() else { return };
        if at + 1 - start >= ROLL_RUN {
            for onset in &high[start..=at] {
                rolling[*onset] = true;
            }
        }
    };

    for at in 1..high.len() {
        let gap = onsets[high[at]].at - onsets[high[at - 1]].at;
        // In sixteenths of a beat's quarter: 1.0 is a sixteenth, 1.333 a
        // sixteenth triplet, 0.667 a thirty-second triplet.
        let sixteenths = gap / (beat_seconds / 4.0);
        let off_duple = (sixteenths - sixteenths.round()).abs() > 0.15;
        let inside_a_beat = gap < beat_seconds;
        if off_duple && inside_a_beat && sixteenths > 0.2 {
            from.get_or_insert(at - 1);
        } else {
            close(&mut from, at - 1, &mut rolling);
        }
    }
    close(&mut from, high.len().saturating_sub(1), &mut rolling);
    rolling
}

/// Pass 2, rule 3 — a run of pitched hits that climbs or falls is a fill.
///
/// ⛔ Mike: *"percussions sounds going up and down"*, and the roadmap: *"evenly
/// spaced pitched hits with a rising/falling centroid → a tom/perc fill, routed
/// in pitch order."*
///
/// ⚠ **Routed in pitch ORDER rather than by pitch**, and the difference matters:
/// four bands cannot tell you what note a tom is, only which of two toms is
/// higher. So the run is sorted by [`Onset::centroid`] and split into low, mid and
/// high thirds — which is what a three-tom fill *is*, and which stays right
/// whatever the drums were actually tuned to.
fn fill_order(onsets: &[Onset], proposals: &[Proposal]) -> Vec<Option<Lane>> {
    let mut out: Vec<Option<Lane>> = vec![None; onsets.len()];

    let mut at = 0;
    while at < onsets.len() {
        if proposals[at] != Proposal::Pitched {
            at += 1;
            continue;
        }
        let mut end = at;
        while end + 1 < onsets.len() && proposals[end + 1] == Proposal::Pitched {
            end += 1;
        }
        let run = at..=end;
        at = end + 1;
        if end + 1 - *run.start() < FILL_RUN {
            continue;
        }

        // Monotone in centroid — up or down, and it has to be one of them. A run
        // that wanders is percussion, not a fill.
        let centroids: Vec<f32> = run.clone().map(|index| onsets[index].centroid()).collect();
        let rising = centroids.windows(2).all(|pair| pair[1] >= pair[0]);
        let falling = centroids.windows(2).all(|pair| pair[1] <= pair[0]);
        if !rising && !falling {
            continue;
        }

        let low = centroids.iter().copied().fold(f32::INFINITY, f32::min);
        let high = centroids.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let span = (high - low).max(f32::MIN_POSITIVE);
        for (index, centroid) in run.zip(&centroids) {
            let share = (centroid - low) / span;
            out[index] = Some(if share < 1.0 / 3.0 {
                Lane::TomLow
            } else if share < 2.0 / 3.0 {
                Lane::Tom
            } else {
                Lane::TomHigh
            });
        }
    }

    out
}

/// A hit's level as a MIDI velocity, referenced to the loudest hit in the file.
fn velocity(level: f32, reference: f32) -> u8 {
    let db = 20.0 * (level.max(f32::MIN_POSITIVE) / reference).log10();
    let share = (1.0 + db / VELOCITY_RANGE_DB).clamp(0.0, 1.0);
    let vel = f32::from(MIN_VELOCITY) + share * f32::from(127 - MIN_VELOCITY);
    vel.round().clamp(f32::from(MIN_VELOCITY), 127.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: Grid = Grid::at(120.0, 1, 4, 4);

    /// A hit at `sixteenth` of the bar with the given band mix.
    fn hit(sixteenth: f32, low: f32, body: f32, presence: f32, air: f32, decay: f32) -> Onset {
        Onset {
            at: sixteenth * GRID.bar_seconds() / 16.0,
            low,
            body,
            presence,
            air,
            decay,
            strength: 1.0,
        }
    }

    fn kick(sixteenth: f32) -> Onset {
        hit(sixteenth, 1.0, 0.15, 0.02, 0.005, 0.12)
    }

    fn snare(sixteenth: f32) -> Onset {
        hit(sixteenth, 0.05, 0.5, 0.6, 0.25, 0.1)
    }

    fn closed_hat(sixteenth: f32) -> Onset {
        hit(sixteenth, 0.002, 0.02, 0.5, 0.4, 0.03)
    }

    fn open_hat(sixteenth: f32) -> Onset {
        hit(sixteenth, 0.002, 0.02, 0.5, 0.4, 0.25)
    }

    /// A tom: body, and no crack at all.
    ///
    /// ⚠ `height` (`0..1`) moves the centroid by shifting energy from the low
    /// band into the body — **not** by adding presence, which is what the first
    /// cut of this fixture did. Presence is exactly what separates a tom from a
    /// snare, so the "highest tom" in that version was classified broadband and
    /// the test was measuring the fixture rather than the rule.
    fn tom(sixteenth: f32, height: f32) -> Onset {
        hit(
            sixteenth,
            0.5 * (1.0 - height),
            0.5 + 0.5 * height,
            0.05,
            0.005,
            0.15,
        )
    }

    fn lanes_of(onsets: &[Onset]) -> Vec<Lane> {
        classify(onsets, &GRID)
            .into_iter()
            .map(|(lane, _)| lane)
            .collect()
    }

    #[test]
    fn the_spectrum_alone_separates_a_kick_a_snare_and_two_hats() {
        let found = lanes_of(&[kick(0.0), closed_hat(2.0), snare(4.0), open_hat(6.0)]);
        assert_eq!(
            found,
            vec![Lane::Kick, Lane::ClosedHat, Lane::Snare, Lane::OpenHat],
        );
    }

    #[test]
    fn the_backbeat_outranks_the_spectrum() {
        // ⛔ Mike's rule: *"broadband hits consistently on 2 and 4 → the backbeat
        // lane, whatever the spectrum guessed."* These read as percussion — body,
        // no crack — and they are on two and four of every bar.
        let grid = Grid { bars: 2, ..GRID };
        let bar = grid.bar_seconds();
        let at = |beat: f32| Onset {
            at: beat * bar / 4.0,
            ..tom(0.0, 0.5)
        };
        let onsets = [at(1.0), at(3.0), at(5.0), at(7.0)];
        let found: Vec<Lane> = classify(&onsets, &grid)
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        assert!(
            found.iter().all(|lane| *lane == Lane::Snare),
            "the bar has to outrank the band: {found:?}"
        );
    }

    #[test]
    fn one_stray_hit_on_beat_four_is_not_a_snare() {
        // ⚠ "Consistently" is load-bearing: a single perc hit that happens to
        // land on the backbeat must not promote itself.
        let found = lanes_of(&[tom(0.0, 0.5), tom(2.0, 0.5), tom(12.0, 0.5)]);
        assert!(
            !found.contains(&Lane::Snare),
            "three toms became a snare: {found:?}"
        );
    }

    #[test]
    fn a_triplet_hat_roll_lands_on_one_lane() {
        // ⛔ Mike: *"triplets being heard for hihats."* These six sit at sixteenth
        // triplets across two beats, and two of them ring long enough that the
        // spectrum alone would call them open hats — which would draw the roll
        // alternating between two lanes.
        let third = 4.0 / 3.0;
        let mut onsets: Vec<Onset> = (0..6).map(|step| closed_hat(step as f32 * third)).collect();
        onsets[2].decay = 0.2;
        onsets[4].decay = 0.2;

        let found = lanes_of(&onsets);
        assert!(
            found.iter().all(|lane| *lane == Lane::ClosedHat),
            "a roll is one lane: {found:?}"
        );
    }

    #[test]
    fn plain_eighth_hats_are_not_a_roll_and_keep_their_own_decays() {
        let onsets = [
            closed_hat(0.0),
            closed_hat(2.0),
            open_hat(4.0),
            closed_hat(6.0),
        ];
        assert_eq!(
            lanes_of(&onsets),
            vec![
                Lane::ClosedHat,
                Lane::ClosedHat,
                Lane::OpenHat,
                Lane::ClosedHat
            ],
        );
    }

    #[test]
    fn a_tom_fill_is_routed_in_pitch_order() {
        // ⛔ Mike: *"percussions sounds going up and down … routed in pitch
        // order."* Four toms climbing.
        let found = lanes_of(&[
            tom(8.0, 0.0),
            tom(10.0, 0.35),
            tom(12.0, 0.7),
            tom(14.0, 1.0),
        ]);
        assert_eq!(found[0], Lane::TomLow);
        assert_eq!(*found.last().expect("four"), Lane::TomHigh);
        // Monotone, which is what "in order" means.
        assert!(
            found
                .windows(2)
                .all(|pair| lane_height(pair[0]) <= lane_height(pair[1])),
            "{found:?}"
        );
    }

    /// Low to high, for the assertion above.
    fn lane_height(lane: Lane) -> u8 {
        match lane {
            Lane::TomLow => 0,
            Lane::Tom => 1,
            Lane::TomHigh => 2,
            _ => 3,
        }
    }

    #[test]
    fn a_run_that_wanders_is_percussion_rather_than_a_fill() {
        // ⚠ A fill goes somewhere. Three toms at random heights is a groove.
        let found = lanes_of(&[tom(0.0, 0.9), tom(4.0, 0.0), tom(8.0, 0.5)]);
        assert!(found.iter().all(|lane| *lane == Lane::Perc), "{found:?}");
    }

    #[test]
    fn velocity_is_relative_to_the_loudest_hit_rather_than_to_full_scale() {
        // ⛔ The same beat rendered four decibels quieter must import identically,
        // or a producer's sample pack decides how hard their drums are played.
        let loud = [kick(0.0), snare(4.0)];
        let quiet: Vec<Onset> = loud
            .iter()
            .map(|onset| Onset {
                low: onset.low * 0.63,
                body: onset.body * 0.63,
                presence: onset.presence * 0.63,
                air: onset.air * 0.63,
                ..*onset
            })
            .collect();
        let vels = |onsets: &[Onset]| -> Vec<u8> {
            classify(onsets, &GRID)
                .into_iter()
                .map(|(_, note)| note.vel)
                .collect()
        };
        assert_eq!(vels(&loud), vels(&quiet));
    }

    #[test]
    fn a_detected_hit_is_never_written_at_a_velocity_that_is_silence() {
        // ⚠ The pad grid draws a dot per note. A dot where nothing sounds is the
        // readout-that-lies failure in miniature.
        let onsets = [kick(0.0), hit(4.0, 0.00002, 0.00001, 0.0, 0.0, 0.02)];
        for (_, note) in classify(&onsets, &GRID) {
            assert!(note.vel >= MIN_VELOCITY, "velocity {}", note.vel);
        }
    }

    #[test]
    fn every_hit_lands_on_the_gm_pitch_its_lane_owns() {
        // ⛔ **The routing is `into_lanes`' job and this has to speak its
        // language.** `engine::smf_read` puts a drum note on a lane by inverting
        // `gm_drum_note`, so a note written at anything else is a note that
        // silently disappears on the way into a pattern.
        for (lane, note) in classify(&[kick(0.0), snare(4.0), closed_hat(2.0)], &GRID) {
            assert_eq!(note.pitch, gm_drum_note(lane));
        }
    }
}
