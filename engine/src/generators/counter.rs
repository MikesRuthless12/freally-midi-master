//! The countermelody generator (FR-006, research ch. 2).
//!
//! A countermelody answers a melody, so unlike every other part this one takes
//! another part as input. Its whole job is to be audible *without* competing:
//! `fillGapsOnly` is the parameter that says so, and
//! `a_gap_filling_counter_rarely_attacks_under_the_melody` is what holds it.
//!
//! **A "gap" is a space between the melody's *attacks*, not a moment when
//! nothing is sustaining.** Two readings were defensible and the other one is
//! unusable: melody notes are held to the following onset, so a legato line
//! covers nearly every tick of the bar. Under the sustain reading `fillGapsOnly`
//! deleted an entire arpeggio — the layer the model asked for silently vanished
//! — and `sustain_pad` became impossible, which would have made uk-drill
//! contradict itself by authoring both. What a producer does not want is the
//! counter *hitting at the same time* as the melody; a pad holding underneath is
//! the whole point of a pad.
//!
//! So one rule covers every style with no exceptions: a counter onset may not
//! coincide with a melody onset. **It is applied by choosing the onsets out of
//! the gaps rather than by generating freely and filtering afterwards** — an
//! invariant you cannot violate beats one you remember to check, and the filter
//! version is what threw the whole part away.
//!
//! Everything here writes on the grid. Feel is [`crate::humanize`]'s job.

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::chords::Chords;
use crate::generators::grid;
use crate::generators::read::{block, flag, number, pair, string_spec, text};
use crate::pattern::{Lane, LaneTrack, Note};
use crate::rng;
use crate::theory;

/// The velocity a counter note is written at.
///
/// Below the melody's 92 on purpose: a countermelody that is as loud as the
/// melody is a second melody, and the mix cannot fix an arrangement decision.
const COUNTER_VELOCITY: u8 = 76;

/// How much quieter an echo is than the note it echoes.
const ECHO_DROP: u8 = 14;

/// Where a bell echo sits when the model authors no `echoOffset`.
const DEFAULT_ECHO: &str = "8th";

/// Every parameter under `countermelody` this generator reads.
///
/// `engine/tests/counter.rs` walks what the models actually author and fails on
/// anything neither listed here nor explicitly excused.
pub const READ_KEYS: &[&str] = &[
    "registerOffset",
    "densityRatio",
    "fillGapsOnly",
    "styles",
    "echoOffset",
];

/// How the counter relates to the melody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// The melody, an octave away and a beat-subdivision late.
    ///
    /// ⛔ **It is delayed, and that is not decoration.** An octave copy at zero
    /// delay is a *doubling*, not an echo — and against `fillGapsOnly`, which
    /// every shipped model authors as true, every note of it lands on a melody
    /// onset and is removed. That made the counter silent for about half of all
    /// seeds and showed up as an oddly uniform 258/500 distinct counters across
    /// every model in the dataset, which is the sort of number that means one
    /// cause rather than twenty.
    OctaveEcho,
    /// The same, quieter and shorter — rage's bell, which rings rather than sings.
    BellEcho,
    /// Chord tones walked in the melody's gaps.
    Arp,
    /// A short figure that answers the phrase.
    AnswerLick,
    /// Held chord tones under the line — drill's strings.
    SustainPad,
}

impl Style {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "octave_echo" => Some(Self::OctaveEcho),
            "bell_echo" => Some(Self::BellEcho),
            "arp" => Some(Self::Arp),
            "answer_lick" => Some(Self::AnswerLick),
            "sustain_pad" => Some(Self::SustainPad),
            _ => None,
        }
    }
}

/// Is this style name one the generator can act on?
///
/// Exposed for the dataset gate: an unknown style would fall back to an octave
/// echo, so the layer the model asked for would silently not be the one played.
pub fn can_read_style(name: &str) -> bool {
    Style::parse(name).is_some()
}

