//! `kitgen` — synthesizes the preview one-shot kits.
//!
//! Every sample is generated from oscillators and filtered noise, so the kits
//! that ship are CC0 by construction: there is no recorded material anywhere in
//! them and no third-party licence to honour (PRD § 15 Q5).
//!
//! Output is deterministic for a given seed. The generated kits are committed,
//! so a rebuild that produced different audio would show up as a permanent
//! spurious diff.
//!
//! ```text
//! kitgen [OUTPUT_DIR]     default: data/kits
//! ```

mod voices;
mod wav;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::json;

/// The seed every kit is generated from. Changing it regenerates every sample.
const KIT_SEED: u64 = 0x5052_4556_4945_5721; // "PREVIEW!"

struct Voice {
    /// File stem, and the pad's id.
    name: &'static str,
    /// The engine lane this pad plays.
    lane: &'static str,
    samples: Vec<f32>,
    /// Pads in the same choke group cut each other off, as a real hi-hat does.
    choke_group: Option<u8>,
    /// The MIDI note this sample was synthesized at, for pads that carry pitch.
    ///
    /// `None` for percussion, which has no pitch to transpose from. Without it
    /// the sampler has no way to play an 808 line in the session's key — every
    /// note would sound at the pitch the sample happens to be, and a bassline
    /// would come out monotone.
    root_note: Option<u8>,
}

/// A kit family: one set of recipes heard through one studio.
///
/// ⛔ **Families, not one kit per genre, and the reason is musical rather than
/// budgetary.** The sample character that separates records is family-level —
/// drill and trap share a lineage and differ in tuning and tightness; boom-bap
/// and country differ from both in the same way. Twenty near-identical kits
/// would be twenty things to keep in tune with each other for a difference
/// nobody could name.
///
/// ⚠ **Every family carries every lane.** `ALL_LANES` is what the pad guard
/// checks, so a family that skipped `cowbell` because "country has no cowbell"
/// would generate silence the moment a model authored one.
struct Family {
    /// The id a model names in `kit.preview`.
    id: &'static str,
    name: &'static str,
    /// ⛔ **Its own seed, so the noise-based voices genuinely differ.** Two
    /// families sharing a seed would have bit-identical hats and snares with
    /// only the tone shaping between them, which is a preset rather than a kit.
    seed: u64,
    /// Where the top ends — see [`voices::shape`].
    brightness_hz: f32,
    drive: f32,
    body_hz: f32,
    /// The sub's root, in Hz, and how long it rings.
    sub_hz: f32,
    sub_len: f32,
    sub_drive: f32,
}

/// The families, and which records each one is for.
///
/// ⚠ Every model's `kit.preview` must name one of these — `datasetc` refuses a
/// name that is not here, because a typo used to mean silently playing trap.
const FAMILIES: &[Family] = &[
    Family {
        id: "trap-default",
        name: "Trap Default",
        seed: KIT_SEED,
        brightness_hz: 20_000.0,
        drive: 1.0,
        body_hz: 0.0,
        sub_hz: 41.2,
        sub_len: 1.4,
        sub_drive: 2.2,
    },
    Family {
        // Drier and tighter than trap, and the sub slides further — the whole
        // point of the drill 808 is that it moves.
        id: "drill-default",
        name: "Drill Default",
        seed: KIT_SEED ^ 0x11,
        brightness_hz: 15_000.0,
        drive: 1.4,
        body_hz: 90.0,
        sub_hz: 38.9,
        sub_len: 1.7,
        sub_drive: 2.6,
    },
    Family {
        // Distorted on purpose. Rage is the loudest thing in this list.
        id: "rage-default",
        name: "Rage Default",
        seed: KIT_SEED ^ 0x22,
        brightness_hz: 18_000.0,
        drive: 3.2,
        body_hz: 70.0,
        sub_hz: 43.7,
        sub_len: 1.2,
        sub_drive: 4.0,
    },
    Family {
        // Soft, warm and slightly dull — plugg is a lullaby with 808s under it.
        id: "plugg-default",
        name: "Plugg Default",
        seed: KIT_SEED ^ 0x33,
        brightness_hz: 11_000.0,
        drive: 1.0,
        body_hz: 120.0,
        sub_hz: 40.0,
        sub_len: 1.6,
        sub_drive: 1.6,
    },
    Family {
        // ⚠ 8 kHz is not a guess: the SP-1200 sampled at 26 kHz, so there is
        // nothing above ~8 kHz on a record made on one. That ceiling IS the
        // boom-bap drum sound, more than any single sample choice.
        id: "boom-bap-default",
        name: "Boom Bap Default",
        seed: KIT_SEED ^ 0x44,
        brightness_hz: 8_000.0,
        drive: 1.9,
        body_hz: 110.0,
        sub_hz: 49.0,
        sub_len: 0.8,
        sub_drive: 1.8,
    },
    Family {
        // Acoustic and open: nothing driven, full top, and a real ride.
        id: "country-default",
        name: "Country Default",
        seed: KIT_SEED ^ 0x55,
        brightness_hz: 20_000.0,
        drive: 1.0,
        body_hz: 0.0,
        sub_hz: 55.0,
        sub_len: 0.7,
        sub_drive: 1.0,
    },
    Family {
        // Smooth and rounded. R&B keeps the air but never bites.
        id: "rnb-default",
        name: "R&B Default",
        seed: KIT_SEED ^ 0x66,
        brightness_hz: 16_000.0,
        drive: 1.0,
        body_hz: 100.0,
        sub_hz: 46.2,
        sub_len: 1.1,
        sub_drive: 1.3,
    },
    Family {
        // Bright and punchy — jerk and west-coast club records are all transient.
        id: "club-default",
        name: "Club Default",
        seed: KIT_SEED ^ 0x77,
        brightness_hz: 20_000.0,
        drive: 1.6,
        body_hz: 80.0,
        sub_hz: 48.0,
        sub_len: 0.9,
        sub_drive: 2.0,
    },
    Family {
        // Lo-fi and crushed, cowbell forward. Phonk is a cassette.
        id: "phonk-default",
        name: "Phonk Default",
        seed: KIT_SEED ^ 0x88,
        brightness_hz: 9_500.0,
        drive: 3.6,
        body_hz: 95.0,
        sub_hz: 41.2,
        sub_len: 1.3,
        sub_drive: 3.4,
    },
    Family {
        // Fast and tight: everything short, nothing woolly.
        id: "dnb-default",
        name: "Drum & Bass Default",
        seed: KIT_SEED ^ 0x99,
        brightness_hz: 18_000.0,
        drive: 1.5,
        body_hz: 60.0,
        sub_hz: 36.7,
        sub_len: 2.0,
        sub_drive: 1.7,
    },
    Family {
        // Clean and polished, nothing driven at all.
        id: "pop-default",
        name: "Pop Default",
        seed: KIT_SEED ^ 0xAA,
        brightness_hz: 19_000.0,
        drive: 1.0,
        body_hz: 0.0,
        sub_hz: 55.0,
        sub_len: 0.9,
        sub_drive: 1.2,
    },
];

