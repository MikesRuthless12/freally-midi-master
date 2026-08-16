//! The part generators.
//!
//! Each one takes a resolved [`crate::StyleModel`], a
//! [`crate::SessionContext`] and a seed, and returns notes on the grid. Feel is
//! applied afterwards by [`crate::humanize`] — keeping generation and feel
//! apart is what lets a test say "the snare is on beat 3" and mean the tick.

pub mod bass;
pub mod chords;
pub mod counter;
pub mod drums;
pub mod grid;
pub mod melody;
pub mod rolls;

use rand::Rng;
use serde_json::Value;

use crate::context::SessionContext;
use crate::dataset::{NumSpec, StrSpec};
use crate::pattern::LaneTrack;

/// Hold every note inside the pattern it belongs to.
///
/// ⛔⛔ **One rule with four call sites, because it was four rules that happened
/// to agree.** A note may not start past the end of the pattern, and may not
/// *sustain* past it either — what every DAW enforces on a clip's contents, and
/// what any generator writing clips has to enforce itself.
///
/// `bass`, `melody` and `counter` each grew their own copy; `drums` never got
/// one at all, so a hit on the final 16th — a `4a` ghost nudged late by
/// `offGridMs`, a tambourine on the last subdivision — carried its length past
/// the end. That was found on 2026-08-10 only because `coherence.rs` widened its
/// sample; the defect had shipped for months.
///
/// ⚠ **And the copies had already drifted**: two spelled it `.max(1).min(room)`
/// and the newest `.min(room).max(1)`, which differ when `room` is zero. This is
/// the seam all four go through instead — the same argument
/// [`read`] makes above, one level up from the generators rather than copied
/// into each of them. The next generator arrives with the rule rather than
/// without it.
pub fn fit_to_clip(lanes: &mut [LaneTrack], ctx: &SessionContext) {
    let total = ctx.total_ticks();
    for track in lanes {
        track.notes.retain(|note| note.start_tick < total);
        for note in &mut track.notes {
            note.len_ticks = note.len_ticks.min(total - note.start_tick).max(1);
        }
    }
}

/// [`fit_to_clip`] for a generator that returns a single lane.
pub fn fit_track_to_clip(track: &mut LaneTrack, ctx: &SessionContext) {
    fit_to_clip(std::slice::from_mut(track), ctx);
}

/// Reading the dataset's authoring forms.
///
/// Every numeric leaf in a model may be an exact value, an inclusive range or a
/// weighted choice (PRD § 3), and every generator has to cope with all three.
/// That is *dataset* knowledge rather than drum knowledge, so it lives here —
/// one level above the generators — instead of being copied into each of them.
/// The melodic generators arriving in Phase 2 use the same readers.
/// Split a held span into phrase-length re-articulations (TASK-162, TASK-165).
///
/// ⛔⛔ **A part that holds one note per chord collapses over a vamp**, and it
/// did so twice for the same reason. `counter.rs::pad_notes` wrote one note per
/// chord event, so `bnyx` and `clams-casino` — `maxChords: 2`, a `harmonicRhythm`
/// weighted to `vamp` — spent half their seeds on a single held note and reached
/// 959 and 963 distinct counters per 1,000 against a 0.98 floor, after four
/// authoring rounds that each fixed something that was not the cause.
/// `bass.rs`'s `reese_sustain` has the identical shape: `mobb-deep` scored **8
/// distinct basslines in 1,000 seeds**, and every model whose research describes
/// a sustained sub ships as `mirror_kick` instead, recording the substitution in
/// its own `notes`.
///
/// ▶ **The variety comes from *where* it takes the note again**, not from the
/// chord count — which is what the roadmap asks for in both entries.
///
/// ⚠ **Only a span longer than two bars is split, so nothing else moves.** A
/// model whose harmony turns over every bar or two gets exactly the one span it
/// had before this existed.
///
/// ⛔⛔ **Two bars, not four, and four was a silent no-op.** The models author
/// `chordDurationBeats` up to 28–32 beats, which reads as seven or eight bars —
/// but a chord is **clipped to the pattern**, and the pattern is four. So
/// "longer than four bars" was a condition a drone could never meet: it changed
/// no output at all, and the two models still failed the floor the moment their
/// `sustain_pad` weight was put back. Measure against the clip a producer
/// actually generates, not the duration the model asks for.
pub fn phrase_spans(
    ctx: &SessionContext,
    start: u32,
    len: u32,
    rng: &mut impl Rng,
) -> Vec<(u32, u32)> {
    let bar = ctx.ticks_per_bar();
    let end = start + len;
    if bar == 0 || len <= bar * 2 {
        return vec![(start, end)];
    }

    // One, two or four bars, drawn per span: the re-attack point is what differs
    // between seeds once the pitch and the length are exhausted.
    let phrase = bar * [1u32, 2, 4][rng.random_range(0..3)];
    let mut spans = Vec::new();
    let mut at = start;
    while at < end {
        let next = (at + phrase).min(end);
        spans.push((at, next));
        at = next;
    }
    spans
}

