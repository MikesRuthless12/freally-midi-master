//! One-shot sampler voices and the master limiter.
//!
//! Everything here runs on the audio callback, which means the rules are
//! absolute: **no allocation, no locking, no I/O, no panicking path**. Voices
//! live in a fixed array and reference kit pads by index, so triggering a note
//! moves no memory and touches no reference count.
//!
//! `engine/tests` cannot reach this and neither can a device — so the whole
//! thing is a pure function of its inputs, and the tests drive it by hand.

use super::kit::Kit;

/// Enough for the densest thing the engine generates — a 64th-note hat roll
/// under a fill — with room to spare. Voices are ~48 bytes, so the array is
/// cheap; running out is what sounds broken.
pub const MAX_VOICES: usize = 48;

/// The knee above which the limiter starts bending. Below it the mix is
/// untouched, so ordinary playback is not coloured at all.
const LIMIT_KNEE: f32 = 0.6;

#[derive(Clone, Copy)]
struct Voice {
    /// Index into `Kit::pads`, and `usize::MAX` when the voice is free.
    pad: usize,
    /// Read position in the pad's samples. Fractional: the step is rarely 1.0.
    pos: f64,
    /// Samples advanced per output frame — device rate against pad rate, times
    /// the pitch ratio.
    step: f64,
    gain_l: f32,
    gain_r: f32,
    choke_group: Option<u8>,
    /// When this voice started, for stealing the oldest.
    started: u64,
}

impl Voice {
    const FREE: usize = usize::MAX;

    fn free() -> Self {
        Voice {
            pad: Self::FREE,
            pos: 0.0,
            step: 1.0,
            gain_l: 0.0,
            gain_r: 0.0,
            choke_group: None,
            started: 0,
        }
    }

    fn active(&self) -> bool {
        self.pad != Self::FREE
    }
}

pub struct Sampler {
    voices: [Voice; MAX_VOICES],
    /// Monotonic counter deciding which voice is oldest.
    clock: u64,
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler {
            voices: [Voice::free(); MAX_VOICES],
            clock: 0,
        }
    }
}

impl Sampler {
    /// Start a pad. `velocity` is 0–1; `semis` is added to the pad's own offset.
    pub fn trigger(&mut self, kit: &Kit, pad_index: usize, velocity: f32, semis: f32, rate: f64) {
        let Some(pad) = kit.pads.get(pad_index) else {
            return;
        };

        // A choke group is a physical claim, not a mix decision: the same hat
        // cannot be open and closed at once, so the new hit silences the old.
        if let Some(group) = pad.choke_group {
            for voice in &mut self.voices {
                if voice.active() && voice.choke_group == Some(group) {
                    *voice = Voice::free();
                }
            }
        }

        let slot = self.claim();
        self.clock += 1;

        // Constant power, so a centred pad is not louder than a panned one.
        let angle = (pad.pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::FRAC_PI_4);
        let gain = pad.gain * velocity.clamp(0.0, 1.0);

        self.voices[slot] = Voice {
            pad: pad_index,
            pos: 0.0,
            step: f64::from(pad.sample_rate) / rate
                * 2f64.powf(f64::from(pad.pitch_semis as f32 + semis) / 12.0),
            gain_l: gain * angle.cos(),
            gain_r: gain * angle.sin(),
            choke_group: pad.choke_group,
            started: self.clock,
        };
    }

    /// A free slot, or the oldest sounding one.
    ///
    /// Stealing the oldest is the right trade for one-shots: the newest hit is
    /// the one the user is listening for, and the oldest is furthest into its
    /// decay, so it is the least audible thing to lose.
    fn claim(&mut self) -> usize {
        let mut oldest = 0usize;
        for (i, voice) in self.voices.iter().enumerate() {
            if !voice.active() {
                return i;
            }
            if voice.started < self.voices[oldest].started {
                oldest = i;
            }
        }
        oldest
    }

