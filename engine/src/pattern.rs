//! The note-level data model: what a generator produces and what the MIDI
//! writer consumes (PRD § 3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ticks per quarter note. Fixed at 960 so 16th-note triplets, 32nds and 64th
/// rolls all land on integer ticks — the roll vocabulary is a first-class
/// deliverable and must never be quantised by the tick grid itself.
pub const PPQ: u32 = 960;

/// Seeds cross the IPC boundary as decimal **strings**.
///
/// A `u64` seed exceeds `Number.MAX_SAFE_INTEGER`, and JSON numbers become
/// IEEE-754 doubles in the WebView. Sending one as a number silently rounds it,
/// so the seed chip's "click to copy, paste to reproduce" promise would break
/// for most seeds. Strings are exact. Numbers are still accepted on the way in
/// so hand-written payloads and older sessions keep working.
mod seed_as_string {
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Num(u64),
        }
        match Repr::deserialize(d)? {
            Repr::Num(n) => Ok(n),
            Repr::Str(s) => s.parse().map_err(de::Error::custom),
        }
    }
}

/// The five generated parts. `Drums` covers the whole kit including the 808,
/// which doubles as the bassline lane in trap-family styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum Part {
    Drums,
    Melody,
    Counter,
    Bass,
    Chords,
}

/// The five parts in the order every surface presents them.
///
/// ⛔ **One list, because two would eventually disagree.** The arrangement's
/// clip rows, the multi-track SMF's track order and the timeline's rows all have
/// to be the same sequence — a song whose rows read one way and whose exported
/// tracks read another looks wrong in the DAW and is nobody's obvious bug. This
/// is deliberately *not* `Part`'s declaration order, which puts Chords last.
pub const PART_ORDER: [Part; 5] = [
    Part::Drums,
    Part::Chords,
    Part::Melody,
    Part::Counter,
    Part::Bass,
];

/// A voice within a pattern. Drum parts use several; melodic parts use one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum Lane {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
    Rim,
    Snap,
    Perc,
    Bass808,
    Melody,
    Counter,
    Bass,
    Chords,
}

/// Performance marking. Carries intent the raw velocity cannot: a ghost note
/// and a quiet main hit are the same number but not the same musical event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum Articulation {
    Ghost,
    Accent,
    Legato,
    Staccato,
    Roll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum Scale {
    // ---- Modes of the major scale --------------------------------------
    Major,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    NaturalMinor,
    Locrian,
    /// The natural minor under its modal name. See [`scale_semitones`] — the
    /// dataset uses both spellings and they must not resolve differently.
    Aeolian,

    // ---- Pentatonic and blues ------------------------------------------
    MajorPentatonic,
    MinorPentatonic,
    MajorBlues,
    /// The minor blues, under the bare name the dataset already authors.
    Blues,

    // ---- Minor and major variants --------------------------------------
    HarmonicMinor,
    MelodicMinor,
    HarmonicMajor,

    // ---- Modes of those ------------------------------------------------
    PhrygianDominant,
    DorianSharp4,
    LydianAugmented,
    LydianDominant,
    SuperLocrian,
    LocrianNatural6,
    IonianSharp5,
    Ultralocrian,

    // ---- Symmetric ------------------------------------------------------
    WholeTone,
    WholeHalfDiminished,
    HalfWholeDiminished,
    Chromatic,

    // ---- World ----------------------------------------------------------
    HungarianMinor,
    EightToneSpanish,
    Bhairav,
    Hirajoshi,
    InSen,
    Iwato,
    Kumoi,
    PelogSelisir,
    PelogTembung,

    // ---- Messiaen modes 3–7 ---------------------------------------------
    MessiaenMode3,
    MessiaenMode4,
    MessiaenMode5,
    MessiaenMode6,
    MessiaenMode7,
}

