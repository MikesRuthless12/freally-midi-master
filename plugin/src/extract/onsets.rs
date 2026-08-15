//! Where the hits are, and what each one is made of (TASK-058F).
//!
//! ⛔ **This is the floor the whole audio path stands on.** Mike's drum design —
//! *"something that has a condition that when it's met like triplets being heard
//! for hihats, snares on the 4 beat … that it splits it into the proper
//! channels"* — is a set of rules about *when* hits land and *what* they contain.
//! Neither question can be asked until something has found the hits, and the
//! roadmap's own note is blunt about it: *"Autocorrelating the onset pattern
//! needs onsets, and onsets are TASK-058F."*
//!
//! ## The shape, and why it is bands rather than a spectrum
//!
//! An energy envelope per band, then the half-wave-rectified rise of each, summed.
//! That is spectral flux with four very wide bins, and four is what the classifier
//! actually consumes — a kick is "loud under 120 Hz, quiet above 2 kHz", not a
//! shape in 1,024 bins. See [`super::bands`] for why there is no FFT here.
//!
//! ⚠ **The rise of the LOG, not of the level.** A hat under a kick is 30 dB down
//! and its rise in linear energy is a rounding error next to the kick's; in log
//! energy the two are comparable, which is the whole reason a detector works on a
//! mixed loop rather than only on a solo'd one.
//!
//! ⛔ **What this cannot do, said here rather than discovered later.** A quiet hit
//! masked under a loud one on the same frame is gone — recovering it is source
//! separation. Heavily bus-compressed loops smear their transients and lose
//! onsets outright. Both are in the roadmap and both are real.

use super::bands;

/// How often the envelopes are sampled, in seconds.
///
/// ⚠ 5 ms. A 32nd note at 200 BPM is 37 ms, so this resolves the fastest thing a
/// producer writes about seven times over — and the timing error it can
/// contribute is half a hop, which is under a twentieth of the grid these onsets
/// are eventually quantised onto.
const HOP_SECONDS: f32 = 0.005;

/// The closest two hits may be and still be two hits.
///
/// ⚠ **30 ms, and it is not a musical bound.** A 64th at 200 BPM is 19 ms, so
/// this is *not* "the fastest note" — it is the length of a drum transient. A
/// kick's attack has several energy rises inside it (the click, the body, the
/// port), and without this every kick is three onsets.
const MIN_GAP_SECONDS: f32 = 0.03;

/// How much of the hit to measure the bands over.
///
/// ⚠ Short deliberately: the point is what the hit *starts* as. Thirty-five
/// milliseconds after a snare is the crack; two hundred is the crack plus
/// whatever the loop was already playing underneath.
///
/// ⚠ **And not shorter than the kick band's own rise time.** The 25 Hz smoother
/// that keeps a 55 Hz kick from rippling takes about 25 ms to reach its level, so
/// a 20 ms head measured a kick part-way up its own attack — which read as a
/// quiet hit and, worse, as one that had already decayed.
const HEAD_SECONDS: f32 = 0.035;

/// How far a hit's decay is followed before it is simply called long.
const MAX_DECAY_SECONDS: f32 = 0.5;

/// How far the dominant band must fall for the hit to have decayed.
///
/// ⚠ −12 dB. Closed hats reach it in 20–40 ms and open hats do not reach it for
/// 150 ms or more, which is the only thing that separates the two — they occupy
/// the same band by definition.
const DECAY_FALL: f32 = 0.25;

/// How far above the local median a peak must sit to be a hit.
///
/// ⚠ A *ratio over a running median*, not a fixed level. A loop that fades, or
/// that has a quiet intro bar, has no single threshold that works across it —
/// and the median is what a percussive signal's noise floor looks like between
/// hits, so this asks "is this rise unlike the last half-second" rather than "is
/// this rise loud".
const PEAK_OVER_MEDIAN: f32 = 1.6;

/// ...and the half-width of the window that median is taken over.
const MEDIAN_SECONDS: f32 = 0.25;

