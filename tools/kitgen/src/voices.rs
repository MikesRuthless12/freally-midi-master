//! The synthesis itself.
//!
//! Every voice is built from oscillators, filtered noise and envelopes — no
//! recorded material is involved at any point, which is what makes the shipped
//! kits CC0 by construction (PRD § 15 Q5). Nothing here is meant to replace a
//! producer's own one-shots; it exists so a fresh install can be auditioned
//! before anyone imports anything.

pub(crate) use engine::rng::stream;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::wav::SAMPLE_RATE;

pub(crate) const SR: f32 = SAMPLE_RATE as f32;

pub(crate) fn seconds(n: f32) -> usize {
    (SR * n) as usize
}

/// Exponential decay to `-60 dB` over `secs`.
pub(crate) fn decay(t: f32, secs: f32) -> f32 {
    (-6.907 * t / secs).exp()
}

/// A short fade at the very start, so a sample never begins on a discontinuity.
pub(crate) fn declick(samples: &mut [f32]) {
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
pub(crate) fn saturate(x: f32, drive: f32) -> f32 {
    (x * drive).tanh() / drive.tanh()
}

/// Normalize to a target peak so the pads feel level against each other.
pub(crate) fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 1e-9 {
        let gain = peak / max;
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }
}

/// One-pole low-pass. `cutoff` in Hz.
pub(crate) fn low_pass(samples: &mut [f32], cutoff: f32) {
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
pub(crate) fn high_pass(samples: &mut [f32], cutoff: f32) {
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

pub(crate) fn noise(rng: &mut ChaCha8Rng, n: usize) -> Vec<f32> {
    (0..n).map(|_| rng.random_range(-1.0f32..1.0f32)).collect()
}

/// A **resonant** two-pole band-pass (state-variable form).
///
/// ⛔ **The one-pole pair above cannot do this, and metal is why it matters.**
/// `high_pass` then `low_pass` carves a broad shelf with no peak, which turns a
/// square-oscillator bank into a buzzer. A cymbal is a set of *ringing* modes,
/// so the filter has to resonate — that is what leaves the partials sounding
/// struck rather than merely present.
///
/// ⚠ **`q` at or below 1.0 has no resonance at all** — damping is `1/q`, and it
/// is clamped at 1.0 to keep the filter stable. Only `q` above 1 peaks, and
/// above ~2 it starts to sing. A call site passing 0.8 or 0.9 is asking for a
/// plain band shape, which is a legitimate thing to want and worth knowing is
/// what it gets.
pub(crate) fn band_pass(samples: &mut [f32], centre_hz: f32, q: f32) {
    // ⛔ **Capped at SR/6, because this is a Chamberlin state-variable filter
    // and its `f = 2 sin(pi fc / SR)` mapping is only accurate below roughly
    // that.** Above it the poles drift and then go real: the shaker asked for
    // 6.8 kHz and peaked at 16 kHz, and the tambourine's 9 kHz stopped
    // resonating altogether. Clamping keeps every centre in the range where the
    // number in the source is the frequency you get.
    let f = 2.0 * (std::f32::consts::PI * centre_hz.clamp(20.0, SR / 6.0) / SR).sin();
    let damp = (1.0 / q.max(0.5)).min(1.0);
    let (mut low, mut band) = (0.0f32, 0.0f32);
    for s in samples.iter_mut() {
        let high = *s - low - damp * band;
        band += f * high;
        low += f * band;
        *s = band;
    }
}

/// A square oscillator, which is what the classic drum machines actually used.
fn square(phase: f32) -> f32 {
    if phase.sin() >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// A rising edge over `secs`, so a voice does not begin on a click.
///
/// ⚠ **A shaker and a tambourine live or die on this.** Both are many small
/// objects picked up by the hand rather than one struck surface, so the energy
/// *swells* over ~10 ms. Given the instant attack every other voice here wants,
/// they read as a hi-hat instead — same spectrum, wrong instrument.
pub(crate) fn attack(t: f32, secs: f32) -> f32 {
    (t / secs.max(1e-6)).min(1.0)
}

/// The TR-808's six square oscillators, in Hz.
///
/// ⛔ **The real ratios, and they are deliberately inharmonic.** The 808 built
/// every metal voice — cymbal, cowbell, open and closed hat — from this one bank
/// and separated them by filtering and envelope alone. Non-integer ratios are
/// precisely why it reads as struck metal instead of as a chord: no partial is a
/// harmonic of another, so the ear hears a plate rather than a pitch.
const METAL_PARTIALS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

/// The metal bank at time `t`, optionally transposed.
pub(crate) fn metal(t: f32, transpose: f32) -> f32 {
    METAL_PARTIALS
        .iter()
        .map(|hz| square(std::f32::consts::TAU * hz * transpose * t))
        .sum::<f32>()
        / METAL_PARTIALS.len() as f32
}

/// Give a finished percussion voice a kit family's character.
///
/// ⛔ **This is what makes a drill kit sound like a drill kit rather than trap
/// at a different tempo.** The voices above are one set of recipes; a family is
/// that set heard through a different room and a different converter. Three
/// controls carry almost all of the audible difference between real kit
/// families:
///
/// - `brightness_hz` — where the top ends. A boom-bap kit sampled at 26 kHz on
///   an SP-1200 simply has no air above ~8 kHz, and that missing top *is* the
///   sound. A club kit keeps all of it.
/// - `drive` — how hard it was pushed. Phonk and rage are distorted on purpose;
///   country and R&B are not.
/// - `body_hz` — a gentle low shelf via a resonant lift, so a family can sit
///   fat or lean without retuning every voice.
///
/// ⚠ **Applied after `normalize`, then normalized again**, because saturation
/// changes the peak and a family that came out quieter than its neighbours
/// would read as a worse kit rather than a different one.
pub fn shape(samples: &mut [f32], brightness_hz: f32, drive: f32, body_hz: f32) {
    // ⛔ **The voice's own level is captured and restored.** This ended with a
    // flat `normalize(samples, 0.82)`, which threw away every per-voice target
    // the recipes set — kick asks for 0.92 and shaker for 0.55, a deliberate
    // 4.5 dB gap, and on disk all seventeen percussion voices measured exactly
    // 0.820 while the five unshaped pitched voices kept theirs. So the drums
    // came out level with each other and out of level with everything else,
    // which is the opposite of what `normalize`'s own doc says it is for.
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if brightness_hz < 20_000.0 {
        low_pass(samples, brightness_hz);
    }
    if body_hz > 0.0 {
        let mut body: Vec<f32> = samples.to_vec();
        band_pass(&mut body, body_hz, 0.8);
        for (s, b) in samples.iter_mut().zip(body) {
            *s += b * 0.35;
        }
    }
    if drive > 1.0 {
        for s in samples.iter_mut() {
            *s = saturate(*s, drive);
        }
    }
    declick(samples);
    normalize(samples, if peak > 1e-9 { peak } else { 0.82 });
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

// ──────────────────────────────────────────── the percussion lanes (TASK-140)
//
// ⛔ **Why these exist**: `drums.rs` gained eight lanes and 15 models were
// already authoring a `percs` block that nothing read. Without a pad each of
// them generates *silence*, and silence is the failure this project keeps
// recording — every test stays green while the producer hears nothing.
//
// ⚠ **Each voice is a recipe, not filtered noise.** `sweep_low_pass`'s comment
// above is the standard this file already set: a static filter pass over noise
// is why a preview reads as a toy. The metal voices below use the TR-808's own
// inharmonic square bank through a *resonant* band-pass, the membranes use pitch
// envelopes, and the hand percussion uses a real attack ramp.
//
// ⚠ **They must be tellable apart from their neighbours**, which is the same
// rule `rim` and `perc` already follow. A tambourine that sounds like an open
// hat has not been added, it has been duplicated.

/// Off-snare: the backbeat's answer, brighter and tighter than the main snare.
///
/// ⚠ **Deliberately not `snare()` with a tweak.** The two play in the same bar
/// and often a 16th apart, so they have to read as two drums: the shell sits a
/// fifth higher, the wires are shorter, and there is less body under it.
pub fn off_snare(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/off_snare");
    let n = seconds(0.16);
    let mut wires = noise(&mut rng, n);

    high_pass(&mut wires, 1_400.0);
    low_pass(&mut wires, 12_500.0);

    let mut out = Vec::with_capacity(n);
    for (i, wire) in wires.iter().enumerate() {
        let t = i as f32 / SR;
        // A fifth above the main snare's 185/331, and decaying faster.
        let body = (std::f32::consts::TAU * 278.0 * t).sin() * decay(t, 0.055) * 0.38
            + (std::f32::consts::TAU * 496.0 * t).sin() * decay(t, 0.032) * 0.22;
        out.push(body + wire * decay(t, 0.085));
    }

    declick(&mut out);
    normalize(&mut out, 0.80);
    out
}

/// Snap: fingers. Almost pure transient with one resonant pop.
///
/// ⛔ **This lane shipped with no pad at all and generated silence** — the
/// handoff recorded it as "No sound" and `trap.json` has been asking for it the
/// whole time. A snap is short, dry and mid-forward: the resonance around 2.2
/// kHz is the flesh-and-bone pop, and nothing rings after it.
pub fn snap(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/snap");
    let n = seconds(0.07);
    let mut out = noise(&mut rng, n);

    // The pop, before the envelope, so the filter rings rather than being cut.
    band_pass(&mut out, 2_200.0, 2.6);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= decay(t, 0.018);
    }
    high_pass(&mut out, 700.0);

    declick(&mut out);
    normalize(&mut out, 0.72);
    out
}

/// Ride: a defined stick ping over a shimmering bed.
///
/// ⚠ **The ping is what separates a ride from a crash**, and it is a *pitched*
/// transient rather than a louder one — the stick striking near the bell excites
/// the bank an octave up for a few tens of milliseconds. Without it the two
/// cymbals are one sound at two lengths.
pub fn ride(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/ride");
    let n = seconds(1.30);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, hiss) in source.iter().enumerate() {
        let t = i as f32 / SR;
        // The bed: the bank, ringing long.
        let bed = metal(t, 1.0) * decay(t, 0.95);
        // The ping: the same bank an octave up, gone in 60 ms.
        let ping = metal(t, 2.0) * decay(t, 0.06) * 0.9;
        out.push(bed * 0.55 + ping + hiss * decay(t, 0.10) * 0.22);
    }

    band_pass(&mut out, 7_200.0, 1.2);
    high_pass(&mut out, 3_000.0);
    declick(&mut out);
    normalize(&mut out, 0.62);
    out
}

/// Crash: a broad wash with no defined ping and a long, slow tail.
pub fn crash(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/crash");
    let n = seconds(2.20);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, hiss) in source.iter().enumerate() {
        let t = i as f32 / SR;
        // Two transpositions of the bank an augmented fourth apart, which is
        // the least consonant interval available — a crash should not imply a
        // pitch the way the ride's bell does.
        let plate = (metal(t, 1.0) + metal(t, 1.414) * 0.8) * decay(t, 1.70);
        // Noise carries most of a crash; the bank is what stops it being a hiss.
        out.push(plate * 0.40 + hiss * decay(t, 1.35) * 0.75);
    }

    // A brief swell: a crash blooms rather than clicking.
    for (i, s) in out.iter_mut().enumerate() {
        *s *= attack(i as f32 / SR, 0.006);
    }
    high_pass(&mut out, 2_400.0);
    // Opening downward as it decays — the highs die first on a real cymbal.
    sweep_low_pass(&mut out, 17_000.0, 6_500.0, 1.8);
    declick(&mut out);
    normalize(&mut out, 0.66);
    out
}

/// Tom: a tuned membrane. A pitch drop, but a gentle one.
///
/// ⚠ **Nothing like the 808's four-octave dive.** A drum head falls roughly a
/// major third as the skin relaxes; sweeping it further is what makes a
/// synthesized tom sound like a laser instead of a drum.
pub fn tom(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/tom");
    let n = seconds(0.45);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for (i, stick) in source.iter().enumerate() {
        let t = i as f32 / SR;
        // 155 Hz settling to ~124 — about a major third, over 90 ms.
        let freq = 124.0 * (1.0 + 0.25 * (-t / 0.09).exp());
        phase += std::f32::consts::TAU * freq / SR;
        let head = phase.sin() * decay(t, 0.30);
        // The second mode of a circular membrane, which is inharmonic.
        let mode = (std::f32::consts::TAU * freq * 1.59 * t).sin() * decay(t, 0.11) * 0.28;
        out.push(head + mode + stick * decay(t, 0.004) * 0.35);
    }

    low_pass(&mut out, 5_200.0);
    declick(&mut out);
    normalize(&mut out, 0.84);
    out
}

/// Shaker: many small beads, so it swells rather than strikes.
///
/// ⛔ **The attack is the whole instrument.** Identical noise with an instant
/// attack is a closed hat; given ~12 ms to build it becomes a shaker. That one
/// envelope is the difference, and it is why [`attack`] exists.
pub fn shaker(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/shaker");
    let n = seconds(0.11);
    let mut out = noise(&mut rng, n);

    band_pass(&mut out, 6_800.0, 0.9);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= attack(t, 0.012) * decay(t, 0.055);
    }
    high_pass(&mut out, 3_800.0);

    declick(&mut out);
    normalize(&mut out, 0.55);
    out
}

