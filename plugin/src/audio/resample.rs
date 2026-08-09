//! Band-limited sample-rate conversion (TASK-053).
//!
//! ⛔⛔ **This never runs on the audio thread, and it structurally cannot.**
//! `rubato` is a *block* resampler: it allocates, it holds internal filter state
//! sized at construction, and it processes in chunks — every one of which is
//! forbidden inside `process`. It runs on the loader thread that already decodes
//! a producer's one-shot, and what reaches the audio thread is a finished
//! buffer, exactly as [`super::import`] already promises.
//!
//! ## What this fixes, and what it deliberately does not
//!
//! [`super::sampler`] reads a pad with **linear interpolation** at
//! `pad.sample_rate / device_rate`. The shipped kit is 44.1 kHz and a device
//! usually is not, so until now *every hit of every pad* played through a
//! permanent linear resample — audible on hats and on anything with air in it,
//! and paid on every note forever. Converting once, here, with a windowed-sinc
//! filter means that ratio becomes exactly 1.0 and the interpolator has nothing
//! left to do on an unpitched pad.
//!
//! ⚠ **The per-note pitch shift is still linear, on purpose.** Transposing a
//! melody one-shot to the note being played is a different ratio per voice per
//! note; a band-limited resampler cannot produce that on the callback. Linear is
//! the "linear fallback" the roadmap names, and it is now fed a correctly-rated
//! source rather than compounding two errors.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// Ratios beyond this are refused rather than filtered.
///
/// ⛔ A bound because the rate comes from a decoded file and from a device, and
/// neither is ours. 8× covers every real pairing — 8 kHz voice memo into a
/// 192 kHz interface is 24×, and that is not a sample anybody is playing a kit
/// with — while stopping an absurd header from asking for a filter the size of
/// the buffer.
const MAX_RATIO: f64 = 8.0;

/// The chunk the resampler is built around.
///
/// Any size works; this one keeps the filter state small while still giving the
/// sinc window enough to chew on. It has no relationship to the host's buffer
/// size, because none of this happens near the host's buffer.
const CHUNK: usize = 1024;

/// `samples` at `to` Hz, band-limited.
///
/// Returns the input untouched when the rates already match, when either is
/// zero, or when the ratio is beyond [`MAX_RATIO`] — and **falls back to linear
/// interpolation** if the filter cannot be built, because a sample that plays
/// slightly rough is better than a pad that does not play.
pub fn to_rate(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == 0 || to == 0 || from == to || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = f64::from(to) / f64::from(from);
    if !(1.0 / MAX_RATIO..=MAX_RATIO).contains(&ratio) {
        return linear(samples, ratio);
    }

    let params = SincInterpolationParameters {
        // 256 taps is the quality end of what is sensible offline; the cost is
        // paid once per imported file rather than per note.
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let Ok(mut resampler) = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, 1) else {
        return linear(samples, ratio);
    };

    // ⚠ The tail is padded to a whole chunk and trimmed afterwards. A fixed-in
    // resampler consumes exactly `CHUNK` frames per call, so the last partial
    // chunk would otherwise be dropped — which is the end of every one-shot.
    //
    // ⛔ **There is deliberately no `output_delay()` compensation here, and it
    // was measured rather than assumed.** A review argued that the filter primes
    // with zeros, so the first `output_delay()` frames are its ramp and dropping
    // them is required. `SincFixedIn` *reports* a delay of 139 at 44.1→48 — and
    // its output nonetheless starts at frame **3** and the source's 2,000-frame
    // burst ends at frame **2,176**, which is exactly `2000 × ratio`. The
    // alignment is already correct; draining 139 frames cut that much off the
    // *front of the audio* and shifted everything early.
    // `the_sound_starts_where_it_started_rather_than_after_the_filter_warms_up`
    // is what says so, and it is a burst rather than a tone precisely because a
    // continuous sine cannot see either error.
    let mut out: Vec<f32> = Vec::with_capacity((samples.len() as f64 * ratio) as usize + CHUNK);
    let mut input: Vec<f32> = Vec::with_capacity(CHUNK);

    for chunk in samples.chunks(CHUNK) {
        input.clear();
        input.extend_from_slice(chunk);
        input.resize(CHUNK, 0.0);

        let Ok(done) = resampler.process(&[&input], None) else {
            return linear(samples, ratio);
        };
        if let Some(channel) = done.first() {
            out.extend_from_slice(channel);
        }
    }

    // Trim the silence the padding produced, to the length the ratio implies.
    let expected = (samples.len() as f64 * ratio).round() as usize;
    out.truncate(expected.min(out.len()));
    out
}

