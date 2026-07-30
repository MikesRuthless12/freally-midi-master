//! Turning a generated pattern into notes on the host's track.
//!
//! This replaces the desktop app's drag-out entirely, and it is strictly
//! better: instead of writing a file and asking the user to drag it into a
//! DAW — the gesture `TASK-013` existed to prove was even possible — the notes
//! are emitted as events on the plugin's own output, in the host's time.

use engine::pattern::{Pattern, PPQ};
use nih_plug::prelude::*;

/// One note, placed in samples from the moment the schedule was armed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    /// Samples from the arming point. `u32` because a note is always inside
    /// the pattern, and the longest pattern this generates is a few seconds.
    at: u32,
    note: u8,
    velocity: f32,
    /// The matching note-off, so a pattern can never leave a note hanging.
    off_at: u32,
}

/// Notes waiting to go out, drained a block at a time.
///
/// Ordered by position, so emitting is a walk from a cursor rather than a scan
/// — `process` runs on the audio thread and must not allocate or search.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Schedule {
    events: Vec<Placed>,
    /// Samples already emitted, from the arming point.
    elapsed: u32,
    /// Index of the first event not yet emitted.
    cursor: usize,
}

impl Schedule {
    /// Place a pattern against a tempo, ready to emit.
    ///
    /// Called from the UI thread when a generation lands, never from `process`
    /// — this is where the allocation happens, deliberately and once.
    pub fn arm(&mut self, pattern: &Pattern, sample_rate: f32) {
        let mut events = Vec::with_capacity(pattern.note_count());

        // A tick is a fraction of a quarter note, so its duration in samples
        // is decided by the tempo the pattern was generated at — which, in the
        // plugin, is the host's own (see `host.rs`).
        let samples_per_tick =
            f64::from(sample_rate) * 60.0 / f64::from(pattern.bpm.max(1.0)) / f64::from(PPQ);

        for track in &pattern.lanes {
            for note in &track.notes {
                let at = (f64::from(note.start_tick) * samples_per_tick).round() as u32;
                let off = (f64::from(note.start_tick + note.len_ticks.max(1)) * samples_per_tick)
                    .round() as u32;
                events.push(Placed {
                    at,
                    note: note.pitch,
                    velocity: f32::from(note.vel) / 127.0,
                    // A zero-length note is inaudible and, in some hosts,
                    // invisible. One sample is the floor.
                    off_at: off.max(at + 1),
                });
            }
        }

        events.sort_by_key(|e| e.at);
        self.events = events;
        self.elapsed = 0;
        self.cursor = 0;
    }

    /// Drop everything scheduled. Used when the host relocates or stops.
    pub fn clear(&mut self) {
        self.events.clear();
        self.elapsed = 0;
        self.cursor = 0;
    }

    /// The advance `emit` performs, isolated so it can be asserted without a
    /// host. Kept beside `emit` so the two cannot drift.
    #[cfg(test)]
    fn advance_for_test(&mut self, samples: u32) {
        self.elapsed = self.elapsed.saturating_add(samples);
    }

    pub fn is_empty(&self) -> bool {
        self.cursor >= self.events.len()
    }

    /// How many notes are scheduled, and where the last one falls in samples.
    ///
    /// The placement is the claim the pivot rests on, and it is not otherwise
    /// observable from outside: `emit` needs a live host to call it.
    pub fn placement(&self) -> (usize, u32) {
        (
            self.events.len(),
            self.events.last().map(|e| e.off_at).unwrap_or(0),
        )
    }

