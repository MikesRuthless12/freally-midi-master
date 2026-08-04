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
    /// What the host reported, unmodified.
    playing: AtomicBool,
    /// Whether time is *actually* advancing — the host's transport and our own
    /// both saying yes (TASK-041T).
    ///
    /// ⛔ **Separate from `playing`, and the separation is the point.** This
    /// used to be folded back into the host's own record before publishing, so
    /// `HostSession::playing()` meant "the host is playing" for seven lines and
    /// "time is advancing" thereafter. One field with two meanings is how the
    /// gate in `process` ended up testing the same term twice.
    running: AtomicBool,
}

impl Default for SharedHost {
    fn default() -> Self {
        Self {
            tempo_bits: AtomicU64::new(0),
            // 4/4, packed as num << 16 | den.
            time_sig: AtomicU32::new(4 << 16 | 4),
            playing: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }
}

impl SharedHost {
    /// Publish what the host reported, and whether time is actually advancing.
    ///
    /// Called from `process`, so it must not allocate, lock or wait — four
    /// atomic stores. `running` is passed in rather than derived here because
    /// `process` is the one place that knows both halves, and it gates on the
    /// same value it publishes.
    pub fn publish(&self, session: &HostSession, running: bool) {
        let bits = session.tempo().map(f64::to_bits).unwrap_or(0);
        self.tempo_bits.store(bits, Ordering::Relaxed);

        let (num, den) = session.time_signature();
        self.time_sig
            .store(u32::from(num) << 16 | u32::from(den), Ordering::Relaxed);
        self.playing.store(session.playing(), Ordering::Relaxed);
        self.running.store(running, Ordering::Relaxed);
    }

    /// Whether time is advancing, for the editor to gate its playhead poll on.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// The host's session as the bridge wants it.
    pub fn snapshot(&self) -> HostSession {
        let bits = self.tempo_bits.load(Ordering::Relaxed);
        let tempo = (bits != 0).then(|| f64::from_bits(bits));
        let packed = self.time_sig.load(Ordering::Relaxed);

        let mut session =
            HostSession::observed_for_test(tempo, (packed >> 16) as u8, (packed & 0xFFFF) as u8);
        // ⛔ The *effective* value, not the raw one. `host_session` is what the
        // page draws its transport from: it gates the playhead poll and enables
        // Stop, so it has to mean "is time advancing" rather than "did the
        // backend claim a transport". The standalone's cpal backend claims one
        // unconditionally, which is exactly the case this distinction exists
        // for. `SharedHost::playing` keeps the raw report for anything that
        // needs to know what the host itself said.
        session.set_playing(self.running.load(Ordering::Relaxed));
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
    /// The one in-flight export and its outcome (TASK-073).
    ///
    /// ⛔ **Here rather than a module global, because this is per-*instance*
    /// state.** A DAW loads this plugin on twenty tracks in one process, and
    /// `take_status` is destructive — a shared slot means one instance's poll
    /// steals another's result. `crate::export::Exports` has the full write-up.
    pub exports: crate::export::Exports,
    /// The id of the clip the editor last handed over.
    ///
    /// ⛔ **`Schedule::arm`'s own resume path cannot work without this.** That
    /// function holds the playhead when the clip it is given is the one already
    /// armed — but `editor.rs` builds a *fresh* `Schedule` for every reply, so
    /// `armed_id` was always `None`, `same_clip` was always false, and every arm
    /// reset the position. Harmless while only a generation re-armed; the moment
    /// muting a part, soloing one, toggling a loop or starting an audition
    /// re-armed the song, clicking any of them mid-playback threw the record
    /// back to bar 1. The test that covers the resume re-arms the *same*
    /// `Schedule`, which is not the shape the plugin produces.
    armed_clip: Mutex<Option<String>>,
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
    /// A note the gutter asked to hear, as `pitch | AUDITION_PENDING`, or `0`
    /// (TASK-041).
    ///
    /// ⛔ **One slot, and the newest click wins — deliberately, unlike a note
    /// queue.** Running a finger down the keyboard sends a click per row far
    /// faster than blocks arrive; queueing them would play the whole run back
    /// seconds later, long after the pointer stopped. What a producer means by
    /// dragging down the gutter is "let me hear where I am now".
    audition_request: AtomicU32,
    /// Whether this process is our own standalone rather than a DAW.
    ///
    /// ⛔ **Read once at construction from the process-wide flag, and held
    /// here.** A free static could not be varied per test, so every standalone
    /// branch was untestable in the same binary as the tests that need a host.
    /// Held as a plain `bool` because it cannot change while the plugin is
    /// loaded — a library does not become an executable.
    pub standalone: bool,
    /// Whether *our* transport is running, which is only ever a question in the
    /// standalone (TASK-041T).
    ///
    /// ⛔ **Inside a DAW this stays true and the host decides.** The host owns
    /// whether time runs; a second run/stop flag of ours would be a way for the
    /// plugin to sit silent through a playing project with nothing on screen
    /// explaining it. `process` gates on the host's transport *and* this, so in
    /// a host the term is a constant and the behaviour is unchanged.
    ///
    /// The standalone is the case this exists for: nih-plug's cpal backend sets
    /// `transport.playing = true` unconditionally, so without this there is no
    /// pause and no stop — a generated pattern loops forever from the moment it
    /// lands, and Stop rewinds to zero and keeps playing.
    running: AtomicBool,
}

impl Shared {
    /// Build the shared state around the session store the host will persist.
    ///
    /// **This is the constructor production code must use.** The store has to
    /// be the *same* `Arc` that `FreallyParams` holds, or the host saves one
    /// value and the editor shows another.
    pub fn new(session: SessionStore) -> Self {
        let standalone = crate::is_standalone();
        Self {
            host: SharedHost::default(),
            handoff: Handoff::default(),
            exports: crate::export::Exports::default(),
            armed_clip: Mutex::new(None),
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
            audition_request: AtomicU32::new(0),
            standalone,
            // ⛔ **True in a host, false in the standalone, and the asymmetry is
            // the point.** In a host this term must never be the one that
            // decides — the DAW's transport is, and a `false` here would silence
            // a playing project with nothing on screen to explain it. In the
            // standalone nih-plug's cpal backend claims a running transport on
            // every block, so `true` here means the editor comes up already
            // "playing": Pause and Stop offered for a pattern that does not
            // exist, and a 30 Hz playhead poll running forever on an idle
            // window. Nothing should advance there until someone presses Play.
            running: AtomicBool::new(!standalone),
        }
    }