pub mod read {
    use super::*;

    /// A block's child, treating an explicit `null` as absent.
    ///
    /// `null` is how a model switches off something `_defaults` gave it — a
    /// country kit saying `"bass808": null`. Without this, a null block reads
    /// as "present with every field defaulted", which is the opposite of what
    /// it says.
    pub fn block<'a>(parent: Option<&'a Value>, key: &str) -> Option<&'a Value> {
        parent
            .and_then(|value| value.get(key))
            .filter(|value| !value.is_null())
    }

    /// A numeric parameter in any of the three authoring forms, or a default.
    pub fn number(block: Option<&Value>, key: &str, default: f64, rng: &mut impl Rng) -> f64 {
        optional_number(block, key, rng).unwrap_or(default)
    }

    /// A numeric parameter, or `None` when the model says nothing.
    ///
    /// Absent and authored-as-zero stay different things: `offbeat8thShare: 0`
    /// means *no* offbeat kicks, while leaving it out means "follow
    /// syncopation".
    pub fn optional_number(block: Option<&Value>, key: &str, rng: &mut impl Rng) -> Option<f64> {
        block
            .and_then(|b| b.get(key))
            .and_then(|v| serde_json::from_value::<NumSpec>(v.clone()).ok())
            .and_then(|spec| spec.sample(rng).ok())
    }

    /// A list of strings — grid positions, lane names, roll vocabularies.
    pub fn strings(block: Option<&Value>, key: &str) -> Vec<String> {
        block
            .and_then(|b| b.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One sample from a categorical parameter, in either authoring form.
    pub fn string_spec(block: Option<&Value>, key: &str, rng: &mut impl Rng) -> Option<String> {
        block
            .and_then(|b| b.get(key))
            .and_then(|v| serde_json::from_value::<StrSpec>(v.clone()).ok())
            .and_then(|spec| spec.sample(rng).ok())
    }

    /// A numeric parameter, leaning to the busy end of what the model authored.
    ///
    /// ⛔ **Only a `[min, max]` range leans** (TASK-125). An exact number is the
    /// model being specific and is returned untouched, and a weighted list is a
    /// set of named choices rather than a spread — leaning either would be
    /// overriding the model instead of scaling within it, which is the one thing
    /// this feature must not do.
    pub fn number_leaning(
        block: Option<&Value>,
        key: &str,
        default: f64,
        complexity: crate::context::Complexity,
        rng: &mut impl Rng,
    ) -> f64 {
        use crate::dataset::schema::NumSpec;

        let spec: Option<NumSpec> = block
            .and_then(|b| b.get(key))
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        match spec {
            Some(NumSpec::Range([min, max])) if max > min => complexity.draw(min, max, rng),
            Some(other) => other.sample(rng).unwrap_or(default),
            None => default,
        }
    }

    /// Sample a categorical parameter, leaning to one end of an authored order.
    ///
    /// ⛔⛔ **This is the whole of TASK-125's "within the model's own ranges"
    /// rule, in one function.** `order` runs plain → busy and is supplied by the
    /// caller, because busy-ness is a fact about the *parameter*: only
    /// `chords.rs` knows that `vamp` is the plainest harmonic rhythm and only
    /// `bass.rs` knows that `mirror_kick` is the plainest bass. What this does is
    /// scale the **authored** weights by position — so a value the model never
    /// listed stays unreachable at every setting, and a model that authors one
    /// value gets that value whatever the producer asked for. A rage vamp made
    /// busy is still a vamp.
    ///
    /// ⚠ **`Authored` returns `string_spec` exactly**, including its draw from
    /// the same rng position, so the switch's default cannot move a saved seed's
    /// beat by a single note.
    ///
    /// ⚠ A value missing from `order` keeps its authored weight rather than being
    /// dropped or pushed to an end — an unknown name is a model saying something
    /// this function was not told about, and guessing where it sits would be
    /// worse than leaving it alone.
    pub fn string_spec_leaning(
        block: Option<&Value>,
        key: &str,
        complexity: crate::context::Complexity,
        order: &[&str],
        rng: &mut impl Rng,
    ) -> Option<String> {
        use crate::dataset::schema::StrSpec;

        let spec: StrSpec = block
            .and_then(|b| b.get(key))
            .and_then(|v| serde_json::from_value(v.clone()).ok())?;

        let StrSpec::Weighted { values, weights } = &spec else {
            return spec.sample(rng).ok();
        };
        if complexity == crate::context::Complexity::Authored || values.len() < 2 {
            return spec.sample(rng).ok();
        }

        // Position each authored value on the plain → busy axis the caller gave,
        // then scale its weight by where it sits.
        let mut scaled: Vec<f64> = match weights {
            Some(w) if w.len() == values.len() => w.clone(),
            _ => vec![1.0; values.len()],
        };
        let mut ranked: Vec<(usize, usize)> = values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                order
                    .iter()
                    .position(|name| name == value)
                    .map(|rank| (index, rank))
            })
            .collect();
        if ranked.len() < 2 {
            return spec.sample(rng).ok();
        }
        ranked.sort_by_key(|(_, rank)| *rank);

        let mut lane: Vec<f64> = ranked.iter().map(|(index, _)| scaled[*index]).collect();
        complexity.reweight(&mut lane);
        for ((index, _), weight) in ranked.iter().zip(lane) {
            scaled[*index] = weight;
        }

        StrSpec::Weighted {
            values: values.clone(),
            weights: Some(scaled),
        }
        .sample(rng)
        .ok()
    }

    /// A two-element numeric array — a register, a velocity range, a ramp.
    pub fn pair(block: Option<&Value>, key: &str) -> Option<(f64, f64)> {
        block
            .and_then(|b| b.get(key))
            .and_then(Value::as_array)
            .filter(|a| a.len() == 2)
            .and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)))
    }

    /// A boolean flag, or a default.
    pub fn flag(block: Option<&Value>, key: &str, default: bool) -> bool {
        block
            .and_then(|b| b.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    /// A plain string field.
    pub fn text<'a>(block: Option<&'a Value>, key: &str) -> Option<&'a str> {
        block.and_then(|b| b.get(key)).and_then(Value::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::rng::root_stream;
    use serde_json::json;

    #[test]
    fn a_null_block_reads_as_absent() {
        // The rule a country kit depends on: `"bass808": null` means there is
        // no 808, not an 808 with every field defaulted.
        let drums = json!({ "bass808": null, "kick": { "densityPerBar": 3 } });
        assert!(read::block(Some(&drums), "bass808").is_none());
        assert!(read::block(Some(&drums), "kick").is_some());
        assert!(read::block(Some(&drums), "hihat").is_none());
    }

    #[test]
    fn numbers_read_in_all_three_authoring_forms() {
        let mut rng = root_stream(1);
        let block = json!({
            "exact": 0.5,
            "range": [2, 4],
            "weighted": { "values": [7, 9], "weights": [1, 0] }
        });
        let block = Some(&block);

        assert_eq!(read::number(block, "exact", 0.0, &mut rng), 0.5);
        let ranged = read::number(block, "range", 0.0, &mut rng);
        assert!((2.0..=4.0).contains(&ranged));
        assert_eq!(read::number(block, "weighted", 0.0, &mut rng), 7.0);
        assert_eq!(read::number(block, "missing", 1.25, &mut rng), 1.25);
    }

    #[test]
    fn absent_and_zero_stay_different() {
        let mut rng = root_stream(2);
        let block = json!({ "share": 0.0 });
        let block = Some(&block);
        assert_eq!(read::optional_number(block, "share", &mut rng), Some(0.0));
        assert_eq!(read::optional_number(block, "other", &mut rng), None);
    }

    #[test]
    fn lists_pairs_flags_and_text_read_or_fall_back() {
        let mut rng = root_stream(3);
        let block = json!({
            "pos": ["1", "2&"],
            "register": [17, 31],
            "muteUnderSnare": true,
            "role": "counter_riff",
            "base": { "values": ["8th"], "weights": [1] }
        });
        let block = Some(&block);

        assert_eq!(read::strings(block, "pos"), vec!["1", "2&"]);
        assert_eq!(read::strings(block, "missing"), Vec::<String>::new());
        assert_eq!(read::pair(block, "register"), Some((17.0, 31.0)));
        assert_eq!(read::pair(block, "pos"), None, "not a numeric pair");
        assert!(read::flag(block, "muteUnderSnare", false));
        assert!(!read::flag(block, "missing", false));
        assert_eq!(read::text(block, "role"), Some("counter_riff"));
        assert_eq!(
            read::string_spec(block, "base", &mut rng),
            Some("8th".to_owned())
        );
    }
}
