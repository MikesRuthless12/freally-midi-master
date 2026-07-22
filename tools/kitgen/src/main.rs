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
}

fn build_trap_kit() -> Vec<Voice> {
    vec![
        Voice {
            name: "kick",
            lane: "kick",
            samples: voices::kick(),
            choke_group: None,
        },
        Voice {
            // E1 — the low end of the trap 808 register in research ch. 2.
            name: "808",
            lane: "bass808",
            samples: voices::eight_o_eight(41.2, 1.4, 2.2),
            choke_group: Some(2),
        },
        Voice {
            name: "snare",
            lane: "snare",
            samples: voices::snare(KIT_SEED),
            choke_group: None,
        },
        Voice {
            name: "clap",
            lane: "clap",
            samples: voices::clap(KIT_SEED),
            choke_group: None,
        },
        Voice {
            name: "closed-hat",
            lane: "closedHat",
            samples: voices::closed_hat(KIT_SEED),
            choke_group: Some(1),
        },
        Voice {
            name: "open-hat",
            lane: "openHat",
            samples: voices::open_hat(KIT_SEED),
            choke_group: Some(1),
        },
        Voice {
            name: "rim",
            lane: "rim",
            samples: voices::rim(KIT_SEED),
            choke_group: None,
        },
        Voice {
            name: "perc",
            lane: "perc",
            samples: voices::perc(KIT_SEED),
            choke_group: None,
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
