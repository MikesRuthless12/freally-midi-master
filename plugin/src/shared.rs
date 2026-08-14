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

use crate::audio::kit::Kit;
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

/// A rebuilt preview kit on its way to the audio thread (TASK-131B).
///
/// The same shape as [`Handoff`] and for the same two reasons — one slot,
/// because a second assignment before the first was picked up is simply the
/// newer kit; and the replaced one is *parked* rather than dropped, because
/// freeing a kit's samples on the audio thread takes the allocator's lock.
///
/// ⛔ **Separate from `Handoff` rather than generic over it.** They are handed
/// over on different events and the audio thread has to do something different
/// on each: a new schedule keeps the sounding voices, a new kit **must cut
/// them**. Sharing the type would have hidden that difference behind a type
/// parameter, and it is the whole reason a kit swap is not free.
#[derive(Debug, Default)]
pub struct KitHandoff {
    incoming: Mutex<Option<Arc<Kit>>>,
    spent: Mutex<Option<Arc<Kit>>>,
}

impl KitHandoff {
    /// Hand a finished kit to the audio thread. Loader/UI thread only —
    /// building it is what allocates, and that happens before this is called.
    pub fn send(&self, kit: Arc<Kit>) {
        if let Ok(mut slot) = self.incoming.lock() {
            *slot = Some(kit);
        }
    }

