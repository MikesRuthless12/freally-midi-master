//! The StyleModel shape and the weighted-value specs its numeric leaves use.
//!
//! Every numeric leaf in a style model may be written three ways (PRD § 3):
//!
//! ```jsonc
//! "syncopation": 0.5                                   // exact
//! "densityPerBar": [2, 4]                              // inclusive range
//! "vocab": { "values": [...], "weights": [3, 1] }      // weighted choice
//! ```
//!
//! Authors pick whichever reads best for the parameter; the loader normalizes
//! all three into something a generator can sample from with one call.

use std::collections::BTreeMap;

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dataset::DatasetError;

/// A numeric parameter, in any of the three authoring forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumSpec {
    /// `0.5`
    Exact(f64),
    /// `[min, max]`, inclusive.
    Range([f64; 2]),
    /// `{ "values": [...], "weights": [...] }` — weights are relative and
    /// normalized at load; omitting them means uniform.
    Weighted {
        values: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weights: Option<Vec<f64>>,
    },
}

/// A categorical parameter — a scale name, a snare placement, a roll subdivision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrSpec {
    Exact(String),
    Weighted {
        values: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weights: Option<Vec<f64>>,
    },
}

/// Relative weights normalized to sum to 1.
fn normalized(weights: Option<&Vec<f64>>, n: usize) -> Result<Vec<f64>, DatasetError> {
    let raw: Vec<f64> = match weights {
        None => vec![1.0; n],
        Some(w) => {
            if w.len() != n {
                return Err(DatasetError::Lint(format!(
                    "weights has {} entries but values has {n}",
                    w.len()
                )));
            }
            if w.iter().any(|x| *x < 0.0 || !x.is_finite()) {
                return Err(DatasetError::Lint(
                    "weights must be finite and non-negative".into(),
                ));
            }
            w.clone()
        }
    };
    let total: f64 = raw.iter().sum();
    if total <= 0.0 {
        return Err(DatasetError::Lint("weights sum to zero".into()));
    }
    Ok(raw.into_iter().map(|x| x / total).collect())
}

/// Pick an index from normalized weights.
fn pick(weights: &[f64], rng: &mut impl Rng) -> usize {
    let roll: f64 = rng.random_range(0.0..1.0);
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        acc += w;
        if roll < acc {
            return i;
        }
    }
    // Floating-point accumulation can land a hair under 1.0; the last value is
    // the correct answer, not a panic.
    weights.len().saturating_sub(1)
}

impl NumSpec {
    /// Validate the spec without sampling it — used by `datasetc` and CI so a
    /// malformed community model fails the build rather than a user's session.
    pub fn check(&self) -> Result<(), DatasetError> {
        match self {
            NumSpec::Exact(v) => {
                if v.is_finite() {
                    Ok(())
                } else {
                    Err(DatasetError::Lint("value must be finite".into()))
                }
            }
            NumSpec::Range([lo, hi]) => {
                if !lo.is_finite() || !hi.is_finite() {
                    Err(DatasetError::Lint("range bounds must be finite".into()))
                } else if lo > hi {
                    Err(DatasetError::Lint(format!(
                        "range [{lo}, {hi}] is inverted"
                    )))
                } else {
                    Ok(())
                }
            }
            NumSpec::Weighted { values, weights } => {
                if values.is_empty() {
                    return Err(DatasetError::Lint("values is empty".into()));
                }
                normalized(weights.as_ref(), values.len()).map(|_| ())
            }
        }
    }

    pub fn sample(&self, rng: &mut impl Rng) -> Result<f64, DatasetError> {
        match self {
            NumSpec::Exact(v) => Ok(*v),
            NumSpec::Range([lo, hi]) => {
                self.check()?;
                Ok(if lo == hi {
                    *lo
                } else {
                    rng.random_range(*lo..=*hi)
                })
            }
            NumSpec::Weighted { values, weights } => {
                let w = normalized(weights.as_ref(), values.len())?;
                Ok(values[pick(&w, rng)])
            }
        }
    }

    /// The value to show in a readout before anything is generated.
    pub fn nominal(&self) -> f64 {
        match self {
            NumSpec::Exact(v) => *v,
            NumSpec::Range([lo, hi]) => (lo + hi) / 2.0,
            NumSpec::Weighted { values, weights } => {
                let w = normalized(weights.as_ref(), values.len()).unwrap_or_default();
                values.iter().zip(w.iter()).map(|(v, w)| v * w).sum::<f64>()
            }
        }
    }
}

