//! Auditioning a sample from the File Explorer (TASK-132).
//!
//! Mike, 2026-08-07: *"ensure that the preview player shows the waveform of the
//! current sample it is playing with a play/pause & stop buttons, and with the
//! ability to click in the sample and play from anywhere, along with it being
//! able to play backwards … along with the playback time out of the total time
//! … stop button goes back to the beginning of the sample, also ensure that
//! there is a loop toggle … ensure that there is a playhead marker."*
//!
//! ## ⛔ Six of those eight are one number
//!
//! The playhead marker, the progress fill, the time readout, click-to-seek,
//! reverse and loop are all **the read position**. Publishing that one value the
//! way `Shared::playhead` already publishes the pattern's is what makes the
//! panel possible; everything else the page draws follows from it. Building six
//! separate channels would have been six things to keep in step.
//!
//! ## The audio-thread rules, which are the same absolute ones
//!
//! [`Preview::render`] runs on the host's callback: **no allocation, no
//! locking, no I/O**. The samples are decoded on the editor thread and handed
//! over as an `Arc` through the same one-slot handoff
//! [`crate::shared::KitHandoff`] uses, and the replaced buffer is **parked**
//! rather than dropped — freeing a `Vec` on the audio thread takes the
//! allocator's lock.
//!
//! Everything the page can change is an atomic, so pressing Play never waits on
//! a callback and the callback never waits on the page.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Nothing pending. A real seek is a frame index, which is never negative.
const NO_SEEK: i64 = -1;

/// What the page draws, polled while a sample is auditioning.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// Whether it is sounding right now.
    pub playing: bool,
    /// Where the playhead is, in seconds.
    pub seconds: f32,
    /// How long the whole sample is, in seconds.
    pub total: f32,
    pub looping: bool,
    pub reverse: bool,
}

/// The audition voice: one sample, one position.
///
/// ⚠ **One at a time, deliberately.** A browser previews the file you clicked;
/// two samples over each other is not a preview, it is a mix nobody asked for.
#[derive(Debug, Default)]
pub struct Preview {
    /// Handed from the editor thread. `None` until something is loaded.
    incoming: Mutex<Option<Arc<Vec<f32>>>>,
    /// What `render` swapped out, waiting for a thread allowed to free it.
    spent: Mutex<Option<Arc<Vec<f32>>>>,
    /// The buffer the audio thread is reading. Audio thread only.
    loaded: Mutex<Option<Arc<Vec<f32>>>>,

    /// The read position in frames, as f64 bits.
    ///
    /// ⚠ **Fractional, because the step is a rate ratio rather than one frame
    /// per frame** — a 44.1 kHz sample in a 48 kHz session advances 0.919 of a
    /// frame each time. Stored as bits because there is no `AtomicF64`; the
    /// audio thread writes it, the page reads it, relaxed on both sides — a
    /// playhead one block stale is invisible and ordering it would put a fence
    /// in the callback for nothing.
    position: AtomicU64,
    /// A position the page asked for, or [`NO_SEEK`].
    seek: AtomicI64,
    frames: AtomicU32,
    /// The sample's own rate, so the position can be reported in seconds.
    rate: AtomicU32,
    playing: AtomicBool,
    /// A buffer is waiting in the handoff. Lets is_active answer without
    /// taking a lock on the audio thread.
    pending: AtomicBool,
    looping: AtomicBool,
    reverse: AtomicBool,
}

impl Preview {
    /// Hand a decoded sample over and start from the top. Editor thread only.
    pub fn load(&self, samples: Vec<f32>, rate: u32) {
        self.frames.store(samples.len() as u32, Ordering::Relaxed);
        self.rate.store(rate.max(1), Ordering::Relaxed);
        self.position.store(0f64.to_bits(), Ordering::Relaxed);
        self.seek.store(NO_SEEK, Ordering::Relaxed);
        // ⚠ Not started here. Loading a sample because the producer clicked its
        // row must not make a noise — clicking a name is browsing, pressing
        // Play is auditioning.
        self.playing.store(false, Ordering::Relaxed);
        // ⛔ Drained HERE, on the editor thread, before parking the new one.
        // The swap in render only proceeds when the park slot is empty, and
        // collect had exactly one caller — the preview_position poll. So after
        // two loads without a poll the slot stayed full and every later sample
        // was silently ignored: the audio thread went on playing the previous
        // buffer while frames/rate/total already described the new one.
        self.collect();
        if let Ok(mut slot) = self.incoming.lock() {
            *slot = Some(Arc::new(samples));
        }
        self.pending.store(true, Ordering::Relaxed);
    }

