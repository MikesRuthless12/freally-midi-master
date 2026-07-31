//! The plugin's own sampler: the preview kit, rendered into the host's buffer.
//!
//! **This is TASK-P17 / FMM-S01, and it is what makes the generators audible.**
//! Until it existed the plugin emitted notes onto the host's track and made no
//! sound of its own — which works only if the producer has already loaded a drum
//! instrument after it. A generator you cannot hear without doing setup first is
//! a generator nobody auditions.
//!
//! ## Ported, not rewritten
//!
//! `kit` and `sampler` came from `src-tauri/src/audio/` almost unchanged, and
//! deliberately: that code has tests, a limiter that has been listened to, and a
//! voice allocator that does not allocate. ⛔ **The `src-tauri` crate cannot be
//! removed until this port exists**, because removing it would delete the only
//! copy — which is why the roadmap pairs the two.
//!
//! One thing did change, and it had to: [`kit::Kit::embedded`] reads the kit out
//! of the binary rather than off disk. A plugin has no resource directory it can
//! trust — it is a shared library inside someone else's process — which is the
//! same argument that already compiles in the dataset and the UI.
//!
//! ## The audio-thread rules, which are absolute
//!
//! Everything under [`sampler`] runs on the host's audio callback: **no
//! allocation, no locking, no I/O, no panicking path.** Voices live in a fixed
//! array and address pads by index. The kit itself is built once, off the audio
//! thread, and handed across — the same shape [`crate::voice::Schedule`] already
//! uses for patterns, and for the same reason.

pub mod kit;
pub mod sampler;

use std::sync::OnceLock;

use include_dir::{include_dir, Dir};

use kit::Kit;

/// The preview kit, compiled in.
///
/// ⚠ **This is a second embed of files `crate::dataset::DATA` already carries.**
/// That `include_dir!` covers all of `data/`, and its `NON_MODEL_DIRS` filter
/// only skips `kits/` when *parsing* models — never when embedding. An earlier
/// version of this comment claimed the opposite, that a narrow root *avoided*
/// duplicating 400 KB; it is the duplication.
///
/// Kept anyway, on a measurement rather than a guess: the real cost is **~1 KB**,
/// because the two embeds are identical `unnamed_addr` constants and the linker
/// folds them. Addressing it through `DATA` would mean every lookup carrying the
/// `kits/trap-default/` prefix — `include_dir`'s `get_entry` matches on paths
/// relative to the macro root — which is a worse API for a kilobyte.
pub(crate) static PREVIEW_KIT: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../data/kits/trap-default");