impl Scale {
    /// Every scale, so a gate can never be written against a subset.
    ///
    /// ⛔ **Hand-maintained, and the compiler is what keeps it honest.**
    /// `theory::scale_semitones` and `theory::scale_character` are both
    /// exhaustive matches, so a variant added to the enum fails to compile until
    /// it has an interval set *and* a character — and whoever is already in both
    /// of those files is one line from this one. The list itself only widens
    /// what the tests cover, so a forgotten entry weakens a gate rather than
    /// breaking the engine, which is why a build-time trick is not worth the
    /// dependency it would cost.
    pub const ALL: [Scale; 41] = [
        Scale::Major,
        Scale::Dorian,
        Scale::Phrygian,
        Scale::Lydian,
        Scale::Mixolydian,
        Scale::NaturalMinor,
        Scale::Locrian,
        Scale::Aeolian,
        Scale::MajorPentatonic,
        Scale::MinorPentatonic,
        Scale::MajorBlues,
        Scale::Blues,
        Scale::HarmonicMinor,
        Scale::MelodicMinor,
        Scale::HarmonicMajor,
        Scale::PhrygianDominant,
        Scale::DorianSharp4,
        Scale::LydianAugmented,
        Scale::LydianDominant,
        Scale::SuperLocrian,
        Scale::LocrianNatural6,
        Scale::IonianSharp5,
        Scale::Ultralocrian,
        Scale::WholeTone,
        Scale::WholeHalfDiminished,
        Scale::HalfWholeDiminished,
        Scale::Chromatic,
        Scale::HungarianMinor,
        Scale::EightToneSpanish,
        Scale::Bhairav,
        Scale::Hirajoshi,
        Scale::InSen,
        Scale::Iwato,
        Scale::Kumoi,
        Scale::PelogSelisir,
        Scale::PelogTembung,
        Scale::MessiaenMode3,
        Scale::MessiaenMode4,
        Scale::MessiaenMode5,
        Scale::MessiaenMode6,
        Scale::MessiaenMode7,
    ];
}

/// How a scale *feels*, for narrowing the picker by mood (TASK-041C).
///
/// ⛔ **A property of the scale, not of the mood.** Mike's rule is that "dark
/// moods should only show dark scales" — and the only way that stays true as
/// scales are added is for each scale to declare its own character once, here,
/// with a test that every one of them has. The alternative, listing scales on
/// every mode, restates the same information in thirty-odd places and drifts.
///
/// `Neutral` is a real answer rather than a fallback: the symmetric scales and
/// the Messiaen modes have no tonic third to be major or minor about, and
/// `uptempo`/`minimal` are tempo and density statements rather than emotional
/// ones, so they inherit the model's full list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum ScaleCharacter {
    Dark,
    Neutral,
    Bright,
}

/// A span of a clip, in absolute ticks (TASK-041E).
///
/// Used for both the loop brace and the clip's own start and end, which are
/// deliberately two different things: a producer loops bar 2 of a four-bar idea
/// to work on it, and trims the clip to bars 1–3 to keep it. One field for both
/// would make either gesture destroy the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Region {
    pub from_tick: u32,
    pub to_tick: u32,
}

impl Region {
    /// The region as a usable span, or `None` if it is empty or inverted.
    ///
    /// ⛔ Checked rather than trusted. A brace dragged past its own other end
    /// arrives here inverted, and a transport that looped from a later tick to
    /// an earlier one would emit nothing and read as playback having broken.
    pub fn valid(self) -> Option<(u32, u32)> {
        (self.to_tick > self.from_tick).then_some((self.from_tick, self.to_tick))
    }
}

/// A single note event. Ticks are absolute from the start of the pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Note {
    pub start_tick: u32,
    pub len_ticks: u32,
    /// MIDI note number, 0–127.
    pub pitch: u8,
    /// MIDI velocity, 1–127.
    pub vel: u8,
    /// What the model itself wrote here, before [`crate::humanize`] spread it
    /// (TASK-041V).
    ///
    /// ⛔ **The velocity lane's "reset" needs this and cannot recompute it.**
    /// Resetting a cap has to put back the value that note's own lane tier
    /// asked for — a ghost note's 40, an accent's 120 — and by the time anyone
    /// can see the lane, `vary` has already spread it by a random factor that
    /// is not invertible. Resetting to a flat 100 instead would quietly delete
    /// the accent pattern that is the difference between a played pattern and a
    /// programmed one, using the control whose whole promise is "put it back".
    ///
    /// `None` on a note nobody generated — one the producer drew — where there
    /// is no model opinion to return to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_vel: Option<u8>,
    /// 808 slide target. The note glides to this pitch; the writer emits the
    /// overlap convention the sampler reads as portamento.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_to_pitch: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub articulation: Option<Articulation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct LaneTrack {
    pub lane: Lane,
    pub notes: Vec<Note>,
}

