//! The small amount of music theory the generators share: interval names,
//! scale degrees, and placing a pitch class in a register.
//!
//! Models name intervals the way musicians do — `"P5"`, `"m7"`, `"P8"` — in the
//! 808's slide vocabulary and, later, in the bassline's passing tones and the
//! melody's leaps. One table, so `"m7"` cannot mean ten semitones in one
//! generator and eleven in another. The scale tables are here for the same
//! reason: the chord builder, the melody and the bass all have to agree on
//! which notes are in the key, or "in scale" means three different things.

use crate::pattern::{Scale, ScaleCharacter};

/// Semitones for an interval name.
///
/// `P` perfect, `M` major, `m` minor, `TT` tritone. Unison and octave are `P1`
/// and `P8`. Anything else is `None` — a name the table does not know is an
/// authoring mistake, and guessing at it would put a bass note somewhere the
/// model never asked for.
pub fn interval_semitones(name: &str) -> Option<i8> {
    Some(match name.trim() {
        "P1" | "unison" => 0,
        "m2" => 1,
        "M2" => 2,
        "m3" => 3,
        "M3" => 4,
        "P4" => 5,
        "TT" | "A4" | "d5" => 6,
        "P5" => 7,
        "m6" => 8,
        "M6" => 9,
        "m7" => 10,
        "M7" => 11,
        "P8" | "octave" => 12,
        _ => return None,
    })
}

/// The lowest MIDI note of this pitch class at or above `low`, or `None` if
/// that lands above `high`.
///
/// A register is a promise about where an instrument sits — an 808 authored
/// `[17, 31]` must not answer with a note two octaves up because the session
/// key moved.
pub fn pitch_class_in_register(pitch_class: u8, low: u8, high: u8) -> Option<u8> {
    if low > high {
        return None;
    }
    let class = pitch_class % 12;
    let first = low + ((12 + class - low % 12) % 12);
    (first <= high).then_some(first)
}

