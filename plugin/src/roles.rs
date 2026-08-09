//! Guessing which lane a sample belongs on, from its filename (TASK-050).
//!
//! ⛔ **A guess, and the UI must treat it as one.** Assignment in this plugin is
//! explicit today — a producer clicks a lane and picks a file — and that stays
//! the truth of it. This exists so that *dropping a folder's worth of samples*
//! has a sensible starting point, and so that "randomise this pad from the
//! selected folder" (TASK-050A) has some way to know which files are candidates
//! for a snare. A wrong guess a producer can override in one click is useful; a
//! wrong guess that silently *becomes* the assignment is not, which is why
//! nothing here assigns anything.
//!
//! ## Why tokens rather than a substring sweep
//!
//! `808_Cmaj.wav` is an 808. `kick_808_layer.wav` is a **kick** — it says so
//! first, and a plain "does the name contain 808" would take it. So the name is
//! split into words and the *earliest* strong match wins, with the specific
//! tokens beating the general ones at the same position: `openhat` is an open
//! hat before it is a hat, and `sub kick` is a sub-kick before it is a kick.
//!
//! ⚠ **ASCII, case-folded, and everything else is a separator.** Sample packs
//! are named by humans with spaces, hyphens, underscores, brackets and numbers,
//! in every combination. Anything that is not a letter or a digit ends a word.

use engine::pattern::Lane;

/// One rule: the token that names a lane, and how specific it is.
///
/// `weight` breaks a tie at the same position — `openhat` (2) beats `hat` (1)
/// when a name contains both at once, which `open_hat_01.wav` does.
struct Rule {
    token: &'static str,
    lane: Lane,
    weight: u8,
}