impl StrSpec {
    pub fn check(&self) -> Result<(), DatasetError> {
        match self {
            StrSpec::Exact(_) => Ok(()),
            StrSpec::Weighted { values, weights } => {
                if values.is_empty() {
                    return Err(DatasetError::Lint("values is empty".into()));
                }
                normalized(weights.as_ref(), values.len()).map(|_| ())
            }
        }
    }

    pub fn sample(&self, rng: &mut impl Rng) -> Result<String, DatasetError> {
        match self {
            StrSpec::Exact(v) => Ok(v.clone()),
            StrSpec::Weighted { values, weights } => {
                let w = normalized(weights.as_ref(), values.len())?;
                Ok(values[pick(&w, rng)].clone())
            }
        }
    }
}

/// Tempo, authored as `{ min, max, mode }` because a style's centre of gravity
/// matters as much as its bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BpmSpec {
    pub min: f64,
    pub max: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<f64>,
}

impl BpmSpec {
    pub fn check(&self) -> Result<(), DatasetError> {
        if self.min > self.max {
            return Err(DatasetError::Lint(format!(
                "bpm min {} exceeds max {}",
                self.min, self.max
            )));
        }
        if let Some(mode) = self.mode {
            if mode < self.min || mode > self.max {
                return Err(DatasetError::Lint(format!(
                    "bpm mode {mode} is outside [{}, {}]",
                    self.min, self.max
                )));
            }
        }
        Ok(())
    }

