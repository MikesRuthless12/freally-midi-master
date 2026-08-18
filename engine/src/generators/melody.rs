//! The melody generator (FR-005, research ch. 2).
//!
//! Four steps, each on its own seeded stream: choose the phrase plan, write the
//! rhythm, walk the pitches, then apply the structure that decides how phrases
//! repeat. Re-rolling the pitches cannot move an onset, and vice versa.
//!
//! **Pitch is chosen as a scale degree and only then turned into a MIDI note.**
//! That is the same shape [`super::chords`] uses for the same reason: the notes
//! come *from* the key, so being in-scale is structural rather than filtered for
//! afterwards. The one device allowed to leave the scale is
//! `halfStepDissonance`, which says so in its name, and
//! `a_melody_stays_in_the_key_unless_the_model_asks_to_leave` is what holds the
//! line.
//!
//! Everything here writes on the grid. Feel is [`crate::humanize`]'s job and the
//! caller applies it afterwards.

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::chords::Chords;
use crate::generators::grid;
use crate::generators::read::{block, flag, number, optional_number, pair, text};
use crate::pattern::{Articulation, Lane, LaneTrack, Note};
use crate::rng;
use crate::theory;

/// The velocity a melody note is written at before feel is applied.
///
/// One number plus a strong-beat accent, rather than a tier table: a lead's
/// dynamics come from the performance, and [`crate::humanize`] spreads
/// `velocityVar` over this. A per-note ramp here would be a second opinion
/// about the same thing.
const MELODY_VELOCITY: u8 = 92;

/// How much louder a note on a strong beat is written.
const ACCENT: u8 = 12;

/// Where the melody sits when the model authors no register. C4–C6.
const DEFAULT_REGISTER: (u8, u8) = (60, 84);

/// Onsets per bar when the model says nothing.
const DEFAULT_DENSITY: (f64, f64) = (4.0, 7.0);

/// The least chord-tone bias a melody carrying the harmony may have.
///
/// When `chords.impliedByRiff` is set there is no pad stating the changes, so
/// the lead is the only thing saying what the harmony is. A wandering line would
/// not be a stylistic choice there, it would leave the progression unstated.
const IMPLIED_HARMONY_BIAS: f64 = 0.85;

/// Every parameter under `melody` this generator reads, as dotted paths.
///
/// A prefix covers everything under it: `"intervals"` covers `intervals.step`,
/// because the generator reads that block's own fields.
///
/// This is not documentation — `engine/tests/melody.rs` walks what the models
/// actually author and fails on anything that is neither in this list nor
/// explicitly excused. A parameter nobody reads is decorative and nobody can
/// tell by looking; the dataset has shipped three of those already.
pub const READ_KEYS: &[&str] = &[
    "register",
    "phraseBars",
    "densityPerBar",
    "structure",
    "intervals",
    "contour",
    "chordToneBias",
    "colorTones",
    "halfStepDissonance",
    "endVariation",
    "octaveJumpProb",
    "motifPitchCount",
    "articulation",
    "loopVerbatim",
    "straightEighthsProb",
    "emptyBarBias",
    "twoBarCellRepeat",
    "monophonicBias",
    "mirrorSnareDisplacement",
];

/// How the phrases of a pattern relate to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Structure {
    /// One phrase, repeated. The default, and what most trap and drill do.
    RiffLoop,
    /// Two phrases: the second answers the first by ending differently.
    QuestionAnswer,
    /// The response is the call an octave down and quieter.
    CallResponse,
    /// One phrase across the whole pattern — no repeat to hear.
    LongArc,
}

impl Structure {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "riff_loop" => Some(Self::RiffLoop),
            "question_answer" => Some(Self::QuestionAnswer),
            "call_response" => Some(Self::CallResponse),
            "long_arc" => Some(Self::LongArc),
            _ => None,
        }
    }
}

/// Is this structure name one the generator can act on?
///
/// Exposed for the dataset gate. A structure that fails to parse would fall
/// back to a riff loop, so the phrase shape the model asked for would silently
/// not happen — and only a test over the shipped data can see that.
pub fn can_read_structure(name: &str) -> bool {
    Structure::parse(name).is_some()
}

/// Is this articulation name one the generator can act on?
pub fn can_read_articulation(name: &str) -> bool {
    matches!(name, "staccato" | "legato")
}

/// The overall direction a phrase travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contour {
    /// Oscillate around a pivot — a riff, not a journey.
    Riff,
    /// Net downward.
    Descend,
    /// Up, then back down.
    Arch,
}

/// One place a note starts, and whether it lands somewhere the ear counts.
#[derive(Debug, Clone, Copy)]
struct Onset {
    tick: u32,
    len: u32,
    /// On a beat rather than between two. Strong beats are where chord-tone
    /// bias is applied, because that is where a wrong note is audible.
    strong: bool,
}

