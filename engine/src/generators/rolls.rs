//! The roll vocabulary — a first-class deliverable, not a decoration (FR-003).
//!
//! Every device in research ch. 1 is the same shape underneath: take a window
//! of time, fill it at a finer subdivision than the part around it, and move
//! the velocity across it. A hat roll, a snare ladder, a clap roll, a kick-roll
//! build and an 8-bar riser differ only in lane, window, subdivision and ramp —
//! so there is one [`Roll`] and the devices are configurations of it.
//!
//! Two halves, and they are scheduled differently on purpose:
//!
//! - **Hat rolls schedule themselves.** The model authors `hihat.rolls` with
//!   `positions` and `freqPerBar`, so the hat part decides where its own rolls
//!   go. [`hat_rolls`] applies them.
//! - **Snare ladders, risers, clap rolls and kick builds are fill devices.**
//!   *When* they fire is the fill logic's call (TASK-022) — every two bars,
//!   every eight, before a section. They are exposed here as functions that
//!   render one, so the scheduler can place them without knowing how a ladder
//!   is built.

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::generators::grid;
use crate::generators::read::{flag, number, pair, string_spec, strings};
use crate::pattern::{Articulation, Lane, Note};

/// Accent grouping inside a roll.
///
/// `strong_weak_weak_weak` is the four-note grouping the snare-roll literature
/// describes: the first of each four carries the pulse so a long roll still has
/// a beat inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    Even,
    StrongWeakWeakWeak,
}

impl Grouping {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "even" => Some(Self::Even),
            "strong_weak_weak_weak" => Some(Self::StrongWeakWeakWeak),
            _ => None,
        }
    }

    /// How loud note `index` is, relative to the ramp value at that point.
    fn scale(self, index: usize) -> f32 {
        match self {
            Self::Even => 1.0,
            // The weak notes sit back; the strong one keeps the pulse audible.
            Self::StrongWeakWeakWeak => {
                if index.is_multiple_of(4) {
                    1.0
                } else {
                    0.72
                }
            }
        }
    }
}

/// One roll: a window filled at a subdivision, with the velocity moving across
/// it.
#[derive(Debug, Clone)]
pub struct Roll {
    pub lane: Lane,
    /// Window start, in absolute ticks.
    pub start_tick: u32,
    /// Window end, exclusive.
    pub end_tick: u32,
    /// Ticks between notes — a 16th is 240, a 32nd triplet 80.
    pub subdivision: u32,
    /// Velocity at the first note and at the last. `from > to` ramps down,
    /// which is half the vocabulary.
    pub from_vel: u8,
    pub to_vel: u8,
    /// Stop after this many notes — a burst rather than a filled window.
    pub max_notes: Option<usize>,
    /// Leave one-note holes, the "insert gaps" mutation from the mixed-res
    /// trap grammar.
    pub gaps: bool,
    /// Start the cluster this many subdivisions late, so it does not begin
    /// exactly on the beat.
    pub offset_subdivisions: u32,
    pub grouping: Grouping,
}

impl Roll {
    /// A roll filling a window at a subdivision, ramping 50% → 100%.
    pub fn new(lane: Lane, start_tick: u32, end_tick: u32, subdivision: u32) -> Self {
        Roll {
            lane,
            start_tick,
            end_tick,
            subdivision: subdivision.max(1),
            from_vel: 64,
            to_vel: 127,
            max_notes: None,
            gaps: false,
            offset_subdivisions: 0,
            grouping: Grouping::Even,
        }
    }

    pub fn ramp(mut self, from: u8, to: u8) -> Self {
        self.from_vel = from;
        self.to_vel = to;
        self
    }

    pub fn burst(mut self, notes: usize) -> Self {
        self.max_notes = Some(notes);
        self
    }

    pub fn with_gaps(mut self, gaps: bool) -> Self {
        self.gaps = gaps;
        self
    }

    pub fn offset(mut self, subdivisions: u32) -> Self {
        self.offset_subdivisions = subdivisions;
        self
    }

