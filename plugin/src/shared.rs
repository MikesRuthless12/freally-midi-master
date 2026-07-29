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
use crate::state::SessionStore;
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
    /// What the host saves with the project, and what the editor restores from.
    ///
    /// The same `Arc` as the persisted field on
    /// [`FreallyParams`](crate::FreallyParams) — see [`Shared::new`].
    pub session: SessionStore,
    /// A window size the UI has asked for, packed `width << 32 | height`, or
    /// `0` for "nothing pending".
    ///
    /// The editor can only resize itself from inside its own event loop, where
    /// baseview hands over the window — but the request arrives on the bridge,
    /// which answers an HTTP call from the page. This is the one-slot handoff
    /// between the two, and it is an atomic because the frame loop reads it on
    /// every tick and must not take a lock to find out there is nothing to do.
    resize_request: AtomicU64,
    /// The host's sample rate, from `initialize`.
    ///
    /// The editor arms schedules in *samples*, so this has to be the real
    /// value: guessing 48 kHz inside a 44.1 kHz session places every note
    /// 8.8% late, which is a whole 16th note out by the end of four bars —
    /// audible, and easy to blame on the generator instead.
    sample_rate: AtomicU32,
}

impl Shared {
    /// Build the shared state around the session store the host will persist.
    ///
    /// **This is the constructor production code must use.** The store has to
    /// be the *same* `Arc` that `FreallyParams` holds, or the host saves one
    /// value and the editor shows another.
    pub fn new(session: SessionStore) -> Self {
        Self {
            host: SharedHost::default(),
            handoff: Handoff::default(),
            session,
            resize_request: AtomicU64::new(0),
            // Only ever read before `initialize` has run, which no host does
            // before opening an editor — but a wrong guess is quieter than a
            // zero, and a zero here would place every note at tick 0.
            sample_rate: AtomicU32::new(48_000),
        }
    }
}

/// A [`Shared`] whose session store is **detached** — connected to no
/// `FreallyParams`, so nothing it holds is ever saved.
///
/// That is what a test wants and what the plugin must never use; the plugin
/// builds its own through [`Shared::new`].
impl Default for Shared {
    fn default() -> Self {
        Self::new(SessionStore::default())
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

    /// Ask the editor to become this size, in physical pixels.
    ///
    /// Nothing happens here: the window belongs to the frame loop, and this is
    /// only where the request waits for it. A second request before the first
    /// is picked up replaces it, for the same reason [`Handoff`] keeps one slot
    /// — the user clicked again because they wanted the newer size.
    pub fn request_resize(&self, width: u32, height: u32) {
        let packed = (u64::from(width) << 32) | u64::from(height);
        self.resize_request.store(packed, Ordering::Relaxed);
    }

    /// Take a pending resize, if there is one. The frame loop is the caller.
    pub fn take_resize(&self) -> Option<(u32, u32)> {
        let packed = self.resize_request.swap(0, Ordering::Relaxed);
        // Zero is the sentinel, and it cannot collide with a real request: a
        // window of zero width or height is not a size anything would ask for.
        (packed != 0).then_some(((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32))
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
    fn a_resize_request_is_taken_exactly_once() {
        // The frame loop runs at sixty hertz; a request that survived being
        // read would resize the window on every tick forever.
        let shared = Shared::default();
        assert_eq!(shared.take_resize(), None);

        shared.request_resize(2160, 1350);
        assert_eq!(shared.take_resize(), Some((2160, 1350)));
        assert_eq!(shared.take_resize(), None, "it should not repeat");
    }

    #[test]
    fn a_second_size_replaces_an_unclaimed_first() {
        let shared = Shared::default();
        shared.request_resize(2160, 1350);
        shared.request_resize(2560, 1528);
        assert_eq!(shared.take_resize(), Some((2560, 1528)));
    }

    #[test]
    fn a_large_size_survives_the_packing() {
        // Packed into one `u64` as `width << 32 | height`. A 4K-wide window is
        // well inside `u32`, but the shift is the kind of arithmetic that is
        // wrong silently, so it gets a test rather than a reading.
        let shared = Shared::default();
        shared.request_resize(3840, 2160);
        assert_eq!(shared.take_resize(), Some((3840, 2160)));
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
