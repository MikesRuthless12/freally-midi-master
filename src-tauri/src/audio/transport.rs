//! The sequencer: a pattern becomes a list of sample-exact triggers, and the
//! audio thread walks it.
//!
//! **Where the zero-drift guarantee comes from.** A [`Schedule`] is built once,
//! on the UI thread, with every trigger at an integer sample offset from the
//! start of the loop, and the loop's own length rounded to an integer too. The
//! audio thread never accumulates a remainder: the time of a trigger in cycle
//! *n* is `n * loop_len + offset`, computed fresh. Two minutes in, the kick is
//! exactly where the arithmetic says it is, because nothing has been added up
//! to get there.
//!
//! The alternative — advancing a float position by a float step every block —
//! is what drifts, and it drifts differently at every buffer size, so it is the
//! kind of bug that only shows up on someone else's machine.

use engine::pattern::{Pattern, PPQ};

use super::kit::Kit;

/// One hit, at an exact sample offset from the start of the loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trigger {
    pub at: u64,
    pub pad: usize,
    pub velocity: f32,
    /// Semitones from the pad's root. Always 0 for percussion.
    pub semis: f32,
}

/// A pattern, resolved against a kit and a sample rate.
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    /// Sorted by `at`, so the audio thread only ever looks at the next one.
    pub triggers: Vec<Trigger>,
    /// The full length of the pattern in samples, which is what a loop repeats.
    /// Rounded once, here, so every cycle is the same integer length.
    pub loop_len: u64,
    /// Beats in one cycle, so a sample position can be shown as a bar count
    /// without the frontend having to know the tempo.
    pub beats: f64,
    /// Notes that had no pad in this kit. Surfaced rather than swallowed: a
    /// lane that silently does not play is indistinguishable from one the
    /// generator did not write.
    pub unplaced: usize,
}

/// Resolve a pattern into triggers. Allocates — never call this from the audio
/// thread.
pub fn schedule(pattern: &Pattern, kit: &Kit, sample_rate: f64) -> Schedule {
    let bpm = if pattern.bpm.is_finite() && pattern.bpm > 0.0 {
        f64::from(pattern.bpm)
    } else {
        120.0
    };
    let ppq = if pattern.ppq > 0 { pattern.ppq } else { PPQ };
    let samples_per_tick = sample_rate * 60.0 / bpm / f64::from(ppq);

    let mut triggers = Vec::new();
    let mut unplaced = 0usize;

    for track in &pattern.lanes {
        let Some(pad_index) = kit.pad_for(track.lane) else {
            unplaced += track.notes.len();
            continue;
        };
        let pad = &kit.pads[pad_index];

        for note in &track.notes {
            // A pitched pad plays the note; a drum pad *is* the note, and its
            // pitch field carries the GM number rather than anything to
            // transpose by.
            let semis = match pad.root_note {
                Some(root) => f32::from(note.pitch) - f32::from(root),
                None => 0.0,
            };

            triggers.push(Trigger {
                at: (f64::from(note.start_tick) * samples_per_tick).round() as u64,
                pad: pad_index,
                velocity: f32::from(note.vel.clamp(1, 127)) / 127.0,
                semis,
            });
        }
    }

    triggers.sort_by_key(|t| t.at);

    // The loop is the pattern's written length, not the end of its last note:
    // a bar that ends in silence still takes up its bar, and a clip whose last
    // hat lands on the 15th 16th must not turn over early.
    let total_ticks = u64::from(pattern.bars.max(1))
        * u64::from(pattern.time_sig_num.max(1))
        * u64::from(ppq)
        * 4
        / u64::from(pattern.time_sig_den.max(1));
    let loop_len = (total_ticks as f64 * samples_per_tick).round() as u64;

    Schedule {
        triggers,
        loop_len: loop_len.max(1),
        beats: total_ticks as f64 / f64::from(ppq),
        unplaced,
    }
}

