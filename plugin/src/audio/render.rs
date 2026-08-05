//! Rendering a pattern to audio, offline (TASK-131F).
//!
//! Mike, 2026-08-05: *"i also need to be able to export the drums by themselves
//! as midi or audio stems or drag them into a daw as midi or audio stems."*
//!
//! ⛔ **This was blocked until TASK-131A/131B, and the block was measured rather
//! than assumed.** `audio::Kit::pad_for` answered `None` for Melody, Counter,
//! Bass and Chords, so rendering those parts would have written four silent
//! files and called them stems — which is worse than not offering them, because
//! a producer imports them, hears nothing, and blames their DAW. That is
//! verbatim why TASK-069 shipped MIDI stems instead. The shipped kit now covers
//! every generated lane, and a producer's own one-shots cover the rest, so the
//! honest answer changed.
//!
//! ## Offline, not real time
//!
//! Nothing here runs on the audio thread. It allocates a whole buffer, walks the
//! pattern once, and hands back bytes — the same [`sampler::Sampler`] the
//! callback uses, driven by a loop instead of by the host. That is the point of
//! the sampler being a pure function of its inputs.

use engine::pattern::{Pattern, PPQ};

use super::kit::Kit;
use super::sampler::{self, Sampler};

/// The rate stems are written at. 44.1 kHz because that is what the kit is, so
/// the common case resamples nothing.
pub const RATE: u32 = 44_100;

/// How long a stem may be, in seconds.
///
/// ⛔ **A bound, because a `Pattern` can arrive from a project file.** Bars and
/// tempo are both attacker-controlled through the same door `check_song`
/// already guards, and `bars * ticks_per_bar` at 60 BPM is minutes of stereo
/// f32 per part. Five minutes is far past any loop the plugin generates.
const MAX_SECONDS: u32 = 300;

/// Let every voice finish rather than cutting the last hit dead.
const TAIL_SECONDS: f64 = 2.0;