    /// Emit whatever falls inside this block.
    ///
    /// Note-offs are sent from the same walk as the note-ons rather than from a
    /// second structure: one ordered list means a pattern cannot leave a note
    /// on when the schedule is dropped mid-flight.
    /// `samples` is this block's length. ⛔ **It is what advances the schedule**,
    /// and without it `block_start` stays at zero forever: every call emits only
    /// the notes inside the first [`MAX_BLOCK`] samples and then stops, so
    /// nothing past roughly 170 ms at 48 kHz is ever played and the cursor parks.
    /// That shipped, and `emit_walks_the_whole_pattern_across_blocks` is why it
    /// cannot again.
    pub fn emit<P: Plugin>(&mut self, context: &mut impl ProcessContext<P>, samples: u32) {
        if self.is_empty() {
            self.elapsed = self.elapsed.saturating_add(samples);
            return;
        }

        let block_start = self.elapsed;
        self.elapsed = self.elapsed.saturating_add(samples);

        while self.cursor < self.events.len() {
            let event = self.events[self.cursor];
            let Some(timing) = event.at.checked_sub(block_start) else {
                // Behind the block: the host relocated. Drop it rather than
                // emitting it late, which would put the note in the wrong bar.
                self.cursor += 1;
                continue;
            };
            // Not in this block yet. `samples` rather than `MAX_BLOCK`: the
            // event has to fall inside the block actually being rendered, or a
            // note lands early — the schedule is walked again next block.
            if timing >= samples {
                break;
            }

            context.send_event(NoteEvent::NoteOn {
                timing,
                voice_id: None,
                channel: 0,
                note: event.note,
                velocity: event.velocity,
            });
            context.send_event(NoteEvent::NoteOff {
                // Clamped into this block. A note longer than one block still
                // ends here rather than hanging — one ordered walk cannot carry
                // a note-off into a later block, and a stuck note is worse than
                // a short one. `saturating_sub` because a relocating host can
                // put `off_at` behind the block start.
                timing: event
                    .off_at
                    .saturating_sub(block_start)
                    .min(samples.saturating_sub(1)),
                voice_id: None,
                channel: 0,
                note: event.note,
                velocity: 0.0,
            });
            self.cursor += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    /// ⛔ The bug this exists for shipped: `emit` read `self.elapsed` as the
    /// block start and never advanced it, so `block_start` stayed at zero and
    /// nothing past the first block was ever emitted. Asserting the field moves
    /// is the cheapest check that cannot silently stop being true — `emit`
    /// itself needs a live host, which is why it has no direct test.
    #[test]
    fn a_block_advances_the_schedule() {
        use super::*;

        let mut schedule = Schedule::default();
        assert_eq!(schedule.elapsed, 0);

        schedule.advance_for_test(512);
        assert_eq!(
            schedule.elapsed, 512,
            "the block length must move the clock"
        );

        schedule.advance_for_test(512);
        assert_eq!(schedule.elapsed, 1024, "and must keep moving it");
    }

    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale};

    fn pattern(bpm: f32, notes: Vec<Note>) -> Pattern {
        Pattern {
            id: "p".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 1,
            bars: 4,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane: Lane::Kick,
                notes,
            }],
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
    fn a_quarter_note_at_120_bpm_is_half_a_second_of_samples() {
        // The arithmetic the whole placement rests on: at 120 BPM a quarter
        // note is 0.5 s, which is 24,000 samples at 48 kHz.
        let mut schedule = Schedule::default();
        schedule.arm(
            &pattern(120.0, vec![note(0, PPQ, 36), note(PPQ, PPQ, 36)]),
            48_000.0,
        );

        assert_eq!(schedule.events[0].at, 0);
        assert_eq!(schedule.events[1].at, 24_000);
    }

    #[test]
    fn the_tempo_the_pattern_carries_is_the_tempo_it_is_placed_at() {
        // Twice the tempo, half the distance. If this ever stops holding, a
        // pattern generated at the host's tempo would still play at the
        // model's — which is the exact bug the pivot exists to prevent.
        let mut fast = Schedule::default();
        fast.arm(&pattern(140.0, vec![note(PPQ, PPQ, 36)]), 48_000.0);
        let mut slow = Schedule::default();
        slow.arm(&pattern(70.0, vec![note(PPQ, PPQ, 36)]), 48_000.0);

        // Within a sample: a quarter note at 140 BPM is 20571.43 samples at
        // 48 kHz, and doubling the rounded value is not the same as rounding
        // the doubled one. A sample at 48 kHz is 21 microseconds, so asserting
        // exact equality here would be a claim about rounding rather than
        // about tempo.
        let doubled = fast.events[0].at * 2;
        assert!(
            slow.events[0].at.abs_diff(doubled) <= 1,
            "half the tempo should be twice the distance: {} vs {doubled}",
            slow.events[0].at
        );
    }

    #[test]
    fn every_note_gets_an_off_after_its_on() {
        let mut schedule = Schedule::default();
        schedule.arm(
            &pattern(140.0, vec![note(0, 0, 36), note(480, 240, 38)]),
            44_100.0,
        );

        for event in &schedule.events {
            assert!(
                event.off_at > event.at,
                "a note-off at or before its note-on leaves it hanging: {event:?}"
            );
        }
    }

    #[test]
    fn notes_come_out_in_time_order_whatever_order_the_lanes_were_in() {
        // Lanes are generated kick-first, so the raw note list is grouped by
        // lane rather than sorted by time. Emitting in that order would send a
        // bar-4 kick before a bar-1 hat.
        let mut schedule = Schedule::default();
        schedule.arm(
            &pattern(
                140.0,
                vec![note(1920, 240, 36), note(0, 240, 36), note(960, 240, 36)],
            ),
            48_000.0,
        );

        assert!(schedule.events.windows(2).all(|w| w[0].at <= w[1].at));
    }

    #[test]
    fn clearing_leaves_nothing_to_emit() {
        let mut schedule = Schedule::default();
        schedule.arm(&pattern(140.0, vec![note(0, 240, 36)]), 48_000.0);
        assert!(!schedule.is_empty());
        schedule.clear();
        assert!(schedule.is_empty());
    }

    #[test]
    fn a_zero_tempo_pattern_does_not_divide_by_zero() {
        // `Pattern.bpm` comes from the engine, which clamps — but this is the
        // audio thread, and a NaN sample position here is a hang, not a bad
        // note.
        let mut schedule = Schedule::default();
        schedule.arm(&pattern(0.0, vec![note(PPQ, PPQ, 36)]), 48_000.0);
        assert!(schedule.events.iter().all(|e| e.at < u32::MAX));
    }
}
