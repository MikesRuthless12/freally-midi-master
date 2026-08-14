//! What note a sample is in (TASK-052).
//!
//! A producer drops a bass one-shot on the Bass lane and the sampler has to know
//! what pitch it already *is*, or transposing it by the interval the generator
//! moves puts it in the wrong key. `Kit::semitones_for` reads `pad.root_note`;
//! this is what can fill it in without asking.
//!
//! ⛔ **A guess with a confidence, never a silent assumption.** A wrong root is
//! worse than none: none plays the sample as it was sampled, which is at least
//! the sound the producer chose, while a wrong one transposes every note away
//! from the key. So this hands back the confidence with the note and the UI is
//! expected to show it — the roadmap's word is *"surfaced"* — and an editable
//! root beside it.
//!
//! ## Why NSDF rather than "autocorrelation + HPS"
//!
//! The roadmap says autocorrelation plus a harmonic product spectrum. HPS needs
//! an FFT, and an FFT here means a new dependency in a crate that is loaded into
//! somebody else's DAW and audited by `deny.toml`. **The normalised square
//! difference function** (McLeod) is the standard answer to the same problem
//! without one: it is autocorrelation divided by the energy in both windows, so
//! its peaks sit in `-1..=1` and their *height is the confidence* rather than a
//! number that scales with how loud the sample happens to be. It is also
//! markedly better than bare autocorrelation at the failure this has to avoid,
//! which is answering an octave down.
//!
//! ⚠ It is still a monophonic detector. A chord stab has no single root and this
//! will report the loudest partial with a low confidence, which is the honest
//! outcome — the caller shows the number and lets the producer decide.

use engine::pattern::Lane;

/// The lowest and highest fundamental worth looking for, in hertz.
///
/// ⛔ **Bounded to what a one-shot can be**, which is also what bounds the work:
/// the search is one lag per candidate period, so the range *is* the cost. 27.5
/// Hz is A0 — below the lowest 808 anybody tunes to — and 2 kHz is above the top
/// of a bell or a pluck. Outside that band a "fundamental" is a artefact.
const MIN_HZ: f32 = 27.5;
const MAX_HZ: f32 = 2_000.0;

/// How much of the sample to look at, in seconds.
///
/// ⚠ **Skipping the attack matters more than the length.** A kick's first 30 ms
/// is a click with no periodicity at all, and including it drags every
/// correlation down. Half a second of the body is plenty for a fundamental.
const SKIP_SECONDS: f32 = 0.03;
const WINDOW_SECONDS: f32 = 0.5;

/// A peak this low is noise wearing a period.
///
/// Chosen at 0.6 because that is where NSDF's own literature puts the knee for
/// "clearly periodic", and because a wrong root is worse than none: below this
/// the answer is `None` and the pad plays as sampled.
const MIN_CLARITY: f32 = 0.6;

/// What a sample is tuned to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Root {
    /// MIDI note number.
    pub note: u8,
    /// How far off that note it actually is, in cents. `-50..=50`.
    ///
    /// Surfaced rather than rounded away: a sample 40 cents flat is a sample a
    /// producer wants to know about, and "it is a C" would be a readout that
    /// hides the reason their bass sounds out of tune.
    pub cents: i8,
    /// NSDF peak height, `0..=1`. Higher is more clearly one pitch.
    pub clarity: f32,
}

/// Detect the fundamental of a mono buffer.
///
/// `None` when nothing periodic enough was found — silence, noise, a chord, or a
/// drum. That is a real answer and the common one for half a drum kit.
pub fn detect_root(samples: &[f32], sample_rate: u32) -> Option<Root> {
    if sample_rate == 0 {
        return None;
    }
    let rate = sample_rate as f32;

    let skip = (SKIP_SECONDS * rate) as usize;
    let take = (WINDOW_SECONDS * rate) as usize;
    let window: &[f32] = samples
        .get(skip..)?
        .get(..take)
        .unwrap_or(samples.get(skip..)?);

    detect_in(window, sample_rate, MIN_HZ, MAX_HZ)
}