/// Render one pattern through `kit` into interleaved stereo f32.
///
/// ⚠ **Silence is returned as `None`, not as a buffer of zeros.** A lane the kit
/// has no pad for renders nothing, and writing that to disk is the "four silent
/// files called stems" failure this module's header exists to record. The caller
/// skips the file instead.
pub fn to_stereo(pattern: &Pattern, kit: &Kit) -> Option<Vec<f32>> {
    let bpm = if pattern.bpm.is_finite() && pattern.bpm > 1.0 {
        f64::from(pattern.bpm)
    } else {
        // A pattern with no usable tempo still has to render *something*
        // rather than divide by zero; 120 is the neutral guess and only a
        // corrupt project reaches it.
        120.0
    };
    let seconds_per_tick = 60.0 / bpm / f64::from(PPQ);

    // ⛔ Mirrors `SessionContext::ticks_per_bar` and `Song::ticks_per_bar`
    // exactly, zero-denominator fallback included — those two already carry a
    // note that they "must agree or the file says one thing and the tick
    // arithmetic another", and a stem that ran short would be a third answer.
    let den = if pattern.time_sig_den == 0 {
        4
    } else {
        pattern.time_sig_den
    };
    let ticks_per_bar = (PPQ * 4 / u32::from(den)).max(1) * u32::from(pattern.time_sig_num.max(1));
    let ticks = ticks_per_bar.saturating_mul(u32::from(pattern.bars));
    let frames = (f64::from(ticks) * seconds_per_tick * f64::from(RATE)) as usize
        + (TAIL_SECONDS * f64::from(RATE)) as usize;
    let frames = frames.min(MAX_SECONDS as usize * RATE as usize);
    if frames == 0 {
        return None;
    }

    // Every note in the pattern, in time order, with the frame it starts on.
    // Collected first so the render is one pass over a sorted list rather than
    // a search per frame.
    let mut hits: Vec<(usize, usize, f32, f32)> = Vec::new();
    for track in &pattern.lanes {
        let Some(pad_index) = kit.pad_for(track.lane) else {
            // ⛔ Skipped, never defaulted to a nearby pad — `pad_for` refuses to
            // guess for the reason its own doc gives, and this must not undo
            // that by picking one here.
            continue;
        };
        let pad = &kit.pads[pad_index];
        for note in &track.notes {
            let at = (f64::from(note.start_tick) * seconds_per_tick * f64::from(RATE)) as usize;
            if at >= frames {
                continue;
            }
            // Percussion transposes from the lane's own GM note and a pitched
            // pad from its root — the same rule `render_preview` follows, so a
            // stem sounds like the preview did (TASK-131D).
            let semis = Kit::semitones_for(pad, track.lane, note.pitch);
            hits.push((at, pad_index, f32::from(note.vel) / 127.0, semis));
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|(at, _, _, _)| *at);

    let mut out = vec![0.0f32; frames * 2];
    let mut sampler = Sampler::default();
    let mut next = 0usize;
    let mut at = 0usize;
    while at < frames {
        while next < hits.len() && hits[next].0 <= at {
            let (_, pad, velocity, semis) = hits[next];
            sampler.trigger(kit, pad, velocity, semis, f64::from(RATE));
            next += 1;
        }
        // Render up to the next trigger, so a hit lands on its own frame rather
        // than being quantised to a block — the same reason `render_preview`
        // splits its block.
        let until = hits
            .get(next)
            .map(|(frame, _, _, _)| (*frame).min(frames))
            .unwrap_or(frames)
            .max(at + 1);
        sampler.render(kit, &mut out[at * 2..until * 2], 2);
        at = until;
    }

    sampler::limit(&mut out);
    let peak = out.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    // A rendered part that is entirely silent is the failure this module exists
    // to avoid writing to disk.
    (peak > 1.0e-4).then_some(out)
}

/// Interleaved stereo f32 as a 16-bit PCM WAV.
///
/// ⚠ **A writer, where `kit::decode_wav` is the reader.** They are deliberately
/// separate: that one accepts a little more than this writes, because it has to
/// survive a file somebody replaced. This one emits exactly one format.
pub fn to_wav(samples: &[f32]) -> Vec<u8> {
    const CHANNELS: u16 = 2;
    const BITS: u16 = 16;

    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    let block = u32::from(CHANNELS) * u32::from(BITS) / 8;
    out.extend_from_slice(&(RATE * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    for sample in samples {
        // ⛔ Clamped before the cast. `as i16` on an out-of-range float
        // saturates in Rust, but the limiter has already bounded this to ±1 and
        // relying on the cast's behaviour instead of saying so is how a
        // rounding change becomes a click.
        let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale};

    fn pattern_with(lane: Lane, pitch: u8) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 1,
            bars: 1,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane,
                notes: vec![Note {
                    start_tick: 0,
                    len_ticks: 240,
                    pitch,
                    vel: 110,
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                }],
            }],
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    #[test]
    fn a_pattern_renders_to_audible_stereo() {
        // ⛔ **The claim TASK-069 could not make.** It shipped MIDI stems
        // because the melodic parts rendered pure silence; this is what changed.
        let kit = crate::audio::preview_kit().expect("the shipped kit must load");
        let rendered =
            to_stereo(&pattern_with(Lane::Kick, 36), kit).expect("a kick must render audio");

        let peak = rendered.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak > 0.01, "the stem is silent (peak {peak})");
        assert!(peak <= 1.0, "the limiter let something past full scale");
        // One bar at 120 BPM is two seconds, plus the tail.
        assert!(rendered.len() >= 2 * RATE as usize * 2, "too short");
    }

    #[test]
    fn every_generated_part_renders_rather_than_writing_a_silent_file() {
        // ⛔ **The reason this module could not exist before TASK-131A.** Four
        // parts out of five had no pad, so a stem export would have written
        // four silent files and called them stems.
        let kit = crate::audio::preview_kit().unwrap();
        for (lane, pitch) in [
            (Lane::Kick, 36),
            (Lane::Snare, 38),
            (Lane::Melody, 84),
            (Lane::Counter, 72),
            (Lane::Bass, 36),
            (Lane::Chords, 60),
            (Lane::Bass808, 28),
        ] {
            let rendered = to_stereo(&pattern_with(lane, pitch), kit)
                .unwrap_or_else(|| panic!("{lane:?} rendered nothing"));
            let peak = rendered.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.01, "{lane:?} rendered silence");
        }
    }

    #[test]
    fn a_lane_the_kit_cannot_play_is_none_rather_than_a_file_of_zeros() {
        // `Snap` has no shipped pad. Writing it out would be the exact failure
        // this module's header records.
        let kit = crate::audio::preview_kit().unwrap();
        assert!(to_stereo(&pattern_with(Lane::Snap, 39), kit).is_none());
    }

    #[test]
    fn the_wav_round_trips_through_our_own_reader() {
        // ⛔ The writer and the reader are separate on purpose, so this is what
        // stops them drifting apart. A header this reader cannot parse is a file
        // a DAW may not parse either.
        let samples = vec![0.5f32, -0.5, 0.25, -0.25];
        let bytes = to_wav(&samples);
        let decoded = crate::audio::kit::decode_wav(&bytes).expect("our own WAV must decode");

        assert_eq!(decoded.sample_rate, RATE);
        // Two stereo frames, downmixed to mono by the reader.
        assert_eq!(decoded.samples.len(), 2);
        assert!(decoded.samples.iter().all(|s| s.abs() < 0.01), "L+R cancel");
    }

    #[test]
    fn full_scale_survives_the_write_without_wrapping() {
        // A float past ±1 cast straight to i16 is the classic way a loud stem
        // comes back as a click.
        let bytes = to_wav(&[1.0, -1.0, 2.0, -2.0]);
        let decoded = crate::audio::kit::decode_wav(&bytes).unwrap();
        assert!(
            decoded.samples.iter().all(|s| s.abs() <= 1.0),
            "a sample wrapped: {:?}",
            decoded.samples
        );
    }
}