pub fn generate(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    chords: &Chords,
    drums: &[LaneTrack],
) -> LaneTrack {
    let empty = LaneTrack {
        lane: Lane::Melody,
        notes: Vec::new(),
    };

    let root = Value::Object(model.blocks.clone().into_iter().collect());
    let Some(melody) = block(Some(&root), "melody").cloned() else {
        return empty;
    };
    let melody = Some(&melody);

    let mut plan_rng = rng::stream(seed, "melody/plan");
    let mut rhythm_rng = rng::stream(seed, "melody/rhythm");
    // Reading the model's own parameters. Kept apart from the walk's streams so
    // that adding a parameter cannot shift the notes.
    let mut param_rng = rng::stream(seed, "melody/params");
    let mut dyad_rng = rng::stream(seed, "melody/dyad");

    let structure = text(melody, "structure")
        .and_then(Structure::parse)
        .unwrap_or(Structure::RiffLoop);

    let (low, high) = register(melody);
    if low >= high {
        return empty;
    }

    // How long a phrase is, in bars, and how many of them fit.
    //
    // `twoBarCellRepeat` overrides the authored phrase length rather than
    // sitting beside it: drill authors both, and two answers to "how long is a
    // phrase" is how one of them silently stops mattering.
    let phrase_bars = if flag(melody, "twoBarCellRepeat", false) {
        2
    } else {
        number(melody, "phraseBars", 2.0, &mut plan_rng)
            .round()
            .max(1.0) as u16
    };
    let phrase_bars = phrase_bars.min(ctx.bars.max(1));

    let phrase_count = match structure {
        // One phrase covering everything: there is no repeat, so the plan is a
        // single phrase as long as the pattern.
        Structure::LongArc => 1,
        _ => ctx.bars.max(1).div_ceil(phrase_bars),
    };

    let ticks_per_bar = ctx.ticks_per_bar();
    let phrase_ticks = if structure == Structure::LongArc {
        ticks_per_bar * u32::from(ctx.bars.max(1))
    } else {
        ticks_per_bar * u32::from(phrase_bars)
    };

    // The rhythm is written once and reused, which is what makes a riff a riff.
    // `endVariation` and the question/answer ending are applied to the *pitches*
    // of the last phrase, not to where the notes fall.
    let snare = mirror_ticks(melody, drums);
    let onsets = rhythm(melody, ctx, phrase_ticks, snare.as_deref(), &mut rhythm_rng);
    if onsets.is_empty() {
        return empty;
    }

    let degrees = theory::scale_semitones(ctx.scale);
    let motif = optional_number(melody, "motifPitchCount", &mut plan_rng)
        .map(|count| count.round().max(1.0) as usize);

    // `chords.impliedByRiff` — rage states its harmony through the lead rather
    // than through a pad, and `engine/tests/chords.rs` records that this
    // generator is the one that should read it. When the riff *is* the harmony
    // it cannot wander off the chord, so the authored bias becomes a floor
    // rather than the whole story.
    let implied = flag(block(Some(&root), "chords"), "impliedByRiff", false);
    let authored_bias = number(melody, "chordToneBias", 0.6, &mut param_rng);
    let chord_tone_bias = if implied {
        authored_bias.max(IMPLIED_HARMONY_BIAS)
    } else {
        authored_bias
    };

    let walk = Walk {
        chord_tone_bias,
        half_step: number(melody, "halfStepDissonance", 0.0, &mut param_rng),
        octave_jump: number(melody, "octaveJumpProb", 0.0, &mut param_rng),
        colour_tones: colour_degrees(melody),
        intervals: interval_weights(melody),
        contour_weights: contour_weights(melody),
        motif,
    };

    let loop_verbatim = flag(melody, "loopVerbatim", false);
    let end_variation = flag(melody, "endVariation", true);
    let staccato = text(melody, "articulation") == Some("staccato");
    let monophonic = number(melody, "monophonicBias", 1.0, &mut param_rng);

    // The walk, run for one phrase.
    //
    // ⛔ **Run per phrase from the *same* seed, not once and copied.** The draws
    // happen in the same order every time, so the contour, the intervals and the
    // rhythm come out identical across repeats — a riff stays the same riff —
    // while `nearest_chord_degree` is asked about *this* phrase's position and
    // therefore follows the progression.
    //
    // Generating once and replaying it is what the first cut did, and over a
    // chord a bar it put half the repeats against the wrong harmony: strong
    // beats landed on a chord tone 0.54 of the time against a bias of 0.8. A
    // loop that moves with the chords is also simply what trap does.
    let phrase_pitches = |offset: u32| {
        walk.pitches(
            &onsets,
            ctx,
            degrees,
            chords,
            low,
            high,
            offset,
            &mut rng::stream(seed, "melody/pitch"),
            &mut rng::stream(seed, "melody/colour"),
        )
    };

    // What a verbatim repeat repeats, and what the answer and response are
    // derived from.
    let base = phrase_pitches(0);

    let mut notes = Vec::new();
    for phrase in 0..phrase_count {
        let offset = phrase_ticks * u32::from(phrase);
        if offset >= ctx.total_ticks() {
            break;
        }

        let last = phrase + 1 == phrase_count;

        // Which pitches this repeat plays.
        //
        // `loopVerbatim` is rage's rule and it means exactly what it says: the
        // same notes every time, no end variation, because the point is the
        // hypnotic repeat rather than the development.
        let pitches = if loop_verbatim {
            base.clone()
        } else {
            // This phrase's own harmony, same shape as the base.
            let fitted = if phrase == 0 {
                base.clone()
            } else {
                phrase_pitches(offset)
            };
            match structure {
                Structure::QuestionAnswer if phrase % 2 == 1 => {
                    answer(&fitted, &onsets, ctx, degrees, chords, low, high, offset)
                }
                Structure::CallResponse if phrase % 2 == 1 => response(&fitted, low, high),
                _ if last && end_variation && phrase_count > 1 => {
                    vary_ending(&fitted, &onsets, ctx, degrees, chords, low, high, offset)
                }
                _ => fitted,
            }
        };

        let quieter = structure == Structure::CallResponse && phrase % 2 == 1;

        for (onset, pitch) in onsets.iter().zip(pitches.iter()) {
            let start = offset + onset.tick;
            if start >= ctx.total_ticks() {
                continue;
            }

            // ⚠ Not clamped to the pattern here — [`super::fit_to_clip`] is the
            // one place that rule lives now, and it runs on the finished lane.
            let len = if staccato {
                grid::SIXTEENTH.min(onset.len)
            } else {
                onset.len
            }
            .max(1);

            let mut vel = if onset.strong {
                MELODY_VELOCITY.saturating_add(ACCENT)
            } else {
                MELODY_VELOCITY
            };
            if quieter {
                vel = vel.saturating_sub(16).max(1);
            }

            notes.push(Note {
                model_vel: None,
                start_tick: start,
                len_ticks: len,
                pitch: *pitch,
                vel,
                slide_to_pitch: None,
                articulation: staccato.then_some(Articulation::Staccato),
                reversed: false,
            });

            // `monophonicBias` is read as *how often the line stays one note*.
            //
            // Two readings were defensible — the other being "how often the
            // whole part is monophonic", a per-pattern coin flip. This one is
            // note-level, which is what the block is for, and it makes drill's
            // 0.8 audible as an occasional doubled note rather than as a
            // property you would have to generate twenty patterns to observe.
            if monophonic < 1.0 && dyad_rng.random_bool((1.0 - monophonic).clamp(0.0, 1.0)) {
                if let Some(double) = dyad(*pitch, chords, start, low, high) {
                    notes.push(Note {
                        model_vel: None,
                        start_tick: start,
                        len_ticks: len,
                        pitch: double,
                        vel: vel.saturating_sub(10).max(1),
                        slide_to_pitch: None,
                        articulation: staccato.then_some(Articulation::Staccato),
                        reversed: false,
                    });
                }
            }
        }
    }

    notes.sort_by_key(|note| (note.start_tick, note.pitch));
    let mut track = LaneTrack {
        lane: Lane::Melody,
        notes,
    };
    super::fit_track_to_clip(&mut track, ctx);
    track
}