pub fn generate(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    chords: &Chords,
    melody: &LaneTrack,
) -> LaneTrack {
    let empty = LaneTrack {
        lane: Lane::Counter,
        notes: Vec::new(),
    };

    let root = Value::Object(model.blocks.clone().into_iter().collect());
    let Some(counter) = block(Some(&root), "countermelody").cloned() else {
        return empty;
    };
    let counter = Some(&counter);

    // A counter answers a melody. With nothing to answer it would just be a
    // second melody, and the arrangement already has one — so a model whose
    // melody generated nothing gets no counter rather than a stand-in for it.
    if melody.notes.is_empty() {
        return empty;
    }

    let mut param_rng = rng::stream(seed, "counter/params");
    let mut place_rng = rng::stream(seed, "counter/place");
    let mut pitch_rng = rng::stream(seed, "counter/pitch");

    let style = string_spec(counter, "styles", &mut param_rng)
        .as_deref()
        .and_then(Style::parse)
        .unwrap_or(Style::OctaveEcho);

    let offset = number(counter, "registerOffset", 12.0, &mut param_rng) as i32;
    let ratio = number(counter, "densityRatio", 0.5, &mut param_rng).clamp(0.0, 4.0);
    let gaps_only = flag(counter, "fillGapsOnly", true);

    let echo = text(counter, "echoOffset")
        .and_then(grid::note_value_ticks)
        .unwrap_or_else(|| grid::note_value_ticks(DEFAULT_ECHO).unwrap_or(grid::SIXTEENTH * 2));

    // The register the counter lives in: the melody's, shifted. Read from the
    // model rather than measured off the notes, because a melody that happened
    // to sit low in its register would drag the counter down with it.
    let (low, high) = counter_register(Some(&root), offset);
    if low >= high {
        return empty;
    }

    let degrees = theory::scale_semitones(ctx.scale);
    let wanted = ((melody.notes.len() as f64) * ratio).round().max(0.0) as usize;
    if wanted == 0 {
        return empty;
    }

    // The ticks this counter may attack on. Built once and handed to whichever
    // style needs to choose onsets, so the gap rule holds by construction.
    let eighths = candidates(melody, ctx, grid::SIXTEENTH * 2, gaps_only);
    let sixteenths = candidates(melody, ctx, grid::SIXTEENTH, gaps_only);

    let mut notes = match style {
        Style::OctaveEcho => echo_notes(melody, offset, low, high, echo, 0, None),
        Style::BellEcho => echo_notes(
            melody,
            offset,
            low,
            high,
            echo,
            ECHO_DROP,
            // A bell is struck, not held.
            Some(grid::SIXTEENTH * 2),
        ),
        Style::Arp => arp_notes(ctx, chords, degrees, low, high, &eighths, &mut pitch_rng),
        Style::AnswerLick => {
            lick_notes(ctx, chords, degrees, low, high, &sixteenths, &mut pitch_rng)
        }
        Style::SustainPad => pad_notes(chords, low, high, &sixteenths, &mut pitch_rng),
    };

    // The echoes are the one family whose onsets are decided by the melody
    // rather than chosen, so they are the only ones the rule has to be applied
    // to after the fact.
    if gaps_only && matches!(style, Style::OctaveEcho | Style::BellEcho) {
        notes.retain(|note| !melody_attacks_near(melody, note.start_tick));
    }

    // Thinned after the gap rule for the echoes and before nothing else, so
    // `densityRatio` is the last word on how many notes there are.
    thin(&mut notes, wanted, &mut place_rng);

    notes.sort_by_key(|note| (note.start_tick, note.pitch));
    let mut track = LaneTrack {
        lane: Lane::Counter,
        notes,
    };
    super::fit_track_to_clip(&mut track, ctx);
    track
}

/// The counter's register: the melody's, shifted by `offset`.
fn counter_register(root: Option<&Value>, offset: i32) -> (u8, u8) {
    let melody = block(root, "melody");
    let (low, high) = pair(melody, "register").unwrap_or((60.0, 84.0));
    let shift = |value: f64| (value as i32 + offset).clamp(0, 127) as u8;
    (shift(low), shift(high))
}

/// Does the melody attack at, or within a sixteenth of, this tick?
///
/// The melody's *onsets* rather than its sustain — see the module comment for
/// why the sustain reading is unusable against a legato line.
pub(crate) fn melody_attacks_near(melody: &LaneTrack, tick: u32) -> bool {
    melody
        .notes
        .iter()
        .any(|note| note.start_tick.abs_diff(tick) < grid::SIXTEENTH)
}

