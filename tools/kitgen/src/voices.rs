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

// ─────────────────────────────────────────────────────── the pitched voices
//
// ⛔ **Why these exist at all** (TASK-131). `Kit::pad_for` answers with the pad
// whose lane matches, and until now the kit carried only percussion — so
// melody, countermelody, bassline and chords returned `None` and played
// *nothing*. That one fact is why TASK-069 shipped MIDI stems instead of wav,
// why the arrangement cannot draw a waveform yet, and why a producer pressing
// Play on a melody heard silence and reasonably concluded the generator was
// broken.
//
// ⚠ **These are previews, not instruments, and the distinction is deliberate.**
// Each is one sample transposed by the sampler, so playing two octaves off its
// root thins it out — which is exactly why the roots below sit in the middle of
// each part's authored register rather than at a tidy C. A producer who wants
// their own sound imports a one-shot; this is what makes a fresh install
// audible before they do.

/// A one-pole low-pass whose cutoff moves while the sample plays.
///
/// ⛔ **The single most important difference between a synthesized preview that
/// sounds like a toy and one that sounds like an instrument.** A static filter
/// pass gives every moment of the note the same brightness; a real plucked or
/// struck sound is bright at the transient and darkens as it decays, because the
/// energy in the high partials dies first. The first cut of these voices used
/// one static pass and that is precisely why they read as cheap.
fn sweep_low_pass(samples: &mut [f32], from_hz: f32, to_hz: f32, over_s: f32) {
    let dt = 1.0 / SR;
    let mut prev = 0.0;
    for (i, s) in samples.iter_mut().enumerate() {
        let progress = ((i as f32 / SR) / over_s).min(1.0);
        let cutoff = (from_hz + (to_hz - from_hz) * progress).max(30.0);
        let rc = 1.0 / (std::f32::consts::TAU * cutoff);
        let alpha = dt / (rc + dt);
        prev += alpha * (*s - prev);
        *s = prev;
    }
}

/// The lead: a Karplus–Strong plucked string.
///
/// ⛔ **A physical model rather than stacked oscillators, and the difference is
/// audible immediately.** Exciting a delay line one wavelength long with noise
/// and feeding it back through a damping filter *is* how a plucked string
/// behaves — the pick noise is real broadband energy, the partials decay at
/// their own rates because the damping filter takes the highs first, and the
/// tone evolves instead of holding still. Additive saws cannot do any of that;
/// they hold one spectrum and fade.
///
/// Cheap, too: one buffer the length of a single cycle, and two adds per sample.
pub fn pluck(root_hz: f32, length_s: f32, seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/pluck");
    let n = seconds(length_s);
    let delay = ((SR / root_hz).round() as usize).clamp(2, n.max(2));

    // The excitation. Low-passed noise rather than raw: a pick is not a click,
    // and unfiltered noise makes the attack read as a burst of static.
    let mut line = noise(&mut rng, delay);
    low_pass(&mut line, 6000.0);

    let mut out = Vec::with_capacity(n);
    let mut index = 0usize;
    for i in 0..n {
        let current = line[index];
        let next = line[(index + 1) % delay];
        // The averaging filter is the string's damping — it is what makes the
        // high partials die before the fundamental. The coefficient sets how
        // long the note rings.
        let damped = 0.5 * (current + next) * 0.9965;
        line[index] = damped;
        // A slow overall decay on top, so the note ends rather than ringing to
        // the end of the buffer.
        out.push(current * decay(i as f32 / SR, length_s * 0.85));
        index = (index + 1) % delay;
    }

    high_pass(&mut out, 80.0);
    declick(&mut out);
    normalize(&mut out, 0.74);
    out
}

/// The countermelody: an FM bell.
///
/// ⛔ **Two-operator FM with a decaying modulation index, not a stack of
/// partials.** Additive partials at fixed levels give a bell that is equally
/// clangorous from start to finish; a real strike is bright and inharmonic for
/// the first moment and settles toward the fundamental as it rings. Decaying the
/// index is what produces that, and it is the reason FM owned this sound for a
/// decade.
///
/// The 3.5 ratio is deliberately inharmonic — an integer ratio would give a
/// harmonic tone, which is a flute rather than a bell.
pub fn bell(root_hz: f32, length_s: f32) -> Vec<f32> {
    let n = seconds(length_s);
    let mut out = Vec::with_capacity(n);

    for i in 0..n {
        let t = i as f32 / SR;
        // Bright at the strike, settling as it rings.
        let index = 7.0 * decay(t, length_s * 0.25);
        let modulator = (std::f32::consts::TAU * root_hz * 3.5 * t).sin() * index;
        let carrier = (std::f32::consts::TAU * root_hz * t + modulator).sin();
        // A second, quieter strike partial a fifth up gives the shimmer a single
        // operator pair cannot.
        let shimmer =
            (std::f32::consts::TAU * root_hz * 2.99 * t).sin() * 0.18 * decay(t, length_s * 0.5);
        out.push((carrier * decay(t, length_s * 0.85) + shimmer) * 0.55);
    }

    high_pass(&mut out, 130.0);
    declick(&mut out);
    normalize(&mut out, 0.68);
    out
}

