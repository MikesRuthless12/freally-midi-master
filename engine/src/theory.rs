//! The small amount of music theory the generators share: interval names,
//! scale degrees, and placing a pitch class in a register.
//!
//! Models name intervals the way musicians do — `"P5"`, `"m7"`, `"P8"` — in the
//! 808's slide vocabulary and, later, in the bassline's passing tones and the
//! melody's leaps. One table, so `"m7"` cannot mean ten semitones in one
//! generator and eleven in another. The scale tables are here for the same
//! reason: the chord builder, the melody and the bass all have to agree on
//! which notes are in the key, or "in scale" means three different things.

use crate::pattern::Scale;

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

/// The semitones of a scale's degrees, from its root.
///
/// The generators ask for this in two different ways and both are here: a
/// melody walks the scale it is *in*, so a minor-pentatonic model gets five
/// degrees; harmony stacks thirds, which needs seven. [`harmonic_degrees`] is
/// the second question.
pub fn scale_semitones(scale: Scale) -> &'static [u8] {
    match scale {
        // Aeolian *is* the natural minor. Two names for one row, because the
        // dataset uses both and a model author picking the other spelling must
        // not get a different scale.
        Scale::NaturalMinor | Scale::Aeolian => &[0, 2, 3, 5, 7, 8, 10],
        Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
        Scale::PhrygianDominant => &[0, 1, 4, 5, 7, 8, 10],
        Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
        Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
        Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
        Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
        Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
        Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
        Scale::Blues => &[0, 3, 5, 6, 7, 10],
    }
}

/// The seven-degree scale harmony is built from.
///
/// Chords stack thirds, and a five-degree scale has no third to stack: asking
/// a minor pentatonic for its degree 6 has no answer, and clamping would put a
/// `VI` on the wrong root. Pentatonic and blues scales are *melodic* choices
/// sitting inside a parent key, so the chords are built from that parent and
/// the melody still gets the five notes it was authored for.
pub fn harmonic_degrees(scale: Scale) -> &'static [u8] {
    let parent = match scale {
        Scale::MinorPentatonic | Scale::Blues => Scale::NaturalMinor,
        Scale::MajorPentatonic => Scale::Major,
        other => other,
    };
    scale_semitones(parent)
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

    /// Every scale, so the invariants above cannot be tested on a subset.
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
}
