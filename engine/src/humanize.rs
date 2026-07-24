//! Humanization: swing, velocity tiers, per-lane timing jitter, quantize blend.
//!
//! Every generator produces notes on the exact grid and then hands them here.
//! Keeping it in one pass is what makes the feel consistent across parts — a
//! swung hat and a straight 808 in the same pattern is a bug, not a style.
//!
//! The four knobs, and what each is for (research ch. 1 cross-genre constants):
//!
//! - **Swing** warps the timeline: MPC scale, 50% straight … 66% perfect
//!   triplet. Deliberate groove, authored per model.
//! - **Velocity tiers** turn a note's *role* — accent, main, ghost — into a
//!   number. Accents 100–127, mains 60–70% of full, ghosts 30–40%.
//! - **Timing jitter** is human sloppiness, bounded per lane in milliseconds,
//!   because a kick and a hat do not breathe alike.
//! - **Quantize strength** is what the producer did about that sloppiness
//!   afterwards. `1.0` snaps hard to the grid; `0.0` leaves the raw
//!   performance. It scales the jitter and never touches the swing — swing is
//!   intended and quantizing it away would be a different feel, not a tighter
//!   one.
//!
//! So the shipped defaults (±1–3 ms jitter, 0.92 strength) are modern-trap
//! tight: essentially on the grid, with the life coming from velocity. A
//! neo-soul model asks for the opposite by authoring a wide jitter and a
//! strength near 0.65–0.75 (research ch. 1 §7).

use rand::Rng;
use serde_json::Value;

use crate::context::{SessionContext, Swing, SwingGrid};
use crate::pattern::{Articulation, Lane, LaneTrack, Note, PPQ};
use crate::rng;

/// The MPC swing scale, as named by the hardware and the research.
///
/// These are the values models should reach for; anything between them is
/// legal. The scale delays every second subdivision, so 0.50 leaves the pair
/// even and 0.667 lands the second note exactly on the triplet.
pub const SWING_STRAIGHT: f32 = 0.50;
/// Subtle groove — modern hip-hop and R&B.
pub const SWING_SUBTLE: f32 = 0.54;
/// Classic MPC boom bap.
pub const SWING_MPC: f32 = 0.58;
/// Heavy shuffle — neo-soul.
pub const SWING_SHUFFLE: f32 = 0.62;
/// Perfect triplet.
pub const SWING_TRIPLET: f32 = 0.66;

/// The swing range the dataset validator accepts.
///
/// The research calls 50–70% the musical zone; the dataset lint allows up to
/// 0.75 and this matches it deliberately. Clamping tighter here would silently
/// change a value `datasetc` told the author was fine, which is the kind of
/// disagreement nobody finds.
const SWING_RANGE: std::ops::RangeInclusive<f32> = 0.5..=0.75;

/// A velocity band for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub lo: u8,
    pub hi: u8,
}

impl Band {
    fn new(lo: u8, hi: u8) -> Self {
        // Ordered rather than trusted. An inverted range is already a dataset
        // lint failure, so this only ever fires on a hand-built band.
        Band {
            // Both ends clamped. `hi` was, `lo` was not, so an authored
            // `[130, 140]` produced `lo: 130, hi: 127`, `pick` saw
            // `lo >= hi` and returned 130 — an illegal MIDI velocity that
            // survived into the pattern and the golden snapshots. The writer
            // clamps on export, so the `.mid` was legal while the Pattern the
            // UI reads was not.
            lo: lo.min(hi).clamp(1, 127),
            hi: hi.max(lo).clamp(1, 127),
        }
    }

    fn pick(&self, rng: &mut impl Rng) -> u8 {
        if self.lo >= self.hi {
            self.lo
        } else {
            rng.random_range(self.lo..=self.hi)
        }
    }
}

/// What a note's role is worth in velocity (research ch. 1: accents 100–127,
/// regular hits 60–70%, ghost notes 30–40%).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VelocityTiers {
    pub accent: Band,
    pub main: Band,
    pub ghost: Band,
}

impl Default for VelocityTiers {
    fn default() -> Self {
        Self {
            accent: Band { lo: 100, hi: 127 },
            main: Band { lo: 76, hi: 89 },
            ghost: Band { lo: 38, hi: 51 },
        }
    }
}