/// An absolute floor under the adaptive threshold.
///
/// ⛔ **Without it, silence is full of hits.** The running median of a silent
/// passage is zero, so *any* rise is infinitely more than 1.6× it — a fade-out or
/// a gap between phrases produced a flurry of onsets on nothing at all.
///
/// ⚠ **0.10, and it is measured rather than picked.** A synthesised 55 Hz kick
/// leaves a weighted-flux artefact of **0.023** in its own tail — the last of the
/// rectifier ripple, after the smoothing and the energy weighting have taken two
/// orders of magnitude off it. The same kick's actual strike measures **7.7**.
/// This sits four times above the artefact and seventy times below the hit, which
/// is also comfortably under the quietest thing worth catching: a ghost snare
/// arriving over a decaying kick weighs in around 0.35.
const MIN_FLUX: f32 = 0.10;

/// The most hits one file may yield.
///
/// A bound rather than trust, the same argument `smf_read::MAX_NOTES` makes: the
/// bytes are a file a producer picked. Four bars of 32nd-note percussion is about
/// 130 onsets; two thousand is far past anything real and far short of a problem.
const MAX_ONSETS: usize = 2_048;

/// One hit, and what it is made of.
#[derive(Debug, Clone, Copy)]
pub struct Onset {
    /// When it starts, in seconds from the top of the file.
    pub at: f32,
    /// Level under 120 Hz over the first [`HEAD_SECONDS`]. The kick band.
    pub low: f32,
    /// 150–400 Hz: the body of a snare, a clap, a tom.
    pub body: f32,
    /// 2–8 kHz: the crack of a snare and the strike of a hat.
    pub presence: f32,
    /// 8–16 kHz: hat and cymbal air, and almost nothing else.
    pub air: f32,
    /// How long the loudest of those took to fall [`DECAY_FALL`], in seconds.
    pub decay: f32,
    /// The detection function at the peak, which is what velocity is read from.
    pub strength: f32,
}

impl Onset {
    /// Everything above 2 kHz, which is what "a hat" means spectrally.
    pub fn high(&self) -> f32 {
        self.presence + self.air
    }

    /// Everything the hit is made of, across all four bands.
    fn total(&self) -> f32 {
        self.low + self.body + self.presence + self.air
    }

    /// What fraction of the hit `part` is.
    ///
    /// ⚠ **A ratio rather than a level, and one place that says so.** The
    /// alternative is a threshold in dBFS — and a loop mastered four decibels
    /// louder would then classify differently from the same loop quieter, which
    /// is not a musical fact. The three callers below were each carrying their
    /// own copy of the sum and of the divide-by-nothing guard.
    fn share(&self, part: f32) -> f32 {
        let total = self.total();
        if total <= f32::EPSILON {
            return 0.0;
        }
        part / total
    }

    /// The share of this hit that is under 150 Hz — the kick band.
    pub fn low_share(&self) -> f32 {
        self.share(self.low)
    }

    /// ...and the share above 2 kHz.
    pub fn high_share(&self) -> f32 {
        self.share(self.high())
    }

    /// ...and the share in the 150–400 Hz body.
    pub fn body_share(&self) -> f32 {
        self.share(self.body)
    }

    /// How loud the hit is, as the level of its loudest band.
    ///
    /// ⚠ **Not [`Self::strength`]**, which is a log *rise* and says how clearly
    /// the hit was detected rather than how hard it was struck — a quiet hat in
    /// silence has a huge rise and a loud one over a sustained pad has a small
    /// one. Velocity has to come from the level.
    pub fn level(&self) -> f32 {
        self.low.max(self.body).max(self.presence).max(self.air)
    }

    /// Where the energy sits, in hertz, as a weighted mean of the four bands.
    ///
    /// ⛔ **Coarse on purpose, and it is only ever used as an ORDER.** Four bands
    /// cannot tell you a tom's pitch; they can tell you that one tom is above
    /// another, which is all Mike's fill rule asks — *"percussions sounds going up
    /// and down … routed in pitch order"*.
    pub fn centroid(&self) -> f32 {
        // The geometric middle of each band, which is where a band's energy sits
        // when it is spread evenly across it in octaves.
        self.share(
            self.low * 60.0 + self.body * 245.0 + self.presence * 4_000.0 + self.air * 11_300.0,
        )
    }
}

/// The four envelopes a file's hits are read out of.
pub struct Envelopes {
    low: Vec<f32>,
    body: Vec<f32>,
    presence: Vec<f32>,
    air: Vec<f32>,
    /// Samples per envelope step.
    hop: usize,
    rate: u32,
}