/// The register, clamped to MIDI and to being the right way round.
fn register(melody: Option<&Value>) -> (u8, u8) {
    match pair(melody, "register") {
        Some((low, high)) if low < high => {
            (low.clamp(0.0, 127.0) as u8, high.clamp(0.0, 127.0) as u8)
        }
        _ => DEFAULT_REGISTER,
    }
}

/// The snare's ticks within a bar, when the model asks the melody to mirror them.
///
/// Returns `None` unless `mirrorSnareDisplacement` is authored *and* there is a
/// snare to mirror. Drill's melody leans on the displaced snare, and that
/// displacement is the drums' decision — reproducing it here from the same
/// parameters would be a second answer to one question, and the two would drift.
fn mirror_ticks(melody: Option<&Value>, drums: &[LaneTrack]) -> Option<Vec<u32>> {
    if !flag(melody, "mirrorSnareDisplacement", false) {
        return None;
    }

    let ticks: Vec<u32> = drums
        .iter()
        .filter(|track| track.lane == Lane::Snare)
        .flat_map(|track| track.notes.iter().map(|note| note.start_tick))
        .collect();

    (!ticks.is_empty()).then_some(ticks)
}

/// Where the notes of one phrase fall.
fn rhythm(
    melody: Option<&Value>,
    ctx: &SessionContext,
    phrase_ticks: u32,
    snare: Option<&[u32]>,
    rng: &mut impl Rng,
) -> Vec<Onset> {
    let sixteenth = grid::SIXTEENTH;
    let per_bar = grid::sixteenths_per_bar(ctx).max(1);
    let ticks_per_bar = ctx.ticks_per_bar();
    let bars = (phrase_ticks / ticks_per_bar.max(1)).max(1);

    let density = match pair(melody, "densityPerBar") {
        Some((low, high)) if high >= low => (low, high),
        _ => match optional_number(melody, "densityPerBar", rng) {
            Some(exact) => (exact, exact),
            None => DEFAULT_DENSITY,
        },
    };

    let straight_eighths = number(melody, "straightEighthsProb", 0.0, rng);
    let empty_bar = number(melody, "emptyBarBias", 0.0, rng);

    let mut onsets: Vec<Onset> = Vec::new();

    for bar in 0..bars {
        // Space is a choice rage makes deliberately: a bar with nothing in it is
        // why the next one lands.
        if empty_bar > 0.0 && rng.random_bool(empty_bar.clamp(0.0, 1.0)) {
            continue;
        }

        // An eighth-note bar has half the slots, so the same density reads as a
        // busier line rather than a faster one.
        let eighths = straight_eighths > 0.0 && rng.random_bool(straight_eighths.clamp(0.0, 1.0));
        let step = if eighths { 2 } else { 1 };
        let slots: Vec<u32> = (0..per_bar).step_by(step).collect();
        if slots.is_empty() {
            continue;
        }

        // ⛔ **TASK-125 leans this draw and does not replace it.** The authored
        // range is the model's own statement about its spread — `[2, 6]` means a
        // sparse bar is as much this artist as a busy one — so Complex moves the
        // *average* toward six while leaving two reachable. A model that authors
        // one number is unmoved at every setting, which is the rule.
        let wanted = ctx
            .complexity
            .draw(density.0, density.1, rng)
            .round()
            .clamp(0.0, slots.len() as f64) as usize;
        if wanted == 0 {
            continue;
        }

        let mut chosen: Vec<u32> = Vec::with_capacity(wanted);

        // Beat one first when there is room for it — a phrase that never states
        // its downbeat does not read as a phrase.
        if let Some(first) = slots.first() {
            chosen.push(*first);
        }

        // The snare's own sixteenths, when mirroring, before anything random.
        if let Some(snare) = snare {
            for tick in snare {
                if chosen.len() >= wanted {
                    break;
                }
                let within = (tick % ticks_per_bar) / sixteenth;
                if slots.contains(&within) && !chosen.contains(&within) {
                    chosen.push(within);
                }
            }
        }

        let mut guard = 0;
        while chosen.len() < wanted && guard < slots.len() * 8 {
            guard += 1;
            let slot = slots[rng.random_range(0..slots.len())];
            if !chosen.contains(&slot) {
                chosen.push(slot);
            }
        }

        chosen.sort_unstable();

        let bar_start = bar * ticks_per_bar;
        for (index, slot) in chosen.iter().enumerate() {
            let tick = bar_start + slot * sixteenth;
            // Held to the next onset in the bar, which is what makes a legato
            // line legato without a second parameter deciding lengths.
            let next = chosen
                .get(index + 1)
                .map(|s| bar_start + s * sixteenth)
                .unwrap_or(bar_start + ticks_per_bar);
            onsets.push(Onset {
                tick,
                len: next.saturating_sub(tick).max(sixteenth),
                strong: grid::is_downbeat(*slot, ctx),
            });
        }
    }

    onsets
}

