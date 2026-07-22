//! Dataset validation: JSON Schema first, then the lints the schema cannot
//! express.
//!
//! A malformed community model must fail CI, never crash a user's session
//! (PRD § 2 Security), so everything here reports the offending file and the
//! JSON pointer rather than panicking.

use serde_json::Value;

use crate::dataset::{schema::StyleModel, DatasetError};

/// A validation problem, located.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// JSON pointer into the model, e.g. `/drums/kick/densityPerBar`.
    pub pointer: String,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = if self.pointer.is_empty() {
            "/"
        } else {
            &self.pointer
        };
        write!(f, "{at}: {}", self.message)
    }
}

/// The MIDI note range every register/pitch field must live inside.
const MIDI_MIN: f64 = 0.0;
const MIDI_MAX: f64 = 127.0;

/// Field names whose values are MIDI note numbers.
///
/// `colorTones` is deliberately absent: those are scale degrees (2, 4, 6), not
/// note numbers, and range-checking them against 0–127 would be meaningless.
const REGISTER_KEYS: &[&str] = &["register"];

/// Field names carrying a probability.
const PROBABILITY_SUFFIXES: &[&str] = &["Prob", "Bias", "Ratio", "Strength", "Var"];

/// Structural + semantic checks that the JSON Schema cannot express.
///
/// The schema catches shape; this catches meaning — an inverted range, weights
/// that do not line up with their values, a register outside MIDI, a BPM whose
/// mode sits outside its own bounds.
pub fn lint(model: &Value) -> Vec<Finding> {
    let mut findings = Vec::new();
    walk(model, String::new(), &mut findings);

    // BPM is authored as its own shape, so check it directly.
    if let Some(bpm) = model.pointer("/session/bpm") {
        if let Ok(spec) = serde_json::from_value::<crate::dataset::schema::BpmSpec>(bpm.clone()) {
            if let Err(e) = spec.check() {
                findings.push(Finding {
                    pointer: "/session/bpm".into(),
                    message: e.to_string(),
                });
            }
        }
    }

    // Swing outside the musical zone is almost always a typo — 0.5–0.7 is the
    // usable range on an MPC-style scale (research ch. 1 constants).
    if let Some(amount) = model
        .pointer("/session/swing/amount")
        .and_then(Value::as_f64)
    {
        if !(0.5..=0.75).contains(&amount) {
            findings.push(Finding {
                pointer: "/session/swing/amount".into(),
                message: format!("swing {amount} is outside the musical zone 0.50–0.75"),
            });
        }
    }

    findings
}

fn walk(value: &Value, pointer: String, out: &mut Vec<Finding>) {
    match value {
        Value::Object(map) => {
            // A `{values, weights}` node is a weighted spec wherever it appears.
            if map.contains_key("values") {
                check_weighted(map, &pointer, out);
                return;
            }
            for (k, v) in map {
                let child = format!("{pointer}/{k}");
                if REGISTER_KEYS.contains(&k.as_str()) {
                    check_midi(v, &child, out);
                }
                if PROBABILITY_SUFFIXES.iter().any(|s| k.ends_with(s)) {
                    check_probability(v, &child, out);
                }
                walk(v, child, out);
            }
        }
        Value::Array(items) => {
            // A two-number array is a range wherever it appears.
            if let (2, Some(lo), Some(hi)) = (
                items.len(),
                items.first().and_then(Value::as_f64),
                items.get(1).and_then(Value::as_f64),
            ) {
                if lo > hi {
                    out.push(Finding {
                        pointer: pointer.clone(),
                        message: format!("range [{lo}, {hi}] is inverted"),
                    });
                }
            }
            for (i, v) in items.iter().enumerate() {
                walk(v, format!("{pointer}/{i}"), out);
            }
        }
        _ => {}
    }
}

fn check_weighted(map: &serde_json::Map<String, Value>, pointer: &str, out: &mut Vec<Finding>) {
    let values = match map.get("values").and_then(Value::as_array) {
        Some(v) => v,
        None => {
            out.push(Finding {
                pointer: pointer.into(),
                message: "values must be an array".into(),
            });
            return;
        }
    };

    if values.is_empty() {
        out.push(Finding {
            pointer: pointer.into(),
            message: "values is empty".into(),
        });
        return;
    }

    let Some(weights) = map.get("weights") else {
        return;
    };
    let Some(weights) = weights.as_array() else {
        out.push(Finding {
            pointer: format!("{pointer}/weights"),
            message: "weights must be an array".into(),
        });
        return;
    };

    if weights.len() != values.len() {
        out.push(Finding {
            pointer: format!("{pointer}/weights"),
            message: format!(
                "{} weights for {} values — they must line up",
                weights.len(),
                values.len()
            ),
        });
        return;
    }

    let nums: Vec<f64> = weights.iter().filter_map(Value::as_f64).collect();
    if nums.len() != weights.len() {
        out.push(Finding {
            pointer: format!("{pointer}/weights"),
            message: "every weight must be a number".into(),
        });
        return;
    }
    if nums.iter().any(|w| *w < 0.0) {
        out.push(Finding {
            pointer: format!("{pointer}/weights"),
            message: "weights must be non-negative".into(),
        });
    }
    if nums.iter().sum::<f64>() <= 0.0 {
        out.push(Finding {
            pointer: format!("{pointer}/weights"),
            message: "weights sum to zero — nothing could ever be chosen".into(),
        });
    }
}

