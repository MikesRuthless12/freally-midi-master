//! What the host tells us about its own session.
//!
//! This is the file the whole pivot was for. The desktop app had to ask the
//! user for a tempo, or negotiate one over Ableton Link, or derive one from a
//! MIDI clock. A plugin is *handed* it, every process block, along with the
//! time signature and where the playhead is.
//!
//! The translation into the engine's own [`SessionContext`] lives here so that
//! the engine keeps knowing nothing about plugins, and so there is exactly one
//! place where "what the host says" becomes "what we generate against".

use engine::context::{SessionContext, SessionOverrides};
use engine::StyleModel;

/// The tempo range a host value is trusted within.
///
/// Hosts report a tempo of `0` while stopped, during scrub, and — in at least
/// one shipping DAW — for the first block after loading a project. Generating
/// against that would produce a pattern at whatever the fallback happened to
/// be, which is the readout-that-lies failure the desktop app's session chips
/// were built to avoid. An out-of-range value means "not known yet", not "very
/// slow", so it is ignored rather than clamped.
const TRUSTED_BPM: std::ops::RangeInclusive<f64> = 20.0..=999.0;

/// The host's session, as last observed.
#[derive(Debug, Clone, PartialEq)]
pub struct HostSession {
    /// `None` until the host has reported a tempo we can believe.
    tempo: Option<f64>,
    time_sig_num: u8,
    time_sig_den: u8,
    playing: bool,
    /// Set for one observation after anything above changed, so the UI can be
    /// told and a pending generation can be re-placed.
    changed: bool,
}

impl Default for HostSession {
    fn default() -> Self {
        Self {
            tempo: None,
            time_sig_num: 4,
            time_sig_den: 4,
            playing: false,
            changed: false,
        }
    }
}

impl HostSession {
    /// Read the host's transport for this block.
    ///
    /// Called once per `process`, on the audio thread, so it allocates nothing
    /// and does no work beyond comparison.
    pub fn observe(&mut self, transport: &nih_plug::prelude::Transport) {
        let tempo = transport.tempo.filter(|bpm| TRUSTED_BPM.contains(bpm));

        // A host that stops reporting a tempo has not changed it to nothing —
        // it has stopped saying. Keeping the last believed value is what makes
        // the readout stable across a transport stop.
        let tempo = tempo.or(self.tempo);

        let num = transport
            .time_sig_numerator
            .filter(|n| *n > 0)
            .map(|n| n as u8)
            .unwrap_or(self.time_sig_num);
        let den = transport
            .time_sig_denominator
            .filter(|d| *d > 0)
            .map(|d| d as u8)
            .unwrap_or(self.time_sig_den);

        let next = Self {
            tempo,
            time_sig_num: num,
            time_sig_den: den,
            playing: transport.playing,
            changed: false,
        };

        self.changed = next.tempo != self.tempo
            || next.time_sig_num != self.time_sig_num
            || next.time_sig_den != self.time_sig_den;
        let playing = transport.playing;
        *self = next;
        self.playing = playing;
    }

    /// The tempo to generate at, or `None` while the host has not said.
    pub fn tempo(&self) -> Option<f64> {
        self.tempo
    }

    pub fn time_signature(&self) -> (u8, u8) {
        (self.time_sig_num, self.time_sig_den)
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Used when rebuilding a session from the shared atomics, which carry the
    /// transport state separately from the tempo.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Did the last observation differ from the one before it?
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// A session as if the host had reported these values.
    ///
    /// `nih_plug::Transport` cannot be constructed outside a real host, so the
    /// integration tests need a way in. Test-only rather than a general
    /// constructor: production code must reach this state through
    /// [`observe`](Self::observe), or a value the host never sent could be
    /// generated against.
    #[doc(hidden)]
    pub fn observed_for_test(tempo: Option<f64>, time_sig_num: u8, time_sig_den: u8) -> Self {
        Self {
            tempo,
            time_sig_num,
            time_sig_den,
            playing: false,
            changed: false,
        }
    }

    /// Build the session a generation runs against.
    ///
    /// The host's tempo and meter win over the model's, because the plugin is
    /// a guest in someone else's project: generating trap at its authored 140
    /// inside a 92 BPM session would produce a clip that does not fit the song
    /// it was asked for. Everything the host does *not* dictate — the key, the
    /// scale, swing, the bar count — is still the model's or the user's, in
    /// that order, exactly as `SessionOverrides` already defines.
    ///
    /// A host that has not reported a tempo yet leaves `bpm` absent, so the
    /// model's own tempo applies and nothing is invented.
    ///
    /// ⛔ **`auto_sync` is what makes the model's own tempo reachable at all**
    /// (TASK-P15). Precedence is **user pin > (host, if `auto_sync`) > model**,
    /// and without the middle clause being switchable there is no way to hear
    /// what an artist is authored at inside a running project: the only exit from
    /// the host's tempo was pinning a number, and pinning 140 *because trap wants
    /// 140* is a different statement from "let trap decide".
    ///
    /// Three states, and the chip has to say which one it is in:
    ///
    /// | pin | `auto_sync` | tempo |
    /// |-----|-------------|-------|
    /// | set | either      | the pin — an instruction is an instruction |
    /// | —   | `true`      | the host's, the default and the reason for the pivot |
    /// | —   | `false`     | the model's own |
    pub fn session_for(
        &self,
        model: &StyleModel,
        user: &SessionOverrides,
        seed: u64,
        auto_sync: bool,
    ) -> SessionContext {
        let mut overrides = user.clone();
        if user.bpm.is_none() && auto_sync {
            if let Some(tempo) = self.tempo {
                overrides.bpm = Some(tempo as f32);
            }
        }

        let mut ctx = SessionContext::from_model(model, &overrides, seed);
        ctx.time_sig_num = self.time_sig_num;
        ctx.time_sig_den = self.time_sig_den;
        ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host session with a believed tempo, without needing a `Transport`.
    fn observed(tempo: Option<f64>, num: u8, den: u8) -> HostSession {
        HostSession {
            tempo,
            time_sig_num: num,
            time_sig_den: den,
            playing: false,
            changed: false,
        }
    }

    fn shipped(id: &str) -> StyleModel {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).unwrap();
        let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();
        models.get(id).cloned().unwrap()
    }

    #[test]
    fn the_hosts_tempo_beats_the_models_own() {
        // The pivot, asserted: trap authors 140, but inside a 92 BPM project a
        // clip at 140 does not fit the song it was asked for.
        let host = observed(Some(92.0), 4, 4);
        let ctx = host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, true);
        assert_eq!(ctx.bpm, 92.0);
    }

