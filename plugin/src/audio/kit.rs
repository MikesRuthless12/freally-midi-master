//! The preview kit: a pad per lane, decoded once into memory.
//!
//! Loading happens on whatever thread asks for it and never on the audio
//! thread — decoding allocates, and the audio callback may not. What the audio
//! thread gets is a finished [`Kit`] it only ever reads.
//!
//! The kits are the ones `tools/kitgen` synthesizes and the repo commits, so
//! the format is known exactly: mono 24-bit PCM at 44.1 kHz. The reader below
//! accepts a little more than that (16-bit, and stereo by downmix) because
//! those are the two ways a replaced file could still be a valid WAV while
//! sounding like noise or silence — an error message is better than either.
//! Decoding *imported* samples is a different problem with its own crate
//! (`symphonia`) and arrives with sample import.

use std::collections::BTreeMap;
use std::sync::Arc;

use engine::pattern::Lane;
use include_dir::Dir;
use serde::Deserialize;

/// One pad: a sample plus how it should be played.
///
/// `Clone` is cheap and has to be: [`Kit::with_one_shots`] rebuilds the whole
/// kit every time a producer assigns a sample, and `samples` is an `Arc<[f32]>`
/// precisely so that copying a pad copies a reference count rather than a
/// megabyte of audio.
#[derive(Clone)]
pub struct Pad {
    pub id: String,
    pub lane: Lane,
    /// Mono f32 in [-1, 1], at [`Pad::sample_rate`].
    pub samples: Arc<[f32]>,
    pub sample_rate: u32,
    /// Linear, already converted from the manifest's decibels.
    pub gain: f32,
    /// -1 hard left, 0 centre, +1 hard right.
    pub pan: f32,
    pub pitch_semis: i32,
    /// Pads sharing a group cut each other off, as a real hi-hat does.
    pub choke_group: Option<u8>,
    /// The MIDI note the sample was recorded at, for pads that carry pitch.
    ///
    /// `None` means percussion: the note's pitch is ignored and the pad plays
    /// as it was sampled. `Some` means the note's pitch is real and the voice
    /// is transposed to it — without which an 808 line plays monotone.
    pub root_note: Option<u8>,
}

#[derive(Clone)]
pub struct Kit {
    pub id: String,
    pub pads: Vec<Pad>,
}

/// ⚠ **Written out rather than derived, because a derived one would print the
/// audio.** A kit is megabytes of samples; `{:?}` on the struct that holds it —
/// which `Shared` does derive — would dump every one of them into a log.
impl std::fmt::Debug for Kit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kit")
            .field("id", &self.id)
            .field("pads", &self.pads.len())
            .finish()
    }
}

/// A sample the producer assigned to a lane (TASK-131B).
///
/// Decoded once, on the loader thread, and then only ever cloned — `samples` is
/// an `Arc<[f32]>` so rebuilding the kit after a *second* assignment does not
/// re-copy the audio of the first.
#[derive(Clone)]
pub struct OneShot {
    /// Where the file came from. This is what the project file stores, and it
    /// is what a reopen reloads from — see [`crate::state::PluginSession`].
    pub path: String,
    /// The file's own name, which is what the KIT panel shows. Held rather than
    /// derived from `path` on every read, because a panel that shows a full
    /// path shows nothing useful in the width a rail has.
    pub name: String,
    pub samples: Arc<[f32]>,
    pub sample_rate: u32,
}

/// Written out for the same reason [`Kit`]'s is: a derived one would print the
/// whole sample.
impl std::fmt::Debug for OneShot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneShot")
            .field("path", &self.path)
            .field("name", &self.name)
            .field("samples", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .finish()
    }
}

impl Kit {
    /// The pad that plays a lane, or `None` if this kit has no voice for it.
    ///
    /// Returned rather than defaulted: a lane silently mapped to the nearest
    /// other pad plays the wrong drum, which is harder to notice than silence
    /// and harder still to explain.
    pub fn pad_for(&self, lane: Lane) -> Option<usize> {
        self.pads.iter().position(|pad| pad.lane == lane)
    }

