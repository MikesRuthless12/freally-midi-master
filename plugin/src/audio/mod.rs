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

pub mod import;
pub mod kit;
pub mod render;
pub mod sampler;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use include_dir::{include_dir, Dir};

use kit::Kit;

/// The preview kit, compiled in.
///
/// ⚠ **This is a second embed of files `crate::dataset::DATA` already carries.**
/// That `include_dir!` covers all of `data/`, and its `NON_MODEL_DIRS` filter
/// only skips `kits/` when *parsing* models — never when embedding. An earlier
/// version of this comment claimed the opposite, that a narrow root *avoided*
/// duplicating ~1.2 MB; it is the duplication.
///
/// Kept anyway, on a measurement rather than a guess: the real cost is **~1 KB**,
/// because the two embeds are identical `unnamed_addr` constants and the linker
/// folds them. Addressing it through `DATA` would mean every lookup carrying the
/// `kits/trap-default/` prefix — `include_dir`'s `get_entry` matches on paths
/// relative to the macro root — which is a worse API for a kilobyte.
pub(crate) static PREVIEW_KIT: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../data/kits/trap-default");

/// Every kit family, so a model can be heard through its own (TASK-140/#23).
///
/// ⛔ **Eight models were already naming a kit and nothing read it.** `rage`
/// asked for `rage-default` and `uk-drill` for `drill-default`; only
/// `trap-default` existed and [`preview_kit`] was hardcoded to it, so both
/// silently played trap samples. That is the same defect as `drums.percs` —
/// a field authored in the dataset with no reader — found twice in one day.
pub(crate) static ALL_KITS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../data/kits");

/// The kit a model asked for, decoded once and kept.
///
/// Falls back to the trap kit for an unknown name rather than going silent: a
/// model naming a kit that does not exist is a dataset bug, and `datasetc`
/// is where it should be caught — not by a producer hearing nothing.
pub fn kit_for(preview_id: &str) -> Option<&'static Arc<Kit>> {
    /// A kit id to the kit it decoded to, or to `None` for one that is not
    /// embedded. The `None` is cached too, so a model naming a missing kit
    /// does not retry the lookup on every poll.
    type Decoded = BTreeMap<String, Option<&'static Arc<Kit>>>;

    static KITS: OnceLock<std::sync::Mutex<Decoded>> = OnceLock::new();
    let cache = KITS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));

    let mut map = cache.lock().ok()?;
    if !map.contains_key(preview_id) {
        // ⛔ **Leaked ONCE PER KIT, on the miss, and the distinction is the
        // whole of it.** A decoded kit does live for the process — the audio
        // thread holds `&'static` — so leaking is the right shape. But an
        // earlier cut cached the `Arc` and leaked a *fresh box around a clone
        // on every call*, and `kit_for_model` is reached from `kit_state`,
        // which the page **polls**. That is one leaked allocation per poll,
        // for as long as the editor is open: invisible to every test, and a
        // DAW quietly growing across a session.
        //
        // Storing the leaked reference means the eleven families cost eleven
        // allocations total, which is what "bounded" was supposed to mean.
        let decoded: Option<&'static Arc<Kit>> = ALL_KITS
            .get_dir(preview_id)
            .and_then(|dir| Kit::embedded(dir).ok())
            .map(|kit| &*Box::leak(Box::new(Arc::new(kit))));
        if decoded.is_none() {
            nih_plug::nih_log!("no kit named {preview_id}; falling back to the trap kit");
        }
        map.insert(preview_id.to_owned(), decoded);
    }

    match map.get(preview_id).copied().flatten() {
        Some(kit) => Some(kit),
        None => {
            drop(map);
            preview_kit()
        }
    }
}

