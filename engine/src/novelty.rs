//! The novelty guard (FR-011).
//!
//! A generated melody is screened against a bundled table of well-known hook
//! fragments, and a take that matches is thrown away and drawn again. The point
//! is not plagiarism detection — it is that a procedural generator working from
//! a narrow interval grammar will eventually write a line somebody already
//! owns, and a producer should not find that out from a label.
//!
//! ⛔ **The table holds hashes and nothing else.** A reference melody enters it
//! as a set of 64-bit fingerprints of its *contour* — where it moved and how
//! long it waited — and there is no way back from one to a note. That is what
//! lets the table ship inside a product that must never carry somebody else's
//! notes. `docs/dataset-protocol.md` § The novelty table records how the file is
//! built and from what.
//!
//! ## What a fingerprint is made of
//!
//! Two choices decide it, and both are deliberate:
//!
//! - **Interval, not pitch.** A hook transposed is the same hook, so a
//!   fingerprint is the sequence of semitone steps between consecutive notes.
//!   Nothing in it names a key.
//! - **Onset gap, not note length.** Whether a line is played staccato or
//!   legato does not change what it is, but when the next note arrives does. The
//!   gap is quantised onto the note-value ladder, so a humanised 478-tick eighth
//!   and an exact 480 fingerprint identically — which is what lets the guard run
//!   before or after [`crate::humanize`] and get the same answer.
//!
//! ⚠ **The ladder carries no dotted values, so a dotted quarter fingerprints as
//! a half.** That coarsens the screen, and coarser errs towards catching more
//! than it should rather than less — the safe direction for a guard whose bad
//! outcome is letting a known hook through.

use std::sync::OnceLock;

use crate::generators::grid::note_value_ticks;
use crate::pattern::{LaneTrack, Note, Part};
use crate::rng::derive_seed;

/// The fingerprint scheme's version, mixed into every hash.
///
/// A change to how a step is built has to invalidate every hash written under
/// the old rules. Without this the two schemes would share a number space and a
/// stale table would go on matching things it never described — a screen that
/// reports on a melody nobody generated.
const SCHEME: u64 = 1;

/// The screen every take must pass: eight steps — nine notes — in a row.
pub const N_TIGHT: usize = 8;

/// The loosened screen, reached only once the tight one has refused four takes.
///
/// **Longer is looser**, which reads backwards until you see why: a twelve-step
/// run is a rarer coincidence than an eight-step one, so requiring twelve
/// rejects less. FR-011's "then loosen contour" is this.
pub const N_LOOSE: usize = 12;

/// How many *extra* takes the guard draws before it loosens.
pub const MAX_RETRIES: u8 = 3;

/// The domain each retry derives its seed from.
///
/// Named rather than numbered so a retry's stream is auditable, and spelled out
/// rather than formatted so the retry path allocates nothing.
const RETRY_DOMAINS: [&str; MAX_RETRIES as usize] =
    ["novelty/retry:1", "novelty/retry:2", "novelty/retry:3"];

/// The note-value ladder a gap is quantised onto, in ticks, ascending.
///
/// ⚠ These are the values [`note_value_ticks`] resolves for `"32"`, `"16T"`,
/// `"16"`, `"8T"`, `"8"`, `"4T"`, `"4"`, `"2"` and `"1"`, and
/// `the_ladder_is_the_projects_own_note_values` holds them to it — a second
/// spelling of the note vocabulary is how the two start to disagree.
const LADDER: [u32; 9] = [120, 160, 240, 320, 480, 640, 960, 1920, 3840];

/// One step of a melodic line: where it moved, and how long it waited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Semitones from the previous note, clamped to ±24.
    ///
    /// Clamped rather than kept exact because a two-octave leap and a
    /// three-octave one are the same event to a contour, and leaving the range
    /// open would let one outlying note make a common phrase unrecognisable.
    pub interval: i8,
    /// Index into [`LADDER`] — the gap between the two onsets.
    pub duration: u8,
}