    /// How far to transpose `pitch` when it is played on `lane`.
    ///
    /// ⛔ **One definition, because there were two and they were already
    /// diverging.** `render_preview` and the offline stem renderer each spelled
    /// this rule out separately, and `render.rs` asserted the invariant in prose
    /// — "the same rule `render_preview` follows, so a stem sounds like the
    /// preview did" — with nothing holding it. Two spellings of one rule is how
    /// an exported stem quietly stops matching what the producer auditioned.
    ///
    /// ⛔ **A `0` on an unpitched lane means "as sampled", not "36 semitones
    /// down".** The drum grid places a hand-drawn hit with the pitch of a note
    /// already in that lane, and has none to copy when the lane was just
    /// emptied. Treating that as a real MIDI note transposed a kick down by its
    /// own GM number — sub-rumble instead of a drum. MIDI note 0 is not a drum
    /// any lane maps to, so it is safe to read as "no opinion".
    pub fn semitones_for(pad: &Pad, lane: Lane, pitch: u8) -> f32 {
        match pad.root_note {
            Some(root) => f32::from(pitch) - f32::from(root),
            None if pitch == 0 => 0.0,
            None => f32::from(pitch) - f32::from(crate::midi_note_for(lane)),
        }
    }

    /// This kit with the producer's own one-shots played instead of its own
    /// (TASK-131B).
    ///
    /// ⛔ **A whole new kit, built off the audio thread, never a mutation of a
    /// live one.** The sampler addresses pads *by index* and holds those indices
    /// inside sounding voices, so editing a kit underneath the callback is how a
    /// voice ends up playing a sample that is not the one it was triggered with.
    /// The finished kit is handed over through [`crate::shared::KitHandoff`],
    /// which is also what cuts the voices that were addressing the old one.
    ///
    /// ## What is inherited, and why
    ///
    /// Everything except the audio: gain, pan, the pitch offset, the choke
    /// group and — the one that matters — **the root note**.
    ///
    /// ⛔ **Inheriting the root is what makes a one-shot on a melodic part play
    /// at roughly its own pitch.** The lead pad is rooted at MIDI 84 because
    /// that is where the melody generator writes; a sample assigned there is
    /// therefore transposed by the *interval* the melody moves, not shifted into
    /// another octave. Rooting a new sample at a fixed C3 instead would pitch it
    /// two octaves up under that same melody, which is the chipmunk failure, and
    /// the producer would have no control that fixes it. Detecting the sample's
    /// real pitch is TASK-052 and is what makes this exact rather than sensible.
    ///
    /// A lane the shipped kit has **no** pad for gets a plain percussion pad:
    /// unity gain, centred, and no root, so its notes play as sampled. Today
    /// `Lane::Snap` is the only such lane — the drum generator can write it and
    /// the kit has never carried it, so assigning one is the only way to hear
    /// that lane at all.
    pub fn with_one_shots(&self, assigned: &BTreeMap<Lane, OneShot>) -> Kit {
        let mut kit = self.clone();
        for (lane, one_shot) in assigned {
            let replacement = |base: Option<&Pad>| Pad {
                id: one_shot.name.clone(),
                lane: *lane,
                samples: Arc::clone(&one_shot.samples),
                sample_rate: one_shot.sample_rate,
                gain: base.map_or(1.0, |pad| pad.gain),
                pan: base.map_or(0.0, |pad| pad.pan),
                pitch_semis: base.map_or(0, |pad| pad.pitch_semis),
                choke_group: base.and_then(|pad| pad.choke_group),
                root_note: base.and_then(|pad| pad.root_note),
            };

            match kit.pad_for(*lane) {
                Some(index) => {
                    let pad = replacement(Some(&kit.pads[index]));
                    kit.pads[index] = pad;
                }
                None => kit.pads.push(replacement(None)),
            }
        }
        kit
    }

