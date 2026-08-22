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

use crate::pad_tweaks::{PadShape, PadTweaks};

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
    /// Hundredths of a semitone on top of [`Self::pitch_semis`] (TASK-055A).
    ///
    /// ⚠ **A second field rather than a float `pitch_semis`**, because the two
    /// are different controls with different ranges — transposition and tuning —
    /// and folding them would let a cents nudge slide into an octave. The
    /// sampler adds them once when it works out the read rate.
    pub pitch_cents: i32,
    /// Pads sharing a group cut each other off, as a real hi-hat does.
    pub choke_group: Option<u8>,
    /// The MIDI note the sample was recorded at, for pads that carry pitch.
    ///
    /// `None` means percussion: the note's pitch is ignored and the pad plays
    /// as it was sampled. `Some` means the note's pitch is real and the voice
    /// is transposed to it — without which an 808 line plays monotone.
    pub root_note: Option<u8>,
    /// The trim window, the fades and the envelope (TASK-055A, TASK-164).
    ///
    /// ⚠ [`PadShape::default`] is the whole sample with no shaping, which is
    /// what every shipped pad and every untouched one-shot carries — see
    /// [`PadShape::is_plain`], which is how the audio thread skips the work.
    pub shape: PadShape,
    /// Where this pad may be looped while a note is held (TASK-053A).
    ///
    /// ⛔ **`None` on every shipped pad, and that is a measurement rather than
    /// a decision.** The kits are synthesized tones under decay envelopes, and
    /// [`crate::audio::sustain::find`] refuses a decay — so nothing this
    /// product already ships changes how it sounds. It becomes `Some` only for
    /// a producer's own sample with a genuine steady state, on a lane where
    /// holding a note means anything.
    pub loop_region: Option<(usize, usize)>,
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
    /// Whether [`Self::samples`] was flipped on the way in (2026-08-11).
    ///
    /// ⚠ **A record of what was done, not an instruction.** The buffer is already
    /// backwards — `oneshot::load` says why it is reversed there rather than at
    /// playback — so nothing reads this to decide how to sound the pad. It exists
    /// so the choice can be written into the project and reapplied on reload,
    /// which is the one thing the reversed samples cannot tell us themselves.
    pub reversed: bool,
    /// What note the sample is actually in, when it is in one (TASK-052).
    ///
    /// ⛔⛔ **This is what makes a one-shot on a melodic part play at the right
    /// octave rather than a plausible one.** [`Kit::with_one_shots`] inherits
    /// `root_note` from the pad the shipped kit ships for that lane — the lead
    /// pad is rooted at MIDI 84 because that is where the melody generator
    /// writes — so a violin sampled at C2 assigned there was transposed as
    /// though it were already a C6. That entry's own doc calls the inheritance
    /// *"sensible"* and names this task as what makes it exact.
    ///
    /// `None` for percussion and for anything with no clear pitch, which is a
    /// real answer and the common one: [`crate::audio::pitch::detect_root`]
    /// refuses noise rather than guessing, and it is only asked at all on the
    /// lanes [`crate::audio::pitch::applies_to`] names.
    pub root: Option<crate::audio::pitch::Root>,
    /// Where this sample can be looped while a note is held (TASK-053A).
    ///
    /// ⛔ **What makes a four-bar chord four bars long.** Without it a held note
    /// sounds for however long the file happens to be and then stops, so the
    /// piano roll's note lengths do nothing audible on a sustaining part.
    ///
    /// `None` for percussion, for plucks and stabs, and for anything else with
    /// no steady state — [`crate::audio::sustain::find`] refuses rather than
    /// approximating, because a loop point in the wrong place is a click on
    /// every held note, which is worse than the shortened note it replaces.
    pub loop_region: Option<(usize, usize)>,
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
                pitch_cents: base.map_or(0, |pad| pad.pitch_cents),
                choke_group: base.and_then(|pad| pad.choke_group),
                // ⛔ **The sample's OWN pitch wins over the pad's** (TASK-052).
                // Inheriting was the sensible fallback this doc describes; a
                // measured root is the exact answer. A sample with no clear
                // pitch — a vocal chop, a noisy pad — detects nothing and still
                // inherits, because a melodic lane with no root at all plays
                // every note as sampled, which is monotone and worse than the
                // octave error this replaces.
                //
                // ⛔⛔ **`applies_to` is asked HERE as well as in
                // `oneshot::load`, and that is not a duplicated rule.** In
                // `load` it is a *cost* decision — an NSDF over half a second
                // is real work and a kit re-roll decodes a dozen files. Here it
                // is the *correctness* one: this is the only place that holds
                // the lane and the measurement together at the moment a pad is
                // built, and a root on a kick makes the drum grid's hand-drawn
                // hits transpose it. Leaving the invariant to whoever happens to
                // construct the `OneShot` is how it gets broken by the next
                // caller that does not know it exists.
                root_note: one_shot
                    .root
                    .filter(|_| crate::audio::pitch::applies_to(*lane))
                    .map(|root| root.note)
                    .or_else(|| base.and_then(|pad| pad.root_note)),
                // ⛔ **Deliberately NOT inherited.** Everything else on this
                // line is a property of the *voice* the kit ships for this lane
                // — where it sits in the mix, what it is rooted at. A shape is a
                // window into a specific buffer, and this is a different buffer:
                // carrying a trim of "the last quarter" onto a sample a tenth
                // the length would play a producer's new kick from somewhere in
                // its tail. [`Kit::with_tweaks`] re-resolves it against the
                // audio that is actually here.
                shape: PadShape::default(),
                // ⛔ **The producer's sample brings its own loop, and it is
                // never inherited** (TASK-053A) — for the reason `shape` right
                // above is not: a loop region is a pair of offsets into a
                // *specific* buffer, and this is a different buffer. Carrying
                // the base pad's would loop somewhere arbitrary in the new
                // sample, which is the click this feature must not produce.
                //
                // ⛔ **Gated on the lane here as well as in `oneshot::load`**,
                // for exactly the reason `root_note` two lines up is: this is
                // the only place holding the lane and the measurement together
                // as a pad is built, and leaving the invariant to whoever
                // constructs the `OneShot` is how it gets broken by the next
                // caller that does not know it exists. ⚠ `is_melodic`, not
                // `applies_to` — a note can only be held on a lane whose notes
                // are gated by their own length, which excludes `Sub`.
                loop_region: one_shot
                    .loop_region
                    .filter(|_| crate::roles::is_melodic(*lane)),
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

    /// This kit with the producer's per-pad edits over it (TASK-055A, TASK-164).
    ///
    /// ⛔⛔ **Applied to EVERY pad, not only the ones carrying a one-shot**, and
    /// that is the difference between this and [`Self::with_one_shots`]. Mike
    /// asked for an editor on *"Kick, Sub Bass, Rim Shot, etc."* — the shipped
    /// voices — and the answer at the time was that the notes had an editor and
    /// the sound did not. A version of this that only reached assigned samples
    /// would leave that still true for every pad a producer had not replaced,
    /// which is most of them.
    ///
    /// ⛔ **Runs AFTER `with_one_shots`, never before.** The trim window and the
    /// normalize peak are both measurements of a specific buffer, so they have to
    /// be taken against the audio that will actually play — resolving them
    /// against the shipped kick and then swapping the producer's kick underneath
    /// would trim a sample nobody measured.
    ///
    /// ⚠ **A whole new kit, off the audio thread**, for the reason
    /// [`Self::with_one_shots`] gives in full: the sampler holds pad *indices*
    /// inside sounding voices, so a kit may never be edited underneath the
    /// callback.
    ///
    /// ⚠ An identity entry is skipped rather than applied, so a map that has
    /// accumulated defaults costs nothing and produces a byte-identical pad.
    pub fn with_tweaks(&self, tweaks: &BTreeMap<Lane, PadTweaks>) -> Kit {
        let mut kit = self.clone();
        for pad in &mut kit.pads {
            let Some(tweak) = tweaks.get(&pad.lane) else {
                continue;
            };
            if tweak.is_identity() {
                continue;
            }
            let tweak = tweak.clamped();
            let shape = tweak.shape_for(pad.samples.len(), pad.sample_rate);

            // ⚠ **Multiplied into the pad's own gain, not written over it.** The
            // manifest's level is what balances the kit — a clap authored 4 dB
            // under the snare stays 4 dB under it — and the producer's control is
            // an offset from that, which is what a mixer fader is.
            pad.gain *= tweak.gain_linear() * tweak.normalize_gain(&pad.samples, &shape);
            // ⛔ **Pan is REPLACED rather than added**, and the asymmetry is
            // deliberate: pan is a position, not an amount. Adding a producer's
            // centre (0.0) to a pad the kit placed left would leave it left while
            // the control read centre — a readout that lies about the one thing
            // it is for. Gain has a natural zero that means "no change"; pan's
            // zero means "the middle".
            pad.pan = tweak.pan;
            pad.pitch_semis += tweak.semis;
            pad.pitch_cents += tweak.cents;
            pad.shape = shape;
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
        // ⛔ **Looked up under the dir's own path first, and this is not
        // defensive coding.** `include_dir`'s `get_file` matches on paths
        // relative to the *macro root*, so a `Dir` obtained through `get_dir`
        // holds entries named `boom-bap-default/kit.json` while a `Dir` that
        // *is* the macro root holds plain `kit.json`. `PREVIEW_KIT` is the
        // second kind and `ALL_KITS.get_dir(..)` is the first, so a single
        // spelling works for exactly one of them — which is how the family
        // kits decoded as "no readable kit.json" the first time.
        let find = |name: &str| {
            dir.get_file(dir.path().join(name))
                .or_else(|| dir.get_file(name))
        };

        let text = find("kit.json")
            .and_then(|file| file.contents_utf8())
            .ok_or("the embedded kit has no readable kit.json")?;
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|e| format!("kit.json is not a manifest: {e}"))?;

        let mut pads = Vec::with_capacity(manifest.pads.len());
        for pad in manifest.pads {
            let bytes = find(&pad.file)
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
                // ⚠ Not in the manifest and deliberately so: cents are for
                // tuning a producer's own sample, and a shipped kit that needed
                // them would be a kit that was synthesized out of tune.
                pitch_cents: 0,
                choke_group: pad.choke_group,
                root_note: pad.root_note,
                shape: PadShape::default(),
                // A shipped pad is a synthesized tone under a decay envelope,
                // and `sustain::find` refuses a decay — so measuring one here
                // would answer None on every pad in every kit at a cost paid on
                // every plugin start. See the field for why that is a
                // measurement rather than a decision.
                loop_region: None,
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
    use crate::pad_tweaks::Adsr;

    /// The shipped kit, out of the binary rather than off disk — the same
    /// bytes the plugin actually plays, so a kit that ships broken fails here.
    fn shipped() -> Kit {
        Kit::embedded(&crate::audio::PREVIEW_KIT).expect("the shipped kit must load")
    }

    /// The shipped kit with one lane's pad taken out.
    ///
    /// ⛔ **Built rather than found, and TASK-140 is why.** The refusal rule
    /// below used to be tested against `Lane::Snap`, which happened to ship
    /// with no pad. Every lane has a default now, so that premise is gone —
    /// and it was never a good one: it tied a rule about `pad_for` to an
    /// *accident* of what the kit happened to cover, so filling the gap looked
    /// like breaking the rule. A user kit or a future genre kit can still lack
    /// a lane, and this is what keeps that path honest.
    fn shipped_without(lane: Lane) -> Kit {
        let mut kit = shipped();
        kit.pads.retain(|pad| pad.lane != lane);
        kit
    }

    #[test]
    fn the_shipped_kit_loads_with_every_pad_audible() {
        let kit = shipped();
        assert_eq!(kit.id, "trap-default");
        assert_eq!(kit.pads.len(), crate::shared::ALL_LANES.len());

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
        assert_eq!(pad(Lane::Sub).root_note, Some(28));
        for lane in [Lane::Kick, Lane::Snare, Lane::ClosedHat, Lane::Perc] {
            assert_eq!(pad(lane).root_note, None, "{lane:?} has no pitch to play");
        }
    }

    #[test]
    fn a_lane_the_kit_has_no_pad_for_is_none_rather_than_the_nearest_drum() {
        // A lane the kit does not cover must answer `None` rather than the
        // nearest drum: a wrong drum is harder to notice than silence, and
        // harder to explain once it is noticed.
        //
        // ⚠ The kit is built without `Snap` rather than relying on the shipped
        // one to lack it — see `shipped_without`. Every lane has a default
        // since TASK-140, so the old premise no longer holds and the rule is
        // now stated directly instead of borrowed from a gap.
        let kit = shipped_without(Lane::Snap);
        assert!(kit.pad_for(Lane::Kick).is_some());
        assert_eq!(kit.pad_for(Lane::Snap), None);

        // And the shipped kit genuinely does cover it now.
        assert!(
            shipped().pad_for(Lane::Snap).is_some(),
            "snap shipped silent for the whole of TASK-131; TASK-140 gave it a voice"
        );
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
            Lane::Sub,
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

    // ── Per-pad edits (TASK-055A, TASK-164) ────────────────────────────────

    /// The tweaks a producer might actually dial in, as one block.
    fn shaped() -> PadTweaks {
        PadTweaks {
            gain_db: -6.0,
            pan: -1.0,
            semis: -2,
            cents: 25,
            trim_start: 0.25,
            trim_end: 0.75,
            adsr: Adsr {
                decay_ms: 195.0,
                sustain_db: -36.0,
                ..Adsr::default()
            },
            ..PadTweaks::default()
        }
    }

    #[test]
    fn a_pad_edit_reaches_a_lane_the_producer_never_replaced() {
        // ⛔⛔ **The whole point of TASK-164.** Mike asked whether the drum lanes
        // had an editor — *"Kick, Sub Bass, Rim Shot, etc."* — and the answer was
        // that the notes had one and the sound did not. A version of this that
        // only reached assigned one-shots would leave that still true for every
        // pad nobody had swapped, which is most of them.
        let kit = shipped().with_tweaks(&BTreeMap::from([(Lane::Kick, shaped())]));
        let pad = &kit.pads[kit.pad_for(Lane::Kick).expect("the kit ships a kick")];

        assert_eq!(pad.pitch_semis, -2, "the shipped kick transposes");
        assert_eq!(pad.pitch_cents, 25);
        assert_eq!(pad.pan, -1.0);
        assert!(pad.shape.adsr.is_some(), "and it carries the envelope");
    }

    #[test]
    fn gain_is_an_offset_from_the_kit_and_pan_is_a_position() {
        // ⛔ **The asymmetry is deliberate.** The manifest's level is what
        // balances the kit — a clap authored 4 dB under the snare stays 4 dB
        // under it — so the producer's gain multiplies. Pan does not: adding a
        // producer's centre (0.0) to a pad the kit placed left would leave it
        // left while the control read centre, which is a readout that lies about
        // the one thing it is for.
        let base = shipped();
        let at = base.pad_for(Lane::Kick).expect("the kit ships a kick");
        let before = base.pads[at].gain;

        let kit = base.with_tweaks(&BTreeMap::from([(
            Lane::Kick,
            PadTweaks {
                gain_db: -6.0,
                pan: 0.0,
                ..PadTweaks::default()
            },
        )]));
        let after = &kit.pads[at];
        assert!(
            (after.gain - before * crate::pad_tweaks::db_to_linear(-6.0)).abs() < 1e-6,
            "gain multiplies the kit's own level"
        );
        assert_eq!(after.pan, 0.0, "pan is replaced, not added");
    }

    #[test]
    fn an_untouched_pad_comes_through_byte_for_byte() {
        // ⛔ Every shipped kit and every pad nobody has opened goes through this
        // function. If an identity entry changed anything, adding an editor
        // would have changed how the product sounds for everyone who never
        // opened it.
        let base = shipped();
        let kit = base.with_tweaks(&BTreeMap::from([(Lane::Kick, PadTweaks::default())]));

        for (before, after) in base.pads.iter().zip(&kit.pads) {
            assert_eq!(before.gain, after.gain, "{}", before.id);
            assert_eq!(before.pan, after.pan, "{}", before.id);
            assert_eq!(before.pitch_semis, after.pitch_semis, "{}", before.id);
            assert_eq!(before.pitch_cents, after.pitch_cents, "{}", before.id);
            assert!(after.shape.is_plain(), "{}", before.id);
        }
    }

    #[test]
    fn the_trim_window_is_resolved_against_the_buffer_that_will_actually_play() {
        // ⛔ A fraction, not an index — the producer dragged a handle to a place
        // in a waveform. Resolving it against this pad's own length is what
        // makes the same trim mean the same thing after the sample is replaced.
        let kit = shipped().with_tweaks(&BTreeMap::from([(Lane::Kick, shaped())]));
        let pad = &kit.pads[kit.pad_for(Lane::Kick).expect("the kit ships a kick")];
        let len = pad.samples.len();

        assert_eq!(pad.shape.start, (0.25 * len as f32) as u32);
        assert_eq!(pad.shape.end, (0.75 * len as f32) as u32);
        assert!(pad.shape.end > pad.shape.start);
    }

    #[test]
    fn normalize_measures_the_trimmed_window_rather_than_the_whole_file() {
        // ⚠ Normalizing to a peak the producer has just trimmed away leaves the
        // audible part quiet and the control looking broken.
        let mut kit = shipped();
        let at = kit.pad_for(Lane::Kick).expect("the kit ships a kick");
        // Loud at the front, quiet behind — and the trim keeps only the quiet.
        let mut samples = vec![0.25f32; 100];
        samples[0] = 1.0;
        kit.pads[at].samples = Arc::from(samples.into_boxed_slice());
        kit.pads[at].gain = 1.0;

        let trimmed = kit.with_tweaks(&BTreeMap::from([(
            Lane::Kick,
            PadTweaks {
                normalize: true,
                trim_start: 0.5,
                ..PadTweaks::default()
            },
        )]));
        assert!(
            (trimmed.pads[at].gain - 4.0).abs() < 1e-4,
            "a 0.25 peak inside the window must come up to full scale, got {}",
            trimmed.pads[at].gain
        );
    }

    #[test]
    fn a_silent_pad_is_not_normalised_to_infinity() {
        // ⛔ `1.0 / 0.0` is an infinite gain on the audio thread. A silent pad
        // stays silent.
        let mut kit = shipped();
        let at = kit.pad_for(Lane::Kick).expect("the kit ships a kick");
        kit.pads[at].samples = Arc::from(vec![0.0f32; 64].into_boxed_slice());
        let before = kit.pads[at].gain;

        let kit = kit.with_tweaks(&BTreeMap::from([(
            Lane::Kick,
            PadTweaks {
                normalize: true,
                ..PadTweaks::default()
            },
        )]));
        assert_eq!(kit.pads[at].gain, before);
    }

    #[test]
    fn a_one_shot_does_not_inherit_the_trim_measured_against_a_different_sample() {
        // ⛔⛔ A shape is a window into a *specific* buffer. Carrying a trim of
        // "the last quarter" onto a sample a tenth the length would play a
        // producer's new kick from somewhere in its tail — and `with_one_shots`
        // is where that inheritance would have happened silently.
        let shaped_kit = shipped().with_tweaks(&BTreeMap::from([(Lane::Kick, shaped())]));
        let assigned = BTreeMap::from([(
            Lane::Kick,
            OneShot {
                path: "C:/samples/new-kick.wav".into(),
                name: "new-kick.wav".into(),
                samples: Arc::from(vec![0.5f32; 16].into_boxed_slice()),
                sample_rate: 48_000,
                reversed: false,
                // A kick has no root to detect — see `pitch::applies_to`, and
                // nothing that rings out can reach a loop point either.
                root: None,
                loop_region: None,
            },
        )]);

        let kit = shaped_kit.with_one_shots(&assigned);
        let pad = &kit.pads[kit.pad_for(Lane::Kick).expect("the kit ships a kick")];
        assert!(
            pad.shape.is_plain(),
            "the new sample must arrive unwindowed and be re-resolved"
        );
    }
}