    /// A [`Shared`] that believes it is the standalone, for tests.
    ///
    /// The process-wide flag is set from the standalone's `main`, which no test
    /// runs — and setting it globally would leak into the tests that need a
    /// host. This is the seam that keeps both testable in one binary.
    #[cfg(test)]
    pub fn standalone_for_test() -> Self {
        Self {
            standalone: true,
            // Both, because `new` derives `running` from `standalone` and the
            // process-wide flag says "host" inside a test binary.
            running: AtomicBool::new(false),
            ..Self::new(SessionStore::default())
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

    /// Note which clip has just been handed over, and say whether it is the one
    /// that was already playing.
    ///
    /// ⛔ **The two are one call because the answer is only true once.** A
    /// caller that asked and then set would race itself on a second arm
    /// arriving between the two, and the cost of getting it wrong is the record
    /// jumping to bar 1 under the producer's hands.
    pub fn arming(&self, id: &str) -> bool {
        let Ok(mut slot) = self.armed_clip.lock() else {
            return false;
        };
        let same = slot.as_deref() == Some(id);
        *slot = Some(id.to_owned());
        same
    }

    /// Forget what was armed, so the next arm starts from the top.
    pub fn disarmed(&self) {
        if let Ok(mut slot) = self.armed_clip.lock() {
            *slot = None;
        }
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

    /// Ask the audio thread to sound one note. UI thread (TASK-041).
    ///
    /// ⛔ The pending flag exists for the same reason [`request_seek`]'s does:
    /// pitch 0 is a real MIDI note, so a bare value could not distinguish
    /// "audition C-1" from "nothing pending" — and C-1 is reachable by
    /// scrolling the gutter to the bottom.
    pub fn request_audition(&self, pitch: u8) {
        self.audition_request.store(
            u32::from(pitch.min(127)) | AUDITION_PENDING,
            Ordering::Relaxed,
        );
    }

    /// Take a pending audition, if there is one. `process` is the caller.
    pub fn take_audition(&self) -> Option<u8> {
        let packed = self.audition_request.swap(0, Ordering::Relaxed);
        (packed & AUDITION_PENDING != 0).then_some((packed & 0x7f) as u8)
    }

    /// Whether our own transport is running (TASK-041T). Read every block.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Start or hold our own transport.
    ///
    /// ⛔ **Self-gating, so the rule lives here rather than at every caller.**
    /// A host owns whether time runs; letting one reach this would hand a DAW a
    /// second transport that can silence the plugin permanently with nothing on
    /// screen to explain it. Written as a no-op rather than an error because
    /// the callers that *can* reach it already refuse first — this is the
    /// backstop, and a backstop that panics is worse than the bug.
    pub fn set_running(&self, running: bool) {
        if !self.standalone {
            return;
        }
        self.running.store(running, Ordering::Relaxed);
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

/// Marks an audition request as present, so pitch 0 is not read as "nothing".
///
/// The same trick as [`SEEK_PENDING`] against a different payload: a MIDI pitch
/// needs seven bits, so the top one is free for the same reason.
const AUDITION_PENDING: u32 = 1 << 31;

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
        shared.publish(&HostSession::observed_for_test(Some(92.5), 6, 8), true);

        let snapshot = shared.snapshot();
        assert_eq!(snapshot.tempo(), Some(92.5));
        assert_eq!(snapshot.time_signature(), (6, 8));
    }

    #[test]
    fn an_unknown_tempo_survives_the_round_trip_as_unknown() {
        // The sentinel has to stay distinguishable from a value, or the chip
        // shows 0 BPM while the host simply has not said yet.
        let shared = SharedHost::default();
        shared.publish(&HostSession::observed_for_test(None, 4, 4), false);
        assert_eq!(shared.snapshot().tempo(), None);
    }

    #[test]
    fn the_snapshot_reports_the_effective_transport_not_the_hosts_claim() {
        // ⛔ The standalone's backend claims a running transport on every block
        // whatever the user pressed, so the page has to be told what is
        // *actually* advancing — otherwise Pause flips back on the next poll and
        // the marker carries on. The raw claim stays available separately.
        let shared = SharedHost::default();
        let mut host = HostSession::observed_for_test(Some(120.0), 4, 4);
        host.set_playing(true);

        shared.publish(&host, false);
        assert!(!shared.running());
        assert!(
            !shared.snapshot().playing(),
            "a paused transport must not read as playing just because the host says so"
        );

        shared.publish(&host, true);
        assert!(shared.snapshot().playing());
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

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn our_own_transport_runs_until_something_holds_it() {
        // ⛔ True by default is what keeps a host's behaviour unchanged. In a
        // DAW nothing ever calls `set_running`, so this term is a constant and
        // the host's own transport is the one that decides.
        let shared = Shared::standalone_for_test();
        // ⛔ Stopped at launch. The standalone's backend claims a running
        // transport on every block, so starting `true` here would have the
        // editor come up already playing a pattern nobody generated — Pause and
        // Stop offered for nothing, and a 30 Hz playhead poll on an idle window.
        assert!(!shared.running());

        shared.set_running(true);
        assert!(shared.running());

        shared.set_running(false);
        assert!(!shared.running());
    }

    #[test]
    fn a_host_cannot_be_handed_a_second_transport() {
        // ⛔ The backstop for the rule the callers already enforce. A DAW owns
        // whether time runs; letting one reach `set_running` would silence the
        // plugin for the rest of the session with nothing on screen to explain
        // it. `Shared::default()` is not the standalone, so this must not take.
        let shared = Shared::default();
        assert!(!shared.standalone);
        shared.set_running(false);
        assert!(
            shared.running(),
            "a host must not be able to hold our transport"
        );
    }

    #[test]
    fn pausing_leaves_the_playhead_where_it_is_and_stopping_does_not() {
        // The whole distinction between the two controls, at the level the
        // audio thread sees it: pause writes nothing to the position, stop
        // asks for a seek to zero. A pause that moved the marker would be a
        // stop, and there would be no way to resume from the middle of a bar.
        let shared = Shared::standalone_for_test();
        shared.set_running(true);
        shared.set_playhead(0.42);

        shared.set_running(false);
        assert_eq!(shared.playhead(), 0.42, "pause must not move the marker");
        assert!(
            shared.take_seek().is_none(),
            "pause must not queue a seek — that is what stop does"
        );

        // What `stop_playback` does, and it has to survive being a seek to
        // exactly zero (see `SEEK_PENDING`).
        shared.request_seek(0.0);
        assert_eq!(shared.take_seek(), Some(0.0));
    }

    #[test]
    fn an_audition_survives_being_the_lowest_note_on_the_keyboard() {
        // ⛔ The same trap `SEEK_PENDING` exists for, one payload over: pitch 0
        // is a real MIDI note and it is reachable — scroll the gutter to the
        // bottom and click. Without the flag it would be indistinguishable from
        // "nothing pending" and C-1 would be the one key that never sounded.
        let shared = Shared::default();
        assert!(shared.take_audition().is_none(), "nothing asked for yet");

        shared.request_audition(0);
        assert_eq!(shared.take_audition(), Some(0));
    }

    #[test]
    fn taking_an_audition_clears_it() {
        // A request that stayed set would re-trigger on every block — the note
        // would not sound once, it would machine-gun for as long as the editor
        // was open. This is the same failure `fired.clear()` guards in `process`.
        let shared = Shared::default();
        shared.request_audition(64);

        assert_eq!(shared.take_audition(), Some(64));
        assert!(shared.take_audition().is_none(), "one click, one note");
    }

    #[test]
    fn the_newest_audition_wins_rather_than_queueing() {
        // Running a finger down the gutter sends clicks far faster than blocks
        // arrive. Queueing them would play the run back seconds late, after the
        // pointer had stopped; what the gesture means is "where am I now".
        let shared = Shared::default();
        shared.request_audition(60);
        shared.request_audition(72);

        assert_eq!(shared.take_audition(), Some(72));
    }

    #[test]
    fn an_out_of_range_audition_is_clamped_rather_than_wrapping() {
        // The pitch shares its word with the pending flag, so a value past 127
        // that was stored raw would corrupt the flag rather than merely being
        // wrong — the request would read back as a different note, or as none.
        let shared = Shared::default();
        shared.request_audition(200);

        assert_eq!(shared.take_audition(), Some(127));
    }
}

#[cfg(test)]
mod arming_tests {
    use super::*;

    #[test]
    fn re_arming_the_same_clip_is_reported_as_the_same_clip() {
        // ⛔ What decides whether the playhead is held. `editor.rs` builds a
        // *fresh* `Schedule` for every reply, so `Schedule::arm`'s own resume
        // path — which keys on its `armed_id` — could never fire: `armed_id`
        // was always `None`. Harmless while only a generation re-armed, and
        // then muting a part, soloing one, toggling a loop or starting an
        // audition all began re-arming the song, so clicking any of them
        // mid-record threw it back to bar 1.
        let shared = Shared::default();

        assert!(
            !shared.arming("trap-song-7-flat"),
            "the first arm of a clip is not a re-arm"
        );
        assert!(
            shared.arming("trap-song-7-flat"),
            "the same clip again is the case the playhead must survive"
        );
        assert!(
            !shared.arming("trap-song-9-flat"),
            "a different song must start from the top"
        );
    }

    #[test]
    fn disarming_makes_the_next_arm_start_from_the_top() {
        // Leaving Song Mode takes the arrangement off the transport, so coming
        // back to it is a fresh start rather than a resume — otherwise the
        // marker would jump to wherever the record had got to before.
        let shared = Shared::default();
        assert!(!shared.arming("trap-song-7-flat"));
        assert!(shared.arming("trap-song-7-flat"));

        shared.disarmed();
        assert!(
            !shared.arming("trap-song-7-flat"),
            "a disarm must not leave the next arm resuming"
        );
    }
}
