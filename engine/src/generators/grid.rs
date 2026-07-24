//! The 16th-note grid and the position notation the dataset is written in.
//!
//! Models name positions the way the research does — `1e&a 2e&a 3e&a 4e&a`, so
//! `"2&"` is the 8th after beat 2 — and note values the way producers do:
//! `"8th"`, `"16T"`, `"32"`. Both spellings resolve to ticks here, once, because
//! the kick grammar, the hat engine, the roll vocabulary and the fill logic all
//! read them and must agree to the tick.
//!
//! Everything lands on an integer tick at PPQ 960: a 16th triplet is 160, a
//! 32nd triplet 80, a 64th 60. That is what the PPQ was chosen for.

use crate::context::SessionContext;
use crate::pattern::PPQ;

/// Ticks in one 16th note. A 16th is a 16th whatever the time signature says —
/// the meter changes how many fit in a bar, not how long one is.
pub const SIXTEENTH: u32 = PPQ / 4;

/// Ticks in one beat, as the time signature counts beats.
pub fn ticks_per_beat(ctx: &SessionContext) -> u32 {
    let den = if ctx.time_sig_den == 0 {
        4
    } else {
        u32::from(ctx.time_sig_den)
    };
    PPQ * 4 / den
}

/// How many 16ths fit in one bar of this meter.
pub fn sixteenths_per_bar(ctx: &SessionContext) -> u32 {
    ctx.ticks_per_bar() / SIXTEENTH
}

/// Parse a grid position like `"1"`, `"2&"`, `"4a"` into ticks from the bar's
/// start.
///
/// `None` for anything that is not a position in *this* meter — including beat
/// 5 of a 4/4 bar, which is the mistake a model makes when it is edited from a
/// different time signature. Callers report it rather than guessing, because a
/// position that silently resolves to nothing is a grammar rule that quietly
/// stops applying.
pub fn position_ticks(text: &str, ctx: &SessionContext) -> Option<u32> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (beat, subdivision) = text.split_at(split);

    let beat: u32 = beat.parse().ok()?;
    if beat == 0 || beat > u32::from(ctx.time_sig_num.max(1)) {
        return None;
    }

    let sixteenths = match subdivision {
        "" => 0,
        "e" => 1,
        "&" => 2,
        "a" => 3,
        _ => return None,
    };

    Some((beat - 1) * ticks_per_beat(ctx) + sixteenths * SIXTEENTH)
}

/// Parse a note value — `"8th"`, `"16"`, `"16T"`, `"32nd"`, `"64"` — into ticks.
///
/// The dataset uses both spellings: `avoidPreSnareGap` says `"8th"` while the
/// roll vocabulary says `"16T"`. A trailing `T` is a triplet, two thirds of the
/// plain value.
pub fn note_value_ticks(text: &str) -> Option<u32> {
    let text = text.trim();
    let (digits, suffix) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let value: u32 = digits.parse().ok()?;
    if value == 0 {
        return None;
    }

    let triplet = match suffix.to_ascii_lowercase().as_str() {
        "" | "th" | "nd" | "rd" | "st" => false,
        "t" => true,
        // Anything else is not a note value, and guessing at it would put roll
        // notes on a subdivision nobody asked for.
        _ => return None,
    };

    let whole = PPQ * 4;
    if !whole.is_multiple_of(value) {
        return None;
    }
    let ticks = whole / value;
    Some(if triplet { ticks * 2 / 3 } else { ticks })
}

/// The tresillo — 3+3+2 in 8ths, so 16th indices 0, 6 and 12 of each bar.
///
/// It is the backbone of the trap and drill kick grammars, and the reason
/// drill's authored two-bar form opens with `["1", "2&", "4"]`: that *is* the
/// tresillo, spelled in grid positions.
pub fn is_tresillo(sixteenth_index: u32) -> bool {
    matches!(sixteenth_index % 16, 0 | 6 | 12)
}

/// Is this 16th one of the beats — 1, 2, 3, 4?
pub fn is_downbeat(sixteenth_index: u32) -> bool {
    sixteenth_index.is_multiple_of(4)
}

/// Is this 16th an offbeat 8th — an "&"?
pub fn is_offbeat_eighth(sixteenth_index: u32) -> bool {
    sixteenth_index % 4 == 2
}