impl Envelopes {
    /// Seconds per envelope step.
    fn step(&self) -> f32 {
        self.hop as f32 / self.rate as f32
    }
}

/// Filter a decoded buffer into the four bands and follow each one.
///
/// ⚠ **The smoothing corners differ per band, and they are not a preference.**
/// See [`bands::envelope`]: each has to sit well under twice its band's lowest
/// frequency or the rectifier's ripple survives, and each is simultaneously a
/// floor under every decay that band can report. 25 Hz clears a 30 Hz kick; the
/// hat bands get 200 Hz because closed-versus-open is *decided* on decay length
/// and a 25 Hz smoother would make both of them the same 25 ms.
pub fn analyse(samples: &[f32], rate: u32) -> Envelopes {
    let hop = ((rate as f32 * HOP_SECONDS).round() as usize).max(1);
    let follow = |low_hz, high_hz, smoothing| {
        bands::envelope(
            &bands::band(samples, rate, low_hz, high_hz),
            rate,
            smoothing,
            hop,
        )
    };
    // ⛔ **The low band ends where the body band begins.** It used to stop at
    // 120 Hz while the body started at 150, and the 120–150 Hz between them was in
    // no band at all — a quarter of an octave the analysis could not see. A kick
    // whose sweep spends its first twenty milliseconds there therefore read as
    // neither low nor mid, and landed on `Perc`.
    Envelopes {
        low: follow(None, Some(150.0), 25.0),
        body: follow(Some(150.0), Some(400.0), 60.0),
        presence: follow(Some(2_000.0), Some(8_000.0), 200.0),
        air: follow(Some(8_000.0), None, 200.0),
        hop,
        rate,
    }
}

/// Every hit in the file, in time order.
pub fn detect(env: &Envelopes) -> Vec<Onset> {
    let flux = flux_of(env);
    let threshold = adaptive_threshold(&flux, env.step());

    let gap = (MIN_GAP_SECONDS / env.step()).round().max(1.0) as usize;
    let mut out: Vec<Onset> = Vec::new();
    // The hit currently held, and the frame the cluster around it opened at.
    let mut last: Option<usize> = None;
    let mut anchor: Option<usize> = None;

    // ⚠ **From zero, and the neighbours are read as silence off either end.** The
    // first frame is where a loop's downbeat lives.
    let neighbour = |at: usize| flux.get(at).copied().unwrap_or(0.0);
    for at in 0..flux.len() {
        if flux[at] < threshold[at] {
            continue;
        }
        // A local maximum, so the rising side of one transient is one hit and not
        // six. `>=` on the right so a two-sample plateau still resolves.
        if flux[at] < neighbour(at.wrapping_sub(1)) || flux[at] < neighbour(at + 1) {
            continue;
        }
        // ⛔ **The louder of two hits inside the gap wins, rather than the
        // earlier.** Taking the first put every kick's pre-click on the grid and
        // dropped the kick — the click is the earlier rise and the quieter one.
        //
        // ⛔⛔ **And the window is measured from where the CLUSTER opened, not
        // from the hit currently held.** Moving the anchor onto each replacement
        // let suppression *chain*: with a 30 ms gap, hits 25 ms apart each louder
        // than the last collapsed to a single onset however long the run went on
        // — so a crescendo buzz roll, which is one of the things Mike named,
        // arrived as one hit. Anchored on the cluster, the damage is bounded to
        // one gap's width whatever comes after it.
        match anchor {
            Some(start) if at - start < gap => {
                if let Some(held) = last {
                    if flux[at] <= flux[held] {
                        continue;
                    }
                }
                out.pop();
            }
            _ => anchor = Some(at),
        }

        out.push(measure(env, at, flux[at]));
        last = Some(at);
        if out.len() >= MAX_ONSETS {
            break;
        }
    }

    out
}