    /// Load `<dir>/kit.json` and every sample it names, out of the binary.
    ///
    /// ⛔ **From an embedded directory, not from disk — a plugin has no
    /// resource path it can trust.** It is a shared library inside someone
    /// else's process: no install layout, no working directory it chose, and a
    /// host that may have copied the bundle anywhere. The dataset and the UI
    /// are compiled in for exactly this reason and the preview kit is the third
    /// thing that has to be.
    ///
    /// It costs ~1.2 MB since the four pitched voices landed (TASK-131), up
    /// from 400 KB, and the samples are `kitgen`'s own synthesis — CC0,
    /// with no recorded material and so no third-party rights riding along
    /// inside the binary.
    pub fn embedded(dir: &Dir<'_>) -> Result<Kit, String> {
        let text = dir
            .get_file("kit.json")
            .and_then(|file| file.contents_utf8())
            .ok_or("the embedded kit has no readable kit.json")?;
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|e| format!("kit.json is not a manifest: {e}"))?;

        let mut pads = Vec::with_capacity(manifest.pads.len());
        for pad in manifest.pads {
            let bytes = dir
                .get_file(&pad.file)
                .map(|file| file.contents())
                .ok_or_else(|| format!("the kit names {} and does not carry it", pad.file))?;
            let audio =
                decode_wav(bytes).map_err(|e| format!("{} is not usable: {e}", pad.file))?;

            pads.push(Pad {
                id: pad.id,
                lane: pad.lane,
                samples: audio.samples.into(),
                sample_rate: audio.sample_rate,
                gain: db_to_gain(pad.gain_db),
                pan: pad.pan.clamp(-1.0, 1.0),
                pitch_semis: pad.pitch_semis,
                choke_group: pad.choke_group,
                root_note: pad.root_note,
            });
        }

        if pads.is_empty() {
            return Err("the embedded kit has no pads".into());
        }
        Ok(Kit {
            id: manifest.id,
            pads,
        })
    }
}

/// Decibels to a linear multiplier. 0 dB is unity, which is what every shipped
/// pad authors.
fn db_to_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