/// The interval classes and their weights, normalised.
fn interval_weights(melody: Option<&Value>) -> [f64; 3] {
    let intervals = block(melody, "intervals");
    let step = intervals
        .and_then(|b| b.get("step"))
        .and_then(Value::as_f64)
        .unwrap_or(0.6);
    let third = intervals
        .and_then(|b| b.get("third"))
        .and_then(Value::as_f64)
        .unwrap_or(0.25);
    let leap = intervals
        .and_then(|b| b.get("leap"))
        .and_then(Value::as_f64)
        .unwrap_or(0.15);

    let total = step + third + leap;
    if total <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    [step / total, third / total, leap / total]
}

/// The contour weights, normalised.
fn contour_weights(melody: Option<&Value>) -> [f64; 3] {
    let contour = block(melody, "contour");
    let riff = contour
        .and_then(|b| b.get("riff"))
        .and_then(Value::as_f64)
        .unwrap_or(0.5);
    let descend = contour
        .and_then(|b| b.get("descend"))
        .and_then(Value::as_f64)
        .unwrap_or(0.3);
    let arch = contour
        .and_then(|b| b.get("arch"))
        .and_then(Value::as_f64)
        .unwrap_or(0.2);

    let total = riff + descend + arch;
    if total <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    [riff / total, descend / total, arch / total]
}

/// The scale degrees a model names as colour, one-based as authored.
fn colour_degrees(melody: Option<&Value>) -> Vec<i32> {
    melody
        .and_then(|b| b.get("colorTones"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_i64)
                .map(|value| value as i32)
                .collect()
        })
        .unwrap_or_default()
}

/// Everything the pitch walk needs that came out of the model.
struct Walk {
    chord_tone_bias: f64,
    half_step: f64,
    octave_jump: f64,
    colour_tones: Vec<i32>,
    intervals: [f64; 3],
    contour_weights: [f64; 3],
    motif: Option<usize>,
}

