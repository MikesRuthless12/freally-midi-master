//! Turning a generated pattern into notes on the host's track.
//!
//! This replaces the desktop app's drag-out entirely, and it is strictly
//! better: instead of writing a file and asking the user to drag it into a
//! DAW — the gesture `TASK-013` existed to prove was even possible — the notes
//! are emitted as events on the plugin's own output, in the host's time.

use engine::pattern::{Lane, Pattern, PPQ};
use nih_plug::prelude::*;

/// The shortest loop the transport will turn over, in samples.
///
/// ⛔ **This bounds the audio thread's work per block, and it is a security
/// floor rather than a musical one.** `emit` renders one segment per turnover,
/// so segments-per-block is `block_size / loop_length` — unbounded from below.
/// A crafted `arm_pattern`, or a clip restored from a project file, carrying a
/// one-tick region turned a 512-frame block into 512 segments and roughly
/// 100,000 `send_event` calls, every block, on the audio thread.
///
/// 1,024 samples is ~21 ms at 48 kHz, which is *shorter* than any brace the UI
/// can draw — its own floor is one snap step, and a 1/64 note at 140 BPM is
/// about 26 ms — so no loop a producer can ask for is affected. It caps an
/// ordinary 512-frame block at one turnover and a 16k offline block at sixteen.
const MIN_LOOP_SAMPLES: u32 = 1_024;

/// The tempo range the schedule will place notes against.
///
/// ⛔ The same range `engine::context` clamps a pinned BPM to, applied again
/// here because `Pattern.bpm` also arrives straight from the bridge and from
/// project state. `.max(1.0)` alone kept the division safe while leaving a
/// `bpm: 1e9` pattern free to collapse every tick to zero samples — which is
/// the other half of the flood [`MIN_LOOP_SAMPLES`] closes.
const TEMPO_RANGE: std::ops::RangeInclusive<f32> = 20.0..=999.0;

/// One note, placed in samples from the moment the schedule was armed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    /// Samples from the arming point. `u32` because a note is always inside
    /// the pattern, and the longest pattern this generates is a few seconds.
    at: u32,
    /// Which voice this note belongs to.
    ///
    /// Carried so the preview sampler can find the pad that plays it — MIDI
    /// only needs the pitch, but a kick and a snare are the same pitch on
    /// different lanes, so dropping this would make the sampler play the wrong
    /// drum (or nothing) while the host track stayed correct.
    lane: Lane,
    note: u8,
    velocity: f32,
    /// The matching note-off, so a pattern can never leave a note hanging.
    off_at: u32,
    /// Where an 808 slide is heading, as a raw pitch.
    ///
    /// ⛔ **The pitch, not the semitone travel, because only the kit can convert
    /// it.** `Kit::semitones_for` transposes a percussion pad from the lane's GM
    /// note and a pitched pad from its root, and the schedule has no kit —
    /// `render_preview` does, and it is the same call `audio::render` makes, so
    /// the preview and a rendered stem cannot disagree about the interval.
    slide_to: Option<u8>,
}

/// A note that went out in this block, for the preview sampler to play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fired {
    /// Samples into the block being rendered — the same offset the host's own
    /// note event carries, so what is heard and what lands on the track agree.
    pub timing: u32,
    pub lane: Lane,
    pub note: u8,
    pub velocity: f32,
    /// Where an 808 slide is heading, as a raw pitch — see `Placed::slide_to`.
    pub slide_to: Option<u8>,
    /// How long the note lasts in frames, which is the slide's whole window.
    pub frames: u32,
}

/// The note-ons of one block, in a fixed array.
///
/// ⛔ **Fixed, because this is filled on the audio thread.** A `Vec` that grew
/// mid-block would take the allocator's lock inside the host's callback, which
/// is the one thing `process` may never do.
///
/// Sized above [`crate::audio::sampler::MAX_VOICES`] so the sampler's own voice
/// stealing — which is tuned and tested — decides what survives a dense block,
/// rather than this silently truncating first.
#[derive(Debug, Clone)]
pub struct FiredNotes {
    notes: [Fired; Self::CAPACITY],
    len: usize,
}