/// ⛔ **Ordered by specificity within a token length, and read in full every
/// time.** A first-match-wins list would make the order load-bearing in a way
/// nobody could see; scoring makes the rule explicit and the tests can pin it.
const RULES: &[Rule] = &[
    // ── The most specific names first, so a compound token is not eaten by its
    // own suffix.
    Rule {
        token: "openhat",
        lane: Lane::OpenHat,
        weight: 3,
    },
    Rule {
        token: "ohat",
        lane: Lane::OpenHat,
        weight: 3,
    },
    Rule {
        token: "closedhat",
        lane: Lane::ClosedHat,
        weight: 3,
    },
    Rule {
        token: "chat",
        lane: Lane::ClosedHat,
        weight: 3,
    },
    Rule {
        token: "pedalhat",
        lane: Lane::PedalHat,
        weight: 3,
    },
    Rule {
        token: "subkick",
        lane: Lane::SubKick,
        weight: 3,
    },
    Rule {
        token: "ridebell",
        lane: Lane::RideBell,
        weight: 3,
    },
    Rule {
        token: "offsnare",
        lane: Lane::OffSnare,
        weight: 3,
    },
    Rule {
        token: "ghostsnare",
        lane: Lane::GhostSnare,
        weight: 3,
    },
    Rule {
        token: "rimshot",
        lane: Lane::Rim,
        weight: 3,
    },
    Rule {
        token: "crosstick",
        lane: Lane::Rim,
        weight: 3,
    },
    Rule {
        token: "hihat",
        lane: Lane::ClosedHat,
        weight: 2,
    },
    Rule {
        token: "open",
        lane: Lane::OpenHat,
        weight: 1,
    },
    // ── Plain names.
    Rule {
        token: "kick",
        lane: Lane::Kick,
        weight: 2,
    },
    Rule {
        token: "bd",
        lane: Lane::Kick,
        weight: 1,
    },
    Rule {
        token: "snare",
        lane: Lane::Snare,
        weight: 2,
    },
    Rule {
        token: "sd",
        lane: Lane::Snare,
        weight: 1,
    },
    Rule {
        token: "clap",
        lane: Lane::Clap,
        weight: 2,
    },
    Rule {
        token: "cp",
        lane: Lane::Clap,
        weight: 1,
    },
    Rule {
        token: "snap",
        lane: Lane::Snap,
        weight: 2,
    },
    Rule {
        token: "finger",
        lane: Lane::Snap,
        weight: 1,
    },
    Rule {
        token: "hat",
        lane: Lane::ClosedHat,
        weight: 1,
    },
    Rule {
        token: "hh",
        lane: Lane::ClosedHat,
        weight: 1,
    },
    Rule {
        token: "ride",
        lane: Lane::Ride,
        weight: 2,
    },
    Rule {
        token: "crash",
        lane: Lane::Crash,
        weight: 2,
    },
    Rule {
        token: "cymbal",
        lane: Lane::Crash,
        weight: 1,
    },
    Rule {
        token: "rim",
        lane: Lane::Rim,
        weight: 2,
    },
    Rule {
        token: "shaker",
        lane: Lane::Shaker,
        weight: 2,
    },
    Rule {
        token: "tamb",
        lane: Lane::Tambourine,
        weight: 2,
    },
    Rule {
        token: "tambourine",
        lane: Lane::Tambourine,
        weight: 3,
    },
    Rule {
        token: "cowbell",
        lane: Lane::Cowbell,
        weight: 2,
    },
    Rule {
        token: "woodblock",
        lane: Lane::Woodblock,
        weight: 2,
    },
    Rule {
        token: "clave",
        lane: Lane::Clave,
        weight: 2,
    },
    Rule {
        token: "conga",
        lane: Lane::Conga,
        weight: 2,
    },
    Rule {
        token: "bongo",
        lane: Lane::Bongo,
        weight: 2,
    },
    Rule {
        token: "timbale",
        lane: Lane::Timbale,
        weight: 2,
    },
    Rule {
        token: "triangle",
        lane: Lane::Triangle,
        weight: 2,
    },
    Rule {
        token: "tomhigh",
        lane: Lane::TomHigh,
        weight: 3,
    },
    Rule {
        token: "hitom",
        lane: Lane::TomHigh,
        weight: 3,
    },
    Rule {
        token: "tomlow",
        lane: Lane::TomLow,
        weight: 3,
    },
    Rule {
        token: "lowtom",
        lane: Lane::TomLow,
        weight: 3,
    },
    Rule {
        token: "floortom",
        lane: Lane::TomLow,
        weight: 3,
    },
    Rule {
        token: "tom",
        lane: Lane::Tom,
        weight: 2,
    },
    Rule {
        token: "riser",
        lane: Lane::Riser,
        weight: 2,
    },
    Rule {
        token: "impact",
        lane: Lane::Impact,
        weight: 2,
    },
    Rule {
        token: "reverse",
        lane: Lane::Reverse,
        weight: 2,
    },
    Rule {
        token: "perc",
        lane: Lane::Perc,
        weight: 1,
    },
    // ── The 808, last among the drums: it is the token most likely to appear
    // inside a name that is really something else. `kick_808_layer` is a kick.
    Rule {
        token: "808",
        lane: Lane::Sub,
        weight: 2,
    },
    Rule {
        token: "sub",
        lane: Lane::Sub,
        weight: 1,
    },
    Rule {
        token: "bass",
        lane: Lane::Bass,
        weight: 1,
    },
    // ── The melodic lanes, so a folder of loops lands somewhere useful.
    Rule {
        token: "lead",
        lane: Lane::Melody,
        weight: 2,
    },
    Rule {
        token: "melody",
        lane: Lane::Melody,
        weight: 2,
    },
    Rule {
        token: "pluck",
        lane: Lane::Melody,
        weight: 1,
    },
    Rule {
        token: "bell",
        lane: Lane::Melody,
        weight: 1,
    },
    Rule {
        token: "chord",
        lane: Lane::Chords,
        weight: 2,
    },
    Rule {
        token: "pad",
        lane: Lane::Chords,
        weight: 1,
    },
    Rule {
        token: "keys",
        lane: Lane::Chords,
        weight: 1,
    },
    Rule {
        token: "arp",
        lane: Lane::Counter,
        weight: 2,
    },
    Rule {
        token: "counter",
        lane: Lane::Counter,
        weight: 2,
    },
];