/// Fold a note back into a register by octaves, keeping its pitch class.
///
/// Used for slide targets: a fifth above the root can leave the register, and
/// the answer is the same note an octave down — not a clamp, which would change
/// the note to one the model did not choose.
pub fn fold_into_register(pitch: i16, low: u8, high: u8) -> Option<u8> {
    if low > high {
        return None;
    }
    let (low, high) = (i16::from(low), i16::from(high));
    let mut pitch = pitch;
    while pitch < low {
        pitch += 12;
    }
    while pitch > high {
        pitch -= 12;
    }
    (pitch >= low && pitch <= high && (0..=127).contains(&pitch)).then_some(pitch as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_table_matches_the_names_models_use() {
        assert_eq!(interval_semitones("P1"), Some(0));
        assert_eq!(interval_semitones("m2"), Some(1));
        assert_eq!(interval_semitones("M2"), Some(2));
        assert_eq!(interval_semitones("m3"), Some(3));
        assert_eq!(interval_semitones("P4"), Some(5));
        assert_eq!(interval_semitones("P5"), Some(7));
        assert_eq!(interval_semitones("m7"), Some(10));
        assert_eq!(interval_semitones("P8"), Some(12));
    }

    #[test]
    fn an_unknown_interval_is_rejected_rather_than_guessed() {
        assert_eq!(interval_semitones("P6"), None);
        assert_eq!(interval_semitones("fifth"), None);
        assert_eq!(interval_semitones(""), None);
    }

    #[test]
    fn a_pitch_class_lands_in_its_register() {
        // C1 is 24; the first C at or above 17 is 24.
        assert_eq!(pitch_class_in_register(0, 17, 31), Some(24));
        // F0 is 17 itself.
        assert_eq!(pitch_class_in_register(5, 17, 31), Some(17));
        // G is 19 within [17, 31].
        assert_eq!(pitch_class_in_register(7, 17, 31), Some(19));
    }

    #[test]
    fn a_register_too_narrow_for_a_pitch_class_answers_none() {
        // [24, 26] holds C, C# and D — an F has nowhere to go.
        assert_eq!(pitch_class_in_register(5, 24, 26), None);
        assert_eq!(pitch_class_in_register(0, 31, 17), None, "inverted");
    }

    #[test]
    fn folding_moves_by_octaves_and_keeps_the_pitch_class() {
        // A fifth above C1 (24) is 31, still inside [17, 31].
        assert_eq!(fold_into_register(31, 17, 31), Some(31));
        // An octave above is 36 — out of range, so the same note an octave down.
        assert_eq!(fold_into_register(36, 17, 31), Some(24));
        assert_eq!(fold_into_register(12, 17, 31), Some(24));
        // The pitch class survives the fold, which is the whole point.
        for pitch in 0..60i16 {
            if let Some(folded) = fold_into_register(pitch, 17, 31) {
                assert_eq!(folded % 12, (pitch % 12) as u8);
                assert!((17..=31).contains(&folded));
            }
        }
    }

    #[test]
    fn a_register_narrower_than_an_octave_can_refuse() {
        // [24, 26] cannot hold every pitch class, and saying so is better than
        // answering with a note the model did not choose.
        assert_eq!(fold_into_register(29, 24, 26), None);
        assert_eq!(fold_into_register(24, 24, 26), Some(24));
    }
}

/// The pitch class of a key name — `"C"`, `"F#m"`, `"Bbm"`.
///
/// The dataset spells keys three ways and all three are legitimate: sharp
/// minors (`"F#m"`) in most models, bare majors (`"G"`) in country and pop,
/// and flats (`"Bbm"`, `"Ebm"`) in R&B. The trailing `m` is a hint about the
/// mode and is *not* what decides the scale — a model states that separately in
/// `session.scales` — so it is accepted and ignored here.
pub fn key_pitch_class(name: &str) -> Option<u8> {
    let mut chars = name.trim().chars();
    let letter = chars.next()?;
    let base = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };

    let rest: String = chars.collect();
    let (accidental, rest) = match rest.chars().next() {
        Some('#') => (1i8, &rest[1..]),
        Some('b') if rest.len() > 1 || rest == "b" => (-1i8, &rest[1..]),
        _ => (0, rest.as_str()),
    };

    // What may follow: nothing, or a mode marker.
    if !matches!(rest, "" | "m" | "min" | "maj" | "M") {
        return None;
    }

    Some(((base as i8 + accidental).rem_euclid(12)) as u8)
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn all_three_spellings_the_dataset_uses_parse() {
        // Sharp minors — most models.
        assert_eq!(key_pitch_class("F#m"), Some(6));
        assert_eq!(key_pitch_class("C#m"), Some(1));
        // Bare majors — country-train, pop-2000s.
        assert_eq!(key_pitch_class("G"), Some(7));
        assert_eq!(key_pitch_class("C"), Some(0));
        // Flats — rnb-2000s.
        assert_eq!(key_pitch_class("Bbm"), Some(10));
        assert_eq!(key_pitch_class("Ebm"), Some(3));
        // Plain minors.
        assert_eq!(key_pitch_class("Am"), Some(9));
        assert_eq!(key_pitch_class("Cm"), Some(0));
    }

    #[test]
    fn the_mode_marker_does_not_change_the_root() {
        // `session.scales` decides the scale; the `m` is only a hint, so a
        // model that spells the same root both ways must not move the key.
        assert_eq!(key_pitch_class("A"), key_pitch_class("Am"));
        assert_eq!(key_pitch_class("Eb"), key_pitch_class("Ebm"));
    }

    #[test]
    fn nonsense_is_rejected_rather_than_guessed() {
        assert_eq!(key_pitch_class("H"), None);
        assert_eq!(key_pitch_class(""), None);
        assert_eq!(key_pitch_class("Cmaj7"), None);
        assert_eq!(key_pitch_class("2"), None);
    }
}

/// Is this pitch one of the key's own notes?
///
/// ⛔ **The same `(pitch - key_root).rem_euclid(12)` test was hand-rolled in
/// five places** — `bass.rs` twice, `chords.rs`, `melody.rs` and `coherence.rs`
/// — because `theory` owned every other statement about a key and not this one.
/// It is a question about a key, so it lives with the key.
pub fn in_scale(pitch: i32, key_root: u8, degrees: &[u8]) -> bool {
    degrees.contains(&((pitch - i32::from(key_root)).rem_euclid(12) as u8))
}