/// Where the playhead is and what it is playing.
pub struct Transport {
    schedule: Option<Box<Schedule>>,
    playing: bool,
    looping: bool,
    /// Samples since play began, across every cycle.
    elapsed: u64,
    /// How many complete cycles have been played.
    cycle: u64,
    /// Index of the next trigger within the current cycle.
    next: usize,
}

impl Default for Transport {
    fn default() -> Self {
        Transport {
            schedule: None,
            playing: false,
            looping: true,
            elapsed: 0,
            cycle: 0,
            next: 0,
        }
    }
}

impl Transport {
    /// Install a schedule and start from the top.
    ///
    /// Returns the schedule it replaced, for the caller to dispose of — never
    /// dropped here, because dropping a `Box` frees memory and this runs on the
    /// audio thread.
    #[must_use = "the old schedule must be retired off the audio thread"]
    pub fn play(&mut self, schedule: Box<Schedule>) -> Option<Box<Schedule>> {
        let previous = self.schedule.replace(schedule);
        self.playing = true;
        self.elapsed = 0;
        self.cycle = 0;
        self.next = 0;
        previous
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.elapsed = 0;
        self.cycle = 0;
        self.next = 0;
    }

    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Samples played since the transport started.
    pub fn elapsed(&self) -> u64 {
        self.elapsed
    }

    /// Position within the current cycle, in samples.
    pub fn position(&self) -> u64 {
        match &self.schedule {
            Some(schedule) if schedule.loop_len > 0 => self.elapsed % schedule.loop_len,
            _ => 0,
        }
    }

    pub fn loop_len(&self) -> u64 {
        self.schedule.as_ref().map_or(0, |s| s.loop_len)
    }

    /// How many frames may be rendered before the next trigger fires, and the
    /// trigger itself once the offset is zero.
    ///
    /// The caller renders in segments so a hit lands on its exact sample rather
    /// than at the start of whichever buffer it fell into. At 256 frames that
    /// is the difference between sample-accurate and up to 5 ms of slop per
    /// note, which on a hat roll is audible as swing that is not there.
    pub fn next_segment(&mut self, remaining: usize) -> Segment {
        if !self.playing || remaining == 0 {
            return Segment {
                frames: remaining,
                trigger: None,
            };
        }
        let Some(schedule) = self.schedule.as_ref() else {
            return Segment {
                frames: remaining,
                trigger: None,
            };
        };

        // Past the end of this cycle: either turn over or stop.
        if self.next >= schedule.triggers.len() {
            let cycle_end = (self.cycle + 1) * schedule.loop_len;
            if self.elapsed >= cycle_end {
                if self.looping {
                    self.cycle += 1;
                    self.next = 0;
                } else {
                    self.playing = false;
                    return Segment {
                        frames: remaining,
                        trigger: None,
                    };
                }
            } else {
                // Silence to the end of the cycle, then re-decide.
                let frames = (cycle_end - self.elapsed).min(remaining as u64) as usize;
                return Segment {
                    frames,
                    trigger: None,
                };
            }
        }

        let trigger = schedule.triggers[self.next];
        let at = self.cycle * schedule.loop_len + trigger.at;

        if at <= self.elapsed {
            self.next += 1;
            return Segment {
                frames: 0,
                trigger: Some(trigger),
            };
        }

        Segment {
            frames: ((at - self.elapsed).min(remaining as u64)) as usize,
            trigger: None,
        }
    }

    /// Advance the playhead after rendering `frames`.
    pub fn advance(&mut self, frames: usize) {
        if self.playing {
            self.elapsed += frames as u64;
        }
    }

    /// Notes that had no pad, from the schedule currently loaded.
    pub fn unplaced(&self) -> usize {
        self.schedule.as_ref().map_or(0, |s| s.unplaced)
    }
}