    pub fn stop_all(&mut self) {
        self.voices = [Voice::free(); MAX_VOICES];
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// Mix every sounding voice into `out`, which is interleaved by `channels`.
    ///
    /// Adds rather than overwrites: the caller renders a block in segments, so
    /// a segment boundary must not erase what came before it.
    pub fn render(&mut self, kit: &Kit, out: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }

        for voice in &mut self.voices {
            if !voice.active() {
                continue;
            }
            let Some(pad) = kit.pads.get(voice.pad) else {
                *voice = Voice::free();
                continue;
            };
            let samples: &[f32] = &pad.samples;

            for frame in out.chunks_mut(channels) {
                let index = voice.pos as usize;
                if index + 1 >= samples.len() {
                    *voice = Voice::free();
                    break;
                }

                // Linear interpolation. The step is almost never 1.0 — the kit
                // is 44.1 kHz and the device usually is not — so reading the
                // nearest sample instead would alias audibly on the hats.
                let frac = (voice.pos - index as f64) as f32;
                let value = samples[index] + (samples[index + 1] - samples[index]) * frac;

                frame[0] += value * voice.gain_l;
                if channels > 1 {
                    frame[1] += value * voice.gain_r;
                }
                // Anything past stereo stays silent rather than being fed a
                // copy: a surround device would otherwise put the kit in the
                // centre and the rears at once.

                voice.pos += voice.step;
            }
        }
    }
}

/// Bend a signal into ±1 without a corner.
///
/// Below the knee this is the identity, so nothing that was already in range is
/// coloured. Above it the curve is continuous in value *and* slope, which is
/// what stops a limiter from sounding like distortion, and it approaches full
/// scale asymptotically — so stacking previews cannot crack the output. Far
/// enough out it lands on exactly 1.0, where `f32` no longer has the resolution
/// to hold it apart; that is the ceiling, not a breach of it.
pub fn soft_clip(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= LIMIT_KNEE {
        return x;
    }
    let headroom = 1.0 - LIMIT_KNEE;
    let shaped = LIMIT_KNEE + headroom * ((magnitude - LIMIT_KNEE) / headroom).tanh();
    shaped.copysign(x)
}

