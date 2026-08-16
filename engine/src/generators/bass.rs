//! The bassline generator (FR-007, research ch. 2).
//!
//! ⛔ **When the 808 is the bassline, this generates nothing, and that is the
//! feature.** Most trap artists author `drums.bass808.role = "bassline"`: the
//! 808 *is* the bass part, played on the drum kit. Generating a second bass
//! under it would put two instruments on the same notes in the same octave,
//! which is not a fuller low end but a muddier one — the single most common way
//! an arrangement goes wrong. So the parts are unified by one of them staying
//! silent and saying why, rather than by both playing and hoping.
//!
//! Everything else follows the same shape as the other generators: pitches come
//! from the chord and the scale, each device has its own seeded stream, and feel
//! is [`crate::humanize`]'s job afterwards.

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::chords::Chords;
use crate::generators::grid;
use crate::generators::read::{block, number, pair, string_spec_leaning, text};
use crate::pattern::{Lane, LaneTrack, Note};
use crate::rng;
use crate::theory;

/// The velocity a bass note is written at. Below the kick, which it sits under.
const BASS_VELOCITY: u8 = 100;

/// Where the bass sits when the model authors no register. C1–G2.
const DEFAULT_REGISTER: (u8, u8) = (24, 43);

/// Every parameter under `bassline` this generator reads.
pub const READ_KEYS: &[&str] = &[
    "followRootsProb",
    "rhythm",
    "register",
    "glideProb",
    "passingTones",
    "cellShapes",
    "anticipationProb",
];

/// How the bass places its notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rhythm {
    /// One note per kick — the trap and drill default, and why the kick and the
    /// bass read as one instrument played twice.
    MirrorKick,
    /// Its own figure, ignoring the kick. Drill's counter-riff.
    IndependentRiff,
    /// Root on the beat, fifth on the offbeat — country and boom-bap.
    BoomChick,
    /// Every offbeat eighth, which is what makes house and afrobeats walk.
    OffbeatEighths,
    /// One long note per chord, detuned in the sound rather than the notes.
    ReeseSustain,
}

impl Rhythm {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "mirror_kick" => Some(Self::MirrorKick),
            "independent_riff" => Some(Self::IndependentRiff),
            "boom_chick" => Some(Self::BoomChick),
            "offbeat_8ths" => Some(Self::OffbeatEighths),
            "reese_sustain" => Some(Self::ReeseSustain),
            _ => None,
        }
    }
}

/// Is this rhythm name one the generator can act on?
pub fn can_read_rhythm(name: &str) -> bool {
    Rhythm::parse(name).is_some()
}

/// Does this model's bassline take its onsets from the kick?
///
/// ⛔ **The novelty guard's one question about the bass** — see
/// [`crate::novelty::screens`]. `mirror_kick` copies the kick's ticks outright,
/// so rerolling that line to dodge a contour would trade the lock that makes it
/// sit with the drums for a match nobody could hear as a quotation. Every other
/// rhythm places its own onsets and is a figure in its own right, which is what
/// a screen exists for.
///
/// ⚠ **The default is `mirror_kick`**, exactly as [`generate`] reads it, and an
/// unparseable name resolves the same way there. Two answers to "what rhythm is
/// this" is how the guard would come to screen a part the generator built
/// differently.
///
/// ⚠ **A model whose 808 *is* the bass writes no bass part at all**
/// ([`eight_o_eight_is_the_bass`]), so there is nothing to screen: it answers
/// `true` and the guard leaves the empty lane alone rather than drawing four
/// takes of nothing.
pub fn follows_the_kick(model: &StyleModel) -> bool {
    if eight_o_eight_is_the_bass(model) {
        return true;
    }
    let root = Value::Object(model.blocks.clone().into_iter().collect());
    let rhythm = text(block(Some(&root), "bassline"), "rhythm")
        .and_then(Rhythm::parse)
        .unwrap_or(Rhythm::MirrorKick);
    rhythm == Rhythm::MirrorKick
}

/// Is this cell-shape degree one the generator can act on?
///
/// `cellShapes` are authored as scale degrees with optional accidentals —
/// `["1", "b3", "2"]`. A degree that fails to parse would drop that step of the
/// figure, so the cell would play shorter than authored with nothing saying so.
pub fn can_read_cell_degree(text: &str) -> bool {
    cell_degree(text).is_some()
}