/// A stretch of output that may be rendered without interruption, and the
/// trigger that fires at its start once `frames` reaches zero.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub frames: usize,
    pub trigger: Option<Trigger>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::kit::Pad;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale};
    use std::sync::Arc;

    fn kit() -> Kit {
        let pad = |lane: Lane, root: Option<u8>| Pad {
            id: format!("{lane:?}"),
            lane,
            samples: Arc::from(vec![1.0f32; 16].into_boxed_slice()),
            sample_rate: 44_100,
            gain: 1.0,
            pan: 0.0,
            pitch_semis: 0,
            choke_group: None,
            root_note: root,
        };
        Kit {
            id: "test".into(),
            pads: vec![
                pad(Lane::Kick, None),
                pad(Lane::Bass808, Some(28)),
                pad(Lane::Snare, None),
            ],
        }
    }

    fn note(tick: u32, pitch: u8) -> Note {
        Note {
            start_tick: tick,
            len_ticks: 240,
            pitch,
            vel: 100,
            slide_to_pitch: None,
            articulation: None,
        }
    }

    fn pattern(lanes: Vec<LaneTrack>, bars: u16, bpm: f32) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "t".into(),
            seed: 1,
            bars,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes,
            ppq: PPQ,
        }
    }

    #[test]
    fn a_bar_at_120_bpm_is_exactly_two_seconds_of_samples() {
        // 120 BPM: a beat is half a second, a 4/4 bar is two.
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(0, 36)],
            }],
            1,
            120.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);
        assert_eq!(s.loop_len, 96_000);
        assert_eq!(s.beats, 4.0, "a 4/4 bar is four beats");
        assert_eq!(s.triggers.len(), 1);
        assert_eq!(s.triggers[0].at, 0);
    }

    #[test]
    fn a_note_lands_on_its_own_sample() {
        // Beat 2 of a 140 BPM bar: 60/140 = 0.428571 s, at 48 kHz = 20571.4
        // samples, which must round rather than truncate toward the beat.
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(PPQ, 36)],
            }],
            1,
            140.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);
        assert_eq!(s.triggers[0].at, 20_571);
    }

    #[test]
    fn a_pitched_lane_is_transposed_from_the_pads_root_and_a_drum_is_not() {
        let p = pattern(
            vec![
                LaneTrack {
                    lane: Lane::Bass808,
                    notes: vec![note(0, 33), note(240, 28)],
                },
                LaneTrack {
                    // A drum's pitch field carries its GM number, which must
                    // not be read as a transposition.
                    lane: Lane::Snare,
                    notes: vec![note(0, 38)],
                },
            ],
            1,
            120.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);

        let semis: Vec<f32> = s
            .triggers
            .iter()
            .filter(|t| t.pad == 1)
            .map(|t| t.semis)
            .collect();
        assert_eq!(semis, vec![5.0, 0.0], "A1 is five semitones above E1");

        let snare = s.triggers.iter().find(|t| t.pad == 2).unwrap();
        assert_eq!(snare.semis, 0.0, "a drum pad plays as it was sampled");
    }

    #[test]
    fn a_lane_with_no_pad_is_counted_rather_than_dropped_quietly() {
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::OpenHat,
                notes: vec![note(0, 46), note(240, 46)],
            }],
            1,
            120.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);
        assert!(s.triggers.is_empty());
        assert_eq!(s.unplaced, 2, "the caller has to be able to see this");
    }

    #[test]
    fn triggers_come_out_in_time_order_across_lanes() {
        // The audio thread only ever looks at the next one, so an unsorted
        // list silently drops everything out of order.
        let p = pattern(
            vec![
                LaneTrack {
                    lane: Lane::Snare,
                    notes: vec![note(PPQ * 2, 38)],
                },
                LaneTrack {
                    lane: Lane::Kick,
                    notes: vec![note(0, 36), note(PPQ * 3, 36)],
                },
            ],
            1,
            120.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);
        assert!(
            s.triggers.windows(2).all(|w| w[0].at <= w[1].at),
            "{:?}",
            s.triggers
        );
    }

    /// Drive the transport for `frames` and collect (absolute sample, pad).
    fn run(transport: &mut Transport, frames: u64, block: usize) -> Vec<(u64, usize)> {
        let mut fired = Vec::new();
        let mut done = 0u64;
        while done < frames {
            let mut left = block.min((frames - done) as usize);
            while left > 0 {
                let segment = transport.next_segment(left);
                if let Some(trigger) = segment.trigger {
                    fired.push((transport.elapsed(), trigger.pad));
                }
                if segment.frames == 0 && segment.trigger.is_none() {
                    break;
                }
                transport.advance(segment.frames);
                left -= segment.frames;
            }
            done += block as u64;
        }
        fired
    }

    #[test]
    fn the_loop_does_not_drift_over_two_minutes() {
        // The roadmap's own acceptance for this task. 140 BPM, one bar, a kick
        // on every beat: after two minutes the kick must be exactly where the
        // arithmetic says, not near it.
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: (0..4).map(|b| note(b * PPQ, 36)).collect(),
            }],
            1,
            140.0,
        );
        let s = schedule(&p, &kit(), 48_000.0);
        let loop_len = s.loop_len;

        let mut transport = Transport::default();
        assert!(transport.play(Box::new(s)).is_none());

        // A buffer size that divides nothing evenly, so a per-block rounding
        // error would show up rather than cancelling out.
        let fired = run(&mut transport, 48_000 * 120, 251);

        // 140 BPM is 140 beats a minute and the kick is on every beat, so two
        // minutes is 280 of them. Exact rather than "roughly": a transport that
        // loses or gains one cycle over two minutes is precisely the failure
        // this test exists for, and a range would let it through.
        assert_eq!(fired.len(), 280, "two minutes at 140 BPM is 280 beats");
        for (index, (at, _)) in fired.iter().enumerate() {
            let cycle = index as u64 / 4;
            let beat = index as u64 % 4;
            let expected = cycle * loop_len
                + (f64::from(beat as u32 * PPQ) * (48_000.0 * 60.0 / 140.0 / f64::from(PPQ)))
                    .round() as u64;
            assert_eq!(
                *at,
                expected,
                "hit {index} drifted by {} samples",
                *at as i64 - expected as i64
            );
        }
    }

    #[test]
    fn a_non_looping_pattern_stops_at_the_end() {
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(0, 36)],
            }],
            1,
            120.0,
        );
        let mut transport = Transport::default();
        transport.set_looping(false);
        let _ = transport.play(Box::new(schedule(&p, &kit(), 48_000.0)));

        let fired = run(&mut transport, 96_000 * 3, 256);
        assert_eq!(fired.len(), 1, "it should play once and stop");
        assert!(!transport.playing());
    }

    #[test]
    fn stopping_puts_the_playhead_back_at_the_top() {
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(0, 36)],
            }],
            2,
            120.0,
        );
        let mut transport = Transport::default();
        let _ = transport.play(Box::new(schedule(&p, &kit(), 48_000.0)));
        let _ = run(&mut transport, 50_000, 256);
        assert!(transport.position() > 0);

        transport.stop();
        assert!(!transport.playing());
        assert_eq!(transport.position(), 0);
    }

    #[test]
    fn replacing_a_schedule_hands_the_old_one_back_to_be_freed() {
        // Dropping it here would free memory on the audio thread.
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(0, 36)],
            }],
            1,
            120.0,
        );
        let mut transport = Transport::default();
        assert!(transport
            .play(Box::new(schedule(&p, &kit(), 48_000.0)))
            .is_none());
        let retired = transport.play(Box::new(schedule(&p, &kit(), 48_000.0)));
        assert!(
            retired.is_some(),
            "the replaced schedule must come back out"
        );
    }

    #[test]
    fn a_pattern_whose_last_note_is_early_still_loops_on_the_barline() {
        // The loop is the written length, not the last note. A one-bar pattern
        // with a single hit on beat 1 must turn over after a whole bar.
        let p = pattern(
            vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![note(0, 36)],
            }],
            1,
            120.0,
        );
        let mut transport = Transport::default();
        let _ = transport.play(Box::new(schedule(&p, &kit(), 48_000.0)));

        let fired = run(&mut transport, 96_000 * 4, 256);
        let times: Vec<u64> = fired.iter().map(|(at, _)| *at).collect();
        assert_eq!(times, vec![0, 96_000, 192_000, 288_000]);
    }
}