    /// Swap in whatever is waiting, parking what it replaces.
    ///
    /// **Audio thread.** `try_lock` for the same reason [`Handoff::receive`]
    /// uses one: a loader thread mid-write must never stall the callback.
    ///
    /// Answers **whether it swapped**, because the caller owes something on a
    /// swap that it does not owe otherwise. ⛔ The sampler holds pad *indices*
    /// inside its sounding voices, and the new kit may have a different pad at
    /// that index — so every voice has to be cut, or the note that was playing
    /// a hi-hat finishes as somebody's vocal chop.
    #[must_use = "a swapped kit means the sounding voices must be cut"]
    pub fn receive(&self, current: &mut Option<Arc<Kit>>) -> bool {
        let Ok(mut slot) = self.incoming.try_lock() else {
            return false;
        };
        let Some(next) = slot.take() else {
            return false;
        };
        drop(slot);

        let previous = current.replace(next);
        // Parked, not dropped: this may be the last reference, and freeing a
        // megabyte of samples here would take the allocator's lock.
        //
        // ⛔ **The `else` is not optional, and its absence was the exact bug
        // this type exists to prevent.** `collect()` runs on every editor
        // redraw, so `try_lock` genuinely loses races — and with no else branch
        // `previous` was simply dropped at the end of the function. On a second
        // assignment that `Arc` is the sole reference to a one-shot kit, so the
        // drop freed megabytes of `Arc<[f32]>` *inside the audio callback*.
        //
        // ⚠ **Leaking is the correct loser's move here.** A kit is ~1.2 MB, the
        // race is rare, and the plugin holds at most a handful over a session —
        // whereas taking the allocator's lock on the callback is a dropout the
        // producer hears. Bounded waste beats an audible failure.
        match self.spent.try_lock() {
            Ok(mut spent) => *spent = previous,
            Err(_) => std::mem::forget(previous),
        }
        true
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
    /// A rebuilt preview kit on its way to the audio thread (TASK-131B).
    ///
    /// ⚠ An `Arc` where [`Self::handoff`] is a plain field, because a kit is
    /// sent from a *detached loader thread* rather than from the bridge — that
    /// thread outlives the call that started it and so has to own a handle.
    pub kits: Arc<KitHandoff>,
    /// The one-shots the producer has assigned, and the dialog that assigns
    /// them (TASK-131B).
    ///
    /// ⛔ **Per instance, for the reason [`crate::export::Exports`] spells out
    /// at length** — its status mailbox is polled and taken destructively, so a
    /// process global would let one instance in a twenty-track session steal
    /// another's result. The kit a producer builds on one track is also simply
    /// not the kit they want on the next.
    pub one_shots: crate::oneshot::OneShots,
    /// The sample browser (TASK-132).
    ///
    /// ⚠ Per instance, for the same reason `one_shots` is: the folder a
    /// producer is working from on one track is not the one they want on the
    /// next, and a process global would let one instance move another's view.
    pub explorer: crate::explorer::Explorer,
    /// The File Explorer's audition voice (TASK-132).
    pub preview: crate::preview::Preview,
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
    /// The one prepared drag-out and its spooled files (TASK-063C).
    ///
    /// ⛔ **Per instance for the same reason `exports` is, and the consequence
    /// is worse here.** A shared slot would let instance B's `drag_start` hand
    /// the OS the files instance A prepared — the producer drops a loop from
    /// one track and gets a different track's loop, with nothing anywhere
    /// saying so.
    pub drags: crate::drag::Drags,
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
    /// Whether the clip repeats when it reaches its end (TASK-138).
    ///
    /// ⛔ **Read every block by the schedule, so it is atomic rather than a
    /// session field the audio thread would have to lock for.** Mike,
    /// 2026-08-06: *"can you have the 'Loop' button toggle off and on and either
    /// loop every time it plays to the end of the 4 or 8 bars or stop at the end
    /// of the 4 or 8 bars."*
    ///
    /// ⚠ **Defaults to on**, which is what the button has always claimed —
    /// `transport.loopAlways` read *"Playback always loops in this phase."*
    looping: AtomicBool,
    /// Lanes whose *audio* is muted, as a bitmask (FMM-S02).
    ///
    /// ⛔ The MIDI keeps flowing for a muted lane — that is the whole feature.
    /// It lets the plugin play the hats while the producer's own sampler in the
    /// DAW takes the snare, which muting the lane outright cannot express.
    muted_lanes: AtomicU64,
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
            kits: Arc::new(KitHandoff::default()),
            one_shots: crate::oneshot::OneShots::default(),
            explorer: crate::explorer::Explorer::default(),
            preview: crate::preview::Preview::default(),
            exports: crate::export::Exports::default(),
            drags: crate::drag::Drags::default(),
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
            looping: AtomicBool::new(true),
            muted_lanes: AtomicU64::new(0),
            playhead_bits: AtomicU32::new(0),
            seek_request: AtomicU32::new(0),
            audition_request: AtomicU32::new(0),
            standalone,
            // ⛔⛔ **FALSE IN BOTH SHELLS SINCE TASK-138, and the `!standalone`
            // that stood here would now be a live defect.** This used to be
            // `true` in a host because the gate was `host AND ours`: our term had
            // to be a constant that never decided, since a `false` would have
            // silenced a playing project.
            //
            // The gate is `host OR preview` now, so the same `true` would mean
            // **the preview is running from the moment the plugin loads** — the
            // pattern sounding continuously with the DAW stopped, which is
            // precisely the failure `process` records in its own comment
            // ("played the whole pattern out loud with the transport stopped").
            // An inverted default is not a tuning choice here; it is the
            // difference between a preview and a plugin that will not shut up.
            //
            // ⚠ The standalone's reason for `false` is unchanged and still
            // holds: cpal claims a running transport on every block, so a `true`
            // would bring the editor up already "playing" — Pause and Stop
            // offered for a pattern that does not exist, and a 30 Hz playhead
            // poll on an idle window.
            running: AtomicBool::new(false),
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

    /// Ask the audio thread to sound one **drum lane** (TASK-043).
    ///
    /// ⛔⛔ **The lane travels as its index in [`ALL_LANES`], and the first cut
    /// sent its General MIDI note instead — which was a real bug.** That form
    /// assumed `gm_drum_note` was injective, and it is not: `Sub`, `SubLow`,
    /// `Melody`, `Counter`, `Bass` and `Chords` all answer `0` because they
    /// carry their own pitch. `sub` and `subLow` are *both* rows in the drum
    /// grid, so clicking either header sounded whichever pitched pad the kit
    /// happened to list first. An index assumes nothing, and `ALL_LANES` is
    /// already the canonical enumeration the mute mask is read back through.
    ///
    /// A lane the list does not hold is refused rather than sent as index 0,
    /// which would audition the kick.
    pub fn request_lane_audition(&self, lane: Lane) {
        let Some(index) = ALL_LANES.iter().position(|known| *known == lane) else {
            return;
        };
        self.audition_request.store(
            (index as u32) | AUDITION_PENDING | AUDITION_DRUM,
            Ordering::Relaxed,
        );
    }

    /// Take a pending audition, if there is one. `process` is the caller.
    ///
    /// [`Audition::Pitch`] carries a MIDI note; [`Audition::Lane`] carries a
    /// lane. See [`AUDITION_DRUM`] for why the two cannot share a resolution
    /// rule, and [`Self::request_lane_audition`] for why the lane is not a note.
    pub fn take_audition(&self) -> Option<Audition> {
        let packed = self.audition_request.swap(0, Ordering::Relaxed);
        if packed & AUDITION_PENDING == 0 {
            return None;
        }
        if packed & AUDITION_DRUM == 0 {
            return Some(Audition::Pitch((packed & 0x7f) as u8));
        }
        // ⚠ A payload past the end of the list cannot happen — only
        // `request_lane_audition` writes one — but answering `None` is better
        // than indexing, because a panic here is on the audio thread.
        ALL_LANES
            .get((packed & 0x7f) as usize)
            .copied()
            .map(Audition::Lane)
    }

    /// Whether our own transport is running (TASK-041T). Read every block.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Start or hold our own **preview** transport (TASK-138).
    ///
    /// ⛔⛔ **The standalone gate that used to be here is GONE, deliberately,
    /// and the old reasoning must not be re-applied.** It read: *"a host owns
    /// whether time runs; letting one reach this would hand a DAW a second
    /// transport that can silence the plugin permanently."* That is right about
    /// the **host's timeline** and wrong about **auditioning**. Mike, 2026-08-04:
    /// *"i do not want to just use Ableton's transpose play button."* A producer
    /// choosing a beat wants to hear the loop without arming a track and rolling
    /// the whole project, which is what every comparable plugin offers.
    ///
    /// ▶ **What keeps it from being a second transport the DAW cannot move:**
    /// `lib.rs`'s gate is `host_playing || preview`, and the host **wins** — when
    /// its transport starts, `process` clears this flag on the same block. So
    /// the two can never both drive the schedule, and a DAW that starts rolling
    /// always takes it back. The failure the old gate feared — a plugin stuck
    /// silent because our flag said stop — cannot happen, because our flag can
    /// only ever *add* playback in a host, never subtract it.
    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }

    /// Whether the preview sampler may sound (FMM-S02). Read every block.
    pub fn audio_enabled(&self) -> bool {
        self.audio_enabled.load(Ordering::Relaxed)
    }

    pub fn set_audio_enabled(&self, on: bool) {
        self.audio_enabled.store(on, Ordering::Relaxed);
    }

    /// Whether the clip repeats at its end (TASK-138). Read every block.
    pub fn looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    /// ⛔ **Not gated on the standalone, unlike [`set_running`].** Looping is a
    /// property of *our* clip inside *our* schedule, not a claim on the host's
    /// timeline — a DAW that is rolling still expects a plugin's own loop to
    /// turn over. That is exactly the distinction `set_running`'s comment draws
    /// and the reason it does not apply here.
    pub fn set_looping(&self, on: bool) {
        self.looping.store(on, Ordering::Relaxed);
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

    /// Mute and solo, combined into the one mask the audio thread reads
    /// (TASK-043).
    ///
    /// ⛔ **Solo silences every lane it does not name, and it cannot un-mute
    /// one.** Both halves matter. Soloing the hats has to quieten the kick that
    /// was never muted — that is what solo *is* — and it must not bring back a
    /// lane the producer deliberately muted to route their own sampler into
    /// (FMM-S02), because then un-soloing would leave that lane audible and the
    /// mute button lit. Mute wins, always.
    ///
    /// An empty solo set means "no solo", not "solo nothing": a rule that
    /// silenced everything would make the first click of S mute the whole
    /// preview.
    ///
    /// ⚠ **The "everything else" term is derived from [`ALL_LANES`]**, the same
    /// list [`Self::muted_lanes`] reads the mask back through. A hand-written
    /// bit range would leave the newest lane audible through a solo, which is
    /// the quietest possible way for this to be wrong.
    ///
    /// ⛔⛔ **…but only the kit's share of it, and that was the bug.** Solo is
    /// offered on one surface — the drum grid's rows — and taking "everything
    /// else" to mean the whole of `ALL_LANES` meant soloing the snare also
    /// silenced the melody, countermelody, bass and chords. Nothing in those
    /// four editors shows a mute or a solo, so the producer switched to the
    /// Melody tab, pressed play, and heard nothing with no visible cause and no
    /// control to undo it. A solo silences what it is a solo *among*.
    pub fn set_lane_audio(&self, muted: &[Lane], soloed: &[Lane]) {
        let bits = |lanes: &[Lane]| lanes.iter().fold(0, |mask, lane| mask | lane_bit(*lane));
        let mut mask = bits(muted);
        if !soloed.is_empty() {
            let kept = bits(soloed);
            mask |= ALL_LANES
                .iter()
                .filter(|lane| !MELODIC_LANES.contains(lane))
                .map(|lane| lane_bit(*lane))
                .filter(|bit| bit & kept == 0)
                .fold(0, |mask, bit| mask | bit);
        }
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
        self.set_lane_audio(&session.muted_lanes, &session.soloed_lanes);
    }

    /// Reload the one-shots a restored project asked for (TASK-131B).
    ///
    /// ⛔ **Separate from [`Self::adopt_session`] because it does I/O.** That
    /// one is four atomic stores and is safe anywhere; this one reads and
    /// decodes files off disk, so it is `initialize`-only and never on a path a
    /// host might call at speed.
    ///
    /// A path that no longer resolves is **logged and skipped**, not fatal: a
    /// producer who moved their sample folder must still get their project
    /// back, with four of five one-shots and a line saying which one is gone.
    /// The lane falls back to the shipped voice, which is audibly different and
    /// therefore self-explaining once they look at the panel.
    pub fn restore_one_shots(&self) {
        let stored = crate::state::with(&self.session, |s| s.one_shots.clone()).unwrap_or_default();
        // ⛔ **Read alongside the paths, or a reversed one-shot reloads
        // forwards.** The reversal is applied at decode time, so it is not
        // recoverable from the file — see `PluginSession::one_shots_reversed`.
        let reversed =
            crate::state::with(&self.session, |s| s.one_shots_reversed.clone()).unwrap_or_default();
        for (lane, path) in stored {
            let backwards = reversed.get(&lane).copied().unwrap_or(false);
            if let Err(reason) =
                self.one_shots
                    .restore(lane, &path, backwards, &self.kits, &self.session)
            {
                nih_plug::nih_log!(
                    "the one-shot saved for {lane:?} could not be reloaded: {reason}"
                );
            }
        }
    }

    /// Put the saved sample-library folders back (TASK-132).
    ///
    /// ⚠ **A folder that no longer resolves is kept rather than skipped**,
    /// which is the opposite of `restore_one_shots` above and deliberately so.
    /// A one-shot that will not load has an audible fallback — the shipped
    /// voice — so dropping it costs the producer nothing they cannot see. A
    /// *library folder* has no fallback: silently forgetting it because an
    /// external drive was unplugged means they have to remember what was in the
    /// list, and there is nowhere to look it up.
    /// ⛔⛔ **Two sources, and the machine's is the one that makes the standalone
    /// work.** Mike, 2026-08-10: *"the folders should persist between app
    /// openings."* `sample_folders` lives in [`crate::state::PluginSession`],
    /// which the **host** writes into the project file — so it came back with an
    /// `.als` and never with a standalone launch, which has no project. The
    /// library is now also written per user (`explorer::saved_folders`), the way
    /// `eula.rs` already argued a per-person fact should be.
    ///
    /// ⚠ **Merged rather than one replacing the other.** Dropping the project's
    /// copy would lose the library of every project saved before this; dropping
    /// the machine's would put the bug straight back. The project's come first
    /// because a producer who opened *this song* most likely wants its folders at
    /// the front, and `Explorer::restore` keeps the first `MAX_ROOTS`.
    pub fn restore_sample_folders(&self) {
        let from_project =
            crate::state::with(&self.session, |s| s.sample_folders.clone()).unwrap_or_default();
        let folders =
            crate::explorer::merge_folders(from_project, crate::explorer::saved_folders());
        self.explorer.restore(&folders);
    }

    /// Write the sample library into the session, so the host saves it.
    pub fn store_sample_folders(&self) {
        let folders = self.explorer.snapshot();
        crate::state::update(&self.session, |session| {
            session.sample_folders = folders;
        });
    }

    /// Ask the producer for a sample and play it on `lane` (TASK-131B).
    ///
    /// ⛔ **Here rather than at the call site, so the three things an
    /// assignment touches cannot come apart.** It needs the kit handoff to
    /// reach the audio thread and the session store to survive a reopen, and a
    /// bridge command that passed one and forgot the other would be an
    /// assignment that plays and is never saved — or is saved and never heard.
    pub fn assign_one_shot(&self, lane: Lane) -> Result<(), String> {
        self.one_shots.assign(lane, &self.kits, &self.session)
    }

    /// The kit the preview is playing right now, one-shots included.
    ///
    /// ⚠ **Resolved from this instance's own session**, so two tracks on two
    /// artists get two kits. A process-global would be the same mistake
    /// `one_shots` documents at its own field.
    /// ⛔ **The producer's per-pad edits travel with it** (TASK-055A/164). This
    /// is the kit an *export* renders through — see
    /// [`crate::oneshot::OneShots::current_kit`] — and a stem that ignored the
    /// envelope and the trim a producer had just dialled in would be the same
    /// defect that doc records for one-shots, one control further along: heard
    /// one way in the preview, written another way to disk.
    pub fn current_kit(&self) -> Option<Arc<Kit>> {
        let (model_id, tweaks) = crate::state::with(&self.session, |s| {
            (s.selected_id.clone(), s.pad_tweaks.clone())
        })
        .unwrap_or_default();
        self.one_shots
            .current_kit(&model_id.unwrap_or_default(), &tweaks)
    }

    /// Replace one pad's edits and rebuild the kit the audio thread is playing.
    ///
    /// ⚠ **Persisted and handed over in one call**, the same coupling
    /// [`Self::assign_one_shot`] exists for: a bridge command that stored the
    /// tweak without rebuilding would show the producer a control that moved and
    /// changed no sound.
    pub fn set_pad_tweaks(&self, lane: Lane, tweaks: crate::pad_tweaks::PadTweaks) {
        let tweaks = tweaks.clamped();
        crate::state::update(&self.session, |stored| {
            // ⛔ **Removed rather than stored when it is the identity**, so
            // returning a control to its default leaves no trace in the project
            // file — and `pad_tweaks` stays absent from every project nobody has
            // edited a pad in.
            if tweaks.is_identity() {
                stored.pad_tweaks.remove(&lane);
            } else {
                stored.pad_tweaks.insert(lane, tweaks);
            }
        });
        self.one_shots.rebuild(&self.kits, &self.session);
    }

    /// Put a lane back on the shipped voice.
    pub fn clear_one_shot(&self, lane: Lane) {
        self.one_shots.clear(lane, &self.kits, &self.session);
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

/// What the editor asked to hear (TASK-043).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audition {
    /// A MIDI note, from the roll's keyboard gutter. Resolves to the nearest
    /// *tuned* pad.
    Pitch(u8),
    /// One drum lane, from the grid's row header. Resolves by lane, exactly.
    Lane(Lane),
}

/// Marks an audition as a **drum lane's** rather than a pitch's (TASK-043).
///
/// ⛔ **The two resolve to different pads and cannot share a rule.** A pitch
/// audition finds the nearest *tuned* pad, because the question a producer asks
/// by clicking the keyboard gutter is "what does this note sound like" and the
/// answer has to track the register. A lane audition is asking to hear one
/// specific voice — the kick, that kick — so it resolves by lane and must never
/// fall back to the nearest anything. Without the distinction, clicking `Kick`
/// would have sounded the 808 transposed to the kick's GM note, which is the
/// "a kick pitched up forty semitones is not a preview of anything" failure
/// `audition` already warns about, arrived at from the other direction.
const AUDITION_DRUM: u32 = 1 << 30;

/// Every lane, so the mask can be read back out as names.
///
/// ⚠ **Also what the KIT panel enumerates** (TASK-131B). It is `pub(crate)`
/// rather than private for that reason and only that reason: a second list of
/// lanes in `editor.rs` would be a second thing to remember on the day the
/// engine gains one, and the panel would quietly stop offering it.
pub(crate) const ALL_LANES: &[Lane] = &[
    Lane::Kick,
    Lane::SubKick,
    Lane::Snare,
    Lane::OffSnare,
    Lane::GhostSnare,
    Lane::Clap,
    Lane::ClosedHat,
    Lane::OpenHat,
    Lane::PedalHat,
    Lane::Ride,
    Lane::RideBell,
    Lane::Crash,
    Lane::Tom,
    Lane::TomHigh,
    Lane::TomLow,
    Lane::Rim,
    Lane::Snap,
    Lane::Perc,
    Lane::Perc2,
    Lane::Shaker,
    Lane::Tambourine,
    Lane::Cowbell,
    Lane::Clave,
    Lane::Conga,
    Lane::Bongo,
    Lane::Timbale,
    Lane::Triangle,
    Lane::Woodblock,
    Lane::Riser,
    Lane::Impact,
    Lane::Reverse,
    Lane::Sub,
    Lane::SubLow,
    Lane::Melody,
    Lane::Counter,
    Lane::Bass,
    Lane::Chords,
];

/// The four lanes that are whole generators rather than rows of the kit.
///
/// ⛔ **Not "the pitched lanes".** `Sub` and `SubLow` are pitched too, and they
/// are drawn as rows of the drum grid with their own mute, solo and padlock —
/// so a drum solo is a solo among them. What separates these four is that they
/// each have their own editor with no mute or solo control on it at all, which
/// is what makes silencing them from the drum grid unexplainable on screen.
pub(crate) const MELODIC_LANES: &[Lane] = &[Lane::Melody, Lane::Counter, Lane::Bass, Lane::Chords];

/// This lane's bit in the mute mask.
///
/// Written out rather than cast from the enum's discriminant, which `Lane` does
/// not expose anyway. ⚠ **The bits are process-local and never reach a file** —
/// `PluginSession.muted_lanes` is a `Vec<Lane>` and `Lane` serializes *by name*
/// (`engine/src/pattern.rs`), so a reordered enum cannot remap anything saved.
/// An earlier version of this comment claimed the opposite and used it to
/// justify the table; the table is still the clearer form, but it is a
/// readability choice rather than a persistence guarantee.
/// ⛔⛔ **A `u64`, and it had to become one.** TASK-043A took the vocabulary
/// past 32 lanes, so a `u32` mask could not hold the kit any more — and the
/// failure mode of overflowing it is the worst kind this file has: `1 << 33`
/// wraps to bit 1, so muting a conga would have silenced the snare, quietly,
/// on the audio thread. `every_lane_has_its_own_bit` is what would have caught
/// it, and widening the word is what makes it pass.
fn lane_bit(lane: Lane) -> u64 {
    match lane {
        Lane::Kick => 1 << 0,
        Lane::Snare => 1 << 1,
        Lane::Clap => 1 << 2,
        Lane::ClosedHat => 1 << 3,
        Lane::OpenHat => 1 << 4,
        Lane::Rim => 1 << 5,
        Lane::Snap => 1 << 6,
        Lane::Perc => 1 << 7,
        Lane::Sub => 1 << 8,
        Lane::Melody => 1 << 9,
        Lane::Counter => 1 << 10,
        Lane::Bass => 1 << 11,
        Lane::Chords => 1 << 12,
        // ⚠ Appended at 13 rather than slotted into kit order, so no existing
        // lane's bit moves. Nothing persists these — the comment above explains
        // why that is safe either way — but a mask in flight between the editor
        // and the audio thread during a reload should not have to be right
        // about which build wrote it.
        Lane::OffSnare => 1 << 13,
        Lane::Ride => 1 << 14,
        Lane::Crash => 1 << 15,
        Lane::Tom => 1 << 16,
        Lane::Shaker => 1 << 17,
        Lane::Tambourine => 1 << 18,
        Lane::Cowbell => 1 << 19,
        Lane::Woodblock => 1 << 20,
        // ── TASK-043A, appended for the reason the note above gives ──────
        Lane::SubKick => 1 << 21,
        Lane::GhostSnare => 1 << 22,
        Lane::PedalHat => 1 << 23,
        Lane::RideBell => 1 << 24,
        Lane::TomHigh => 1 << 25,
        Lane::TomLow => 1 << 26,
        Lane::Perc2 => 1 << 27,
        Lane::Clave => 1 << 28,
        Lane::Conga => 1 << 29,
        Lane::Bongo => 1 << 30,
        Lane::Timbale => 1 << 31,
        Lane::Triangle => 1 << 32,
        Lane::Riser => 1 << 33,
        Lane::Impact => 1 << 34,
        Lane::Reverse => 1 << 35,
        Lane::SubLow => 1 << 36,
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

    /// A kit with one nameable pad, so a swap can be told from a no-op.
    fn kit_named(id: &str) -> Arc<Kit> {
        Arc::new(Kit {
            id: id.to_owned(),
            pads: Vec::new(),
        })
    }

    #[test]
    fn an_empty_kit_handoff_leaves_the_current_kit_alone() {
        let kits = KitHandoff::default();
        let mut current = Some(kit_named("shipped"));
        assert!(!kits.receive(&mut current), "nothing was waiting");
        assert_eq!(current.as_ref().unwrap().id, "shipped");
    }

    #[test]
    fn a_sent_kit_is_swapped_in_and_reported_as_a_swap() {
        // ⛔ The bool is what makes `process` cut its voices. A swap that
        // reported `false` would leave every sounding voice addressing a pad
        // index in the *old* kit — a hi-hat finishing as somebody's vocal chop.
        let kits = KitHandoff::default();
        let mut current = Some(kit_named("shipped"));
        kits.send(kit_named("with-one-shots"));

        assert!(kits.receive(&mut current), "a swap must report itself");
        assert_eq!(current.as_ref().unwrap().id, "with-one-shots");
    }

    #[test]
    fn the_replaced_kit_is_parked_rather_than_freed_on_the_audio_thread() {
        // ⛔ **The rule this whole type exists for.** A kit is megabytes of
        // samples, and dropping the last reference to it inside the callback
        // takes the allocator's lock — the one thing a process callback must
        // never do. It has to stay alive until the editor thread collects it.
        let kits = KitHandoff::default();
        let mut current = Some(kit_named("shipped"));
        let outgoing = Arc::downgrade(current.as_ref().unwrap());
        kits.send(kit_named("with-one-shots"));

        assert!(kits.receive(&mut current));
        assert!(
            outgoing.upgrade().is_some(),
            "the outgoing kit was freed on the audio thread"
        );

        kits.collect();
        assert!(
            outgoing.upgrade().is_none(),
            "the editor thread must be what actually frees it"
        );
    }

    #[test]
    fn a_second_assignment_replaces_an_unclaimed_first() {
        // Assigning twice before the audio thread looked means the producer
        // wanted the second sample — the same rule `Handoff` follows for a
        // second Generate, and for the same reason.
        let kits = KitHandoff::default();
        kits.send(kit_named("first"));
        kits.send(kit_named("second"));

        let mut current = Some(kit_named("shipped"));
        assert!(kits.receive(&mut current));
        assert_eq!(current.as_ref().unwrap().id, "second");
        assert!(!kits.receive(&mut current), "one receive drains the slot");
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
        for lane in [Lane::Kick, Lane::ClosedHat, Lane::Sub, Lane::Chords] {
            assert!(!shared.lane_muted(lane), "{lane:?} should be untouched");
        }

        shared.set_lane_muted(Lane::Snare, false);
        assert!(!shared.lane_muted(Lane::Snare));
    }

    #[test]
    fn every_lane_has_its_own_bit() {
        // Two lanes sharing a bit would mute in pairs, which is the kind of
        // mistake a hand-written mapping exists to make findable.
        //
        // ⛔ **`u64`, and TASK-043A is why it had to change.** The vocabulary
        // went past 32 lanes, and `1 << 33` on a `u32` wraps to bit 1 — so
        // muting a conga would have silenced the snare. Widening the
        // accumulator is not cosmetic: a `u32` here would go on passing while
        // `lane_bit` overflowed, because both sides would wrap together.
        let mut seen = 0u64;
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
        shared.set_muted_lanes(&[Lane::OpenHat, Lane::Sub, Lane::Chords]);

        let mut back = shared.muted_lanes();
        back.sort();
        let mut expected = vec![Lane::OpenHat, Lane::Sub, Lane::Chords];
        expected.sort();
        assert_eq!(back, expected);

        // Replacing the set clears what was there rather than adding to it.
        shared.set_muted_lanes(&[Lane::Kick]);
        assert_eq!(shared.muted_lanes(), vec![Lane::Kick]);
    }

    #[test]
    fn solo_silences_every_other_row_of_the_kit_and_no_whole_generator() {
        // TASK-043. Soloing the closed hat has to quieten the kick that nobody
        // muted — that is what solo *is* — and it must reach every drum lane in
        // `ALL_LANES`, not only the ones a kit happens to hold.
        //
        // ⛔⛔ **This test used to say "every lane" and that was the bug.** It
        // walked the whole of `ALL_LANES`, so it *required* the melody,
        // countermelody, bass and chords to go silent when a producer soloed a
        // drum row — with no mute or solo control in any of those four editors
        // to show why, and none to undo it. A gate can be wrong about what the
        // right answer is, and this one was.
        let shared = Shared::default();
        shared.set_lane_audio(&[], &[Lane::ClosedHat]);

        assert!(
            !shared.lane_muted(Lane::ClosedHat),
            "the soloed lane sounds"
        );
        for lane in ALL_LANES
            .iter()
            .filter(|lane| **lane != Lane::ClosedHat && !MELODIC_LANES.contains(lane))
        {
            assert!(shared.lane_muted(*lane), "{lane:?} should be soloed away");
        }
        for lane in MELODIC_LANES {
            assert!(
                !shared.lane_muted(*lane),
                "{lane:?} has no solo control of its own, so a drum solo must not silence it"
            );
        }
        // ⚠ The 808 and its sub layer *are* rows of the grid, with their own
        // mute and solo, so they go quiet with the rest of the kit.
        assert!(shared.lane_muted(Lane::Sub));
        assert!(shared.lane_muted(Lane::SubLow));
    }

    #[test]
    fn an_empty_solo_set_means_no_solo_rather_than_solo_nothing() {
        // The distinction the first click of S depends on: a rule that read an
        // empty set as "keep nothing" would mute the whole preview the moment
        // solo was switched off again.
        let shared = Shared::default();
        shared.set_lane_audio(&[Lane::Snare], &[]);

        assert!(shared.lane_muted(Lane::Snare));
        for lane in ALL_LANES.iter().filter(|lane| **lane != Lane::Snare) {
            assert!(!shared.lane_muted(*lane), "{lane:?} should still sound");
        }
    }

    #[test]
    fn a_mute_survives_a_solo_that_names_the_muted_lane() {
        // ⛔ **Mute wins.** A producer mutes our kick because they routed the
        // MIDI into their own sampler (FMM-S02); if solo could un-mute it, the
        // moment they soloed the kick to check it they would hear ours *and*
        // theirs — and switching solo off again would leave it audible with the
        // mute button still lit.
        let shared = Shared::default();
        shared.set_lane_audio(&[Lane::Kick], &[Lane::Kick, Lane::Snare]);

        assert!(shared.lane_muted(Lane::Kick), "the mute outranks the solo");
        assert!(!shared.lane_muted(Lane::Snare));
        assert!(shared.lane_muted(Lane::ClosedHat));
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
    /// ⛔⛔ **INVERTED at TASK-138.** This asserted that a host *could not* reach
    /// `set_running` — the backstop for "a DAW owns whether time runs". The
    /// plugin drives its own **preview** transport now, so a host must be able
    /// to move it. `Shared::set_running` records why the old reasoning does not
    /// apply, and `lib.rs`'s gate is what keeps the two from colliding.
    fn a_host_drives_its_own_preview_transport() {
        let shared = Shared::default();
        assert!(!shared.standalone);
        // ⛔ Stopped on load, in a host as well as the standalone. A `true` here
        // and the gate `host || preview` would sound the pattern the moment the
        // plugin was inserted — see `Shared::new`.
        assert!(!shared.running(), "a freshly loaded plugin is not playing");

        shared.set_running(true);
        assert!(shared.running(), "a host may start its own preview");

        shared.set_running(false);
        assert!(!shared.running(), "and may hold it again");
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
        assert_eq!(shared.take_audition(), Some(Audition::Pitch(0)));
    }

    #[test]
    fn taking_an_audition_clears_it() {
        // A request that stayed set would re-trigger on every block — the note
        // would not sound once, it would machine-gun for as long as the editor
        // was open. This is the same failure `fired.clear()` guards in `process`.
        let shared = Shared::default();
        shared.request_audition(64);

        assert_eq!(shared.take_audition(), Some(Audition::Pitch(64)));
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

        assert_eq!(shared.take_audition(), Some(Audition::Pitch(72)));
    }

    #[test]
    fn a_lane_audition_is_told_apart_from_a_pitch_one() {
        // ⛔ The two resolve to different pads (see `AUDITION_DRUM`), so the
        // flag is the whole mechanism: without it, clicking `Kick` would reach
        // `audition`'s nearest-tuned-pad rule and sound the 808 pitched to the
        // kick's GM note.
        let shared = Shared::default();

        shared.request_lane_audition(Lane::Kick);
        assert_eq!(shared.take_audition(), Some(Audition::Lane(Lane::Kick)));

        // And the flag does not leak into the next request through the shared
        // word — a pitch audition after a lane one is still a pitch audition.
        shared.request_audition(36);
        assert_eq!(shared.take_audition(), Some(Audition::Pitch(36)));
    }

    #[test]
    fn a_lane_audition_survives_lanes_that_share_a_gm_note() {
        // ⛔⛔ **The regression this exists for.** The first cut sent the lane's
        // General MIDI note, which assumed `gm_drum_note` was injective — and
        // it is not: `Sub`, `SubLow`, `Melody`, `Counter`, `Bass` and `Chords`
        // all answer 0, because they carry their own pitch. `sub` and `subLow`
        // are *both* rows in the drum grid, so clicking either header asked for
        // note 0 and sounded whichever pitched pad the kit happened to list
        // first. The lane travels as an index now, which assumes nothing.
        let shared = Shared::default();

        shared.request_lane_audition(Lane::Sub);
        assert_eq!(shared.take_audition(), Some(Audition::Lane(Lane::Sub)));

        shared.request_lane_audition(Lane::SubLow);
        assert_eq!(shared.take_audition(), Some(Audition::Lane(Lane::SubLow)));

        // And every lane, so this cannot regress for one while passing for the
        // two named above.
        for lane in ALL_LANES {
            shared.request_lane_audition(*lane);
            assert_eq!(
                shared.take_audition(),
                Some(Audition::Lane(*lane)),
                "{lane:?} did not survive the round trip"
            );
        }
    }

    #[test]
    fn an_out_of_range_audition_is_clamped_rather_than_wrapping() {
        // The pitch shares its word with the pending flag, so a value past 127
        // that was stored raw would corrupt the flag rather than merely being
        // wrong — the request would read back as a different note, or as none.
        let shared = Shared::default();
        shared.request_audition(200);

        assert_eq!(shared.take_audition(), Some(Audition::Pitch(127)));
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