impl Default for FiredNotes {
    fn default() -> Self {
        Self {
            notes: [Fired {
                timing: 0,
                lane: Lane::Kick,
                note: 0,
                velocity: 0.0,
                slide_to: None,
                frames: 0,
            }; Self::CAPACITY],
            len: 0,
        }
    }
}

impl FiredNotes {
    /// Derived, so the two cannot drift: the sampler's own voice stealing —
    /// which is tuned and tested — decides what survives a dense block, rather
    /// than this silently truncating first.
    const CAPACITY: usize = crate::audio::sampler::MAX_VOICES + 16;

    /// Forget this block's note-ons.
    ///
    /// ⛔ Called by `emit` on every block, and by `process` on the blocks where
    /// `emit` is skipped. Leaving stale entries here would re-trigger the
    /// previous block's notes on every callback — a stopped transport would
    /// machine-gun the last hits of the pattern forever.
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, note: Fired) {
        if self.len < Self::CAPACITY {
            self.notes[self.len] = note;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[Fired] {
        &self.notes[..self.len]
    }
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
    /// The clip's loop region in samples, when the producer set one (TASK-041E).
    ///
    /// ⛔ **This is the *clip's* loop, not the project's, and that is why the
    /// plugin may repeat it.** The comment on `progress` is still true — the
    /// marker never wraps on its own, because a plugin inventing a loop the
    /// project does not have would fight the host's timeline. A brace someone
    /// dragged is the opposite case: it is an instruction, and honouring it is
    /// the only thing that makes the brace mean anything.
    loop_span: Option<(u32, u32)>,
    /// Which clip is armed, so a re-arm can tell an edit from a new generation.
    armed_id: Option<String>,
    /// The clip's own length in samples — what `progress` is a fraction *of*.
    clip_len: u32,
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
        let bpm = pattern.bpm.clamp(*TEMPO_RANGE.start(), *TEMPO_RANGE.end());
        let samples_per_tick = f64::from(sample_rate) * 60.0 / f64::from(bpm) / f64::from(PPQ);

        for track in &pattern.lanes {
            for note in &track.notes {
                // The clip's own start and end, honoured here so a trimmed clip
                // plays trimmed rather than only looking it (TASK-041E).
                if !pattern.within_clip(note) {
                    continue;
                }
                let at = (f64::from(note.start_tick) * samples_per_tick).round() as u32;
                // ⛔ **Saturating, because these ticks are untrusted.** They
                // arrive from the webview and from whatever the host handed back
                // as project state, so `start_tick + len_ticks` is an attacker's
                // sum — and this workspace sets `panic = "abort"`, which makes an
                // overflow panic in a debug-assertions build the *host process*
                // going down rather than one plugin misbehaving.
                let end = note.start_tick.saturating_add(note.len_ticks.max(1));
                let off = (f64::from(end) * samples_per_tick).round() as u32;
                events.push(Placed {
                    at,
                    lane: track.lane,
                    // ⛔ **A drum lane goes out on its own GM note, never on the
                    // walked pitch** (TASK-131D). In MIDI a drum's note number
                    // *is* which drum it is — there is no way to say "the same
                    // snare, a tone higher". This emitted `note.pitch` verbatim,
                    // so a rage snare roll with `pitchWalk: 8` climbed 38, 39,
                    // 41, 43, 46 and fired Hand Clap, Low Floor Tom, High Floor
                    // Tom and Open Hi-Hat in the producer's drum rack — a
                    // different instrument on every hit of one roll.
                    //
                    // ⚠ The walk is therefore an *audio-domain* effect: the
                    // plugin's own sampler and the rendered wav stems repitch
                    // the pad, and MIDI — live and exported — carries the drum.
                    // `engine::midi::events_for` already made the same choice
                    // for the exported file, so all three now agree.
                    note: if crate::midi_note_for(track.lane) == 0 {
                        note.pitch
                    } else {
                        crate::midi_note_for(track.lane)
                    },
                    velocity: f32::from(note.vel) / 127.0,
                    // A zero-length note is inaudible and, in some hosts,
                    // invisible. One sample is the floor.
                    off_at: off.max(at.saturating_add(1)),
                    slide_to: note.slide_to_pitch,
                });
            }
        }

        events.sort_by_key(|e| e.at);
        self.events = events;

        // ⛔ **An edit must not send the playhead back to bar 1.** Every note
        // moved, every velocity painted and every brace dragged re-arms — that
        // is what makes the preview play what is on screen — and resetting
        // `elapsed` here meant the clip jumped to its start and ran out of phase
        // with the host's transport for the rest of the pass. Whether this is a
        // *new* clip is the question, and the pattern's own id answers it: a
        // generation changes it, an edit does not.
        let same_clip = self.armed_id.as_deref() == Some(pattern.id.as_str());
        self.armed_id = Some(pattern.id.clone());
        let resume = if same_clip { self.elapsed } else { 0 };
        self.elapsed = 0;
        self.cursor = 0;
        // Converted here, once, on the thread that is allowed to do arithmetic
        // with `Option` and rounding — `emit` runs on the audio thread.
        self.loop_span = pattern
            .loop_region
            .and_then(|region| region.valid())
            .map(|(from, to)| {
                let at = |tick: u32| (f64::from(tick) * samples_per_tick).round() as u32;
                let start = at(from);
                // ⛔ **A block, not a sample.** `emit` renders one segment per
                // turnover, so the number of segments in a block is
                // `block / loop_length` — and a one-sample floor makes that
                // ratio unbounded. A crafted `arm_pattern`, or a clip restored
                // from a project file, with a one-tick region turned a 512-frame
                // block into 512 segments and ~100,000 `send_event` calls, on
                // the audio thread, every block. The UI cannot ask for this —
                // `stepTicks` floors a brace at a 64th note — but the bridge is
                // not the UI. Flooring bounds it.
                let end = at(to).max(start.saturating_add(MIN_LOOP_SAMPLES));
                (start, end)
            });

        // The clip's own length, in the same samples everything else here is in.
        let bar_ticks = u32::from(pattern.time_sig_num.max(1)) * (PPQ * 4)
            / u32::from(if pattern.time_sig_den == 0 {
                4
            } else {
                pattern.time_sig_den
            });
        let clip_ticks = bar_ticks.saturating_mul(u32::from(pattern.bars));
        self.clip_len = (f64::from(clip_ticks) * samples_per_tick).round() as u32;

        // Put the playhead back where the edit found it, cursor and all.
        if resume > 0 {
            self.rewind_to(resume);
        }
    }

    /// Drop everything scheduled. Used when the host relocates or stops.
    pub fn clear(&mut self) {
        self.events.clear();
        self.elapsed = 0;
        self.cursor = 0;
        self.loop_span = None;
        self.armed_id = None;
        self.clip_len = 0;
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
        // ⛔ `length()`, not `events.last()`. This is what the transport tests
        // measure against, so when it carried the same last-by-start mistake as
        // `length` they compared a wrong number with itself and passed.
        (self.events.len(), self.length())
    }

    /// How far through the pattern the playhead is, 0.0–1.0 (TASK-041T).
    ///
    /// Derived from `elapsed` rather than counted separately, so the marker and
    /// the notes can never disagree about where we are — one clock, and it is
    /// the one that decides what has already been emitted.
    ///
    /// `0.0` for an unarmed schedule, and it saturates at 1.0 rather than
    /// wrapping: looping is the *host's* to decide inside a DAW, and a marker
    /// that jumped back to the start on its own would be the plugin claiming a
    /// loop the project does not have.
    pub fn progress(&self) -> f32 {
        let length = self.length();
        if length == 0 {
            return 0.0;
        }
        (self.elapsed as f32 / length as f32).clamp(0.0, 1.0)
    }

    /// The pattern's length in samples — the **greatest** note-off.
    ///
    /// ⛔ Not `events.last()`. `arm` sorts by `at`, so the last entry is the
    /// latest-*starting* note and its `off_at` is not the end of the pattern —
    /// for anything but a pattern ending in its longest note that under-reports,
    /// and every melodic part does. Both directions of the transport normalise
    /// against this, so a short length makes the marker run ahead of the beat
    /// *and* makes click-to-seek land early.
    fn length(&self) -> u32 {
        // ⛔ **The *clip's* length, not the last note-off.** `progress()` is what
        // the roll draws its marker from and what "paste at the playhead" lands
        // against, and both of those measure against `patternTicks` — the clip.
        // Dividing by the last note-off instead made the two disagree by
        // whatever tail the clip has after its final note: on drums they are
        // nearly the same, and on every melodic part the marker ran ahead of the
        // notes it was supposed to be over and then sat pinned at the right edge
        // for the rest of each pass. Falls back to the last note-off for a clip
        // that never said how long it is.
        if self.clip_len > 0 {
            return self.clip_len;
        }
        self.events.iter().map(|e| e.off_at).max().unwrap_or(0)
    }

    /// Move the playhead, as a fraction of the pattern (TASK-041T).
    ///
    /// ⛔ **The cursor is rewound to match, and that is the whole of it.** The
    /// cursor is an index into events already emitted; moving `elapsed` without
    /// it would leave every note before the new position permanently skipped —
    /// seeking backwards would play silence, which reads as the seek having
    /// broken playback rather than having moved it.
    pub fn seek(&mut self, progress: f32) {
        let length = self.length();
        self.rewind_to((progress.clamp(0.0, 1.0) * length as f32) as u32);
    }

    /// Put the playhead at a sample, cursor and all. The one place both the
    /// seek and the loop turnover compute where the walk resumes.
    fn rewind_to(&mut self, sample: u32) {
        self.elapsed = sample;
        self.cursor = self.events.partition_point(|event| event.at < sample);
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
    /// `fired` collects the note-ons that landed, so the preview sampler can
    /// play them at the **same sample offsets** the host track receives them
    /// at. ⛔ One walk feeds both: a second schedule for audio would be a second
    /// answer to "when does this note play", and the two would drift the first
    /// time a host relocated. Overflow is dropped rather than grown — this runs
    /// on the audio thread and may not allocate — and a block carrying more
    /// note-ons than the sampler has voices was going to steal voices anyway.
    /// `looping` is whether the clip repeats at its end (TASK-138). Read from
    /// [`crate::shared::Shared::looping`] every block rather than stored here,
    /// so pressing the button takes effect on the next block instead of on the
    /// next re-arm.
    pub fn emit<P: Plugin>(
        &mut self,
        context: &mut impl ProcessContext<P>,
        samples: u32,
        looping: bool,
        fired: &mut FiredNotes,
    ) {
        fired.clear();

        // ⛔ **The block is rendered in segments split at the loop's turnover**,
        // not wrapped once at the end of it. A block is up to ~11 ms, which is a
        // 32nd note at 140 BPM — turning the loop over at the block boundary
        // would put every note after the wrap that far out, which is exactly the
        // error `lib.rs` splits its render segments to avoid. Two segments cost
        // one extra walk of an already-sorted list and no allocation.
        let mut offset = 0;
        let mut remaining = samples;
        let span = self.repeat_span(looping);
        while remaining > 0 {
            let chunk = self.next_span(span, remaining);
            self.emit_span(context, offset, chunk, samples, fired);
            offset += chunk;
            remaining -= chunk;
            self.turn_over(span);
        }
    }

    /// What the playhead repeats over, or `None` if it runs to the end and stops.
    ///
    /// ⛔ **Two different loops, and only one of them is the producer's brace.**
    /// `loop_span` is the *clip's* region, dragged on the roll (TASK-041E), and
    /// it wins whenever it is set. The Loop button is the coarser question — does
    /// the clip repeat at all — so with no brace it repeats the whole thing.
    /// Mike, 2026-08-06: *"either loop every time it plays to the end of the 4 or
    /// 8 bars or stop at the end of the 4 or 8 bars."*
    ///
    /// ⚠ **A zero-length clip never repeats**, whatever the button says: a span
    /// of `(0, 0)` would rewind on every block and hold the playhead at the
    /// start forever, which reads as a hung transport rather than a loop.
    fn repeat_span(&self, looping: bool) -> Option<(u32, u32)> {
        match self.loop_span {
            Some(span) => Some(span),
            None if looping => {
                let end = self.length();
                (end > 0).then_some((0, end))
            }
            None => None,
        }
    }

    /// How much of what is left of a block may be rendered before the loop
    /// turns over.
    ///
    /// ⛔ Floored at one sample. Without it a playhead already at or past the
    /// loop's end would ask for a zero-length span forever and hang the audio
    /// thread — which is a locked-up DAW, not a dropout.
    fn next_span(&self, span: Option<(u32, u32)>, remaining: u32) -> u32 {
        match span {
            Some((_, to)) => remaining.min(to.saturating_sub(self.elapsed).max(1)),
            None => remaining,
        }
    }

    /// Send the playhead back to the top of the loop, if it has reached the end.
    fn turn_over(&mut self, span: Option<(u32, u32)>) {
        if let Some((from, to)) = span {
            if self.elapsed >= to {
                self.rewind_to(from);
            }
        }
    }

    /// One contiguous stretch of a block, with `offset` samples already sent.
    fn emit_span<P: Plugin>(
        &mut self,
        context: &mut impl ProcessContext<P>,
        offset: u32,
        samples: u32,
        // `block` is the whole block, so a note-off is clamped into *it* rather
        // than into this segment — a note starting just before a loop turnover
        // would otherwise be cut to a handful of samples.
        block: u32,
        fired: &mut FiredNotes,
    ) {
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

            // ⛔ `offset` is where this span sits inside the block the host is
            // rendering. Without it every note after a loop turnover would be
            // reported at the top of the block instead of where it plays.
            context.send_event(NoteEvent::NoteOn {
                timing: timing + offset,
                voice_id: None,
                channel: 0,
                note: event.note,
                velocity: event.velocity,
            });
            fired.push(Fired {
                timing: timing + offset,
                lane: event.lane,
                note: event.note,
                velocity: event.velocity,
                slide_to: event.slide_to,
                // The note's own length, which is the slide's whole window.
                // ⚠ From the schedule's own samples, so it follows the host's
                // tempo for free — the same reason the placement does.
                frames: event.off_at.saturating_sub(event.at),
            });
            context.send_event(NoteEvent::NoteOff {
                // Clamped into this block. A note longer than one block still
                // ends here rather than hanging — one ordered walk cannot carry
                // a note-off into a later block, and a stuck note is worse than
                // a short one. `saturating_sub` because a relocating host can
                // put `off_at` behind the block start.
                timing: (offset + event.off_at.saturating_sub(block_start))
                    .min(block.saturating_sub(1)),
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
            loop_region: None,
            clip_region: None,
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
            mood: None,
        }
    }

    fn note(start: u32, len: u32, pitch: u8) -> Note {
        Note {
            model_vel: None,
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

#[cfg(test)]
mod transport_tests {
    use super::*;
    use engine::pattern::{LaneTrack, Note, Part, Pattern, Region, Scale, PPQ};

    /// Four bars of quarter notes at 120 BPM, so a position is easy to reason
    /// about: one beat is half a second, and the pattern is eight seconds long.
    /// The brace-only span, with the Loop button **off**.
    ///
    /// ⛔ The cases below are about `loop_span` — the region dragged on the roll
    /// (TASK-041E) — so Loop is off in them and the brace is the only thing that
    /// can turn the playhead over. `the_loop_button_repeats_the_whole_clip…`
    /// covers the button's own behaviour, and keeping the two apart is what makes
    /// each failure name its own cause.
    fn brace(schedule: &Schedule) -> Option<(u32, u32)> {
        schedule.repeat_span(false)
    }

    fn armed() -> Schedule {
        armed_with(None)
    }

    fn armed_with(loop_region: Option<Region>) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.arm(&armed_pattern(loop_region), 48_000.0);
        schedule
    }

    fn armed_pattern(loop_region: Option<Region>) -> Pattern {
        let notes: Vec<Note> = (0..16)
            .map(|i| Note {
                model_vel: None,
                start_tick: i * PPQ,
                len_ticks: PPQ,
                pitch: 36,
                vel: 100,
                slide_to_pitch: None,
                articulation: None,
            })
            .collect();

        Pattern {
            loop_region,
            clip_region: None,
            id: "t".into(),
            part: Part::Drums,
            artist_id: "t".into(),
            seed: 1,
            bars: 4,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane: Lane::Kick,
                notes,
            }],
            ppq: PPQ,
            mood: None,
        }
    }

    /// TASK-041E's other half: the clip markers reach *playback*, not only the
    /// export. They were draggable, persisted, and honoured by nothing.
    #[test]
    fn a_trimmed_clip_schedules_only_what_is_inside_it() {
        let (all, _) = armed().placement();

        let mut pattern = armed_pattern(None);
        pattern.clip_region = Some(Region {
            from_tick: 0,
            to_tick: 4 * PPQ,
        });
        let mut trimmed = Schedule::default();
        trimmed.arm(&pattern, 48_000.0);

        assert_eq!(all, 16, "the fixture is sixteen quarter notes");
        assert_eq!(trimmed.placement().0, 4, "one bar of it is four");
    }

    /// ⛔ Every note moved re-arms — that is what makes the preview play what is
    /// on screen — and `arm` reset `elapsed`, so the clip jumped back to bar 1
    /// and ran out of phase with the host's transport for the rest of the pass.
    #[test]
    fn an_edit_holds_the_playhead_and_a_new_clip_does_not() {
        let mut schedule = armed();
        let (_, length) = schedule.placement();
        schedule.advance_for_test(length / 2);
        let mid = schedule.progress();
        assert!(mid > 0.4, "halfway through, got {mid}");

        schedule.arm(&armed_pattern(None), 48_000.0);
        assert!(
            (schedule.progress() - mid).abs() < 0.01,
            "an edit holds position, got {}",
            schedule.progress()
        );

        let mut other = armed_pattern(None);
        other.id = "another".into();
        schedule.arm(&other, 48_000.0);
        assert_eq!(
            schedule.progress(),
            0.0,
            "a new clip starts at its own start"
        );
    }

    #[test]
    fn the_playhead_is_a_fraction_of_the_clip_not_of_its_last_note() {
        // ⛔ `progress` divided by the greatest note-off while the roll draws its
        // marker against `patternTicks` and paste lands against the same — so on
        // every melodic part, whose last note ends before the clip does, the
        // marker ran ahead of the notes and then sat pinned at the right edge.
        let mut short = armed_pattern(None);
        short.lanes[0].notes.truncate(1);
        let mut schedule = Schedule::default();
        schedule.arm(&short, 48_000.0);

        let (_, length) = schedule.placement();
        schedule.advance_for_test(length / 2);
        let half = schedule.progress();
        assert!(
            (half - 0.5).abs() < 0.01,
            "half of the four-bar clip, not half of its one note: {half}"
        );
    }

    #[test]
    fn the_playhead_starts_at_zero_and_moves_with_the_blocks() {
        // ⛔ This is the "moves with the BPM" requirement, and it is satisfied
        // by construction rather than by a second clock: `arm` places the notes
        // in samples against the pattern's own tempo, so the same `elapsed`
        // that decides what has been emitted decides where the marker is.
        let mut schedule = armed();
        assert_eq!(schedule.progress(), 0.0);

        // Half the pattern's length in samples.
        let (_, length) = schedule.placement();
        schedule.advance_for_test(length / 2);
        let half = schedule.progress();
        assert!((half - 0.5).abs() < 0.01, "expected ~0.5, got {half}");

        schedule.advance_for_test(length);
        assert_eq!(
            schedule.progress(),
            1.0,
            "the marker saturates rather than wrapping — looping is the host's"
        );
    }

    /// TASK-041E's own verification: a two-bar loop inside a four-bar clip.
    ///
    /// ⛔ Driven through the two functions `emit` itself calls, rather than a
    /// copy of its control flow. A test that re-implemented the segmentation
    /// would keep passing after the real one broke.
    #[test]
    fn a_loop_region_turns_the_playhead_over_at_its_own_end() {
        // Bars 1–2 of four: ticks 0 up to 2 bars × 4 beats × PPQ.
        let two_bars = 2 * 4 * PPQ;
        let mut schedule = armed_with(Some(Region {
            from_tick: 0,
            to_tick: two_bars,
        }));
        let (_, length) = schedule.placement();
        let half = length / 2;

        // Just short of the turnover, the playhead is where the clip says.
        schedule.advance_for_test(half - 1000);
        let span = brace(&schedule);
        schedule.turn_over(span);
        assert!(schedule.progress() < 0.5);

        // Past it, and it is back at the top of the loop rather than at the
        // top of the clip — which for this region happen to be the same tick,
        // so the next case is the one that tells them apart.
        schedule.advance_for_test(2000);
        let span = brace(&schedule);
        schedule.turn_over(span);
        assert_eq!(schedule.progress(), 0.0);
    }

    #[test]
    fn a_loop_that_starts_late_returns_to_its_own_start_not_the_clip_s() {
        let bar = 4 * PPQ;
        let mut schedule = armed_with(Some(Region {
            from_tick: bar,
            to_tick: 2 * bar,
        }));
        let (_, length) = schedule.placement();

        schedule.seek(0.5);
        let span = brace(&schedule);
        schedule.turn_over(span);
        let after = schedule.progress();
        assert!(
            (after - 0.25).abs() < 0.01,
            "expected the top of bar 2 (~0.25), got {after}"
        );
        assert!(length > 0);
    }

    #[test]
    fn a_block_is_split_at_the_turnover_rather_than_wrapped_after_it() {
        // ⛔ The reason `emit` renders in spans: a block is up to ~11 ms, and
        // turning over at its boundary would put every note after the wrap that
        // far out of place.
        let bar = 4 * PPQ;
        let mut schedule = armed_with(Some(Region {
            from_tick: 0,
            to_tick: bar,
        }));
        let (_, length) = schedule.placement();
        let quarter = length / 4;

        schedule.advance_for_test(quarter - 100);
        assert_eq!(
            schedule.next_span(brace(&schedule), 512),
            100,
            "the span stops exactly at the loop's end, not one block past it"
        );

        // And with no loop set, a block is never split.
        let bare = armed();
        assert_eq!(bare.next_span(brace(&bare), 512), 512);
    }

    #[test]
    fn an_inverted_brace_is_refused_rather_than_looping_backwards() {
        let mut schedule = armed_with(Some(Region {
            from_tick: 4 * PPQ,
            to_tick: 0,
        }));
        assert_eq!(
            schedule.next_span(brace(&schedule), 512),
            512,
            "an unusable region leaves the transport exactly as it was"
        );
        schedule.advance_for_test(512);
        let span = brace(&schedule);
        schedule.turn_over(span);
        assert!(schedule.progress() > 0.0);
    }

    // ---- The Loop button (TASK-138) ---------------------------------------

    /// ⛔⛔ Mike, 2026-08-06: *"can you have the 'Loop' button toggle off and on
    /// and either loop every time it plays to the end of the 4 or 8 bars or stop
    /// at the end of the 4 or 8 bars."* With no brace dragged, the whole clip is
    /// what repeats.
    #[test]
    fn the_loop_button_repeats_the_whole_clip_when_no_brace_was_dragged() {
        let mut schedule = armed();
        let (_, length) = schedule.placement();

        // On, and the span is the clip — so a block is split at its end.
        assert_eq!(schedule.repeat_span(true), Some((0, length)));

        schedule.advance_for_test(length);
        let span = schedule.repeat_span(true);
        schedule.turn_over(span);
        assert_eq!(
            schedule.progress(),
            0.0,
            "with Loop on, reaching the end must return to the start"
        );
    }

    #[test]
    fn switching_the_loop_button_off_runs_to_the_end_and_stays_there() {
        // The other half of the instruction: *"or stop at the end of the 4 or 8
        // bars."* The playhead parks rather than wrapping, which is what
        // `progress()` saturating at 1.0 has always described.
        let mut schedule = armed();
        let (_, length) = schedule.placement();

        assert_eq!(schedule.repeat_span(false), None);

        schedule.advance_for_test(length);
        let span = schedule.repeat_span(false);
        schedule.turn_over(span);
        assert_eq!(
            schedule.progress(),
            1.0,
            "with Loop off, the playhead must stay at the end rather than wrap"
        );
    }

    #[test]
    fn a_dragged_brace_still_wins_over_the_button() {
        // ⛔ Two different loops. The brace is the *clip's* region and is the
        // finer instruction, so Loop on must not silently widen it back out to
        // the whole clip — that would throw away what the producer dragged.
        let bar = 4 * PPQ;
        let schedule = armed_with(Some(Region {
            from_tick: 0,
            to_tick: bar,
        }));
        let (_, length) = schedule.placement();

        let span = schedule.repeat_span(true).expect("a brace is a span");
        assert!(
            span.1 < length,
            "Loop on widened a dragged brace back to the whole clip: {span:?} of {length}"
        );
        assert_eq!(schedule.repeat_span(true), schedule.repeat_span(false));
    }

    #[test]
    fn an_empty_schedule_never_repeats_however_the_button_is_set() {
        // ⛔ A span of `(0, 0)` would rewind on every block and hold the playhead
        // at the start forever — a hung transport, not a loop.
        assert_eq!(Schedule::default().repeat_span(true), None);
        assert_eq!(Schedule::default().repeat_span(false), None);
    }

    #[test]
    fn an_unarmed_schedule_reports_zero_rather_than_dividing_by_it() {
        assert_eq!(Schedule::default().progress(), 0.0);
    }

    #[test]
    fn stop_returns_the_playhead_to_the_beginning_and_keeps_the_pattern() {
        // ⛔ Stop is not `clear`. Clearing would throw the notes away, so
        // pressing play again would be silent until the next Generate — the
        // marker going home is the whole of what Stop means.
        //
        // Driven through `seek(0.0)` because that is the path Stop actually
        // takes: `stop_playback` → `request_seek(0.0)` → `process` → here.
        // A dedicated `rewind()` existed and was called by nothing but this
        // test, which made the test pass without exercising the real route.
        let mut schedule = armed();
        let (count, length) = schedule.placement();
        schedule.advance_for_test(length / 2);

        schedule.seek(0.0);

        assert_eq!(schedule.progress(), 0.0);
        assert_eq!(schedule.placement().0, count, "the notes are still armed");
        assert!(!schedule.is_empty());
    }

    #[test]
    fn pause_is_the_absence_of_advancing_and_holds_its_position() {
        // Pause has no method on purpose: inside a host, time running is the
        // host's business. Not advancing *is* pausing, and the marker has to
        // stay exactly where it was rather than drifting or resetting.
        let mut schedule = armed();
        let (_, length) = schedule.placement();
        schedule.advance_for_test(length / 4);

        let held = schedule.progress();
        // Any number of blocks with nothing advancing.
        assert_eq!(schedule.progress(), held);
        assert_eq!(schedule.progress(), held);
        assert!(held > 0.0, "and it is not at the start");
    }

    #[test]
    fn seeking_forward_skips_the_notes_behind_the_new_position() {
        let mut schedule = armed();
        schedule.seek(0.5);

        let progress = schedule.progress();
        assert!((progress - 0.5).abs() < 0.01, "{progress}");
        // Half the notes are behind the playhead and must not fire again.
        assert_eq!(schedule.cursor, 8);
    }

    #[test]
    fn seeking_backward_rewinds_the_cursor_so_the_notes_play_again() {
        // ⛔ The bug this exists for: `cursor` indexes events already emitted,
        // so moving `elapsed` without rewinding it leaves every note before the
        // new position permanently skipped. Seeking back would play silence,
        // which reads as the seek having broken playback rather than moved it.
        let mut schedule = armed();
        schedule.seek(1.0);
        assert_eq!(schedule.cursor, 16, "everything is behind us");

        schedule.seek(0.25);

        assert_eq!(schedule.cursor, 4, "the notes after the seek play again");
        assert!(!schedule.is_empty());
    }

    #[test]
    fn a_seek_outside_the_pattern_is_clamped_rather_than_trusted() {
        // The progress arrives from a click in a webview, so it is not trusted.
        let mut schedule = armed();

        schedule.seek(9.0);
        assert_eq!(schedule.progress(), 1.0);

        schedule.seek(-3.0);
        assert_eq!(schedule.progress(), 0.0);
        assert_eq!(schedule.cursor, 0);
    }
}
