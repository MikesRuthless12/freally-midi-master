//! The chords generator (FR-004, research ch. 2).
//!
//! Four steps, in this order: sample a progression family, lay it out against
//! the harmonic rhythm, realize each roman numeral in the session's key, then
//! voice it. Each has its own seeded stream, so re-rolling the voicing cannot
//! change which chords were chosen.
//!
//! Chords are **stacked in thirds through the scale**, not built from fixed
//! interval formulas. That is what makes FR-004's acceptance criterion —
//! everything in scale except a modeled borrowing — structural rather than
//! something to check afterwards: the notes come *from* the key, so the only
//! way out of it is a device that says it is leaving. The roman numeral's case
//! is then a correction on top (`V` in a minor key raises the third), which is
//! exactly what a borrowed chord is.
//!
//! Like the drums, everything here writes on the grid. Feel is
//! [`crate::humanize`]'s job and the caller applies it afterwards.

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::grid;
use crate::generators::read::{self, block, flag, number, pair, string_spec_leaning, text};
use crate::pattern::{Lane, LaneTrack, Note};
use crate::rng;
use crate::theory;

/// The velocity every chord note is written at.
///
/// One number rather than a tier table: a pad's dynamics come from the
/// performance, and [`crate::humanize`] applies `velocityVar` over this. A
/// per-voice ramp here would be a second opinion about the same thing.
const CHORD_VELOCITY: u8 = 88;

/// Where a chord sits when the model authors no register.
///
/// C3–C5, the range every authored `voicing.register` in the dataset uses.
const DEFAULT_REGISTER: (u8, u8) = (48, 72);

/// Every parameter under `chords` this generator reads, as dotted paths.
///
/// A prefix covers everything under it: `"borrowed"` covers `borrowed.V`,
/// because the generator walks that map rather than naming its entries.
///
/// This is not documentation — `engine/tests/chords.rs` walks what the models
/// actually author and fails on anything that is neither in this list nor
/// explicitly excused. A parameter nobody reads is decorative and nobody can
/// tell by looking; the dataset has shipped three of those already.
pub const READ_KEYS: &[&str] = &[
    "progressionFamilies",
    "avoid",
    "maxChords",
    "extensions",
    "voicing.style",
    "voicing.register",
    "voicing.topVoiceStepwise",
    "voicing.adjacentSecondProb",
    "harmonicRhythm",
    "chordDurationBeats",
    "susSubstitutionProb",
    "susOrDimProb",
    "borrowed",
];

/// Is this roman numeral one the generator can act on?
///
/// Exposed for the dataset gate: a numeral that fails to parse is dropped, so
/// the chord it named never sounds, and only a test over the shipped data can
/// see that.
pub fn can_read_numeral(text: &str) -> bool {
    Numeral::parse(text).is_some()
}

/// What a chord is beyond its notes — the metadata the melody and the bass
/// read (FR-004: "emits chord symbols as metadata for melody/bass").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordEvent {
    /// The roman numeral as the model authored it, for display and debugging.
    pub roman: String,
    /// Pitch class of the chord's root, 0 = C.
    pub root: u8,
    /// Pitch classes in the chord, root first, in the order they were stacked.
    pub tones: Vec<u8>,
    pub start_tick: u32,
    pub len_ticks: u32,
}

impl ChordEvent {
    /// Is this pitch one of the chord's own tones?
    pub fn has_tone(&self, pitch: u8) -> bool {
        self.tones.contains(&(pitch % 12))
    }

    /// The chord's tones, root first, less any the session's scale cannot play.
    ///
    /// ⛔ **A chord is stacked through [`theory::harmonic_degrees`] — the
    /// seven-note key its numeral is read in — and a five- or six-note scale
    /// sounds *over* that key rather than beside it.** Major pentatonic has no
    /// fourth, so the root of a `IV` is a pitch the session's own scale does not
    /// contain: a part that states the harmony by playing a raw chord tone
    /// leaves the key there, which is what
    /// `coherence::every_arrangement_stays_in_one_key` fails on and what
    /// `kygo` — major-pentatonic, `halfStepDissonance: 0` — proved.
    ///
    /// ⚠ **Only the notes the reduced scale *removed*.** A chord's own
    /// alterations — drill's middle drop, a borrowed `V`'s major third, a `bII`'s
    /// root — sit outside the scale too, and those are the chord saying
    /// something rather than the scale being short of notes. Dropping them would
    /// silence a modelled device, so a tone the seven-note key does not have
    /// either is kept. That also makes this a no-op for every seven-note scale,
    /// where the two tables are the same row.
    pub fn tones_in_scale(&self, ctx: &SessionContext) -> Vec<u8> {
        let scale = theory::scale_semitones(ctx.scale);
        let key = theory::harmonic_degrees(ctx.scale);
        self.tones
            .iter()
            .copied()
            .filter(|class| {
                let relative = (i32::from(*class) - i32::from(ctx.key_root)).rem_euclid(12) as u8;
                scale.contains(&relative) || !key.contains(&relative)
            })
            .collect()
    }
}

/// What the generator produced: the notes, and what they mean.
#[derive(Debug, Clone, PartialEq)]
pub struct Chords {
    pub track: LaneTrack,
    /// In time order, covering the whole pattern with no gaps.
    pub events: Vec<ChordEvent>,
}