/// The nearest pitch that *is* one of the key's own notes.
///
/// ⚠ **Ties resolve downward**, and that is a musical decision rather than an
/// implementation detail: a line repaired onto the note below keeps its shape
/// where one pushed up would invert it. Stated once here so the next caller
/// inherits the choice instead of re-deciding it.
///
/// Searches ±6 semitones, which always finds something — no scale in
/// [`scale_semitones`] has a gap that wide.
pub fn nearest_scale_tone(pitch: i32, key_root: u8, degrees: &[u8]) -> i32 {
    if in_scale(pitch, key_root, degrees) {
        return pitch;
    }
    (1..=6)
        .flat_map(|delta| [pitch - delta, pitch + delta])
        .find(|candidate| in_scale(*candidate, key_root, degrees))
        .unwrap_or(pitch)
}

/// The semitones of a scale's degrees, from its root.
///
/// The generators ask for this in two different ways and both are here: a
/// melody walks the scale it is *in*, so a minor-pentatonic model gets five
/// degrees; harmony stacks thirds, which needs seven. [`harmonic_degrees`] is
/// the second question.
pub fn scale_semitones(scale: Scale) -> &'static [u8] {
    match scale {
        // ---- Modes of the major scale ----------------------------------
        Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
        Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
        Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
        Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
        Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
        // Aeolian *is* the natural minor. Two names for one row, because the
        // dataset uses both and a model author picking the other spelling must
        // not get a different scale.
        Scale::NaturalMinor | Scale::Aeolian => &[0, 2, 3, 5, 7, 8, 10],
        Scale::Locrian => &[0, 1, 3, 5, 6, 8, 10],

        // ---- Pentatonic and blues --------------------------------------
        Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
        Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
        Scale::MajorBlues => &[0, 2, 3, 4, 7, 9],
        // The minor blues. Named bare because the dataset already authors it
        // that way, for the same reason `Aeolian` is kept beside `NaturalMinor`.
        Scale::Blues => &[0, 3, 5, 6, 7, 10],

        // ---- Minor and major variants ----------------------------------
        Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        Scale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
        Scale::HarmonicMajor => &[0, 2, 4, 5, 7, 8, 11],

        // ---- Modes of those --------------------------------------------
        Scale::PhrygianDominant => &[0, 1, 4, 5, 7, 8, 10],
        Scale::DorianSharp4 => &[0, 2, 3, 6, 7, 9, 10],
        Scale::LydianAugmented => &[0, 2, 4, 6, 8, 9, 11],
        Scale::LydianDominant => &[0, 2, 4, 6, 7, 9, 10],
        Scale::SuperLocrian => &[0, 1, 3, 4, 6, 8, 10],
        Scale::LocrianNatural6 => &[0, 1, 3, 5, 6, 9, 10],
        Scale::IonianSharp5 => &[0, 2, 4, 5, 8, 9, 11],
        Scale::Ultralocrian => &[0, 1, 3, 4, 6, 8, 9],

        // ---- Symmetric --------------------------------------------------
        Scale::WholeTone => &[0, 2, 4, 6, 8, 10],
        Scale::WholeHalfDiminished => &[0, 2, 3, 5, 6, 8, 9, 11],
        Scale::HalfWholeDiminished => &[0, 1, 3, 4, 6, 7, 9, 10],
        Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],

        // ---- World ------------------------------------------------------
        Scale::HungarianMinor => &[0, 2, 3, 6, 7, 8, 11],
        Scale::EightToneSpanish => &[0, 1, 3, 4, 5, 6, 8, 10],
        Scale::Bhairav => &[0, 1, 4, 5, 7, 8, 11],
        Scale::Hirajoshi => &[0, 2, 3, 7, 8],
        Scale::InSen => &[0, 1, 5, 7, 10],
        Scale::Iwato => &[0, 1, 5, 6, 10],
        Scale::Kumoi => &[0, 2, 3, 7, 9],
        Scale::PelogSelisir => &[0, 1, 3, 7, 8],
        Scale::PelogTembung => &[0, 1, 5, 7, 8],

        // ---- Messiaen modes 3–7 -----------------------------------------
        Scale::MessiaenMode3 => &[0, 2, 3, 4, 6, 7, 8, 10, 11],
        Scale::MessiaenMode4 => &[0, 1, 2, 5, 6, 7, 8, 11],
        Scale::MessiaenMode5 => &[0, 1, 5, 6, 7, 11],
        Scale::MessiaenMode6 => &[0, 2, 4, 5, 6, 8, 10, 11],
        Scale::MessiaenMode7 => &[0, 1, 2, 3, 5, 6, 7, 8, 9, 11],
    }
}