/// The ticks on `step` a counter is allowed to attack on.
///
/// With `gaps_only` these are the spaces between the melody's attacks; without
/// it, every tick on the grid.
fn candidates(melody: &LaneTrack, ctx: &SessionContext, step: u32, gaps_only: bool) -> Vec<u32> {
    (0..ctx.total_ticks())
        .step_by(step.max(1) as usize)
        .filter(|tick| !gaps_only || !melody_attacks_near(melody, *tick))
        .collect()
}

/// The melody, transposed and late.
#[allow(clippy::too_many_arguments)]
fn echo_notes(
    melody: &LaneTrack,
    offset: i32,
    low: u8,
    high: u8,
    delay: u32,
    drop: u8,
    len: Option<u32>,
) -> Vec<Note> {
    melody
        .notes
        .iter()
        .filter_map(|note| {
            let pitch =
                theory::fold_into_register((i32::from(note.pitch) + offset) as i16, low, high)?;
            Some(Note {
                model_vel: None,
                start_tick: note.start_tick + delay,
                len_ticks: len.unwrap_or(note.len_ticks),
                pitch,
                vel: note.vel.saturating_sub(drop).max(1),
                slide_to_pitch: None,
                articulation: None,
            })
        })
        .collect()
}

/// Chord tones walked across the ticks the counter is allowed to attack on.
fn arp_notes(
    ctx: &SessionContext,
    chords: &Chords,
    degrees: &[u8],
    low: u8,
    high: u8,
    on: &[u32],
    rng: &mut impl Rng,
) -> Vec<Note> {
    let step = grid::SIXTEENTH * 2;
    let mut notes = Vec::new();
    let mut index = 0usize;

    for tick in on.iter().copied() {
        let pitch = match chords.at(tick) {
            Some(event) if !event.tones.is_empty() => {
                let class = event.tones[index % event.tones.len()];
                index += 1;
                theory::pitch_class_in_register(class, low, high)
            }
            // No harmony authored: walk the scale instead of going silent, since
            // rage states its chords through the lead and still wants a bell.
            _ => {
                let degree = rng.random_range(1..=degrees.len() as i32);
                theory::fold_into_register(
                    (60 + i32::from(ctx.key_root) + theory::degree_semitone(degrees, degree))
                        as i16,
                    low,
                    high,
                )
            }
        };

        if let Some(pitch) = pitch {
            notes.push(Note {
                model_vel: None,
                start_tick: tick,
                len_ticks: step,
                pitch,
                vel: COUNTER_VELOCITY,
                slide_to_pitch: None,
                articulation: None,
            });
        }
    }

    notes
}

/// A short figure at the end of each bar, answering the phrase.
#[allow(clippy::too_many_arguments)]
fn lick_notes(
    ctx: &SessionContext,
    chords: &Chords,
    degrees: &[u8],
    low: u8,
    high: u8,
    on: &[u32],
    rng: &mut impl Rng,
) -> Vec<Note> {
    let bar = ctx.ticks_per_bar();
    let step = grid::SIXTEENTH;
    let mut notes = Vec::new();

    // The last beat of each bar is where an answer goes: the melody's phrase has
    // said its thing and the bar has not turned over yet.
    let mut bar_start = 0;
    while bar_start < ctx.total_ticks() {
        let from = bar_start + bar.saturating_sub(grid::ticks_per_beat(ctx));
        let mut degree = rng.random_range(1..=degrees.len() as i32);

        // Only the allowed ticks inside that last beat, so the answer lands in
        // the melody's gaps by construction rather than being filtered later.
        for tick in on
            .iter()
            .copied()
            .filter(|tick| *tick >= from && *tick < bar_start + bar)
        {
            // Descending, which is what an answer does — it settles.
            degree -= 1;
            let base = 60 + i32::from(ctx.key_root) + theory::degree_semitone(degrees, degree);
            let pitch = chords
                .at(tick)
                .and_then(|event| {
                    (0..12).find_map(|shift| {
                        let candidate = base - shift;
                        event
                            .has_tone((candidate.rem_euclid(128)) as u8)
                            .then_some(candidate)
                    })
                })
                .unwrap_or(base);

            if let Some(pitch) = theory::fold_into_register(pitch as i16, low, high) {
                notes.push(Note {
                    model_vel: None,
                    start_tick: tick,
                    len_ticks: step,
                    pitch,
                    vel: COUNTER_VELOCITY,
                    slide_to_pitch: None,
                    articulation: None,
                });
            }
        }

        bar_start += bar;
    }

    notes
}