/// One generated clip for one part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Pattern {
    pub id: String,
    pub part: Part,
    /// The style model this came from — an artist or a genre archetype.
    pub artist_id: String,
    #[serde(with = "seed_as_string")]
    #[ts(type = "string")]
    pub seed: u64,
    pub bars: u16,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    /// Pitch class of the key root, 0 = C.
    pub key_root: u8,
    pub scale: Scale,
    pub lanes: Vec<LaneTrack>,
    pub ppq: u32,
    /// The mode this was generated in, when the model offers any (TASK-040V).
    ///
    /// Carried on the pattern because a mood picked by the seed is otherwise
    /// invisible: "Any" has to be able to say *which* one it landed on, for the
    /// same reason the seed box echoes back the seed it used. `None` means the
    /// model offers no modes, not that one was declined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood: Option<String>,
    /// What the transport repeats, in ticks (TASK-041E).
    ///
    /// `None` is the whole clip, which is what every pattern is until someone
    /// drags a brace. Carried on the pattern rather than held beside it because
    /// it is a property of the clip: it saves with the project, travels with a
    /// preset, and an edited clip that forgot its own loop on reload would be
    /// the kind of quiet loss nobody attributes to the right feature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_region: Option<Region>,
    /// The clip's own start and end, independent of the loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_region: Option<Region>,
}

impl Pattern {
    /// Whether a note is inside the clip's own start and end (TASK-041E).
    ///
    /// ⛔ **The trim is honoured by everything that reads the clip, or it is a
    /// lie.** The markers were draggable, saved with the project and read by
    /// nothing: a producer trimmed a clip to bars 1–3, and the transport still
    /// played four bars while the export still wrote four. A boundary that only
    /// moves two marks on screen is worse than no boundary at all.
    ///
    /// A note is kept if it *starts* inside the region. Judging by its end would
    /// drop a held note the producer can see sounding across the boundary, which
    /// is the opposite of what trimming the end of a clip means.
    pub fn within_clip(&self, note: &Note) -> bool {
        match self.clip_region.and_then(Region::valid) {
            None => true,
            Some((from, to)) => note.start_tick >= from && note.start_tick < to,
        }
    }

    /// Last tick occupied by any note. `0` for an empty pattern.
    pub fn end_tick(&self) -> u32 {
        self.lanes
            .iter()
            .flat_map(|l| l.notes.iter())
            .map(|n| n.start_tick + n.len_ticks)
            .max()
            .unwrap_or(0)
    }

    pub fn note_count(&self) -> usize {
        self.lanes.iter().map(|l| l.notes.len()).sum()
    }

    /// This clip's meter, with the values a project file can carry normalised.
    ///
    /// See [`normalise_meter`] for why the fallback lives in one place.
    pub fn time_sig(&self) -> (u8, u8) {
        normalise_meter(self.time_sig_num, self.time_sig_den)
    }

    /// How many ticks one bar of this clip's meter is.
    pub fn ticks_per_bar(&self) -> u32 {
        ticks_per_bar_of(self.time_sig_num, self.time_sig_den)
    }
}

/// A meter with the values a project file can actually carry made safe.
///
/// ⛔ **One place for this, because it had been written out four times and the
/// comments at each one asserted they agreed.** A zero denominator is not
/// hypothetical — the meter is deserialized from someone's project and every
/// caller then *divides* by it — and a zero numerator gives a bar of no beats.
pub fn normalise_meter(num: u8, den: u8) -> (u8, u8) {
    (num.max(1), if den == 0 { 4 } else { den })
}

