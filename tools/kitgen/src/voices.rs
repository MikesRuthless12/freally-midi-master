//! The synthesis itself.
//!
//! Every voice is built from oscillators, filtered noise and envelopes — no
//! recorded material is involved at any point, which is what makes the shipped
//! kits CC0 by construction (PRD § 15 Q5). Nothing here is meant to replace a
//! producer's own one-shots; it exists so a fresh install can be auditioned
//! before anyone imports anything.

use engine::rng::stream;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::wav::SAMPLE_RATE;

const SR: f32 = SAMPLE_RATE as f32;

fn seconds(n: f32) -> usize {
    (SR * n) as usize
}

/// Exponential decay to `-60 dB` over `secs`.
fn decay(t: f32, secs: f32) -> f32 {
    (-6.907 * t / secs).exp()
}

/// A short fade at the very start, so a sample never begins on a discontinuity.
fn declick(samples: &mut [f32]) {
    let n = (SR * 0.002) as usize; // 2 ms
    for (i, s) in samples.iter_mut().take(n).enumerate() {
        *s *= i as f32 / n as f32;
    }
    // And at the end, where a truncated tail would otherwise click.
    let len = samples.len();
    for i in 0..n.min(len) {
        samples[len - 1 - i] *= i as f32 / n as f32;
    }
}

/// Soft saturation. `drive` of 1.0 is clean; higher values fatten and clip.
fn saturate(x: f32, drive: f32) -> f32 {
    (x * drive).tanh() / drive.tanh()
}

/// Normalize to a target peak so the pads feel level against each other.
fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 1e-9 {
        let gain = peak / max;
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }
}

/// One-pole low-pass. `cutoff` in Hz.
fn low_pass(samples: &mut [f32], cutoff: f32) {
    let dt = 1.0 / SR;
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
    let alpha = dt / (rc + dt);
    let mut prev = 0.0;
    for s in samples.iter_mut() {
        prev += alpha * (*s - prev);
        *s = prev;
    }
}

/// One-pole high-pass.
fn high_pass(samples: &mut [f32], cutoff: f32) {
    let dt = 1.0 / SR;
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
    let alpha = rc / (rc + dt);
    let mut prev_in = 0.0;
    let mut prev_out = 0.0;
    for s in samples.iter_mut() {
        let out = alpha * (prev_out + *s - prev_in);
        prev_in = *s;
        prev_out = out;
        *s = out;
    }
}

fn noise(rng: &mut ChaCha8Rng, n: usize) -> Vec<f32> {
    (0..n).map(|_| rng.random_range(-1.0f32..1.0f32)).collect()
}

/// The 808: a sine with a fast downward pitch envelope into a long body, then
/// driven. The pitch drop is what reads as the "click" of the attack — it is
/// the same oscillator, not a layered transient.
pub fn eight_o_eight(root_hz: f32, length_s: f32, drive: f32) -> Vec<f32> {
    let n = seconds(length_s);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f32 / SR;
        // Start ~2 octaves up and fall to the root within ~30 ms.
        let pitch_env = 1.0 + 3.0 * (-t / 0.03).exp();
        let freq = root_hz * pitch_env;
        phase += std::f32::consts::TAU * freq / SR;

        let amp = decay(t, length_s * 0.9);
        out.push(saturate(phase.sin() * amp, drive));
    }

    declick(&mut out);
    normalize(&mut out, 0.89);
    out
}

/// Closed hat: band-passed noise with a very fast decay.
pub fn closed_hat(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/closed_hat");
    let n = seconds(0.06);
    let mut out = noise(&mut rng, n);

    high_pass(&mut out, 7_000.0);
    low_pass(&mut out, 16_000.0);
    for (i, s) in out.iter_mut().enumerate() {
        *s *= decay(i as f32 / SR, 0.035);
    }

    declick(&mut out);
    normalize(&mut out, 0.62);
    out
}

/// Open hat: the same voice with a long tail, so the pair sits together.
pub fn open_hat(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/open_hat");
    let n = seconds(0.42);
    let mut out = noise(&mut rng, n);

    high_pass(&mut out, 6_500.0);
    low_pass(&mut out, 15_000.0);
    for (i, s) in out.iter_mut().enumerate() {
        *s *= decay(i as f32 / SR, 0.30);
    }

    declick(&mut out);
    normalize(&mut out, 0.60);
    out
}

/// Clap: three noise bursts a few milliseconds apart, then a short room tail.
/// The spread is what makes it a clap rather than a snare — a single burst
/// reads as noise, several read as hands.
pub fn clap(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/clap");
    let n = seconds(0.30);
    let mut out = vec![0.0f32; n];
    let source = noise(&mut rng, n);

    for (burst, offset_ms) in [(0, 0.0f32), (1, 9.0), (2, 18.0)] {
        let offset = seconds(offset_ms / 1000.0);
        let level = 1.0 - burst as f32 * 0.18;
        for i in 0..seconds(0.012) {
            if offset + i < n {
                out[offset + i] += source[i] * decay(i as f32 / SR, 0.008) * level;
            }
        }
    }

    // The tail that turns three claps into one gesture.
    let tail_start = seconds(0.026);
    for i in tail_start..n {
        out[i] += source[i] * decay((i - tail_start) as f32 / SR, 0.10) * 0.35;
    }

    high_pass(&mut out, 1_100.0);
    low_pass(&mut out, 9_000.0);
    declick(&mut out);
    normalize(&mut out, 0.80);
    out
}

