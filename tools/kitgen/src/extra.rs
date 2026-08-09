//! The voices TASK-043A added, built out of the ones that were already here.
//!
//! ⛔ **Sixteen lanes, five shapes.** A high tom, a conga, a bongo and a timbale
//! are one instrument — a struck membrane — at four pitches and four decays, and
//! writing four hand-tuned oscillator loops for them would be four places to fix
//! the same bug. `voices.rs` already made that argument for [`voices::metal`];
//! this file makes it for the rest.
//!
//! ⚠ **Every one of these is a *default*.** The point of the lane is that a
//! producer can drop their own sample on it; these exist so the lane is audible
//! before they do, because `Kit::pad_for` answering `None` is silence and
//! silence is harder to notice than a wrong drum.

use crate::voices::{self, SR};

/// A struck membrane: a pitch that bends down into its fundamental, an
/// inharmonic second mode, and a stick transient on top.
///
/// This is `voices::tom`'s algorithm with its four constants opened up. The
/// toms, the congas, the bongo and the timbale are all this.
fn membrane(
    seed: u64,
    domain: &str,
    fundamental_hz: f32,
    bend: f32,
    bend_s: f32,
    body_s: f32,
    brightness_hz: f32,
) -> Vec<f32> {
    let mut rng = voices::stream(seed, domain);
    let n = voices::seconds(body_s * 1.5);
    let source = voices::noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for (i, stick) in source.iter().enumerate() {
        let t = i as f32 / SR;
        let freq = fundamental_hz * (1.0 + bend * (-t / bend_s).exp());
        phase += std::f32::consts::TAU * freq / SR;
        let head = phase.sin() * voices::decay(t, body_s);
        // The second mode of a circular membrane, which is inharmonic — it is
        // what stops this reading as a sine blip.
        let mode = (std::f32::consts::TAU * freq * 1.59 * t).sin()
            * voices::decay(t, body_s * 0.37)
            * 0.28;
        out.push(head + mode + stick * voices::decay(t, 0.004) * 0.35);
    }

    voices::low_pass(&mut out, brightness_hz);
    voices::declick(&mut out);
    voices::normalize(&mut out, 0.84);
    out
}

/// Struck metal: [`voices::metal`]'s inharmonic partials with a decay and a
/// high-pass of their own. The ride bell and the triangle are this.
fn struck_metal(seed: u64, domain: &str, transpose: f32, ring_s: f32, floor_hz: f32) -> Vec<f32> {
    let mut rng = voices::stream(seed, domain);
    let n = voices::seconds(ring_s * 1.2);
    let source = voices::noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    for (i, grit) in source.iter().enumerate() {
        let t = i as f32 / SR;
        let ring = voices::metal(t, transpose) * voices::decay(t, ring_s);
        // The stick, and the only thing carrying the family's seed — a pure
        // partial stack is byte-identical in every kit, which is the argument
        // `voices::cowbell` already makes.
        out.push(ring + grit * voices::decay(t, 0.003) * 0.2);
    }

    voices::high_pass(&mut out, floor_hz);
    voices::declick(&mut out);
    voices::normalize(&mut out, 0.7);
    out
}

/// A sine that drops in pitch and dies — the kick's body without its beater.
fn sine_drop(
    seed: u64,
    domain: &str,
    from_hz: f32,
    to_hz: f32,
    drop_s: f32,
    body_s: f32,
    drive: f32,
) -> Vec<f32> {
    let mut rng = voices::stream(seed, domain);
    let n = voices::seconds(body_s * 1.4);
    let source = voices::noise(&mut rng, n);

    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for (i, thud) in source.iter().enumerate() {
        let t = i as f32 / SR;
        let freq = to_hz + (from_hz - to_hz) * (-t / drop_s).exp();
        phase += std::f32::consts::TAU * freq / SR;
        let body = phase.sin() * voices::decay(t, body_s);
        out.push(voices::saturate(
            body + thud * voices::decay(t, 0.003) * 0.12,
            drive,
        ));
    }

    voices::declick(&mut out);
    voices::normalize(&mut out, 0.92);
    out
}

/// Filtered noise with an envelope — the hats' shape, opened up.
///
/// ⚠ Eight parameters, and clippy is right that it is a lot. They are the eight
/// numbers that separate a pedal hat from a ghost snare, though, and bundling
/// them into a struct would put a type between the caller and the only thing it
/// is saying — which of the eight it wants different.
#[allow(clippy::too_many_arguments)]
fn noise_hit(
    seed: u64,
    domain: &str,
    length_s: f32,
    centre_hz: f32,
    q: f32,
    attack_s: f32,
    decay_s: f32,
    peak: f32,
) -> Vec<f32> {
    let mut rng = voices::stream(seed, domain);
    let n = voices::seconds(length_s);
    let mut out = voices::noise(&mut rng, n);

    voices::band_pass(&mut out, centre_hz, q);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= voices::attack(t, attack_s) * voices::decay(t, decay_s);
    }

    voices::declick(&mut out);
    voices::normalize(&mut out, peak);
    out
}

// ── The kick family ──────────────────────────────────────────────────────────

/// The second, lower kick — boom bap's two-kick layering.
///
/// ⚠ Deliberately duller and longer than [`voices::kick`], with no beater at
/// all: a sub kick that had one would be a second kick rather than a layer, and
/// the two would flam.
pub fn sub_kick(seed: u64) -> Vec<f32> {
    sine_drop(seed, "kit/subKick", 130.0, 41.0, 0.030, 0.34, 1.2)
}

// ── The snare family ─────────────────────────────────────────────────────────