    #[test]
    fn a_user_pinned_tempo_still_beats_the_host() {
        // Deliberate: the chip is the user's own instruction, and a host that
        // silently overrode it would make the readout a lie — the same rule
        // TASK-033 set for the artist's value.
        let host = observed(Some(92.0), 4, 4);
        let pinned = SessionOverrides {
            bpm: Some(150.0),
            ..Default::default()
        };
        let ctx = host.session_for(&shipped("trap"), &pinned, 7, true);
        assert_eq!(ctx.bpm, 150.0);
    }

    #[test]
    fn a_host_that_has_not_said_leaves_the_model_its_own_tempo() {
        // Nothing is invented while the host is silent: trap's authored 140.
        let host = observed(None, 4, 4);
        let ctx = host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, true);
        assert_eq!(ctx.bpm, 140.0);
    }

    #[test]
    fn auto_sync_off_leaves_the_model_its_own_tempo_even_while_the_host_reports_one() {
        // ⛔ TASK-P15, and the gap it closes. With auto-sync on there is no way
        // to hear what an artist is *authored* at inside a running project: the
        // only exit from the host's tempo was pinning a number, and pinning 140
        // because trap wants 140 is a different statement from letting trap
        // decide. Every one of the 26 shipped models authors a `bpm.mode` that
        // was unreachable in a host before this.
        let host = observed(Some(92.0), 4, 4);
        let ctx = host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, false);
        assert_eq!(ctx.bpm, 140.0, "trap's own tempo, not the host's 92");

        // And the same host with auto-sync on still follows it, so the toggle is
        // the only difference between the two.
        let synced = host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, true);
        assert_eq!(synced.bpm, 92.0);
    }

    #[test]
    fn a_pin_beats_the_host_whether_auto_sync_is_on_or_off() {
        // A pin is an instruction, and the toggle does not get a vote on it.
        let host = observed(Some(92.0), 4, 4);
        let pinned = SessionOverrides {
            bpm: Some(150.0),
            ..Default::default()
        };

        for auto_sync in [true, false] {
            let ctx = host.session_for(&shipped("trap"), &pinned, 7, auto_sync);
            assert_eq!(ctx.bpm, 150.0, "auto_sync = {auto_sync}");
        }
    }

    #[test]
    fn the_meter_follows_the_host_whatever_auto_sync_says() {
        // ⛔ The toggle is about *tempo*. A 6/8 project is 6/8 whether or not the
        // user wants the artist's own BPM — bar arithmetic is not a preference,
        // and generating 4/4 bars into a 6/8 song puts every position the
        // grammar names somewhere else.
        let host = observed(Some(120.0), 6, 8);
        for auto_sync in [true, false] {
            let ctx =
                host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, auto_sync);
            assert_eq!(
                (ctx.time_sig_num, ctx.time_sig_den),
                (6, 8),
                "auto_sync = {auto_sync}"
            );
        }
    }

    #[test]
    fn the_hosts_meter_reaches_the_bar_arithmetic() {
        // A 6/8 project must generate 6/8 bars, or every position the grammar
        // names lands somewhere else.
        let host = observed(Some(120.0), 6, 8);
        let ctx = host.session_for(&shipped("trap"), &SessionOverrides::default(), 7, true);
        assert_eq!((ctx.time_sig_num, ctx.time_sig_den), (6, 8));
        // Six eighths is three quarter notes, which is what `ticks_per_bar`
        // already knows — this is wiring, not new arithmetic.
        assert_eq!(ctx.ticks_per_bar(), engine::pattern::PPQ * 3);
    }

    #[test]
    fn a_nonsense_tempo_is_ignored_rather_than_believed() {
        // Hosts report 0 while stopped and during scrub. Generating against it
        // would produce a pattern at whatever the fallback happened to be.
        assert!(!TRUSTED_BPM.contains(&0.0));
        assert!(!TRUSTED_BPM.contains(&-120.0));
        assert!(!TRUSTED_BPM.contains(&100_000.0));
        assert!(TRUSTED_BPM.contains(&92.0));
        assert!(TRUSTED_BPM.contains(&20.0));
    }
}