impl Walk {
    /// One pitch per onset.
    #[allow(clippy::too_many_arguments)]
    fn pitches(
        &self,
        onsets: &[Onset],
        ctx: &SessionContext,
        degrees: &[u8],
        chords: &Chords,
        low: u8,
        high: u8,
        offset: u32,
        rng: &mut impl Rng,
        colour: &mut impl Rng,
    ) -> Vec<u8> {
        let contour = self.contour(rng);

        // Start on a chord tone near the middle of the register, so a phrase
        // has room to move in both directions before it has to fold.
        let mut degree = start_degree(ctx, degrees, chords, low, high, offset);
        let mut pitches = Vec::with_capacity(onsets.len());
        let count = onsets.len().max(1);

        for (index, onset) in onsets.iter().enumerate() {
            if index > 0 {
                let direction = match contour {
                    Contour::Riff => {
                        // Oscillate: away from the pivot, then back.
                        if pitches.len() % 2 == 0 {
                            1
                        } else {
                            -1
                        }
                    }
                    Contour::Descend => -1,
                    Contour::Arch => {
                        if index * 2 < count {
                            1
                        } else {
                            -1
                        }
                    }
                };

                degree += direction * self.interval(rng);

                if self.octave_jump > 0.0 && rng.random_bool(self.octave_jump.clamp(0.0, 1.0)) {
                    degree += if direction > 0 {
                        degrees.len() as i32
                    } else {
                        -(degrees.len() as i32)
                    };
                }
            }

            // Chord-tone bias applies where a wrong note is audible: on the
            // beat. Off the beat the walk is free, which is what makes the line
            // move rather than arpeggiate.
            let mut chosen = degree;
            if onset.strong && colour.random_bool(self.chord_tone_bias.clamp(0.0, 1.0)) {
                if let Some(snapped) =
                    nearest_chord_degree(ctx, degrees, chords, offset + onset.tick, degree)
                {
                    chosen = snapped;
                    degree = snapped;
                }
            } else if !self.colour_tones.is_empty() && colour.random_bool(0.5) {
                // A colour tone is a *scale* degree the model names as fair
                // game off the beat — 2, 4 and 6 in trap. It moves the note
                // inside the key rather than out of it.
                let pick = self.colour_tones[colour.random_range(0..self.colour_tones.len())];
                chosen = nearest_degree_with_class(degrees, degree, pick);
            }

            let mut pitch = pitch_of(ctx, degrees, chosen);

            // The only device allowed out of the scale, and it says so.
            if self.half_step > 0.0 && colour.random_bool(self.half_step.clamp(0.0, 1.0)) {
                pitch += if colour.random_bool(0.5) { 1 } else { -1 };
            }

            let folded = theory::fold_into_register(pitch as i16, low, high)
                .unwrap_or_else(|| low.saturating_add((high - low) / 2));
            pitches.push(folded);
        }

        // A motif is a *pitch* restriction, applied after the walk: rage's two
        // or three notes are the whole hook, and generating a free line and then
        // reducing it keeps the rhythm the model asked for.
        if let Some(count) = self.motif {
            reduce_to_motif(&mut pitches, count);
        }

        pitches
    }

    fn contour(&self, rng: &mut impl Rng) -> Contour {
        let roll = rng.random_range(0.0..1.0);
        if roll < self.contour_weights[0] {
            Contour::Riff
        } else if roll < self.contour_weights[0] + self.contour_weights[1] {
            Contour::Descend
        } else {
            Contour::Arch
        }
    }

    /// How far the next note moves, in scale degrees.
    fn interval(&self, rng: &mut impl Rng) -> i32 {
        let roll = rng.random_range(0.0..1.0);
        if roll < self.intervals[0] {
            1
        } else if roll < self.intervals[0] + self.intervals[1] {
            2
        } else {
            rng.random_range(3..=5)
        }
    }
}

/// The MIDI pitch of a scale degree, before the register is applied.
///
/// Degree 1 is the key root in octave 5 (MIDI 60 for C), which is the middle of
/// every authored melody register — so folding has as little to do as possible.
fn pitch_of(ctx: &SessionContext, degrees: &[u8], degree: i32) -> i32 {
    60 + i32::from(ctx.key_root) + theory::degree_semitone(degrees, degree)
}

/// A degree whose pitch is a chord tone, as close to `from` as possible.
fn nearest_chord_degree(
    ctx: &SessionContext,
    degrees: &[u8],
    chords: &Chords,
    tick: u32,
    from: i32,
) -> Option<i32> {
    let event = chords.at(tick)?;
    // Search outwards so a tie goes to the smaller move, which is what keeps
    // the snap from turning the line into a series of jumps.
    (0..=degrees.len() as i32)
        .flat_map(|distance| [from + distance, from - distance])
        .find(|candidate| {
            let pitch = pitch_of(ctx, degrees, *candidate);
            pitch >= 0 && event.has_tone((pitch % 128) as u8)
        })
}

/// The nearest degree to `from` whose position in the scale is `class`.
///
/// `class` is one-based as the dataset authors it, so `2` is the scale's second
/// degree in whichever octave is closest.
fn nearest_degree_with_class(degrees: &[u8], from: i32, class: i32) -> i32 {
    if degrees.is_empty() {
        return from;
    }
    let count = degrees.len() as i32;
    let wanted = (class - 1).rem_euclid(count);
    let octave = (from - 1).div_euclid(count);
    // Two candidates: the wanted class in this octave and in the next one up,
    // whichever is nearer.
    let here = octave * count + wanted + 1;
    let above = here + count;
    let below = here - count;
    [here, above, below]
        .into_iter()
        .min_by_key(|candidate| (candidate - from).abs())
        .unwrap_or(from)
}

/// Where the phrase starts: a chord tone nearest the register's middle.
fn start_degree(
    ctx: &SessionContext,
    degrees: &[u8],
    chords: &Chords,
    low: u8,
    high: u8,
    offset: u32,
) -> i32 {
    let middle = i32::from(low) + (i32::from(high) - i32::from(low)) / 2;
    let nearest = (-24..=24)
        .min_by_key(|degree| (pitch_of(ctx, degrees, *degree) - middle).abs())
        .unwrap_or(1);
    nearest_chord_degree(ctx, degrees, chords, offset, nearest).unwrap_or(nearest)
}