/// Tambourine: a handful of jingles, which is metal *and* scatter.
///
/// ⚠ **The scatter is not decoration.** Every jingle is a pair of loose discs
/// and they do not land together; without a few milliseconds of spread they sum
/// into one hit and the voice reads as a bright closed hat. Same reasoning as
/// `clap`'s three bursts, and the same trap avoided — each burst reads its own
/// slice of noise rather than a delayed copy of one slice, which would comb.
pub fn tambourine(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/tambourine");
    let n = seconds(0.34);
    let source = noise(&mut rng, n);
    let mut out = vec![0.0f32; n];

    // Six jingles over ~11 ms, quieter as they trail.
    for jingle in 0..6 {
        let offset = seconds(jingle as f32 * 0.0022);
        let level = 1.0 - jingle as f32 * 0.11;
        for i in 0..seconds(0.05) {
            if offset + i < n {
                let t = i as f32 / SR;
                // ⛔ **Absolute time, not the intra-burst index.** With `t`
                // the ring was bit-identical in all six jingles, offset by a
                // fixed number of samples — a comb, notching every ~455 Hz,
                // which is exactly what the note above says this avoids. The
                // noise half already read absolutely; the metal half did not.
                let ring = metal((offset + i) as f32 / SR, 3.1) * decay(t, 0.030);
                out[offset + i] += (ring * 0.5 + source[offset + i] * decay(t, 0.012)) * level;
            }
        }
    }

    // The shimmer left hanging after the strike.
    let tail = seconds(0.05);
    for i in tail..n {
        let t = (i - tail) as f32 / SR;
        out[i] += (metal(t, 3.1) * 0.3 + source[i] * 0.5) * decay(t, 0.16) * 0.30;
    }

    band_pass(&mut out, 9_000.0, 1.1);
    high_pass(&mut out, 4_500.0);
    declick(&mut out);
    normalize(&mut out, 0.64);
    out
}

