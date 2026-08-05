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

fn build_trap_kit() -> Vec<Voice> {
    vec![
        Voice {
            name: "kick",
            lane: "kick",
            samples: voices::kick(),
            choke_group: None,
            root_note: None,
        },
        Voice {
            // E1 — the low end of the trap 808 register in research ch. 2.
            name: "808",
            lane: "bass808",
            samples: voices::eight_o_eight(41.2, 1.4, 2.2),
            choke_group: Some(2),
            root_note: Some(28),
        },
        Voice {
            name: "snare",
            lane: "snare",
            samples: voices::snare(KIT_SEED),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "clap",
            lane: "clap",
            samples: voices::clap(KIT_SEED),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "closed-hat",
            lane: "closedHat",
            samples: voices::closed_hat(KIT_SEED),
            choke_group: Some(1),
            root_note: None,
        },
        Voice {
            name: "open-hat",
            lane: "openHat",
            samples: voices::open_hat(KIT_SEED),
            choke_group: Some(1),
            root_note: None,
        },
        Voice {
            name: "rim",
            lane: "rim",
            samples: voices::rim(KIT_SEED),
            choke_group: None,
            root_note: None,
        },
        Voice {
            name: "perc",
            lane: "perc",
            samples: voices::perc(KIT_SEED),
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
            samples: voices::pluck(1046.5, 1.1, KIT_SEED),
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

fn write_kit(out_dir: &Path, id: &str, name: &str, voices: Vec<Voice>) -> std::io::Result<()> {
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
        "seed": KIT_SEED.to_string(),
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

    println!("trap-default -> {}", out_dir.join("trap-default").display());
    match write_kit(&out_dir, "trap-default", "Trap Default", build_trap_kit()) {
        Ok(()) => {
            println!(
                "\nok: kit written. Audition before committing — these are the sounds a \
new user hears first."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