#[derive(Deserialize)]
struct Manifest {
    id: String,
    pads: Vec<ManifestPad>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestPad {
    id: String,
    file: String,
    /// The engine lane this pad plays. Deserialized as `Lane`, so a manifest
    /// naming a lane the engine does not have fails the load rather than
    /// producing a pad nothing ever triggers.
    lane: Lane,
    #[serde(default)]
    gain_db: f32,
    #[serde(default)]
    pan: f32,
    #[serde(default)]
    pitch_semis: i32,
    #[serde(default)]
    choke_group: Option<u8>,
    #[serde(default)]
    root_note: Option<u8>,
}

#[derive(Debug)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Decode a PCM WAV into mono f32.
pub fn decode_wav(bytes: &[u8]) -> Result<DecodedAudio, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }

    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut format = 0u16;
    let mut data: Option<&[u8]> = None;

    // Walk the chunk list rather than assuming fmt-then-data at fixed offsets:
    // a WAV is allowed to carry LIST/fact chunks in between, and a fixed offset
    // reads those as audio.
    let mut cursor = 12usize;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        let body_start = cursor + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];

        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err("the fmt chunk is truncated".into());
                }
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]);
                sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }

        // Chunks are word-aligned: an odd size is followed by a pad byte.
        cursor = body_start + size + (size & 1);
    }

    // Absence first. Reading the defaults instead reports a file with no fmt
    // chunk at all as "compressed format 0", which sends the reader looking for
    // a codec problem that does not exist.
    if channels == 0 || sample_rate == 0 || bits == 0 {
        return Err("there is no fmt chunk, so nothing describes the audio".into());
    }
    // 1 is PCM; 0xFFFE is WAVE_FORMAT_EXTENSIBLE, which kitgen does not write
    // but which is still plain PCM in the layouts we accept.
    if format != 1 && format != 0xFFFE {
        return Err(format!("compressed WAV (format {format}) is not supported"));
    }
    let data = data.ok_or("there is no data chunk")?;

    let frames: Vec<f32> = match bits {
        24 => data
            .chunks_exact(3)
            .map(|b| {
                // Sign-extend 24 bits into an i32 by putting them in the top
                // three bytes and shifting back down.
                let raw = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
                raw as f32 / 8_388_608.0
            })
            .collect(),
        16 => data
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0)
            .collect(),
        other => return Err(format!("{other}-bit samples are not supported")),
    };

    if frames.is_empty() {
        return Err("the data chunk is empty".into());
    }

    // Downmix by averaging. Summing would clip a correlated stereo pair.
    let samples = if channels == 1 {
        frames
    } else {
        let n = usize::from(channels);
        frames
            .chunks(n)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    Ok(DecodedAudio {
        samples,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped kit, out of the binary rather than off disk — the same
    /// bytes the plugin actually plays, so a kit that ships broken fails here.
    fn shipped() -> Kit {
        Kit::embedded(&crate::audio::PREVIEW_KIT).expect("the shipped kit must load")
    }

    #[test]
    fn the_shipped_kit_loads_with_every_pad_audible() {
        let kit = shipped();
        assert_eq!(kit.id, "trap-default");
        assert_eq!(kit.pads.len(), 12);

        for pad in &kit.pads {
            assert!(!pad.samples.is_empty(), "{} decoded to nothing", pad.id);
            assert_eq!(pad.sample_rate, 44_100, "{}", pad.id);
            // Silence would decode "successfully" and play nothing, which is
            // exactly the failure a load test is supposed to catch.
            let peak = pad.samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert!(
                peak > 0.05,
                "{} is effectively silent (peak {peak})",
                pad.id
            );
            assert!(peak <= 1.0, "{} is past full scale (peak {peak})", pad.id);
        }
    }

    #[test]
    fn the_hats_choke_each_other_and_nothing_else_does() {
        // An open hat that rings through the closed hat under it is the single
        // most obvious way a drum preview sounds wrong.
        let kit = shipped();
        let group = |lane: Lane| kit.pads[kit.pad_for(lane).unwrap()].choke_group;

        assert!(group(Lane::ClosedHat).is_some());
        assert_eq!(group(Lane::ClosedHat), group(Lane::OpenHat));
        assert_ne!(group(Lane::Kick), group(Lane::ClosedHat));
        assert_eq!(group(Lane::Kick), None);
        assert_eq!(group(Lane::Snare), None);
    }

    #[test]
    fn the_pitched_pad_says_what_note_it_is_and_the_drums_do_not() {
        // Without a root the sampler has nothing to transpose from, so every
        // 808 note plays at the pitch the sample happens to be and the whole
        // bassline comes out monotone — in tune with nothing, and only
        // audible as "wrong" rather than as a missing field.
        let kit = shipped();
        let pad = |lane: Lane| &kit.pads[kit.pad_for(lane).unwrap()];

        // E1: the low end of the trap 808 register, and what kitgen renders.
        assert_eq!(pad(Lane::Bass808).root_note, Some(28));
        for lane in [Lane::Kick, Lane::Snare, Lane::ClosedHat, Lane::Perc] {
            assert_eq!(pad(lane).root_note, None, "{lane:?} has no pitch to play");
        }
    }

    #[test]
    fn a_lane_the_kit_has_no_pad_for_is_none_rather_than_the_nearest_drum() {
        let kit = shipped();
        assert!(kit.pad_for(Lane::Kick).is_some());
        // ⚠ `Snap` rather than `Melody`: the melodic lanes gained pads in
        // TASK-131, and this test is about the *refusal* — a lane the kit does
        // not cover must answer `None` rather than the nearest drum, because a
        // wrong drum is harder to notice than silence and harder to explain.
        assert_eq!(kit.pad_for(Lane::Snap), None);
    }

    #[test]
    fn every_generated_part_has_a_voice_to_sound_through() {
        // ⛔ **The gate TASK-131 exists to hold.** `pad_for` answering `None`
        // means the trigger is skipped entirely, so a producer pressing Play on
        // a melody heard nothing and had no way to tell a silent kit from a
        // broken generator. That was true of four lanes out of five.
        let kit = shipped();
        for lane in [
            Lane::Kick,
            Lane::Bass808,
            Lane::Melody,
            Lane::Counter,
            Lane::Bass,
            Lane::Chords,
        ] {
            assert!(
                kit.pad_for(lane).is_some(),
                "{lane:?} has no pad, so that part plays silence"
            );
        }

        // ...and every pitched pad declares the root it was synthesized at, or
        // the sampler has nothing to transpose from and the part comes out
        // monotone — the same failure the 808 had before it carried one.
        for lane in [Lane::Melody, Lane::Counter, Lane::Bass, Lane::Chords] {
            let index = kit.pad_for(lane).unwrap();
            assert!(
                kit.pads[index].root_note.is_some(),
                "{lane:?} is pitched and must name its root note"
            );
        }
    }

    #[test]
    fn a_truncated_or_foreign_file_is_refused_with_a_reason() {
        assert!(decode_wav(b"").is_err());
        // A header and nothing else: the reason must be the missing fmt chunk,
        // not a guess about the codec.
        assert!(decode_wav(b"RIFFxxxxWAVE")
            .unwrap_err()
            .contains("no fmt chunk"));

        // A real header describing a codec we cannot decode.
        let mut mp3ish = b"RIFF\x24\x00\x00\x00WAVEfmt \x10\x00\x00\x00".to_vec();
        mp3ish.extend_from_slice(&85u16.to_le_bytes()); // MPEG Layer 3
        mp3ish.extend_from_slice(&1u16.to_le_bytes());
        mp3ish.extend_from_slice(&44_100u32.to_le_bytes());
        mp3ish.extend_from_slice(&[0; 6]);
        mp3ish.extend_from_slice(&16u16.to_le_bytes());
        assert!(decode_wav(&mp3ish).unwrap_err().contains("compressed"));
    }

    #[test]
    fn a_chunk_between_fmt_and_data_does_not_become_audio() {
        // A LIST chunk in the middle is legal and common. Reading data at a
        // fixed offset would decode the metadata as samples — a burst of noise
        // where the attack should be.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes()); // size is not trusted
        wav.extend_from_slice(b"WAVE");

        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&[0; 6]);
        wav.extend_from_slice(&16u16.to_le_bytes());

        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&5u32.to_le_bytes());
        wav.extend_from_slice(b"INFOx"); // odd size, so a pad byte follows
        wav.push(0);

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&i16::MAX.to_le_bytes());
        wav.extend_from_slice(&i16::MIN.to_le_bytes());

        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 2);
        assert!((decoded.samples[0] - 0.999).abs() < 0.01);
        assert!((decoded.samples[1] + 1.0).abs() < 0.01);
    }

    #[test]
    fn stereo_is_downmixed_rather_than_played_at_double_speed() {
        // Interleaved frames read as mono would play twice as fast and an
        // octave up, which sounds like a broken sample rather than a bug.
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes()); // stereo
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&[0; 6]);
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&8u32.to_le_bytes());
        for v in [i16::MAX, 0i16, i16::MIN, 0i16] {
            wav.extend_from_slice(&v.to_le_bytes());
        }

        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(
            decoded.samples.len(),
            2,
            "two stereo frames are two samples"
        );
        assert!(
            (decoded.samples[0] - 0.5).abs() < 0.01,
            "averaged, not summed"
        );
        assert!((decoded.samples[1] + 0.5).abs() < 0.01);
    }

    #[test]
    fn decibels_become_a_linear_multiplier() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!((db_to_gain(-6.0) - 0.501).abs() < 0.01);
        assert!((db_to_gain(6.0) - 1.995).abs() < 0.01);
    }
}