    pub fn nominal(&self) -> f64 {
        self.mode.unwrap_or((self.min + self.max) / 2.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Artist,
    Genre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Flagship,
    Standard,
    Inherited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwingSpec {
    /// `"8th"` or `"16th"`.
    pub grid: String,
    /// 0.50 straight … 0.667 triplet; the musical zone is 0.50–0.70.
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanizeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantize_strength: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_var: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub timing_jitter_ms: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<BpmSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_time: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<StrSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scales: Option<StrSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swing: Option<SwingSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humanize: Option<HumanizeSpec>,
}

/// A style model, resolved or raw.
///
/// Identity and the session block are typed because the app needs them from
/// Phase 1. The per-part blocks (`drums`, `melody`, `chords`, `bassline`,
/// `countermelody`, `arrangement`, `kit`) stay as JSON here and gain typed
/// structs as each generator lands — the JSON Schema is what validates their
/// shape in the meantime, so nothing is unchecked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleModel {
    pub id: String,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    /// Ordered parents. Later entries win over earlier ones; the model itself
    /// wins over all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub era: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Part blocks not yet given typed structs.
    #[serde(flatten)]
    pub blocks: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::root_stream;

    #[test]
    fn the_three_numeric_forms_all_parse() {
        let exact: NumSpec = serde_json::from_str("0.5").unwrap();
        assert_eq!(exact, NumSpec::Exact(0.5));

        let range: NumSpec = serde_json::from_str("[2, 4]").unwrap();
        assert_eq!(range, NumSpec::Range([2.0, 4.0]));

        let weighted: NumSpec =
            serde_json::from_str(r#"{"values":[1,2],"weights":[3,1]}"#).unwrap();
        assert!(matches!(weighted, NumSpec::Weighted { .. }));
    }

    #[test]
    fn an_exact_value_always_samples_to_itself() {
        let mut rng = root_stream(1);
        let spec = NumSpec::Exact(0.25);
        for _ in 0..32 {
            assert_eq!(spec.sample(&mut rng).unwrap(), 0.25);
        }
    }

    #[test]
    fn a_range_stays_inside_its_bounds() {
        let mut rng = root_stream(2);
        let spec = NumSpec::Range([2.0, 4.0]);
        for _ in 0..256 {
            let v = spec.sample(&mut rng).unwrap();
            assert!((2.0..=4.0).contains(&v), "{v} escaped [2, 4]");
        }
    }

    #[test]
    fn a_degenerate_range_is_not_an_error() {
        let mut rng = root_stream(3);
        assert_eq!(NumSpec::Range([3.0, 3.0]).sample(&mut rng).unwrap(), 3.0);
    }

    #[test]
    fn an_inverted_range_is_rejected() {
        assert!(NumSpec::Range([4.0, 2.0]).check().is_err());
    }

    #[test]
    fn weights_follow_their_declared_ratio() {
        let spec = NumSpec::Weighted {
            values: vec![0.0, 1.0],
            weights: Some(vec![3.0, 1.0]),
        };
        let mut rng = root_stream(4);
        let n = 4000;
        let ones = (0..n)
            .filter(|_| spec.sample(&mut rng).unwrap() == 1.0)
            .count();
        let ratio = ones as f64 / n as f64;
        assert!((ratio - 0.25).abs() < 0.03, "expected ~0.25, got {ratio}");
    }

    #[test]
    fn omitting_weights_means_uniform() {
        let spec = NumSpec::Weighted {
            values: vec![0.0, 1.0],
            weights: None,
        };
        let mut rng = root_stream(5);
        let n = 4000;
        let ones = (0..n)
            .filter(|_| spec.sample(&mut rng).unwrap() == 1.0)
            .count();
        let ratio = ones as f64 / n as f64;
        assert!((ratio - 0.5).abs() < 0.03, "expected ~0.5, got {ratio}");
    }

    #[test]
    fn a_zero_weight_value_is_never_chosen() {
        let spec = NumSpec::Weighted {
            values: vec![7.0, 9.0],
            weights: Some(vec![1.0, 0.0]),
        };
        let mut rng = root_stream(6);
        for _ in 0..500 {
            assert_eq!(spec.sample(&mut rng).unwrap(), 7.0);
        }
    }

    #[test]
    fn malformed_weights_are_rejected_rather_than_sampled() {
        // Length mismatch.
        assert!(NumSpec::Weighted {
            values: vec![1.0],
            weights: Some(vec![1.0, 2.0])
        }
        .check()
        .is_err());
        // All zero.
        assert!(NumSpec::Weighted {
            values: vec![1.0],
            weights: Some(vec![0.0])
        }
        .check()
        .is_err());
        // Negative.
        assert!(NumSpec::Weighted {
            values: vec![1.0, 2.0],
            weights: Some(vec![-1.0, 2.0])
        }
        .check()
        .is_err());
        // Empty.
        assert!(NumSpec::Weighted {
            values: vec![],
            weights: None
        }
        .check()
        .is_err());
    }

    #[test]
    fn nominal_reports_a_useful_readout_value() {
        assert_eq!(NumSpec::Exact(140.0).nominal(), 140.0);
        assert_eq!(NumSpec::Range([130.0, 150.0]).nominal(), 140.0);
        let w = NumSpec::Weighted {
            values: vec![0.0, 4.0],
            weights: Some(vec![3.0, 1.0]),
        };
        assert_eq!(w.nominal(), 1.0);
    }

    #[test]
    fn string_specs_parse_and_sample() {
        let exact: StrSpec = serde_json::from_str(r#""halftime_3""#).unwrap();
        let mut rng = root_stream(7);
        assert_eq!(exact.sample(&mut rng).unwrap(), "halftime_3");

        let weighted: StrSpec =
            serde_json::from_str(r#"{"values":["a","b"],"weights":[1,0]}"#).unwrap();
        for _ in 0..100 {
            assert_eq!(weighted.sample(&mut rng).unwrap(), "a");
        }
    }

    #[test]
    fn bpm_bounds_and_mode_are_checked() {
        assert!(BpmSpec {
            min: 130.0,
            max: 170.0,
            mode: Some(140.0)
        }
        .check()
        .is_ok());
        assert!(BpmSpec {
            min: 170.0,
            max: 130.0,
            mode: None
        }
        .check()
        .is_err());
        assert!(BpmSpec {
            min: 130.0,
            max: 170.0,
            mode: Some(200.0)
        }
        .check()
        .is_err());
        assert_eq!(
            BpmSpec {
                min: 130.0,
                max: 150.0,
                mode: None
            }
            .nominal(),
            140.0
        );
    }

    #[test]
    fn sampling_is_reproducible_from_a_seed() {
        let spec = NumSpec::Range([0.0, 1.0]);
        let a: Vec<f64> = (0..16)
            .map({
                let mut rng = root_stream(99);
                move |_| spec.sample(&mut rng).unwrap()
            })
            .collect();
        let spec2 = NumSpec::Range([0.0, 1.0]);
        let b: Vec<f64> = (0..16)
            .map({
                let mut rng = root_stream(99);
                move |_| spec2.sample(&mut rng).unwrap()
            })
            .collect();
        assert_eq!(a, b);
    }
}