/// The seven-degree scale harmony is built from.
///
/// Chords stack thirds, and a five-degree scale has no third to stack: asking
/// a minor pentatonic for its degree 6 has no answer, and clamping would put a
/// `VI` on the wrong root. Pentatonic and blues scales are *melodic* choices
/// sitting inside a parent key, so the chords are built from that parent and
/// the melody still gets the five notes it was authored for.
///
/// ⛔ **Every scale that is not seven notes long needs a parent here, and
/// `every_scale_answers_seven_harmonic_degrees` is what makes that impossible to
/// forget.** The rule for choosing one is the *tonic triad*: a scale is sent to
/// the seven-note key its first, third and fifth degrees belong to, so the
/// chords stay in the world the melody is being written in. A 5-, 6-, 8- or
/// 12-note scale with no parent would hand the chord builder fewer degrees than
/// it asks for and silently put roman numerals on the wrong roots.
pub fn harmonic_degrees(scale: Scale) -> &'static [u8] {
    let parent = match scale {
        // Minor third, flat sixth: an ordinary natural minor.
        Scale::MinorPentatonic
        | Scale::Blues
        | Scale::Hirajoshi
        | Scale::WholeHalfDiminished
        | Scale::MessiaenMode3
        | Scale::MessiaenMode7 => Scale::NaturalMinor,

        // Minor third, *natural* sixth. Dorian rather than the natural minor,
        // because that sixth is the note the scale is chosen for and a flat one
        // underneath it would put the harmony outside the melody's own key.
        Scale::Kumoi => Scale::Dorian,

        // A flat second is the defining note of these, so the parent has to
        // have one. Iwato's flat fifth takes Locrian instead.
        Scale::InSen | Scale::PelogSelisir | Scale::PelogTembung => Scale::Phrygian,
        Scale::Iwato => Scale::Locrian,

        // Major-third tonic.
        Scale::MajorPentatonic | Scale::MajorBlues | Scale::Chromatic | Scale::MessiaenMode6 => {
            Scale::Major
        }

        // A dominant-flavoured eight- or six-note scale: major third, flat
        // seventh. Mixolydian is the seven-note key that describes it.
        Scale::HalfWholeDiminished | Scale::EightToneSpanish => Scale::Mixolydian,

        // Whole tone and Messiaen 5 have an augmented or bare fifth and a major
        // third; Lydian augmented is the nearest seven-note key that keeps both.
        Scale::WholeTone | Scale::MessiaenMode5 => Scale::LydianAugmented,

        // Messiaen 4 opens with a minor second over a major third — the same
        // shape Phrygian dominant names.
        Scale::MessiaenMode4 => Scale::PhrygianDominant,

        // Everything else already answers with seven degrees of its own.
        other => other,
    };
    scale_semitones(parent)
}