/// The fundamental of an exact window, searched only between two frequencies.
///
/// ⛔⛔ **One detector, two callers — and the second caller is why this is split
/// out at all.** [`detect_root`] answers *"what note is this one-shot"*: it skips
/// the attack, takes half a second, and looks across the whole audible range,
/// because a pad may hold anything. `crate::extract::pitched` asks a different
/// question of the same machinery — *"what note is this 40 ms of the bassline"* —
/// and it already knows the band, because it filtered the audio into one.
///
/// ▶ **Writing a second NSDF for it was the alternative and would have been the
/// mistake this repo keeps recording.** `engine::smf_read`'s own header states
/// the rule for the MIDI side — *one measurement path, not two* — and a second
/// pitch detector would drift from this one exactly the way a second extension
/// list did before `engine::formats` took it away.
///
/// ⚠ **Narrowing the band is also what makes a pitch *track* affordable.** The
/// cost is `window · lags` and `lags` is `rate / min_hz`; asking for 27.5 Hz when
/// the signal was low-passed at 250 is paying for six octaves nobody filtered
/// into the buffer.
pub fn detect_in(window: &[f32], sample_rate: u32, min_hz: f32, max_hz: f32) -> Option<Root> {
    if sample_rate == 0 || !min_hz.is_finite() || min_hz <= 0.0 || max_hz <= min_hz {
        return None;
    }
    let rate = sample_rate as f32;

    let min_lag = (rate / max_hz).floor().max(2.0) as usize;
    let max_lag = (rate / min_hz).ceil() as usize;
    if window.len() < min_lag * 2 + 2 {
        return None;
    }
    let max_lag = max_lag.min(window.len() / 2);
    if max_lag <= min_lag {
        return None;
    }

    // The normalised square difference function, evaluated per lag.
    //
    // ⚠ Written as the plain O(n·lags) double loop rather than through an FFT,
    // deliberately: this runs once per imported file on the loader thread, never
    // on the audio thread, and half a second at 48 kHz over ~1,700 lags is a few
    // milliseconds. An FFT would be faster and would cost a dependency.
    let nsdf: Vec<f32> = (0..=max_lag)
        .map(|lag| {
            let mut correlation = 0.0_f64;
            let mut energy = 0.0_f64;
            for i in 0..window.len() - lag {
                let a = f64::from(window[i]);
                let b = f64::from(window[i + lag]);
                correlation += a * b;
                energy += a * a + b * b;
            }
            if energy > 0.0 {
                (2.0 * correlation / energy) as f32
            } else {
                0.0
            }
        })
        .collect();

    // ⛔⛔ **Peaks are only looked for after the first negative lag, and this is
    // the whole detector.** NSDF starts at 1.0 at lag 0 and comes down; a low
    // note is still strongly self-similar a few samples along — a 55 Hz sine
    // correlates at 0.98 with itself at lag 24 — so any search that starts at
    // the shortest allowed lag finds a "peak" in that first descent and reports
    // a note five octaves up. McLeod's rule is to wait for the function to cross
    // zero, which is the point past which similarity means periodicity.
    let after = nsdf.iter().position(|value| *value < 0.0)?;

    // Every local maximum from there on, then McLeod's choice: the **first**
    // peak within 90% of the tallest, rather than the tallest itself.
    //
    // ⛔ The peaks repeat at every multiple of the true period and a later one
    // is often marginally taller — taking the tallest is exactly how a detector
    // answers an octave down, which for a bass one-shot means every note comes
    // out an octave high.
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    for lag in (after.max(min_lag) + 1)..max_lag {
        if nsdf[lag] > nsdf[lag - 1] && nsdf[lag] >= nsdf[lag + 1] {
            peaks.push((lag, nsdf[lag]));
        }
    }

    let tallest = peaks
        .iter()
        .map(|(_, value)| *value)
        .fold(f32::NEG_INFINITY, f32::max);
    if !tallest.is_finite() || tallest < MIN_CLARITY {
        return None;
    }

    let (lag, clarity) = peaks
        .into_iter()
        .find(|(_, value)| *value >= tallest * 0.9)?;

    // ⛔⛔ **The peak between two lags, not the lag it sat on.** McLeod's own
    // paper specifies this and the first cut of this detector skipped it, which
    // was invisible while the only caller fed it 48 kHz — there a bass note is a
    // lag of a thousand samples and one sample of error is a fifth of a cent.
    // ▶ `crate::extract::pitched` runs the same NSDF over a *decimated* band,
    // where the same note is a lag of thirty and one sample of error is **half a
    // semitone**: a bassline came back as 37-40-38 where 36-41-39 was played.
    // A parabola through the peak and its two neighbours recovers the fraction.
    let lag = {
        let (before, here, after) = (nsdf[lag - 1], nsdf[lag], nsdf[lag + 1]);
        let curve = before - 2.0 * here + after;
        // ⚠ Guarded: a flat top gives a zero denominator, and the vertex of a
        // straight line is not a number. The bare lag is the right answer there.
        if curve.abs() > f32::EPSILON {
            lag as f32 + 0.5 * (before - after) / curve
        } else {
            lag as f32
        }
    };
    let hz = rate / lag;
    if !(min_hz..=max_hz).contains(&hz) {
        return None;
    }

    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let note = midi.round();
    if !(0.0..=127.0).contains(&note) {
        return None;
    }

    Some(Root {
        note: note as u8,
        cents: ((midi - note) * 100.0).round().clamp(-50.0, 50.0) as i8,
        clarity: clarity.clamp(0.0, 1.0),
    })
}