/// Which of the [`LADDER`]'s values a gap is nearest, in ratio.
///
/// ⛔ **Integer arithmetic, with no logarithm and no square root.** The
/// comparison is against the *geometric* midpoint of each neighbouring pair —
/// `t² ≥ lo·hi` rather than `t ≥ √(lo·hi)` — because a fingerprint has to be
/// bit-identical on Windows, macOS and Linux, and `f64::ln` is not guaranteed
/// to be. A table built on one platform must match on all three.
fn duration_class(ticks: u32) -> u8 {
    let squared = u64::from(ticks) * u64::from(ticks);
    let mut class = 0;
    for pair in LADDER.windows(2) {
        if squared < u64::from(pair[0]) * u64::from(pair[1]) {
            break;
        }
        class += 1;
    }
    class
}

/// FNV-1a, as an accumulator.
///
/// ⚠ **Deliberately not [`crate::rng`]'s copy of the same algorithm.** That one
/// derives a seed from a domain name and this one fingerprints a contour; they
/// share an implementation and nothing else, and folding them together would
/// mean a change made for one silently moved every value of the other. Two six-
/// line functions is the cheaper mistake.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, b: u8) {
        self.0 ^= u64::from(b);
        self.0 = self.0.wrapping_mul(0x1000_0000_01b3);
    }

    fn mix(&mut self, value: u64) {
        for b in value.to_le_bytes() {
            self.byte(b);
        }
    }
}

/// The contour of a line of notes.
///
/// One note per onset — the top of a stack rather than a chord's worth of zero
/// intervals — so this is total over anything with pitches in it, not only over
/// the monophonic parts the guard actually screens.
pub fn steps(notes: &[Note]) -> Vec<Step> {
    let mut line: Vec<(u32, u8)> = notes.iter().map(|n| (n.start_tick, n.pitch)).collect();
    line.sort_unstable_by_key(|&(start, pitch)| (start, std::cmp::Reverse(pitch)));
    line.dedup_by_key(|&mut (start, _)| start);

    line.windows(2)
        .map(|pair| Step {
            interval: (i16::from(pair[1].1) - i16::from(pair[0].1)).clamp(-24, 24) as i8,
            duration: duration_class(pair[1].0 - pair[0].0),
        })
        .collect()
}

/// Every `n`-step window of a contour, as hashes.
///
/// Empty when the line is shorter than the window, which is a real answer: a
/// four-note riff cannot be an eight-step quotation of anything.
pub fn grams(steps: &[Step], n: usize) -> Vec<u64> {
    if n == 0 || steps.len() < n {
        return Vec::new();
    }
    steps.windows(n).map(gram).collect()
}

/// One window's hash.
///
/// The window's own length goes in, so an eight-step run and the twelve-step run
/// that opens with it can never collide — the table stores both widths and a
/// lookup has to mean the width it was built at.
fn gram(window: &[Step]) -> u64 {
    let mut hash = Fnv::new();
    hash.mix(SCHEME);
    hash.mix(window.len() as u64);
    for step in window {
        hash.byte(step.interval as u8);
        hash.byte(step.duration);
    }
    hash.0
}

/// Where a table or a contour listing failed to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based, so it matches what an editor shows.
    pub line: usize,
    pub message: String,
}