/// One held chord tone per chord.
///
/// The onset is nudged forward to the first allowed tick, so a pad under a
/// legato line still starts in a gap rather than being thrown away for landing
/// on the same attack as the melody.
///
/// **Which tone it holds is chosen, not always the root.** Drill's strings sit
/// on the third or the fifth as readily as the root, so a pad that always took
/// the root was both the least interesting voicing available and — because
/// uk-drill weights this style at 60% — the reason its counter reached only 264
/// distinct shapes in 500 seeds where every other model reached 500.
fn pad_notes(chords: &Chords, low: u8, high: u8, on: &[u32], rng: &mut impl Rng) -> Vec<Note> {
    let mut notes = Vec::new();

    for event in &chords.events {
        let end = event.start_tick + event.len_ticks;
        let Some(start) = on
            .iter()
            .copied()
            .find(|tick| *tick >= event.start_tick && *tick < end)
        else {
            continue;
        };

        // ⛔ **One or two tones, and it may breathe.** A pad that always held a
        // single note for the whole chord was the least interesting voicing
        // available *and* the reason uk-drill — which weights this style at 60%
        // — reached only 264 of 500 distinct counters where every other model
        // reached 500. Strings in drill are a held two-note voicing that lifts
        // before the change as often as not, so the musical fix and the variety
        // fix are the same fix.
        let voices = if event.tones.len() > 1 && rng.random_bool(0.5) {
            2
        } else {
            1
        };
        let breathe = rng.random_bool(0.4);
        let len = if breathe {
            // Up before the chord turns over, so the next one lands.
            (end - start)
                .saturating_sub(grid::SIXTEENTH * 2)
                .max(grid::SIXTEENTH)
        } else {
            end - start
        };

        let mut used: Vec<u8> = Vec::with_capacity(voices);
        for voice in 0..voices {
            let tone = if event.tones.is_empty() {
                event.root
            } else {
                event.tones[rng.random_range(0..event.tones.len())]
            };
            // Which octave the voicing sits in, rather than always the lowest
            // one the register allows. `pitch_class_in_register` answers with
            // the bottom-most, and a pad pinned to the floor of its register is
            // both a narrower grammar and a less musical one — strings in drill
            // sit high as often as low.
            let places = octaves_of(tone, low, high);
            if places.is_empty() {
                continue;
            }
            let pitch = places[rng.random_range(0..places.len())];
            if used.contains(&pitch) {
                continue;
            }
            used.push(pitch);

            notes.push(Note {
                model_vel: None,
                start_tick: start,
                len_ticks: len,
                pitch,
                // The upper voice sits under the lower one, so the voicing reads
                // as one sound rather than two parts.
                vel: COUNTER_VELOCITY.saturating_sub(8 + voice as u8 * 6).max(1),
                slide_to_pitch: None,
                articulation: None,
            });
        }
    }

    notes
}

/// Every pitch of this class that fits inside the register.
///
/// [`theory::pitch_class_in_register`] answers with the lowest one, which is the
/// right answer when a part wants a definite pitch and the wrong one when it is
/// choosing a voicing.
fn octaves_of(class: u8, low: u8, high: u8) -> Vec<u8> {
    (low..=high)
        .filter(|pitch| pitch % 12 == class % 12)
        .collect()
}

