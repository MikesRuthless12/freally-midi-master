//! Freally MIDI Master as a plugin.
//!
//! The whole reason this exists rather than the desktop app: a plugin is handed
//! the host's tempo, time signature and playhead in its process callback. There
//! is no protocol to negotiate, no MIDI cable and no discovery — the DAW's
//! session *is* the session, and a generated pattern lands on the track in the
//! project's own key and tempo.
//!
//! One plugin, three formats. This crate exports **CLAP** and **VST3**
//! directly through `nih-plug`; **AUv2 and AUv3** are projected from the CLAP
//! by `clap-wrapper` at packaging time. Nothing here is format-specific, and
//! nothing here should become so.
//!
//! [`engine`] is untouched by any of this. It has no Tauri types, no network
//! and no plugin types either — it takes a [`StyleModel`] and a
//! [`SessionContext`] and returns notes. This crate is the second consumer of
//! it, not a rewrite of it.

use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

pub mod audio;
pub mod bridge;
pub mod dataset;
pub mod drag;
mod editor;
pub mod eula;
pub mod explorer;
pub mod export;
pub mod favourites;
pub mod host;
pub mod kits;
pub mod models;
pub mod oneshot;
pub mod patterns;
pub mod presets;
pub mod preview;
pub mod roles;
pub mod shared;
pub mod state;
pub mod voice;

pub use bridge::{dispatch, Request};
pub use host::HostSession;
pub use shared::{Shared, SharedState};
pub use state::{PluginSession, SessionStore};
pub use voice::Schedule;