impl ParseError {
    fn at(index: usize, message: impl Into<String>) -> Self {
        Self {
            line: index + 1,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// The reference fingerprints, sorted so a lookup is a binary search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    hashes: Vec<u64>,
}

impl Table {
    /// Read `data/novelty/hooks.hash`'s format: one `0x`-prefixed hex u64 per
    /// line, `#` starting a comment.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut hashes = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let digits = line.strip_prefix("0x").ok_or_else(|| {
                ParseError::at(index, format!("`{line}` is not a 0x-prefixed hash"))
            })?;
            let value = u64::from_str_radix(digits, 16)
                .map_err(|e| ParseError::at(index, format!("`{line}`: {e}")))?;
            hashes.push(value);
        }
        hashes.sort_unstable();
        hashes.dedup();
        Ok(Self { hashes })
    }

    /// Build a table from reference contours — what `datasetc novelty` runs.
    ///
    /// Both widths go in, because a lookup at `n` only ever finds a hash written
    /// at `n`.
    pub fn from_melodies(melodies: &[Vec<Step>]) -> Self {
        let mut hashes: Vec<u64> = melodies
            .iter()
            .flat_map(|steps| {
                grams(steps, N_TIGHT)
                    .into_iter()
                    .chain(grams(steps, N_LOOSE))
            })
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        Self { hashes }
    }

    /// The file body — the hashes only. A caller writes its own header.
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.hashes.len() * 19);
        for hash in &self.hashes {
            out.push_str(&format!("0x{hash:016x}\n"));
        }
        out
    }

    pub fn contains(&self, hash: u64) -> bool {
        self.hashes.binary_search(&hash).is_ok()
    }

    /// Does this contour quote something in the table at width `n`?
    pub fn hits(&self, steps: &[Step], n: usize) -> bool {
        !self.hashes.is_empty() && grams(steps, n).iter().any(|hash| self.contains(*hash))
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

/// A contour listing — whitespace-separated `<interval>:<note value>` tokens,
/// `#` starting a comment, one melody per file.
///
/// The note value is spelled the way the rest of the dataset spells one, and is
/// read by [`note_value_ticks`] rather than by a second parser.
pub fn contour(text: &str) -> Result<Vec<Step>, ParseError> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default();
        for token in line.split_whitespace() {
            let (interval, value) = token.split_once(':').ok_or_else(|| {
                ParseError::at(index, format!("`{token}` is not <interval>:<note value>"))
            })?;
            let interval: i16 = interval.parse().map_err(|_| {
                ParseError::at(index, format!("`{interval}` is not a semitone count"))
            })?;
            let ticks = note_value_ticks(value)
                .ok_or_else(|| ParseError::at(index, format!("`{value}` is not a note value")))?;
            steps.push(Step {
                interval: interval.clamp(-24, 24) as i8,
                duration: duration_class(ticks),
            });
        }
    }
    Ok(steps)
}

/// The table compiled into the binary.
///
/// `include_str!` rather than a file read, because the engine has no filesystem
/// and the plugin has no install layout it can rely on — the same reason
/// `plugin/src/dataset.rs` compiles `data/` in.
const BUNDLED: &str = include_str!("../../data/novelty/hooks.hash");

/// The bundled table, parsed once.
///
/// ⚠ **A malformed table degrades to an empty one rather than panicking.** This
/// crate is loaded into someone else's process under `panic = "abort"`, so a
/// parse failure here would take a DAW down over a build-time mistake in a file
/// that ships inside the binary. `the_bundled_table_parses_and_is_not_empty` is
/// the gate that makes the degraded path unreachable — a broken file fails the
/// suite long before it can fail silently.
pub fn bundled() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        Table::parse(BUNDLED).unwrap_or_else(|error| {
            if cfg!(debug_assertions) {
                eprintln!("novelty: the bundled table does not parse ({error}); guard is off");
            }
            Table::default()
        })
    })
}

/// What the guard did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Not a part the guard screens, or nothing to screen against.
    NotScreened,
    /// The first take was clear.
    Clear,
    /// A later take was clear, and it is the one returned.
    Regenerated,
    /// Every take matched at [`N_TIGHT`]; the one returned is clear at
    /// [`N_LOOSE`].
    Loosened,
    /// Every take matched at both widths. The last one is returned anyway,
    /// because a producer pressing Generate must get notes.
    Exhausted,
}

/// The guard's telemetry for one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    /// How many takes were generated, including the one kept.
    pub takes: u8,
}

