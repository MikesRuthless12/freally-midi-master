//! Finding the part of a sample that can be held (TASK-053A).
//!
//! ⛔⛔ **The defect this exists to fix: a held note that stops dead.** Every
//! triggered one-shot runs to the end of its file whatever the note says, so a
//! whole-note chord under a four-bar loop sounds for however long the sample
//! happens to be — 1.2 seconds, say — and then nothing. The piano roll's note
//! lengths do nothing a producer can hear on a sustaining part.
//!
//! ▶ **The fix is to loop the sample's steady state for as long as the note is
//! held**, which is what every sampler has done since hardware ones, and the
//! whole difficulty is picking the two points so the splice is inaudible.
//!
//! ## The rules, and why each one is here
//!
//! - **The material decides, not a mode switch.** TASK-053A describes three
//!   behaviours — one-shot, gated and sustaining — and defaults the choice from
//!   the role guess. But the distinction between *gated* and *sustaining* is
//!   really a question about the audio: a pluck or a stab has no steady state
//!   to hold, and a pad or a bowed string does. So a sample that has one gets a
//!   loop and a sample that does not plays to its end and releases, which is
//!   exactly the two behaviours the task asks for — with no flag to get wrong,
//!   and no way for a producer to set "sustaining" on a stab and hear a
//!   stuttering artefact. The task's own fallback clause says this outright:
//!   *"Where no usable loop region exists … the voice plays to the end and
//!   stops."*
//! - **Which LANES this is asked of is a different question, and it is
//!   `roles::is_melodic`.** A note can only be held on a lane whose notes are
//!   gated by their own length; `sampler::hold_for` is where that is decided
//!   and `is_melodic` is the list it reads. ⚠ **That excludes `Lane::Sub`** —
//!   an 808 rings past its note so its slide can arrive — so despite being the
//!   most sustained-sounding thing in a kit, an 808 is never looped and is
//!   never asked about. `oneshot::load` has the full note.
//! - **Refuse rather than approximate.** Every threshold below is set so that
//!   an ordinary decaying one-shot finds nothing. A loop point in the wrong
//!   place is a click on every held note, which is worse than the shortened
//!   note it replaces — the task says so: *"no click at the loop point, which
//!   is the failure that makes a sustain loop worse than no sustain loop"*.
//! - **Never on the audio thread.** This walks the whole buffer. It runs once,
//!   on the loader thread, beside the decode and the pitch detection.

/// The shortest region worth looping, in seconds.
///
/// ⛔ **A short loop is a buzz, not a sustain.** Looping 5 ms of a sample
/// repeats it 200 times a second, which is an audible tone at 200 Hz laid over
/// whatever the sample was.
///
/// ⛔⛔ **And it is the number that refuses a decaying one-shot, which the
/// first cut of this got wrong.** At 50 ms a decay is only two or three
/// windows long, and *any* curve looks flat over three windows — so an
/// ordinary decaying tone found a "steady state" and would have been looped.
/// Across 200 ms a decay audible as a decay has lost far more than
/// [`STEADY_TOLERANCE`], so it is refused by arithmetic rather than by luck.
/// A genuinely sustained sample — a pad, a bowed string, an organ — has
/// hundreds of milliseconds of steady state and is unaffected.
const MIN_LOOP_SECONDS: f32 = 0.2;

/// How much of the front of a sample is never part of the steady state.
///
/// The attack is the one part of a sample that is by definition not steady, and
/// including it would put the loop's start inside the transient — so every
/// repeat would re-attack, which is the stutter this is written to avoid.
const SKIP_ATTACK_SECONDS: f32 = 0.05;

/// The window the steady state is measured in, in seconds.
///
/// ⚠ Long enough to average over a cycle of anything above about 100 Hz, short
/// enough that a region of a few hundred milliseconds is several windows.
const WINDOW_SECONDS: f32 = 0.02;