/// Every percussion voice for a family, shaped by its tone.
fn shaped(family: &Family, mut samples: Vec<f32>) -> Vec<f32> {
    voices::shape(
        &mut samples,
        family.brightness_hz,
        family.drive,
        family.body_hz,
    );
    samples
}

fn build_kit(family: &Family) -> Vec<Voice> {
    let seed = family.seed;
    let mut kit = build_trap_kit_at(seed, family);
    for voice in &mut kit {
        // ⚠ The pitched voices are left alone. Shaping is a *drum* room; running
        // a lead or a chord pad through boom-bap's 8 kHz ceiling would not read
        // as an era, it would read as a broken instrument.
        if matches!(voice.lane, "melody" | "counter" | "bass" | "chords" | "sub") {
            continue;
        }
        voice.samples = shaped(family, std::mem::take(&mut voice.samples));
    }
    kit
}

fn build_trap_kit_at(seed: u64, family: &Family) -> Vec<Voice> {
    vec![
        Voice {
            name: "kick",
            lane: "kick",
            samples: voices::kick(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            // E1 — the low end of the trap 808 register in research ch. 2.
            name: "808",
            // The lane is `sub` since TASK-140 — it is the pitched sliding
            // sub-bass, not the bass drum. The *file* stays `808.wav` because
            // that is what the sound is called, whichever machine it came from.
            lane: "sub",
            samples: voices::eight_o_eight(family.sub_hz, family.sub_len, family.sub_drive),
            choke_group: Some(2),
            root_note: Some(28),
        },
        Voice {
            name: "snare",
            lane: "snare",
            samples: voices::snare(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "clap",
            lane: "clap",
            samples: voices::clap(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "closed-hat",
            lane: "closedHat",
            samples: voices::closed_hat(seed),
            choke_group: Some(1),
            root_note: None,
        },
        Voice {
            name: "open-hat",
            lane: "openHat",
            samples: voices::open_hat(seed),
            choke_group: Some(1),
            root_note: None,
        },
        Voice {
            name: "rim",
            lane: "rim",
            samples: voices::rim(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "perc",
            lane: "perc",
            samples: voices::perc(seed),
            choke_group: None,
            root_note: None,
        },
        // ── The percussion lanes (TASK-140) ──────────────────────────────
        //
        // ⛔ **Without these the new lanes are silent, and every test still
        // passes.** `drums.rs` gained eight lanes; `uk-drill` writes
        // `woodblock`, `country-train` and `west-coast-club` write
        // `tambourine`, and `trap` has been asking for `snap` since it
        // shipped. `Kit::pad_for` answers `None` for a lane with no pad and
        // the trigger is simply skipped — which is the readout-that-lies
        // failure this project keeps recording.
        //
        // ⚠ **These are defaults.** A sample dropped on a lane replaces the
        // one below it and this must not sound underneath — see TASK-22.
        Voice {
            name: "off-snare",
            lane: "offSnare",
            samples: voices::off_snare(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "snap",
            lane: "snap",
            samples: voices::snap(seed),
            choke_group: None,
            root_note: None,
        },
        // ⚠ The cymbals share choke group 3 rather than the hats' group 1: a
        // ride and a crash ring into each other on a real kit, but neither is
        // closed by a hi-hat pedal and choking them with the hats would cut
        // every cymbal the moment a closed hat lands — which, on a 16th hat
        // stream, is always.
        Voice {
            name: "ride",
            lane: "ride",
            samples: voices::ride(seed),
            choke_group: Some(3),
            root_note: None,
        },
        Voice {
            name: "crash",
            lane: "crash",
            samples: voices::crash(seed),
            choke_group: Some(3),
            root_note: None,
        },
        Voice {
            name: "tom",
            lane: "tom",
            samples: voices::tom(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "shaker",
            lane: "shaker",
            samples: voices::shaker(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "tambourine",
            lane: "tambourine",
            samples: voices::tambourine(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "cowbell",
            lane: "cowbell",
            samples: voices::cowbell(seed),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "woodblock",
            lane: "woodblock",
            samples: voices::woodblock(seed),
            choke_group: None,
            root_note: None,
        },
        // ── The pitched pads (TASK-131) ──────────────────────────────────
        //
        // ⛔ **Without these four the melodic generators are silent**, because
        // `Kit::pad_for` answers `None` for a lane the kit has no pad for and
        // the trigger is simply skipped. A producer pressing Play on a melody
        // heard nothing and had no way to tell a silent kit from a broken
        // generator.
        //
        // ⚠ **Each root sits in the middle of that part's authored register**,
        // not at a tidy C, because the sampler transposes one sample and a
        // two-octave stretch thins it out. Melody is authored around C5–C7
        // (research ch. 2 §1 "bells C5–C7"), chords around C3–C5, and a bassline
        // around C1–G2 — so the roots below are the centre of each of those.
        //
        // ⚠ **Each is choke-free.** A melody holds while the next note starts;
        // choking it would cut every legato line into staccato.
        Voice {
            // C6 — the middle of the 72–96 lead register.
            name: "lead",
            lane: "melody",
            samples: voices::pluck(1046.5, 1.1, seed),
            choke_group: None,
            root_note: Some(84),
        },
        Voice {
            // C5 — the counter sits an octave under the lead as often as over it.
            name: "bell",
            lane: "counter",
            samples: voices::bell(523.25, 1.8),
            choke_group: None,
            root_note: Some(72),
        },
        Voice {
            // C2 — the middle of the 24–43 bass register.
            name: "bass",
            lane: "bass",
            samples: voices::synth_bass(65.41, 1.3),
            choke_group: None,
            root_note: Some(36),
        },
        Voice {
            // C4 — the middle of the 48–72 chord voicing register.
            name: "keys",
            lane: "chords",
            samples: voices::keys(261.63, 2.4),
            choke_group: None,
            root_note: Some(60),
        },
    ]
}

fn write_kit(
    out_dir: &Path,
    id: &str,
    name: &str,
    seed: u64,
    voices: Vec<Voice>,
) -> std::io::Result<()> {
    let dir = out_dir.join(id);
    fs::create_dir_all(&dir)?;

    let mut pads = Vec::new();
    for (index, voice) in voices.iter().enumerate() {
        let file = format!("{}.wav", voice.name);
        let path = dir.join(&file);
        wav::write_wav(BufWriter::new(File::create(&path)?), &voice.samples)?;

        let seconds = voice.samples.len() as f32 / wav::SAMPLE_RATE as f32;
        println!(
            "  {file:<16} {:>7.3}s  {:>7} samples",
            seconds,
            voice.samples.len()
        );

        pads.push(json!({
            "padIndex": index,
            "id": voice.name,
            "lane": voice.lane,
            "file": file,
            "gainDb": 0.0,
            "pitchSemis": 0,
            "pan": 0.0,
            "chokeGroup": voice.choke_group,
            "rootNote": voice.root_note,
        }));
    }

    let manifest = json!({
        "id": id,
        "name": name,
        "sampleRate": wav::SAMPLE_RATE,
        "bitDepth": 24,
        "generatedBy": "kitgen",
        "seed": seed.to_string(),
        "license": "CC0-1.0",
        "notice": "Every sample here is synthesized from oscillators and filtered \
    noise by tools/kitgen. No recorded material is used, so the kit carries no \
    third-party rights. Regenerate with `cargo run -p kitgen`.",
        "pads": pads,
    });

    let manifest_path = dir.join("kit.json");
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    println!("  {:<16} manifest", "kit.json");

    Ok(())
}

fn main() -> ExitCode {
    let out_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "data/kits".to_string()),
    );

    for family in FAMILIES {
        println!("\n{} -> {}", family.id, out_dir.join(family.id).display());
        if let Err(error) = write_kit(
            &out_dir,
            family.id,
            family.name,
            family.seed,
            build_kit(family),
        ) {
            eprintln!("kitgen: {}: {error}", family.id);
            return ExitCode::FAILURE;
        }
    }

    match Ok::<(), std::io::Error>(()) {
        Ok(()) => {
            println!(
                "\nok: {} kits written. Audition before committing — these are the sounds a \
new user hears first.",
                FAMILIES.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