/// Ticks in one bar of `num`/`den`.
///
/// ⛔⛔ **A free function precisely because the four callers are four different
/// types.** `Pattern`, `Song` and `SessionContext` each need this and none of
/// them can inherit a method from another, which is exactly how the formula came
/// to be copied — the copies' own comments say they "must agree or the file says
/// one thing and the tick arithmetic another", which is a claim no compiler was
/// checking. Now they delegate and it is checked by construction.
pub fn ticks_per_bar_of(num: u8, den: u8) -> u32 {
    let (num, den) = normalise_meter(num, den);
    (PPQ * 4 / u32::from(den)).max(1) * u32::from(num)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum SectionKind {
    Intro,
    Verse,
    /// The lift into a chorus.
    ///
    /// ⛔ **Added because the data needs it, not to round the list out.**
    /// `style-research.md` ch. 1 states pop's form as `V-PC-C-V2-PC-C-B-C`
    /// outright, and there is no honest way to spell that in the other five —
    /// writing a pre-chorus as a bridge would put the section that *builds into*
    /// the chorus where the section that departs from it goes, and the drop-out
    /// rule keys off what follows.
    PreChorus,
    Hook,
    Bridge,
    Outro,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct PatternRef {
    pub pattern_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Section {
    #[serde(rename = "type")]
    pub kind: SectionKind,
    pub start_bar: u32,
    pub bars: u16,
    /// One pattern per part present in this section.
    pub patterns: BTreeMap<Part, PatternRef>,
    /// Beats of silence at the end of this section (TASK-066).
    ///
    /// The drop-out before a hook: the track cuts out for the last beat or two
    /// so the hook lands. It is a property of *where the section sits* rather
    /// than of its notes, because the clip underneath it loops — putting the
    /// silence in the pattern would drop a beat out of every repeat instead of
    /// once, at the end.
    #[serde(default)]
    pub drop_out_beats: u8,
    /// Whether this section fades across its length (TASK-066).
    ///
    /// Same reasoning: a decay that lived in the clip would reset on every
    /// repeat, which is a stutter rather than an outro.
    #[serde(default)]
    pub decay: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
}

/// A full arrangement — what Song Mode produces and what the multi-track SMF
/// export walks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Song {
    pub id: String,
    pub artist_id: String,
    #[serde(with = "seed_as_string")]
    #[ts(type = "string")]
    pub seed: u64,
    pub bpm: f32,
    pub key_root: u8,
    pub scale: Scale,
    pub sections: Vec<Section>,
    /// Time signature, carried so a song reads without its `SessionContext`.
    #[serde(default = "four")]
    pub time_sig_num: u8,
    #[serde(default = "four")]
    pub time_sig_den: u8,
    /// Every pattern the sections name, by [`PatternRef::pattern_id`].
    ///
    /// ⛔ **A `PatternRef` is an id, and until this existed nothing anywhere
    /// held what the id named** — a `Song` described which pattern played where
    /// and could not answer what any of them was, so it could not be drawn,
    /// exported or played.
    ///
    /// A store rather than a `Pattern` inline in each section, because sharing
    /// is the point: verse 1 and verse 2 are the same beat, and in these genres
    /// that is the rule rather than an optimisation. One entry per distinct
    /// pattern is also what makes a re-roll of one section a change to one
    /// section — the UI repoints that section's ref instead of editing a
    /// pattern two sections are looking at.
    #[serde(default)]
    pub patterns: BTreeMap<String, Pattern>,
    pub ppq: u32,
}

fn four() -> u8 {
    4
}

/// How quiet a decaying section gets by its last bar, as a share of the
/// velocity it started at.
///
/// Not zero: an outro that reaches silence before it ends leaves the producer
/// dragging in bars of nothing, and every DAW draws that as the clip being
/// broken rather than as a fade.
pub const DECAY_FLOOR: f32 = 0.35;

/// How one section lays its clip out in time.
///
/// ⛔ **One implementation, because this arithmetic has been wrong twice and
/// both times it shipped.** The exporter tiles a song into a MIDI file and the
/// transport tiles the same song into a schedule to play; when those were two
/// walks over the same fields they were free to disagree, and what a producer
/// hears would not be what they exported. The two bugs were the tiling guard
/// dropping note-ons while writing their offs, and the decay ramp measured in
/// whole clip repeats — which is zero for a section exactly one clip long, the
/// shape the shipped data actually produces.
///
/// Everything is in ticks. `repeats` is a **count**, deliberately: written as
/// `while offset < sounding` with `offset += clip_len` the loop is a state
/// machine over `k · clip_len mod 2^32`, and release builds have no overflow
/// checks — for a long enough section over a long enough clip it orbits past
/// `sounding` forever without reaching it, pinning the thread the host draws
/// its editor from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionTiling {
    /// Where the section begins on the song's timeline.
    pub section_start: u32,
    /// How long the section runs, whatever the clip does.
    pub section_len: u32,
    /// How long one repeat of the clip is. Never zero.
    pub clip_len: u32,
    /// How much of the section actually sounds — the drop-out is taken off the
    /// end (TASK-066).
    pub sounding: u32,
    /// How many times the clip is laid down.
    pub repeats: u32,
    /// Whether the section fades across its length.
    pub decay: bool,
}

impl SectionTiling {
    /// Work out where `clip` falls inside `section`.
    pub fn of(song: &Song, section: &Section, clip_bars: u16) -> Self {
        let ticks_per_bar = song.ticks_per_bar();
        let beat_ticks = (ticks_per_bar / u32::from(song.time_sig_num.max(1))).max(1);

        // ⛔ **Saturating, because these ticks are untrusted.** A `Song` reaches
        // here from the webview — a project file somebody else saved, or
        // devtools — and this workspace sets `panic = "abort"`, so an
        // overflow-checked build turns an arithmetic wrap into the *host*
        // process dying. In the shipped release profile it wraps silently and
        // places the notes at the wrong ticks instead, which is a corrupted
        // export nobody can explain.
        let section_start = section.start_bar.saturating_mul(ticks_per_bar);
        let section_len = u32::from(section.bars).saturating_mul(ticks_per_bar);
        let clip_len = u32::from(clip_bars).saturating_mul(ticks_per_bar).max(1);
        let sounding = section_len
            .saturating_sub(u32::from(section.drop_out_beats).saturating_mul(beat_ticks));

        Self {
            section_start,
            section_len,
            clip_len,
            sounding,
            repeats: sounding.div_ceil(clip_len),
            decay: section.decay,
        }
    }

    /// Where repeat `repeat` starts, relative to the section.
    pub fn offset(&self, repeat: u32) -> u32 {
        repeat.saturating_mul(self.clip_len)
    }

    /// Whether a note whose onset is `origin` within the clip sounds at all.
    ///
    /// ⛔ **Judged on the *onset*, so both ends of a note are kept or dropped
    /// together.** Testing each event's own tick dropped a note-on past the end
    /// of a section and wrote its note-off anyway; the orphan landed inside the
    /// next section, on the same channel and key, where a DAW paired it with
    /// that section's own note and cut it dead. A note that starts inside and
    /// rings past the end is allowed to finish, exactly as one does at a clip
    /// boundary.
    pub fn sounds(&self, repeat: u32, origin: u32) -> bool {
        self.offset(repeat).saturating_add(origin) < self.sounding
    }

    /// The velocity of `velocity` at `tick` within repeat `repeat`.
    ///
    /// ⛔ **Measured across the *section*, not per whole clip repeat.**
    /// `offset / clip_len` is zero for the whole of a section exactly one clip
    /// long — which is what the shipped data produces, since `_defaults`
    /// authors a 4-bar outro and the bars chip defaults to 4 — so every
    /// decaying outro came out dead flat while the timeline drew the badge.
    pub fn velocity(&self, repeat: u32, tick: u32, velocity: u8) -> u8 {
        if !self.decay {
            return velocity;
        }
        let position = self.offset(repeat).saturating_add(tick) as f32;
        let through = (position / self.section_len.max(1) as f32).clamp(0.0, 1.0);
        let scale = 1.0 - (1.0 - DECAY_FLOOR) * through;
        ((f32::from(velocity) * scale).round() as u8).clamp(1, 127)
    }
}

impl Song {
    /// Total length in bars.
    /// Total length in bars.
    ///
    /// ⚠ **Saturating, like every other tick sum over these fields.** Both
    /// operands come off the wire on the song path, and `check_song` caps them
    /// long before this is reached today — but that guard lives at the bridge
    /// and this is a method on the type. `flatten_parts` is now a caller, and a
    /// future one that does not cross the bridge would be an overflow panic in
    /// a debug-assertions build, which with `panic = "abort"` is the host
    /// process dying rather than one plugin misbehaving.
    pub fn total_bars(&self) -> u32 {
        self.sections
            .iter()
            .map(|s| s.start_bar.saturating_add(u32::from(s.bars)))
            .max()
            .unwrap_or(0)
    }

    /// Ticks in one bar at this song's meter.
    ///
    /// ⛔ Mirrors [`crate::SessionContext::ticks_per_bar`] exactly, including the
    /// zero-denominator fallback to 4 — that comment says the two "must agree or
    /// the file says one thing and the tick arithmetic another", and the SMF
    /// writer reads this one.
    pub fn ticks_per_bar(&self) -> u32 {
        ticks_per_bar_of(self.time_sig_num, self.time_sig_den)
    }

    /// The pattern a section's reference names, if the store holds it.
    pub fn pattern(&self, reference: &PatternRef) -> Option<&Pattern> {
        self.patterns.get(&reference.pattern_id)
    }

    /// References naming no pattern in the store.
    ///
    /// A dangling ref is a section that draws as empty and exports as silence,
    /// with nothing anywhere saying why. `arrange` cannot produce one and the
    /// gate below proves it; this exists so a *loaded* song — an older project
    /// file, a hand-edited one — is checked rather than trusted.
    pub fn dangling_refs(&self) -> Vec<String> {
        let mut missing: Vec<String> = self
            .sections
            .iter()
            .flat_map(|section| section.patterns.values())
            .filter(|reference| !self.patterns.contains_key(&reference.pattern_id))
            .map(|reference| reference.pattern_id.clone())
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    /// The whole arrangement as one clip, laid out on the song's timeline
    /// (TASK-072).
    ///
    /// ⛔ **This is what makes a song playable at all, and it is a flattening
    /// rather than a new scheduler.** The transport arms a [`Pattern`] — that is
    /// the only thing it knows how to place, seek within and report a position
    /// through — and until this existed the plugin had no notion of playing a
    /// `Song`. Teaching the audio thread about sections instead would have meant
    /// a second tiling implementation on the thread least able to afford being
    /// wrong, and the exporter's tiling has already been wrong twice.
    ///
    /// So the sections are collapsed here, on the UI thread, through the same
    /// [`SectionTiling`] the exporter reads. **What is heard and what is
    /// exported come out of one piece of arithmetic**, which is the property
    /// this codebase's own rule about `drop_out_beats` and `decay` asks for: a
    /// field the export honours and playback ignores is the same failure as a
    /// field the export ignores.
    ///
    /// ⚠ Lanes are preserved rather than folded into channels — the export
    /// needs channels, and the preview sampler and the per-lane mutes need
    /// lanes. That is the one place the two paths legitimately differ.
    pub fn flatten(&self) -> Pattern {
        self.flatten_parts(None)
    }

    /// The arrangement as one clip, with only `parts` playing.
    ///
    /// ⚠ **`None` is every part; a list is an *audition* filter and nothing
    /// more.** Muting or soloing a row in the timeline is "let me hear this
    /// without the melody", not an edit — the song is unchanged and the export
    /// is unchanged, which is the same distinction the per-lane audio mute
    /// already draws and labels *preview* on screen. A filter that silently
    /// changed the file would be a much worse control.
    pub fn flatten_parts(&self, parts: Option<&[Part]>) -> Pattern {
        // Lane order is the order lanes are first met, so a flattened song draws
        // and mutes in the same order the sections do.
        let mut lanes: Vec<LaneTrack> = Vec::new();

        for section in &self.sections {
            for (part, reference) in &section.patterns {
                if parts.is_some_and(|keep| !keep.contains(part)) {
                    continue;
                }
                let Some(clip) = self.pattern(reference) else {
                    // A dangling reference is silence here, exactly as it is in
                    // the export. `dangling_refs` is what reports it; inventing
                    // a substitute would hide the problem behind sound.
                    continue;
                };
                let tiling = SectionTiling::of(self, section, clip.bars);

                for track in &clip.lanes {
                    let slot = match lanes.iter().position(|l| l.lane == track.lane) {
                        Some(index) => index,
                        None => {
                            lanes.push(LaneTrack {
                                lane: track.lane,
                                notes: Vec::new(),
                            });
                            lanes.len() - 1
                        }
                    };
                    for repeat in 0..tiling.repeats {
                        let offset = tiling.offset(repeat);
                        for note in &track.notes {
                            if !clip.within_clip(note) || !tiling.sounds(repeat, note.start_tick) {
                                continue;
                            }
                            let vel = tiling.velocity(repeat, note.start_tick, note.vel);
                            lanes[slot].notes.push(Note {
                                start_tick: tiling
                                    .section_start
                                    .saturating_add(offset)
                                    .saturating_add(note.start_tick),
                                vel,
                                ..*note
                            });
                        }
                    }
                }
            }
        }

        for track in &mut lanes {
            track
                .notes
                .sort_by_key(|note| (note.start_tick, note.pitch));
        }

        Pattern {
            id: format!("{}-flat", self.id),
            // ⛔ **The part is carried through when the filter names exactly
            // one, and only falls back to a stand-in for a true whole-song
            // flatten.** A song is every part at once and `Part` has no name for
            // that — but `pattern_to_smf` writes its track name from this field,
            // so a fabricated `Drums` put `trap — Drums` *inside* every file
            // called `FMM Melody.mid`. The name on disk and the name in the file
            // contradicting each other is the failure this codebase writes down
            // in capitals; the whole-song case never reaches the writer.
            part: match parts {
                Some([only]) => *only,
                _ => Part::Drums,
            },
            artist_id: self.artist_id.clone(),
            seed: self.seed,
            // The whole song, so `Schedule::progress` is a position through the
            // arrangement rather than through whichever clip is playing.
            bars: u16::try_from(self.total_bars()).unwrap_or(u16::MAX),
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
            time_sig_den: self.time_sig_den,
            key_root: self.key_root,
            scale: self.scale,
            lanes,
            ppq: self.ppq,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    /// The span one section occupies in the flattened clip, in ticks
    /// (TASK-072's loop-section toggle).
    pub fn section_span(&self, index: usize) -> Option<Region> {
        let section = self.sections.get(index)?;
        let ticks_per_bar = self.ticks_per_bar();
        Some(Region {
            from_tick: section.start_bar.saturating_mul(ticks_per_bar),
            to_tick: section
                .start_bar
                .saturating_add(u32::from(section.bars))
                .saturating_mul(ticks_per_bar),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pattern(seed: u64) -> Pattern {
        Pattern {
            loop_region: None,
            clip_region: None,
            id: "p1".into(),
            part: Part::Drums,
            artist_id: "osamason".into(),
            seed,
            bars: 4,
            bpm: 150.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 6,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane: Lane::Bass808,
                notes: vec![
                    Note {
                        model_vel: None,
                        start_tick: 0,
                        len_ticks: PPQ,
                        pitch: 30,
                        vel: 110,
                        slide_to_pitch: Some(35),
                        articulation: Some(Articulation::Legato),
                    },
                    Note {
                        model_vel: None,
                        start_tick: PPQ * 2,
                        len_ticks: PPQ / 2,
                        pitch: 30,
                        vel: 90,
                        slide_to_pitch: None,
                        articulation: None,
                    },
                ],
            }],
            ppq: PPQ,
            mood: None,
        }
    }

    #[test]
    fn pattern_roundtrips_through_json() {
        let original = sample_pattern(12345);
        let json = serde_json::to_string(&original).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn a_seed_beyond_javascripts_safe_integer_survives_the_roundtrip() {
        // 2^53 + 1 is the first integer a JS number cannot represent. If the
        // seed were serialized as a number this would come back as 2^53.
        let seed = 9_007_199_254_740_993_u64;
        let json = serde_json::to_string(&sample_pattern(seed)).unwrap();
        assert!(
            json.contains("\"seed\":\"9007199254740993\""),
            "seed must be a JSON string, got: {json}"
        );
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seed, seed);
    }

    #[test]
    fn u64_max_survives_the_roundtrip() {
        let json = serde_json::to_string(&sample_pattern(u64::MAX)).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seed, u64::MAX);
    }

    #[test]
    fn a_numeric_seed_is_still_accepted_on_the_way_in() {
        let json = serde_json::to_string(&sample_pattern(7)).unwrap();
        let with_number = json.replace("\"seed\":\"7\"", "\"seed\":7");
        let back: Pattern = serde_json::from_str(&with_number).unwrap();
        assert_eq!(back.seed, 7);
    }

    #[test]
    fn optional_note_fields_are_omitted_when_absent() {
        let json = serde_json::to_string(&sample_pattern(1)).unwrap();
        // The first note has both; the second has neither.
        assert_eq!(json.matches("slideToPitch").count(), 1);
        assert_eq!(json.matches("articulation").count(), 1);
    }

    #[test]
    fn field_names_reach_json_as_camel_case() {
        let json = serde_json::to_string(&sample_pattern(1)).unwrap();
        for key in ["startTick", "lenTicks", "artistId", "timeSigNum", "keyRoot"] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert!(!json.contains("start_tick"));
    }

    #[test]
    fn end_tick_and_note_count_read_across_lanes() {
        let p = sample_pattern(1);
        assert_eq!(p.note_count(), 2);
        assert_eq!(p.end_tick(), PPQ * 2 + PPQ / 2);
    }

    #[test]
    fn an_empty_pattern_has_no_end_tick() {
        let mut p = sample_pattern(1);
        p.lanes.clear();
        assert_eq!(p.end_tick(), 0);
        assert_eq!(p.note_count(), 0);
    }

    #[test]
    fn section_kind_serializes_under_the_key_type() {
        let section = Section {
            kind: SectionKind::Hook,
            start_bar: 16,
            bars: 8,
            patterns: BTreeMap::from([(
                Part::Drums,
                PatternRef {
                    pattern_id: "p1".into(),
                },
            )]),
            drop_out_beats: 0,
            decay: false,
            markers: vec![],
        };
        let json = serde_json::to_string(&section).unwrap();
        assert!(json.contains("\"type\":\"hook\""), "got {json}");
        // Empty markers stay out of the payload.
        assert!(!json.contains("markers"));
        let back: Section = serde_json::from_str(&json).unwrap();
        assert_eq!(section, back);
    }

    #[test]
    fn song_roundtrips_and_reports_its_length() {
        let song = Song {
            id: "s1".into(),
            artist_id: "osamason".into(),
            seed: u64::MAX,
            bpm: 150.0,
            key_root: 6,
            scale: Scale::Phrygian,
            sections: vec![
                Section {
                    kind: SectionKind::Intro,
                    start_bar: 0,
                    bars: 8,
                    patterns: BTreeMap::new(),
                    drop_out_beats: 0,
                    decay: false,
                    markers: vec!["drop".into()],
                },
                Section {
                    kind: SectionKind::Hook,
                    start_bar: 8,
                    bars: 16,
                    patterns: BTreeMap::from([(
                        Part::Melody,
                        PatternRef {
                            pattern_id: "p2".into(),
                        },
                    )]),
                    drop_out_beats: 0,
                    decay: false,
                    markers: vec![],
                },
            ],
            time_sig_num: 4,
            time_sig_den: 4,
            patterns: BTreeMap::from([("p2".to_owned(), sample_pattern(1))]),
            ppq: PPQ,
        };
        assert_eq!(song.total_bars(), 24);
        let back: Song = serde_json::from_str(&serde_json::to_string(&song).unwrap()).unwrap();
        assert_eq!(song, back);
    }

    #[test]
    fn a_reference_resolves_through_the_store_and_a_missing_one_is_named() {
        // The failure this is here to catch draws as an empty section and
        // exports as silence, so it has to be reportable rather than merely
        // absent.
        let hook = Section {
            kind: SectionKind::Hook,
            start_bar: 0,
            bars: 8,
            patterns: BTreeMap::from([
                (
                    Part::Melody,
                    PatternRef {
                        pattern_id: "held".into(),
                    },
                ),
                (
                    Part::Drums,
                    PatternRef {
                        pattern_id: "gone".into(),
                    },
                ),
            ]),
            drop_out_beats: 0,
            decay: false,
            markers: vec![],
        };
        let song = Song {
            id: "s1".into(),
            artist_id: "osamason".into(),
            seed: 1,
            bpm: 150.0,
            key_root: 6,
            scale: Scale::Phrygian,
            sections: vec![hook.clone()],
            time_sig_num: 4,
            time_sig_den: 4,
            patterns: BTreeMap::from([("held".to_owned(), sample_pattern(2))]),
            ppq: PPQ,
        };

        assert_eq!(
            song.pattern(&hook.patterns[&Part::Melody]),
            Some(&sample_pattern(2))
        );
        assert_eq!(song.pattern(&hook.patterns[&Part::Drums]), None);
        assert_eq!(song.dangling_refs(), vec!["gone".to_owned()]);
    }

    #[test]
    fn ppq_divides_every_subdivision_the_roll_vocabulary_needs() {
        // 16ths, 16th triplets, 32nds, 32nd triplets and 64ths must all land on
        // whole ticks, or rolls drift against the grid.
        for div in [4, 6, 8, 12, 16, 24] {
            assert_eq!(PPQ % div, 0, "PPQ {PPQ} is not divisible by {div}");
        }
    }
}
