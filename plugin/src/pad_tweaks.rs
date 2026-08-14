//! What the producer did to a pad, as distinct from what the kit shipped
//! (TASK-055A, TASK-164).
//!
//! ⛔⛔ **These are non-destructive and they are the pad's, not the sample's.**
//! Mike asked for *"reverse … start/end trim handles … fade in/out, gain, pitch
//! in semitones and cents, pan, and a normalize toggle"* and said what they are
//! not: *"a first-class toggle on the pad and in the sample editor, **not a
//! destructive re-import**"*. So nothing here rewrites a file, and nothing here
//! rewrites the decoded buffer either — [`PadTweaks`] is applied as *offsets and
//! multipliers over* the samples the pad already holds.
//!
//! ⚠ **Reverse is the one exception, and it is deliberately not in this
//! struct.** Flipping the buffer happens once at decode
//! ([`crate::oneshot::load`]) because it never changes for that assignment, and
//! it is already persisted as `PluginSession::one_shots_reversed`. Moving it
//! here would be a second answer to a question that already has one — the drift
//! this codebase keeps recording.
//!
//! ## Why the default has to be exactly "do nothing"
//!
//! Every pad in the product gets a [`PadTweaks`] whether or not a producer has
//! ever opened the editor, so [`PadTweaks::default`] is the identity: 0 dB, dead
//! centre, no transposition, the whole sample, no fades and no envelope. That is
//! not tidiness — [`PadTweaks::is_identity`] is what lets
//! [`crate::audio::sampler`] skip the envelope arithmetic entirely, so an
//! untouched kit costs the audio thread nothing at all for a feature it is not
//! using.

use serde::{Deserialize, Serialize};

/// The quietest a pad can be pushed before it is simply off.
///
/// ⚠ A floor rather than an open range: `-inf dB` is a valid decibel value and
/// an unhelpful control, and a producer who wants silence has a mute.
pub const MIN_GAIN_DB: f32 = -60.0;

/// The loudest. ⚠ Positive headroom is deliberate — a quiet sample needs
/// bringing up, and `sampler::limit` is what stops the bus cracking.
pub const MAX_GAIN_DB: f32 = 12.0;

/// How far a pad may be transposed, in semitones each way.
///
/// Two octaves. Past that a sampled one-shot is an artefact rather than a note,
/// and the read rate is far enough from 1.0 that the interpolator is audible.
pub const MAX_SEMIS: i32 = 24;

/// The one-shot's own amplitude envelope (TASK-164).
///
/// ⛔⛔ **Sustain is in DECIBELS, not a 0–1 ratio**, and that is read off the
/// reference Mike supplied rather than chosen: *"A 0.00 ms · D 195 ms · S −36.00
/// dB · R 5.00 s"*. A ratio control spends most of its travel in the top 6 dB
/// where nothing much happens; a dB control spends it where the ear is.
///
/// ⚠ **The default is the identity envelope** — no attack ramp, no decay,
/// sustain at unity, no release — so a pad nobody has edited sounds exactly as
/// it did before this existed. See [`Adsr::is_identity`].
/// ⛔ **`default` on the container, exactly like [`PadTweaks`], and for the
/// same reason it is load-bearing there.** Without it a project written by an
/// older build — or a page sending only the stage it changed — is refused whole
/// with `missing field attackMs`, so one absent field would lose the entire
/// envelope rather than that one stage. Found by
/// `a_pad_edit_is_stored_and_comes_back_on_the_lane_it_was_made_on`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Adsr {
    pub attack_ms: f32,
    pub decay_ms: f32,
    /// The level held after the decay, in dB relative to the peak. `0.0` is
    /// unity; [`MIN_GAIN_DB`] is the floor, for the same reason it is there.
    pub sustain_db: f32,
    pub release_ms: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Adsr {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain_db: 0.0,
            release_ms: 0.0,
        }
    }
}