/// Keep `wanted` of these notes, spread across the pattern.
///
/// Drops from the middle rather than the end: truncating would leave the counter
/// piled into the first bars and silent afterwards, which reads as a generator
/// that ran out rather than a part with space in it.
fn thin(notes: &mut Vec<Note>, wanted: usize, rng: &mut impl Rng) {
    if notes.len() <= wanted {
        return;
    }
    if wanted == 0 {
        notes.clear();
        return;
    }

    // Walk the list keeping every `stride`-th note, then randomise which offset
    // the walk starts at so two seeds do not thin identically.
    let start = rng.random_range(0..notes.len());
    let mut keep: Vec<bool> = vec![false; notes.len()];
    for index in 0..wanted {
        let at = (start + index * notes.len() / wanted) % notes.len();
        keep[at] = true;
    }

    let mut index = 0;
    notes.retain(|_| {
        let this = keep[index];
        index += 1;
        this
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::{chords as harmony, melody};
    use serde_json::json;

    fn model_with(counter: Value) -> StyleModel {
        serde_json::from_value(json!({
            "id": "test",
            "type": "genre",
            "name": "Test",
            "melody": { "register": [60, 84], "densityPerBar": [4, 6], "halfStepDissonance": 0 },
            "countermelody": counter,
            "chords": {
                "progressionFamilies": [{ "roman": ["i", "VI", "III", "VII"], "weight": 1 }],
                "harmonicRhythm": "1_per_bar",
                "voicing": { "register": [48, 72] }
            }
        }))
        .expect("the test model must parse")
    }

    fn run(counter: Value, seed: u64) -> (LaneTrack, LaneTrack) {
        let model = model_with(counter);
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, seed);
        let lead = melody::generate(&model, &ctx, seed, &chords, &[]);
        let track = generate(&model, &ctx, seed, &chords, &lead);
        (track, lead)
    }

    /// The share of counter onsets that land while the melody is sounding.
    fn overlap(counter: &LaneTrack, melody: &LaneTrack) -> f64 {
        if counter.notes.is_empty() {
            return 0.0;
        }
        let clashing = counter
            .notes
            .iter()
            .filter(|note| melody_attacks_near(melody, note.start_tick))
            .count();
        clashing as f64 / counter.notes.len() as f64
    }

    #[test]
    fn a_model_with_no_countermelody_block_generates_nothing() {
        let model: StyleModel =
            serde_json::from_value(json!({ "id": "bare", "type": "genre", "name": "Bare" }))
                .unwrap();
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 1);
        let lead = melody::generate(&model, &ctx, 1, &chords, &[]);
        assert!(generate(&model, &ctx, 1, &chords, &lead).notes.is_empty());
    }

    #[test]
    fn with_no_melody_to_answer_there_is_no_counter() {
        // A countermelody without a melody is just a second melody, and the
        // arrangement already has one.
        let model = model_with(json!({ "styles": "arp" }));
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 1);
        let silent = LaneTrack {
            lane: Lane::Melody,
            notes: Vec::new(),
        };
        assert!(generate(&model, &ctx, 1, &chords, &silent).notes.is_empty());
    }

    #[test]
    fn a_gap_filling_counter_rarely_attacks_under_the_melody() {
        // ⛔ FR-006's acceptance criterion: overlap ≤ 20% when `fillGapsOnly`.
        // Measured on onsets, for the reason in the module comment.
        for style in [
            "octave_echo",
            "arp",
            "answer_lick",
            "sustain_pad",
            "bell_echo",
        ] {
            for seed in 1..20u64 {
                let (counter, lead) = run(
                    json!({ "styles": style, "fillGapsOnly": true, "densityRatio": 0.5 }),
                    seed,
                );
                let ratio = overlap(&counter, &lead);
                assert!(
                    ratio <= 0.2,
                    "{style} seed {seed}: {ratio:.2} of counter onsets landed under the melody"
                );
            }
        }
    }

    #[test]
    fn without_the_gap_rule_a_counter_may_double_the_melody() {
        // The other side of the same switch: `fillGapsOnly: false` has to
        // actually let the counter sit on the melody, or the flag does nothing.
        let mut doubled = false;
        for seed in 1..30u64 {
            let (counter, lead) = run(
                json!({ "styles": "octave_echo", "fillGapsOnly": false, "densityRatio": 1.0 }),
                seed,
            );
            if overlap(&counter, &lead) > 0.2 {
                doubled = true;
                break;
            }
        }
        assert!(
            doubled,
            "fillGapsOnly: false never let the counter double the lead"
        );
    }

    #[test]
    fn the_register_offset_moves_the_counter_off_the_melody() {
        // Drill offsets by -12 and trap by +12; both have to actually land
        // outside the melody's own register.
        let (up, _) = run(json!({ "styles": "arp", "registerOffset": 12 }), 4);
        let (down, _) = run(json!({ "styles": "arp", "registerOffset": -12 }), 4);

        assert!(!up.notes.is_empty() && !down.notes.is_empty());

        // Averages, not extremes. The two registers are the melody's shifted
        // ±12, so they *touch* at 72 — comparing min against max asks the two
        // ranges to be disjoint, which they are not and need not be. Where the
        // part sits is the claim being made.
        let mean = |track: &LaneTrack| {
            track.notes.iter().map(|n| f64::from(n.pitch)).sum::<f64>() / track.notes.len() as f64
        };
        let (below, above) = (mean(&down), mean(&up));
        assert!(
            below < above - 6.0,
            "an offset of -12 averaged {below:.1} and +12 averaged {above:.1}"
        );
    }

    #[test]
    fn the_density_ratio_is_a_ratio_of_the_melodys_own_count() {
        for ratio in [0.25, 0.5, 1.0] {
            let (counter, lead) = run(
                json!({ "styles": "octave_echo", "fillGapsOnly": false, "densityRatio": ratio }),
                12,
            );
            let wanted = (lead.notes.len() as f64 * ratio).round() as usize;
            assert_eq!(
                counter.notes.len(),
                wanted,
                "ratio {ratio} over {} melody notes",
                lead.notes.len()
            );
        }
    }

    #[test]
    fn a_bell_echo_arrives_after_the_note_it_echoes() {
        let (counter, lead) = run(
            json!({
                "styles": "bell_echo",
                "echoOffset": "1/8",
                "fillGapsOnly": false,
                "densityRatio": 1.0
            }),
            8,
        );
        let eighth = grid::SIXTEENTH * 2;
        assert!(!counter.notes.is_empty());
        // Every echo sits an eighth after some melody onset.
        for note in &counter.notes {
            assert!(
                lead.notes
                    .iter()
                    .any(|source| source.start_tick + eighth == note.start_tick),
                "an echo at {} answers no melody note an eighth earlier",
                note.start_tick
            );
        }
    }

    #[test]
    fn a_sustained_pad_holds_each_chord() {
        let (counter, _) = run(
            json!({ "styles": "sustain_pad", "densityRatio": 1.0, "fillGapsOnly": false }),
            6,
        );
        assert!(!counter.notes.is_empty());
        // A pad is long. Anything a sixteenth long is not a pad.
        for note in &counter.notes {
            assert!(
                note.len_ticks > grid::SIXTEENTH * 2,
                "{} ticks",
                note.len_ticks
            );
        }
    }

    #[test]
    fn the_same_seed_reproduces_the_same_counter() {
        let spec = json!({ "styles": "arp", "densityRatio": 0.5 });
        assert_eq!(run(spec.clone(), 77).0, run(spec, 77).0);
    }

    #[test]
    fn every_authored_style_name_is_readable() {
        for name in [
            "octave_echo",
            "bell_echo",
            "arp",
            "answer_lick",
            "sustain_pad",
        ] {
            assert!(can_read_style(name), "{name}");
        }
        assert!(!can_read_style("shimmer"));
    }

    #[test]
    fn notes_never_run_past_the_end_of_the_pattern() {
        for style in ["octave_echo", "arp", "answer_lick", "sustain_pad"] {
            for seed in 1..15u64 {
                let (counter, _) = run(json!({ "styles": style, "densityRatio": 1.0 }), seed);
                let ctx = SessionContext::default();
                for note in &counter.notes {
                    assert!(
                        note.start_tick + note.len_ticks <= ctx.total_ticks(),
                        "{style} seed {seed}: a note runs past the pattern"
                    );
                    assert!(note.len_ticks > 0 && note.vel >= 1);
                }
            }
        }
    }
}
