//! The part generators.
//!
//! Each one takes a resolved [`crate::StyleModel`], a
//! [`crate::SessionContext`] and a seed, and returns notes on the grid. Feel is
//! applied afterwards by [`crate::humanize`] — keeping generation and feel
//! apart is what lets a test say "the snare is on beat 3" and mean the tick.

pub mod drums;
pub mod grid;
pub mod rolls;

use rand::Rng;
use serde_json::Value;

use crate::dataset::{NumSpec, StrSpec};

/// Reading the dataset's authoring forms.
///
/// Every numeric leaf in a model may be an exact value, an inclusive range or a
/// weighted choice (PRD § 3), and every generator has to cope with all three.
/// That is *dataset* knowledge rather than drum knowledge, so it lives here —
/// one level above the generators — instead of being copied into each of them.
/// The melodic generators arriving in Phase 2 use the same readers.
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