/// The kit a *model* is heard through — the one it names in `kit.preview`.
///
/// ⛔ **This is the reader those eight models never had.** `rage` has asked for
/// `rage-default` and `uk-drill` for `drill-default` since they were authored,
/// and until this existed every one of them played the trap kit. Resolving the
/// name here, rather than at each call site, is what stops the next caller
/// quietly reaching for [`preview_kit`] again.
///
/// Falls back to the trap kit for a model that names nothing, or names a kit
/// that is not embedded. ⚠ **The fallback is not the safety net it looks like** —
/// it is why the defect was invisible for so long. `datasetc` refusing an
/// unknown `kit.preview` is the real guard, and
/// `every_kit_a_model_asks_for_exists_and_covers_every_lane` is what holds the
/// two together in the meantime.
pub fn kit_for_model(model_id: &str) -> Option<&'static Arc<Kit>> {
    // ⚠ **Borrowed, not cloned.** `dataset::model()` hands back an *owned*
    // `StyleModel`, whose `#[serde(flatten)] blocks` carries the whole body of
    // a 5-7 KB model as parsed `Value` trees — several kilobytes across
    // hundreds of allocations, cloned and thrown away to read one string, on a
    // path the KIT panel calls on every refresh. `loaded()` is `&'static`, so
    // reading through it costs nothing.
    let named = crate::dataset::loaded()
        .models
        .get(model_id)
        .and_then(|model| model.blocks.get("kit"))
        .and_then(|kit| kit.get("preview"))
        .and_then(|preview| preview.as_str());

    match named {
        Some(id) => kit_for(id),
        None => {
            // ⚠ Logged, because the silent fallback is what hid the original
            // defect for as long as it did. `kit_for` already says so for a
            // kit that is named and missing; this is the other half — a model
            // that names none at all and lands on trap.
            nih_plug::nih_log!("{model_id} names no kit; using the trap kit");
            preview_kit()
        }
    }
}

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
///
/// ⚠ **An `Arc` since TASK-131B, and the reason is that the kit stopped being
/// the only one.** A producer's own one-shots are applied by building a *whole
/// new* kit over this one and handing it to the audio thread, so the thing the
/// callback plays is no longer a `'static` — it is whichever kit was handed over
/// last. This one is still the base every rebuild starts from, and is still
/// decoded exactly once.
pub fn preview_kit() -> Option<&'static Arc<Kit>> {
    static KIT: OnceLock<Option<Arc<Kit>>> = OnceLock::new();
    KIT.get_or_init(|| match Kit::embedded(&PREVIEW_KIT) {
        Ok(kit) => Some(Arc::new(kit)),
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
    use crate::shared::ALL_LANES;
    use engine::pattern::Lane;

    #[test]
    fn the_preview_kit_is_in_the_binary_and_decodes() {
        // ⛔ The gate is that it *decodes*, not that the files are present.
        // `include_dir!` embedding WAVs that turn out to be unreadable would
        // leave the plugin silent with nothing in the build to say so. What each
        // pad actually sounds like is `kit`'s own tests' business.
        let kit = preview_kit().expect("the preview kit should be compiled in and decodable");
        assert_eq!(kit.id, "trap-default");
        // Seventeen percussion pads plus the four pitched voices — one per lane
        // the engine has (TASK-131, widened by TASK-140).
        assert_eq!(
            kit.pads.len(),
            ALL_LANES.len(),
            "the shipped kit authors one pad per lane"
        );
    }

    #[test]
    fn every_lane_the_drum_generator_writes_has_a_pad_to_play_it() {
        // A lane with no pad is silence, and silence is harder to notice than a
        // wrong drum — `pad_for` returns `None` rather than defaulting for that
        // reason, so this is what checks the kit actually covers the generator.
        //
        // ⛔ **Driven by `ALL_LANES`, never a list written out here, and
        // TASK-140 is why.** This test used to name eight lanes by hand. The
        // engine then gained eight more — `uk-drill` began writing `woodblock`,
        // `country-train` a `tambourine` — and every one of them generated
        // *silence* while this test stayed green, because a hand-written list
        // cannot notice what it was never told about. That is the
        // readout-that-lies failure this project keeps recording, and a
        // hardcoded list is how it gets in.
        //
        // ⚠ It now covers the melodic lanes too, which the eight-lane version
        // never did — a melody part going silent is the same defect and had no
        // guard at all.
        let kit = preview_kit().unwrap();
        for lane in ALL_LANES {
            assert!(kit.pad_for(*lane).is_some(), "no pad plays {lane:?}");
        }
    }

    #[test]
    fn every_kit_a_model_asks_for_exists_and_covers_every_lane() {
        // ⛔ **The gate #23 exists for.** Eight models named a kit before any of
        // this: `rage` asked for `rage-default`, `uk-drill` for
        // `drill-default`, and only `trap-default` was on disk — so both
        // silently played trap samples with nothing anywhere saying so. This
        // reads what the models actually ask for rather than a list kept here,
        // so a family added to kitgen and forgotten in the data, or named in
        // the data and never generated, fails right here.
        let mut asked: std::collections::BTreeSet<String> = Default::default();
        for model in crate::dataset::loaded().models.values() {
            if let Some(id) = model
                .blocks
                .get("kit")
                .and_then(|kit| kit.get("preview"))
                .and_then(|preview| preview.as_str())
            {
                asked.insert(id.to_owned());
            }
        }
        assert!(!asked.is_empty(), "models must name their kits");

        for id in &asked {
            let dir = ALL_KITS
                .get_dir(id.as_str())
                .unwrap_or_else(|| panic!("a model asks for kit '{id}' and it is not embedded"));
            let kit = Kit::embedded(dir).unwrap_or_else(|e| panic!("kit '{id}' is unusable: {e}"));
            for lane in ALL_LANES {
                assert!(
                    kit.pad_for(*lane).is_some(),
                    "kit '{id}' has no pad for {lane:?}"
                );
            }
        }
    }

    #[test]
    fn a_model_is_heard_through_its_own_kit_and_not_through_trap() {
        // ⛔ **The guard for the half that was missing.** The families were
        // generated, embedded, mapped and covered by tests — and `kit_for` had
        // no callers, so every artist still played trap. Everything upstream of
        // this passed while the feature did nothing, which is precisely the
        // shape of the defect it was built to fix.
        assert_eq!(
            kit_for_model("uk-drill").map(|kit| kit.id.as_str()),
            Some("drill-default"),
            "uk-drill has asked for drill-default since it was authored"
        );
        assert_eq!(
            kit_for_model("country-train").map(|kit| kit.id.as_str()),
            Some("country-default")
        );
        assert_eq!(
            kit_for_model("trap").map(|kit| kit.id.as_str()),
            Some("trap-default")
        );

        // A model nobody has heard of falls back rather than going silent.
        assert_eq!(
            kit_for_model("not-an-artist").map(|kit| kit.id.as_str()),
            Some("trap-default")
        );
    }

    #[test]
    fn asking_for_the_same_kit_twice_hands_back_the_same_one() {
        // ⛔ **A leak guard, not an identity curiosity.** `kit_for` leaks its
        // decoded kits deliberately — the audio thread holds `&'static` — and
        // an earlier cut leaked a *fresh box per call* while caching only the
        // `Arc`. `kit_for_model` is reached from `kit_state`, which the page
        // **polls**, so that was one allocation per poll for as long as the
        // editor stayed open: invisible to every other test here, and a DAW
        // growing quietly across a session.
        //
        // Pointer equality is exactly the property that distinguishes
        // leak-once from leak-per-call, which is why it is asserted rather
        // than kit contents.
        let first = kit_for("drill-default").expect("drill-default is embedded");
        let second = kit_for("drill-default").expect("and stays embedded");
        assert!(
            std::ptr::eq(first, second),
            "each call leaked a new allocation instead of reusing the cached one"
        );

        // The miss is cached too, or a model naming a kit that does not exist
        // would re-walk the embed on every poll.
        let a = kit_for("no-such-kit");
        let b = kit_for("no-such-kit");
        assert!(matches!((a, b), (Some(x), Some(y)) if std::ptr::eq(x, y)));
    }

    #[test]
    fn two_families_are_audibly_different_rather_than_the_same_kit_renamed() {
        // Two kits that decode to identical samples are one kit shipped twice.
        // Each family carries its own seed for exactly this reason.
        let trap = Kit::embedded(ALL_KITS.get_dir("trap-default").unwrap()).unwrap();
        let boom = Kit::embedded(ALL_KITS.get_dir("boom-bap-default").unwrap()).unwrap();

        let snare_of = |kit: &Kit| kit.pads[kit.pad_for(Lane::Snare).unwrap()].samples.clone();
        assert_ne!(
            snare_of(&trap),
            snare_of(&boom),
            "trap and boom-bap ship the same snare"
        );
    }

    #[test]
    fn no_two_lanes_share_a_pad() {
        // Every lane having *a* pad is not enough: two lanes resolving to the
        // same sample is a lane that has been duplicated rather than added, and
        // it reads as the generator repeating itself.
        let kit = preview_kit().unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for lane in ALL_LANES {
            let index = kit.pad_for(*lane).expect("every lane has a pad");
            if let Some(other) = seen.insert(index, lane) {
                panic!(
                    "{lane:?} and {other:?} both resolve to pad '{}'",
                    kit.pads[index].id
                );
            }
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
    fn every_melodic_part_renders_audio_and_transposes_with_the_note() {
        // ⛔ **The end-to-end claim TASK-131 is actually for.** `pad_for`
        // answering `Some` only proves the kit has a pad; it does not prove the
        // voice sounds, and it does not prove the note's pitch reaches it. Four
        // parts out of five rendered pure silence before this, and the failure
        // presented to a producer as "the melody generator is broken".
        let kit = preview_kit().unwrap();

        for lane in [Lane::Melody, Lane::Counter, Lane::Bass, Lane::Chords] {
            let pad = kit
                .pad_for(lane)
                .unwrap_or_else(|| panic!("{lane:?} has no pad"));
            let root = kit.pads[pad]
                .root_note
                .unwrap_or_else(|| panic!("{lane:?} is pitched and must name a root"));

            let render_at = |semis: f32| {
                let mut sampler = sampler::Sampler::default();
                sampler.trigger(kit, pad, 1.0, semis, 48_000.0);
                let mut out = vec![0.0f32; 8192];
                sampler.render(kit, &mut out, 2);
                out
            };

            let at_root = render_at(0.0);
            let peak = at_root.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.01, "{lane:?} rendered silence (peak {peak})");

            // ⛔ **And it must actually transpose.** A pad that ignored `semis`
            // would render the identical buffer for every note — a melody that
            // plays but is monotone, which is the failure the 808 had before it
            // carried a root note. Compared an octave up, where a real repitch
            // cannot possibly produce the same samples.
            let an_octave_up = render_at(12.0);
            assert_ne!(
                at_root, an_octave_up,
                "{lane:?} rendered the same audio an octave apart — it is not transposing"
            );
            // The root is only meaningful if it is inside MIDI's range.
            assert!(root <= 127, "{lane:?} root {root} is out of range");
        }
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