/// Snare: noise for the wires, plus two detuned sine bodies for the shell.
pub fn snare(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/snare");
    let n = seconds(0.22);
    let mut wires = noise(&mut rng, n);

    high_pass(&mut wires, 900.0);
    low_pass(&mut wires, 11_000.0);

    let mut out = Vec::with_capacity(n);
    for (i, wire) in wires.iter().enumerate() {
        let t = i as f32 / SR;
        let body = (std::f32::consts::TAU * 185.0 * t).sin() * decay(t, 0.09) * 0.5
            + (std::f32::consts::TAU * 331.0 * t).sin() * decay(t, 0.05) * 0.3;
        out.push(body + wire * decay(t, 0.13));
    }

    declick(&mut out);
    normalize(&mut out, 0.86);
    out
}

/// Rim: a very short pitched click. Nearly all attack.
pub fn rim(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/rim");
    let n = seconds(0.05);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, noise) in source.iter().enumerate() {
        let t = i as f32 / SR;
        let tone = (std::f32::consts::TAU * 1_720.0 * t).sin() * decay(t, 0.012);
        out.push(tone * 0.7 + noise * decay(t, 0.006) * 0.5);
    }

    high_pass(&mut out, 400.0);
    declick(&mut out);
    normalize(&mut out, 0.66);
    out
}

/// Perc: a wooden tone, useful for offbeat placements.
pub fn perc(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/perc");
    let n = seconds(0.12);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, noise) in source.iter().enumerate() {
        let t = i as f32 / SR;
        let tone = (std::f32::consts::TAU * 840.0 * t).sin() * decay(t, 0.045)
            + (std::f32::consts::TAU * 1_260.0 * t).sin() * decay(t, 0.025) * 0.4;
        out.push(tone * 0.8 + noise * decay(t, 0.004) * 0.3);
    }

    high_pass(&mut out, 300.0);
    low_pass(&mut out, 7_000.0);
    declick(&mut out);
    normalize(&mut out, 0.70);
    out
}

/// Kick: a short, tight sine drop, separate from the 808's long body.
pub fn kick() -> Vec<f32> {
    let n = seconds(0.35);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f32 / SR;
        let freq = 52.0 * (1.0 + 5.0 * (-t / 0.018).exp());
        phase += std::f32::consts::TAU * freq / SR;
        out.push(saturate(phase.sin() * decay(t, 0.22), 1.8));
    }

    declick(&mut out);
    normalize(&mut out, 0.92);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, x| m.max(x.abs()))
    }

    #[test]
    fn every_voice_produces_audio_within_full_scale() {
        let voices: Vec<(&str, Vec<f32>)> = vec![
            ("808", eight_o_eight(41.2, 1.4, 2.2)),
            ("kick", kick()),
            ("closed_hat", closed_hat(1)),
            ("open_hat", open_hat(1)),
            ("clap", clap(1)),
            ("snare", snare(1)),
            ("rim", rim(1)),
            ("perc", perc(1)),
        ];
        for (name, v) in voices {
            assert!(!v.is_empty(), "{name} produced no samples");
            let p = peak(&v);
            assert!(p > 0.1, "{name} is essentially silent (peak {p})");
            assert!(p <= 1.0, "{name} exceeds full scale (peak {p})");
        }
    }

    #[test]
    fn synthesis_is_deterministic() {
        // Same seed, same bytes — the kits are committed, so a rebuild that
        // produced different audio would show up as a spurious diff forever.
        assert_eq!(clap(7), clap(7));
        assert_eq!(snare(7), snare(7));
        assert_eq!(kick(), kick());
    }

    #[test]
    fn different_seeds_give_different_noise() {
        assert_ne!(clap(1), clap(2));
    }

    #[test]
    fn voices_start_and_end_at_silence() {
        // A sample that begins or ends mid-waveform clicks on every trigger.
        for v in [closed_hat(3), snare(3), clap(3), kick()] {
            assert!(v[0].abs() < 1e-4, "starts at {}", v[0]);
            assert!(v[v.len() - 1].abs() < 1e-4, "ends at {}", v[v.len() - 1]);
        }
    }

    #[test]
    fn the_open_hat_rings_longer_than_the_closed_hat() {
        assert!(
            open_hat(1).len() > closed_hat(1).len() * 3,
            "the pair must be usable as an open/closed pair"
        );
    }

    #[test]
    fn the_808_holds_while_the_kick_is_short() {
        // The 808 carries the bassline; the kick is a transient under it.
        assert!(eight_o_eight(41.2, 1.4, 2.2).len() > kick().len() * 3);
    }
}