/// Which parts carry a hook, and are therefore worth screening.
///
/// ⛔ **Two parts are left out for two different reasons, and neither of them
/// is "later".** Drums have no pitch, so there is no contour to take. Chords are
/// polyphonic, and the interval line through a stack of voicings describes
/// nothing anybody could recognise — and a chord *progression* is common
/// property in a way a melody is not, so screening one would be a claim nothing
/// supports.
///
/// ⛔⛔ **THE BASS IS SCREENED WHEN IT IS NOT LOCKED TO THE KICK, and the rule
/// this replaces was right about one bassline in five.** It read: *"A bassline
/// is locked to the kick — `mirror_kick` copies its ticks outright — so
/// rerolling it to dodge a contour would trade the thing that makes it sit with
/// the drums for a match that cannot be heard as a quotation anyway."* That
/// holds exactly while the rhythm **is** `mirror_kick`, and `bass.rs` reads five:
/// `independent_riff`, `boom_chick`, `offbeat_8ths` and `reese_sustain` all
/// place their own onsets, and an independent riff is as recognisable as any
/// topline — a bass figure is what a great many records are known by. So the
/// exclusion now follows the *rhythm* rather than the part, and
/// [`crate::generators::bass::follows_the_kick`] is the one place that question
/// is answered.
///
/// ▶ **This matters more since the roster was returned to its researched
/// values** (owner's instruction, 2026-08-15): models are deliberately allowed
/// to overlap so that an artist sounds like themselves, which means two models
/// can now reach the same figure by design. What must never happen is that the
/// figure is one somebody already owns, and this guard — not model-to-model
/// difference — is what prevents that.
pub fn screens(part: Part, bass_follows_the_kick: bool) -> bool {
    match part {
        Part::Melody | Part::Counter => true,
        Part::Bass => !bass_follows_the_kick,
        Part::Drums | Part::Chords => false,
    }
}

/// Generate `part`, and keep drawing until the take is not a known hook.
///
/// Returns the lanes, **the take seed that produced them**, and what happened.
/// The seed comes back because the feel belongs to the take that was kept:
/// humanising the fourth take with the first take's seed would give two
/// different performances one shared set of jitter.
///
/// ⚠ **Still a pure function of its inputs.** The retry seeds are derived, not
/// drawn, so the same seed always walks the same chain and a saved seed rebuilds
/// the pattern the producer heard.
///
/// ⚠ **`bass_follows_the_kick` is asked of the model by the caller**, because
/// this module has no model — see [`screens`] for what the answer decides and
/// [`crate::generators::bass::follows_the_kick`] for how it is read. It is
/// meaningless for every part except the bass, and callers that cannot have one
/// pass `true`, which is the "leave it alone" answer.
pub fn screen<F>(
    table: &Table,
    part: Part,
    bass_follows_the_kick: bool,
    take: u64,
    mut generate: F,
) -> (Vec<LaneTrack>, u64, Report)
where
    F: FnMut(u64) -> Vec<LaneTrack>,
{
    if !screens(part, bass_follows_the_kick) || table.is_empty() {
        return (
            generate(take),
            take,
            Report {
                outcome: Outcome::NotScreened,
                takes: 1,
            },
        );
    }

    let mut loose: Option<(Vec<LaneTrack>, u64)> = None;
    let mut last = None;

    for attempt in 0..=MAX_RETRIES {
        let seed = match attempt.checked_sub(1) {
            None => take,
            Some(index) => derive_seed(take, RETRY_DOMAINS[index as usize]),
        };
        let lanes = generate(seed);
        let contours: Vec<Vec<Step>> = lanes.iter().map(|lane| steps(&lane.notes)).collect();

        if !contours.iter().any(|steps| table.hits(steps, N_TIGHT)) {
            let outcome = if attempt == 0 {
                Outcome::Clear
            } else {
                Outcome::Regenerated
            };
            return (
                lanes,
                seed,
                Report {
                    outcome,
                    takes: attempt + 1,
                },
            );
        }

        if loose.is_none() && !contours.iter().any(|steps| table.hits(steps, N_LOOSE)) {
            loose = Some((lanes.clone(), seed));
        }
        last = Some((lanes, seed));
    }

    let takes = MAX_RETRIES + 1;
    if let Some((lanes, seed)) = loose {
        return (
            lanes,
            seed,
            Report {
                outcome: Outcome::Loosened,
                takes,
            },
        );
    }

    let (lanes, seed) = last.expect("the loop body runs at least once");
    (
        lanes,
        seed,
        Report {
            outcome: Outcome::Exhausted,
            takes,
        },
    )
}