/// Whether this process is our own standalone binary rather than a DAW.
///
/// ⛔ **Set by `main`, which a host never runs** — the same gate
/// `own_message_queue` uses, and for the same reason: a library loaded into
/// Ableton has no `main`, so there is no way for a host to trip this by
/// accident. Do not infer the standalone from a behavioural quirk instead
/// ("the host sends no frame ticks" was tried and is a quirk, not an identity).
static STANDALONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Declare that this process is the standalone. Called once, from its `main`.
pub fn mark_standalone() {
    STANDALONE.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// True when there is no host — so the transport is ours to run (TASK-041T).
pub fn is_standalone() -> bool {
    STANDALONE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The GM note a drum lane sounds at. One hop to the engine, so the audio
/// module has a single name for it rather than reaching across inline.
pub fn midi_note_for(lane: engine::pattern::Lane) -> u8 {
    engine::midi::gm_drum_note(lane)
}

/// The plugin's own state, held across the process callbacks.
pub struct FreallyMidiMaster {
    params: Arc<FreallyParams>,
    /// What the host said about tempo and meter last time we looked, so a
    /// change can be *noticed* rather than merely read.
    session: HostSession,
    /// Notes waiting to be emitted, in host time. Armed on the UI thread and
    /// handed over whole; drained by `process`.
    pending: Schedule,
    /// The two-way channel with the editor.
    shared: SharedState,
    /// The preview sampler's voices, and the note-ons of the current block.
    ///
    /// Both live on the plugin rather than inside `process` because both must
    /// exist without being allocated on the audio thread: voices persist across
    /// blocks by definition, and the fired-note array is reused every block.
    sampler: audio::sampler::Sampler,
    /// The kit the preview plays right now.
    ///
    /// ⛔ **Owned here rather than reached for statically, because since
    /// TASK-131B it can change.** It starts as the shipped kit and is replaced,
    /// whole, whenever the producer assigns one of their own one-shots —
    /// `shared.kits` is the handoff and `process` is the only place the swap
    /// happens. `None` only while the shipped kit failed to decode, which
    /// `audio::preview_kit` already logs.
    kit: Option<Arc<audio::kit::Kit>>,
    fired: voice::FiredNotes,
    /// Whether the host's transport was rolling on the previous block.
    ///
    /// ⛔ Only so the *start* of the host's playback can be noticed (TASK-138).
    /// The preview transport yields to the host on that edge; see the gate in
    /// `process`, which explains why an edge and not a level.
    host_was_playing: bool,
    /// Interleaved scratch the sampler renders into before it is added to the
    /// host's buffer.
    ///
    /// ⛔ Sized in `initialize`, never in `process`. nih-plug hands out
    /// per-channel slices and the sampler writes interleaved, so one of the two
    /// has to be converted — doing it through a buffer allocated once is what
    /// keeps the audio thread free of the allocator.
    scratch: Vec<f32>,
}

/// No automatable parameters, and one persisted field.
///
/// A generator driven by a roster, a seed and a set of pins has nothing a host
/// would sensibly draw a knob for or write automation against — the artist is
/// not a continuous value. What it does have is a *session*, and that is what
/// belongs in the project file.
///
/// `#[persist]` is how `nih-plug` carries arbitrary serde data through the
/// host's own state calls, which is what TASK-P07 asked for: no settings file,
/// no per-machine store, no path the plugin has to find. The DAW saves it with
/// the song and hands it back on open.
#[derive(Params, Default)]
pub struct FreallyParams {
    #[persist = "session"]
    pub session: SessionStore,
}

impl Default for FreallyMidiMaster {
    fn default() -> Self {
        // **One store, threaded into both.** `params` is what the host
        // serializes; `shared` is what the editor reads and writes. They must
        // be the same `Arc` — two stores would save one and display the other,
        // which presents as "the DAW did not save my session" and stays
        // invisible until someone reopens a project.
        let params = Arc::new(FreallyParams::default());
        let shared = Arc::new(Shared::new(params.session.clone()));

        Self {
            params,
            session: HostSession::default(),
            pending: Schedule::default(),
            shared,
            sampler: audio::sampler::Sampler::default(),
            // Decoded lazily by `initialize`, off the audio thread.
            kit: None,
            fired: voice::FiredNotes::default(),
            // ⚠ `false`, so a plugin loaded into an already-rolling project sees
            // the host start on its first block and takes the preview flag down
            // rather than letting a stale one through.
            host_was_playing: false,
            scratch: Vec::new(),
        }
    }
}

/// Whether the host's transport has just *started* (TASK-138).
///
/// ⛔ **An edge, not a level, and the difference is the whole design.** The
/// preview yields to the host so the two can never both drive the schedule —
/// but yielding on the *level* (`host_playing`) would clear the flag on every
/// block a DAW rolls, making Play unpressable while a project plays. On the
/// edge, the host takes it back once and the producer can start a preview again
/// afterwards.
fn host_takes_over(host_playing: bool, host_was_playing: bool) -> bool {
    host_playing && !host_was_playing
}

/// Whether the schedule advances this block.
///
/// ⛔⛔ **`host OR preview` in a plugin — reversed from `host AND ours` at
/// TASK-138**, which made Play inside a DAW impossible by construction.
/// `crate::shared::Shared::set_running` carries the full reasoning.
///
/// ⚠ **The standalone is a separate branch and must stay one.** nih-plug's cpal
/// backend hardcodes `transport.playing = true` on every block whatever the user
/// pressed, so `host_playing` is a constant `true` there — an `||` would make
/// Pause impossible.
///
/// ⚠ Extracted from `process` purely so it can be tested, the same reason
/// `editor::fit` is a free function: the rule is four lines and the failure it
/// prevents is a DAW that will not stop.
fn transport_runs(standalone: bool, host_playing: bool, preview: bool) -> bool {
    if standalone {
        preview
    } else {
        host_playing || preview
    }
}

#[cfg(test)]
mod transport_gate {
    use super::*;

    #[test]
    fn a_plugin_plays_when_the_host_does_even_with_no_preview() {
        // Unchanged from before TASK-138 and the case that must never regress:
        // a DAW rolling plays the pattern whatever our own flag says.
        assert!(transport_runs(false, true, false));
    }

    #[test]
    fn a_plugin_plays_its_preview_with_the_host_stopped() {
        // ⛔⛔ The whole of TASK-138. Mike, 2026-08-04: *"i do not want to just
        // use Ableton's transpose play button."* This was impossible before —
        // the gate was a conjunction, so a stopped host meant silence.
        assert!(transport_runs(false, false, true));
    }

    #[test]
    fn a_plugin_with_neither_is_silent() {
        assert!(!transport_runs(false, false, false));
    }

    #[test]
    fn the_standalone_ignores_the_backend_s_hardcoded_playing_flag() {
        // ⛔ cpal claims `playing = true` on every block. An `||` here would make
        // Pause do nothing at all, which is the defect the old conjunction was
        // written to avoid and this branch preserves.
        assert!(!transport_runs(true, true, false), "Pause must hold");
        assert!(transport_runs(true, true, true));
    }

    #[test]
    fn the_host_takes_the_transport_back_when_it_starts_and_only_then() {
        // ⛔ The safety property: exactly one of the two drives the schedule.
        // Leaving a preview running and then rolling the DAW would otherwise
        // sound the pattern twice.
        assert!(host_takes_over(true, false), "the block the host starts on");

        // ⚠ And not afterwards — otherwise Play could never be pressed again
        // while the project rolls.
        assert!(
            !host_takes_over(true, true),
            "already rolling is not an edge"
        );
        assert!(!host_takes_over(false, true), "stopping is not a takeover");
        assert!(!host_takes_over(false, false));
    }
}

impl Plugin for FreallyMidiMaster {
    /// What the host puts on the plugin's window and in its browser.
    ///
    /// The identity a DAW saves in a project is `CLAP_ID` and the VST3 class
    /// id, neither of which is this — so changing the display name never
    /// orphans a plugin already placed on someone's track.
    const NAME: &'static str = "Freally MIDI Master By: Mike Weaver";
    const VENDOR: &'static str = "Mike Weaver";
    const URL: &'static str = "https://github.com/MikesRuthless12/freally-midi-master";
    const EMAIL: &'static str = "mythodikalone@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// A silent stereo pass-through, even though this plugin makes notes
    /// rather than sound.
    ///
    /// The honest layout is no audio buses at all — and **Ableton Live refuses
    /// to open a VST3 that declares none**, with "This VST3 plug-in could not
    /// be opened" and nothing more specific. Live is not unusual here; the
    /// VST3 hosts generally expect at least one audio bus, and the MIDI-effect
    /// plugins that work in them declare a pass-through for exactly this
    /// reason.
    ///
    /// The audio is untouched: `process` writes nothing to the buffer, so
    /// whatever comes in goes out. What the plugin actually produces is note
    /// events on its MIDI output.
    ///
    /// Two layouts rather than one, so a host can insert it on a mono track
    /// without a channel-count negotiation failure — which presents as the
    /// same unhelpful message.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    /// The pattern is placed against the host's own timeline, so a note's
    /// position has to survive automation-block splitting.
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.shared.clone())
    }

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // The editor arms schedules in samples, so it needs the real rate
        // rather than a guess: 48 kHz assumed inside a 44.1 kHz session places
        // every note 8.8% late, which is a whole 16th out by bar four.
        self.shared.set_sample_rate(buffer_config.sample_rate);

        // ⛔ The one place the scratch may be sized. `max_buffer_size` is the
        // largest block the host promises to ask for, so allocating for it here
        // means `process` never has to grow anything. Two channels because the
        // sampler renders stereo; a mono layout takes the left of each frame.
        self.scratch
            .resize(buffer_config.max_buffer_size as usize * 2, 0.0);

        // ⛔ The host has already deserialized the project into the store by
        // now, and it never went near `save_session_state` — so this is where a
        // restored session reaches the atomics the audio thread reads. Without
        // it a project saved MIDI-only reopens making sound.
        self.shared.adopt_session();

        // Decode the preview kit now, off the audio thread. The first `process`
        // would otherwise be the first caller and would allocate inside the
        // host's callback.
        //
        // ⛔ **And take a reference to it, rather than only warming the cache.**
        // Since TASK-131B the callback plays whatever kit it was last handed,
        // so it needs one to start from — without this the plugin is silent
        // until the producer assigns a one-shot, which is the four-silent-parts
        // failure of TASK-131A wearing a different hat.
        // ⛔ **Only when there is nothing to keep.** This assigned
        // unconditionally, and nih-plug calls `initialize` on every buffer or
        // sample-rate change — so changing the buffer size threw away whatever
        // kit `process` had been handed and reverted the producer to the stock
        // samples. `restore_one_shots` below only puts them back if every path
        // still resolves; when one does not it returns early, no rebuilt kit is
        // sent, and the KIT panel goes on naming their files while they hear
        // the shipped ones. A readout that lies, in the panel built to end that.
        if self.kit.is_none() {
            self.kit = audio::preview_kit().cloned();
        }

        // ⛔ **The one-shots a reopened project asked for, reloaded here.** The
        // host has already deserialized the session by now (see
        // `adopt_session` above), so this is the first moment their paths are
        // known — and `initialize` is off the audio thread, which is where
        // reading files from disk has to happen.
        self.shared.restore_one_shots();
        // The sample library, from the project **and** from this machine
        // (TASK-132). ⚠ `initialize` is the standalone's path as well as the
        // plugin's, which is what makes *"the folders should persist between app
        // openings"* work where there is no project at all.
        self.shared.restore_sample_folders();
        true
    }

    fn reset(&mut self) {
        // The host has jumped or stopped. The playhead goes home, but the
        // pattern stays armed.
        //
        // ⛔ **`seek(0.0)`, not `clear()`.** Hosts call `reset` on every
        // transport relocation and re-activation, so clearing the events threw
        // the generated pattern away on the first stop — the producer pressed
        // play again and got silence, with nothing on screen to say why, and had
        // to press Generate for every pass of the transport.
        self.pending.seek(0.0);
        // Voices belong to notes that are no longer where we are. Letting an
        // 808 ring through a relocation is the audible half of the same bug.
        self.sampler.stop_all();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // The pivot's whole point, in two lines: the host tells us the tempo
        // and the meter, every block, for free.
        self.session.observe(context.transport());

        // ⛔ **Derived once, here, and both used and published from the same
        // value** (TASK-041T). `HostSession` stays a pure record of what the
        // host said — folding our own flag back into it gave `playing()` two
        // meanings seven lines apart, and made the second term of the gate below
        // unreachable.
        //
        // The page needs the *effective* answer: `host_session` is what gates
        // its playhead poll and enables Stop. In the standalone nih-plug's cpal
        // backend reports `transport.playing = true` on every block whatever the
        // user pressed, so publishing that raw left Pause looking like it did
        // nothing — the flag flipped back on the next 500 ms poll and the marker
        // carried on.
        // ⛔⛔ **`host OR preview` in a plugin, and the host WINS (TASK-138).**
        // This used to be `host AND ours`, which made Play inside a DAW
        // impossible by construction — `Shared::set_running` records why that
        // was reversed. A producer auditioning a beat must not have to arm a
        // track and roll the whole project.
        //
        // ⛔ **The host taking over is an *edge*, not a level.** Clearing our
        // flag whenever the host is playing would make Play unpressable while a
        // DAW rolls; clearing it on the transition is what stops the preview
        // outliving the host's own playback. After this, exactly one of the two
        // is driving the schedule, which is the property the old gate was
        // protecting and this keeps.
        let host_playing = self.session.playing();
        if host_takes_over(host_playing, self.host_was_playing) {
            self.shared.set_running(false);
        }
        self.host_was_playing = host_playing;

        let running = transport_runs(self.shared.standalone, host_playing, self.shared.running());
        self.shared.host.publish(&self.session, running);

        // Take a newly generated pattern if one is waiting. The schedule this
        // replaces is handed back rather than dropped — freeing its `Vec` here
        // would take the allocator's lock on the audio thread.
        self.pending = self
            .shared
            .handoff
            .receive(std::mem::take(&mut self.pending));

        // A kit the producer just assigned a one-shot into (TASK-131B).
        //
        // ⛔ **Every voice is cut on a swap, and it is not optional.** A voice
        // holds the *index* of the pad it is playing, and the new kit may have a
        // different pad at that index — so a note that started as a hi-hat would
        // finish as somebody's vocal chop. Cutting them is also what a producer
        // expects: changing the sound under a sounding note is not a crossfade.
        if self.shared.kits.receive(&mut self.kit) {
            self.sampler.stop_all();
        }

        // The keyboard gutter's click-to-audition (TASK-041).
        //
        // ⛔ **Before the transport gate, because an audition is not playback.**
        // A producer clicks a key to find a register *while the DAW is stopped*,
        // which is the normal case — putting this after the `!running` early
        // return would make the gutter silent exactly when it is most used.
        if let Some(asked) = self.shared.take_audition() {
            self.audition(asked);
        }

        // ⛔ **Nothing advances unless the host's transport is running.**
        // `process` is called continuously whenever the plugin is active, not
        // only while the DAW plays — so without this the schedule started
        // advancing the moment a generation landed and the preview sampler
        // played the whole pattern out loud with the transport stopped, while
        // `playback_status` was telling the same user to press play in their
        // DAW. The pattern stays armed and the playhead stays put; a seek is
        // still honoured, so clicking the grid while stopped moves the marker.
        //
        // `running` above is the conjunction: the host's transport *and* ours.
        // A DAW that is stopped stays stopped whatever our flag says, and the
        // standalone's cpal backend hardcodes `transport.playing = true`, so its
        // pause has to live somewhere the backend does not overwrite.
        if !running {
            if let Some(progress) = self.shared.take_seek() {
                self.pending.seek(progress);
                self.sampler.stop_all();
            }
            self.shared.set_playhead(self.pending.progress());
            // ⛔ `emit` is what normally clears this, and it did not run. Stale
            // entries would be re-triggered on every callback, so a stopped
            // transport would machine-gun the last block of notes forever.
            self.fired.clear();
            // Voices already sounding are allowed to finish rather than being
            // cut mid-tail, which is what a transport stop sounds like on real
            // hardware; nothing new is triggered.
            self.render_preview(buffer);
            return ProcessStatus::Normal;
        }

        // ⛔ Seek *before* emitting, never after (TASK-041T). Emitting first
        // would play this block at the old position and only then jump, so a
        // click on the timeline is heard as a stutter before it takes effect.
        if let Some(progress) = self.shared.take_seek() {
            self.pending.seek(progress);
            // Voices belong to notes before the new position. Letting an 808
            // ring across a seek is the same bug as letting it ring across a
            // host relocation, which `reset` already handles.
            self.sampler.stop_all();
        }

        // ⛔ The block length is what advances the schedule. Passing it is not
        // bookkeeping: without it `emit` replays the first block forever and
        // nothing past ~170 ms of a pattern is ever heard.
        self.pending.emit(
            context,
            buffer.samples() as u32,
            self.shared.looping(),
            &mut self.fired,
        );

        // Where the marker goes (TASK-041T). Published every block, so it moves
        // with the tempo for free: the schedule is placed in samples against the
        // host's own BPM, so a tempo change moves the notes and the marker
        // together rather than letting them drift apart.
        self.shared.set_playhead(self.pending.progress());

        // The preview kit, played at the same sample offsets the host track
        // just received (TASK-P17). Silent when the kit failed to decode.
        self.render_preview(buffer);

        ProcessStatus::Normal
    }
}