impl VelocityTiers {
    /// Read `drums.velocityTiers` from a resolved model.
    ///
    /// Falls back **per band**, not all-or-nothing: a model that states only its
    /// ghost range keeps the researched values for the other two rather than
    /// losing them to a partial object.
    pub fn from_json(drums: Option<&Value>) -> Self {
        let defaults = Self::default();
        let Some(tiers) = drums.and_then(|d| d.get("velocityTiers")) else {
            return defaults;
        };

        let band = |name: &str, fallback: Band| {
            tiers
                .get(name)
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .and_then(|a| {
                    let lo = a[0].as_f64()?;
                    let hi = a[1].as_f64()?;
                    Some(Band::new(lo.round() as u8, hi.round() as u8))
                })
                .unwrap_or(fallback)
        };

        Self {
            accent: band("accent", defaults.accent),
            main: band("main", defaults.main),
            ghost: band("ghost", defaults.ghost),
        }
    }

    /// The band a note of this articulation belongs in.
    ///
    /// Everything that is not explicitly a ghost or an accent is a main hit,
    /// including `Roll` — roll velocities come from [`ramp`], which the roll
    /// engine applies over the whole run rather than note by note.
    pub fn band(&self, articulation: Option<Articulation>) -> Band {
        match articulation {
            Some(Articulation::Ghost) => self.ghost,
            Some(Articulation::Accent) => self.accent,
            _ => self.main,
        }
    }

    /// The velocity for one note of a given role.
    ///
    /// This is the *musical* value only. Human inconsistency is added once, by
    /// [`humanize`], so a generator calling this cannot double-apply it.
    pub fn pick(&self, articulation: Option<Articulation>, rng: &mut impl Rng) -> u8 {
        self.band(articulation).pick(rng)
    }
}

/// Ticks spanned by one swing pair — two of whatever subdivision is swung.
fn pair_ticks(grid: SwingGrid) -> u32 {
    match grid {
        // Two 8ths make a quarter note.
        SwingGrid::Eighth => PPQ,
        SwingGrid::Sixteenth => PPQ / 2,
    }
}

/// Warp one tick through the swing map.
///
/// The whole timeline is warped, not just the notes that happen to sit on a
/// subdivision: the first half of each pair is stretched to `amount` of its
/// length and the second half compressed into what is left. That keeps the map
/// continuous and monotonic, so a 32nd-note roll inside a swung 8th is carried
/// along with the beat it belongs to instead of being torn off it — and notes
/// never change order, whatever resolution they were written at.
pub fn swing_tick(tick: u32, swing: Swing) -> u32 {
    let amount = swing.amount.clamp(*SWING_RANGE.start(), *SWING_RANGE.end());
    if (amount - SWING_STRAIGHT).abs() < 1e-6 {
        return tick;
    }

    let pair = pair_ticks(swing.grid) as f64;
    let position = tick as f64 / pair;
    let index = position.floor();
    let within = position - index;

    let amount = f64::from(amount);
    let warped = if within < 0.5 {
        within * (amount / 0.5)
    } else {
        amount + (within - 0.5) * ((1.0 - amount) / 0.5)
    };

    ((index + warped) * pair).round() as u32
}

/// A linear velocity ramp across a run of notes — hat rolls, snare builds,
/// risers (research ch. 1: rolls ramp 50→100% into the target beat; the generic
/// 8-bar riser ramps 16→127 linear).
///
/// Ramping down is the same call with the arguments swapped, which the roll
/// vocabulary needs — the ramp-down variants are half of it.
pub fn ramp(notes: &mut [Note], from: u8, to: u8) {
    let last = notes.len().saturating_sub(1);
    if last == 0 {
        // A one-note ramp is its own destination: the arrival is the point of
        // the gesture, and there is no run to climb.
        if let Some(note) = notes.first_mut() {
            note.vel = to.max(1);
        }
        return;
    }

    // Interpolated rather than stepped by an integer division: truncation is
    // toward zero, so a descending ramp — half the roll vocabulary — landed a
    // unit short of its destination while the ascending one was exact.
    let (from, to) = (f32::from(from), f32::from(to));
    for (i, note) in notes.iter_mut().enumerate() {
        let t = i as f32 / last as f32;
        note.vel = (from + (to - from) * t).round().clamp(1.0, 127.0) as u8;
    }
}