/// The half-wave-rectified rise of the four log envelopes, weighted by where the
/// energy actually is.
///
/// ⛔⛔ **The weighting is not a refinement — without it a decaying kick is
/// sixteen hits.** A 55 Hz kick leaks into the 150–400 Hz band about 21 dB down,
/// and *there* its rectifier ripple survives the smoother at around nine per
/// cent. Nine per cent of a band nobody can hear is a log-rise of 0.09, which is
/// indistinguishable from a genuine quiet hit if all four bands are added up
/// blind. Scaled by that band's share of the energy at that instant — three per
/// cent — it becomes 0.003 and stops existing.
///
/// ▶ **And a hat over a kick still counts**, which is the case a plain absolute
/// threshold would lose: the hat owns most of the high band at the moment it is
/// struck, so its share is large even though the kick is louder overall.
fn flux_of(env: &Envelopes) -> Vec<f32> {
    let len = env
        .low
        .len()
        .min(env.body.len())
        .min(env.presence.len())
        .min(env.air.len());
    // ⚠ Small enough to be well below any real signal and large enough that
    // `ln` of a silent block is a number rather than an infinity.
    const FLOOR: f32 = 1.0e-6;

    (0..len)
        .map(|at| {
            let all = [&env.low, &env.body, &env.presence, &env.air];
            let total: f32 = all.iter().map(|band| band[at]).sum::<f32>() + FLOOR;
            all.iter()
                .map(|band| {
                    let now = (band[at] + FLOOR).ln();
                    // ⛔⛔ **Before the first frame is SILENCE, not zero flux.** A
                    // sample-pack loop starts with a hit on beat one, and a hat
                    // struck at t=0 has already fully risen inside the first five
                    // millisecond block — so comparing frame 0 against "nothing to
                    // compare it to" dropped the downbeat of every loop. The file
                    // was not playing before it started, and that is a measurement
                    // rather than a convention.
                    // ⚠ Self-limiting on a fade-in, which is the case this could
                    // have invented a hit for: there the first frame is barely
                    // above the floor and the rise is small.
                    let before = if at == 0 {
                        FLOOR.ln()
                    } else {
                        (band[at - 1] + FLOOR).ln()
                    };
                    (now - before).max(0.0) * (band[at] / total)
                })
                .sum()
        })
        .collect()
}

/// [`PEAK_OVER_MEDIAN`] times the local median, never below [`MIN_FLUX`].
///
/// ⚠ **`select_nth_unstable_by`, not a sort.** Only the middle element is wanted,
/// and there are twelve thousand of these windows on a sixty-second file — a full
/// stable sort of 101 floats is ~670 comparisons *and a heap allocation for
/// std's merge scratch*, twelve thousand times over. Selection is linear and
/// allocates nothing.
fn adaptive_threshold(flux: &[f32], step: f32) -> Vec<f32> {
    let half = (MEDIAN_SECONDS / step).round().max(1.0) as usize;
    let mut window: Vec<f32> = Vec::with_capacity(half * 2 + 1);
    (0..flux.len())
        .map(|at| {
            window.clear();
            window
                .extend_from_slice(&flux[at.saturating_sub(half)..(at + half + 1).min(flux.len())]);
            let middle = window.len() / 2;
            let (_, median, _) = window.select_nth_unstable_by(middle, f32::total_cmp);
            (*median * PEAK_OVER_MEDIAN).max(MIN_FLUX)
        })
        .collect()
}

/// What the hit at envelope step `at` is made of.
fn measure(env: &Envelopes, at: usize, strength: f32) -> Onset {
    let head = (HEAD_SECONDS / env.step()).round().max(1.0) as usize;
    let peak_of = |band: &[f32]| {
        band[at..(at + head).min(band.len())]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max)
    };

    let low = peak_of(&env.low);
    let body = peak_of(&env.body);
    let presence = peak_of(&env.presence);
    let air = peak_of(&env.air);

    // ⚠ The decay is followed in whichever band the hit is loudest in. Following
    // a fixed one would time a hat's decay in the kick band, where it never was.
    let loudest: &[f32] = if low >= body && low >= presence && low >= air {
        &env.low
    } else if body >= presence && body >= air {
        &env.body
    } else if presence >= air {
        &env.presence
    } else {
        &env.air
    };

    Onset {
        at: at as f32 * env.step(),
        low,
        body,
        presence,
        air,
        decay: decay_of(loudest, at, env.step()),
        strength,
    }
}