/// A cell-shape degree as `(degree, semitone offset)`.
fn cell_degree(text: &str) -> Option<(i32, i32)> {
    let text = text.trim();
    let (accidental, rest) = match text.strip_prefix('b') {
        Some(rest) => (-1, rest),
        None => match text.strip_prefix('#') {
            Some(rest) => (1, rest),
            None => (0, text),
        },
    };
    let degree: i32 = rest.parse().ok()?;
    (degree >= 1).then_some((degree, accidental))
}

/// Does this model's 808 already play the bassline?
///
/// Read from the *drums* block, because that is where the 808 lives. The bass
/// part deferring to it is the whole of FR-007's "unify with the 808 lane".
pub fn eight_o_eight_is_the_bass(model: &StyleModel) -> bool {
    let root = Value::Object(model.blocks.clone().into_iter().collect());
    text(block(block(Some(&root), "drums"), "bass808"), "role") == Some("bassline")
}

pub fn generate(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    chords: &Chords,
    drums: &[LaneTrack],
) -> LaneTrack {
    let empty = LaneTrack {
        lane: Lane::Bass,
        notes: Vec::new(),
    };

    // ⛔ See the module comment: the 808 is already the bassline for most trap
    // artists, and doubling it is a production mistake rather than a fuller
    // sound.
    if eight_o_eight_is_the_bass(model) {
        return empty;
    }

    let root = Value::Object(model.blocks.clone().into_iter().collect());
    let Some(bass) = block(Some(&root), "bassline").cloned() else {
        return empty;
    };
    let bass = Some(&bass);

    let mut param_rng = rng::stream(seed, "bass/params");
    let mut place_rng = rng::stream(seed, "bass/place");
    let mut pitch_rng = rng::stream(seed, "bass/pitch");

    // ⛔ **Plain → busy for TASK-125, and `text` is kept as the fallback.** Almost
    // every model authors one rhythm as a bare string, which `string_spec_leaning`
    // returns untouched; the lean only reaches a model that authored a weighted
    // choice, and then only among the values it listed. A sustained reese is the
    // plainest thing this generator writes and an independent riff the busiest —
    // *"a walking rather than root-following bass"* is the roadmap's phrase for
    // that end.
    const BUSYNESS: &[&str] = &[
        "reese_sustain",
        "mirror_kick",
        "boom_chick",
        "offbeat_8ths",
        "independent_riff",
    ];
    let rhythm = string_spec_leaning(bass, "rhythm", ctx.complexity, BUSYNESS, &mut param_rng)
        .as_deref()
        .and_then(Rhythm::parse)
        .unwrap_or(Rhythm::MirrorKick);
    let follow_roots = number(bass, "followRootsProb", 0.85, &mut param_rng).clamp(0.0, 1.0);
    let glide = number(bass, "glideProb", 0.0, &mut param_rng).clamp(0.0, 1.0);
    let anticipation = number(bass, "anticipationProb", 0.0, &mut param_rng).clamp(0.0, 1.0);

    let (low, high) = match pair(bass, "register") {
        Some((lo, hi)) if lo < hi => (lo.clamp(0.0, 127.0) as u8, hi.clamp(0.0, 127.0) as u8),
        _ => DEFAULT_REGISTER,
    };

    let passing: Vec<i8> = bass
        .and_then(|b| b.get("passingTones"))
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .filter_map(theory::interval_semitones)
                .collect()
        })
        .unwrap_or_default();

    let cells: Vec<Vec<(i32, i32)>> = bass
        .and_then(|b| b.get("cellShapes"))
        .and_then(Value::as_array)
        .map(|shapes| {
            shapes
                .iter()
                .filter_map(Value::as_array)
                .map(|shape| {
                    shape
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(cell_degree)
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();

    let onsets = onsets_for(rhythm, ctx, chords, drums, &mut place_rng);
    if onsets.is_empty() {
        return empty;
    }

    let degrees = theory::scale_semitones(ctx.scale);
    let cell = (!cells.is_empty()).then(|| &cells[place_rng.random_range(0..cells.len())]);

    // Each note carries whether it was *proposed* as a passing tone — the only
    // ones pass two may move. A root or a cell degree is the line's skeleton and
    // stays exactly where the model put it.
    //
    // ⚠ **Carried on the note rather than in a parallel `Vec<bool>`.** The two
    // stayed in step only because both `continue`s below happen to sit above the
    // push; a third early exit added later would have shifted every flag by one,
    // silently.
    let mut written: Vec<(Note, bool)> = Vec::with_capacity(onsets.len());
    for (index, (tick, len)) in onsets.iter().enumerate() {
        let event = chords.at(*tick);
        let chord_root = event.map(|e| e.root).unwrap_or(ctx.key_root);
        let mut passing_slot = false;

        // The blend FR-007 asks for: follow the chord's root, or say something
        // of your own. A cell shape, when the model authors one, is what "your
        // own" means — otherwise it is a passing tone off the root.
        let pitch_class = if pitch_rng.random_bool(follow_roots) {
            i32::from(chord_root)
        } else if let Some(cell) = cell {
            let (degree, accidental) = cell[index % cell.len().max(1)];
            let raw = i32::from(chord_root) + theory::degree_semitone(degrees, degree) + accidental;
            // ⛔ **A cell degree is measured in the KEY's scale from the CHORD's
            // root, so on any chord but the tonic it can land outside the key.**
            // `amapiano` walks `1 3 5` over a VI chord — root 8 plus a dorian
            // third is 11, a tritone from the key — and nothing had ever noticed,
            // because the old sample generated 20 (model, seed) pairs.
            //
            // ▶ A plain degree is a *figure*, not a chromatic device: the author
            // asked for the third of the shape, not for a tritone, so it snaps
            // into the key and keeps the contour. An accidental is the opposite —
            // `b5` is a blue note somebody asked for by name — so it stays and
            // pass two makes it land instead of deleting it.
            if accidental == 0 {
                theory::nearest_scale_tone(raw, ctx.key_root, degrees)
            } else {
                passing_slot = true;
                raw
            }
        } else if !passing.is_empty() {
            // ⚠ **Pass one only proposes.** A passing tone is a fixed interval
            // off the chord root, and whether it *passes through* anything is a
            // fact about the note that follows — which `followRootsProb` has not
            // decided yet. [`resolve_passing_tones`] settles every one of these
            // once the skeleton exists.
            passing_slot = true;
            i32::from(chord_root) + i32::from(passing[index % passing.len()])
        } else {
            i32::from(chord_root)
        };

        // The register is a promise about where the instrument sits, so the
        // pitch class is placed inside it rather than transposed freely — but
        // *which* octave inside it is a choice, not always the bottom one.
        // Octave pops are what bass players do, and a line pinned to the floor
        // of its register is both less musical and less varied: it left
        // `ny-drill` reaching 628 distinct basslines in 1,000 seeds where the
        // others reached 1,000.
        let places = octaves_of(pitch_class.rem_euclid(12) as u8, low, high);
        if places.is_empty() {
            continue;
        }
        let pitch = if places.len() > 1 && pitch_rng.random_bool(0.25) {
            places[pitch_rng.random_range(1..places.len())]
        } else {
            places[0]
        };

        // Anticipation pushes the note *early*, which is what makes drill's bass
        // lean. Never before the pattern starts.
        let start = if anticipation > 0.0 && place_rng.random_bool(anticipation) {
            tick.saturating_sub(grid::SIXTEENTH)
        } else {
            *tick
        };
        // A glide is the 808's own convention: this note slides to the next
        // one's pitch. Recorded on the note so the writer emits the overlap the
        // sampler reads as portamento.
        //
        // ⚠ Nothing is clamped to the pattern here — [`super::fit_to_clip`] is
        // the one place that rule lives now, and it runs on the finished lane.
        written.push((
            Note {
                model_vel: None,
                start_tick: start,
                len_ticks: (*len).max(1),
                pitch,
                vel: BASS_VELOCITY,
                slide_to_pitch: None,
                articulation: None,
                reversed: false,
            },
            passing_slot,
        ));
    }

    // ⛔ **Time order first, because "the note after this one" is a claim about
    // time and not about the order the onsets were visited.** `anticipationProb`
    // pulls a note a 16th early, which can put it *before* the one authored
    // ahead of it — so resolving in index order aimed some approach notes at a
    // neighbour that had already been overtaken. The line is sorted once here
    // and both passes below read it in the order a listener hears it.
    written.sort_by_key(|(note, _)| (note.start_tick, note.pitch));

    // Pass two. Settled once the whole line exists, for the same reason glides
    // are: whether a note passes *through* is a fact about the note after it.
    resolve_passing_tones(&mut written, ctx, degrees, low, high);

    let mut notes: Vec<Note> = written.into_iter().map(|(note, _)| note).collect();

    // Glides are applied after placement, because a glide needs to know what it
    // is gliding *to* — which is the next note, not a parameter.
    if glide > 0.0 {
        let mut glide_rng = rng::stream(seed, "bass/glide");
        for index in 0..notes.len().saturating_sub(1) {
            let next = notes[index + 1].pitch;
            if next != notes[index].pitch && glide_rng.random_bool(glide) {
                notes[index].slide_to_pitch = Some(next);
            }
        }
    }

    // ⚠ Already in time order from the sort above, and nothing since has moved a
    // `start_tick` — pass two rewrites pitches and the glide pass rewrites
    // `slide_to_pitch`. Re-sorting here would only re-break same-tick ties.
    let mut track = LaneTrack {
        lane: Lane::Bass,
        notes,
    };
    super::fit_track_to_clip(&mut track, ctx);
    track
}

/// Pass two: point every chromatic passing tone at the note it precedes.
///
/// ⛔⛔ **A passing tone that passes through nothing is just a wrong note**, and
/// this is the rule the commercial generators encode. A `passingTones` entry is a
/// fixed interval off the *chord* root, and over an ordinary diatonic root that
/// interval routinely leaves the key: `P5` above the supertonic is a semitone
/// outside every minor key, because ii's own fifth is diminished. `_defaults`
/// authors `["P5", "m7"]`, so **every model inherits the exposure**.
///
/// ▶ **Band-in-a-Box and Toontrack's EZbass both aim the chromatic note at its
/// TARGET** — a semitone off the note it is about to reach, so the ear hears it
/// lead somewhere. Scaler and the other scale-locked tools take the opposite
/// route and snap everything into the scale, which would refuse the walked ♭7
/// over a major chord that country and blues bass is made of; `coherence.rs`
/// says in as many words that refusing it would mean refusing the idiom.
///
/// ## Why this is a second pass rather than a decision made in the first
///
/// ⛔ The first attempt aimed each passing tone at the next *chord root* while
/// building the line, and it was wrong: `followRootsProb` decides the next note
/// independently, so the target frequently never arrived and the approach note
/// resolved into another approach note. A note's target is only knowable once
/// the skeleton around it exists.
///
/// So: walk **backwards**, so a run of consecutive passing tones resolves from
/// its landing point outwards. The last one leans into a real chord tone; the
/// ones before it become diatonic and walk toward it, which is what a bass
/// player does with a bar to fill.
///
/// ⚠ **Only slots proposed as passing tones move.** Roots and cell degrees are
/// the line's skeleton, and a generator that "fixes" those is rewriting the
/// model rather than serving it.
///
/// ⚠ **Targets are compared by pitch class**, because `octaves_of` pops an
/// octave a quarter of the time on purpose — a resolution is still a resolution
/// when it lands an octave away.
fn resolve_passing_tones(
    written: &mut [(Note, bool)],
    ctx: &SessionContext,
    degrees: &[u8],
    low: u8,
    high: u8,
) {
    let in_key = |pitch: i32| theory::in_scale(pitch, ctx.key_root, degrees);

    // The pitch of this class that sits nearest `near` without leaving the
    // register — the register is a promise, so a repair may not step outside it.
    let place = |class: i32, near: i32| -> Option<u8> {
        octaves_of(class.rem_euclid(12) as u8, low, high)
            .into_iter()
            .min_by_key(|pitch| (i32::from(*pitch) - near).abs())
    };

    for index in (0..written.len()).rev() {
        if !written[index].1 {
            continue;
        }
        let pitch = i32::from(written[index].0.pitch);
        if in_key(pitch) {
            continue;
        }

        let next = written
            .get(index + 1)
            .map(|(note, _)| i32::from(note.pitch));
        let target = match next {
            // There is something in the key to lean into: approach it by a
            // semitone, from whichever side this note already sat on.
            Some(next_pitch) if in_key(next_pitch) => {
                let below = next_pitch - 1;
                let above = next_pitch + 1;
                if (pitch - below).abs() <= (pitch - above).abs() {
                    below
                } else {
                    above
                }
            }
            // Nothing to lead into — the end of the line, or another chromatic
            // note still waiting its turn. Become diatonic instead.
            //
            // ⛔ **Searched inside the register, not around the pitch.** The
            // approach branch above always has somewhere to land: the target sits
            // between this note and its neighbour, so it is inside the register by
            // construction. This branch has no such guarantee — the nearest scale
            // tone can fall outside `[low, high]` entirely, and `uk-drill` authors
            // a five-semitone register (`[24, 28]`) where that is routine. When it
            // happened, `place` returned `None` and the note was left on its raw
            // chromatic pitch: the one outcome this whole pass exists to prevent.
            _ => (low..=high)
                .map(i32::from)
                .filter(|candidate| in_key(*candidate))
                .min_by_key(|candidate| (candidate - pitch).abs())
                .unwrap_or(pitch),
        };

        if let Some(placed) = place(target, pitch) {
            written[index].0.pitch = placed;
        }
    }
}

/// Every pitch of this class that fits inside the register, lowest first.
///
/// [`theory::pitch_class_in_register`] answers with the lowest, which is right
/// when a part wants a definite pitch and wrong when it is choosing an octave.
fn octaves_of(class: u8, low: u8, high: u8) -> Vec<u8> {
    (low..=high).filter(|pitch| pitch % 12 == class).collect()
}

/// Where the bass plays, as `(tick, length)`.
fn onsets_for(
    rhythm: Rhythm,
    ctx: &SessionContext,
    chords: &Chords,
    drums: &[LaneTrack],
    rng: &mut impl Rng,
) -> Vec<(u32, u32)> {
    let sixteenth = grid::SIXTEENTH;
    let beat = grid::ticks_per_beat(ctx);
    let total = ctx.total_ticks();

    match rhythm {
        // One note per kick. Read off the generated kick rather than
        // reconstructed from the same parameters — two answers to "where is the
        // kick" is how the two parts drift apart.
        Rhythm::MirrorKick => {
            let mut ticks: Vec<u32> = drums
                .iter()
                .filter(|track| track.lane == Lane::Kick)
                .flat_map(|track| track.notes.iter().map(|note| note.start_tick))
                .collect();
            ticks.sort_unstable();
            ticks.dedup();

            // No kick to mirror: fall back to the downbeats rather than going
            // silent, so a model with no drums still has a bass part.
            if ticks.is_empty() {
                ticks = (0..total).step_by(beat as usize).collect();
            }
            held_to_next(ticks, total)
        }

        Rhythm::IndependentRiff => {
            // Its own figure: a note every other sixteenth, jittered by whether
            // this bar leans early or late, so the riff is not a metronome.
            let mut ticks = Vec::new();
            let mut tick = 0;
            while tick < total {
                ticks.push(tick);
                tick += if rng.random_bool(0.5) {
                    sixteenth * 2
                } else {
                    sixteenth * 3
                };
            }
            held_to_next(ticks, total)
        }

        Rhythm::BoomChick => {
            let ticks: Vec<u32> = (0..total).step_by((beat / 2).max(1) as usize).collect();
            held_to_next(ticks, total)
        }

        Rhythm::OffbeatEighths => {
            let ticks: Vec<u32> = (0..total)
                .step_by((beat / 2).max(1) as usize)
                .skip(1)
                .step_by(2)
                .collect();
            held_to_next(ticks, total)
        }

        // One note per chord, held — but a chord that outlasts a phrase takes
        // the note again rather than holding for the whole clip. The detune that
        // makes a reese is a *sound*, not a note, so nothing here doubles it.
        //
        // ⛔⛔ **Held meant one note per chord, and over a vamp that is one note
        // per clip** (TASK-162). `mobb-deep` scored **8 distinct basslines in
        // 1,000 seeds** against a floor of 500; `dmx` 98; `capone-n-noreaga`
        // 208. All three describe a sustained sub in the research and none of
        // them could be authored that way, so every such model ships as
        // `mirror_kick` with a high root lock and records the substitution in its
        // own `notes` — a workaround, not the thing the research describes.
        // `grid::phrase_spans` is the same fix `sustain_pad` needed, and the
        // variety comes from *where* it re-articulates rather than the chord
        // count.
        Rhythm::ReeseSustain => {
            // ⚠ **Through `held_to_next` like the other four arms**, rather than
            // converting each span's end to a length here. The clamp-and-floor
            // rule for "how long is a note that runs to the next boundary"
            // belongs in one place — this file's own note on `fit_to_clip`
            // records what it cost when three call sites each spelled it.
            let mut ticks = Vec::new();
            for event in &chords.events {
                ticks.extend(
                    super::phrase_spans(ctx, event.start_tick, event.len_ticks, rng)
                        .into_iter()
                        .map(|(start, _)| start),
                );
            }
            held_to_next(ticks, total)
        }
    }
}

/// Hold each onset to the next one, so the line is legato by construction.
fn held_to_next(ticks: Vec<u32>, total: u32) -> Vec<(u32, u32)> {
    ticks
        .iter()
        .enumerate()
        .filter(|(_, tick)| **tick < total)
        .map(|(index, tick)| {
            let end = ticks.get(index + 1).copied().unwrap_or(total).min(total);
            (*tick, end.saturating_sub(*tick).max(1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::chords as harmony;
    use serde_json::json;

    fn model_with(bass: Value, drums: Value) -> StyleModel {
        serde_json::from_value(json!({
            "id": "test",
            "type": "genre",
            "name": "Test",
            "bassline": bass,
            "drums": drums,
            "chords": {
                "progressionFamilies": [{ "roman": ["i", "VI", "III", "VII"], "weight": 1 }],
                "harmonicRhythm": "1_per_bar",
                "voicing": { "register": [48, 72] }
            }
        }))
        .expect("the test model must parse")
    }

    fn run(bass: Value, seed: u64) -> (LaneTrack, Chords, SessionContext) {
        let model = model_with(bass, json!({ "kick": { "densityPerBar": 3 } }));
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, seed);
        let kit = crate::generators::drums::generate(&model, &ctx, seed);
        let track = generate(&model, &ctx, seed, &chords, &kit);
        (track, chords, ctx)
    }

    #[test]
    fn a_model_whose_808_is_the_bassline_does_not_also_generate_a_bass() {
        // ⛔ FR-007's unification. Two instruments on the same notes in the same
        // octave is a muddier low end, not a fuller one.
        let model = model_with(
            json!({ "rhythm": "mirror_kick", "register": [24, 43] }),
            json!({ "kick": { "densityPerBar": 3 }, "bass808": { "role": "bassline" } }),
        );
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 1);
        let kit = crate::generators::drums::generate(&model, &ctx, 1);

        assert!(eight_o_eight_is_the_bass(&model));
        assert!(generate(&model, &ctx, 1, &chords, &kit).notes.is_empty());
    }

    #[test]
    fn an_808_playing_something_else_leaves_the_bass_to_generate() {
        let model = model_with(
            json!({ "rhythm": "mirror_kick", "register": [24, 43] }),
            json!({ "kick": { "densityPerBar": 3 }, "bass808": { "role": "counter_riff" } }),
        );
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 1);
        let kit = crate::generators::drums::generate(&model, &ctx, 1);

        assert!(!eight_o_eight_is_the_bass(&model));
        assert!(!generate(&model, &ctx, 1, &chords, &kit).notes.is_empty());
    }

    #[test]
    fn a_model_with_no_bassline_block_generates_nothing() {
        let model: StyleModel =
            serde_json::from_value(json!({ "id": "bare", "type": "genre", "name": "Bare" }))
                .unwrap();
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 1);
        assert!(generate(&model, &ctx, 1, &chords, &[]).notes.is_empty());
    }

    #[test]
    fn mirroring_the_kick_puts_a_bass_note_on_every_kick() {
        // Read off the generated kick, not reconstructed — the two parts must
        // not have separate opinions about where the kick is.
        let model = model_with(
            json!({ "rhythm": "mirror_kick", "register": [24, 43], "followRootsProb": 1.0 }),
            json!({ "kick": { "densityPerBar": 3 } }),
        );
        let ctx = SessionContext::default();
        let chords = harmony::generate(&model, &ctx, 5);
        let kit = crate::generators::drums::generate(&model, &ctx, 5);
        let bass = generate(&model, &ctx, 5, &chords, &kit);

        let kicks: Vec<u32> = kit
            .iter()
            .filter(|t| t.lane == Lane::Kick)
            .flat_map(|t| t.notes.iter().map(|n| n.start_tick))
            .collect();
        assert!(!kicks.is_empty(), "the fixture must generate kicks");

        for kick in kicks {
            assert!(
                bass.notes.iter().any(|n| n.start_tick == kick),
                "no bass note on the kick at {kick}"
            );
        }
    }

    #[test]
    fn every_note_lands_inside_the_authored_register() {
        for seed in 1..25u64 {
            let (bass, _, _) = run(
                json!({ "rhythm": "independent_riff", "register": [17, 31], "glideProb": 0.5 }),
                seed,
            );
            for note in &bass.notes {
                assert!(
                    (17..=31).contains(&note.pitch),
                    "seed {seed}: {} is outside 17..=31",
                    note.pitch
                );
            }
        }
    }

    #[test]
    fn following_the_roots_completely_means_every_note_is_a_chord_root() {
        for seed in 1..20u64 {
            let (bass, chords, _) = run(
                json!({ "rhythm": "boom_chick", "register": [24, 43], "followRootsProb": 1.0 }),
                seed,
            );
            for note in &bass.notes {
                if let Some(event) = chords.at(note.start_tick) {
                    assert_eq!(
                        note.pitch % 12,
                        event.root % 12,
                        "seed {seed}: {} is not the chord root",
                        note.pitch
                    );
                }
            }
        }
    }

    #[test]
    fn a_glide_targets_the_pitch_of_the_note_it_slides_into() {
        let mut glided = 0;
        for seed in 1..30u64 {
            let (bass, _, _) = run(
                json!({
                    "rhythm": "independent_riff",
                    "register": [17, 31],
                    "glideProb": 1.0,
                    "followRootsProb": 0.0,
                    "passingTones": ["P5", "m7"]
                }),
                seed,
            );
            for (index, note) in bass.notes.iter().enumerate() {
                if let Some(target) = note.slide_to_pitch {
                    glided += 1;
                    assert_eq!(
                        Some(target),
                        bass.notes.get(index + 1).map(|n| n.pitch),
                        "a glide must target the next note"
                    );
                }
            }
        }
        assert!(glided > 0, "a glideProb of 1.0 produced no glides at all");
    }

    #[test]
    fn every_rhythm_and_cell_degree_the_dataset_uses_is_readable() {
        for name in [
            "mirror_kick",
            "independent_riff",
            "boom_chick",
            "offbeat_8ths",
            "reese_sustain",
        ] {
            assert!(can_read_rhythm(name), "{name}");
        }
        assert!(!can_read_rhythm("walking"));

        for degree in ["1", "2", "b3", "#4", "b7", "8"] {
            assert!(can_read_cell_degree(degree), "{degree}");
        }
        assert!(!can_read_cell_degree("x"));
        assert!(!can_read_cell_degree("0"));
        assert!(!can_read_cell_degree(""));
    }

    #[test]
    fn a_reese_holds_one_note_per_chord() {
        let (bass, chords, _) = run(
            json!({ "rhythm": "reese_sustain", "register": [24, 43], "followRootsProb": 1.0 }),
            9,
        );
        assert_eq!(bass.notes.len(), chords.events.len());
        for note in &bass.notes {
            assert!(
                note.len_ticks > grid::SIXTEENTH * 4,
                "{} ticks",
                note.len_ticks
            );
        }
    }

    #[test]
    fn the_same_seed_reproduces_the_same_bassline() {
        let spec = json!({ "rhythm": "independent_riff", "register": [17, 31], "glideProb": 0.4 });
        assert_eq!(run(spec.clone(), 42).0, run(spec, 42).0);
    }

    #[test]
    fn notes_never_run_past_the_end_of_the_pattern() {
        for rhythm in [
            "mirror_kick",
            "independent_riff",
            "boom_chick",
            "offbeat_8ths",
            "reese_sustain",
        ] {
            for seed in 1..12u64 {
                let (bass, _, ctx) = run(
                    json!({ "rhythm": rhythm, "register": [24, 43], "anticipationProb": 0.5 }),
                    seed,
                );
                for note in &bass.notes {
                    assert!(
                        note.start_tick + note.len_ticks <= ctx.total_ticks(),
                        "{rhythm} seed {seed}: a note runs past the pattern"
                    );
                    assert!(note.len_ticks > 0 && note.vel >= 1);
                }
            }
        }
    }
}