/// A stable RNG domain per lane.
///
/// Spelled out rather than derived from `Debug`, which is a formatting detail
/// that could change and silently reseed every existing pattern. Exhaustive, so
/// a new lane is a compile error here rather than a lane that quietly shares
/// another's stream.
fn lane_domain(lane: Lane) -> &'static str {
    match lane {
        Lane::Kick => "humanize/kick",
        Lane::Snare => "humanize/snare",
        Lane::Clap => "humanize/clap",
        Lane::ClosedHat => "humanize/closedHat",
        Lane::OpenHat => "humanize/openHat",
        Lane::Rim => "humanize/rim",
        Lane::Snap => "humanize/snap",
        Lane::Perc => "humanize/perc",
        Lane::Bass808 => "humanize/bass808",
        Lane::Melody => "humanize/melody",
        Lane::Counter => "humanize/counter",
        Lane::Bass => "humanize/bass",
        Lane::Chords => "humanize/chords",
    }
}

/// Apply swing, timing jitter and velocity variance to generated lanes.
///
/// Each lane draws from its own seeded stream, so rerolling the hats cannot
/// move the kick — the same rule the part streams follow (see [`crate::rng`]).
pub fn humanize(lanes: &mut [LaneTrack], ctx: &SessionContext, seed: u64) {
    let quantize = ctx.humanize.quantize_strength.clamp(0.0, 1.0);
    let variance = ctx.humanize.velocity_var.clamp(0.0, 1.0);
    // What survives quantization. At 1.0 nothing does and the notes sit exactly
    // on the swung grid.
    let looseness = 1.0 - quantize;

    for track in lanes.iter_mut() {
        let mut stream = rng::stream(seed, lane_domain(track.lane));
        let jitter_ticks = ctx
            .humanize
            .timing_jitter_ms
            .get(&track.lane)
            .map(|ms| ctx.ms_to_ticks(ms.abs()) * looseness)
            .unwrap_or(0.0);

        for note in &mut track.notes {
            let start = swing_tick(note.start_tick, ctx.swing);
            let end = swing_tick(note.start_tick + note.len_ticks, ctx.swing);

            // The note moves as a whole: a late hit is late, not stretched.
            let offset = if jitter_ticks > 0.0 {
                stream.random_range(-jitter_ticks..=jitter_ticks)
            } else {
                0.0
            };

            // A pattern cannot start before its own beginning, so an early hit
            // on tick 0 is pinned there rather than wrapping into a huge u32.
            note.start_tick = (start as f32 + offset).round().max(0.0) as u32;
            note.len_ticks = end.saturating_sub(start).max(1);
            note.vel = vary(note.vel, variance, &mut stream);
        }

        // Jitter can reorder two notes that were a hair apart. Lanes are
        // consumed as ordered streams — by the MIDI writer, the sequencer and
        // the grid alike — so the order is restored rather than left to them.
        track.notes.sort_by_key(|n| n.start_tick);
    }
}