/// Apply the limiter across a rendered block.
pub fn limit(out: &mut [f32]) {
    for sample in out {
        *sample = soft_clip(*sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::kit::Pad;
    use engine::pattern::Lane;
    use std::sync::Arc;

    /// A kit of test tones: every pad is a run of 1.0s, so a rendered sample
    /// says exactly which pad and gain produced it.
    fn test_kit() -> Kit {
        let pad = |id: &str, lane: Lane, choke: Option<u8>, pan: f32| Pad {
            id: id.into(),
            lane,
            samples: Arc::from(vec![1.0f32; 64].into_boxed_slice()),
            sample_rate: 48_000,
            gain: 1.0,
            pan,
            pitch_semis: 0,
            choke_group: choke,
            root_note: None,
        };
        Kit {
            id: "test".into(),
            pads: vec![
                pad("kick", Lane::Kick, None, 0.0),
                pad("closed", Lane::ClosedHat, Some(1), 0.0),
                pad("open", Lane::OpenHat, Some(1), 0.0),
                pad("left", Lane::Perc, None, -1.0),
            ],
        }
    }

    fn render_block(sampler: &mut Sampler, kit: &Kit, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        sampler.render(kit, &mut out, 2);
        out
    }

    #[test]
    fn a_triggered_pad_is_heard_immediately() {
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        let out = render_block(&mut sampler, &kit, 4);
        assert!(out[0] > 0.5, "the first frame must already carry the hit");
    }

    #[test]
    fn a_hat_choke_silences_the_hat_under_it() {
        // The open hat and the closed hat are the same piece of metal.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 1, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 1);

        sampler.trigger(&kit, 2, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 1, "the closed hat should be gone");

        // ...and a kick, which is in no group, survives both.
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 2);
    }

    #[test]
    fn velocity_scales_the_output_and_pan_moves_it() {
        let kit = test_kit();
        let mut sampler = Sampler::default();

        sampler.trigger(&kit, 0, 0.5, 0.0, 48_000.0);
        let half = render_block(&mut sampler, &kit, 1);
        sampler.stop_all();

        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        let full = render_block(&mut sampler, &kit, 1);
        assert!(half[0] < full[0], "velocity must change the level");

        // Hard left: the right channel stays silent.
        sampler.stop_all();
        sampler.trigger(&kit, 3, 1.0, 0.0, 48_000.0);
        let left = render_block(&mut sampler, &kit, 1);
        assert!(left[0] > 0.5, "left should carry it");
        assert!(left[1].abs() < 0.001, "right should not, got {}", left[1]);
    }

    #[test]
    fn a_pad_plays_at_its_own_rate_whatever_the_device_runs_at() {
        // The kit is 44.1 kHz and a device is usually 48. Ignoring that plays
        // every drum ~9% sharp, which is subtle enough to ship by accident.
        let kit = test_kit();
        let mut sampler = Sampler::default();

        sampler.trigger(&kit, 0, 1.0, 0.0, 96_000.0);
        // Pad at 48 kHz played at 96 kHz: half a sample per frame, so 64
        // samples last ~128 frames rather than 64.
        let out = render_block(&mut sampler, &kit, 100);
        assert!(
            out.chunks(2).all(|f| f[0] > 0.5),
            "a half-speed voice must still be sounding after 100 frames"
        );
    }

    #[test]
    fn a_voice_ends_at_the_end_of_its_sample_and_frees_its_slot() {
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        let _ = render_block(&mut sampler, &kit, 128);
        assert_eq!(
            sampler.active_voices(),
            0,
            "a finished one-shot must free up"
        );
    }

    #[test]
    fn running_out_of_voices_steals_the_oldest_rather_than_dropping_the_newest() {
        // The newest hit is the one being listened for.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        for _ in 0..MAX_VOICES + 8 {
            sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        }
        assert_eq!(sampler.active_voices(), MAX_VOICES);

        let out = render_block(&mut sampler, &kit, 1);
        assert!(out[0] > 0.0, "the pool must still be producing audio");
    }

    #[test]
    fn the_limiter_leaves_an_ordinary_mix_alone() {
        for x in [0.0, 0.1, -0.25, 0.6, -0.6] {
            assert_eq!(soft_clip(x), x, "{x} is inside the knee");
        }
    }

    #[test]
    fn the_limiter_bounds_anything_without_a_corner() {
        // Bounded everywhere: full scale is the ceiling and nothing may pass
        // it, however much is stacked up. The curve approaches 1.0 and reaches
        // it only where f32 runs out of room to tell the difference.
        let mut x = -64.0f32;
        while x <= 64.0 {
            assert!(soft_clip(x).abs() <= 1.0, "{x} left the range");
            x += 0.05;
        }

        // Strictly monotonic across the range a mix actually occupies. A hard
        // clamp is bounded too — this is what it fails, because everything past
        // the threshold collapses onto one value and the differences between
        // loud things stop existing.
        let mut previous = f32::NEG_INFINITY;
        let mut x = -2.0f32;
        while x <= 2.0 {
            let y = soft_clip(x);
            assert!(y > previous, "not monotonic at {x}: {y} after {previous}");
            previous = y;
            x += 0.01;
        }

        // The slope either side of the knee matches: below it the gain is 1,
        // and immediately above it must still be ~1, not a step.
        let below = (soft_clip(LIMIT_KNEE) - soft_clip(LIMIT_KNEE - 0.001)) / 0.001;
        let above = (soft_clip(LIMIT_KNEE + 0.001) - soft_clip(LIMIT_KNEE)) / 0.001;
        assert!(
            (below - above).abs() < 0.01,
            "kink at the knee: {below} vs {above}"
        );
    }

    #[test]
    fn a_stack_of_voices_cannot_crack_the_output() {
        // Every voice in the pool at full velocity on the same pad — far more
        // than playback produces, and the point is that it still cannot clip.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        for _ in 0..MAX_VOICES {
            sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        }

        let mut out = render_block(&mut sampler, &kit, 8);
        assert!(
            out.iter().any(|s| s.abs() > 1.0),
            "the raw mix should be over full scale, or this proves nothing"
        );
        limit(&mut out);
        assert!(
            out.iter().all(|s| s.abs() <= 1.0),
            "the limiter let something past full scale"
        );
    }
}