/// The words in a filename, lower-cased, with everything else as a separator.
///
/// The extension is dropped: `.wav` is not a role, and `wav` as a token would
/// match nothing today but is exactly the sort of thing a later rule trips on.
fn tokens(name: &str) -> Vec<String> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.rsplit_once('.').map(|(head, _)| head).unwrap_or(stem);

    let mut out = Vec::new();
    let mut word = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() {
            word.push(ch.to_ascii_lowercase());
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Which lane this filename is probably for, if anything about it says so.
///
/// `None` is a real answer and the common one: most samples in a pack are named
/// after the pack rather than the sound, and a guess pulled out of nothing is
/// worse than admitting there was nothing to go on.
pub fn guess(name: &str) -> Option<Lane> {
    let words = tokens(name);

    let mut best: Option<(usize, u8, Lane)> = None;
    let mut consider = |at: usize, weight: u8, lane: Lane| {
        let better = match best {
            // Earlier in the name wins: a producer writes the role first.
            Some((best_at, best_weight, _)) => {
                at < best_at || (at == best_at && weight > best_weight)
            }
            None => true,
        };
        if better {
            best = Some((at, weight, lane));
        }
    };

    for (at, word) in words.iter().enumerate() {
        for rule in RULES {
            // ⚠ `contains`, not equality: pack names run words together
            // (`kick01`, `OH_clean`), and the token list is specific enough that
            // a substring match inside one word is still about that word.
            if word.contains(rule.token) {
                consider(at, rule.weight, rule.lane);
            }
        }

        // ⛔ **Adjacent pairs, but only for tokens that genuinely span the
        // join.** `sub kick 3.wav` is a sub-kick and reading its words one at a
        // time answers "sub" — the 808 — because that word comes first.
        //
        // ⚠ The guard is what stops this becoming a worse bug than the one it
        // fixes: joining `808` and `kick` makes a string containing *both*
        // roles, and without the check it would match `kick` at position 0 and
        // decide `808_kick_sub.wav` is a kick. A pair may only satisfy a token
        // that neither half satisfies alone.
        let Some(next) = words.get(at + 1) else {
            continue;
        };
        let joined = format!("{word}{next}");
        for rule in RULES {
            if joined.contains(rule.token)
                && !word.contains(rule.token)
                && !next.contains(rule.token)
            {
                consider(at, rule.weight, rule.lane);
            }
        }
    }

    best.map(|(_, _, lane)| lane)
}

/// Every file in `names` that could be this lane's, in the order given.
///
/// What "randomise this pad from the selected folder" (TASK-050A) draws from.
pub fn candidates(lane: Lane, names: &[String]) -> Vec<&String> {
    names
        .iter()
        .filter(|name| guess(name) == Some(lane))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roadmaps_own_example_lands_where_it_says() {
        // The verify line of TASK-050, word for word: `"808_Cmaj.wav" lands on
        // the 808 pad`.
        assert_eq!(guess("808_Cmaj.wav"), Some(Lane::Sub));
    }

    #[test]
    fn the_role_a_name_states_first_is_the_one_that_wins() {
        // ⛔ The failure a plain substring sweep has: this file is a **kick**
        // with an 808 layered under it, and a sweep for "808" takes it.
        assert_eq!(guess("kick_808_layer.wav"), Some(Lane::Kick));
        assert_eq!(guess("808_kick_sub.wav"), Some(Lane::Sub));
    }

    #[test]
    fn a_compound_name_beats_its_own_suffix() {
        assert_eq!(guess("open_hat_01.wav"), Some(Lane::OpenHat));
        assert_eq!(guess("OpenHat 04.wav"), Some(Lane::OpenHat));
        assert_eq!(guess("closed hat.wav"), Some(Lane::ClosedHat));
        assert_eq!(guess("sub kick 3.wav"), Some(Lane::SubKick));
        assert_eq!(guess("low tom.aiff"), Some(Lane::TomLow));
    }

    #[test]
    fn the_separators_a_sample_pack_actually_uses_all_work() {
        for name in [
            "Snare.wav",
            "SNARE_03.wav",
            "snare-hard.wav",
            "01 snare (dry).wav",
            "Vol2/Drums/snare.wav",
            "snare02.wav",
        ] {
            assert_eq!(guess(name), Some(Lane::Snare), "{name}");
        }
    }

    #[test]
    fn a_name_that_says_nothing_is_answered_with_nothing() {
        // ⛔ The common case in a real pack, and a guess pulled out of nothing
        // is worse than admitting there was nothing to go on.
        for name in [
            "Untitled-1.wav",
            "SP_0042.wav",
            "loop.wav",
            "final FINAL.wav",
        ] {
            assert_eq!(guess(name), None, "{name}");
        }
    }

    #[test]
    fn a_path_is_read_by_its_filename_and_not_by_its_folders() {
        // ⚠ Otherwise a pack in a folder called `Kicks` would make every file
        // in it a kick, including the snares.
        assert_eq!(guess("C:/Packs/Kicks/snare.wav"), Some(Lane::Snare));
        assert_eq!(guess("/home/mike/hats/clap 01.wav"), Some(Lane::Clap));
    }

    #[test]
    fn candidates_are_the_files_that_could_be_that_lane_and_only_those() {
        let names: Vec<String> = ["kick 1.wav", "snare.wav", "kick 2.wav", "nothing.wav"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();

        let kicks = candidates(Lane::Kick, &names);
        assert_eq!(kicks.len(), 2);
        assert!(kicks.iter().all(|name| name.contains("kick")));
        assert!(candidates(Lane::Riser, &names).is_empty());
    }
}
