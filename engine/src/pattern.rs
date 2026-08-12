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
    /// The second, lower kick layer — boom bap's "2-kick layering standard"
    /// (research ch. 1 §8) and phonk's 808-doubled kick.
    ///
    /// ⛔ **Not [`Lane::Sub`].** The sub kick is an unpitched *drum* that
    /// reinforces the kick's low end; `Sub` is the pitched, sliding 808 that
    /// plays a bassline. Sharing a lane would make a country kit's kick layer
    /// follow the session key.
    SubKick,
    Snare,
    /// A second snare voice on the off-beats, authored per model.
    ///
    /// Its own lane rather than extra hits in [`Lane::Snare`], on Mike's call:
    /// he named it alongside claps, and a clap is a lane. Being separate is
    /// also what makes it worth anything to `drum_variety` — a moved hit in an
    /// existing lane is not a different beat, but a different voice is.
    OffSnare,
    /// The quiet answering snare on the e/a slots, at 20–40% in boom bap and
    /// drill (research ch. 1 §§2, 8).
    ///
    /// ⚠ **A lane rather than an [`Articulation::Ghost`] on [`Lane::Snare`],
    /// and the two coexist.** The articulation says *how* a note is played and
    /// is what the humanizer's velocity tiers read; this says *which pad*, so a
    /// producer can put a different sample under their ghosts — which is the
    /// whole reason TASK-043A asks for it.
    GhostSnare,
    Clap,
    ClosedHat,
    OpenHat,
    /// The hat closed with the foot: shorter and duller than a stick-struck
    /// closed hat, and the third voice of a real hi-hat.
    PedalHat,
    Ride,
    /// The ride struck on its bell — a pitched ping rather than a wash.
    RideBell,
    Crash,
    /// The **mid** tom. Named `Tom` since before there were three, and
    /// deliberately left alone: every model that authors `"tom"` today means
    /// this one, and renaming a data key to tidy an enum is how a genre loses
    /// its percussion in silence.
    Tom,
    TomHigh,
    TomLow,
    Rim,
    Snap,
    Perc,
    /// A second generic percussion voice, for the models that layer two.
    Perc2,
    Shaker,
    Tambourine,
    Cowbell,
    Clave,
    Conga,
    Bongo,
    Timbale,
    Triangle,
    Woodblock,
    /// ── FX ──────────────────────────────────────────────────────────────
    ///
    /// ⚠ **Lanes, not effects.** They are triggered like any other pad and
    /// carry no processing of their own — a riser is a sample that rises. What
    /// makes them worth their own lanes is that a producer routes and replaces
    /// them separately from the kit.
    Riser,
    Impact,
    Reverse,
    /// The pitched, sliding sub-bass — **not** the bass drum, which is
    /// [`Lane::Kick`].
    ///
    /// Named for the role rather than the machine, on Mike's call: "you can
    /// have a 606, a 707, an 808, or a 909". The dataset still authors it as
    /// `drums.bass808` and that key is deliberately untouched — renaming a data
    /// key would mean revisiting every model, which is exactly what TASK-140 is
    /// sequenced before the roster to avoid.
    ///
    /// ⛔ **The alias is load-bearing.** `Lane` serializes *by name*, and
    /// `PluginSession.muted_lanes` is a `Vec<Lane>`, so without it every saved
    /// project with a muted 808 fails to open.
    #[serde(alias = "bass808")]
    Sub,
    /// The 808's own sub layer — a sine under the distorted one, which is how
    /// every phonk and drill 808 is actually built (research ch. 1 §7).
    SubLow,
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
    /// Play this one note's sample **backwards** (2026-08-11).
    ///
    /// ⛔⛔ **Mike:** *"if you have a drum pad or something playing forward in
    /// the pad, but you want it to play backwards in the drum pattern, you should
    /// be able to switch it … select the note and press like 'Ctrl+R' or
    /// 'Command+R' on macOS to reverse the note just for that single note being
    /// played. i think this would be a VERY USABLE AND COOL FEATURE to have."*
    ///
    /// ⛔ **Per NOTE, and deliberately not the same thing as a reversed
    /// one-shot.** A pad can be *assigned* a reversed sample (`Ctrl`+← in the
    /// browser, stored in `PluginSession::one_shots_reversed`), and that flips the
    /// buffer once at load time because it never changes. This is the opposite
    /// case: the same pad sounding forwards on one hit and backwards on the next
    /// inside one pattern, so it can only be read per voice, at trigger time.
    ///
    /// ⛔ **A `.mid` cannot carry it.** There is no SMF representation of "play
    /// this sample backwards", so a dragged or exported MIDI clip loses it and
    /// only the audio keeps it. That is a real limit of the format rather than
    /// something to work around, and it is written here so the next reader does
    /// not go looking for the encoding.
    ///
    /// ⚠ `#[serde(default)]` and skipped when false, so every pattern already on
    /// disk — and every golden snapshot — deserializes unchanged and gains
    /// nothing in its serialized form.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
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
    /// **The take's seed.** What rerolls on every press of Generate.
    #[serde(with = "seed_as_string")]
    #[ts(type = "string")]
    pub seed: u64,
    /// **The record's seed** — the harmonic plan every part is written against
    /// (TASK-141).
    ///
    /// ⛔ **The page has to hand this back on the next Generate**, or the five
    /// parts stop agreeing. That is the whole mechanism: the take rerolls, the
    /// record is carried. A caller that ignores it gets the pre-TASK-141
    /// behaviour, where Generate on Drums and then Generate on Melody drew two
    /// unrelated seeds and wrote the melody against a harmony the chords tab
    /// had never seen.
    ///
    /// ⚠ Defaults to [`Self::seed`] when absent, so a project saved before this
    /// existed reopens as one seed for everything — which is exactly what it
    /// meant.
    #[serde(with = "seed_as_string", default)]
    #[ts(type = "string")]
    pub song_seed: u64,
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

    /// Several parts as one clip, so a single schedule can sound them together
    /// (TASK-127).
    ///
    /// ⛔ **One schedule holds one `Pattern`, and that is the whole reason this
    /// exists.** `arm_pattern` took a single clip and echoed it back, so Play
    /// could only ever sound the part on the visible tab — Mike, 2026-08-06:
    /// *"you should be able to play one generator at a time by toggling it on or
    /// off, or play all generators together at the same time if you want to."*
    /// Toggling is the caller's job: it passes the parts that are on, and one of
    /// them is the ordinary "solo" case.
    ///
    /// ⚠ **The first clip's timing wins**, because a schedule has one tempo and
    /// one meter. Parts are generated one at a time and a producer can change
    /// the tempo between two of them, so the alternative is refusing to play a
    /// perfectly ordinary session. ▶ Ticks *are* reconciled — a clip written at a
    /// different `ppq` is rebased rather than played at the wrong speed, which
    /// would be silent corruption rather than a visible refusal.
    ///
    /// ⚠ `clip_region` is applied here and not carried: the trim says which
    /// notes exist, so a merged clip that kept it would trim twice, once against
    /// its own bar 1. `loop_region` is a transport instruction about the result
    /// and belongs to whoever arms it.
    pub fn merge(parts: &[Pattern]) -> Option<Pattern> {
        let first = parts.first()?;
        if parts.len() == 1 {
            return Some(first.clone());
        }
        // A zero would divide below. `normalise_meter`'s reasoning, applied to
        // the other field a project file can carry a nonsense value in.
        let ppq = first.ppq.max(1);

        // Lane order is the order lanes are first met, matching `flatten_parts`
        // so a merged clip draws and mutes the way an arranged one does.
        let mut lanes: Vec<LaneTrack> = Vec::new();
        for clip in parts {
            let from = clip.ppq.max(1);
            let rebase = |tick: u32| {
                if from == ppq {
                    tick
                } else {
                    u32::try_from(u64::from(tick) * u64::from(ppq) / u64::from(from))
                        .unwrap_or(u32::MAX)
                }
            };
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
                for note in track.notes.iter().filter(|note| clip.within_clip(note)) {
                    lanes[slot].notes.push(Note {
                        start_tick: rebase(note.start_tick),
                        // ⚠ Rebased too. Scaling the start and not the length
                        // would stretch or crush every note against the grid.
                        len_ticks: rebase(note.len_ticks).max(1),
                        ..*note
                    });
                }
            }
        }

        for track in &mut lanes {
            track
                .notes
                .sort_by_key(|note| (note.start_tick, note.pitch));
        }

        Some(Pattern {
            id: format!("{}-merged", first.id),
            // ⛔ **A stand-in, and `flatten_parts` records why in full**: this
            // field names the track `pattern_to_smf` writes, so a fabricated
            // part puts the wrong name *inside* a correctly named file. Merged
            // clips are armed for playback and never written, and the one-part
            // case returned above keeps its own name.
            part: first.part,
            artist_id: first.artist_id.clone(),
            seed: first.seed,
            song_seed: first.song_seed,
            // The longest part, so a short one does not truncate the record.
            bars: parts.iter().map(|p| p.bars).max().unwrap_or(first.bars),
            bpm: first.bpm,
            time_sig_num: first.time_sig_num,
            time_sig_den: first.time_sig_den,
            key_root: first.key_root,
            scale: first.scale,
            lanes,
            ppq,
            mood: None,
            // ⛔ **Carried from the first clip, not dropped.** This was `None`,
            // and nothing downstream put it back: the solo path returns the
            // clip untouched and keeps its brace, while `session.ts` sends the
            // parts and `editor.rs` arms the reply verbatim. So a dragged loop
            // brace worked with one generator on and was **silently ignored the
            // moment a second one was switched on** — which is the default.
            // `voice.rs::a_dragged_brace_still_wins_over_the_button` asserts
            // the brace wins and could not, because it never reached the
            // schedule in the merged case.
            //
            // ⚠ First-clip-wins, which is the rule this function already
            // applies to tempo, meter and key for the reason stated above: the
            // brace is drawn against a timeline, and the timeline is the first
            // clip's.
            // ⛔ **The first clip that HAS one, not the first clip.** `first` is
            // whatever `armedClips` listed first — always Drums, because that is
            // GENERATED_PARTS order — while the brace is dragged onto whichever
            // tab is open. So a brace drawn on the Melody roll was dropped, which
            // is every case except drawing it on Drums.
            loop_region: parts.iter().find_map(|clip| clip.loop_region),
            clip_region: None,
        })
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
    /// How many bars of the clip this row plays before it repeats (TASK-142).
    ///
    /// ⛔ **This is what "resize a clip" means in an arrangement, and it had to
    /// live on the *reference* rather than on the pattern.** Sections of the
    /// same kind share one [`Pattern`] — that is [`crate::arrange`]'s second
    /// stated decision, and the whole reason this is an id — so shortening the
    /// pattern would shorten every verse in the song at once. On the reference
    /// it is what a producer means: *this* row, in *this* section, loops on two
    /// bars instead of four.
    ///
    /// ⚠ **`None` is the pattern's own length**, which is what every song built
    /// before this field meant and what a fresh arrangement still means. It is
    /// not "zero" and not "one bar": a default here would silently retile every
    /// saved project on the first reopen.
    ///
    /// ⛔ Read only through [`SectionTiling::of`], which is the one place the
    /// exporter and the transport both go — see that type's own note on why
    /// two walks over these fields is a bug this project has already shipped
    /// twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<u16>,
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
    ///
    /// ⛔ **Takes the reference as well as the pattern, so "how long is one
    /// repeat" is decided here and nowhere else.** It used to take a bare
    /// `clip_bars: u16`, which meant each of the two callers spelled the answer
    /// out — and once [`PatternRef::bars`] existed (TASK-142) that would have
    /// been two places to remember it. This type's own header records what
    /// happens when the exporter and the transport each walk these fields: they
    /// disagreed twice, and both times what a producer heard was not what they
    /// exported.
    ///
    /// ⚠ A resize of `0` is refused rather than honoured. `repeats` is
    /// `div_ceil(clip_len)` and `clip_len` is floored at 1, so a zero would not
    /// divide by zero — it would lay the clip down `sounding` times, which for a
    /// sixteen-bar section is 61,440 copies of every note.
    ///
    /// ⚠ **There is deliberately no `of_bars(…, clip_bars: u16)` beside this.**
    /// The first cut added one "for the tests" and nothing ever called it — a
    /// second `pub` way to resolve a tiling that *skips* [`PatternRef::bars`],
    /// which is precisely the two-walks-over-one-set-of-fields hazard this
    /// type's header records having shipped twice. If a test ever genuinely
    /// needs a bare bar count, make it private then.
    pub fn of(song: &Song, section: &Section, reference: &PatternRef, clip: &Pattern) -> Self {
        let clip_bars = reference.bars.filter(|bars| *bars > 0).unwrap_or(clip.bars);
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

    /// How long a note that starts at `origin` may ring before the loop turns
    /// over.
    ///
    /// ⛔⛔ **A resized clip must not ring into its own next repeat.** `sounds`
    /// keeps or drops a note by its *onset*, which is right — the two halves of
    /// one note have to be kept or dropped together — but it says nothing about
    /// length. So with `clip_len` shortened by TASK-142's resize, a note longer
    /// than the new loop kept its full length: repeat 0's two-bar pad was still
    /// two bars long while repeat 1 had already re-struck the same pitch on the
    /// same channel a bar earlier, and a DAW pairs that stale note-off with the
    /// live note and cuts it dead. The last repeat's tail could land in the
    /// *next section* and kill its note of the same pitch. That is precisely the
    /// orphan-note-off failure [`Self::sounds`]'s own note says the design
    /// exists to prevent, reopened by making `clip_len` shrinkable.
    ///
    /// ⚠ **A no-op for a clip nobody resized.** `clip_len` is then the pattern's
    /// own length and every generator already clamps its notes inside that, so
    /// nothing an unresized song plays or exports changes. It is also what a DAW
    /// does with a loop brace: the loop point cuts the note.
    ///
    /// ⚠ Floors at 1, because a zero-length note is not a rest — it is a note
    /// event some hosts drop and others hold forever.
    pub fn held_within(&self, origin: u32, len: u32) -> u32 {
        self.clip_len.saturating_sub(origin).min(len).max(1)
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
        // ⛔⛔ **A note past the end of one repeat does not sound, and this line
        // is what makes TASK-142's clip resize mean anything.** Every repeat
        // lays the *whole* clip down at `offset`, so with `clip_len` shortened
        // below the pattern's own length the copies overlap: a four-bar clip
        // looped on two bars played bars 3 and 4 on top of the next repeat's
        // bars 1 and 2. Measured on the fixture — 14 notes where 8 were asked
        // for, at ticks nothing in the arrangement lines up with.
        //
        // ⚠ **A no-op when nothing has been resized**, which is why it can be
        // added here rather than guarded: `clip_len` is then the pattern's own
        // length and every note it holds is inside it by construction.
        if origin >= self.clip_len {
            return false;
        }
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
                let tiling = SectionTiling::of(self, section, reference, clip);

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
                                // ⛔ Trimmed at the loop point — see
                                // `SectionTiling::held_within`. Without it a
                                // resized clip's long note rings into the next
                                // repeat, where the same pitch has already been
                                // re-struck.
                                len_ticks: tiling.held_within(note.start_tick, note.len_ticks),
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
            // A song is generated from one seed and `render_section` shares it
            // across every part, which is the coherent case the two-seed
            // design exists to reach for single patterns. Song Mode was
            // already there, so the record and the take are the same value.
            song_seed: self.seed,
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

    #[test]
    fn a_project_saved_before_the_sub_rename_still_opens() {
        // ⛔ `Lane` serializes **by name**, and `PluginSession.muted_lanes` is a
        // `Vec<Lane>`, so without the alias every saved project with a muted 808
        // would fail to open — silently, on a producer's own session.
        //
        // ⚠ This test exists because `cargo clippy` prints "failed to parse
        // serde attribute" for the alias. That is **ts-rs** not understanding
        // `alias` while generating the TypeScript binding, not serde ignoring
        // it. Reading that warning as "the alias does not work" would be wrong,
        // and this is what tells the two apart rather than leaving it to
        // somebody's judgement.
        let old: Lane =
            serde_json::from_str("\"bass808\"").expect("the pre-rename name must still parse");
        assert_eq!(old, Lane::Sub);

        // ...and the new name is what gets written from here on.
        assert_eq!(serde_json::to_string(&Lane::Sub).unwrap(), "\"sub\"");

        // The lane it is most often confused with is a different lane, and the
        // rename was made precisely so that stays obvious.
        assert_ne!(Lane::Sub, Lane::Kick);
    }

    fn sample_pattern(seed: u64) -> Pattern {
        Pattern {
            loop_region: None,
            clip_region: None,
            id: "p1".into(),
            part: Part::Drums,
            artist_id: "osamason".into(),
            seed,
            song_seed: seed,
            bars: 4,
            bpm: 150.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 6,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane: Lane::Sub,
                notes: vec![
                    Note {
                        model_vel: None,
                        start_tick: 0,
                        len_ticks: PPQ,
                        pitch: 30,
                        vel: 110,
                        slide_to_pitch: Some(35),
                        articulation: Some(Articulation::Legato),
                        reversed: false,
                    },
                    Note {
                        model_vel: None,
                        start_tick: PPQ * 2,
                        len_ticks: PPQ / 2,
                        pitch: 30,
                        vel: 90,
                        slide_to_pitch: None,
                        articulation: None,
                        reversed: false,
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
                    bars: None,
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
                            bars: None,
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
                        bars: None,
                    },
                ),
                (
                    Part::Drums,
                    PatternRef {
                        pattern_id: "gone".into(),
                        bars: None,
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

    /// A melodic clip beside `sample_pattern`'s drums, for the merge (TASK-127).
    fn melody_pattern(start_tick: u32) -> Pattern {
        Pattern {
            id: "p2".into(),
            part: Part::Melody,
            lanes: vec![LaneTrack {
                lane: Lane::Melody,
                notes: vec![Note {
                    model_vel: None,
                    start_tick,
                    len_ticks: PPQ,
                    pitch: 64,
                    vel: 100,
                    slide_to_pitch: None,
                    articulation: None,
                    reversed: false,
                }],
            }],
            ..sample_pattern(7)
        }
    }

    #[test]
    fn merging_nothing_is_nothing_rather_than_an_empty_clip() {
        // ⛔ `None`, not a `Pattern` with no lanes. Arming a clip of nothing
        // would leave a transport running over silence with the UI insisting
        // something is playing — the shape of failure this file keeps recording.
        assert!(Pattern::merge(&[]).is_none());
    }

    #[test]
    fn one_part_merges_to_itself_including_its_name() {
        // ⛔ The solo case, and it must not go through the stand-in `part` that
        // the many-part case uses: `pattern_to_smf` writes its track name from
        // that field, so a soloed melody has to still say Melody.
        let only = melody_pattern(0);
        let merged = Pattern::merge(std::slice::from_ref(&only)).unwrap();
        assert_eq!(merged, only);
    }

    #[test]
    fn several_parts_become_one_clip_holding_every_lane() {
        // The whole point: one schedule, one `Pattern`, both parts sounding.
        let drums = sample_pattern(1);
        let melody = melody_pattern(PPQ);
        let merged = Pattern::merge(&[drums.clone(), melody.clone()]).unwrap();

        assert_eq!(
            merged.lanes.iter().map(|l| l.lane).collect::<Vec<_>>(),
            vec![Lane::Sub, Lane::Melody],
            "lane order is the order the parts were handed over"
        );
        assert_eq!(
            merged.note_count(),
            drums.note_count() + melody.note_count(),
            "no note may be dropped, and none invented"
        );
    }

    #[test]
    fn a_trimmed_clip_contributes_only_the_notes_it_still_has() {
        // ⛔ `clip_region` is honoured here or the trim is a lie — the same rule
        // `within_clip` exists for, applied at the merge. And it is NOT carried
        // onto the result: keeping it would trim the merged clip a second time.
        let mut drums = sample_pattern(1);
        drums.clip_region = Some(Region {
            from_tick: 0,
            to_tick: PPQ,
        });
        let merged = Pattern::merge(&[drums, melody_pattern(0)]).unwrap();

        let kept = merged.lanes.iter().find(|l| l.lane == Lane::Sub).unwrap();
        assert_eq!(kept.notes.len(), 1, "the note past the trim must not sound");
        assert!(
            merged.clip_region.is_none(),
            "the trim must not apply twice"
        );
    }

    #[test]
    fn a_clip_written_at_another_resolution_is_rebased_rather_than_played_fast() {
        // ⛔⛔ Silent corruption if this is skipped: a clip at half the ticks per
        // beat would play at double speed with nothing on screen to say so. The
        // start AND the length both move, or every note is stretched or crushed.
        let drums = sample_pattern(1);
        let mut half = melody_pattern(PPQ / 2);
        half.ppq = PPQ / 2;
        half.lanes[0].notes[0].len_ticks = PPQ / 2;

        let merged = Pattern::merge(&[drums, half]).unwrap();
        let note = &merged
            .lanes
            .iter()
            .find(|l| l.lane == Lane::Melody)
            .unwrap()
            .notes[0];

        assert_eq!(merged.ppq, PPQ, "the first clip's resolution wins");
        assert_eq!(
            note.start_tick, PPQ,
            "half a beat in, at the new resolution"
        );
        assert_eq!(note.len_ticks, PPQ, "and one beat long, not half of one");
    }

    #[test]
    fn the_merged_clip_is_as_long_as_its_longest_part() {
        // A four-bar drum loop under an eight-bar melody is eight bars of
        // record; taking the first part's length would cut the melody in half.
        let drums = sample_pattern(1);
        let mut long = melody_pattern(0);
        long.bars = 8;
        assert_eq!(Pattern::merge(&[drums, long]).unwrap().bars, 8);
    }

    #[test]
    fn notes_come_out_in_time_order_within_each_lane() {
        // The schedule walks a lane forwards; two parts writing the same lane
        // interleave, and an unsorted lane would drop or reorder events.
        let mut early = melody_pattern(0);
        early.lanes[0].notes[0].pitch = 60;
        let late = melody_pattern(PPQ * 3);
        let earlier = melody_pattern(PPQ);

        let merged = Pattern::merge(&[early, late, earlier]).unwrap();
        let notes = &merged
            .lanes
            .iter()
            .find(|l| l.lane == Lane::Melody)
            .unwrap()
            .notes;

        assert_eq!(
            notes.iter().map(|n| n.start_tick).collect::<Vec<_>>(),
            vec![0, PPQ, PPQ * 3],
            "three parts writing one lane must interleave in time"
        );
    }
}
