//! What the audio thread and the editor thread share.
//!
//! Two directions, and they have different rules:
//!
//! - **Audio → UI** is the host's tempo and meter. Written every process
//!   block, read whenever the UI feels like it. Atomics, so `process` never
//!   waits for a UI thread that is busy laying out a webview.
//! - **UI → audio** is a generated pattern, already placed. The
//!   [`Schedule`](crate::Schedule) is *armed on the UI thread* — that is where
//!   the allocation happens, deliberately — and handed over as a whole.
//!
//! The handoff follows the rule this codebase already set for the desktop
//! app's transport: **a replaced schedule is handed back, never dropped.**
//! Freeing a `Vec` on the audio thread takes the allocator's lock, which is
//! the one thing a callback must not do.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::host::HostSession;
use crate::voice::Schedule;

/// The host's transport, readable from any thread without blocking.
#[derive(Debug)]
pub struct SharedHost {
    /// `f64` bits, or `0` for "the host has not said". A real tempo is never
    /// zero, so the sentinel cannot collide with a value.
    tempo_bits: AtomicU64,
    time_sig: AtomicU32,
    playing: AtomicBool,
}

impl Default for SharedHost {
    fn default() -> Self {
        Self {
            tempo_bits: AtomicU64::new(0),
            // 4/4, packed as num << 16 | den.
            time_sig: AtomicU32::new(4 << 16 | 4),
            playing: AtomicBool::new(false),
        }
    }
}

impl SharedHost {
    /// Publish what the host reported. Called from `process`, so it must not
    /// allocate, lock or wait — three atomic stores.
    pub fn publish(&self, session: &HostSession) {
        let bits = session.tempo().map(f64::to_bits).unwrap_or(0);
        self.tempo_bits.store(bits, Ordering::Relaxed);

        let (num, den) = session.time_signature();
        self.time_sig
            .store(u32::from(num) << 16 | u32::from(den), Ordering::Relaxed);
        self.playing.store(session.playing(), Ordering::Relaxed);
    }

    /// The host's session as the bridge wants it.
    pub fn snapshot(&self) -> HostSession {
        let bits = self.tempo_bits.load(Ordering::Relaxed);
        let tempo = (bits != 0).then(|| f64::from_bits(bits));
        let packed = self.time_sig.load(Ordering::Relaxed);

        let mut session =
            HostSession::observed_for_test(tempo, (packed >> 16) as u8, (packed & 0xFFFF) as u8);
        session.set_playing(self.playing.load(Ordering::Relaxed));
        session
    }
}

/// A generated pattern on its way to the audio thread, and the spent one on
/// its way back.
///
/// One slot in each direction rather than a queue: a second Generate before
/// the first was picked up replaces it, which is what the user meant — they
/// pressed the button again because they wanted the newer beat.
#[derive(Debug, Default)]
pub struct Handoff {
    incoming: Mutex<Option<Schedule>>,
    /// What `process` swapped out, waiting for a thread that is allowed to
    /// free it.
    spent: Mutex<Option<Schedule>>,
}

impl Handoff {
    /// Hand an armed schedule to the audio thread. UI thread only.
    pub fn send(&self, schedule: Schedule) {
        if let Ok(mut slot) = self.incoming.lock() {
            *slot = Some(schedule);
        }
    }

    /// Take whatever is waiting, and leave the old one behind to be freed.
    ///
    /// **Audio thread.** `try_lock` rather than `lock`: a UI thread mid-write
    /// must never stall the callback. A missed block costs a few milliseconds
    /// before the pattern starts, which nobody can hear; a blocked callback is
    /// a dropout, which everybody can.
    #[must_use = "the schedule this replaces must be handed back, not dropped here"]
    pub fn receive(&self, current: Schedule) -> Schedule {
        let Ok(mut slot) = self.incoming.try_lock() else {
            return current;
        };
        let Some(next) = slot.take() else {
            return current;
        };
        drop(slot);

        // The outgoing one is *parked*, not dropped: freeing its `Vec` here
        // would take the allocator's lock on the audio thread.
        if let Ok(mut spent) = self.spent.try_lock() {
            *spent = Some(current);
        }
        next
    }

    /// Free anything the audio thread parked. UI thread only.
    pub fn collect(&self) {
        if let Ok(mut spent) = self.spent.lock() {
            spent.take();
        }
    }
}