    pub fn grouped(mut self, grouping: Grouping) -> Self {
        self.grouping = grouping;
        self
    }

    /// The ticks this roll would occupy, before gaps are cut.
    fn ticks(&self) -> Vec<u32> {
        let start = self.start_tick + self.offset_subdivisions * self.subdivision;
        let mut ticks = Vec::new();
        let mut tick = start;
        while tick < self.end_tick {
            ticks.push(tick);
            if let Some(max) = self.max_notes {
                if ticks.len() >= max {
                    break;
                }
            }
            tick += self.subdivision;
        }
        ticks
    }

    /// Render the roll.
    ///
    /// The ramp runs across the notes that are *kept*, so cutting a gap does
    /// not leave a step in the velocity curve.
    pub fn render(&self, rng: &mut impl Rng) -> Vec<Note> {
        let mut ticks = self.ticks();

        if self.gaps && ticks.len() > 3 {
            // One hole, never at either end: a gap on the first note delays the
            // roll and a gap on the last one swallows its arrival.
            let hole = rng.random_range(1..ticks.len() - 1);
            ticks.remove(hole);
        }

        let last = ticks.len().saturating_sub(1).max(1) as f32;
        let (from, to) = (f32::from(self.from_vel), f32::from(self.to_vel));

        ticks
            .into_iter()
            .enumerate()
            .map(|(i, tick)| {
                let ramped = from + (to - from) * (i as f32 / last);
                let vel = (ramped * self.grouping.scale(i)).round().clamp(1.0, 127.0) as u8;
                Note {
                    start_tick: tick,
                    len_ticks: self.subdivision.max(1),
                    pitch: crate::midi::gm_drum_note(self.lane),
                    vel,
                    slide_to_pitch: None,
                    articulation: Some(Articulation::Roll),
                }
            })
            .collect()
    }
}

/// Where the model may put a roll (research ch. 1 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollPosition {
    /// The last beat of a four-bar phrase.
    PhraseEnd,
    /// The beat before a snare hit.
    PreSnare,
    /// The last beat of every fourth bar.
    Bar4,
    /// The last beat of each two-beat group — the busiest option.
    TwoBeatPhraseEnd,
    /// The beat before a downbeat.
    PreDownbeat,
}

impl RollPosition {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "phrase_end" => Some(Self::PhraseEnd),
            "pre_snare" => Some(Self::PreSnare),
            "bar_4" => Some(Self::Bar4),
            "two_beat_phrase_end" => Some(Self::TwoBeatPhraseEnd),
            "pre_downbeat" => Some(Self::PreDownbeat),
            _ => None,
        }
    }

    /// The one-beat windows this position offers in a given bar, as absolute
    /// `(start, end)` ticks. Empty when the position does not apply to this bar.
    fn windows(self, bar: u32, ctx: &SessionContext, snares: &[u32]) -> Vec<(u32, u32)> {
        let beat = grid::ticks_per_beat(ctx);
        let bar_ticks = ctx.ticks_per_bar();
        let bar_start = bar * bar_ticks;
        let beats = u32::from(ctx.time_sig_num.max(1));
        let last_beat = bar_start + bar_ticks - beat;

        match self {
            // A phrase is four bars; its end is the last beat of the fourth.
            Self::PhraseEnd => {
                if (bar + 1).is_multiple_of(4) || u32::from(ctx.bars) == bar + 1 {
                    vec![(last_beat, last_beat + beat)]
                } else {
                    vec![]
                }
            }
            Self::Bar4 => {
                if (bar + 1).is_multiple_of(4) {
                    vec![(last_beat, last_beat + beat)]
                } else {
                    vec![]
                }
            }
            Self::PreDownbeat => vec![(last_beat, last_beat + beat)],
            Self::PreSnare => snares
                .iter()
                .filter(|snare| **snare >= beat)
                .map(|snare| (bar_start + snare - beat, bar_start + snare))
                .collect(),
            Self::TwoBeatPhraseEnd => (0..beats)
                .filter(|b| b % 2 == 1)
                .map(|b| (bar_start + b * beat, bar_start + (b + 1) * beat))
                .collect(),
        }
    }
}

