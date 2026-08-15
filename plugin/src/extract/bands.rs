//! Splitting a decoded buffer into the bands a kit separates on (TASK-058F).
//!
//! ⛔ **No FFT, and that is a decision rather than an omission.** The same one
//! [`crate::audio::pitch`] records: an FFT here means a new dependency in a crate
//! that is loaded into somebody else's DAW and audited by `deny.toml`, and
//! everything this module is asked for — "how much energy is under 120 Hz", "how
//! much is between 2 and 8 kHz" — is a *band* question rather than a spectrum
//! question. Two cascaded biquads answer it in twenty lines and no supply chain.
//!
//! ⚠ **Forward filtering, so every band arrives a little late.** A 4-pole
//! low-pass at 120 Hz has a group delay around 2 ms; the 2–8 kHz band has almost
//! none, so the low band is systematically ~2 ms behind the high one. That is
//! *not* corrected, on a measurement: a sixteenth at 140 BPM is 107 ms, and the
//! grid these onsets are quantised onto is a forty-eighth of a bar. Two
//! milliseconds is two per cent of the smallest thing this can express.
//! ⛔ Zero-phase (forward-then-backward) filtering was the alternative and is
//! worse here: it smears a transient *backwards*, which moves a kick's onset
//! earlier by more than the delay it removes.

/// One biquad section, direct form I.
///
/// ⚠ `f64` state on `f32` audio. A 120 Hz low-pass at 48 kHz puts its poles very
/// close to the unit circle, where the difference between the coefficients and
/// their single-precision rounding is audible as a slowly drifting DC offset —
/// and an energy envelope built on a drifting offset reads as a band that never
/// decays.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

/// The two section Qs of a 4th-order Butterworth, low to high.
///
/// ⚠ Cascading two identical biquads does *not* give a Butterworth — it gives a
/// response that is already 6 dB down at the corner. These are the standard pair.
const BUTTERWORTH_4: [f64; 2] = [0.541_196_1, 1.306_562_9];