/// The fallback: plain linear interpolation at `ratio`.
///
/// ⚠ Not dead code and not a placeholder — it is what runs when a rate pairing
/// is beyond what the filter will take, and what runs if the filter refuses to
/// build. Silence would be the alternative, and this module exists to make a
/// producer's sample *sound*.
fn linear(samples: &[f32], ratio: f64) -> Vec<f32> {
    let len = ((samples.len() as f64) * ratio).round() as usize;
    (0..len)
        .map(|i| {
            let at = i as f64 / ratio;
            let index = at as usize;
            let Some(here) = samples.get(index) else {
                return 0.0;
            };
            let next = samples.get(index + 1).copied().unwrap_or(*here);
            let frac = (at - index as f64) as f32;
            here + (next - here) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `hz`, `seconds` long, at `rate`.
    fn sine(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    /// Peak absolute value in the steady middle, away from the filter's edges.
    fn body_peak(samples: &[f32]) -> f32 {
        let skip = samples.len() / 4;
        samples[skip..samples.len() - skip]
            .iter()
            .fold(0.0_f32, |acc, s| acc.max(s.abs()))
    }

    #[test]
    fn matching_rates_are_handed_back_untouched() {
        // ⚠ Not merely an optimisation: the common case is a shipped pad already
        // at the device rate, and running it through a filter would colour it
        // for nothing.
        let input = sine(440.0, 48_000, 0.1);
        assert_eq!(to_rate(&input, 48_000, 48_000), input);
    }

    #[test]
    fn the_length_follows_the_ratio() {
        let input = sine(440.0, 44_100, 0.5);
        let up = to_rate(&input, 44_100, 48_000);
        let expected = (input.len() as f64 * 48_000.0 / 44_100.0).round() as usize;
        assert_eq!(up.len(), expected);

        let down = to_rate(&input, 44_100, 22_050);
        assert_eq!(down.len(), input.len() / 2);
    }

    #[test]
    fn a_tone_survives_conversion_at_its_own_amplitude() {
        // ⛔ **The tail is the point.** A fixed-input resampler consumes whole
        // chunks, so the last partial chunk of every one-shot is dropped unless
        // it is padded — and the end of a sample is its release. A tone that
        // came back quiet, or short, would be that bug.
        let input = sine(1_000.0, 44_100, 0.25);
        let out = to_rate(&input, 44_100, 48_000);

        assert!(
            (body_peak(&out) - 1.0).abs() < 0.05,
            "a unit sine came back at {}",
            body_peak(&out)
        );
        assert!(
            out.iter().rev().take(64).any(|s| s.abs() > 0.1),
            "the tail was dropped rather than converted"
        );
    }

    #[test]
    fn the_sound_starts_where_it_started_rather_than_after_the_filter_warms_up() {
        // ⛔⛔ **Where the sound sits in the output, pinned — because a review
        // argued it was wrong and the measurement said otherwise.** The claim
        // was that a windowed-sinc filter primes with zeros, so its first
        // `output_delay()` frames are the ramp and must be dropped.
        // `SincFixedIn` *reports* 139 at 44.1→48; its output actually begins at
        // frame 3 and this 2,000-frame burst ends at 2,176, which is exactly
        // `2000 × ratio`. Dropping 139 would cut the front off every pad.
        //
        // ⚠ A **burst**, not a continuous tone, and that is the whole point: a
        // sine has energy everywhere, so it cannot see a shift in either
        // direction. Whichever way this ever breaks — late, early, or clipped —
        // this is the test that notices.
        let rate = 44_100;
        let burst: Vec<f32> = (0..rate / 10)
            .map(|i| {
                if i < 2_000 {
                    (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / rate as f32).sin()
                } else {
                    0.0
                }
            })
            .collect();

        let out = to_rate(&burst, rate, 48_000);

        // The attack is at the front, not a few hundred frames in.
        let first_loud = out
            .iter()
            .position(|s| s.abs() > 0.3)
            .expect("it must sound");
        assert!(
            first_loud < 64,
            "the sound starts {first_loud} frames in — the filter's ramp was kept"
        );

        // And the burst still ends where it should, rather than being pushed
        // past the end and clipped.
        let last_loud = out
            .iter()
            .rposition(|s| s.abs() > 0.3)
            .expect("it must end");
        let expected_end = (2_000.0 * 48_000.0 / 44_100.0) as usize;
        assert!(
            last_loud.abs_diff(expected_end) < 128,
            "the burst ends at {last_loud}, expected about {expected_end}"
        );
    }

    #[test]
    fn an_absurd_pairing_falls_back_rather_than_building_a_filter_for_it() {
        // 8 kHz into 192 kHz is 24×. Refused by the filter, still answered —
        // a sample that plays slightly rough beats a pad that does not play.
        let input = sine(300.0, 8_000, 0.05);
        let out = to_rate(&input, 8_000, 192_000);
        assert_eq!(out.len(), input.len() * 24);
        assert!(body_peak(&out) > 0.5, "the fallback produced silence");
    }

    #[test]
    fn nothing_in_produces_nothing_out_rather_than_a_panic() {
        assert!(to_rate(&[], 44_100, 48_000).is_empty());
        assert_eq!(to_rate(&[0.5, 0.5], 0, 48_000), vec![0.5, 0.5]);
        assert_eq!(to_rate(&[0.5, 0.5], 44_100, 0), vec![0.5, 0.5]);
    }

    #[test]
    fn the_band_limited_path_is_cleaner_than_the_linear_one() {
        // ⛔ **The reason the dependency is here at all**, stated as a
        // measurement rather than a claim. Downsampling a tone above the new
        // Nyquist is where linear interpolation aliases: the energy folds back
        // as a spurious tone instead of being filtered out. 15 kHz into 16 kHz
        // (Nyquist 8 kHz) is well past it.
        let input = sine(15_000.0, 44_100, 0.2);
        let filtered = to_rate(&input, 44_100, 16_000);
        let aliased = linear(&input, 16_000.0 / 44_100.0);

        assert!(
            body_peak(&filtered) < body_peak(&aliased) * 0.5,
            "band-limited {} vs linear {} — the filter is not removing the alias",
            body_peak(&filtered),
            body_peak(&aliased)
        );
    }
}