impl Adsr {
    /// Does this envelope change anything?
    ///
    /// ⛔ **The audio thread branches on this once per voice, not per frame.**
    /// An identity envelope is the overwhelmingly common case — every shipped
    /// kit, every pad nobody has opened — and paying four multiplies a frame
    /// across 48 voices to compute a gain of 1.0 is a cost with no feature
    /// behind it.
    pub fn is_identity(&self) -> bool {
        self.attack_ms <= 0.0
            && self.decay_ms <= 0.0
            && self.sustain_db >= 0.0
            && self.release_ms <= 0.0
    }

    /// The same envelope with every field inside its range.
    pub fn clamped(&self) -> Adsr {
        Adsr {
            // ⚠ No upper bound worth naming: a ten-second attack is a swell, and
            // the voice ends at the end of its sample regardless.
            attack_ms: self.attack_ms.max(0.0),
            decay_ms: self.decay_ms.max(0.0),
            sustain_db: self.sustain_db.clamp(MIN_GAIN_DB, 0.0),
            release_ms: self.release_ms.max(0.0),
        }
    }
}

/// Everything a producer can do to a pad without touching the file.
///
/// ⚠ **`#[serde(default)]` on every field, and that is load-bearing.** This
/// rides in the project blob, so a project written by an older build arrives
/// with fields missing — and a missing field must mean "untouched", never zero
/// where zero is a change. [`Self::trim_end`] is the one where those differ:
/// `0.0` would be an empty pad and the default is `1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PadTweaks {
    pub gain_db: f32,
    /// -1 hard left, 0 centre, +1 hard right.
    pub pan: f32,
    pub semis: i32,
    /// Hundredths of a semitone, for tuning a sample that was not recorded in
    /// tune. ⚠ Separate from [`Self::semis`] rather than a single float,
    /// because the two are different controls: one is transposition and one is
    /// tuning, and a producer nudging cents must not be able to jump an octave.
    pub cents: i32,
    /// Scale the pad so its loudest sample reaches full scale.
    ///
    /// ⚠ Measured off the decoded buffer when the pad is built, never on the
    /// audio thread — it is a scan of the whole sample.
    pub normalize: bool,
    /// Where the pad starts playing, as a fraction of the sample.
    pub trim_start: f32,
    /// Where it stops, as a fraction of the sample. Defaults to `1.0`.
    pub trim_end: f32,
    /// Seconds of ramp in from silence at [`Self::trim_start`].
    pub fade_in_s: f32,
    /// Seconds of ramp out to silence at [`Self::trim_end`].
    pub fade_out_s: f32,
    pub adsr: Adsr,
}

impl Default for PadTweaks {
    fn default() -> Self {
        PadTweaks {
            gain_db: 0.0,
            pan: 0.0,
            semis: 0,
            cents: 0,
            normalize: false,
            trim_start: 0.0,
            trim_end: 1.0,
            fade_in_s: 0.0,
            fade_out_s: 0.0,
            adsr: Adsr::default(),
        }
    }
}

impl PadTweaks {
    /// Is this pad untouched?
    ///
    /// ⛔ **What decides whether the entry is stored at all.** A project holding
    /// thirty-seven identity entries is thirty-seven rows of nothing in a blob
    /// the host serializes on every save, and it would make `pad_tweaks` present
    /// in every project file ever written from this build.
    pub fn is_identity(&self) -> bool {
        *self == PadTweaks::default()
    }

    /// The same tweaks with every field inside its range.
    ///
    /// ⛔ **Applied on the way IN, not on the way out.** The values arrive from
    /// the page as JSON, which can carry anything — and an unclamped `trim_start`
    /// past `trim_end` is an inverted slice, which is the shape that aborted the
    /// host in `smf_read`. Clamping at the door means every reader downstream can
    /// treat these as sane.
    pub fn clamped(&self) -> PadTweaks {
        // ⚠ `trim_start` first, then `trim_end` against it, so the window can
        // never invert however the two arrived.
        let trim_start = self.trim_start.clamp(0.0, 1.0);
        PadTweaks {
            gain_db: self.gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB),
            pan: self.pan.clamp(-1.0, 1.0),
            semis: self.semis.clamp(-MAX_SEMIS, MAX_SEMIS),
            cents: self.cents.clamp(-100, 100),
            normalize: self.normalize,
            trim_start,
            trim_end: self.trim_end.clamp(trim_start, 1.0),
            fade_in_s: self.fade_in_s.max(0.0),
            fade_out_s: self.fade_out_s.max(0.0),
            adsr: self.adsr.clamped(),
        }
    }

    /// The linear multiplier [`Self::gain_db`] names.
    pub fn gain_linear(&self) -> f32 {
        db_to_linear(self.gain_db)
    }

    /// Total transposition in semitones, cents folded in.
    pub fn pitch_semis(&self) -> f32 {
        self.semis as f32 + self.cents as f32 / 100.0
    }
}