/// How long `band` takes to fall [`DECAY_FALL`] from its head level.
///
/// ⛔ **Measured from the PEAK, not from the onset.** The envelope is smoothed,
/// so a hit takes a few milliseconds to reach its level — and searching from the
/// onset frame found the very first sample already "below a quarter of the peak",
/// because the peak had not happened yet. Every hit in the file reported a decay
/// of zero, which would have made every hat a closed one.
fn decay_of(band: &[f32], at: usize, step: f32) -> f32 {
    let head = (HEAD_SECONDS / step).round().max(1.0) as usize;
    let until_head = (at + head).min(band.len());
    let Some((top, peak)) = band[at..until_head]
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(offset, peak)| (at + offset, peak))
    else {
        return 0.0;
    };
    if peak <= f32::EPSILON {
        return 0.0;
    }
    let limit = (MAX_DECAY_SECONDS / step).round() as usize;
    let until = (top + limit).min(band.len());
    for (steps, level) in band[top..until].iter().enumerate() {
        if *level < peak * DECAY_FALL {
            return steps as f32 * step;
        }
    }
    // ⚠ Reported as the cap rather than as the true length. Past half a second
    // the question the classifier asks — "is this a closed hat or an open one" —
    // has already been answered, and following a ride's tail to its end would
    // make every crash in the file a different number for no decision.
    MAX_DECAY_SECONDS
}

#[cfg(test)]
pub(super) mod fixtures {
    /// How a struck sound is faded out at the end of its own window.
    ///
    /// ⛔⛔ **A fixture that just stops is a fixture with a click in it**, and the
    /// detector is right to find that click. A 55 Hz tone whose exponential decay
    /// has only reached 0.058 when the loop writing it runs out of samples ends on
    /// a step — broadband, instantaneous, and a perfectly real onset a quarter of
    /// a second after the kick. Two tests were "failing" on a hit their own
    /// fixture had put there.
    fn shape(i: usize, n: usize, seconds: f32, decay_share: f32) -> f32 {
        let t = i as f32 / n as f32 * seconds;
        let decay = (-t / (seconds * decay_share)).exp();
        // A linear ramp over the last tenth, which is enough to take a 6% step
        // down to nothing over several hundred samples.
        let tail = n / 10;
        let fade = if i + tail >= n {
            (n - i) as f32 / tail.max(1) as f32
        } else {
            1.0
        };
        decay * fade
    }

    /// A decaying sine — a kick, a tom, or an 808 depending on the arguments.
    pub fn tone(out: &mut [f32], at: usize, rate: u32, hz: f32, seconds: f32, level: f32) {
        let n = (rate as f32 * seconds) as usize;
        for i in 0..n {
            let Some(slot) = out.get_mut(at + i) else {
                return;
            };
            let t = i as f32 / rate as f32;
            *slot +=
                level * shape(i, n, seconds, 0.35) * (2.0 * std::f32::consts::PI * hz * t).sin();
        }
    }

    /// A kick: a fast downward pitch sweep under a click.
    ///
    /// ⛔ **Not [`tone`], and the difference is the point.** A decaying sine at
    /// 55 Hz *is* a bass note — it is what an 808 is — so a fixture built from one
    /// cannot test the claim that a kick does not become a bassline. A real kick
    /// drops an octave and a half in its first forty milliseconds and is over in
    /// sixty, which is why no window of it holds a pitch.
    pub fn kick(out: &mut [f32], at: usize, rate: u32, seconds: f32, level: f32) {
        let n = (rate as f32 * seconds) as usize;
        let mut phase = 0.0_f32;
        for i in 0..n {
            let Some(slot) = out.get_mut(at + i) else {
                return;
            };
            let t = i as f32 / rate as f32;
            // 110 Hz down to 48 in about ten milliseconds, which is what an 808
            // kick's pitch envelope does and is why it is a kick rather than a
            // tom.
            let hz = 48.0 + 62.0 * (-t / 0.010).exp();
            phase += 2.0 * std::f32::consts::PI * hz / rate as f32;
            *slot += level * shape(i, n, seconds, 0.4) * phase.sin();
        }
    }