impl FreallyMidiMaster {
    /// Play this block's note-ons through the preview kit, into the host's out.
    ///
    /// ⛔ **Rendered in segments split at each note-on, not once per block.**
    /// Triggering everything at the block boundary would quantise every hit to
    /// the block grid — up to 11 ms at 512 samples, which is a 32nd note at 140
    /// BPM and audibly not the pattern that was generated. The host's own note
    /// events carry sample offsets for the same reason, and these are the same
    /// offsets, so the preview and the track agree.
    ///
    /// **Added to the buffer rather than replacing it.** The layout is declared
    /// as a pass-through (see `AUDIO_IO_LAYOUTS`), so whatever a host routes in
    /// must still come out.
    /// Sound one note for the piano roll's keyboard gutter (TASK-041).
    ///
    /// ⛔ **Played through the one *tuned* pad in the preview kit, not through
    /// whichever pad happens to be first.** `render_preview` already documents
    /// that this kit is a drum kit and that the four melodic parts therefore
    /// render silence until pitched instrument voices exist. An audition has the
    /// same problem and one narrow way out: the 808 is the only pad that carries
    /// a `root_note`, so it is the only one that can be transposed to an
    /// arbitrary pitch and still be the note that was asked for. A kick pitched
    /// up forty semitones is not a preview of anything.
    ///
    /// ⚠ **So the gutter previews with an 808 tone, and that is a stand-in.**
    /// When TASK-052/053 give each melodic part its own instrument slot with
    /// chromatic voices, this should play *that part's* instrument. Written down
    /// rather than left to be discovered as "why does my melody sound like a
    /// bass".
    ///
    /// Silent when the preview is switched off, because that switch means
    /// MIDI-only and an audition is not MIDI (FMM-S02).
    fn audition(&mut self, asked: crate::shared::Audition) {
        if !self.shared.audio_enabled() {
            return;
        }
        let Some(kit) = self.kit.as_ref() else {
            return;
        };
        let pitch = match asked {
            crate::shared::Audition::Pitch(pitch) => pitch,
            crate::shared::Audition::Lane(_) => 0,
        };

        // ⛔ **A lane audition resolves by lane and never falls back** (TASK-043).
        // Clicking `Kick` in the grid's row header is not a question about
        // pitch, so the nearest-tuned-pad rule below is exactly wrong for it —
        // it would sound the 808 transposed to the kick's GM note. The lane
        // arrives as that GM note because `gm_drum_note` is the mapping the
        // engine already owns; matching each pad through the same function is
        // what keeps there from being a second one. A kit with no pad for the
        // lane stays **silent**, which is the honest answer: there is nothing
        // loaded to hear, and a stand-in would be a preview of the wrong drum.
        if let crate::shared::Audition::Lane(lane) = asked {
            // ⛔ **By lane, not by note.** `gm_drum_note` is not injective — six
            // lanes answer 0 — so matching on the note sounded the wrong pad
            // for `sub` and `subLow`. `pad_for` is the kit's own lookup.
            if let Some(index) = kit.pad_for(lane) {
                self.sampler.trigger(
                    kit,
                    index,
                    0.8,
                    // As sampled. A drum pad has no root to transpose against,
                    // and `semitones_for` reads 0 as "no opinion" for exactly
                    // this case.
                    0.0,
                    f64::from(self.shared.sample_rate()),
                );
            }
            return;
        }

        // The tuned pad, found by the property that makes it usable rather than
        // by name: a kit that renamed its 808 would still audition correctly,
        // and one with no tuned pad at all stays silent rather than guessing.
        //
        // ⛔ **The *nearest* tuned pad, not the first one.** This took whichever
        // rooted pad came first in the manifest, which was the 808 — fine while
        // it was the only pad carrying a root, and wrong the moment TASK-131
        // gave melody, counter, bass and chords their own. Clicking C6 in the
        // roll's keyboard gutter then triggered the 808 at **+56 semitones**,
        // which is the "a kick pitched up forty semitones is not a preview of
        // anything" case this function's own note warns about — and pressing
        // Play on the same note sounded the lead pad instead, so the audition
        // and the playback were two different instruments.
        //
        // ⚠ Nearest-root rather than the part's own pad, because the audition
        // request carries a pitch and not a part. It reaches the right voice for
        // every register the roll can show: a C6 finds the lead, a C2 finds the
        // bass. Threading the part through would be more exact and is worth
        // doing when the gutter learns to audition drums too.
        let Some((pad_index, root)) = kit
            .pads
            .iter()
            .enumerate()
            .filter_map(|(index, pad)| pad.root_note.map(|root| (index, root)))
            .min_by_key(|(_, root)| u8::abs_diff(*root, pitch))
        else {
            return;
        };
        self.sampler.trigger(
            kit,
            pad_index,
            // A fixed, comfortable level. An audition is a question about pitch,
            // not about dynamics, and the note being previewed does not exist
            // yet to have a velocity of its own.
            0.8,
            f32::from(pitch) - f32::from(root),
            f64::from(self.shared.sample_rate()),
        );
    }