/// Whether a detected root should be applied to this lane at all.
///
/// ⛔ **Only the pitched lanes.** `Kit::semitones_for` treats `root_note` as "the
/// note this sample already is", and setting one on a kick makes the drum grid's
/// hand-drawn hits transpose it. A snare has a pitch; it does not have a *root*,
/// and the difference is whether anything should ever move it.
pub fn applies_to(lane: Lane) -> bool {
    matches!(
        lane,
        Lane::Melody | Lane::Counter | Lane::Bass | Lane::Chords | Lane::Sub
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of a sine at `hz`, plus a click at the front so the skip is
    /// doing something.
    fn sine(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        let mut out: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect();
        for sample in out.iter_mut().take(rate as usize / 100) {
            *sample = 1.0;
        }
        out
    }

    #[test]
    fn a_sine_is_detected_as_the_note_it_is() {
        // A440 is MIDI 69 by definition, which makes it the one case with no
        // arithmetic to argue about.
        let root = detect_root(&sine(440.0, 48_000, 1.0), 48_000).expect("a sine has a pitch");
        assert_eq!(root.note, 69);
        assert!(root.cents.abs() <= 5, "{} cents off", root.cents);
        assert!(root.clarity > 0.9, "clarity was {}", root.clarity);
    }

    #[test]
    fn the_octave_below_is_not_the_answer() {
        // ⛔⛔ **The failure mode this detector exists to avoid.** A correlation
        // repeats at every multiple of the true period, so taking the tallest
        // peak answers an octave — or two — down, and a bass one-shot detected
        // an octave low is transposed an octave high on every note.
        for (hz, midi) in [(55.0, 33), (110.0, 45), (220.0, 57), (880.0, 81)] {
            let root = detect_root(&sine(hz, 48_000, 1.0), 48_000)
                .unwrap_or_else(|| panic!("{hz} Hz must be detected"));
            assert_eq!(root.note, midi, "{hz} Hz came back as MIDI {}", root.note);
        }
    }

    #[test]
    fn a_note_between_two_is_reported_with_the_cents_rather_than_rounded_away() {
        // 40 cents sharp of A440. A producer whose bass sits here wants to be
        // told, not to read "it is an A".
        let sharp = 440.0 * 2f32.powf(0.40 / 12.0);
        let root = detect_root(&sine(sharp, 48_000, 1.0), 48_000).expect("still a pitch");
        assert_eq!(root.note, 69);
        assert!(root.cents >= 30, "{} cents", root.cents);
    }

    #[test]
    fn noise_and_silence_are_answered_with_nothing_rather_than_a_guess() {
        // ⛔ A wrong root is worse than none: none plays the sample as it was
        // sampled, which is at least the sound the producer chose.
        assert_eq!(detect_root(&[0.0; 48_000], 48_000), None);

        // Deterministic pseudo-noise — no clock, no rand, so the test cannot
        // flake on the machine that runs it.
        let mut state = 0x_D1CE_u32;
        let noise: Vec<f32> = (0..48_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / f32::from(u16::MAX) % 2.0 - 1.0
            })
            .collect();
        let found = detect_root(&noise, 48_000);
        assert!(
            found.is_none_or(|root| root.clarity < 0.9),
            "noise came back as a confident pitch: {found:?}"
        );
    }

    #[test]
    fn a_buffer_too_short_to_hold_a_period_is_refused_rather_than_read_past() {
        assert_eq!(detect_root(&[0.1, 0.2, 0.3], 48_000), None);
        assert_eq!(detect_root(&sine(440.0, 48_000, 1.0), 0), None);
    }

    #[test]
    fn only_the_pitched_lanes_take_a_root() {
        // ⛔ Setting one on a kick makes the drum grid's hand-drawn hits
        // transpose it — a snare has a pitch, it does not have a root.
        assert!(applies_to(Lane::Bass));
        assert!(applies_to(Lane::Melody));
        assert!(applies_to(Lane::Sub));
        assert!(!applies_to(Lane::Kick));
        assert!(!applies_to(Lane::Snare));
        assert!(!applies_to(Lane::ClosedHat));
    }
}