/// Keep only `count` distinct pitches, mapping the rest onto the nearest kept one.
///
/// Rage's `motifPitchCount` of 2–3. The kept pitches are the ones that occur
/// most, so the hook survives and the ornaments collapse into it.
fn reduce_to_motif(pitches: &mut [u8], count: usize) {
    if count == 0 || pitches.len() <= count {
        return;
    }

    // Frequency, in first-seen order so the result does not depend on hashing.
    let mut tally: Vec<(u8, usize)> = Vec::new();
    for pitch in pitches.iter() {
        match tally.iter_mut().find(|(value, _)| value == pitch) {
            Some((_, seen)) => *seen += 1,
            None => tally.push((*pitch, 1)),
        }
    }
    if tally.len() <= count {
        return;
    }

    // Most frequent first; ties go to the lower pitch so the choice is total.
    tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let keep: Vec<u8> = tally.iter().take(count).map(|(pitch, _)| *pitch).collect();

    for pitch in pitches.iter_mut() {
        if !keep.contains(pitch) {
            if let Some(nearest) = keep
                .iter()
                .min_by_key(|kept| i32::from(**kept).abs_diff(i32::from(*pitch)))
            {
                *pitch = *nearest;
            }
        }
    }
}

/// The answer to a question: the same line, resolved onto the chord's root.
#[allow(clippy::too_many_arguments)]
fn answer(
    base: &[u8],
    onsets: &[Onset],
    ctx: &SessionContext,
    degrees: &[u8],
    chords: &Chords,
    low: u8,
    high: u8,
    offset: u32,
) -> Vec<u8> {
    let mut pitches = base.to_vec();
    let Some(last) = onsets.last() else {
        return pitches;
    };

    // The root of whatever is sounding at *this phrase's* end, in the register.
    // ⛔ The offset matters: without it every phrase resolves onto the chord
    // that happened to be under the first one.
    //
    // ⛔ **The root only while the session's scale has it.** The chord was
    // stacked in `theory::harmonic_degrees`, and a five- or six-note scale
    // sounds over that key with notes missing from it: major pentatonic has no
    // fourth, so `IV`'s root is a pitch this melody may not play. Resolving onto
    // it is the one place this generator left its own key — `kygo`, which
    // authors `major_pentatonic` and `halfStepDissonance: 0`, failed both
    // `melody::every_shipped_melody_stays_in_its_key_and_register` and
    // `coherence` on exactly that note. The next tone of the *same chord* the
    // scale does have is still a resolution onto the harmony; the tonic is the
    // fallback when there is no chord at all. See
    // [`ChordEvent::tones_in_scale`].
    let root = chords
        .at(offset + last.tick)
        .and_then(|event| event.tones_in_scale(ctx).first().copied())
        .unwrap_or(ctx.key_root);
    let resolved = theory::pitch_class_in_register(root, low, high)
        .or_else(|| theory::fold_into_register(pitch_of(ctx, degrees, 1) as i16, low, high));

    if let (Some(resolved), Some(slot)) = (resolved, pitches.last_mut()) {
        *slot = resolved;
    }
    pitches
}

/// The response to a call: the call, an octave down where the register allows.
fn response(base: &[u8], low: u8, high: u8) -> Vec<u8> {
    base.iter()
        .map(|pitch| {
            let down = i32::from(*pitch) - 12;
            theory::fold_into_register(down as i16, low, high).unwrap_or(*pitch)
        })
        .collect()
}

/// `endVariation`: the last phrase ends somewhere else.
///
/// Only the final note moves. A variation that rewrote the phrase would not be
/// a variation of it, and the thing a listener notices at the end of a loop is
/// whether it lands where it landed last time.
#[allow(clippy::too_many_arguments)]
fn vary_ending(
    base: &[u8],
    onsets: &[Onset],
    ctx: &SessionContext,
    degrees: &[u8],
    chords: &Chords,
    low: u8,
    high: u8,
    offset: u32,
) -> Vec<u8> {
    answer(base, onsets, ctx, degrees, chords, low, high, offset)
}