/// Whether a scale reads dark, neutral or bright (TASK-041C).
///
/// ⛔ **One table, and `every_scale_has_a_character` fails on a scale added
/// without one.** This is what makes "a dark mood offers dark scales" hold for
/// scales nobody has authored a mood against yet.
///
/// The rule is the third and the sixth: a minor third reads dark, a major third
/// bright, and a scale with no stable third — the symmetric ones, where every
/// transposition is the same set — reads neutral. The named exceptions are the
/// ones where the sixth or the second overrides the third: Dorian's natural
/// sixth lifts a minor scale to neutral, and Phrygian's flat second darkens one
/// that is already minor.
pub fn scale_character(scale: Scale) -> ScaleCharacter {
    use ScaleCharacter::{Bright, Dark, Neutral};
    match scale {
        // A minor third, with nothing lifting it — or a flat second, which
        // darkens a scale whatever its third is doing.
        //
        // ⛔ **Phrygian dominant and Bhairav are here despite a *major* third**,
        // and they are the reason the rule is stated as "minor third or flat
        // second" rather than "minor third". Both are ♭2/♭6 shapes and read as
        // the Spanish/raga colour rather than as a major scale.
        // `midi::key_signature` files them under minor in the same words, and
        // the two have to agree — otherwise the roll tints a scale bright while
        // the exported key signature calls it minor.
        //
        // Iwato, In-Sen and Pelog Tembung have no third at all; the flat second
        // is what places them.
        Scale::NaturalMinor
        | Scale::Aeolian
        | Scale::Phrygian
        | Scale::Locrian
        | Scale::HarmonicMinor
        | Scale::MinorPentatonic
        | Scale::Blues
        | Scale::PhrygianDominant
        | Scale::Bhairav
        | Scale::SuperLocrian
        | Scale::LocrianNatural6
        | Scale::Ultralocrian
        | Scale::HungarianMinor
        | Scale::EightToneSpanish
        | Scale::Iwato
        | Scale::InSen
        | Scale::Hirajoshi
        | Scale::PelogSelisir
        | Scale::PelogTembung
        | Scale::DorianSharp4 => Dark,

        // A major third and no flat second.
        Scale::Major
        | Scale::Lydian
        | Scale::Mixolydian
        | Scale::MajorPentatonic
        | Scale::MajorBlues
        | Scale::HarmonicMajor
        | Scale::LydianDominant
        | Scale::LydianAugmented
        | Scale::IonianSharp5 => Bright,

        // ⛔ Dorian is minor but its natural sixth is the whole reason a
        // producer reaches for it instead of the natural minor — it is the
        // "not-sad minor", and filing it under Dark would hide it from every
        // mood that is not explicitly gloomy.
        Scale::Dorian
        | Scale::MelodicMinor
        | Scale::Kumoi
        // Symmetric: no tonic to be major or minor about.
        | Scale::WholeTone
        | Scale::WholeHalfDiminished
        | Scale::HalfWholeDiminished
        | Scale::Chromatic
        | Scale::MessiaenMode3
        | Scale::MessiaenMode4
        | Scale::MessiaenMode5
        | Scale::MessiaenMode6
        | Scale::MessiaenMode7 => Neutral,
    }
}

