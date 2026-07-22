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
    NaturalMinor,
    HarmonicMinor,
    Phrygian,
    PhrygianDominant,
    Dorian,
    Major,
    Mixolydian,
    Lydian,
    Aeolian,
    MinorPentatonic,
    MajorPentatonic,
    Blues,
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
}

impl Pattern {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum SectionKind {
    Intro,
    Verse,
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
    pub ppq: u32,
}

impl Song {
    /// Total length in bars.
    pub fn total_bars(&self) -> u32 {
        self.sections
            .iter()
            .map(|s| s.start_bar + u32::from(s.bars))
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pattern(seed: u64) -> Pattern {
        Pattern {
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
                        start_tick: 0,
                        len_ticks: PPQ,
                        pitch: 30,
                        vel: 110,
                        slide_to_pitch: Some(35),
                        articulation: Some(Articulation::Legato),
                    },
                    Note {
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
                    markers: vec![],
                },
            ],
            ppq: PPQ,
        };
        assert_eq!(song.total_bars(), 24);
        let back: Song = serde_json::from_str(&serde_json::to_string(&song).unwrap()).unwrap();
        assert_eq!(song, back);
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