/// FR-011's "guard result logged in dev builds".
///
/// Only the interesting outcomes. A line per generation would bury them under
/// the hundreds of clear ones the test suite alone produces, and a log nobody
/// can read is not telemetry.
pub fn log(part: Part, report: &Report) {
    if cfg!(debug_assertions) && !matches!(report.outcome, Outcome::NotScreened | Outcome::Clear) {
        eprintln!(
            "novelty: {part:?} → {:?} after {} takes",
            report.outcome, report.takes
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start_tick: u32, pitch: u8) -> Note {
        Note {
            start_tick,
            len_ticks: 240,
            pitch,
            vel: 100,
            model_vel: None,
            slide_to_pitch: None,
            slide_ms: None,
            slide_overlap_ticks: None,
            timing_locked: false,
            articulation: None,
            reversed: false,
        }
    }

    #[test]
    fn the_ladder_is_the_projects_own_note_values() {
        // A second spelling of the note vocabulary is how the two start to
        // disagree, so the ladder is held to what `grid` already resolves.
        let names = ["32", "16T", "16", "8T", "8", "4T", "4", "2", "1"];
        for (name, ticks) in names.iter().zip(LADDER) {
            assert_eq!(note_value_ticks(name), Some(ticks), "{name}");
        }
    }

    #[test]
    fn a_gap_lands_on_the_nearest_value_in_ratio() {
        assert_eq!(duration_class(480), 4, "an exact eighth");
        assert_eq!(
            duration_class(478),
            4,
            "a humanised eighth is still an eighth"
        );
        assert_eq!(duration_class(240), 2);
        assert_eq!(duration_class(960), 6);
        assert_eq!(duration_class(0), 0, "no gap at all cannot panic");
        assert_eq!(
            duration_class(100_000),
            8,
            "longer than a whole note clamps"
        );
        // A dotted quarter has no rung of its own and rounds to the half.
        assert_eq!(duration_class(1440), 7);
    }

    #[test]
    fn a_contour_is_transposition_and_articulation_blind() {
        let low: Vec<Note> = [(0, 60), (480, 62), (960, 65)]
            .into_iter()
            .map(|(t, p)| note(t, p))
            .collect();
        let high: Vec<Note> = [(0, 72), (480, 74), (960, 77)]
            .into_iter()
            .map(|(t, p)| {
                let mut n = note(t, p);
                // Legato rather than staccato: a different performance of the
                // same line, and it must fingerprint the same.
                n.len_ticks = 480;
                n
            })
            .collect();
        assert_eq!(steps(&low), steps(&high));
        assert_eq!(
            steps(&low),
            vec![
                Step {
                    interval: 2,
                    duration: 4
                },
                Step {
                    interval: 3,
                    duration: 4
                }
            ]
        );
    }

    #[test]
    fn a_stack_at_one_onset_is_one_note_not_two_zero_intervals() {
        let chord = vec![note(0, 60), note(0, 64), note(0, 67), note(480, 65)];
        assert_eq!(
            steps(&chord),
            vec![Step {
                interval: -2,
                duration: 4
            }],
            "the top voice, once"
        );
    }

    #[test]
    fn a_line_shorter_than_the_window_has_no_grams() {
        let short: Vec<Note> = (0..4).map(|i| note(i * 240, 60 + i as u8)).collect();
        assert!(grams(&steps(&short), N_TIGHT).is_empty());
    }

    #[test]
    fn the_two_widths_never_collide() {
        let line: Vec<Note> = (0..16).map(|i| note(i * 240, 60 + (i % 5) as u8)).collect();
        let steps = steps(&line);
        let tight = grams(&steps, N_TIGHT);
        let loose = grams(&steps, N_LOOSE);
        assert!(!tight.is_empty() && !loose.is_empty());
        for hash in &loose {
            assert!(
                !tight.contains(hash),
                "widths must not share a number space"
            );
        }
    }

    #[test]
    fn a_table_round_trips_through_its_file_format() {
        let line: Vec<Note> = (0..16).map(|i| note(i * 240, 60 + (i % 7) as u8)).collect();
        let table = Table::from_melodies(&[steps(&line)]);
        assert!(!table.is_empty());
        let reparsed = Table::parse(&table.to_text()).unwrap();
        assert_eq!(table, reparsed);
    }

    #[test]
    fn a_table_rejects_what_it_cannot_read() {
        assert!(Table::parse("# only a comment\n\n").unwrap().is_empty());
        let error = Table::parse("0xdeadbeef\nnot-a-hash\n").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.to_string().contains("not-a-hash"), "{error}");
    }

    #[test]
    fn a_contour_listing_reads_the_datasets_own_note_values() {
        let steps = contour("# a comment\n+2:8 -1:16 0:1/4 +12:8T\n").unwrap();
        assert_eq!(
            steps,
            vec![
                Step {
                    interval: 2,
                    duration: 4
                },
                Step {
                    interval: -1,
                    duration: 2
                },
                Step {
                    interval: 0,
                    duration: 6
                },
                Step {
                    interval: 12,
                    duration: 3
                },
            ]
        );
    }

    #[test]
    fn a_contour_listing_names_the_line_it_could_not_read() {
        let error = contour("+2:8\n+3:banana\n").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.to_string().contains("banana"), "{error}");

        let error = contour("+2\n").unwrap_err();
        assert!(error.to_string().contains("interval"), "{error}");
    }

    #[test]
    fn an_empty_table_screens_nothing() {
        let table = Table::default();
        let (lanes, seed, report) = screen(&table, Part::Melody, true, 7, |s| {
            vec![LaneTrack {
                lane: crate::pattern::Lane::Melody,
                notes: vec![note(0, 60 + (s % 12) as u8)],
            }]
        });
        assert_eq!(report.outcome, Outcome::NotScreened);
        assert_eq!(seed, 7);
        assert_eq!(lanes.len(), 1);
    }

    #[test]
    fn the_parts_without_a_hook_are_not_screened() {
        // ⚠ **The bass answers to its rhythm rather than to its part** since
        // 2026-08-15 — a line that copies the kick's ticks is not a figure, and
        // one that places its own onsets is. Both directions are asserted here
        // because the flag is the whole rule.
        for part in [Part::Drums, Part::Chords] {
            assert!(!screens(part, true), "{part:?}");
            assert!(!screens(part, false), "{part:?}");
        }
        for part in [Part::Melody, Part::Counter] {
            assert!(screens(part, true), "{part:?}");
            assert!(screens(part, false), "{part:?}");
        }
        assert!(
            !screens(Part::Bass, true),
            "a kick-locked bass has no figure of its own to screen"
        );
        assert!(
            screens(Part::Bass, false),
            "a bass that places its own onsets is a figure, and 207 shipped models write one"
        );
    }

    #[test]
    fn the_bundled_table_parses_and_is_not_empty() {
        // The gate that keeps `bundled`'s degraded path unreachable: a broken
        // file fails here rather than shipping a guard that quietly does
        // nothing.
        let table = Table::parse(BUNDLED).expect("data/novelty/hooks.hash must parse");
        assert!(
            table.len() >= 64,
            "the shipped table holds {} hashes",
            table.len()
        );
        assert_eq!(bundled(), &table);
    }
}