    /// Start, rewinding first if the playhead has nowhere left to travel.
    ///
    /// ⛔ **Two ways to get one frame of sound and then silence, and both read
    /// as "the player is broken".**
    /// - `load` parks at 0; arming reverse does not move it; so a freshly
    ///   selected sample played backwards rendered frame 0, stepped below zero
    ///   and stopped. That is the *common* case — Mike's ask is "play backwards
    ///   if you have the file selected and press left arrow".
    /// - Forward, a sample that ran out is parked at the last frame on purpose
    ///   (so the producer can see it finished), and pressing Play again did the
    ///   same thing at the other end.
    ///
    /// So Play rewinds when it is already at the end of travel for the
    /// direction it is about to go — which is what the button means.
    pub fn play(&self) {
        let frames = self.frames.load(Ordering::Relaxed);
        if frames == 0 {
            return;
        }
        let last = f64::from(frames) - 1.0;
        let at = f64::from_bits(self.position.load(Ordering::Relaxed));
        let spent = if self.reverse.load(Ordering::Relaxed) {
            at <= 0.0
        } else {
            at >= last
        };
        if spent {
            let start = if self.reverse.load(Ordering::Relaxed) {
                last
            } else {
                0.0
            };
            // Through the seek request, not the cursor — see `seek`.
            self.seek.store(start as i64, Ordering::Relaxed);
        }
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    /// Stop, and rewind. ⛔ Mike asked for exactly this: *"stop button goes back
    /// to the beginning of the sample"* — pause holds position, stop does not.
    ///
    /// ⚠ Rewinds to the **end** when reverse is armed, because that is where a
    /// backwards pass begins. Sending it to frame 0 would leave Stop followed
    /// by Play doing nothing at all.
    /// ⚠ **The cursor is written directly here, and that is safe only because
    /// `playing` is cleared first.** `seek` deliberately goes through a request
    /// atomic because the audio thread owns `position` while it renders — it
    /// reads it at the top of a block and stores it back at the bottom, so a
    /// concurrent write is overwritten. Clearing `playing` before the write
    /// means the *next* block returns early without touching the cursor, and
    /// the only block that can still overwrite is the one already in flight.
    ///
    /// ⛔ That one block is a real race: Stop pressed mid-block left the
    /// playhead where it was instead of at the start, and no later block
    /// corrected it because playback had stopped. So the request atomic is set
    /// as well — a block in flight consumes it, and one that is not leaves it
    /// for `play` to apply.
    pub fn stop(&self) {
        self.playing.store(false, Ordering::Relaxed);
        let start = if self.reverse.load(Ordering::Relaxed) {
            (f64::from(self.frames.load(Ordering::Relaxed)) - 1.0).max(0.0)
        } else {
            0.0
        };
        self.position.store(start.to_bits(), Ordering::Relaxed);
        self.seek.store(start as i64, Ordering::Relaxed);
    }

    /// Play from `at` seconds — a click in the waveform.
    pub fn seek(&self, at: f32) {
        let rate = self.rate.load(Ordering::Relaxed).max(1);
        let frames = i64::from(self.frames.load(Ordering::Relaxed));
        let frame = ((at.max(0.0) * rate as f32) as i64).clamp(0, frames.max(1) - 1);
        // ⛔ Published as a *request* rather than written into `position`
        // directly. The audio thread owns that field while it is rendering, and
        // two writers to one cursor is a click at best and a skipped block at
        // worst.
        self.seek.store(frame, Ordering::Relaxed);

        // ⛔⛔ **...but a seek while PAUSED has to move the readout too, and
        // this is the half that was missing.** `render` returns early when
        // nothing is playing, so the request just sat there and `position` went
        // on reporting the old cursor. The page writes the new position
        // optimistically when the producer clicks — and the very next
        // `preview_position` poll, at most half a second later, overwrote it
        // with the stale one. Clicking a paused waveform moved the playhead and
        // then visibly undid itself.
        //
        // ⚠ **Safe for exactly the reason `stop` gives**: a block that is not
        // playing returns without touching `position`, so there is no second
        // writer. If Play is pressed concurrently the worst case is that the
        // one block already in flight overwrites this — and it consumed the same
        // seek request on its way, so it lands in the same place regardless.
        if !self.playing.load(Ordering::Relaxed) {
            self.position
                .store((frame as f64).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn set_looping(&self, on: bool) {
        self.looping.store(on, Ordering::Relaxed);
    }

    /// Play backwards. Mike: *"able to play backwards if you have the file
    /// selected and press left arrow"*.
    pub fn set_reverse(&self, on: bool) {
        self.reverse.store(on, Ordering::Relaxed);
    }

    /// What the page polls.
    pub fn position(&self) -> Position {
        let rate = self.rate.load(Ordering::Relaxed).max(1) as f32;
        let frames = self.frames.load(Ordering::Relaxed) as f32;
        Position {
            playing: self.playing.load(Ordering::Relaxed),
            seconds: (f64::from_bits(self.position.load(Ordering::Relaxed)).max(0.0) as f32) / rate,
            total: frames / rate,
            looping: self.looping.load(Ordering::Relaxed),
            reverse: self.reverse.load(Ordering::Relaxed),
        }
    }

    /// Is there anything for [`Self::render`] to do?
    ///
    /// ⚠ **Cheap enough to ask on every block**: two relaxed loads, and it is
    /// what lets `process` skip the whole preview path when nothing is
    /// auditioning while still reaching it when something is. `pending` matters
    /// as much as `playing` — a sample handed over but not yet picked up still
    /// needs a block to complete the swap.
    pub fn is_active(&self) -> bool {
        self.playing.load(Ordering::Relaxed) || self.pending.load(Ordering::Relaxed)
    }

    /// Free anything the audio thread parked. Editor thread only.
    pub fn collect(&self) {
        if let Ok(mut spent) = self.spent.lock() {
            spent.take();
        }
    }

    /// Mix the audition into `out`. **Audio thread.**
    ///
    /// `out` is interleaved with `channels` per frame, and this *adds* rather
    /// than overwrites — the preview sits alongside the pattern rather than
    /// replacing it.
    ///
    /// `device_rate` is the host's rate; the sample carries its own, and the
    /// ratio is what stops a 44.1 kHz one-shot playing sharp in a 48 kHz
    /// session.
    pub fn render(&self, out: &mut [f32], channels: usize, device_rate: f32) {
        // ⛔ **The park slot is claimed FIRST, and only an empty one accepts a
        // swap.** Assigning over an occupied slot drops the buffer it held —
        // a real `free()` of megabytes, on the audio callback, which is exactly
        // what this module's header promises never happens. `collect()` runs
        // only from the `preview_position` poll, so a page that stopped polling
        // (panel closed, webview reloaded) leaves the slot full and the *next*
        // swap freeing on the callback.
        //
        // ⚠ `try_lock` everywhere: an editor thread mid-write must never stall
        // the callback. A missed swap costs one block before the new sample
        // starts, which nobody can hear.
        if let Ok(mut spent) = self.spent.try_lock() {
            if spent.is_none() {
                if let Ok(mut incoming) = self.incoming.try_lock() {
                    if let Some(next) = incoming.take() {
                        drop(incoming);
                        if let Ok(mut loaded) = self.loaded.try_lock() {
                            *spent = loaded.replace(next);
                            self.pending.store(false, Ordering::Relaxed);
                        } else {
                            // ⛔ **The `else` is not optional**, and
                            // `KitHandoff` records the same rule: dropping
                            // `next` here would free on the callback. Leaked
                            // instead — one buffer, once, on a race that needs
                            // a lock nothing else takes.
                            std::mem::forget(next);
                        }
                    }
                }
            }
        }

        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        let Ok(loaded) = self.loaded.try_lock() else {
            return;
        };
        let Some(samples) = loaded.as_ref() else {
            return;
        };
        if samples.is_empty() || channels == 0 {
            return;
        }

        let last = samples.len() as f64 - 1.0;
        let mut at = f64::from_bits(self.position.load(Ordering::Relaxed));
        let pending = self.seek.swap(NO_SEEK, Ordering::Relaxed);
        if pending != NO_SEEK {
            at = (pending as f64).clamp(0.0, last);
        }

        // ⛔ **The sample's rate against the device's, not one frame per
        // frame.** `at += 1` reads the buffer at the host's rate whatever the
        // file was recorded at, so a 44.1 kHz one-shot auditioned in a 48 kHz
        // session played about 8.8% sharp — an audible semitone and a half.
        // `sampler.rs` has computed this ratio for every pad since it was
        // written; the audition had simply skipped it.
        let rate = self.rate.load(Ordering::Relaxed).max(1) as f64;
        let ratio = if device_rate > 0.0 {
            rate / f64::from(device_rate)
        } else {
            1.0
        };
        let step = if self.reverse.load(Ordering::Relaxed) {
            -ratio
        } else {
            ratio
        };
        let looping = self.looping.load(Ordering::Relaxed);

        for frame in out.chunks_mut(channels) {
            if at < 0.0 || at > last {
                if looping {
                    // Wrapped rather than freed, which is the whole of "loop".
                    at = if step > 0.0 { 0.0 } else { last };
                } else {
                    self.playing.store(false, Ordering::Relaxed);
                    // ⛔ Parked at the end it reached, not at 0. Stop is what
                    // rewinds; a sample that ran out should leave the playhead
                    // where the producer can see it finished.
                    at = at.clamp(0.0, last);
                    break;
                }
            }
            // ⚠ Attenuated: an audition plays over whatever the pattern is
            // doing, and a full-scale one-shot on top of a full-scale mix
            // clips. `limit` catches it either way, but limiting is not a
            // level.
            //
            // ⚠ Linear interpolation, because `at` is fractional now. Reading
            // the floor alone is a zero-order hold, which is the aliasing
            // whine a resampled preview is known for.
            let index = at as usize;
            let frac = at - at.floor();
            let a = samples[index.min(samples.len() - 1)];
            let b = samples[(index + 1).min(samples.len() - 1)];
            let value = (a + (b - a) * frac as f32) * 0.7;
            for sample in frame.iter_mut() {
                *sample += value;
            }
            at += step;
        }

        self.position
            .store(at.clamp(0.0, last).to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The device rate the tests render at.
    ///
    /// ⚠ **Equal to the fixtures own rate**, so one output frame is one sample
    /// frame and the position assertions stay readable. The ratio itself is
    /// covered by its own test rather than smeared through every other one.
    const RATE: f32 = 100.0;

    /// A ramp, so a test can tell which frame it is hearing.
    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 / n as f32).collect()
    }

    fn preview(n: usize) -> Preview {
        let p = Preview::default();
        p.load(ramp(n), 100);
        // The first render picks the handoff up.
        let mut out = vec![0.0; 0];
        p.render(&mut out, 1, RATE);
        p
    }

    #[test]
    fn loading_a_sample_does_not_start_it() {
        // Clicking a row is browsing; pressing Play is auditioning. A browser
        // that shouts the moment you touch a name is one nobody scrolls.
        let p = preview(10);
        assert!(!p.position().playing);

        let mut out = vec![0.0; 4];
        p.render(&mut out, 1, RATE);
        assert!(out.iter().all(|s| *s == 0.0), "silent until asked");
    }

    #[test]
    fn play_advances_and_the_position_is_readable() {
        let p = preview(100);
        p.play();
        let mut out = vec![0.0; 10];
        p.render(&mut out, 1, RATE);

        let at = p.position();
        assert!(at.playing);
        assert!(out.iter().any(|s| *s != 0.0), "it sounded");
        // 10 frames at 100 Hz is a tenth of a second.
        assert!((at.seconds - 0.1).abs() < 0.02, "{}", at.seconds);
        assert!((at.total - 1.0).abs() < 0.01, "{}", at.total);
    }

    #[test]
    fn stop_rewinds_and_pause_does_not() {
        // ⛔ Mike's own distinction: "stop button goes back to the beginning".
        let p = preview(100);
        p.play();
        p.render(&mut [0.0; 20], 1, RATE);
        assert!(p.position().seconds > 0.0);

        p.pause();
        let held = p.position();
        assert!(!held.playing);
        assert!(held.seconds > 0.0, "pause holds its place");

        p.stop();
        let stopped = p.position();
        assert!(!stopped.playing);
        assert_eq!(stopped.seconds, 0.0, "stop rewinds");
    }

    #[test]
    fn a_click_in_the_waveform_plays_from_there() {
        let p = preview(100);
        p.seek(0.5);
        p.play();
        p.render(&mut [0.0; 2], 1, RATE);
        // Started at frame 50, advanced 2.
        assert!(
            (p.position().seconds - 0.52).abs() < 0.02,
            "{:?}",
            p.position()
        );
    }

    #[test]
    fn reverse_walks_backwards_and_stop_rewinds_to_the_end() {
        // ⛔ The end is where a backwards pass *begins*. Sending Stop to frame 0
        // would leave Stop-then-Play doing nothing at all.
        let p = preview(100);
        p.set_reverse(true);
        p.stop();
        let start = p.position();
        assert!(start.reverse);
        assert!((start.seconds - 0.99).abs() < 0.02, "{:?}", start);

        p.play();
        p.render(&mut [0.0; 10], 1, RATE);
        assert!(
            p.position().seconds < start.seconds,
            "it must move backwards"
        );
    }

    #[test]
    fn the_end_stops_it_unless_it_is_looping() {
        let p = preview(10);
        p.play();
        // Far more frames than the sample has.
        p.render(&mut [0.0; 50], 1, RATE);
        assert!(!p.position().playing, "it ran out and stopped");

        p.set_looping(true);
        p.stop();
        p.play();
        p.render(&mut [0.0; 50], 1, RATE);
        assert!(p.position().playing, "looping does not run out");
    }

    #[test]
    fn a_stereo_buffer_gets_the_sample_in_both_channels() {
        let p = preview(100);
        p.play();
        let mut out = vec![0.0; 20];
        p.render(&mut out, 2, RATE);
        for frame in out.chunks(2) {
            assert_eq!(frame[0], frame[1], "a mono preview is centred");
        }
    }

    #[test]
    fn rendering_adds_rather_than_replacing() {
        // The audition sits alongside the pattern; overwriting would mute the
        // beat every time somebody previewed a snare.
        let p = preview(100);
        p.play();
        let mut out = vec![0.5; 10];
        p.render(&mut out, 1, RATE);
        assert!(out.iter().all(|s| *s >= 0.5), "the pattern survived");
    }

    #[test]
    fn a_sample_plays_at_its_own_rate_rather_than_the_devices() {
        // ⛔ **The audition advanced one frame per output frame**, whatever the
        // file was recorded at — so a 44.1 kHz one-shot in a 48 kHz session
        // played about 8.8% sharp, a semitone and a half. `sampler.rs` has
        // computed this ratio for every pad since it was written; this path had
        // simply skipped it.
        let p = preview(1_000);
        p.play();

        // 100 output frames at twice the sample's rate should consume ~50
        // frames of the sample, not 100.
        p.render(&mut [0.0; 100], 1, RATE * 2.0);
        let half = p.position().seconds;
        assert!(
            (half - 0.5).abs() < 0.05,
            "at double the device rate the sample should advance half as fast, got {half}"
        );

        // And the other direction.
        p.stop();
        p.play();
        p.render(&mut [0.0; 100], 1, RATE / 2.0);
        let double = p.position().seconds;
        assert!(
            (double - 2.0).abs() < 0.05,
            "at half the device rate it should advance twice as fast, got {double}"
        );
    }

    #[test]
    fn a_device_rate_of_zero_does_not_divide_by_it() {
        // Hosts do report nonsense during teardown; a NaN position would poison
        // every subsequent block.
        let p = preview(100);
        p.play();
        p.render(&mut [0.0; 10], 1, 0.0);
        assert!(
            p.position().seconds.is_finite(),
            "a zero device rate must not produce NaN"
        );
    }

    // ───────────────────────────────────────────────────────────────────────
    // What is actually heard.
    //
    // ⛔ **Everything above asserts a *position*, and a position is not a
    // sound.** A render that advanced the cursor and wrote silence, or wrote
    // the right samples in the wrong order, passes every one of them. These
    // assert the buffer — the ramp fixture is `i / n`, so a sample value names
    // the frame it came from, and the whole point of a ramp is that a reversed
    // pass is visibly the mirror of a forward one.
    // ───────────────────────────────────────────────────────────────────────

    /// The frame each output sample came from, given the `0.7` attenuation and
    /// the `i / n` ramp. Rounded, because interpolation lands between frames.
    fn frames_heard(out: &[f32], n: usize) -> Vec<i64> {
        out.iter()
            .map(|s| ((s / 0.7) * n as f32).round() as i64)
            .collect()
    }

    #[test]
    fn playing_forward_reads_the_sample_from_the_front() {
        let p = preview(100);
        p.play();
        let mut out = [0.0; 8];
        p.render(&mut out, 1, RATE);

        assert_eq!(
            frames_heard(&out, 100),
            vec![0, 1, 2, 3, 4, 5, 6, 7],
            "forward playback must walk the sample in order"
        );
    }

    #[test]
    fn playing_backwards_reads_the_same_sample_in_reverse() {
        // ⛔ **The claim Mike's "play backwards" actually makes**, and the one
        // nothing checked: the position going down is not the same thing as the
        // audio coming out reversed. A render that decremented the cursor and
        // read `samples[index]` with a stale `frac` would pass the position
        // test above and produce a stuttering mess.
        let p = preview(100);
        p.set_reverse(true);
        p.play(); // rewinds to the last frame, because that is where a pass begins
        let mut out = [0.0; 8];
        p.render(&mut out, 1, RATE);

        assert_eq!(
            frames_heard(&out, 100),
            vec![99, 98, 97, 96, 95, 94, 93, 92],
            "backwards playback must walk the sample in reverse"
        );
    }

    #[test]
    fn a_backwards_pass_is_the_mirror_of_the_forward_one() {
        // The property, rather than a hand-written list: whatever forward
        // plays, backwards plays the same values in the opposite order.
        let forward = preview(64);
        forward.play();
        let mut ahead = [0.0; 64];
        forward.render(&mut ahead, 1, RATE);

        let back = preview(64);
        back.set_reverse(true);
        back.play();
        let mut behind = [0.0; 64];
        back.render(&mut behind, 1, RATE);
        behind.reverse();

        // ⚠ The two passes read the same 64 frames from opposite ends, so they
        // are equal frame for frame once one is flipped.
        for (index, (a, b)) in ahead.iter().zip(behind.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1e-6,
                "frame {index}: forward {a} against reversed {b}"
            );
        }
    }

    #[test]
    fn looping_wraps_to_the_top_and_keeps_sounding() {
        // ⛔ `the_end_stops_it_unless_it_is_looping` only asks whether the flag
        // is still set. A loop that wrapped the cursor and stopped writing
        // samples — or wrapped to the wrong end — passes it.
        let p = preview(8);
        p.set_looping(true);
        p.play();
        let mut out = [0.0; 20];
        p.render(&mut out, 1, RATE);

        let heard = frames_heard(&out, 8);
        assert_eq!(
            heard,
            vec![0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3],
            "a loop plays the sample over and over from the top"
        );
        assert!(p.position().playing, "a loop never runs out");
    }

    #[test]
    fn looping_backwards_wraps_to_the_end_rather_than_the_start() {
        // ⚠ The other end, and it is the one a `step > 0.0` check gets wrong in
        // the quiet direction: wrapping a backwards pass to frame 0 leaves it
        // immediately out of travel again, so the loop plays one frame per
        // block forever.
        let p = preview(8);
        p.set_reverse(true);
        p.set_looping(true);
        p.play();
        let mut out = [0.0; 20];
        p.render(&mut out, 1, RATE);

        assert_eq!(
            frames_heard(&out, 8),
            vec![7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4],
        );
        assert!(p.position().playing);
    }

    #[test]
    fn a_loop_survives_a_rate_ratio_that_never_lands_on_a_frame() {
        // ⛔ **The wrap is `at = 0.0`, not `at -= len`**, so a fractional step
        // cannot accumulate an offset across repeats — but it also must not
        // stall: at a ratio where every position is fractional, a wrap that
        // compared `at > last` against a stale bound would either drop a repeat
        // or spin. Rendered long enough to wrap many times.
        let p = preview(7);
        p.set_looping(true);
        p.play();
        let mut out = [0.0; 512];
        p.render(&mut out, 1, RATE * 3.0 / 7.0);

        assert!(p.position().playing, "it must still be going");
        assert!(
            out.iter().any(|s| *s > 0.3),
            "a loop has to reach the loud end of the ramp, not creep along the quiet one"
        );
        let at = p.position();
        assert!(at.seconds.is_finite() && at.seconds >= 0.0 && at.seconds <= at.total);
    }

    #[test]
    fn a_click_lands_on_the_frame_it_names_at_any_device_rate() {
        // ⛔ The seek is in *seconds* and the cursor is in frames, so the two
        // conversions have to agree — and the device rate must not enter into
        // it at all. A seek that went through the device rate would land in a
        // different place in a 48 kHz session than in a 44.1 kHz one.
        for device in [RATE, RATE * 2.0, RATE / 2.0] {
            let p = preview(100);
            p.seek(0.25); // a quarter of a second at 100 Hz is frame 25
            p.play();
            let mut out = [0.0; 1];
            p.render(&mut out, 1, device);
            assert_eq!(
                frames_heard(&out, 100),
                vec![25],
                "a click at 0.25s must start on frame 25 whatever the device rate"
            );
        }
    }

    #[test]
    fn a_click_moves_the_readout_even_while_it_is_paused() {
        // ⛔⛔ **The marker used to snap back.** `render` returns early when
        // nothing is playing, so a paused seek sat in the request atomic and
        // `position` still held the old cursor — and the page's own optimistic
        // write was then overwritten by the very next `preview_position` poll,
        // half a second later. Clicking a paused waveform moved the playhead and
        // then visibly undid itself.
        let p = preview(100);
        p.play();
        p.render(&mut [0.0; 10], 1, RATE);
        p.pause();

        p.seek(0.6);
        assert!(
            (p.position().seconds - 0.6).abs() < 0.02,
            "a paused seek must be visible immediately, got {:?}",
            p.position()
        );

        // ...and it is still where the click put it once playback resumes.
        p.play();
        let mut out = [0.0; 1];
        p.render(&mut out, 1, RATE);
        assert_eq!(frames_heard(&out, 100), vec![60]);
    }

    #[test]
    fn a_seek_while_playing_takes_effect_on_the_next_block() {
        let p = preview(100);
        p.play();
        p.render(&mut [0.0; 10], 1, RATE);

        p.seek(0.8);
        let mut out = [0.0; 2];
        p.render(&mut out, 1, RATE);
        assert_eq!(frames_heard(&out, 100), vec![80, 81]);
    }

    #[test]
    fn play_after_it_ran_out_starts_over_rather_than_doing_nothing() {
        // A sample that finished is parked at the end it reached on purpose, so
        // Play has to rewind or it renders one frame and stops — which reads as
        // the player being broken.
        let p = preview(8);
        p.play();
        p.render(&mut [0.0; 32], 1, RATE);
        assert!(!p.position().playing, "it ran out");

        p.play();
        let mut out = [0.0; 3];
        p.render(&mut out, 1, RATE);
        assert_eq!(frames_heard(&out, 8), vec![0, 1, 2], "Play starts it over");
    }

    #[test]
    fn reverse_rewinds_to_the_end_only_when_there_is_nowhere_left_to_go_back_to() {
        // ⛔ **Two different situations, and they must not be conflated — this
        // test was written asserting they were, and the code was right.**
        //
        // Pressing ← *part way through* a sample means "go back from here",
        // which is a scrub and is what a producer expects from a direction
        // toggle. Pressing it on a sample that is parked at frame 0 — freshly
        // selected, or just stopped — means "play it backwards", and there is
        // no travel left in that direction, so it rewinds to the end.
        let mid = preview(8);
        mid.play();
        mid.render(&mut [0.0; 3], 1, RATE); // now sitting on frame 3
        mid.set_reverse(true);
        mid.play();
        let mut out = [0.0; 3];
        mid.render(&mut out, 1, RATE);
        assert_eq!(
            frames_heard(&out, 8),
            vec![3, 2, 1],
            "mid-sample, ← walks back from where the playhead is"
        );

        // Parked at the start — the common case, since `load` parks at 0 and
        // Mike's ask is "select a file and press left arrow".
        let fresh = preview(8);
        fresh.set_reverse(true);
        fresh.play();
        let mut back = [0.0; 3];
        fresh.render(&mut back, 1, RATE);
        assert_eq!(
            frames_heard(&back, 8),
            vec![7, 6, 5],
            "from the start, ← has to rewind to the end or it plays one frame"
        );

        // ...and a sample that ran out forwards is already at the far end, so ←
        // walks straight back from there without rewinding anywhere.
        let done = preview(8);
        done.play();
        done.render(&mut [0.0; 32], 1, RATE);
        done.set_reverse(true);
        done.play();
        let mut after = [0.0; 3];
        done.render(&mut after, 1, RATE);
        assert_eq!(frames_heard(&after, 8), vec![7, 6, 5]);
    }

    #[test]
    fn a_one_frame_sample_plays_once_and_stops_without_panicking() {
        // The degenerate end of every bound in `render`: `last` is 0.0, so the
        // forward and backward "out of travel" tests are both true immediately.
        let p = preview(1);
        p.play();
        let mut out = [0.0; 4];
        p.render(&mut out, 1, RATE);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(!p.position().playing);

        p.set_reverse(true);
        p.play();
        p.render(&mut [0.0; 4], 1, RATE);
        assert!(p.position().seconds.is_finite());
    }

    #[test]
    fn a_second_sample_replaces_the_first_rather_than_playing_over_it() {
        // ⚠ One at a time, which is what a browser preview means. The handoff
        // slot is the thing that can go wrong here — `load` drains the park slot
        // itself precisely so a second load is not silently ignored.
        let p = preview(100);
        p.play();
        p.render(&mut [0.0; 10], 1, RATE);

        // A flat sample, so its value cannot be confused with the ramp's.
        p.load(vec![0.5; 100], 100);
        p.play();
        let mut out = [0.0; 4];
        p.render(&mut out, 1, RATE);
        for sample in out {
            assert!(
                (sample - 0.5 * 0.7).abs() < 1e-6,
                "the second sample must be what is heard, got {sample}"
            );
        }
    }

    // ───────────────────────────────────────────────────────────────────────
    // The seams the fixtures above skip: a real file, and the two commands the
    // panel maps a click through.
    // ───────────────────────────────────────────────────────────────────────

    /// A 16-bit PCM WAV of `frames` frames at `rate`, one channel, ramping.
    ///
    /// ⚠ **`pub(crate)` so `editor.rs`'s end-to-end test uses this one.** That
    /// test needs the identical fixture, and this crate already carries four
    /// near-identical RIFF writers across its test modules — a fifth and sixth
    /// would mean finding six places the day a header field or an accepted rate
    /// has to change.
    pub(crate) fn wav_bytes(frames: usize, rate: u32) -> Vec<u8> {
        let pcm: Vec<i16> = (0..frames)
            .map(|i| ((i as f32 / frames as f32) * 30_000.0) as i16)
            .collect();
        let data: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend(((36 + data.len()) as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn a_real_decoded_file_plays_forwards_and_backwards() {
        // ⛔ **Every fixture above hands `load` a `Vec<f32>` it built itself**,
        // so none of them can see the seam that actually matters: what
        // `audio::import` returns. If the decoder ever handed back *interleaved*
        // frames rather than the mono fold it promises, `frames` would be double
        // the truth — the sample would play at half speed, the total time would
        // read double, and every click in the waveform would land in the wrong
        // place. All of it silently.
        // ⚠ A real rate, because `import` refuses one no device could run —
        // which is itself a guard worth not defeating with a fixture.
        let dir = std::env::temp_dir().join(format!("fmm-preview-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ramp.wav");
        std::fs::write(&path, wav_bytes(4_410, 44_100)).expect("a temp wav");

        let audio = crate::audio::import::decode_file(&path).expect("it decodes");
        assert_eq!(audio.samples.len(), 4_410, "mono frames, not interleaved");

        let p = Preview::default();
        p.load(audio.samples, audio.sample_rate);
        p.render(&mut [], 1, 44_100.0);

        // 4,410 frames at 44.1 kHz is a tenth of a second.
        assert!(
            (p.position().total - 0.1).abs() < 1e-4,
            "{:?}",
            p.position()
        );

        p.play();
        let mut forward = [0.0; 8];
        p.render(&mut forward, 1, 44_100.0);
        assert!(
            forward.windows(2).all(|w| w[1] >= w[0]),
            "a ramp read forwards climbs: {forward:?}"
        );

        p.stop();
        p.set_reverse(true);
        p.play();
        let mut back = [0.0; 8];
        p.render(&mut back, 1, 44_100.0);
        assert!(
            back.windows(2).all(|w| w[1] <= w[0]),
            "the same ramp read backwards falls: {back:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_waveform_and_the_player_agree_about_how_long_the_sample_is() {
        // ⛔⛔ **This is the seam a click in the waveform is mapped through, and
        // the two halves are computed in different modules.** The panel turns a
        // pixel into a fraction of the drawn wave, multiplies by the *total* the
        // player reports, and sends that to `seek`. If `explorer::waveform`'s
        // `seconds` and `Preview`'s `total` ever disagreed, every click would
        // land proportionally wrong — furthest out at the end of the sample,
        // which is the hardest place to notice and the easiest to blame on the
        // file.
        let dir = std::env::temp_dir().join(format!("fmm-preview-agree-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("ramp.wav");
        std::fs::write(&path, wav_bytes(4_410, 44_100)).expect("a temp wav");

        let explorer = crate::explorer::Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);
        let wave =
            crate::explorer::waveform(&explorer, path.to_str().unwrap()).expect("it describes");

        let audio = crate::audio::import::decode_file(&path).expect("it decodes");
        let p = Preview::default();
        p.load(audio.samples, audio.sample_rate);

        assert!(
            (wave.seconds - p.position().total).abs() < 1e-4,
            "the drawn wave is {}s and the player says {}s",
            wave.seconds,
            p.position().total
        );

        // ...and a click at the half-way point of the drawing lands half way
        // through the audio, which is what the panel actually computes.
        p.seek(wave.seconds / 2.0);
        p.play();
        p.render(&mut [0.0; 1], 1, RATE);
        let at = p.position();
        assert!(
            (at.seconds - at.total / 2.0).abs() < 0.01,
            "a click at the middle landed at {:?}",
            at
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_loaded_is_silent_rather_than_a_panic() {
        let p = Preview::default();
        p.play();
        let mut out = vec![0.0; 8];
        p.render(&mut out, 1, RATE);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(p.position().total, 0.0);
    }
}