/// The bassline: a filtered saw over a sine sub.
///
/// ⛔ **Distinct from [`eight_o_eight`] rather than a copy of it** — the 808 is a
/// sine with a pitch-drop transient and heavy drive, and a bass part that sounded
/// identical would make the two lanes indistinguishable in the styles that
/// author both.
///
/// The filter envelope is what makes it read as a played bass rather than a
/// sustained tone: bright on the attack, closing to a round body within a couple
/// of hundred milliseconds.
pub fn synth_bass(root_hz: f32, length_s: f32) -> Vec<f32> {
    let n = seconds(length_s);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f32 / SR;
        phase += std::f32::consts::TAU * root_hz / SR;
        // A bandlimited saw — enough harmonics for the filter to have something
        // to work on, few enough that transposing up does not alias.
        let saw = phase.sin()
            + 0.5 * (2.0 * phase).sin()
            + 0.33 * (3.0 * phase).sin()
            + 0.25 * (4.0 * phase).sin();
        // The sub is a clean sine an octave down, which is what survives on a
        // laptop speaker and what holds the low end on a system.
        let sub = (phase * 0.5).sin() * 0.9;
        let amp = decay(t, length_s * 0.8);
        out.push((saw * 0.35 + sub) * amp * 0.6);
    }

    // Bright attack closing into the body — the played-bass shape.
    sweep_low_pass(&mut out, 3200.0, 420.0, 0.22);
    // Gentle drive for weight, well short of the 808's clipping.
    for s in out.iter_mut() {
        *s = saturate(*s, 1.5);
    }
    declick(&mut out);
    normalize(&mut out, 0.88);
    out
}

/// The chord voice: an FM electric piano.
///
/// ⛔ **The Rhodes sound is FM, and additive sine stacks do not get close.** A
/// tine piano is a struck bar: a hard metallic transient that dies in
/// milliseconds over a soft sustained body. That is a modulation index which
/// decays fast — the "bark" — sitting on a carrier that decays slowly. The first
/// cut of this was a fundamental plus a fifth and an octave, which is an organ.
///
/// A 14:1 tine ratio over a 1:1 body is the classic pairing.
pub fn keys(root_hz: f32, length_s: f32) -> Vec<f32> {
    let n = seconds(length_s);
    let mut out = Vec::with_capacity(n);

    for i in 0..n {
        let t = i as f32 / SR;
        // The tine strike: high ratio, very fast index decay.
        let tine_index = 2.6 * decay(t, 0.09);
        let tine = (std::f32::consts::TAU * root_hz * 14.0 * t).sin() * tine_index;
        // The body: 1:1 FM, which thickens without adding inharmonicity.
        let body_index = 1.1 * decay(t, length_s * 0.4);
        let body = (std::f32::consts::TAU * root_hz * t).sin() * body_index;

        let carrier = (std::f32::consts::TAU * root_hz * t + tine + body).sin();
        // A short attack rather than an instant one — a hammer has travel.
        let attack = (1.0 - (-t / 0.006).exp()).min(1.0);
        out.push(carrier * decay(t, length_s * 0.85) * attack * 0.6);
    }

    sweep_low_pass(&mut out, 6500.0, 2200.0, 0.5);
    high_pass(&mut out, 70.0);
    declick(&mut out);
    normalize(&mut out, 0.72);
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
                // Read at the ABSOLUTE position, so each burst is a different
                // slice of noise. Reading `source[i]` gave all three bursts the
                // identical 12 ms slice, which is a signal summed with two
                // delayed copies of itself — a comb filter, notching at
                // multiples of ~111 Hz and ~55 Hz. It rang metallic and flanged
                // instead of sounding like three pairs of hands, and no test
                // could see it: peak level, determinism and seed-difference all
                // hold just as well for the correlated version.
                out[offset + i] += source[offset + i] * decay(i as f32 / SR, 0.008) * level;
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

    /// Pearson correlation between two equal-length windows.
    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (a, b) = (&a[..n], &b[..n]);
        let mean = |s: &[f32]| s.iter().sum::<f32>() / n as f32;
        let (ma, mb) = (mean(a), mean(b));
        let mut num = 0.0;
        let (mut da, mut db) = (0.0f32, 0.0f32);
        for i in 0..n {
            let (x, y) = (a[i] - ma, b[i] - mb);
            num += x * y;
            da += x * x;
            db += y * y;
        }
        if da == 0.0 || db == 0.0 {
            return 0.0;
        }
        num / (da.sqrt() * db.sqrt())
    }

    #[test]
    fn the_claps_three_bursts_are_not_the_same_noise() {
        // Each burst must be an independent slice of noise. Indexing the source
        // by the intra-burst offset made all three byte-identical, which is one
        // signal summed with two delayed copies of itself — a comb filter, and
        // audibly a metallic flange rather than three pairs of hands.
        //
        // Nothing else here could catch it: peak level, determinism and
        // seed-difference are all satisfied by the correlated version, and this
        // clap ships as the preview kit a new user hears first.
        //
        // The threshold is calibrated against both versions rather than guessed.
        // Correlated: 0.40 (burst 2) and 0.33 (burst 3). Independent: 0.09 and
        // -0.02 — not 0, because burst 1's tail runs under the later windows and
        // the band-pass correlates neighbouring samples. 0.2 sits in the gap.
        let out = clap(1);
        let burst = seconds(0.012);
        for (label, offset) in [("second", seconds(0.009)), ("third", seconds(0.018))] {
            let c = correlation(&out[0..burst], &out[offset..offset + burst]);
            assert!(
                c.abs() < 0.2,
                "the {label} burst correlates with the first at {c} —                  they are the same noise delayed, which combs rather than claps"
            );
        }
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