fn check_midi(value: &Value, pointer: &str, out: &mut Vec<Finding>) {
    let mut bad = |n: f64, at: String| {
        if !(MIDI_MIN..=MIDI_MAX).contains(&n) {
            out.push(Finding {
                pointer: at,
                message: format!("MIDI note {n} is outside 0–127"),
            });
        }
    };
    match value {
        Value::Number(n) => {
            if let Some(v) = n.as_f64() {
                bad(v, pointer.into());
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if let Some(v) = item.as_f64() {
                    bad(v, format!("{pointer}/{i}"));
                }
            }
        }
        _ => {}
    }
}

fn check_probability(value: &Value, pointer: &str, out: &mut Vec<Finding>) {
    if let Some(v) = value.as_f64() {
        if !(0.0..=1.0).contains(&v) {
            out.push(Finding {
                pointer: pointer.into(),
                message: format!("probability {v} is outside 0.0–1.0"),
            });
        }
    }
}

/// Parse a model into its typed form, surfacing serde's own path in the error.
pub fn parse(value: &Value) -> Result<StyleModel, DatasetError> {
    serde_json::from_value(value.clone()).map_err(|e| DatasetError::Shape(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_clean_model_produces_no_findings() {
        let model = json!({
            "id": "trap", "type": "genre", "name": "Trap",
            "session": { "bpm": { "min": 130, "max": 170, "mode": 140 },
                         "swing": { "grid": "16th", "amount": 0.5 } },
            "drums": { "kick": { "densityPerBar": [2, 5], "syncopation": 0.5 } },
            "melody": { "register": [60, 84] }
        });
        assert_eq!(lint(&model), vec![]);
    }

    #[test]
    fn an_inverted_range_is_found_with_its_pointer() {
        let model = json!({ "drums": { "kick": { "densityPerBar": [5, 2] } } });
        let found = lint(&model);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pointer, "/drums/kick/densityPerBar");
        assert!(found[0].message.contains("inverted"));
    }

    #[test]
    fn mismatched_weights_are_found() {
        let model = json!({ "session": { "keys": { "values": ["Fm", "Gm"], "weights": [1] } } });
        let found = lint(&model);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("line up"));
    }

    #[test]
    fn weights_that_sum_to_zero_are_found() {
        let model = json!({ "a": { "values": ["x", "y"], "weights": [0, 0] } });
        let found = lint(&model);
        assert!(
            found.iter().any(|f| f.message.contains("sum to zero")),
            "{found:?}"
        );
    }

    #[test]
    fn negative_weights_are_found() {
        let model = json!({ "a": { "values": ["x", "y"], "weights": [-1, 2] } });
        assert!(lint(&model)
            .iter()
            .any(|f| f.message.contains("non-negative")));
    }

    #[test]
    fn a_register_outside_midi_is_found() {
        let model = json!({ "melody": { "register": [60, 200] } });
        let found = lint(&model);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("0–127"));
        assert_eq!(found[0].pointer, "/melody/register/1");
    }

    #[test]
    fn a_probability_outside_zero_to_one_is_found() {
        let model = json!({ "drums": { "bass808": { "slideProb": 1.5 } } });
        let found = lint(&model);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("0.0–1.0"));
    }

    #[test]
    fn bpm_with_a_mode_outside_its_bounds_is_found() {
        let model = json!({ "session": { "bpm": { "min": 130, "max": 150, "mode": 170 } } });
        assert!(lint(&model).iter().any(|f| f.pointer == "/session/bpm"));
    }

    #[test]
    fn swing_outside_the_musical_zone_is_found() {
        let model = json!({ "session": { "swing": { "grid": "16th", "amount": 0.2 } } });
        assert!(lint(&model)
            .iter()
            .any(|f| f.message.contains("musical zone")));
    }

    #[test]
    fn a_weighted_node_is_not_walked_as_a_range() {
        // `values: [2, 5]` is a choice between 2 and 5, not the range [2, 5];
        // it must not be range-checked, and reversing it is legal.
        let model = json!({ "a": { "values": [5, 2], "weights": [1, 1] } });
        assert_eq!(lint(&model), vec![]);
    }

    #[test]
    fn findings_render_readably() {
        let f = Finding {
            pointer: "/drums/kick".into(),
            message: "boom".into(),
        };
        assert_eq!(f.to_string(), "/drums/kick: boom");
    }

    #[test]
    fn a_model_missing_a_required_field_fails_to_parse() {
        let err = parse(&json!({ "type": "genre", "name": "No id" }));
        assert!(matches!(err, Err(DatasetError::Shape(_))));
    }

    #[test]
    fn a_valid_model_parses_into_its_typed_form() {
        let model = parse(&json!({
            "id": "trap", "type": "genre", "name": "Trap",
            "session": { "bpm": { "min": 130, "max": 170, "mode": 140 } },
            "drums": { "kick": { "syncopation": 0.5 } }
        }))
        .unwrap();
        assert_eq!(model.id, "trap");
        assert_eq!(model.session.unwrap().bpm.unwrap().nominal(), 140.0);
        // Untyped part blocks survive the round trip.
        assert!(model.blocks.contains_key("drums"));
    }
}
