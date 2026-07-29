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

use std::sync::Arc;

use nih_plug::prelude::*;

pub mod bridge;
pub mod dataset;
mod editor;
pub mod host;
pub mod shared;
pub mod voice;

pub use bridge::{dispatch, Request};
pub use host::HostSession;
pub use shared::{Shared, SharedState};
pub use voice::Schedule;

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
}

#[derive(Params, Default)]
pub struct FreallyParams {}

impl Default for FreallyMidiMaster {
    fn default() -> Self {
        Self {
            params: Arc::new(FreallyParams::default()),
            session: HostSession::default(),
            pending: Schedule::default(),
            shared: Arc::new(Shared::default()),
        }
    }
}

impl Plugin for FreallyMidiMaster {
    const NAME: &'static str = "Freally MIDI Master";
    const VENDOR: &'static str = "Havoc Software";
    const URL: &'static str = "https://github.com/MikesRuthless12/freally-midi-master";
    const EMAIL: &'static str = "mythodikalone@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// A MIDI-effect layout: no audio in, no audio out.
    ///
    /// This plugin makes notes, not sound. Declaring an audio layout it does
    /// not use would have hosts insert it on an audio track and wonder why it
    /// is silent — the layout is how a DAW knows to offer it as a MIDI device.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: None,
        ..AudioIOLayout::const_default()
    }];

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
        true
    }

    fn reset(&mut self) {
        // The host has jumped or stopped. Anything still scheduled belongs to a
        // timeline position that is no longer where we are, and emitting it
        // would leave notes hanging on the track.
        self.pending.clear();
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // The pivot's whole point, in two lines: the host tells us the tempo
        // and the meter, every block, for free.
        self.session.observe(context.transport());
        self.shared.host.publish(&self.session);

        // Take a newly generated pattern if one is waiting. The schedule this
        // replaces is handed back rather than dropped — freeing its `Vec` here
        // would take the allocator's lock on the audio thread.
        self.pending = self
            .shared
            .handoff
            .receive(std::mem::take(&mut self.pending));

        self.pending.emit(context);

        ProcessStatus::Normal
    }
}

impl ClapPlugin for FreallyMidiMaster {
    const CLAP_ID: &'static str = "com.havocsoftware.freally-midi-master";
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