/// The quiet answering snare on the e/a slots.
///
/// ⛔ **Normalised to 0.55, not to full.** A ghost is defined by being quiet,
/// and normalising it like every other pad would leave the *velocity* as the
/// only thing making it one — so a producer who painted a ghost lane flat would
/// get a second backbeat.
pub fn ghost_snare(seed: u64) -> Vec<f32> {
    noise_hit(
        seed,
        "kit/ghostSnare",
        0.075,
        2_400.0,
        0.7,
        0.0004,
        0.035,
        0.55,
    )
}

// ── The hats ─────────────────────────────────────────────────────────────────

/// The hat closed with the foot: shorter and darker than the stick-struck one.
pub fn pedal_hat(seed: u64) -> Vec<f32> {
    noise_hit(
        seed,
        "kit/pedalHat",
        0.055,
        5_600.0,
        1.1,
        0.0006,
        0.020,
        0.62,
    )
}

/// The ride struck on its bell — a ping with a fundamental, not a wash.
pub fn ride_bell(seed: u64) -> Vec<f32> {
    struck_metal(seed, "kit/rideBell", 1.9, 1.10, 900.0)
}

/// A triangle: the same metal, tiny and ringing for a long time.
pub fn triangle(seed: u64) -> Vec<f32> {
    struck_metal(seed, "kit/triangle", 4.2, 1.60, 3_600.0)
}

// ── The membranes ────────────────────────────────────────────────────────────

pub fn tom_high(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/tomHigh", 196.0, 0.25, 0.075, 0.24, 6_000.0)
}

pub fn tom_low(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/tomLow", 82.0, 0.22, 0.110, 0.42, 4_200.0)
}

/// A second generic percussion body, pitched between the perc and the toms.
pub fn perc2(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/perc2", 260.0, 0.30, 0.045, 0.16, 7_400.0)
}

pub fn conga(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/conga", 220.0, 0.18, 0.055, 0.20, 5_600.0)
}

pub fn bongo(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/bongo", 340.0, 0.22, 0.030, 0.11, 8_200.0)
}

/// A timbale: a metal shell, so it rings brighter and longer than a conga.
pub fn timbale(seed: u64) -> Vec<f32> {
    membrane(seed, "kit/timbale", 300.0, 0.16, 0.040, 0.26, 9_500.0)
}

// ── Wood ─────────────────────────────────────────────────────────────────────

/// Claves: drier and higher than the woodblock, which is the whole difference.
pub fn clave(seed: u64) -> Vec<f32> {
    let mut out = membrane(seed, "kit/clave", 2_400.0, 0.06, 0.004, 0.045, 12_000.0);
    voices::high_pass(&mut out, 900.0);
    voices::normalize(&mut out, 0.8);
    out
}

// ── FX ───────────────────────────────────────────────────────────────────────

/// A riser: noise whose band climbs for a second and gets louder doing it.
///
/// ⛔ **It swells rather than strikes**, which is why it cannot come out of
/// [`noise_hit`] — every other voice here decays from its own attack, and this
/// one is the opposite shape.
pub fn riser(seed: u64) -> Vec<f32> {
    let mut rng = voices::stream(seed, "kit/riser");
    let n = voices::seconds(1.2);
    let source = voices::noise(&mut rng, n);

    // The sweep is done by summing two band-passed copies rather than by
    // modulating one filter, because the filters here are one-pole and cannot
    // be retuned per sample without ringing.
    let mut low = source.clone();
    voices::band_pass(&mut low, 700.0, 0.8);
    let mut high = source;
    voices::band_pass(&mut high, 7_000.0, 0.8);

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let travel = (t / 1.0).clamp(0.0, 1.0);
        // Crossfade low → high while the whole thing swells.
        let blend = low[i] * (1.0 - travel) + high[i] * travel;
        out.push(blend * (0.08 + 0.92 * travel * travel));
    }

    voices::declick(&mut out);
    voices::normalize(&mut out, 0.78);
    out
}

/// An impact: the boom a section lands on. A long low drop with noise on it.
pub fn impact(seed: u64) -> Vec<f32> {
    let mut out = sine_drop(seed, "kit/impact", 240.0, 33.0, 0.090, 1.05, 2.4);
    voices::low_pass(&mut out, 3_000.0);
    voices::normalize(&mut out, 0.95);
    out
}

/// A reverse: a cymbal played backwards, which is a swell into a cut.
///
/// ⛔ **Built by reversing [`voices::crash`] rather than by writing a swell.**
/// That is what a reverse cymbal *is* — and it means the reverse and the crash
/// in a kit are audibly the same instrument, which they should be.
pub fn reverse(seed: u64) -> Vec<f32> {
    let mut out = voices::crash(seed);
    out.reverse();
    voices::declick(&mut out);
    voices::normalize(&mut out, 0.85);
    out
}

// ── The 808's sub layer ──────────────────────────────────────────────────────

/// A clean sine under the distorted 808 — pitched, so it carries a root note.
///
/// ⚠ **No drive at all, and that is the point.** [`voices::eight_o_eight`] is
/// saturated because distortion is what makes an 808 audible on a phone; this
/// is the layer underneath that keeps the fundamental intact on a system that
/// can reproduce it.
pub fn sub_low(root_hz: f32, length_s: f32) -> Vec<f32> {
    let n = voices::seconds(length_s);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / SR;
        phase += std::f32::consts::TAU * root_hz / SR;
        // A long, flat body with a short fade — a sub that decays like a drum
        // would not sustain under a bassline.
        out.push(phase.sin() * voices::decay(t, length_s * 0.9));
    }
    voices::declick(&mut out);
    voices::normalize(&mut out, 0.9);
    out
}