/// Is this 16th one of the in-between "e" and "a" positions?
pub fn is_sixteenth_offbeat(sixteenth_index: u32) -> bool {
    sixteenth_index % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SessionContext {
        SessionContext::default()
    }

    #[test]
    fn positions_read_the_way_the_research_writes_them() {
        let c = ctx();
        assert_eq!(position_ticks("1", &c), Some(0));
        assert_eq!(position_ticks("1e", &c), Some(240));
        assert_eq!(position_ticks("1&", &c), Some(480));
        assert_eq!(position_ticks("1a", &c), Some(720));
        assert_eq!(position_ticks("2", &c), Some(960));
        assert_eq!(position_ticks("2&", &c), Some(1440));
        assert_eq!(position_ticks("3", &c), Some(1920));
        assert_eq!(position_ticks("4", &c), Some(2880));
        assert_eq!(position_ticks("4&", &c), Some(3360));
    }

    #[test]
    fn whitespace_is_forgiven_but_nonsense_is_not() {
        let c = ctx();
        assert_eq!(position_ticks(" 2& ", &c), Some(1440));
        assert_eq!(position_ticks("", &c), None);
        assert_eq!(position_ticks("&", &c), None);
        assert_eq!(position_ticks("0", &c), None, "beats are 1-based");
        assert_eq!(position_ticks("2x", &c), None);
        assert_eq!(position_ticks("two", &c), None);
    }

    #[test]
    fn a_beat_the_meter_does_not_have_is_rejected() {
        // The mistake a model makes when it is edited from another time
        // signature. Resolving it to *something* would leave a grammar rule
        // quietly pointing outside the bar.
        let c = ctx();
        assert_eq!(position_ticks("5", &c), None);

        let three_four = SessionContext {
            time_sig_num: 3,
            ..Default::default()
        };
        assert_eq!(position_ticks("4", &three_four), None);
        assert_eq!(position_ticks("3", &three_four), Some(1920));
    }

    #[test]
    fn the_tresillo_is_the_drill_kick_grammar() {
        let c = ctx();
        // ["1", "2&", "4"] — drill's authored bar 1 — is 16ths 0, 6, 12.
        let positions = ["1", "2&", "4"];
        for (position, expected) in positions.iter().zip([0, 6, 12]) {
            let ticks = position_ticks(position, &c).unwrap();
            assert_eq!(ticks / SIXTEENTH, expected);
            assert!(
                is_tresillo(ticks / SIXTEENTH),
                "{position} is a tresillo hit"
            );
        }
        assert!(!is_tresillo(4), "beat 2 is not part of the 3-3-2");
    }

    #[test]
    fn note_values_read_in_both_spellings() {
        assert_eq!(note_value_ticks("4"), Some(960));
        assert_eq!(note_value_ticks("8"), Some(480));
        assert_eq!(note_value_ticks("8th"), Some(480));
        assert_eq!(note_value_ticks("16"), Some(240));
        assert_eq!(note_value_ticks("16th"), Some(240));
        assert_eq!(note_value_ticks("32"), Some(120));
        assert_eq!(note_value_ticks("32nd"), Some(120));
        assert_eq!(note_value_ticks("64"), Some(60));
    }

    #[test]
    fn triplets_land_on_whole_ticks() {
        // The whole reason PPQ is 960: the roll vocabulary switches between
        // these mid-beat and none of them may be rounded.
        assert_eq!(note_value_ticks("16T"), Some(160));
        assert_eq!(note_value_ticks("32T"), Some(80));
        assert_eq!(note_value_ticks("8T"), Some(320));
        // Three 16th triplets fill exactly one 8th.
        assert_eq!(note_value_ticks("16T").unwrap() * 3, 480);
    }

    #[test]
    fn a_note_value_that_does_not_divide_the_bar_is_rejected() {
        assert_eq!(note_value_ticks("0"), None);
        assert_eq!(
            note_value_ticks("7"),
            None,
            "a 7th note is not a note value"
        );
        assert_eq!(note_value_ticks("quarter"), None);
        assert_eq!(note_value_ticks(""), None);
    }

    #[test]
    fn the_bar_divides_into_sixteen_sixteenths_in_four_four() {
        assert_eq!(sixteenths_per_bar(&ctx()), 16);
        assert_eq!(ticks_per_beat(&ctx()), 960);
        assert_eq!(
            sixteenths_per_bar(&SessionContext {
                time_sig_num: 3,
                ..Default::default()
            }),
            12
        );
    }

    #[test]
    fn the_grid_predicates_partition_the_bar() {
        // Every 16th is exactly one of: a beat, an offbeat 8th, or an e/a.
        for i in 0..16 {
            let kinds = [
                is_downbeat(i),
                is_offbeat_eighth(i),
                is_sixteenth_offbeat(i),
            ]
            .iter()
            .filter(|x| **x)
            .count();
            assert_eq!(kinds, 1, "16th {i} belongs to {kinds} categories");
        }
    }
}