/// Cowbell: the 808's, which is two of the six squares and nothing else.
///
/// ⛔ **This is the actual recipe, not an approximation.** The TR-808 cowbell is
/// the 540 Hz and 800 Hz oscillators from [`METAL_PARTIALS`], band-passed, with
/// a fast decay. Their ratio is 1.481 — close to a fifth and audibly not one,
/// which is exactly why the sound is unmistakable.
pub fn cowbell(seed: u64) -> Vec<f32> {
    // ⚠ Seeded only to break the tie between families. The bank is two square
    // oscillators with no noise in it, so the seed moves the strike's tiny
    // noise floor rather than the tone — without it `trap-default/cowbell.wav`
    // and `country-default/cowbell.wav` were the same file, byte for byte.
    let mut rng = stream(seed, "kit/cowbell");
    let n = seconds(0.40);
    let source = noise(&mut rng, n);
    let mut out = Vec::with_capacity(n);

    for (i, strike_noise) in source.iter().enumerate().take(n) {
        let t = i as f32 / SR;
        let two =
            square(std::f32::consts::TAU * 540.0 * t) + square(std::f32::consts::TAU * 800.0 * t);
        // A short body under a longer ring, which is the enclosure resonating
        // after the strike stops driving it.
        let strike = strike_noise * decay(t, 0.003) * 0.04;
        out.push(two * 0.5 * (decay(t, 0.30) * 0.7 + decay(t, 0.020) * 0.5) + strike);
    }

    band_pass(&mut out, 2_640.0, 1.6);
    high_pass(&mut out, 500.0);
    declick(&mut out);
    normalize(&mut out, 0.68);
    out
}