/// Apply the model's hat rolls to a closed-hat stream, in place.
///
/// A roll **replaces** the stream inside its window rather than layering over
/// it: the point of switching to 32nd triplets for a beat is that the beat is
/// now 32nd triplets, and leaving the 16ths underneath would double every hit.
///
/// `snares_by_bar` gives the `pre_snare` position something to be before.
pub fn hat_rolls(
    closed: &mut Vec<Note>,
    hihat: Option<&Value>,
    ctx: &SessionContext,
    snares_by_bar: &[Vec<u32>],
    rng: &mut impl Rng,
) {
    let rolls = hihat.and_then(|h| h.get("rolls"));
    if rolls.is_none() {
        return;
    }

    let vocab: Vec<u32> = strings(rolls, "vocab")
        .iter()
        .filter_map(|v| grid::note_value_ticks(v))
        .collect();
    // `vocab` may also be authored as a weighted spec; the values are what
    // matter here and the weights are handled by sampling the spec itself.
    let vocab = if vocab.is_empty() {
        string_spec(rolls, "vocab", rng)
            .and_then(|value| grid::note_value_ticks(&value))
            .map(|ticks| vec![ticks])
            .unwrap_or_default()
    } else {
        vocab
    };
    if vocab.is_empty() {
        return;
    }

    let positions: Vec<RollPosition> = strings(rolls, "positions")
        .iter()
        .filter_map(|p| RollPosition::parse(p))
        .collect();
    if positions.is_empty() {
        return;
    }

    let frequency = number(rolls, "freqPerBar", 0.4, rng).max(0.0);
    let (from_fraction, to_fraction) = pair(rolls, "rampRange").unwrap_or((0.5, 1.0));
    let ramping = flag(rolls, "velocityRamp", true);
    let gaps = flag(rolls, "insertGaps", false);
    let offset = number(rolls, "clusterOffsetSubdivisions", 0.0, rng).max(0.0) as u32;
    // Drill authors `burstNotes`; rage says `burstOnly`, which is the same
    // instruction without a count — three notes is what the research describes.
    let burst = rolls
        .and_then(|r| r.get("burstNotes"))
        .map(|_| number(rolls, "burstNotes", 3.0, rng).round().max(1.0) as usize)
        .or_else(|| flag(rolls, "burstOnly", false).then_some(3));

    for bar in 0..u32::from(ctx.bars) {
        // A frequency below 1 is a chance; above 1, that many rolls a bar.
        let mut count = frequency.floor() as u32;
        if rng.random_bool((frequency - frequency.floor()).clamp(0.0, 1.0)) {
            count += 1;
        }

        // Hoisted: the bar's snares do not change between the rolls in it, and
        // cloning the Vec per roll was an allocation for a slice read.
        let snares: &[u32] = snares_by_bar
            .get(bar as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for _ in 0..count {
            let position = positions[rng.random_range(0..positions.len())];
            let windows = position.windows(bar, ctx, snares);
            if windows.is_empty() {
                continue;
            }
            let (start, end) = windows[rng.random_range(0..windows.len())];
            let subdivision = vocab[rng.random_range(0..vocab.len())];

            // Ramps run both ways (research ch. 1 §1). A roll that only ever
            // gets louder is half the vocabulary.
            let (from, to) = if !ramping {
                ((to_fraction * 127.0) as u8, (to_fraction * 127.0) as u8)
            } else if rng.random_bool(0.75) {
                ((from_fraction * 127.0) as u8, (to_fraction * 127.0) as u8)
            } else {
                ((to_fraction * 127.0) as u8, (from_fraction * 127.0) as u8)
            };

            let mut roll = Roll::new(Lane::ClosedHat, start, end, subdivision)
                .ramp(from.max(1), to.max(1))
                .with_gaps(gaps)
                .offset(offset);
            if let Some(notes) = burst {
                roll = roll.burst(notes);
            }

            let notes = roll.render(rng);
            if notes.is_empty() {
                continue;
            }
            // The stream inside the window steps aside for the roll.
            closed.retain(|n| !(start..end).contains(&n.start_tick));
            closed.extend(notes);
        }
    }

    closed.sort_by_key(|n| n.start_tick);
}

/// The snare-roll ladder: 1/4 → 1/8 → 1/16 → 1/32 across the window, with the
/// velocity climbing the whole way (research ch. 1 §1, "up-and-down").
///
/// Reverse it by authoring the ladder backwards — the descent is a real device
/// and the model is where that choice belongs.
pub fn snare_ladder(
    block: Option<&Value>,
    ctx: &SessionContext,
    lane: Lane,
    start_tick: u32,
    length: u32,
    rng: &mut impl Rng,
) -> Vec<Note> {
    let steps: Vec<u32> = strings(block, "ladder")
        .iter()
        .filter_map(|v| grid::note_value_ticks(v))
        .collect();
    let steps = if steps.is_empty() {
        vec![960, 480, 240, 120]
    } else {
        steps
    };

    let (from, to) = pair(block, "velocityRampRange")
        .map(|(lo, hi)| (lo as u8, hi as u8))
        .unwrap_or((16, 127));

    let grouping = block
        .and_then(|b| b.get("grouping"))
        .and_then(Value::as_str)
        .and_then(Grouping::parse)
        .unwrap_or(Grouping::Even);

    // Each rung gets an equal slice of the window and its own slice of the ramp,
    // so the climb is continuous across the whole gesture rather than restarting
    // at every subdivision change.
    let slice = length / steps.len().max(1) as u32;
    let span = f32::from(to) - f32::from(from);
    let mut notes = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        let rung_start = start_tick + i as u32 * slice;
        let low = f32::from(from) + span * (i as f32 / steps.len() as f32);
        let high = f32::from(from) + span * ((i + 1) as f32 / steps.len() as f32);
        notes.extend(
            Roll::new(lane, rung_start, rung_start + slice, *step)
                .ramp(low.round().max(1.0) as u8, high.round().max(1.0) as u8)
                .grouped(grouping)
                .render(rng),
        );
    }

    // Build-and-stop: cut the last beat and leave silence into the downbeat.
    // The silence is the gesture — it is what makes the drop land.
    let stop_chance = number(block, "buildAndStopProb", 0.0, rng).clamp(0.0, 1.0);
    if rng.random_bool(stop_chance) {
        let cut = start_tick + length - grid::ticks_per_beat(ctx);
        notes.retain(|n| n.start_tick < cut);
    }

    // The dual-layer roll: a second snare marking the quarters over the top,
    // which is what stops a long roll turning into a texture.
    let dual_chance = number(block, "dualLayerProb", 0.0, rng).clamp(0.0, 1.0);
    if rng.random_bool(dual_chance) {
        let beat = grid::ticks_per_beat(ctx);
        notes.extend(
            Roll::new(lane, start_tick, start_tick + length, beat)
                .ramp(from.max(1), to.max(1))
                .render(rng),
        );
    }

    notes.sort_by_key(|n| n.start_tick);
    notes
}

