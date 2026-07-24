//! The small amount of music theory the generators share: interval names and
//! placing a pitch class in a register.
//!
//! Models name intervals the way musicians do — `"P5"`, `"m7"`, `"P8"` — in the
//! 808's slide vocabulary and, later, in the bassline's passing tones and the
//! melody's leaps. One table, so `"m7"` cannot mean ten semitones in one
//! generator and eleven in another.

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