    /// Render only the File Explorer's audition into `buffer`.
    ///
    /// ⛔ **The path taken when nothing is armed**, which is the state the
    /// sample browser is used from. `render_preview` returns early with no kit
    /// and again when the sampler is idle — both correct for a *pattern*, both
    /// fatal for an audition, which needs neither.
    ///
    /// Same audio-thread rules: it borrows the pre-allocated scratch, adds into
    /// it, limits and de-interleaves, and allocates nothing.
    fn render_preview_only(&mut self, buffer: &mut Buffer) {
        let frames = buffer.samples();
        let channels = buffer.channels().clamp(1, 2);
        let needed = frames * channels;
        if needed > self.scratch.len() {
            return;
        }

        let scratch = &mut self.scratch[..needed];
        scratch.fill(0.0);
        self.shared
            .preview
            .render(scratch, channels, self.shared.sample_rate());
        audio::sampler::limit(scratch);

        for (channel, samples) in buffer.as_slice().iter_mut().enumerate() {
            let source = channel.min(channels - 1);
            for (frame, sample) in samples.iter_mut().enumerate().take(frames) {
                *sample += scratch[frame * channels + source];
            }
        }
    }

    fn render_preview(&mut self, buffer: &mut Buffer) {
        // ⛔⛔ **The audition is checked BEFORE the kit, the idle gate AND the
        // MIDI-only switch, and it has to be all three.** `Preview` is the File
        // Explorer's sample player: it needs no kit and no armed pattern, and
        // the state a producer is in when they open the browser and press Play
        // on a sample is *exactly* the state those gates return on. So the
        // audition was silent in the only situation it exists for — nothing
        // sounded, the handoff never picked the buffer up, and
        // `preview_position` reported `playing: true, seconds: 0.0` forever.
        let auditioning = self.shared.preview.is_active();

        // ⛔ **MIDI-only, checked before anything else *about the pattern*.**
        // The notes have already gone out by the time this runs, so returning
        // here silences the plugin without silencing the pattern — which is
        // exactly what a producer routing into their own drum sampler wants
        // (FMM-S02).
        //
        // ⛔⛔ **The audition is NOT gated on it, and that is a deliberate
        // change rather than an oversight.** This gate is older than the sample
        // browser and nobody revisited it when `Preview` landed — its own doc
        // still says "whether the *preview sampler* may sound", which is
        // `self.sampler`, the kit. FMM-S02 is about the plugin not doubling the
        // producer's own drum sampler on the *generated pattern*; a file
        // browser that cannot play the file you just clicked is not a file
        // browser. And the failure it produced was the readout-that-lies shape
        // this file guards against everywhere else: Play lit, `playing: true`,
        // the playhead frozen at 0:00 because `render` was never reached, and
        // nothing on screen saying why.
        //
        // ⚠ The piano roll's keyboard gutter *is* still gated — see
        // `audition()`. That one plays the kit, which is the thing MIDI-only is
        // switching off.
        if !self.shared.audio_enabled() {
            self.sampler.stop_all();
            if auditioning {
                self.render_preview_only(buffer);
            }
            return;
        }

        let Some(kit) = self.kit.as_ref() else {
            // ⚠ A kit that failed to decode must not silence a sample preview;
            // the two have nothing to do with each other.
            if auditioning {
                self.render_preview_only(buffer);
            }
            return;
        };

        // ⚠ **`kit` is whatever was last handed over, not a constant.** It is
        // the shipped kit until the producer assigns one of their own one-shots
        // (TASK-131B), and the swap happens in `process` rather than here — a
        // kit must never change part way through a block, or the segment loop
        // below would trigger half its notes on one kit and render them on
        // another.
        //
        // ⚠ A lane the current kit has no pad for still renders silence, which
        // is `pad_for` refusing to guess rather than a gap: `Lane::Snap` is the
        // one lane the shipped kit has never covered, and assigning a one-shot
        // to it is now how that gets a sound.

        // ⛔ Idle is this plugin's *normal* state — nothing is armed until
        // someone presses Generate, and nothing sounds between patterns. Without
        // this the block still costs a `fill`, a `limit` pass and a
        // de-interleaving add over every sample, ~94 times a second, to write
        // silence on top of silence.
        if self.fired.as_slice().is_empty() && self.sampler.is_silent() {
            if auditioning {
                self.render_preview_only(buffer);
            }
            return;
        }

        let frames = buffer.samples();
        let channels = buffer.channels().clamp(1, 2);
        let needed = frames * channels;
        if needed > self.scratch.len() {
            // The host asked for a block longer than it declared. Growing here
            // would allocate on the audio thread, so the extra is skipped
            // instead — a short render is recoverable, a stall is not.
            return;
        }

        let scratch = &mut self.scratch[..needed];
        scratch.fill(0.0);

        // Walk the block, stopping at each note-on to trigger it. `at` is the
        // frame the next segment starts on.
        let fired = self.fired.as_slice();
        let mut at = 0usize;
        let mut next = 0usize;
        while at < frames {
            while next < fired.len() && fired[next].timing as usize <= at {
                let note = fired[next];
                // A muted lane is muted here and *only* here: its note-on has
                // already been sent, so the DAW's own instrument still plays
                // it. That is the difference between this and muting the lane.
                if self.shared.lane_muted(note.lane) {
                    next += 1;
                    continue;
                }
                if let Some(pad_index) = kit.pad_for(note.lane) {
                    // A pad with a root note is transposed to what was actually
                    // played, without which an 808 line comes out monotone.
                    //
                    // ⛔ **Percussion transposes too, from the lane's own GM
                    // note (TASK-131D).** It used to be pinned at 0, which was
                    // right while every drum note in a pattern carried the same
                    // pitch — and became a readout that lies the moment the
                    // generator learned to walk a roll's pitch. The producer
                    // would have seen the movement in the grid and in the
                    // exported file, and heard a flat roll. Mike, 2026-08-05:
                    // "hihat rolls can go up and down so can kicks, 808s,
                    // snares, etc. in actual song arrangements."
                    let semis =
                        audio::kit::Kit::semitones_for(&kit.pads[pad_index], note.lane, note.note);
                    // ⛔ **The 808 slide, heard live (2026-08-06).** Mike asked
                    // for it *"for my generator playback"* as well as for the
                    // export and the drag. The same `semitones_for` the offline
                    // renderer uses, so a preview and a rendered stem cannot
                    // disagree about the interval; and the same half-and-half
                    // window, so both match where `midi.rs` puts the
                    // destination note-on.
                    let glide = note.slide_to.map(|target| audio::sampler::Glide {
                        semis: audio::kit::Kit::semitones_for(
                            &kit.pads[pad_index],
                            note.lane,
                            target,
                        ) - semis,
                        delay: note.frames / 2,
                        frames: note.frames - note.frames / 2,
                    });
                    self.sampler.trigger_with(
                        kit,
                        pad_index,
                        note.velocity,
                        semis,
                        f64::from(self.shared.sample_rate()),
                        glide,
                    );
                }
                next += 1;
            }

            // Render up to the next trigger, or to the end of the block.
            let until = fired
                .get(next)
                .map(|note| (note.timing as usize).min(frames))
                .unwrap_or(frames)
                .max(at + 1);
            let segment = &mut scratch[at * channels..until * channels];
            self.sampler.render(kit, segment, channels);
            at = until;
        }

        // The File Explorer audition, mixed in before limiting (TASK-132) so a
        // preview over a loud pattern is caught by the same limiter rather
        // than clipping the host output.
        self.shared
            .preview
            .render(scratch, channels, self.shared.sample_rate());

        audio::sampler::limit(scratch);

        // De-interleave onto the host's per-channel slices.
        for (channel, samples) in buffer.as_slice().iter_mut().enumerate() {
            let source = channel.min(channels - 1);
            for (frame, sample) in samples.iter_mut().enumerate().take(frames) {
                *sample += scratch[frame * channels + source];
            }
        }
    }
}