/// How much a window's level may differ from the region's and still count as
/// steady.
///
/// ⛔ **The number that decides whether a decaying one-shot is refused**, which
/// is the safety of the whole feature: every pad in every shipped kit is a
/// synthesized tone with a decay envelope, and finding a "steady state" in one
/// would start looping sounds this product already ships. A decay audible as a
/// decay loses far more than a fifth of its level across the region lengths
/// below.
const STEADY_TOLERANCE: f32 = 0.2;

/// The level a region must reach to be worth holding, relative to the sample's
/// own peak.
///
/// A near-silent tail is extremely steady and holding it is holding silence.
const MIN_LEVEL: f32 = 0.1;

/// Where a sample can be looped while a note is held, or `None`.
///
/// `None` is the common answer and a real one: percussion, plucks, stabs, vocal
/// chops and every decaying synth tone have no steady state, and the caller
/// plays them to the end instead.
///
/// The returned range is `start..end` into `samples`, both snapped to
/// **rising** zero crossings so that splicing `end` back to `start` is
/// continuous in both value and direction — a match in value alone can still
/// reverse the slope, which is a corner rather than a step but is audible on a
/// held low note.
pub fn find(samples: &[f32], sample_rate: u32) -> Option<(usize, usize)> {
    if sample_rate == 0 {
        return None;
    }
    let rate = sample_rate as f32;
    let window = (WINDOW_SECONDS * rate) as usize;
    let min_len = (MIN_LOOP_SECONDS * rate) as usize;
    let skip = (SKIP_ATTACK_SECONDS * rate) as usize;
    if window == 0 || samples.len() <= skip + min_len {
        return None;
    }

    let peak = samples.iter().fold(0.0f32, |top, s| top.max(s.abs()));
    if peak <= 0.0 {
        return None;
    }
    let floor = peak * MIN_LEVEL;

    // One RMS per window, from the end of the attack onward.
    let levels: Vec<f32> = samples[skip..]
        .chunks(window)
        .map(|chunk| {
            let sum: f32 = chunk.iter().map(|s| s * s).sum();
            (sum / chunk.len() as f32).sqrt()
        })
        .collect();

    // The longest run of windows that all sit within `STEADY_TOLERANCE` of the
    // run's own first window and above the floor.
    //
    // ⚠ **Compared against the run's FIRST window rather than a running mean.**
    // A slow decay drifts, and a mean that drifts with it would call the whole
    // decay steady — which is the one answer this must never give.
    let (mut best_at, mut best_len) = (0usize, 0usize);
    let (mut run_at, mut run_len) = (0usize, 0usize);
    for (at, &level) in levels.iter().enumerate() {
        let steady = run_len > 0 && {
            let first = levels[run_at];
            first > 0.0 && (level - first).abs() / first <= STEADY_TOLERANCE
        };
        if level >= floor && steady {
            run_len += 1;
        } else {
            run_at = at;
            run_len = if level >= floor { 1 } else { 0 };
        }
        if run_len > best_len {
            best_at = run_at;
            best_len = run_len;
        }
    }

    let start = skip + best_at * window;
    let end = (start + best_len * window).min(samples.len());
    if end.saturating_sub(start) < min_len {
        return None;
    }

    // ⛔ Snapped **inward**, so neither point can leave the region that was
    // measured. Both are rising crossings, so the splice keeps the slope.
    let start = rising_at_or_after(samples, start, end)?;
    let end = rising_at_or_before(samples, end, start)?;
    if end.saturating_sub(start) < min_len {
        return None;
    }
    Some((start, end))
}

/// The first rising zero crossing at or after `from`, before `limit`.
fn rising_at_or_after(samples: &[f32], from: usize, limit: usize) -> Option<usize> {
    (from.max(1)..limit).find(|&at| samples[at - 1] <= 0.0 && samples[at] > 0.0)
}