/// Everything both threads reach.
#[derive(Debug)]
pub struct Shared {
    pub host: SharedHost,
    pub handoff: Handoff,
    /// The host's sample rate, from `initialize`.
    ///
    /// The editor arms schedules in *samples*, so this has to be the real
    /// value: guessing 48 kHz inside a 44.1 kHz session places every note
    /// 8.8% late, which is a whole 16th note out by the end of four bars —
    /// audible, and easy to blame on the generator instead.
    sample_rate: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            host: SharedHost::default(),
            handoff: Handoff::default(),
            // Only ever read before `initialize` has run, which no host does
            // before opening an editor — but a wrong guess is quieter than a
            // zero, and a zero here would place every note at tick 0.
            sample_rate: AtomicU32::new(48_000),
        }
    }
}

impl Shared {
    /// Publish the rate the host initialised us at.
    pub fn set_sample_rate(&self, rate: f32) {
        // A host reporting a nonsense rate is not a rate. Keeping the last
        // believed value is better than placing a pattern against zero.
        // `is_finite` first: `contains` on a range answers false for NaN, but
        // saying so explicitly is what makes the NaN case readable rather than
        // incidental.
        if rate.is_finite() && (8_000.0..=768_000.0).contains(&rate) {
            self.sample_rate.store(rate as u32, Ordering::Relaxed);
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate.load(Ordering::Relaxed) as f32
    }
}

pub type SharedState = Arc<Shared>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_tempo_reads_back_exactly() {
        let shared = SharedHost::default();
        shared.publish(&HostSession::observed_for_test(Some(92.5), 6, 8));

        let snapshot = shared.snapshot();
        assert_eq!(snapshot.tempo(), Some(92.5));
        assert_eq!(snapshot.time_signature(), (6, 8));
    }

    #[test]
    fn an_unknown_tempo_survives_the_round_trip_as_unknown() {
        // The sentinel has to stay distinguishable from a value, or the chip
        // shows 0 BPM while the host simply has not said yet.
        let shared = SharedHost::default();
        shared.publish(&HostSession::observed_for_test(None, 4, 4));
        assert_eq!(shared.snapshot().tempo(), None);
    }

    #[test]
    fn the_audio_thread_never_drops_the_schedule_it_replaces() {
        // The rule the desktop app's transport already set. If `receive` freed
        // the outgoing schedule, that `Vec` would be released on the audio
        // thread — the allocator lock a callback must not take.
        let handoff = Handoff::default();
        handoff.send(Schedule::default());

        let replaced = handoff.receive(Schedule::default());
        let _ = replaced;
        assert!(
            handoff.spent.lock().unwrap().is_some(),
            "the outgoing schedule should have been parked for the UI thread"
        );

        handoff.collect();
        assert!(handoff.spent.lock().unwrap().is_none());
    }

    #[test]
    fn the_hosts_sample_rate_reaches_the_editor() {
        // Guessing 48 kHz in a 44.1 kHz session places every note 8.8% late —
        // a whole 16th out by the end of four bars, and easy to blame on the
        // generator instead of on this number.
        let shared = Shared::default();
        assert_eq!(shared.sample_rate(), 48_000.0);

        shared.set_sample_rate(44_100.0);
        assert_eq!(shared.sample_rate(), 44_100.0);

        shared.set_sample_rate(96_000.0);
        assert_eq!(shared.sample_rate(), 96_000.0);
    }

    #[test]
    fn a_nonsense_sample_rate_leaves_the_last_believed_one() {
        // Zero would place every note at tick 0, which reads as "the generator
        // produced one chord" rather than as a bad rate.
        let shared = Shared::default();
        shared.set_sample_rate(44_100.0);

        for bad in [0.0, -48_000.0, f32::NAN, f32::INFINITY, 1.0] {
            shared.set_sample_rate(bad);
            assert_eq!(shared.sample_rate(), 44_100.0, "accepted {bad}");
        }
    }

    #[test]
    fn an_empty_handoff_returns_what_it_was_given() {
        let handoff = Handoff::default();
        let mine = Schedule::default();
        let back = handoff.receive(mine.clone());
        assert_eq!(back, mine, "nothing waiting means nothing changes");
    }

    #[test]
    fn a_second_generation_replaces_an_unclaimed_first() {
        // Pressing Generate twice before the audio thread looked means the
        // user wanted the second beat, not both.
        let handoff = Handoff::default();
        handoff.send(Schedule::default());
        handoff.send(Schedule::default());
        assert!(handoff.incoming.lock().unwrap().is_some());

        let _ = handoff.receive(Schedule::default());
        assert!(
            handoff.incoming.lock().unwrap().is_none(),
            "one receive should have drained it"
        );
    }
}