impl ClapPlugin for FreallyMidiMaster {
    const CLAP_ID: &'static str = "com.mikeweaver.freally-midi-master";
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "Artist-accurate MIDI, generated by a rule-based engine. No AI, no accounts, no telemetry.",
    );
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::NoteEffect,
        ClapFeature::Utility,
        ClapFeature::Custom("midi-generator"),
    ];
}

// **No `Vst3Plugin` impl, and no `nih_export_vst3!` — deliberately.**
//
// nih-plug's VST3 export is built on `vst3-sys`, a third-party Rust
// reimplementation of the VST3 interfaces licensed **GPLv3**. Linking it would
// put this proprietary, All-Rights-Reserved product in breach. Steinberg's own
// VST3 SDK went MIT in November 2025 — nih-plug does not use it.
//
// VST3 and AU are projected from this CLAP by `clap-wrapper` at packaging
// time (MIT and Apache-2.0, over Steinberg's MIT SDK and Apple's AudioUnitSDK),
// which is one plugin and three formats rather than three plugins. See
// TASK-P08.
//
// `cargo deny` is what found this, and it is why the `nih_plug` dependency
// carries `default-features = false` in both manifests that name it.
nih_export_clap!(FreallyMidiMaster);

// VST3, projected from the CLAP above by `clap-wrapper` — the same binary
// answering a second set of entry points, not a second implementation.
//
// This is what Ableton, Logic, Pro Tools and Cubase load; none of them speaks
// CLAP. Licence-clean by construction: `clap-wrapper` is MIT/Apache-2.0 over
// Steinberg's VST3 SDK, which went MIT in November 2025.
clap_wrapper::export_vst3!();