/// The generic build: an N-bar stream at one subdivision with a linear velocity
/// ramp — the "8-bar riser" (`drums.build`, research ch. 1 cross-genre).
pub fn riser(
    block: Option<&Value>,
    ctx: &SessionContext,
    lane: Lane,
    start_tick: u32,
    rng: &mut impl Rng,
) -> Vec<Note> {
    let bars = number(block, "riserBars", 8.0, rng).round().max(1.0) as u32;
    let subdivision = block
        .and_then(|b| b.get("snareStreamSubdivision"))
        .and_then(Value::as_str)
        .and_then(grid::note_value_ticks)
        .unwrap_or(grid::SIXTEENTH);
    let (from, to) = pair(block, "velocityRampRange")
        .map(|(lo, hi)| (lo as u8, hi as u8))
        .unwrap_or((16, 127));

    let length = bars * ctx.ticks_per_bar();
    Roll::new(lane, start_tick, start_tick + length, subdivision)
        .ramp(from.max(1), to.max(1))
        .render(rng)
}

/// A stutter cluster: a short burst offset off the grid, the jerk device
/// ("clustered snare stutters", research ch. 1 §12 / ch. 4).
pub fn stutter_cluster(
    lane: Lane,
    start_tick: u32,
    subdivision: u32,
    notes: usize,
    rng: &mut impl Rng,
) -> Vec<Note> {
    Roll::new(
        lane,
        start_tick,
        start_tick + subdivision * notes as u32 * 2,
        subdivision,
    )
    .burst(notes)
    .ramp(112, 84)
    .render(rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng as engine_rng;
    use serde_json::json;

    fn rng() -> impl Rng {
        engine_rng::root_stream(7)
    }

    fn ctx(bars: u16) -> SessionContext {
        SessionContext {
            bars,
            ..Default::default()
        }
    }

    #[test]
    fn a_roll_fills_its_window_at_its_subdivision() {
        let notes = Roll::new(Lane::ClosedHat, 0, 960, 240).render(&mut rng());
        let ticks: Vec<u32> = notes.iter().map(|n| n.start_tick).collect();
        assert_eq!(ticks, vec![0, 240, 480, 720]);
        assert!(notes
            .iter()
            .all(|n| n.articulation == Some(Articulation::Roll)));
    }

    #[test]
    fn the_subdivision_ladder_lands_on_whole_ticks() {
        // 16 -> 16T -> 32 -> 32T -> 64, the vocabulary from the research. None
        // of these may be rounded, which is what PPQ 960 buys.
        for (value, step, expected) in [
            ("16", 240, 4),
            ("16T", 160, 6),
            ("32", 120, 8),
            ("32T", 80, 12),
            ("64", 60, 16),
        ] {
            let ticks = grid::note_value_ticks(value).unwrap();
            assert_eq!(ticks, step, "{value}");
            let notes = Roll::new(Lane::ClosedHat, 0, 960, ticks).render(&mut rng());
            assert_eq!(notes.len(), expected, "{value} in one beat");
        }
    }

    #[test]
    fn a_ramp_runs_up_and_down() {
        let up = Roll::new(Lane::Snare, 0, 960, 120)
            .ramp(40, 120)
            .render(&mut rng());
        assert_eq!(up.first().unwrap().vel, 40);
        assert_eq!(up.last().unwrap().vel, 120);

        let down = Roll::new(Lane::Snare, 0, 960, 120)
            .ramp(120, 40)
            .render(&mut rng());
        assert_eq!(down.first().unwrap().vel, 120);
        assert_eq!(down.last().unwrap().vel, 40);
    }

    #[test]
    fn a_burst_stops_after_its_notes_rather_than_filling_the_window() {
        let notes = Roll::new(Lane::ClosedHat, 0, 3840, 120)
            .burst(3)
            .render(&mut rng());
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[2].start_tick, 240);
    }

    #[test]
    fn a_gap_is_cut_from_the_middle_and_never_from_the_ends() {
        for seed in 0..50u64 {
            let mut stream = engine_rng::root_stream(seed);
            let notes = Roll::new(Lane::ClosedHat, 0, 960, 120)
                .with_gaps(true)
                .render(&mut stream);
            assert_eq!(notes.len(), 7, "one hole in eight notes");
            assert_eq!(notes.first().unwrap().start_tick, 0, "seed {seed}");
            assert_eq!(notes.last().unwrap().start_tick, 840, "seed {seed}");
        }
    }

    #[test]
    fn the_ramp_still_reaches_its_ends_after_a_gap_is_cut() {
        // The ramp is computed over the notes that survive, so a hole does not
        // leave a step in the curve or stop the roll arriving at full velocity.
        let notes = Roll::new(Lane::ClosedHat, 0, 960, 120)
            .with_gaps(true)
            .ramp(50, 120)
            .render(&mut rng());
        assert_eq!(notes.first().unwrap().vel, 50);
        assert_eq!(notes.last().unwrap().vel, 120);
    }

    #[test]
    fn a_cluster_offset_starts_the_roll_late() {
        let notes = Roll::new(Lane::ClosedHat, 960, 1920, 120)
            .offset(1)
            .render(&mut rng());
        assert_eq!(notes.first().unwrap().start_tick, 1080);
    }

    #[test]
    fn the_four_note_grouping_puts_a_pulse_inside_the_roll() {
        let notes = Roll::new(Lane::Snare, 0, 960, 120)
            .ramp(100, 100)
            .grouped(Grouping::StrongWeakWeakWeak)
            .render(&mut rng());
        let velocities: Vec<u8> = notes.iter().map(|n| n.vel).collect();
        assert_eq!(velocities.len(), 8);
        for (i, vel) in velocities.iter().enumerate() {
            if i % 4 == 0 {
                assert_eq!(*vel, 100, "note {i} is a strong one");
            } else {
                assert!(*vel < 100, "note {i} should sit back: {velocities:?}");
            }
        }
    }

    #[test]
    fn roll_positions_parse_from_the_names_the_dataset_uses() {
        for name in [
            "phrase_end",
            "pre_snare",
            "bar_4",
            "two_beat_phrase_end",
            "pre_downbeat",
        ] {
            assert!(RollPosition::parse(name).is_some(), "{name}");
        }
        assert!(RollPosition::parse("whenever").is_none());
    }

    #[test]
    fn pre_snare_windows_end_where_the_snare_starts() {
        let context = ctx(2);
        let windows = RollPosition::PreSnare.windows(1, &context, &[1920]);
        // Bar 2 starts at 3840; the snare is on beat 3 of it.
        assert_eq!(windows, vec![(3840 + 960, 3840 + 1920)]);
    }

    #[test]
    fn bar_four_only_offers_a_window_on_the_fourth_bar() {
        let context = ctx(8);
        assert!(RollPosition::Bar4.windows(0, &context, &[]).is_empty());
        assert!(!RollPosition::Bar4.windows(3, &context, &[]).is_empty());
        assert!(!RollPosition::Bar4.windows(7, &context, &[]).is_empty());
    }

    #[test]
    fn a_hat_roll_replaces_the_stream_inside_its_window() {
        // Layering would double every hit in the window; the whole point of
        // switching subdivision is that the beat is now that subdivision.
        let context = ctx(1);
        let mut closed: Vec<Note> = (0..16)
            .map(|i| Note {
                start_tick: i * 240,
                len_ticks: 240,
                pitch: 42,
                vel: 100,
                slide_to_pitch: None,
                articulation: None,
            })
            .collect();

        let hihat = json!({
            "rolls": {
                "vocab": ["32"],
                "positions": ["pre_downbeat"],
                "freqPerBar": 1.0,
                "velocityRamp": true
            }
        });
        hat_rolls(&mut closed, Some(&hihat), &context, &[vec![]], &mut rng());

        // The last beat is now 32nds and nothing from the old stream survives
        // inside it.
        let in_window: Vec<&Note> = closed
            .iter()
            .filter(|n| (2880..3840).contains(&n.start_tick))
            .collect();
        assert_eq!(in_window.len(), 8, "a beat of 32nds");
        assert!(in_window
            .iter()
            .all(|n| n.articulation == Some(Articulation::Roll)));
        // ...and the rest of the bar is untouched.
        assert_eq!(closed.iter().filter(|n| n.start_tick < 2880).count(), 12);
    }

    #[test]
    fn a_model_with_no_rolls_block_gets_no_rolls() {
        let context = ctx(4);
        let mut closed = vec![Note {
            start_tick: 0,
            len_ticks: 240,
            pitch: 42,
            vel: 100,
            slide_to_pitch: None,
            articulation: None,
        }];
        let before = closed.clone();
        hat_rolls(&mut closed, Some(&json!({})), &context, &[], &mut rng());
        assert_eq!(closed, before);
    }

    #[test]
    fn the_snare_ladder_climbs_through_its_rungs() {
        let context = ctx(1);
        let block = json!({
            "ladder": ["4", "8", "16", "32"],
            "velocityRampRange": [1, 127],
            "grouping": "even"
        });
        let notes = snare_ladder(Some(&block), &context, Lane::Snare, 0, 3840, &mut rng());

        // Each rung is denser than the last.
        let rung = |i: u32| {
            notes
                .iter()
                .filter(|n| (i * 960..(i + 1) * 960).contains(&n.start_tick))
                .count()
        };
        assert_eq!((rung(0), rung(1), rung(2), rung(3)), (1, 2, 4, 8));

        // And the velocity climbs across the whole gesture, not per rung.
        assert!(notes.first().unwrap().vel < 20);
        assert!(notes.last().unwrap().vel > 110);
    }

    #[test]
    fn build_and_stop_leaves_silence_into_the_downbeat() {
        let context = ctx(1);
        let block = json!({
            "ladder": ["4", "8", "16", "32"],
            "buildAndStopProb": 1.0
        });
        let notes = snare_ladder(Some(&block), &context, Lane::Snare, 0, 3840, &mut rng());
        assert!(
            notes.iter().all(|n| n.start_tick < 2880),
            "the last beat should be silent"
        );
        assert!(!notes.is_empty());
    }

    #[test]
    fn a_dual_layer_roll_adds_quarters_over_the_ladder() {
        let context = ctx(1);
        let plain = json!({ "ladder": ["16"], "dualLayerProb": 0.0 });
        let dual = json!({ "ladder": ["16"], "dualLayerProb": 1.0 });

        let single = snare_ladder(Some(&plain), &context, Lane::Snare, 0, 3840, &mut rng()).len();
        let layered = snare_ladder(Some(&dual), &context, Lane::Snare, 0, 3840, &mut rng()).len();
        assert_eq!(layered, single + 4, "one accent per quarter");
    }

    #[test]
    fn the_riser_ramps_across_its_whole_length() {
        let context = ctx(8);
        let block = json!({
            "riserBars": 8,
            "snareStreamSubdivision": "16",
            "velocityRampRange": [16, 127]
        });
        let notes = riser(Some(&block), &context, Lane::Snare, 0, &mut rng());

        assert_eq!(notes.len(), 8 * 16, "16ths for eight bars");
        assert_eq!(notes.first().unwrap().vel, 16);
        assert_eq!(notes.last().unwrap().vel, 127);
        // Monotonic: a riser that dips is not a riser.
        for pair in notes.windows(2) {
            assert!(pair[1].vel >= pair[0].vel);
        }
    }

    #[test]
    fn a_stutter_cluster_is_short_and_falls_away() {
        let notes = stutter_cluster(Lane::Snare, 1920, 120, 4, &mut rng());
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].start_tick, 1920);
        assert!(notes.last().unwrap().vel < notes[0].vel);
    }

    #[test]
    fn rolls_are_reproducible_from_a_seed() {
        let context = ctx(4);
        let hihat = json!({
            "rolls": { "vocab": ["16", "32"], "positions": ["phrase_end", "pre_downbeat"],
                       "freqPerBar": 1.0 }
        });
        let run = |seed: u64| {
            let mut closed: Vec<Note> = Vec::new();
            hat_rolls(
                &mut closed,
                Some(&hihat),
                &context,
                &[],
                &mut engine_rng::root_stream(seed),
            );
            closed
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }
}