impl Biquad {
    /// RBJ's cookbook coefficients, normalised by `a0`.
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn low_pass(rate: f64, hz: f64, q: f64) -> Self {
        let (cos, alpha) = shape(rate, hz, q);
        Self::new(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn high_pass(rate: f64, hz: f64, q: f64) -> Self {
        let (cos, alpha) = shape(rate, hz, q);
        Self::new(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// `(cos w0, alpha)` for a corner and a Q.
///
/// ⚠ The corner is clamped below Nyquist. A caller asking for a 16 kHz band on a
/// 22.05 kHz file is asking for something that does not exist there, and past
/// Nyquist the coefficients describe a filter that grows rather than one that
/// does nothing.
fn shape(rate: f64, hz: f64, q: f64) -> (f64, f64) {
    let hz = hz.clamp(1.0, rate * 0.45);
    let w0 = 2.0 * std::f64::consts::PI * hz / rate;
    (w0.cos(), w0.sin() / (2.0 * q))
}

/// Everything between `low_hz` and `high_hz`, at 24 dB per octave each side.
///
/// `None` on either side means "no corner there": `band(x, rate, None, Some(120.0))`
/// is a low-pass and `band(x, rate, Some(8_000.0), None)` is a high-pass.
pub fn band(samples: &[f32], rate: u32, low_hz: Option<f32>, high_hz: Option<f32>) -> Vec<f32> {
    let rate = f64::from(rate.max(1));
    let mut stages: Vec<Biquad> = Vec::with_capacity(4);
    if let Some(hz) = low_hz {
        for q in BUTTERWORTH_4 {
            stages.push(Biquad::high_pass(rate, f64::from(hz), q));
        }
    }
    if let Some(hz) = high_hz {
        for q in BUTTERWORTH_4 {
            stages.push(Biquad::low_pass(rate, f64::from(hz), q));
        }
    }

    samples
        .iter()
        .map(|sample| {
            let mut value = f64::from(*sample);
            for stage in &mut stages {
                value = stage.run(value);
            }
            value as f32
        })
        .collect()
}

/// How many one-pole sections smooth a rectified band.
///
/// ⚠ **Cascaded one-poles rather than one Butterworth**, and the reason is
/// overshoot: a 4th-order Butterworth rings about 11% past a step, so a single
/// struck note produces rise-fall-rise in its own envelope — and the detector
/// above reads the second rise as a second hit. Identical one-poles cannot
/// overshoot at all.
const SMOOTHING_POLES: usize = 4;

/// The level of `samples` over time, sampled every `hop`.
///
/// Rectify, then smooth at `cutoff_hz`.
///
/// ⛔⛔ **Neither block RMS nor a peak follower, and both were tried.** Five
/// milliseconds of a 55 Hz kick is a quarter of one cycle, so its block RMS rises
/// and falls twenty times on the way down and *every* one of those is an onset —
/// a four-kick loop came back as twenty hits. A peak follower has the same
/// disease for the same reason: between two peaks of a 55 Hz wave it releases for
/// nine milliseconds and then gets pushed back up, once per cycle, forever.
///
/// ▶ **A rectified band low-passed well below its own frequency has nothing left
/// to ripple.** The rectifier puts the ripple at *twice* the band's frequency and
/// four poles take it out; what is left is the level.
///
/// ⚠ **`cutoff_hz` is per band and it is not a preference.** It has to sit far
/// under twice the band's lowest frequency or the ripple survives — and it is
/// simultaneously a floor under every decay this band can report, which is why
/// the hat bands get a much higher one: closed-versus-open is *decided* on decay
/// length, and a 25 Hz smoother would make both of them 25 ms.
pub fn envelope(samples: &[f32], rate: u32, cutoff_hz: f32, hop: usize) -> Vec<f32> {
    let hop = hop.max(1);
    // One-pole coefficient from a corner: `1 - exp(-2π f / rate)`.
    let a = 1.0
        - f64::exp(
            -2.0 * std::f64::consts::PI * f64::from(cutoff_hz.max(0.1)) / f64::from(rate.max(1)),
        );

    let mut poles = [0.0_f64; SMOOTHING_POLES];
    let mut out: Vec<f32> = Vec::with_capacity(samples.len() / hop + 1);
    let mut block_peak = 0.0_f32;
    // ⚠ **A countdown, not `at % hop`.** `hop` is a runtime value, so the modulo
    // is a hardware division on **every input sample** — four bands over a
    // sixty-second file is 11.5 million of them, for a boundary a decrement
    // already knows about.
    let mut until_emit = hop;
    for sample in samples {
        let mut value = f64::from(sample.abs());
        for pole in &mut poles {
            *pole += (value - *pole) * a;
            value = *pole;
        }
        // ⚠ The **peak within the block**, not the value at its end. Sampling the
        // end alone loses a transient that lands early in a block to its own
        // decay, which moves a hit one hop later than it happened.
        block_peak = block_peak.max(value as f32);
        until_emit -= 1;
        if until_emit == 0 {
            out.push(block_peak);
            block_peak = 0.0;
            until_emit = hop;
        }
    }
    if !samples.len().is_multiple_of(hop) {
        out.push(block_peak);
    }
    out
}

/// Drop the rate by `factor`, low-passing first so nothing aliases down.
///
/// ⛔⛔ **This is what makes a pitch *track* affordable at all.** The NSDF in
/// [`crate::audio::pitch`] is O(window · lags), and both scale with the sample
/// rate — a 50 ms bass window at 48 kHz is 2,400 samples against 1,200 lags,
/// about 3 million operations for **one** frame of one line. Decimating 48 kHz to
/// 6 kHz before tracking a band that ends at 250 Hz costs nothing musically and
/// takes the same frame to 45 thousand. Sixty times less work for a signal that
/// had nothing above 3 kHz in it.
///
/// ⚠ **The anti-alias filter is not optional even though the band was already
/// filtered.** A 4-pole low-pass is 24 dB/octave, not a wall: a hat two octaves
/// above the corner is still 48 dB down rather than gone, and decimation folds it
/// back into the band as a tone that was never played.
pub fn decimate(samples: &[f32], rate: u32, factor: usize) -> (Vec<f32>, u32) {
    let factor = factor.max(1);
    if factor == 1 {
        return (samples.to_vec(), rate);
    }
    let out_rate = (rate / factor as u32).max(1);
    // Below the new Nyquist with room to spare, because the filter is a slope.
    let guarded = band(samples, rate, None, Some(out_rate as f32 * 0.4));
    (guarded.iter().step_by(factor).copied().collect(), out_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of a sine at `hz`.
    fn sine(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    /// The second half, so the filter's own settling is not what is measured.
    fn settled(samples: &[f32]) -> &[f32] {
        &samples[samples.len() / 2..]
    }

    #[test]
    fn a_band_passes_what_is_inside_it_and_stops_what_is_not() {
        let inside = band(&sine(300.0, 48_000, 0.5), 48_000, Some(150.0), Some(400.0));
        assert!(
            rms(settled(&inside)) > 0.6,
            "300 Hz must survive a 150–400 band, got {}",
            rms(settled(&inside))
        );

        // Two octaves clear of each corner, where 24 dB/octave is ~48 dB down.
        for (hz, name) in [(40.0, "below"), (3_000.0, "above")] {
            let outside = band(&sine(hz, 48_000, 0.5), 48_000, Some(150.0), Some(400.0));
            assert!(
                rms(settled(&outside)) < 0.05,
                "{hz} Hz ({name}) leaked through at {}",
                rms(settled(&outside))
            );
        }
    }

    #[test]
    fn one_open_side_is_a_plain_low_or_high_pass() {
        let low = band(&sine(40.0, 48_000, 0.5), 48_000, None, Some(120.0));
        assert!(rms(settled(&low)) > 0.6, "{}", rms(settled(&low)));

        let high = band(&sine(40.0, 48_000, 0.5), 48_000, Some(2_000.0), None);
        assert!(rms(settled(&high)) < 0.01, "{}", rms(settled(&high)));
    }

    #[test]
    fn a_corner_past_nyquist_is_clamped_rather_than_blowing_the_filter_up() {
        // ⛔ A caller asking for a 16 kHz band on a 22.05 kHz file is asking for
        // something that is not there. Unclamped, the coefficients go unstable
        // and the "band" comes back as an exponentially growing NaN — which then
        // reaches an envelope, a threshold, and a note.
        let out = band(&sine(400.0, 22_050, 0.2), 22_050, Some(16_000.0), None);
        assert!(
            out.iter().all(|s| s.is_finite()),
            "the filter went unstable"
        );
    }

    #[test]
    fn the_envelope_never_rises_while_a_note_is_only_decaying() {
        // ⛔⛔ **The property the whole onset detector rests on.** A block-RMS
        // envelope of a 55 Hz decay ripples at 55 Hz — and so does a peak
        // follower, which releases for nine milliseconds between cycle peaks and
        // is pushed straight back up. Either way the detector above reads every
        // ripple as a new hit: a four-kick loop came back as twenty.
        let decaying: Vec<f32> = (0..48_000)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                (-t * 6.0).exp() * (2.0 * std::f32::consts::PI * 55.0 * t).sin()
            })
            .collect();
        // Past the attack, so what is measured is the decay rather than the rise.
        let env = envelope(&decaying, 48_000, 25.0, 240);
        let rises = env[10..]
            .windows(2)
            .filter(|pair| pair[1] > pair[0])
            .count();
        assert_eq!(rises, 0, "the envelope rose {rises} times inside one decay");
    }

    #[test]
    fn the_envelope_reaches_the_level_and_comes_back_to_silence() {
        let mut samples = sine(1_000.0, 48_000, 0.4);
        samples.extend(std::iter::repeat_n(0.0, 48_000));
        let env = envelope(&samples, 48_000, 200.0, 480);
        // A rectified full-scale sine averages 2/π, which is what a smoother far
        // below its ripple settles at.
        assert!(
            (0.5..0.75).contains(&env[20]),
            "a full-scale sine rectifies to ~0.637, got {}",
            env[20]
        );
        assert!(
            *env.last().expect("blocks") < 1e-6,
            "silence must read as silence, got {:?}",
            env.last()
        );
    }

    #[test]
    fn decimating_keeps_the_pitch_it_was_told_to_keep() {
        // ⛔ The point of the anti-alias filter: a 5 kHz tone decimated to 6 kHz
        // without one folds down to 1 kHz and becomes a note nobody played.
        let mut mixed = sine(100.0, 48_000, 0.5);
        for (at, sample) in sine(5_000.0, 48_000, 0.5).iter().enumerate() {
            mixed[at] += sample;
        }
        let (small, rate) = decimate(&mixed, 48_000, 8);
        assert_eq!(rate, 6_000);
        assert_eq!(small.len(), mixed.len().div_ceil(8));

        // What is left is the 100 Hz, at close to its original level.
        let level = rms(settled(&small));
        assert!(
            (0.5..0.9).contains(&level),
            "the 100 Hz should survive alone, got {level}"
        );
    }
}