/// A second note under `pitch`, from the chord, for `monophonicBias`.
fn dyad(pitch: u8, chords: &Chords, tick: u32, low: u8, high: u8) -> Option<u8> {
    let event = chords.at(tick)?;
    // A chord tone a third to a sixth below, whichever the chord actually has.
    (3..=9)
        .filter_map(|distance| {
            let candidate = i32::from(pitch) - distance;
            (candidate >= i32::from(low)).then_some(candidate)
        })
        .find(|candidate| event.has_tone((*candidate % 128) as u8))
        .and_then(|candidate| theory::fold_into_register(candidate as i16, low, high))
        .filter(|found| *found != pitch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::chords;
    use serde_json::json;

    /// A model carrying just the blocks these tests need.
    ///
    /// Built through serde rather than a struct literal: `blocks` is
    /// `#[serde(flatten)]`, so the part blocks land there exactly as they do
    /// when the dataset is read — which is the shape the generator is given in
    /// production, not a hand-assembled approximation of it.
    fn model_with(melody: Value) -> StyleModel {
        serde_json::from_value(json!({
            "id": "test",
            "type": "genre",
            "name": "Test",
            "melody": melody,
            "chords": {
                // ⛔ `roman`, not `chords`. The first cut of this fixture used
                // the wrong key, `sample_family` found no numerals, the harmony
                // came back empty and the chord-tone test read a flat 0.00 —
                // which looks exactly like a broken generator.
                "progressionFamilies": [{ "roman": ["i", "VI", "III", "VII"], "weight": 1 }],
                "harmonicRhythm": "1_per_bar",
                "voicing": { "register": [48, 72] }
            }
        }))
        .expect("the test model must parse")
    }

    /// A model with no part blocks at all.
    fn bare_model() -> StyleModel {
        serde_json::from_value(json!({ "id": "bare", "type": "genre", "name": "Bare" }))
            .expect("the bare model must parse")
    }

    fn run(melody: Value, seed: u64) -> (LaneTrack, Chords, SessionContext) {
        let model = model_with(melody);
        let ctx = SessionContext::default();
        let harmony = chords::generate(&model, &ctx, seed);
        let track = generate(&model, &ctx, seed, &harmony, &[]);
        (track, harmony, ctx)
    }

    #[test]
    fn a_model_with_no_melody_block_generates_nothing() {
        // Not an empty-but-present lane full of silence: a part the model never
        // authored has to be distinguishable from one that generated badly.
        let model = bare_model();
        let ctx = SessionContext::default();
        // A model with no `chords` block generates no harmony either, which is
        // exactly the state the melody has to cope with.
        let harmony = chords::generate(&model, &ctx, 1);
        let track = generate(&model, &ctx, 1, &harmony, &[]);
        assert!(track.notes.is_empty());
        assert_eq!(track.lane, Lane::Melody);
    }

    #[test]
    fn every_note_lands_inside_the_authored_register() {
        for seed in 1..40u64 {
            let (track, _, _) = run(
                json!({ "register": [60, 72], "densityPerBar": [4, 6], "halfStepDissonance": 0 }),
                seed,
            );
            for note in &track.notes {
                assert!(
                    (60..=72).contains(&note.pitch),
                    "seed {seed} wrote {} outside 60..=72",
                    note.pitch
                );
            }
        }
    }

    #[test]
    fn a_melody_stays_in_the_key_unless_the_model_asks_to_leave() {
        // ⛔ FR-005's structural claim. Pitches are chosen as scale degrees, so
        // the only way out is `halfStepDissonance` — with it at zero, nothing
        // may be out of the scale.
        let ctx = SessionContext::default();
        let allowed = theory::scale_semitones(ctx.scale);

        for seed in 1..40u64 {
            let (track, _, _) = run(
                json!({
                    "register": [60, 84],
                    "densityPerBar": [5, 8],
                    "halfStepDissonance": 0,
                    "colorTones": [2, 4, 6]
                }),
                seed,
            );

            for note in &track.notes {
                let class = (i32::from(note.pitch) - i32::from(ctx.key_root)).rem_euclid(12) as u8;
                assert!(
                    allowed.contains(&class),
                    "seed {seed}: pitch {} is not in the scale",
                    note.pitch
                );
            }
        }
    }

    #[test]
    fn strong_beats_land_on_chord_tones_at_least_as_often_as_the_bias_asks() {
        // FR-005's acceptance criterion. Measured across seeds rather than one,
        // because a single phrase is far too small a sample to hold a
        // probability to.
        let bias = 0.8;
        let mut strong = 0;
        let mut on_chord = 0;

        for seed in 1..60u64 {
            let (track, harmony, ctx) = run(
                json!({
                    "register": [60, 84],
                    "densityPerBar": [4, 6],
                    "chordToneBias": bias,
                    "halfStepDissonance": 0,
                    "octaveJumpProb": 0
                }),
                seed,
            );

            let ticks_per_beat = grid::ticks_per_beat(&ctx);
            for note in &track.notes {
                if note.start_tick % ticks_per_beat != 0 {
                    continue;
                }
                strong += 1;
                if harmony
                    .at(note.start_tick)
                    .is_some_and(|event| event.has_tone(note.pitch))
                {
                    on_chord += 1;
                }
            }
        }

        assert!(strong > 100, "not enough strong beats sampled: {strong}");
        let ratio = on_chord as f64 / strong as f64;
        assert!(
            ratio >= bias - 0.1,
            "strong beats landed on a chord tone {ratio:.2} of the time, asked for {bias}"
        );
    }

    #[test]
    fn the_same_seed_reproduces_the_same_melody() {
        let spec = json!({ "register": [60, 84], "densityPerBar": [4, 7] });
        let (first, _, _) = run(spec.clone(), 99);
        let (again, _, _) = run(spec, 99);
        assert_eq!(first, again);
    }

    #[test]
    fn a_different_seed_reaches_a_different_melody() {
        // Over a range rather than a pair: neighbouring seeds are entitled to
        // agree, and asserting on two is how `chords.rs` got this wrong first.
        let spec = json!({ "register": [60, 84], "densityPerBar": [4, 7] });
        let first = run(spec.clone(), 1).0;
        let differs = (2..25u64).any(|seed| run(spec.clone(), seed).0 != first);
        assert!(differs, "no seed in 2..25 produced a different melody");
    }

    #[test]
    fn a_motif_count_limits_how_many_distinct_pitches_are_played() {
        // Rage: the hook is two or three notes and repeating them is the point.
        for seed in 1..25u64 {
            let (track, _, _) = run(
                json!({
                    "register": [60, 84],
                    "densityPerBar": [5, 8],
                    "motifPitchCount": 3,
                    "halfStepDissonance": 0,
                    "loopVerbatim": true
                }),
                seed,
            );

            let mut distinct: Vec<u8> = track.notes.iter().map(|n| n.pitch).collect();
            distinct.sort_unstable();
            distinct.dedup();
            assert!(
                distinct.len() <= 3,
                "seed {seed} played {} distinct pitches, motif asked for 3",
                distinct.len()
            );
        }
    }

    #[test]
    fn staccato_shortens_every_note_and_marks_it() {
        let (track, _, _) = run(
            json!({ "register": [60, 84], "densityPerBar": 4, "articulation": "staccato" }),
            7,
        );
        assert!(!track.notes.is_empty());
        for note in &track.notes {
            assert!(
                note.len_ticks <= grid::SIXTEENTH,
                "{} ticks",
                note.len_ticks
            );
            assert_eq!(note.articulation, Some(Articulation::Staccato));
        }
    }

    #[test]
    fn loop_verbatim_repeats_the_phrase_exactly() {
        // The rage rule. Bar-for-bar identical pitches, because the hypnotic
        // repeat is the effect — an end variation would undo it.
        let (track, _, ctx) = run(
            json!({
                "register": [60, 84],
                "densityPerBar": 4,
                "phraseBars": 1,
                "loopVerbatim": true,
                "emptyBarBias": 0,
                "straightEighthsProb": 0
            }),
            11,
        );

        let bar = ctx.ticks_per_bar();
        let first: Vec<(u32, u8)> = track
            .notes
            .iter()
            .filter(|n| n.start_tick < bar)
            .map(|n| (n.start_tick, n.pitch))
            .collect();
        let second: Vec<(u32, u8)> = track
            .notes
            .iter()
            .filter(|n| n.start_tick >= bar && n.start_tick < bar * 2)
            .map(|n| (n.start_tick - bar, n.pitch))
            .collect();

        assert!(!first.is_empty(), "nothing in the first bar");
        assert_eq!(first, second);
    }

    #[test]
    fn an_empty_bar_bias_of_one_leaves_the_bar_empty() {
        // Absent and zero stay different everywhere else in this dataset; a
        // bias of 1 has to mean silence rather than "ignore me".
        let (track, _, _) = run(
            json!({ "register": [60, 84], "densityPerBar": 6, "emptyBarBias": 1.0 }),
            3,
        );
        assert!(
            track.notes.is_empty(),
            "{} notes survived",
            track.notes.len()
        );
    }

    #[test]
    fn mirroring_the_snare_puts_notes_where_the_snare_is() {
        // Drill's device. Without the drums there is nothing to mirror, and the
        // melody must still generate rather than refusing.
        let model = model_with(json!({
            "register": [48, 72],
            "densityPerBar": 3,
            "mirrorSnareDisplacement": true,
            "emptyBarBias": 0,
            "straightEighthsProb": 0
        }));
        let ctx = SessionContext::default();
        let harmony = chords::generate(&model, &ctx, 5);

        let snare_tick = grid::ticks_per_beat(&ctx) * 2 + grid::SIXTEENTH;
        let drums = vec![LaneTrack {
            lane: Lane::Snare,
            notes: vec![Note {
                model_vel: None,
                start_tick: snare_tick,
                len_ticks: grid::SIXTEENTH,
                pitch: 38,
                vel: 100,
                slide_to_pitch: None,
                articulation: None,
                reversed: false,
            }],
        }];

        let with = generate(&model, &ctx, 5, &harmony, &drums);
        assert!(
            with.notes.iter().any(|n| n.start_tick == snare_tick),
            "the melody did not land on the snare's displaced sixteenth"
        );

        let without = generate(&model, &ctx, 5, &harmony, &[]);
        assert!(
            !without.notes.is_empty(),
            "no drums must not mean no melody"
        );
    }

    #[test]
    fn every_authored_structure_and_articulation_name_is_readable() {
        // The vocabulary gate, in miniature — `engine/tests/melody.rs` walks the
        // shipped dataset. A name that does not parse costs the model its
        // phrase shape in silence.
        for name in ["riff_loop", "question_answer", "call_response", "long_arc"] {
            assert!(can_read_structure(name), "{name}");
        }
        assert!(!can_read_structure("spiral"));
        for name in ["staccato", "legato"] {
            assert!(can_read_articulation(name), "{name}");
        }
    }

    #[test]
    fn notes_never_run_past_the_end_of_the_pattern() {
        for seed in 1..30u64 {
            let (track, _, ctx) = run(
                json!({ "register": [60, 84], "densityPerBar": [6, 8], "phraseBars": 4 }),
                seed,
            );
            for note in &track.notes {
                assert!(
                    note.start_tick + note.len_ticks <= ctx.total_ticks(),
                    "seed {seed}: a note runs past the pattern"
                );
                assert!(note.len_ticks > 0);
                assert!(note.vel >= 1);
            }
        }
    }
}