impl Chords {
    /// An empty result, for a model that authors no chords at all.
    fn empty() -> Self {
        Self {
            track: LaneTrack {
                lane: Lane::Chords,
                notes: Vec::new(),
            },
            events: Vec::new(),
        }
    }

    /// The chord sounding at a tick, for the melody and bass to write against.
    pub fn at(&self, tick: u32) -> Option<&ChordEvent> {
        self.events
            .iter()
            .find(|event| tick >= event.start_tick && tick < event.start_tick + event.len_ticks)
    }
}

/// How long each chord is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HarmonicRhythm {
    /// One chord for the whole pattern — rage, techno, afrobeats.
    Vamp,
    /// A chord a bar — pop and country.
    OnePerBar,
    /// Two bars a chord — drill and ballads.
    TwoBarsPerChord,
    /// Cells of 3–5 beats, which is what makes trap and plugg *bounce*: the
    /// changes stop lining up with the bar (research ch. 2).
    SyncopatedCell,
}

impl HarmonicRhythm {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "vamp" => Some(Self::Vamp),
            "1_per_bar" => Some(Self::OnePerBar),
            "2_bars_per_chord" => Some(Self::TwoBarsPerChord),
            "syncopated_cell" => Some(Self::SyncopatedCell),
            _ => None,
        }
    }
}

/// A roman numeral, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Numeral {
    /// 1–7 within the scale.
    degree: i32,
    /// A `b` or `#` prefix, in semitones.
    accidental: i32,
    /// Uppercase — the third is major whatever the scale says.
    major: bool,
    /// A `°`/`dim` suffix.
    diminished: bool,
}

impl Numeral {
    /// Parse `"i"`, `"VI"`, `"bII"`, `"vii°"`.
    ///
    /// `None` for anything else. A numeral the table cannot read is an
    /// authoring mistake, and guessing at it would put a chord on a root the
    /// model never asked for — `engine/tests/chords.rs` fails on any authored
    /// numeral that lands here.
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (accidental, rest) = match text.strip_prefix('b') {
            Some(rest) => (-1, rest),
            None => match text.strip_prefix('#') {
                Some(rest) => (1, rest),
                None => (0, text),
            },
        };

        let letters: String = rest
            .chars()
            .take_while(|c| matches!(c, 'i' | 'I' | 'v' | 'V'))
            .collect();
        if letters.is_empty() {
            return None;
        }
        let suffix = &rest[letters.len()..];

        let degree = match letters.to_ascii_lowercase().as_str() {
            "i" => 1,
            "ii" => 2,
            "iii" => 3,
            "iv" => 4,
            "v" => 5,
            "vi" => 6,
            "vii" => 7,
            _ => return None,
        };

        // Mixed case is ambiguous — `Iv` says major and minor at once — so it
        // is rejected rather than resolved by whichever character came first.
        let major = letters.chars().all(|c| c.is_ascii_uppercase());
        if !major && !letters.chars().all(|c| c.is_ascii_lowercase()) {
            return None;
        }

        // Three spellings, all in use: `°` in the textbooks, `dim` in most
        // writing, and `_dim` in the dataset's `avoid` list — JSON keys and
        // ids elsewhere in `data/` are snake_case and that list followed suit.
        let diminished = match suffix {
            "" => false,
            "°" | "o" | "dim" | "_dim" => true,
            _ => return None,
        };

        Some(Self {
            degree,
            accidental,
            major,
            diminished,
        })
    }
}

/// The colour laid over a realized chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    None,
    /// The third replaced by the step above the root.
    Sus2,
    /// The third replaced by the step below the fifth.
    Sus4,
    /// The middle voice dropped a semitone — drill's device (PRD FR-004).
    MiddleDrop,
}