/// The semitone of a scale degree, counting from 1, wrapping into octaves.
///
/// Degree 8 is degree 1 an octave up, which is what makes stacking thirds a
/// matter of adding 2 to the degree — the caller never has to know how many
/// degrees the scale has.
pub fn degree_semitone(degrees: &[u8], degree: i32) -> i32 {
    if degrees.is_empty() {
        return 0;
    }
    let count = degrees.len() as i32;
    let index = (degree - 1).rem_euclid(count);
    let octave = (degree - 1).div_euclid(count);
    i32::from(degrees[index as usize]) + octave * 12
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn every_scale_is_ordered_and_starts_on_its_root() {
        // A scale that does not start at 0 is not the scale it names, and one
        // that is not ascending breaks the degree arithmetic below.
        for scale in ALL_SCALES {
            let degrees = scale_semitones(scale);
            assert_eq!(degrees[0], 0, "{scale:?} does not start on its root");
            assert!(
                degrees.windows(2).all(|w| w[0] < w[1]),
                "{scale:?} is not ascending: {degrees:?}"
            );
            assert!(*degrees.last().unwrap() < 12, "{scale:?} spills an octave");
        }
    }

    #[test]
    fn harmony_always_has_seven_degrees_to_stack_thirds_on() {
        // The property the chord builder relies on: `VI` has an answer in every
        // scale a session can be in, including the five-note ones.
        for scale in ALL_SCALES {
            assert_eq!(
                harmonic_degrees(scale).len(),
                7,
                "{scale:?} cannot support a triad"
            );
        }
    }

    #[test]
    fn a_pentatonic_borrows_its_parents_harmony_and_keeps_its_own_melody() {
        assert_eq!(scale_semitones(Scale::MinorPentatonic).len(), 5);
        assert_eq!(
            harmonic_degrees(Scale::MinorPentatonic),
            scale_semitones(Scale::NaturalMinor)
        );
        assert_eq!(
            harmonic_degrees(Scale::MajorPentatonic),
            scale_semitones(Scale::Major)
        );
    }

    #[test]
    fn aeolian_and_natural_minor_are_the_same_scale() {
        assert_eq!(
            scale_semitones(Scale::Aeolian),
            scale_semitones(Scale::NaturalMinor)
        );
    }

    #[test]
    fn degrees_wrap_into_octaves_so_thirds_can_be_stacked() {
        let minor = scale_semitones(Scale::NaturalMinor);
        assert_eq!(degree_semitone(minor, 1), 0);
        assert_eq!(degree_semitone(minor, 3), 3);
        assert_eq!(degree_semitone(minor, 5), 7);
        // Degree 8 is the octave, and 9 is the ninth — a step above it.
        assert_eq!(degree_semitone(minor, 8), 12);
        assert_eq!(degree_semitone(minor, 9), 14);
        // ...and below the root, which a voicing reaches for.
        assert_eq!(degree_semitone(minor, 0), -2);
    }

    #[test]
    fn every_scale_has_a_character() {
        // ⛔ **The gate that makes "a dark mood offers dark scales" hold for
        // scales nobody has authored a mood against yet.** `scale_character` is
        // an exhaustive match, so this cannot fail at runtime — which is the
        // point: adding a scale without classifying it fails to *compile*, and
        // this test is what says so out loud to whoever hits that error.
        for scale in Scale::ALL {
            let character = scale_character(scale);
            assert!(
                matches!(
                    character,
                    ScaleCharacter::Dark | ScaleCharacter::Neutral | ScaleCharacter::Bright
                ),
                "{scale:?} has no character"
            );
        }
    }

    #[test]
    fn the_intervals_agree_with_the_characters() {
        // ⛔ **The rule the table is written to, checked rather than trusted —
        // and it caught the table being wrong on its first run.** Phrygian
        // dominant was filed Dark against a "dark means minor third" rule that
        // it breaks, which is how the rule turned out to be *minor third or
        // flat second*: a ♭2 darkens a scale whatever its third does. Iwato,
        // In-Sen and Pelog Tembung have no third at all and are placed by the
        // same ♭2.
        //
        // Neutral is exempt, which is what neutral means here: the symmetric
        // scales have no tonic to be major or minor about.
        for scale in Scale::ALL {
            let degrees = scale_semitones(scale);
            let minor_third = degrees.contains(&3);
            let major_third = degrees.contains(&4);
            let flat_second = degrees.contains(&1);

            match scale_character(scale) {
                ScaleCharacter::Dark => assert!(
                    minor_third || flat_second,
                    "{scale:?} is filed dark but has neither a minor third nor a flat second: {degrees:?}"
                ),
                ScaleCharacter::Bright => assert!(
                    major_third && !flat_second,
                    "{scale:?} is filed bright but is not a major third without a flat second: {degrees:?}"
                ),
                ScaleCharacter::Neutral => {}
            }
        }
    }

    #[test]
    fn a_scale_carrying_both_thirds_is_still_placed_by_its_character() {
        // ⛔ **The case that made `midi::key_signature` defer to this table.**
        // The major blues scale is the major pentatonic plus a ♭3 blue note, so
        // it holds *both* thirds — and any rule phrased as "has a minor third
        // ⇒ minor" calls it minor while this table calls it bright. The blue
        // note is a colour over a major scale, not a change of mode.
        let degrees = scale_semitones(Scale::MajorBlues);
        assert!(degrees.contains(&3) && degrees.contains(&4), "{degrees:?}");
        assert_eq!(scale_character(Scale::MajorBlues), ScaleCharacter::Bright);

        // Its minor namesake keeps the opposite answer, so the pair cannot be
        // collapsed by anyone tidying up later.
        assert_eq!(scale_character(Scale::Blues), ScaleCharacter::Dark);
    }

    #[test]
    fn every_scale_is_listed_in_all_exactly_once() {
        // `Scale::ALL` widens what every gate above covers, so a duplicate
        // quietly means one scale is tested twice and — far more likely — that
        // a paste went wrong and another is not tested at all.
        let mut seen = std::collections::BTreeSet::new();
        for scale in Scale::ALL {
            assert!(
                seen.insert(format!("{scale:?}")),
                "{scale:?} is listed twice"
            );
        }
        assert_eq!(seen.len(), Scale::ALL.len());
    }

    #[test]
    fn the_two_names_for_the_natural_minor_stay_one_scale() {
        // The dataset authors both spellings. They were one row before the set
        // grew to forty-one and they have to stay one row.
        assert_eq!(
            scale_semitones(Scale::Aeolian),
            scale_semitones(Scale::NaturalMinor)
        );
        assert_eq!(
            scale_character(Scale::Aeolian),
            scale_character(Scale::NaturalMinor)
        );
    }

    /// Every scale, so the invariants above cannot be tested on a subset.
    ///
    /// Points at the production list rather than restating it — a second copy
    /// here is exactly how a gate ends up silently covering twelve of forty-one.
    const ALL_SCALES: [Scale; 41] = Scale::ALL;
}