/// Spread one velocity by the session's variance, keeping it a legal MIDI value.
fn vary(velocity: u8, variance: f32, rng: &mut impl Rng) -> u8 {
    if variance <= 0.0 {
        return velocity.max(1);
    }
    let factor = 1.0 + rng.random_range(-variance..=variance);
    (f32::from(velocity) * factor).round().clamp(1.0, 127.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Humanize;
    use serde_json::json;

    fn note(start: u32, len: u32) -> Note {
        Note {
            start_tick: start,
            len_ticks: len,
            pitch: 36,
            vel: 100,
            slide_to_pitch: None,
            articulation: None,
        }
    }

    fn swing(amount: f32) -> Swing {
        Swing {
            grid: SwingGrid::Sixteenth,
            amount,
        }
    }

    /// A feel: quantize strength, velocity variance, and per-lane jitter in ms.
    fn feel(quantize: f32, variance: f32, jitter: &[(Lane, f32)]) -> Humanize {
        Humanize {
            quantize_strength: quantize,
            velocity_var: variance,
            timing_jitter_ms: jitter.iter().copied().collect(),
        }
    }

    /// A default session wearing that feel.
    fn session(humanize: Humanize) -> SessionContext {
        SessionContext {
            humanize,
            ..Default::default()
        }
    }

    #[test]
    fn straight_swing_moves_nothing() {
        for tick in [0, 240, 480, 720, 960, 1337] {
            assert_eq!(swing_tick(tick, swing(SWING_STRAIGHT)), tick);
        }
    }

    #[test]
    fn triplet_swing_lands_the_offbeat_sixteenth_on_the_triplet() {
        // 66% is the perfect triplet: the second 16th of each pair moves from
        // half way (240) to two thirds (320) of the 8th.
        let s = swing(0.6667);
        assert_eq!(swing_tick(0, s), 0, "downbeats never move");
        assert_eq!(swing_tick(240, s), 320);
        assert_eq!(swing_tick(480, s), 480, "the next pair starts on the grid");
        assert_eq!(swing_tick(720, s), 800);
        assert_eq!(swing_tick(960, s), 960);
    }

    #[test]
    fn the_named_presets_delay_progressively() {
        let offbeat = |amount: f32| swing_tick(240, swing(amount));
        let straight = offbeat(SWING_STRAIGHT);
        let subtle = offbeat(SWING_SUBTLE);
        let mpc = offbeat(SWING_MPC);
        let shuffle = offbeat(SWING_SHUFFLE);
        let triplet = offbeat(SWING_TRIPLET);

        assert_eq!(straight, 240);
        assert!(
            straight < subtle && subtle < mpc && mpc < shuffle && shuffle < triplet,
            "{straight} {subtle} {mpc} {shuffle} {triplet}"
        );
        // 58% classic MPC: 58% of the 480-tick pair.
        assert_eq!(mpc, 278);
    }

    #[test]
    fn an_eighth_note_grid_swings_the_eighths_instead() {
        let s = Swing {
            grid: SwingGrid::Eighth,
            amount: 0.6667,
        };
        // The off-8th (480) moves to the 8th-note triplet (640); the 16th
        // between them is carried along rather than left behind.
        assert_eq!(swing_tick(480, s), 640);
        assert_eq!(swing_tick(0, s), 0);
        assert_eq!(swing_tick(960, s), 960);
        assert!(swing_tick(240, s) > 240);
    }

    #[test]
    fn swing_never_reorders_notes() {
        // The property that lets rolls survive swing: a 64th-note run inside a
        // swung pair must come out in the order it went in.
        for amount in [0.5, 0.54, 0.58, 0.62, 0.66, 0.75] {
            let s = swing(amount);
            let mut previous = 0;
            for tick in (0..1920).step_by(15) {
                let warped = swing_tick(tick, s);
                assert!(
                    warped >= previous,
                    "amount {amount}: tick {tick} warped to {warped}, behind {previous}"
                );
                previous = warped;
            }
        }
    }

    #[test]
    fn swing_outside_the_legal_range_is_clamped_not_wrapped() {
        // Beyond the range the validator accepts, the map must stay monotonic
        // rather than folding notes back over each other.
        assert_eq!(swing_tick(240, swing(2.0)), swing_tick(240, swing(0.75)));
        assert_eq!(swing_tick(240, swing(-1.0)), 240);
    }

    #[test]
    fn a_hard_quantized_session_lands_exactly_on_the_swung_grid() {
        // quantizeStrength 1.0 is the claim "nothing survives the grid". If
        // jitter leaked through here, every golden test downstream would be
        // seed-dependent in a way nobody could see.
        let mut ctx = SessionContext {
            swing: swing(0.6667),
            ..Default::default()
        };
        ctx.humanize = Humanize {
            quantize_strength: 1.0,
            velocity_var: 0.0,
            timing_jitter_ms: [(Lane::ClosedHat, 30.0)].into_iter().collect(),
        };

        let mut lanes = vec![LaneTrack {
            lane: Lane::ClosedHat,
            notes: (0..8).map(|i| note(i * 240, 60)).collect(),
        }];
        humanize(&mut lanes, &ctx, 42);

        let starts: Vec<u32> = lanes[0].notes.iter().map(|n| n.start_tick).collect();
        assert_eq!(starts, vec![0, 320, 480, 800, 960, 1280, 1440, 1760]);
    }

    #[test]
    fn an_unquantized_session_moves_notes_within_the_lane_bound() {
        // And it must actually move them: a jitter that rounds to nothing would
        // pass every "within bounds" assertion while doing nothing at all.
        let ctx = session(feel(0.0, 0.0, &[(Lane::Snare, 20.0)]));
        let bound = ctx.ms_to_ticks(20.0);

        let mut lanes = vec![LaneTrack {
            lane: Lane::Snare,
            notes: (0..32).map(|i| note(960 + i * 480, 60)).collect(),
        }];
        let before: Vec<u32> = lanes[0].notes.iter().map(|n| n.start_tick).collect();
        humanize(&mut lanes, &ctx, 7);

        let mut moved = 0;
        for (note, grid) in lanes[0].notes.iter().zip(&before) {
            let delta = note.start_tick as f32 - *grid as f32;
            assert!(
                delta.abs() <= bound.ceil(),
                "{delta} exceeds the {bound} tick bound"
            );
            if note.start_tick != *grid {
                moved += 1;
            }
        }
        assert!(
            moved > 16,
            "only {moved} of 32 notes moved — jitter is inert"
        );
    }

    #[test]
    fn quantize_strength_scales_the_jitter_rather_than_switching_it() {
        // Half strength must land between hard-quantized and free, or the knob
        // is a two-position switch wearing a percentage.
        let spread = |strength: f32| {
            let ctx = session(feel(strength, 0.0, &[(Lane::Kick, 40.0)]));
            let mut lanes = vec![LaneTrack {
                lane: Lane::Kick,
                notes: (0..64).map(|i| note(960 + i * 480, 60)).collect(),
            }];
            humanize(&mut lanes, &ctx, 11);
            lanes[0]
                .notes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.start_tick as f32 - (960 + i as u32 * 480) as f32).abs())
                .fold(0.0_f32, f32::max)
        };

        let loose = spread(0.0);
        let half = spread(0.5);
        let tight = spread(1.0);
        assert_eq!(tight, 0.0);
        assert!(half < loose * 0.75, "half {half} vs loose {loose}");
        assert!(half > 0.0);
    }

    #[test]
    fn swing_survives_quantization() {
        // Swing is intent, not sloppiness. A hard-quantized session keeps it;
        // pulling it back to the straight grid would be a different feel.
        let mut ctx = SessionContext {
            swing: swing(SWING_TRIPLET),
            ..Default::default()
        };
        ctx.humanize.quantize_strength = 1.0;

        let mut lanes = vec![LaneTrack {
            lane: Lane::ClosedHat,
            notes: vec![note(240, 60)],
        }];
        humanize(&mut lanes, &ctx, 1);
        assert!(lanes[0].notes[0].start_tick > 240);
    }

    #[test]
    fn a_note_on_the_downbeat_cannot_be_pushed_before_the_pattern() {
        let ctx = session(feel(0.0, 0.0, &[(Lane::Kick, 500.0)]));
        let mut lanes = vec![LaneTrack {
            lane: Lane::Kick,
            notes: (0..16).map(|_| note(0, 60)).collect(),
        }];
        humanize(&mut lanes, &ctx, 3);
        // Reaching this at all is the assertion — an early hit at tick 0 used to
        // be a u32 subtraction away from the far end of the pattern.
        assert!(lanes[0].notes.iter().all(|n| n.start_tick < 10_000));
    }

    #[test]
    fn lanes_come_back_in_time_order() {
        let ctx = session(feel(0.0, 0.0, &[(Lane::Perc, 60.0)]));
        let mut lanes = vec![LaneTrack {
            lane: Lane::Perc,
            // 60 ms of jitter against a 30 ms grid: notes will cross.
            notes: (0..24).map(|i| note(i * 60, 30)).collect(),
        }];
        humanize(&mut lanes, &ctx, 5);

        let starts: Vec<u32> = lanes[0].notes.iter().map(|n| n.start_tick).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn one_lane_does_not_move_another() {
        // The property rerolling depends on: regenerating the hats must leave
        // the kick byte-identical.
        let ctx = session(feel(
            0.2,
            0.2,
            &[(Lane::Kick, 10.0), (Lane::ClosedHat, 10.0)],
        ));

        let kick = || LaneTrack {
            lane: Lane::Kick,
            notes: (0..8).map(|i| note(i * 480, 60)).collect(),
        };

        let mut with_few_hats = vec![
            kick(),
            LaneTrack {
                lane: Lane::ClosedHat,
                notes: (0..4).map(|i| note(i * 240, 60)).collect(),
            },
        ];
        let mut with_many_hats = vec![
            kick(),
            LaneTrack {
                lane: Lane::ClosedHat,
                notes: (0..64).map(|i| note(i * 60, 30)).collect(),
            },
        ];

        humanize(&mut with_few_hats, &ctx, 99);
        humanize(&mut with_many_hats, &ctx, 99);
        assert_eq!(with_few_hats[0], with_many_hats[0]);
    }

    #[test]
    fn the_same_seed_reproduces_the_same_feel() {
        let ctx = session(feel(0.3, 0.2, &[(Lane::Snare, 15.0)]));
        let lanes = || {
            vec![LaneTrack {
                lane: Lane::Snare,
                notes: (0..16).map(|i| note(i * 240, 60)).collect(),
            }]
        };

        let (mut a, mut b, mut c) = (lanes(), lanes(), lanes());
        humanize(&mut a, &ctx, 2024);
        humanize(&mut b, &ctx, 2024);
        humanize(&mut c, &ctx, 2025);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn velocity_tiers_default_to_the_research_constants() {
        let tiers = VelocityTiers::default();
        assert_eq!(tiers.accent, Band { lo: 100, hi: 127 });
        assert_eq!(tiers.main, Band { lo: 76, hi: 89 });
        assert_eq!(tiers.ghost, Band { lo: 38, hi: 51 });
    }

    #[test]
    fn a_ghost_is_always_quieter_than_a_main_and_a_main_than_an_accent() {
        let tiers = VelocityTiers::default();
        let mut stream = rng::root_stream(1);
        for _ in 0..500 {
            let ghost = tiers.pick(Some(Articulation::Ghost), &mut stream);
            let main = tiers.pick(None, &mut stream);
            let accent = tiers.pick(Some(Articulation::Accent), &mut stream);
            assert!(ghost < main && main < accent, "{ghost} {main} {accent}");
        }
    }

    #[test]
    fn a_roll_note_takes_the_main_band_because_its_velocity_comes_from_the_ramp() {
        let tiers = VelocityTiers::default();
        assert_eq!(tiers.band(Some(Articulation::Roll)), tiers.main);
        assert_eq!(tiers.band(Some(Articulation::Legato)), tiers.main);
    }

    #[test]
    fn tiers_read_from_a_model_and_fall_back_one_band_at_a_time() {
        let drums = json!({ "velocityTiers": { "ghost": [20, 30] } });
        let tiers = VelocityTiers::from_json(Some(&drums));

        assert_eq!(tiers.ghost, Band { lo: 20, hi: 30 });
        // The bands the model said nothing about keep the researched values,
        // rather than a partial object costing them.
        assert_eq!(tiers.main, VelocityTiers::default().main);
        assert_eq!(tiers.accent, VelocityTiers::default().accent);
    }

    #[test]
    fn a_model_with_no_tiers_block_gets_the_defaults() {
        assert_eq!(VelocityTiers::from_json(None), VelocityTiers::default());
        assert_eq!(
            VelocityTiers::from_json(Some(&json!({ "kick": {} }))),
            VelocityTiers::default()
        );
    }

    #[test]
    fn a_ramp_climbs_linearly_and_hits_both_ends() {
        let mut notes: Vec<Note> = (0..5).map(|i| note(i * 60, 60)).collect();
        ramp(&mut notes, 50, 100);

        let velocities: Vec<u8> = notes.iter().map(|n| n.vel).collect();
        assert_eq!(velocities.first(), Some(&50));
        assert_eq!(velocities.last(), Some(&100));
        for pair in velocities.windows(2) {
            assert!(pair[1] > pair[0], "{velocities:?} is not climbing");
        }
        assert_eq!(velocities, vec![50, 63, 75, 88, 100]);
    }

    #[test]
    fn a_ramp_runs_downhill_too() {
        // Half the roll vocabulary is the ramp-down (research ch. 1 §1).
        let mut notes: Vec<Note> = (0..4).map(|i| note(i * 60, 60)).collect();
        ramp(&mut notes, 120, 40);
        let velocities: Vec<u8> = notes.iter().map(|n| n.vel).collect();
        assert_eq!(velocities.first(), Some(&120));
        assert_eq!(velocities.last(), Some(&40));
        for pair in velocities.windows(2) {
            assert!(pair[1] < pair[0], "{velocities:?} is not falling");
        }
    }

    #[test]
    fn the_riser_ramp_from_the_research_spans_its_whole_range() {
        // The generic 8-bar build: 16th-note stream, velocity 16 → 127 linear.
        let mut notes: Vec<Note> = (0..128).map(|i| note(i * 240, 60)).collect();
        ramp(&mut notes, 16, 127);
        assert_eq!(notes[0].vel, 16);
        assert_eq!(notes[127].vel, 127);
        assert!(
            notes[64].vel > 70 && notes[64].vel < 74,
            "{}",
            notes[64].vel
        );
    }

    #[test]
    fn a_one_note_ramp_is_its_destination() {
        let mut notes = vec![note(0, 60)];
        ramp(&mut notes, 16, 127);
        assert_eq!(notes[0].vel, 127);
        ramp(&mut [], 1, 2); // must not panic
    }

    #[test]
    fn no_velocity_variance_leaves_velocities_alone() {
        let mut ctx = SessionContext::default();
        ctx.humanize.velocity_var = 0.0;
        let mut lanes = vec![LaneTrack {
            lane: Lane::Clap,
            notes: (0..16).map(|i| note(i * 240, 60)).collect(),
        }];
        humanize(&mut lanes, &ctx, 4);
        assert!(lanes[0].notes.iter().all(|n| n.vel == 100));
    }

    #[test]
    fn velocity_variance_spreads_without_leaving_midi() {
        let mut ctx = SessionContext::default();
        ctx.humanize.velocity_var = 0.5;
        let mut lanes = vec![LaneTrack {
            lane: Lane::Clap,
            notes: (0..200).map(|i| note(i * 60, 60)).collect(),
        }];
        humanize(&mut lanes, &ctx, 8);

        let velocities: Vec<u8> = lanes[0].notes.iter().map(|n| n.vel).collect();
        assert!(velocities.iter().all(|v| (1..=127).contains(v)));
        assert!(
            velocities.iter().any(|v| *v != 100),
            "variance of 0.5 changed nothing"
        );
        // ±50% of 100 is 50–127 after the MIDI ceiling; nothing may fall out.
        assert!(velocities.iter().all(|v| *v >= 45));
    }

    #[test]
    fn an_extreme_variance_cannot_silence_or_overflow_a_note() {
        let mut ctx = SessionContext::default();
        // Clamped to 1.0 on the way in; the arithmetic still has to hold.
        ctx.humanize.velocity_var = 4.0;
        let mut lanes = vec![LaneTrack {
            lane: Lane::Clap,
            notes: (0..500).map(|i| note(i * 60, 60)).collect(),
        }];
        humanize(&mut lanes, &ctx, 6);
        assert!(lanes[0].notes.iter().all(|n| n.vel >= 1 && n.vel <= 127));
    }

    #[test]
    fn a_swung_note_keeps_its_length_in_warped_time() {
        // A note is warped end to end, so a legato 808 still meets the next one
        // after swing rather than opening a gap.
        let ctx = SessionContext {
            swing: swing(SWING_TRIPLET),
            humanize: Humanize {
                quantize_strength: 1.0,
                velocity_var: 0.0,
                timing_jitter_ms: Default::default(),
            },
            ..Default::default()
        };
        let mut lanes = vec![LaneTrack {
            lane: Lane::Bass808,
            notes: vec![note(0, 240), note(240, 240)],
        }];
        humanize(&mut lanes, &ctx, 12);

        let first = &lanes[0].notes[0];
        let second = &lanes[0].notes[1];
        assert_eq!(
            first.start_tick + first.len_ticks,
            second.start_tick,
            "the legato join opened up"
        );
        assert_eq!(second.start_tick + second.len_ticks, 480);
    }
}