/// The preview kit, decoded once.
///
/// ⛔ **Built lazily and then never again, and never on the audio thread.**
/// Decoding eight WAVs allocates; doing it inside `process` would be the exact
/// real-time violation this module's rules exist to prevent. The first caller is
/// `initialize`, which the host runs before it ever asks for audio.
///
/// A failed decode yields `None` and the plugin stays silent rather than
/// refusing to load: a broken preview kit must not cost a producer the plugin
/// that generates their MIDI.
pub fn preview_kit() -> Option<&'static Kit> {
    static KIT: OnceLock<Option<Kit>> = OnceLock::new();
    KIT.get_or_init(|| match Kit::embedded(&PREVIEW_KIT) {
        Ok(kit) => Some(kit),
        Err(error) => {
            nih_plug::nih_log!(
                "the preview kit could not be loaded, so the plugin is silent: {error}"
            );
            None
        }
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::Lane;

    #[test]
    fn the_preview_kit_is_in_the_binary_and_decodes() {
        // ⛔ The gate is that it *decodes*, not that the files are present.
        // `include_dir!` embedding eight WAVs that turn out to be unreadable
        // would leave the plugin silent with nothing in the build to say so.
        // What each pad actually sounds like is `kit`'s own tests' business.
        let kit = preview_kit().expect("the preview kit should be compiled in and decodable");
        assert_eq!(kit.id, "trap-default");
        assert_eq!(kit.pads.len(), 8, "the shipped kit authors eight pads");
    }

    #[test]
    fn every_lane_the_drum_generator_writes_has_a_pad_to_play_it() {
        // A lane with no pad is silence, and silence is harder to notice than a
        // wrong drum — `pad_for` returns `None` rather than defaulting for that
        // reason, so this is what checks the kit actually covers the generator.
        let kit = preview_kit().unwrap();
        for lane in [
            Lane::Kick,
            Lane::Snare,
            Lane::Clap,
            Lane::ClosedHat,
            Lane::OpenHat,
            Lane::Rim,
            Lane::Perc,
            Lane::Bass808,
        ] {
            assert!(kit.pad_for(lane).is_some(), "no pad plays {lane:?}");
        }
    }

    #[test]
    fn triggering_a_pad_actually_produces_audio() {
        // ⛔ **The gate this whole task exists for.** Everything above proves the
        // kit decoded; this proves a note becomes a signal. A plugin that loads
        // its kit perfectly and renders silence looks identical to every other
        // test in this file — and silence is exactly what it did before P17.
        let kit = preview_kit().unwrap();
        let mut sampler = sampler::Sampler::default();

        let mut out = vec![0.0f32; 2048];
        sampler.render(kit, &mut out, 2);
        assert!(
            out.iter().all(|s| *s == 0.0),
            "an untriggered sampler must be silent"
        );

        sampler.trigger(kit, kit.pad_for(Lane::Kick).unwrap(), 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 1);

        sampler.render(kit, &mut out, 2);
        let peak = out.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(
            peak > 0.01,
            "a triggered kick rendered silence (peak {peak})"
        );
    }

    #[test]
    fn a_mono_insert_is_the_same_loudness_as_a_stereo_one() {
        // ⛔ The pan law is an equal-power *split*: a centred pad is 0.707 on
        // each side. Writing only the left on a mono bus therefore put the
        // whole preview 3 dB below the stereo track next to it, and would have
        // silenced the first hard-panned pad anyone authored.
        let kit = preview_kit().unwrap();
        let peak = |channels: usize| {
            let mut sampler = sampler::Sampler::default();
            sampler.trigger(kit, kit.pad_for(Lane::Kick).unwrap(), 1.0, 0.0, 48_000.0);
            let mut out = vec![0.0f32; 4096];
            sampler.render(kit, &mut out, channels);
            out.chunks(channels)
                .map(|frame| frame.iter().map(|s| s * s).sum::<f32>().sqrt())
                .fold(0.0f32, f32::max)
        };

        // Compared as total power across the frame, which is what "the same
        // loudness" means when the channel count differs: one mono channel has
        // to carry what two stereo channels carried between them.
        let (mono, stereo) = (peak(1), peak(2));
        assert!(mono > 0.01 && stereo > 0.01, "both should sound");
        assert!(
            (mono - stereo).abs() / stereo < 0.05,
            "mono {mono} and stereo {stereo} should carry the same power"
        );
    }

    #[test]
    fn the_hats_choke_each_other_through_the_sampler() {
        // The kit *declares* the choke group; this is the sampler honouring it.
        // An open hat ringing under the closed hat that should have cut it is
        // the most audible way a drum preview sounds wrong.
        let kit = preview_kit().unwrap();
        let mut sampler = sampler::Sampler::default();

        sampler.trigger(kit, kit.pad_for(Lane::OpenHat).unwrap(), 1.0, 0.0, 48_000.0);
        sampler.trigger(
            kit,
            kit.pad_for(Lane::ClosedHat).unwrap(),
            1.0,
            0.0,
            48_000.0,
        );
        assert_eq!(
            sampler.active_voices(),
            1,
            "the closed hat should have cut the open one"
        );

        // A kick is in no choke group and must survive alongside it.
        sampler.trigger(kit, kit.pad_for(Lane::Kick).unwrap(), 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 2);
    }
}
