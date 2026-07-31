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

use engine::pattern::Lane;
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
    /// Whether the preview sampler may make sound at all (FMM-S02).
    ///
    /// ⛔ **MIDI-only is a first-class mode, not a degraded one.** A producer
    /// routing this plugin's notes into Battery or Kontakt does not want the
    /// preview kit doubling every hit — that was the plugin's only behaviour
    /// before the sampler existed, and it has to stay reachable in one switch.
    audio_enabled: AtomicBool,
    /// Lanes whose *audio* is muted, as a bitmask (FMM-S02).
    ///
    /// ⛔ The MIDI keeps flowing for a muted lane — that is the whole feature.
    /// It lets the plugin play the hats while the producer's own sampler in the
    /// DAW takes the snare, which muting the lane outright cannot express.
    muted_lanes: AtomicU32,
    /// Where the playhead is, 0.0–1.0 through the pattern (TASK-041T).
    ///
    /// Stored as the `f32`'s bits so the audio thread can publish it with one
    /// relaxed store and the editor can read it without a lock — the same shape
    /// [`SharedHost`] already uses for the tempo, and for the same reason: the
    /// editor polls this at frame rate and `process` must never wait for it.
    playhead_bits: AtomicU32,
    /// A seek the UI has asked for, as `bits | SEEK_PENDING`, or `0`.
    ///
    /// One slot, like the resize request: clicking twice before a block runs
    /// means the second click is where the user wants to be.
    seek_request: AtomicU32,
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
            // ⛔ On by default, and the reason it is safe to default this way
            // is a product fact rather than a technical one: **no build has
            // ever shipped**, so no saved project predates the sampler and
            // none can be surprised by it. A generator nobody can hear without
            // wiring an instrument up first is the problem P17 exists to fix.
            audio_enabled: AtomicBool::new(true),
            muted_lanes: AtomicU32::new(0),
            playhead_bits: AtomicU32::new(0),
            seek_request: AtomicU32::new(0),
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

    /// Publish where the playhead is. Called from `process`.
    pub fn set_playhead(&self, progress: f32) {
        // Already 0..1 — `Schedule::progress` clamps. (`request_seek` clamps for
        // a different reason: it is what keeps the sign bit free for
        // `SEEK_PENDING`, so that one is load-bearing.)
        self.playhead_bits
            .store(progress.to_bits(), Ordering::Relaxed);
    }

    /// Where the playhead is, for the editor to draw.
    pub fn playhead(&self) -> f32 {
        f32::from_bits(self.playhead_bits.load(Ordering::Relaxed))
    }

    /// Ask the audio thread to move the playhead. UI thread.
    pub fn request_seek(&self, progress: f32) {
        // ⛔ The flag is what makes a seek to 0.0 expressible. Storing the bits
        // alone would make "go back to the start" — which is what Stop does —
        // indistinguishable from "nothing pending", so Stop would do nothing.
        let bits = progress.clamp(0.0, 1.0).to_bits();
        self.seek_request
            .store(bits | SEEK_PENDING, Ordering::Relaxed);
    }

    /// Take a pending seek, if there is one. `process` is the caller.
    pub fn take_seek(&self) -> Option<f32> {
        let packed = self.seek_request.swap(0, Ordering::Relaxed);
        (packed & SEEK_PENDING != 0).then(|| f32::from_bits(packed & !SEEK_PENDING))
    }

    /// Whether the preview sampler may sound (FMM-S02). Read every block.
    pub fn audio_enabled(&self) -> bool {
        self.audio_enabled.load(Ordering::Relaxed)
    }

    pub fn set_audio_enabled(&self, on: bool) {
        self.audio_enabled.store(on, Ordering::Relaxed);
    }

    /// Whether this lane's *audio* is muted. Its notes go out regardless.
    pub fn lane_muted(&self, lane: Lane) -> bool {
        self.muted_lanes.load(Ordering::Relaxed) & lane_bit(lane) != 0
    }

    pub fn set_lane_muted(&self, lane: Lane, muted: bool) {
        let bit = lane_bit(lane);
        let mask = self.muted_lanes.load(Ordering::Relaxed);
        self.muted_lanes.store(
            if muted { mask | bit } else { mask & !bit },
            Ordering::Relaxed,
        );
    }

    /// Replace the whole mute set, for a session arriving from the host.
    pub fn set_muted_lanes(&self, lanes: &[Lane]) {
        let mask = lanes.iter().fold(0, |mask, lane| mask | lane_bit(*lane));
        self.muted_lanes.store(mask, Ordering::Relaxed);
    }

    /// Copy the stored session's audio settings into the atomics.
    ///
    /// ⛔ **The audio thread cannot read the session** — taking that lock in
    /// `process` is the dropout this module exists to avoid — so these two live
    /// as atomics and this is what keeps them true.
    ///
    /// ⛔ **Called from `initialize`, not only after a save, and that is the
    /// whole point.** [`SessionStore`] is the `#[persist]` field, so the host
    /// deserializes a restored project *straight into the store* and never goes
    /// near `save_session_state`. Projecting only on save meant a project saved
    /// MIDI-only reopened making sound, until the page happened to persist
    /// something unrelated — the setting was on disk and simply not being read.
    pub fn adopt_session(&self) {
        let session = crate::state::read(&self.session);
        self.set_audio_enabled(session.audio_enabled);
        self.set_muted_lanes(&session.muted_lanes);
    }

    /// Which lanes are muted, for saving with the project.
    pub fn muted_lanes(&self) -> Vec<Lane> {
        let mask = self.muted_lanes.load(Ordering::Relaxed);
        ALL_LANES
            .iter()
            .copied()
            .filter(|lane| mask & lane_bit(*lane) != 0)
            .collect()
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

/// Marks a seek request as present, so a seek to 0.0 is not read as "nothing".
///
/// The top bit of an `f32`'s bit pattern is its sign, and a playhead is never
/// negative — so it is free to borrow.
const SEEK_PENDING: u32 = 1 << 31;

/// Every lane, so the mask can be read back out as names.
const ALL_LANES: &[Lane] = &[
    Lane::Kick,
    Lane::Snare,
    Lane::Clap,
    Lane::ClosedHat,
    Lane::OpenHat,
    Lane::Rim,
    Lane::Snap,
    Lane::Perc,
    Lane::Bass808,
    Lane::Melody,
    Lane::Counter,
    Lane::Bass,
    Lane::Chords,
];

/// This lane's bit in the mute mask.
///
/// Written out rather than cast from the enum's discriminant, which `Lane` does
/// not expose anyway. ⚠ **The bits are process-local and never reach a file** —
/// `PluginSession.muted_lanes` is a `Vec<Lane>` and `Lane` serializes *by name*
/// (`engine/src/pattern.rs`), so a reordered enum cannot remap anything saved.
/// An earlier version of this comment claimed the opposite and used it to
/// justify the table; the table is still the clearer form, but it is a
/// readability choice rather than a persistence guarantee.
fn lane_bit(lane: Lane) -> u32 {
    match lane {
        Lane::Kick => 1 << 0,
        Lane::Snare => 1 << 1,
        Lane::Clap => 1 << 2,
        Lane::ClosedHat => 1 << 3,
        Lane::OpenHat => 1 << 4,
        Lane::Rim => 1 << 5,
        Lane::Snap => 1 << 6,
        Lane::Perc => 1 << 7,
        Lane::Bass808 => 1 << 8,
        Lane::Melody => 1 << 9,
        Lane::Counter => 1 << 10,
        Lane::Bass => 1 << 11,
        Lane::Chords => 1 << 12,
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

#[cfg(test)]
mod bypass_tests {
    use super::*;

    #[test]
    fn the_sampler_sounds_by_default_and_mutes_nothing() {
        // ⛔ On by default is the product decision that makes the sampler worth
        // porting: a generator nobody hears without wiring an instrument up
        // first is the problem it was built to fix.
        let shared = Shared::default();
        assert!(shared.audio_enabled());
        assert!(shared.muted_lanes().is_empty());
    }

    #[test]
    fn a_muted_lane_is_muted_and_its_neighbours_are_not() {
        // The bitmask is saved into a project file, so setting one lane must
        // not disturb another — an off-by-one here mutes the wrong drum and
        // only shows up as "the snare stopped working" in someone's session.
        let shared = Shared::default();
        shared.set_lane_muted(Lane::Snare, true);

        assert!(shared.lane_muted(Lane::Snare));
        for lane in [Lane::Kick, Lane::ClosedHat, Lane::Bass808, Lane::Chords] {
            assert!(!shared.lane_muted(lane), "{lane:?} should be untouched");
        }

        shared.set_lane_muted(Lane::Snare, false);
        assert!(!shared.lane_muted(Lane::Snare));
    }

    #[test]
    fn every_lane_has_its_own_bit() {
        // Two lanes sharing a bit would mute in pairs, which is the kind of
        // mistake a hand-written mapping exists to make findable.
        let mut seen = 0u32;
        for lane in ALL_LANES {
            let bit = lane_bit(*lane);
            assert_eq!(bit.count_ones(), 1, "{lane:?} is not a single bit");
            assert_eq!(seen & bit, 0, "{lane:?} collides with an earlier lane");
            seen |= bit;
        }
        assert_eq!(seen.count_ones() as usize, ALL_LANES.len());
    }

    #[test]
    fn a_mute_set_survives_the_round_trip_a_project_puts_it_through() {
        // What is saved has to come back as the same lanes, or reopening a
        // project silences a different part of the kit than it did on save.
        let shared = Shared::default();
        shared.set_muted_lanes(&[Lane::OpenHat, Lane::Bass808, Lane::Chords]);

        let mut back = shared.muted_lanes();
        back.sort();
        let mut expected = vec![Lane::OpenHat, Lane::Bass808, Lane::Chords];
        expected.sort();
        assert_eq!(back, expected);

        // Replacing the set clears what was there rather than adding to it.
        shared.set_muted_lanes(&[Lane::Kick]);
        assert_eq!(shared.muted_lanes(), vec![Lane::Kick]);
    }
}