/// Generate the chord part.
///
/// Returns [`Chords::empty`] for a model with no `chords` block: a genre that
/// does not author harmony has none, and inventing a progression for it would
/// put a key under a beat the model never claimed had one.
pub fn generate(model: &StyleModel, ctx: &SessionContext, seed: u64) -> Chords {
    let root = Value::Object(model.blocks.clone().into_iter().collect());
    let Some(chords) = block(Some(&root), "chords").cloned() else {
        return Chords::empty();
    };
    let chords = Some(&chords);

    let family = sample_family(chords, &mut rng::stream(seed, "chords/family"));
    if family.is_empty() {
        return Chords::empty();
    }

    let slots = layout(
        chords,
        ctx,
        family.len(),
        &mut rng::stream(seed, "chords/rhythm"),
    );
    if slots.is_empty() {
        return Chords::empty();
    }

    let mut colour_rng = rng::stream(seed, "chords/colour");
    let mut voicing_rng = rng::stream(seed, "chords/voicing");

    let (low, high) = register(chords);
    let open = text(block(chords, "voicing"), "style") == Some("open");
    let stepwise = flag(block(chords, "voicing"), "topVoiceStepwise", false);
    let adjacent_second = number(
        block(chords, "voicing"),
        "adjacentSecondProb",
        0.0,
        &mut colour_rng,
    );

    let mut notes = Vec::new();
    let mut events = Vec::new();
    let mut previous_top: Option<u8> = None;

    for (index, (start, len)) in slots.iter().enumerate() {
        // A vamp holds one chord, so the family is walked by slot rather than
        // by bar — otherwise a three-chord family over four bars would repeat
        // its first chord instead of coming back round.
        let authored = &family[index % family.len()];
        let Some(numeral) = Numeral::parse(authored) else {
            continue;
        };
        let (numeral, authored) = borrow(&numeral, authored, chords, &mut colour_rng);

        let tones = realize(
            &numeral,
            chords,
            ctx,
            &mut colour_rng,
            adjacent_second > 0.0,
        );
        if tones.is_empty() {
            continue;
        }

        let voiced = voice(
            &tones,
            low,
            high,
            open,
            stepwise.then_some(previous_top).flatten(),
            &mut voicing_rng,
        );
        if voiced.is_empty() {
            continue;
        }
        previous_top = voiced.last().copied();

        // The colour note goes on top of the voicing rather than into the
        // chord's tones: it is a *voicing* decision, and calling it a chord
        // tone would let the melody land on it as if it were consonant.
        let mut voiced = voiced;
        if adjacent_second > 0.0 && voicing_rng.random_bool(adjacent_second.clamp(0.0, 1.0)) {
            if let Some(colour) = adjacent_colour(&voiced, ctx, high) {
                voiced.push(colour);
            }
        }

        for pitch in &voiced {
            notes.push(Note {
                model_vel: None,
                start_tick: *start,
                len_ticks: *len,
                pitch: *pitch,
                vel: CHORD_VELOCITY,
                slide_to_pitch: None,
                articulation: None,
                reversed: false,
            });
        }

        events.push(ChordEvent {
            roman: authored.into_owned(),
            root: (tones[0] % 12) as u8,
            tones: tones.iter().map(|t| (t % 12) as u8).collect(),
            start_tick: *start,
            len_ticks: *len,
        });
    }

    Chords {
        track: LaneTrack {
            lane: Lane::Chords,
            notes,
        },
        events,
    }
}

/// The register a chord is voiced into.
fn register(chords: Option<&Value>) -> (u8, u8) {
    match pair(block(chords, "voicing"), "register") {
        Some((low, high)) if low >= 0.0 && high > low && high <= 127.0 => (low as u8, high as u8),
        _ => DEFAULT_REGISTER,
    }
}