/// A key as a producer would write it on a file: `"C# Minor"`, `"F Major"`.
///
/// ⛔ **The inverse of [`key_pitch_class`], and it exists for file names**
/// (TASK-131F). Mike, 2026-08-05: an exported stem should be labelled
/// *"Artist/Genre - Snares - 140 BPM - C# Minor"* — a folder of those is
/// readable without opening any of them, which is the same argument the
/// suggested-name logic in `export.rs` already makes.
///
/// ⚠ **Sharps, not flats, and one spelling per pitch class.** A key signature
/// has a correct spelling that depends on the key — Eb minor is not D# minor to
/// a musician — but a file name is an identifier, and two spellings for one
/// pitch class means two names for one file. The dataset authors both
/// (`rnb-2000s` writes `Ebm`, most models write sharps) and they resolve to the
/// same number here, so this picks the one spelling that covers all twelve.
pub fn key_label(root: u8, scale: crate::pattern::Scale) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let name = NAMES[usize::from(root % 12)];
    format!("{name} {}", scale_label(scale))
}

/// A scale's name with spaces in it: `NaturalMinor` → `"Natural Minor"`.
///
/// ⚠ **Derived from the variant rather than a table**, so a scale added to the
/// enum cannot arrive here unnamed. A hand-written table is a second list of the
/// 41 scales, and the one thing this codebase has learned about second lists is
/// that they fall behind.
fn scale_label(scale: crate::pattern::Scale) -> String {
    // ⚠ The two scales a producer names differently from the enum. Everything
    // else reads correctly split.
    let debug = match scale {
        crate::pattern::Scale::NaturalMinor | crate::pattern::Scale::Aeolian => "Minor".to_owned(),
        other => format!("{other:?}"),
    };
    let mut out = String::with_capacity(debug.len() + 2);
    for (index, character) in debug.chars().enumerate() {
        if index > 0 && character.is_uppercase() {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

#[cfg(test)]
mod key_label_tests {
    use super::*;
    use crate::pattern::Scale;

    #[test]
    fn a_key_reads_the_way_a_producer_would_write_it() {
        assert_eq!(key_label(1, Scale::NaturalMinor), "C# Minor");
        assert_eq!(key_label(0, Scale::Major), "C Major");
        assert_eq!(key_label(6, Scale::NaturalMinor), "F# Minor");
    }

    #[test]
    fn every_root_has_a_name_and_none_of_them_panics() {
        // ⛔ `root` reaches here from a `Pattern` that arrived as JSON from the
        // webview, so it is not guaranteed to be 0..12.
        for root in 0..=255u8 {
            let label = key_label(root, Scale::NaturalMinor);
            assert!(!label.is_empty(), "root {root} produced no name");
        }
    }

    #[test]
    fn a_multi_word_scale_gets_its_spaces() {
        assert_eq!(scale_label(Scale::HarmonicMinor), "Harmonic Minor");
        assert_eq!(scale_label(Scale::MinorPentatonic), "Minor Pentatonic");
        // A single-word one is left alone.
        assert_eq!(scale_label(Scale::Dorian), "Dorian");
    }
}