/// [`PadTweaks`] resolved against a decoded buffer — what a [`crate::audio::kit::Pad`]
/// carries and what the audio thread reads.
///
/// ⛔⛔ **Resolved off the audio thread, once per kit build, and never
/// recomputed inside the callback.** Turning "0.25 of the sample" into a sample
/// index needs the buffer's length, and turning "12 ms" into frames needs a rate
/// — both are divisions, and neither answer changes while a voice is sounding.
///
/// ⚠ **The window is in SAMPLE indices, not frames**, because that is what the
/// read position walks in. The two differ whenever the device rate is not the
/// pad's, which is nearly always.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadShape {
    /// First sample the voice reads.
    pub start: u32,
    /// One past the last. ⚠ [`u32::MAX`] in the default means "to the end of
    /// whatever buffer this pad turns out to hold" — the sampler takes the `min`
    /// with the real length once, at trigger, so a shape can never outlive the
    /// audio it was measured against.
    pub end: u32,
    /// Samples of ramp in from silence at [`Self::start`]. 0 for none.
    pub fade_in: u32,
    /// Samples of ramp out to silence at [`Self::end`]. 0 for none.
    pub fade_out: u32,
    /// The producer's envelope, or `None` for the identity — see
    /// [`Adsr::is_identity`] for why the `None` matters.
    pub adsr: Option<Adsr>,
}

impl Default for PadShape {
    fn default() -> Self {
        PadShape {
            start: 0,
            end: u32::MAX,
            fade_in: 0,
            fade_out: 0,
            adsr: None,
        }
    }
}

impl PadShape {
    /// Does this pad need the shaping arithmetic at all?
    pub fn is_plain(&self) -> bool {
        *self == PadShape::default()
    }
}

impl PadTweaks {
    /// Turn the fractions and milliseconds into sample counts for `samples`.
    ///
    /// ⚠ **`len` is the buffer this pad actually holds**, so a trim survives the
    /// sample being replaced by a longer or shorter one only in the sense that
    /// the *fraction* is preserved — which is the honest behaviour, because the
    /// producer dragged a handle to a place in a waveform, not to a sample index.
    pub fn shape_for(&self, len: usize, sample_rate: u32) -> PadShape {
        let tweaks = self.clamped();
        let len = len as f32;
        let start = (tweaks.trim_start * len) as u32;
        // ⚠ `max(start + 1)`: a window of no samples is a pad that cannot make a
        // sound, and the sampler would read `samples[start]` past its own end.
        let end = ((tweaks.trim_end * len) as u32).max(start.saturating_add(1));
        let per_second = sample_rate as f32;
        PadShape {
            start,
            end,
            fade_in: (tweaks.fade_in_s * per_second) as u32,
            fade_out: (tweaks.fade_out_s * per_second) as u32,
            adsr: (!tweaks.adsr.is_identity()).then_some(tweaks.adsr),
        }
    }

    /// The extra gain that makes the loudest sample in the window reach full
    /// scale, or `1.0` when [`Self::normalize`] is off.
    ///
    /// ⛔ **A scan of the buffer, so it happens where kits are built and nowhere
    /// near the callback.** ⚠ Measured over the **trimmed** window rather than
    /// the whole file: normalizing to a peak the producer has just trimmed away
    /// would leave the audible part quiet and the control looking broken.
    pub fn normalize_gain(&self, samples: &[f32], shape: &PadShape) -> f32 {
        if !self.normalize {
            return 1.0;
        }
        let start = (shape.start as usize).min(samples.len());
        let end = (shape.end as usize).min(samples.len());
        let peak = samples[start..end]
            .iter()
            .fold(0.0f32, |worst, sample| worst.max(sample.abs()));
        // ⚠ Silence has no peak to normalize to and `1.0 / 0.0` is an infinite
        // gain on the audio thread. A silent pad stays silent.
        if peak <= f32::EPSILON {
            1.0
        } else {
            1.0 / peak
        }
    }
}