/// The last rising zero crossing at or before `from`, after `floor_at`.
fn rising_at_or_before(samples: &[f32], from: usize, floor_at: usize) -> Option<usize> {
    (floor_at.max(1)..=from.min(samples.len() - 1))
        .rev()
        .find(|&at| samples[at - 1] <= 0.0 && samples[at] > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f32, seconds: f32) -> Vec<f32> {
        let n = (RATE as f32 * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// A sine under an exponential decay — what every shipped pitched pad is.
    fn decaying(hz: f32, seconds: f32) -> Vec<f32> {
        let n = (RATE as f32 * seconds) as usize;
        sine(hz, seconds)
            .iter()
            .enumerate()
            .map(|(i, s)| s * (-6.0 * i as f32 / n as f32).exp())
            .collect()
    }

    #[test]
    fn a_held_tone_has_a_region_to_loop() {
        let (start, end) = find(&sine(220.0, 1.0), RATE).expect("a steady tone can be held");
        assert!(end > start);
        assert!(
            end - start >= (MIN_LOOP_SECONDS * RATE as f32) as usize,
            "the region must be long enough not to buzz"
        );
    }

    #[test]
    fn a_decaying_one_shot_is_refused_rather_than_looped() {
        // ⛔⛔ **The safety of the whole feature.** Every pitched pad in every
        // shipped kit is a synthesized tone under a decay envelope. Finding a
        // "steady state" in one would start looping sounds this product already
        // ships, unheard, inside a release.
        assert_eq!(find(&decaying(440.0, 1.5), RATE), None);
    }

    #[test]
    fn silence_and_a_stub_are_refused() {
        assert_eq!(find(&[0.0; 48_000], RATE), None);
        assert_eq!(find(&sine(220.0, 0.02), RATE), None);
        assert_eq!(find(&[], RATE), None);
        assert_eq!(find(&sine(220.0, 1.0), 0), None);
    }

    #[test]
    fn both_ends_are_rising_zero_crossings_so_the_splice_keeps_its_slope() {
        // ⛔ A match in value alone can still reverse the slope, which is a
        // corner rather than a step — inaudible on a hat and very audible held
        // under a low chord.
        let samples = sine(220.0, 1.0);
        let (start, end) = find(&samples, RATE).expect("a steady tone can be held");

        for at in [start, end] {
            assert!(at >= 1, "a crossing needs a sample before it");
            assert!(samples[at - 1] <= 0.0 && samples[at] > 0.0, "at {at}");
        }
    }

    #[test]
    fn the_loop_never_reaches_back_into_the_attack() {
        // Looping the transient would re-attack on every repeat, which is the
        // stutter this exists to avoid.
        let samples = sine(220.0, 1.0);
        let (start, _) = find(&samples, RATE).expect("a steady tone can be held");
        assert!(start >= (SKIP_ATTACK_SECONDS * RATE as f32) as usize);
    }

    #[test]
    fn a_near_silent_tail_is_not_mistaken_for_a_sustain() {
        // ⛔ **A tail is the steadiest thing in any sample, and holding it is
        // holding silence.** The loud head is deliberately long enough to be
        // loopable on its own and the tail is four times longer, so the *longest
        // steady run* is the tail — which is exactly the trap: an unguarded
        // longest-run scan picks it.
        //
        // ⚠ The first cut of this test made the head 0.2 s, which is shorter
        // than `MIN_LOOP_SECONDS` once the attack skip is taken off — so `find`
        // answered `None` for a reason that had nothing to do with the tail and
        // the assertion never ran at all.
        let mut samples = sine(220.0, 0.8);
        samples.extend(sine(220.0, 3.0).iter().map(|s| s * 0.01));

        let (start, end) = find(&samples, RATE).expect("the loud head is loopable");
        let peak = samples.iter().fold(0.0f32, |top, s| top.max(s.abs()));
        let level = samples[start..end]
            .iter()
            .fold(0.0f32, |top, s| top.max(s.abs()));

        assert!(level >= peak * MIN_LEVEL, "it held the silent tail");
        assert!(
            end <= (0.8 * RATE as f32) as usize,
            "the region reached past the loud head into the tail: {start}..{end}"
        );
    }
}