    /// Deterministic pseudo-noise, band-shaped by the caller.
    ///
    /// ⚠ No clock and no `rand`: a fixture that flakes on the machine that runs
    /// it is worse than no fixture.
    pub fn noise(out: &mut [f32], at: usize, rate: u32, seconds: f32, level: f32, seed: u32) {
        let n = (rate as f32 * seconds) as usize;
        let mut state = seed | 1;
        for i in 0..n {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let Some(slot) = out.get_mut(at + i) else {
                return;
            };
            // ⚠ Scaled by the width of the bits actually taken. Dividing a
            // 23-bit value by `u16::MAX` made "noise at level 0.6" a signal that
            // peaked near 64, which then reached a log and a threshold.
            let value = (state >> 9) as f32 / f32::from(u16::MAX) / 128.0 - 0.5;
            *slot += level * shape(i, n, seconds, 0.3) * value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{noise, tone};
    use super::*;

    const RATE: u32 = 48_000;

    #[test]
    fn four_kicks_on_the_beat_are_four_onsets() {
        let mut buffer = vec![0.0_f32; RATE as usize * 2];
        for beat in 0..4 {
            tone(&mut buffer, beat * RATE as usize / 2, RATE, 55.0, 0.2, 0.9);
        }
        let found = detect(&analyse(&buffer, RATE));
        assert_eq!(found.len(), 4, "{found:#?}");
        for (beat, onset) in found.iter().enumerate() {
            let expected = beat as f32 * 0.5;
            assert!(
                (onset.at - expected).abs() < 0.02,
                "hit {beat} landed at {} rather than {expected}",
                onset.at
            );
        }
    }

    #[test]
    fn a_kick_reads_low_and_a_hat_reads_high() {
        // ⛔ The one measurement everything downstream is built on. If the bands
        // do not separate these two, no rule above them can.
        let mut buffer = vec![0.0_f32; RATE as usize];
        tone(&mut buffer, 0, RATE, 55.0, 0.25, 0.9);
        // Shaped into the hat band, so the fixture is a hat rather than a click.
        let mut hat = vec![0.0_f32; RATE as usize];
        noise(&mut hat, RATE as usize / 2, RATE, 0.04, 0.6, 0x51DE);
        let hat = bands::band(&hat, RATE, Some(4_000.0), None);
        for (at, sample) in hat.iter().enumerate() {
            buffer[at] += sample;
        }

        let found = detect(&analyse(&buffer, RATE));
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(
            found[0].low_share() > 0.8,
            "the kick read {} of its energy low",
            found[0].low_share()
        );
        assert!(
            found[1].high_share() > 0.8,
            "the hat read {} of its energy high",
            found[1].high_share()
        );
    }

    #[test]
    fn an_open_hat_decays_longer_than_a_closed_one() {
        // ⛔ **The only thing that separates them**, because they are the same
        // band by definition — see `DECAY_FALL`.
        let mut buffer = vec![0.0_f32; RATE as usize];
        let mut hits = vec![0.0_f32; RATE as usize];
        noise(&mut hits, 0, RATE, 0.03, 0.6, 0x0C10);
        noise(&mut hits, RATE as usize / 2, RATE, 0.35, 0.6, 0x0_9E_11);
        let shaped = bands::band(&hits, RATE, Some(4_000.0), None);
        for (at, sample) in shaped.iter().enumerate() {
            buffer[at] += sample;
        }

        let found = detect(&analyse(&buffer, RATE));
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(
            found[1].decay > found[0].decay * 2.0,
            "closed {} vs open {}",
            found[0].decay,
            found[1].decay
        );
    }

    #[test]
    fn silence_is_not_full_of_hits() {
        // ⛔ The running median of silence is zero, so without `MIN_FLUX` every
        // rounding error in a fade-out is infinitely more than 1.6× it.
        assert!(detect(&analyse(&vec![0.0_f32; RATE as usize], RATE)).is_empty());
    }

    #[test]
    fn one_transient_is_one_hit_rather_than_the_three_rises_inside_it() {
        // ⚠ A kick's click, body and port are three separate energy rises within
        // 30 ms of each other. See `MIN_GAP_SECONDS`.
        let mut buffer = vec![0.0_f32; RATE as usize];
        tone(&mut buffer, 0, RATE, 55.0, 0.3, 0.9);
        tone(&mut buffer, RATE as usize / 400, RATE, 180.0, 0.05, 0.4);
        tone(&mut buffer, RATE as usize / 200, RATE, 3_000.0, 0.01, 0.3);
        let found = detect(&analyse(&buffer, RATE));
        assert_eq!(found.len(), 1, "{found:#?}");
    }
}