/// Decibels to a linear multiplier.
///
/// ⚠ [`MIN_GAIN_DB`] maps to 0.001 rather than to zero, and that is honest: the
/// control's floor is −60 dB, which is a real level and not silence. A producer
/// who wants nothing has a mute button that says so.
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A linear multiplier back to decibels, for a readout.
///
/// ⚠ Floors at [`MIN_GAIN_DB`] rather than returning `-inf`, because this feeds
/// a number the UI prints and `-inf dB` in a text field is not a readout.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        return MIN_GAIN_DB;
    }
    (20.0 * linear.log10()).max(MIN_GAIN_DB)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⛔⛔ **The wire names, pinned, because the page hand-writes this type.**
    ///
    /// `ts-rs` exports `engine`'s IPC types and deliberately does **not** export
    /// this one: two test binaries writing one `ipc-types.ts` in a parallel
    /// `cargo test --workspace` is a race, and that file is already gated by a
    /// `git diff --exit-code` in `ci:local`. So `src/state/kit.ts` mirrors these
    /// names by hand — and *"mirrors X" in a comment is not a mirror*, which is
    /// this codebase's own recorded lesson from `state/lanes.ts` drifting to 21
    /// of 37 lanes while two green tests watched.
    ///
    /// ▶ **What holds it instead:** the page never invents a `PadTweaks`. Every
    /// one it edits is a copy of what `kit_state` sent, so the plugin owns the
    /// defaults outright. This test is the other half — renaming a field here
    /// fails a test that names the page file, rather than silently leaving a
    /// control bound to `undefined`.
    #[test]
    fn the_wire_names_the_page_binds_to_do_not_move_silently() {
        let json = serde_json::to_value(PadTweaks::default()).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "adsr",
                "cents",
                "fadeInS",
                "fadeOutS",
                "gainDb",
                "normalize",
                "pan",
                "semis",
                "trimEnd",
                "trimStart",
            ],
            "src/state/kit.ts binds these names by hand — change both or neither"
        );

        let adsr = json.get("adsr").unwrap();
        let mut stages: Vec<&str> = adsr
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        stages.sort_unstable();
        assert_eq!(
            stages,
            ["attackMs", "decayMs", "releaseMs", "sustainDb"],
            "src/components/Kit/PadEditor.tsx draws these four by name"
        );
    }

    #[test]
    fn an_untouched_pad_is_the_identity_and_says_so() {
        // ⛔ The whole cost model rests on this: if the default were not the
        // identity, every pad in the product would pay for the envelope.
        let plain = PadTweaks::default();
        assert!(plain.is_identity());
        assert!(plain.adsr.is_identity());
        assert_eq!(plain.gain_linear(), 1.0);
        assert_eq!(plain.pitch_semis(), 0.0);
        assert_eq!(
            plain.trim_end, 1.0,
            "the default window is the whole sample"
        );
    }

    #[test]
    fn a_missing_field_means_untouched_rather_than_zero() {
        // ⛔⛔ The project-blob case. An older build's entry has no `trimEnd`,
        // and `0.0` there would be an empty pad — a sample that silently stopped
        // making a sound when the project was reopened on a newer build.
        let sparse: PadTweaks = serde_json::from_str(r#"{"gainDb": -6.0}"#).unwrap();
        assert_eq!(sparse.gain_db, -6.0);
        assert_eq!(sparse.trim_end, 1.0);
        assert_eq!(sparse.trim_start, 0.0);
        assert!(sparse.adsr.is_identity());
    }

    #[test]
    fn a_partial_envelope_keeps_the_stages_it_names_instead_of_being_refused() {
        // ⛔⛔ **`Adsr` needs `default` on its own container, not only its
        // parent.** Without it this is `missing field attackMs` and the *whole*
        // block is refused — so a project written by an older build, or a page
        // sending only the stage that moved, loses the entire envelope rather
        // than the one field. Found across the bridge by
        // `a_pad_edit_is_stored_and_comes_back_on_the_lane_it_was_made_on`.
        let partial: PadTweaks =
            serde_json::from_str(r#"{"adsr": {"decayMs": 195.0, "sustainDb": -36.0}}"#).unwrap();
        assert_eq!(partial.adsr.decay_ms, 195.0);
        assert_eq!(partial.adsr.sustain_db, -36.0);
        assert_eq!(partial.adsr.attack_ms, 0.0);
        assert_eq!(partial.adsr.release_ms, 0.0);
        assert!(!partial.adsr.is_identity());
    }

    #[test]
    fn an_inverted_trim_window_is_closed_rather_than_inverted() {
        // ⛔ An inverted slice is the shape that aborted the host in `smf_read`.
        // The page cannot send one past this.
        let crossed = PadTweaks {
            trim_start: 0.8,
            trim_end: 0.2,
            ..PadTweaks::default()
        }
        .clamped();
        assert_eq!(crossed.trim_start, 0.8);
        assert_eq!(crossed.trim_end, 0.8, "the window closes, it does not flip");
    }

    #[test]
    fn every_control_is_bounded_at_the_door() {
        let wild = PadTweaks {
            gain_db: 400.0,
            pan: -9.0,
            semis: 900,
            cents: -5_000,
            fade_in_s: -3.0,
            adsr: Adsr {
                attack_ms: -10.0,
                decay_ms: -1.0,
                sustain_db: 40.0,
                release_ms: -2.0,
            },
            ..PadTweaks::default()
        }
        .clamped();

        assert_eq!(wild.gain_db, MAX_GAIN_DB);
        assert_eq!(wild.pan, -1.0);
        assert_eq!(wild.semis, MAX_SEMIS);
        assert_eq!(wild.cents, -100);
        assert_eq!(wild.fade_in_s, 0.0);
        assert_eq!(wild.adsr.attack_ms, 0.0);
        assert_eq!(
            wild.adsr.sustain_db, 0.0,
            "sustain is relative to the peak, so above unity is not a level"
        );
    }

    #[test]
    fn decibels_and_linear_gain_are_inverses_where_it_matters() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_linear(-6.0) - 0.501_187).abs() < 1e-5);
        for db in [-48.0f32, -12.0, -6.0, 0.0, 6.0] {
            assert!(
                (linear_to_db(db_to_linear(db)) - db).abs() < 1e-4,
                "{db} dB"
            );
        }
    }

    #[test]
    fn silence_reads_as_the_floor_rather_than_negative_infinity() {
        // ⚠ This feeds a number the UI prints.
        assert_eq!(linear_to_db(0.0), MIN_GAIN_DB);
        assert!(linear_to_db(1e-9).is_finite());
    }

    #[test]
    fn cents_are_a_hundredth_of_a_semitone_and_add_to_it() {
        let tuned = PadTweaks {
            semis: -2,
            cents: 50,
            ..PadTweaks::default()
        };
        assert!((tuned.pitch_semis() - -1.5).abs() < 1e-6);
    }

    #[test]
    fn an_envelope_with_any_stage_set_is_not_the_identity() {
        // ⚠ Each stage on its own, because the audio thread's skip is an `&&`
        // chain and one wrong operator there would silently drop a producer's
        // envelope.
        for adsr in [
            Adsr {
                attack_ms: 5.0,
                ..Adsr::default()
            },
            Adsr {
                decay_ms: 195.0,
                ..Adsr::default()
            },
            Adsr {
                sustain_db: -36.0,
                ..Adsr::default()
            },
            Adsr {
                release_ms: 5_000.0,
                ..Adsr::default()
            },
        ] {
            assert!(!adsr.is_identity(), "{adsr:?} changes the sound");
        }
    }
}