/// Pick a progression by weight, honouring `maxChords`.
fn sample_family(chords: Option<&Value>, rng: &mut impl Rng) -> Vec<String> {
    let families = chords
        .and_then(|c| c.get("progressionFamilies"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if families.is_empty() {
        return Vec::new();
    }

    let weights: Vec<f64> = families
        .iter()
        .map(|f| {
            f.get("weight")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .max(0.0)
        })
        .collect();
    let total: f64 = weights.iter().sum();

    // Every weight zero is an authoring mistake rather than "never play
    // chords"; falling back to the first family keeps the part audible and
    // `datasetc`'s weight-sanity lint is what reports it.
    let chosen = if total <= 0.0 {
        0
    } else {
        let mut point = rng.random_range(0.0..total);
        weights
            .iter()
            .position(|w| {
                point -= w;
                point < 0.0
            })
            .unwrap_or(0)
    };

    let mut roman: Vec<String> = families[chosen]
        .get("roman")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    // `avoid` is a veto, applied after the family is picked rather than by
    // skipping families that contain a vetoed chord: a model that avoids `ii°`
    // and authors one family containing it still wants that family's other
    // chords, and dropping the whole progression would leave it silent.
    let avoided = avoid(chords);
    if !avoided.is_empty() {
        roman.retain(|text| Numeral::parse(text).is_none_or(|numeral| !avoided.contains(&numeral)));
    }

    if let Some(max) = chords
        .and_then(|c| c.get("maxChords"))
        .and_then(Value::as_u64)
    {
        roman.truncate(max.max(1) as usize);
    }
    roman
}

/// Numerals the model refuses to use.
///
/// `_defaults` avoids `ii_dim` for the reason the corpus gives — a diminished
/// ii is the one common-practice chord that pop and hip-hop essentially never
/// use — and every model inherits it.
fn avoid(chords: Option<&Value>) -> Vec<Numeral> {
    chords
        .and_then(|c| c.get("avoid"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter_map(Numeral::parse)
                .collect()
        })
        .unwrap_or_default()
}

/// Where each chord starts and how long it lasts, filling the whole pattern.
fn layout(
    chords: Option<&Value>,
    ctx: &SessionContext,
    family_len: usize,
    rng: &mut impl Rng,
) -> Vec<(u32, u32)> {
    let total = ctx.total_ticks();
    if total == 0 || family_len == 0 {
        return Vec::new();
    }

    // ⛔ **Plain → busy, and this list is the ordering TASK-125 leans along.** A
    // vamp is one chord for the whole clip; a syncopated cell changes two to four
    // times a bar and stops lining up with it. Only values the model authored are
    // reachable — see `read::string_spec_leaning`.
    const BUSYNESS: &[&str] = &["vamp", "2_bars_per_chord", "1_per_bar", "syncopated_cell"];
    let rhythm = string_spec_leaning(
        "chords",
        chords,
        "harmonicRhythm",
        ctx.complexity,
        BUSYNESS,
        rng,
    )
    .as_deref()
    .and_then(HarmonicRhythm::parse)
    .unwrap_or(HarmonicRhythm::OnePerBar);

    let bar = ctx.ticks_per_bar();
    let beat = grid::ticks_per_beat(ctx);

    let mut slots = Vec::new();
    match rhythm {
        HarmonicRhythm::Vamp => slots.push((0, total)),
        HarmonicRhythm::OnePerBar => fill(&mut slots, total, |_| bar),
        HarmonicRhythm::TwoBarsPerChord => fill(&mut slots, total, |_| bar * 2),
        HarmonicRhythm::SyncopatedCell => {
            // `chordDurationBeats` is a range, and each cell is drawn from it
            // independently — a fixed cell length would land back on the bar
            // and stop being syncopated.
            let (min, max) = pair(chords, "chordDurationBeats").unwrap_or((3.0, 5.0));
            let (min, max) = (min.max(1.0) as u32, max.max(1.0) as u32);
            // ⚠ **Inverted, because a SHORTER cell is the busier one**: two beats
            // a chord is busier than eight. Drawn over `u32` — `draw` is generic
            // precisely so this stays a whole-number draw off the same stream the
            // line it replaced used; see its doc for why that matters more than
            // it looks.
            // ⚠ The inversion is declared in `read::LEANING`, not asserted here
            // (TASK-170) — that table is the one place the switch's reach is
            // reviewable.
            let busy = read::leaning(ctx.complexity, "chords", "chordDurationBeats");
            fill(&mut slots, total, |_| beat * busy.draw(min, max, rng));
        }
    }
    slots
}

/// Lay slots end to end until the pattern is full, trimming the last one.
///
/// The trim is what keeps the part inside the pattern: a 5-beat cell starting
/// in the last bar would otherwise write a chord that runs past the loop and
/// gets exported as a note with no end.
fn fill(slots: &mut Vec<(u32, u32)>, total: u32, mut length: impl FnMut(usize) -> u32) {
    let mut at = 0;
    while at < total {
        let len = length(slots.len()).max(1).min(total - at);
        slots.push((at, len));
        at += len;
    }
}

/// Turn a numeral into pitch classes, in stacking order.
fn realize(
    numeral: &Numeral,
    chords: Option<&Value>,
    ctx: &SessionContext,
    rng: &mut impl Rng,
    _colour_ready: bool,
) -> Vec<i32> {
    let degrees = theory::harmonic_degrees(ctx.scale);
    let key = i32::from(ctx.key_root);

    let root = key + theory::degree_semitone(degrees, numeral.degree) + numeral.accidental;

    // Stack thirds *through the scale*: degrees d, d+2, d+4 (+6, +8). The
    // chord is therefore in the key by construction.
    let stacked = |step: i32| {
        key + theory::degree_semitone(degrees, numeral.degree + step) + numeral.accidental
    };
    let mut tones = vec![root, stacked(2), stacked(4)];

    // The numeral's case is a correction on the scale's own answer. When they
    // agree it changes nothing; when they disagree, that disagreement *is* the
    // borrowed chord (a `V` in a minor key).
    let third = tones[1] - tones[0];
    if numeral.major && third == 3 {
        tones[1] += 1;
    } else if !numeral.major && third == 4 && !numeral.diminished {
        tones[1] -= 1;
    }
    if numeral.diminished {
        if tones[1] - tones[0] == 4 {
            tones[1] -= 1;
        }
        if tones[2] - tones[0] == 7 {
            tones[2] -= 1;
        }
    }

    // Extensions, as authored weights over triad/seventh/ninth.
    let extensions = block(chords, "extensions");
    let triad = number(extensions, "triad", 1.0, rng).max(0.0);
    let seventh = number(extensions, "seventh", 0.0, rng).max(0.0);
    let ninth = number(extensions, "ninth", 0.0, rng).max(0.0);
    let total = triad + seventh + ninth;
    if total > 0.0 {
        let point = rng.random_range(0.0..total);
        if point >= triad {
            tones.push(stacked(6));
            if point >= triad + seventh {
                tones.push(stacked(8));
            }
        }
    }

    match colour(chords, rng) {
        Colour::None => {}
        // Sus chords replace the third with a scale step, which is what makes
        // them 0-2-7 and 0-5-7 rather than an arbitrary cluster.
        Colour::Sus2 => tones[1] = stacked(1),
        Colour::Sus4 => tones[1] = stacked(3),
        // Drill's "middle note drop": the third alone falls a semitone, which
        // is a colour rather than a change of chord — the root and fifth are
        // untouched, so the progression still reads as the numeral authored.
        Colour::MiddleDrop => tones[1] -= 1,
    }

    tones
}

/// Swap in a borrowed chord on the degree it borrows from.
///
/// `"borrowed": { "V": 0.1 }` is trap's authoring for the most common
/// borrowing in a minor key (research: "V is the most common borrowed chord in
/// minor"). It applies to whatever the family put on that degree — a family
/// with a `v` in it becomes a family with a `V` in it one time in ten — which
/// is why it is a separate knob from writing `V` into the family itself.
///
/// The accidental has to match as well as the degree: a `bII` and a `II` are
/// different chords, and borrowing one for the other would move the root.
fn borrow<'a>(
    numeral: &Numeral,
    authored: &'a str,
    chords: Option<&Value>,
    rng: &mut impl Rng,
) -> (Numeral, std::borrow::Cow<'a, str>) {
    let Some(borrowed) = chords
        .and_then(|c| c.get("borrowed"))
        .and_then(Value::as_object)
    else {
        return (numeral.clone(), std::borrow::Cow::Borrowed(authored));
    };

    for (symbol, probability) in borrowed {
        let Some(candidate) = Numeral::parse(symbol) else {
            continue;
        };
        if candidate.degree != numeral.degree || candidate.accidental != numeral.accidental {
            continue;
        }
        if candidate == *numeral {
            continue;
        }
        let probability = probability.as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
        if rng.random_bool(probability) {
            return (candidate, std::borrow::Cow::Owned(symbol.clone()));
        }
    }

    (numeral.clone(), std::borrow::Cow::Borrowed(authored))
}

/// Which colour, if any, this chord takes.
fn colour(chords: Option<&Value>, rng: &mut impl Rng) -> Colour {
    // Two spellings in the dataset for two different devices: trap authors
    // `susSubstitutionProb` (sus only), drill authors `susOrDimProb` (sus or
    // the middle-note drop). Both are read; a model authoring neither takes no
    // colour.
    let sus_only = chords
        .and_then(|c| c.get("susSubstitutionProb"))
        .and_then(Value::as_f64);
    let sus_or_dim = chords
        .and_then(|c| c.get("susOrDimProb"))
        .and_then(Value::as_f64);

    let (probability, with_dim) = match (sus_only, sus_or_dim) {
        (Some(p), _) => (p, false),
        (None, Some(p)) => (p, true),
        (None, None) => return Colour::None,
    };

    if !rng.random_bool(probability.clamp(0.0, 1.0)) {
        return Colour::None;
    }
    if with_dim && rng.random_bool(0.5) {
        return Colour::MiddleDrop;
    }
    if rng.random_bool(0.5) {
        Colour::Sus2
    } else {
        Colour::Sus4
    }
}

/// Place the chord's pitch classes in the register.
///
/// `previous_top` asks for the inversion whose top voice moves least from the
/// last chord — `topVoiceStepwise`, which is what stops a progression sounding
/// like four unrelated block chords.
/// How far above the best voice-leading cost a voicing may sit and still be
/// worth reaching for, in semitones.
///
/// ⛔ **This is what stops the harmony saturating (TASK-126).** Taking the strict
/// minimum made a voicing a pure function of the chord, so a model's whole
/// reachable harmony was *families × extension rolls* and nothing else: measured
/// 2026-08-04, `rage` reached **8 distinct progressions in 1,000 seeds** while
/// its melody reached 823. Every other part varies its note content per seed and
/// this one did not.
///
/// **Two semitones rather than "any voicing", because voice leading is the
/// point of the scoring in the first place.** With a previous chord the cost is
/// how far the top voice moves, so a tolerance of 2 keeps the top voice stepwise
/// — which is what `topVoiceStepwise` asks for — while letting the inversion and
/// the octave underneath it move. With no previous chord the cost is the
/// inversion index, so every inversion is in range and the first chord of a
/// progression is free, which is correct: there is nothing yet to lead from.
const VOICING_TOLERANCE: i32 = 2;

fn voice(
    tones: &[i32],
    low: u8,
    high: u8,
    open: bool,
    previous_top: Option<u8>,
    rng: &mut impl Rng,
) -> Vec<u8> {
    // Every voicing that fits, with its cost — rather than the running best, so
    // the choice among the good ones can be made once at the end.
    let mut candidates: Vec<(i32, Vec<u8>)> = Vec::new();

    // An open voicing needs somewhere to drop *to*. Stacking from the
    // register's floor leaves none — the dropped voice would land under it and
    // the chord comes out closed, which is how "open" silently stopped meaning
    // anything. So the open attempt starts an octave up and falls back to the
    // floor when the register is too narrow to hold it there.
    let bases: &[i32] = if open { &[12, 0] } else { &[0] };

    for offset_octave in bases {
        let floor = i32::from(low) + offset_octave;

        for inversion in 0..tones.len() {
            let mut pitches = Vec::with_capacity(tones.len());
            // Stack upwards from the floor, so the chord is never spelled
            // downwards out of range.
            let mut previous = floor - 1;
            let mut fits = true;

            for offset in 0..tones.len() {
                let class = tones[(inversion + offset) % tones.len()].rem_euclid(12);
                let mut pitch = floor + (class - floor.rem_euclid(12)).rem_euclid(12);
                while pitch <= previous {
                    pitch += 12;
                }
                if pitch > i32::from(high) {
                    fits = false;
                    break;
                }
                previous = pitch;
                pitches.push(pitch as u8);
            }

            if !fits || pitches.is_empty() {
                continue;
            }

            if open && pitches.len() >= 3 {
                // Drop-2: the second voice from the top falls an octave, which
                // is what "open" means in every voicing text.
                let index = pitches.len() - 2;
                let dropped = i32::from(pitches[index]) - 12;
                if dropped < i32::from(low) {
                    continue;
                }
                pitches[index] = dropped as u8;
                pitches.sort_unstable();
            }

            let cost = match (previous_top, pitches.last()) {
                (Some(previous), Some(top)) => (i32::from(*top) - i32::from(previous)).abs(),
                // With nothing to move from, root position is the answer, so
                // every later inversion costs more.
                _ => inversion as i32,
            };

            candidates.push((cost, pitches));
        }

        // The higher base worked, so do not also try the floor — that fallback
        // exists for a register too narrow to open, not as a second candidate.
        if !candidates.is_empty() {
            break;
        }
    }

    let Some(best_cost) = candidates.iter().map(|(cost, _)| *cost).min() else {
        return Vec::new();
    };

    // ⚠ **Retain rather than sort.** The candidates are built in inversion
    // order, so keeping them in that order means the chosen index depends only
    // on this chord's own options — a sort would make the pick depend on how
    // ties happened to compare, which is the kind of thing that moves every
    // golden snapshot for no musical reason.
    candidates.retain(|(cost, _)| *cost <= best_cost + VOICING_TOLERANCE);

    let pick = rng.random_range(0..candidates.len());
    candidates.swap_remove(pick).1
}

/// The scale step above the voicing's top note — pluggnb's adjacent-2nd colour
/// (FR-004).
///
/// A *step*, not a semitone: the note has to be in the key or it stops being a
/// colour and becomes a wrong note.
///
/// ⛔ **The key is [`theory::harmonic_degrees`], not [`theory::scale_semitones`],
/// because that is the table [`realize`] stacked the chord out of.** The two are
/// the same row for every seven-note scale and disagree on the reduced ones: the
/// blues scale's flat fifth is not a degree of the natural minor the harmony is
/// built in, so stepping through the *scale* put a colour note on it that was
/// neither a chord tone nor in the key —
/// `chords::every_generated_pitch_is_in_the_key_or_is_a_modeled_alteration`
/// caught it on `bj-the-chicago-kid`. Seven degrees also means the step above is
/// always a second; through a five-note scale it was a third or a fourth.
fn adjacent_colour(voiced: &[u8], ctx: &SessionContext, high: u8) -> Option<u8> {
    let top = *voiced.last()?;
    let degrees = theory::harmonic_degrees(ctx.scale);
    let relative = (i32::from(top) - i32::from(ctx.key_root)).rem_euclid(12);
    let index = degrees.iter().position(|d| i32::from(*d) == relative)?;
    let step = theory::degree_semitone(degrees, index as i32 + 2) - i32::from(degrees[index]);
    let colour = i32::from(top) + step;
    (colour <= i32::from(high) && colour <= 127).then_some(colour as u8)
}

/// Every scale a session can be in, for the tests below.
#[cfg(test)]
use crate::pattern::Scale;

#[cfg(test)]
const ALL_SCALES: [Scale; 12] = [
    Scale::NaturalMinor,
    Scale::HarmonicMinor,
    Scale::Phrygian,
    Scale::PhrygianDominant,
    Scale::Dorian,
    Scale::Major,
    Scale::Mixolydian,
    Scale::Lydian,
    Scale::Aeolian,
    Scale::MinorPentatonic,
    Scale::MajorPentatonic,
    Scale::Blues,
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_numerals_the_dataset_authors_all_parse() {
        for (text, degree, major) in [
            ("i", 1, false),
            ("iv", 4, false),
            ("v", 5, false),
            ("V", 5, true),
            ("VI", 6, true),
            ("III", 3, true),
            ("VII", 7, true),
        ] {
            let parsed = Numeral::parse(text).unwrap_or_else(|| panic!("{text} did not parse"));
            assert_eq!(parsed.degree, degree, "{text}");
            assert_eq!(parsed.major, major, "{text}");
            assert_eq!(parsed.accidental, 0, "{text}");
        }

        let flat_two = Numeral::parse("bII").unwrap();
        assert_eq!(
            (flat_two.degree, flat_two.accidental, flat_two.major),
            (2, -1, true)
        );

        let dim = Numeral::parse("vii°").unwrap();
        assert!(dim.diminished);
    }

    #[test]
    fn a_numeral_the_table_cannot_read_is_refused() {
        // Guessing would put a chord on a root the model never asked for.
        assert!(Numeral::parse("").is_none());
        assert!(Numeral::parse("viii").is_none());
        assert!(Numeral::parse("Iv").is_none(), "mixed case says two things");
        assert!(Numeral::parse("I7").is_none());
        assert!(Numeral::parse("8").is_none());
    }

    #[test]
    fn a_minor_key_realizes_its_diatonic_chords() {
        // C natural minor: i = C-Eb-G, VI = Ab-C-Eb, III = Eb-G-Bb.
        let ctx = SessionContext {
            key_root: 0,
            scale: Scale::NaturalMinor,
            ..Default::default()
        };
        let mut rng = rng::root_stream(1);
        let mut triad = |text: &str| {
            let tones = realize(&Numeral::parse(text).unwrap(), None, &ctx, &mut rng, false);
            tones
                .iter()
                .take(3)
                .map(|t| t.rem_euclid(12))
                .collect::<Vec<_>>()
        };

        assert_eq!(triad("i"), vec![0, 3, 7]);
        assert_eq!(triad("VI"), vec![8, 0, 3]);
        assert_eq!(triad("III"), vec![3, 7, 10]);
        assert_eq!(triad("iv"), vec![5, 8, 0]);
        assert_eq!(triad("VII"), vec![10, 2, 5]);
    }

    #[test]
    fn an_uppercase_numeral_borrows_the_major_third_the_scale_does_not_have() {
        // The borrowed dominant: `v` in C minor is G-Bb-D, `V` is G-B-D. The
        // case is the whole difference, and it is the one note that leaves the
        // key — which is what a borrowed chord *is*.
        let ctx = SessionContext {
            key_root: 0,
            scale: Scale::NaturalMinor,
            ..Default::default()
        };
        let mut rng = rng::root_stream(2);

        let minor = realize(&Numeral::parse("v").unwrap(), None, &ctx, &mut rng, false);
        let major = realize(&Numeral::parse("V").unwrap(), None, &ctx, &mut rng, false);
        assert_eq!(minor[1] - minor[0], 3);
        assert_eq!(major[1] - major[0], 4);
        assert_eq!(major[0], minor[0], "same root, different third");
    }

    #[test]
    fn a_diatonic_chord_is_in_the_scale_in_every_scale() {
        // FR-004's acceptance criterion, structurally: stacking thirds through
        // the scale cannot leave it, so this holds without a filter anywhere.
        for scale in ALL_SCALES {
            let ctx = SessionContext {
                key_root: 7,
                scale,
                ..Default::default()
            };
            let degrees = theory::harmonic_degrees(scale);
            let mut rng = rng::root_stream(3);

            for numeral in ["i", "iv", "VI", "VII", "III"] {
                let tones = realize(
                    &Numeral::parse(numeral).unwrap(),
                    None,
                    &ctx,
                    &mut rng,
                    false,
                );
                for tone in &tones {
                    let relative = (tone - i32::from(ctx.key_root)).rem_euclid(12);
                    // The case correction may raise a third out of the scale —
                    // that is the modeled borrowing, and only the third.
                    let in_scale = degrees.iter().any(|d| i32::from(*d) == relative);
                    let borrowed_third = tones.get(1) == Some(tone);
                    assert!(
                        in_scale || borrowed_third,
                        "{scale:?} {numeral}: {tone} is neither in scale nor the third"
                    );
                }
            }
        }
    }

    #[test]
    fn a_voicing_stays_inside_its_register_and_ascends() {
        // Every candidate, not one draw: the register and the ascent are
        // invariants of *all* the voicings sampling can now reach, so a single
        // call would leave the other two thirds of them unchecked.
        let mut rng = rng::stream(5, "test/voicing");
        for tones in [vec![0, 3, 7], vec![0, 4, 7, 10], vec![2, 5, 9, 0, 4]] {
            for _ in 0..50 {
                let voiced = voice(&tones, 48, 72, false, None, &mut rng);
                assert!(!voiced.is_empty(), "{tones:?} could not be voiced");
                assert!(
                    voiced.iter().all(|p| (48..=72).contains(p)),
                    "{tones:?} -> {voiced:?}"
                );
                assert!(
                    voiced.windows(2).all(|w| w[0] < w[1]),
                    "{tones:?} -> {voiced:?} is not ascending"
                );
            }
        }
    }

    #[test]
    fn a_register_too_narrow_for_the_chord_writes_nothing() {
        // Better than a chord folded on top of itself: the caller drops the
        // slot, and `datasetc`'s register lint is what reports the authoring.
        let mut rng = rng::stream(1, "test/voicing");
        assert!(voice(&[0, 4, 7], 60, 62, false, None, &mut rng).is_empty());
    }

    #[test]
    fn the_top_voice_leads_more_closely_than_an_unled_one() {
        // `topVoiceStepwise`: from a chord topping out at 67, the next chord's
        // top note stays near it rather than snapping to root position.
        //
        // ⚠ **Stated over a run rather than a single draw** (TASK-126). The
        // voicing now samples among the candidates within `VOICING_TOLERANCE`
        // of the best, so any one draw may legitimately be a semitone or two
        // off the nearest — the claim `topVoiceStepwise` actually makes is
        // about where the top voice *tends* to sit, and a single pair of calls
        // was only ever testing the tie-break order.
        let mean_move = |previous: Option<u8>| {
            let mut rng = rng::stream(7, "test/voicing");
            let runs = 200;
            let total: i32 = (0..runs)
                .map(|_| {
                    let voiced = voice(&[5, 9, 0], 48, 72, false, previous, &mut rng);
                    (i32::from(*voiced.last().unwrap()) - 67).abs()
                })
                .sum();
            f64::from(total) / f64::from(runs)
        };

        let led = mean_move(Some(67));
        let free = mean_move(None);
        assert!(
            led < free,
            "a led top voice ({led:.2} semitones) should move less than an unled one ({free:.2})"
        );
    }

    #[test]
    fn an_open_voicing_is_wider_than_a_close_one() {
        // Drop-2 lowers the second voice from the top by an octave, so an open
        // voicing is wider on every draw rather than on average.
        let mut rng = rng::stream(3, "test/voicing");
        let span = |v: &Vec<u8>| i32::from(*v.last().unwrap()) - i32::from(v[0]);
        for _ in 0..50 {
            let close = voice(&[0, 3, 7], 48, 72, false, None, &mut rng);
            let open = voice(&[0, 3, 7], 48, 72, true, None, &mut rng);
            assert!(span(&open) > span(&close), "{open:?} vs {close:?}");
        }
    }

    #[test]
    fn a_vamp_holds_one_chord_and_a_bar_rhythm_changes_every_bar() {
        let ctx = SessionContext::default();
        let mut rng = rng::root_stream(4);

        let vamp = json!({ "harmonicRhythm": "vamp" });
        assert_eq!(
            layout(Some(&vamp), &ctx, 4, &mut rng),
            vec![(0, ctx.total_ticks())]
        );

        let per_bar = json!({ "harmonicRhythm": "1_per_bar" });
        let slots = layout(Some(&per_bar), &ctx, 4, &mut rng);
        assert_eq!(slots.len(), 4);
        assert!(slots.iter().all(|(_, len)| *len == ctx.ticks_per_bar()));
    }

    #[test]
    fn a_syncopated_cell_stops_lining_up_with_the_bar() {
        // The bounce: 3–5 beat cells against a 4-beat bar. If every change
        // landed on a bar line the device would be doing nothing.
        let ctx = SessionContext::default();
        let mut rng = rng::root_stream(5);
        let block = json!({
            "harmonicRhythm": "syncopated_cell",
            "chordDurationBeats": [3, 5]
        });

        let slots = layout(Some(&block), &ctx, 4, &mut rng);
        let bar = ctx.ticks_per_bar();
        assert!(slots.len() > 1);
        assert!(
            slots.iter().any(|(start, _)| !start.is_multiple_of(bar)),
            "no change fell off the bar: {slots:?}"
        );
    }

    #[test]
    fn every_layout_covers_the_pattern_exactly_once() {
        // A gap is a bar with no harmony under it; an overlap is two chords at
        // once. Both are silent failures in a MIDI file.
        let ctx = SessionContext::default();
        for rhythm in ["vamp", "1_per_bar", "2_bars_per_chord", "syncopated_cell"] {
            let block = json!({ "harmonicRhythm": rhythm });
            for seed in 0..8u64 {
                let slots = layout(Some(&block), &ctx, 3, &mut rng::root_stream(seed));
                let mut at = 0;
                for (start, len) in &slots {
                    assert_eq!(*start, at, "{rhythm} seed {seed}: {slots:?}");
                    at += len;
                }
                assert_eq!(at, ctx.total_ticks(), "{rhythm} seed {seed}: {slots:?}");
            }
        }
    }

    #[test]
    fn sus_replaces_the_third_with_a_scale_step() {
        // 0-2-7 and 0-5-7, which is what a sus chord is. Read off the C minor
        // tonic so the numbers are the textbook ones.
        let ctx = SessionContext {
            key_root: 0,
            scale: Scale::NaturalMinor,
            ..Default::default()
        };
        let mut rng = rng::root_stream(6);
        let numeral = Numeral::parse("i").unwrap();
        let chords = json!({ "susSubstitutionProb": 1.0, "extensions": { "triad": 1.0 } });

        let mut sus2 = 0;
        let mut sus4 = 0;
        for seed in 0..40u64 {
            let mut rng = rng::root_stream(seed);
            let tones = realize(&numeral, Some(&chords), &ctx, &mut rng, false);
            let interval = tones[1] - tones[0];
            assert!(
                interval == 2 || interval == 5,
                "got {interval} from {tones:?}"
            );
            if interval == 2 {
                sus2 += 1;
            } else {
                sus4 += 1;
            }
        }
        assert!(sus2 > 0 && sus4 > 0, "both forms should occur");
        let _ = &mut rng;
    }

    #[test]
    fn a_model_with_no_chords_block_writes_nothing() {
        // A genre that does not author harmony has none. Inventing a
        // progression would put a key under a beat that never claimed one.
        let mut model = StyleModel {
            id: "x".into(),
            model_type: crate::dataset::ModelType::Genre,
            name: "X".into(),
            aliases: Vec::new(),
            tier: None,
            extends: Vec::new(),
            era: None,
            genres: Vec::new(),
            related_genres: Vec::new(),
            confidence: None,
            sources: Vec::new(),
            session: None,
            notes: None,
            blocks: Default::default(),
        };
        let chords = generate(&model, &SessionContext::default(), 1);
        assert!(chords.track.notes.is_empty());
        assert!(chords.events.is_empty());

        // ...and an empty family is the same answer, not a panic.
        model
            .blocks
            .insert("chords".into(), json!({ "progressionFamilies": [] }));
        assert!(generate(&model, &SessionContext::default(), 1)
            .events
            .is_empty());
    }

    #[test]
    fn the_chord_at_a_tick_is_the_one_sounding_there() {
        let chords = Chords {
            track: LaneTrack {
                lane: Lane::Chords,
                notes: Vec::new(),
            },
            events: vec![
                ChordEvent {
                    roman: "i".into(),
                    root: 0,
                    tones: vec![0, 3, 7],
                    start_tick: 0,
                    len_ticks: 100,
                },
                ChordEvent {
                    roman: "VI".into(),
                    root: 8,
                    tones: vec![8, 0, 3],
                    start_tick: 100,
                    len_ticks: 100,
                },
            ],
        };

        assert_eq!(chords.at(0).unwrap().roman, "i");
        assert_eq!(chords.at(99).unwrap().roman, "i");
        // The boundary belongs to the chord that starts on it, not the one
        // that ended there.
        assert_eq!(chords.at(100).unwrap().roman, "VI");
        assert!(chords.at(200).is_none());
        assert!(
            chords.at(0).unwrap().has_tone(60),
            "C is in C minor's tonic"
        );
        assert!(!chords.at(0).unwrap().has_tone(61));
    }
}