/// Woodblock: a hard, dry, high knock with almost no tail.
///
/// ⚠ **Kept clearly above `perc`'s 840 Hz.** `uk-drill` authors `rim` and
/// `woodblock` together, so those two land in the same bar and must not be the
/// same click at two volumes.
pub fn woodblock(seed: u64) -> Vec<f32> {
    let mut rng = stream(seed, "kit/woodblock");
    let n = seconds(0.10);
    let source = noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, knock) in source.iter().enumerate() {
        let t = i as f32 / SR;
        // Two inharmonic modes: a block is a bar, not a string.
        let tone = (std::f32::consts::TAU * 2_360.0 * t).sin() * decay(t, 0.026)
            + (std::f32::consts::TAU * 3_640.0 * t).sin() * decay(t, 0.014) * 0.45;
        out.push(tone * 0.85 + knock * decay(t, 0.0035) * 0.30);
    }

    high_pass(&mut out, 900.0);
    low_pass(&mut out, 11_000.0);
    declick(&mut out);
    normalize(&mut out, 0.71);
    out
}

/// Kick: a short, tight sine drop, separate from the 808's long body.
pub fn kick(seed: u64) -> Vec<f32> {
    // ⚠ Same reason as `cowbell`: a pure sine drop is identical in every
    // family, so the beater noise carries the family's seed.
    let mut rng = stream(seed, "kit/kick");
    let n = seconds(0.35);
    let source = noise(&mut rng, n);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;

    for (i, beater_noise) in source.iter().enumerate().take(n) {
        let t = i as f32 / SR;
        let freq = 52.0 * (1.0 + 5.0 * (-t / 0.018).exp());
        phase += std::f32::consts::TAU * freq / SR;
        // The beater against the head: a few milliseconds of noise over the
        // sine drop. It is what a kick's attack actually is, and it is also
        // what carries the family's seed — without it a pure sine drop is
        // byte-identical in every kit.
        let beater = beater_noise * decay(t, 0.0025) * 0.18;
        out.push(saturate(phase.sin() * decay(t, 0.22) + beater, 1.8));
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
            ("kick", kick(1)),
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
        assert_eq!(kick(1), kick(1));
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
        for v in [closed_hat(3), snare(3), clap(3), kick(1)] {
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
        assert!(eight_o_eight(41.2, 1.4, 2.2).len() > kick(1).len() * 3);
    }
}
