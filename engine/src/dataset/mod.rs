//! The style dataset: loading `data/`, resolving inheritance, validating.
//!
//! Models are read as plain JSON, deep-merged along their `extends` chain, and
//! only then parsed into typed form. Merging before typing is what lets a genre
//! archetype and an artist share one shape without every field being optional
//! twice over.

pub mod files;
pub mod inherit;
pub mod schema;
pub mod validate;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub use schema::{
    BpmSpec, Confidence, HumanizeSpec, ModelType, NumSpec, SessionSpec, StrSpec, StyleModel,
    SwingSpec, Tier,
};
pub use validate::Finding;

/// Everything that can go wrong loading or resolving the dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetError {
    /// `extends` names a model that is not in the registry.
    UnknownParent(String),
    /// An inheritance loop, rendered as the path that closes it.
    Cycle(String),
    /// The JSON does not match the model shape.
    Shape(String),
    /// A semantic problem the shape alone cannot catch.
    Lint(String),
    /// The file could not be read or parsed.
    Io { path: String, message: String },
}

impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatasetError::UnknownParent(id) => write!(f, "unknown parent model `{id}`"),
            DatasetError::Cycle(path) => write!(f, "inheritance cycle: {path}"),
            DatasetError::Shape(m) => write!(f, "model shape: {m}"),
            DatasetError::Lint(m) => write!(f, "{m}"),
            DatasetError::Io { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for DatasetError {}

/// A model that failed to load, kept so the UI can show a badge count rather
/// than the app refusing to start (FR-001).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedModel {
    pub path: PathBuf,
    pub error: DatasetError,
}

/// Raw models by id, plus whatever was rejected on the way in.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    models: BTreeMap<String, Value>,
    rejected: Vec<RejectedModel>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a raw model. The id comes from the model's own `id` field, because
    /// that is what `extends` refers to — not the filename.
    pub fn insert(&mut self, path: &Path, model: Value) -> Result<(), DatasetError> {
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DatasetError::Io {
                path: path.display().to_string(),
                message: "model has no `id`".into(),
            })?
            .to_owned();

        if let Some(existing) = self.models.get(&id) {
            let existing_name = existing.get("name").and_then(Value::as_str).unwrap_or("?");
            return Err(DatasetError::Io {
                path: path.display().to_string(),
                message: format!("duplicate id `{id}` (already defined by `{existing_name}`)"),
            });
        }

        self.models.insert(id, model);
        Ok(())
    }

    pub fn reject(&mut self, path: PathBuf, error: DatasetError) {
        self.rejected.push(RejectedModel { path, error });
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.models.keys()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    pub fn raw(&self, id: &str) -> Option<&Value> {
        self.models.get(id)
    }

    pub fn rejected(&self) -> &[RejectedModel] {
        &self.rejected
    }

    /// Resolve one model through its inheritance chain and parse it.
    pub fn resolve(&self, id: &str) -> Result<StyleModel, DatasetError> {
        let merged = inherit::resolve(id, &self.models)?;
        let findings = validate::lint(&merged);
        if let Some(first) = findings.first() {
            return Err(DatasetError::Lint(format!(
                "{first}{}",
                if findings.len() > 1 {
                    format!(" (and {} more)", findings.len() - 1)
                } else {
                    String::new()
                }
            )));
        }
        validate::parse(merged)
    }

    /// Resolve every model, collecting failures instead of stopping at the
    /// first — a broken model must not hide the others.
    pub fn resolve_all(&self) -> (BTreeMap<String, StyleModel>, Vec<(String, DatasetError)>) {
        let mut ok = BTreeMap::new();
        let mut errors = Vec::new();
        for id in self.models.keys() {
            match self.resolve(id) {
                Ok(model) => {
                    ok.insert(id.clone(), model);
                }
                Err(e) => errors.push((id.clone(), e)),
            }
        }
        (ok, errors)
    }
}

/// Build a registry from in-memory `(path, json)` pairs.
///
/// Reading the files is the caller's job so the engine stays filesystem-free at
/// its core — `src-tauri` loads from bundled resources, `datasetc` from disk,
/// and tests from string literals.
pub fn registry_from(entries: impl IntoIterator<Item = (PathBuf, String)>) -> Registry {
    let mut registry = Registry::new();
    for (path, text) in entries {
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => {
                if let Err(e) = registry.insert(&path, value) {
                    registry.reject(path, e);
                }
            }
            Err(e) => {
                let err = DatasetError::Io {
                    path: path.display().to_string(),
                    message: format!("invalid JSON at line {}: {e}", e.line()),
                };
                registry.reject(path, err);
            }
        }
    }
    registry
}

/// One entry in the searchable roster (PRD § 3 Indexes, § 4 `roster_summary`).
///
/// Everything here except identity is read from the model's **own** file rather
/// than from its resolved form. Inheritance is for musical parameters: merging
/// metadata would hand every artist their genre archetype's aliases, so typing
/// one alias would surface every artist that happens to extend it. `id`, `name`
/// and `type` come from the resolved model, which `inherit` already guarantees
/// are the model's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
#[serde(rename_all = "camelCase")]
pub struct RosterEntry {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    pub tier: Option<Tier>,
    pub genres: Vec<String>,
    pub era: Option<String>,
}

/// A model the app could not use, in the form the UI reports it (FR-001).
///
/// A list rather than a count, because a badge saying "3" tells a user nothing
/// they can act on — the file and the reason do.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
#[serde(rename_all = "camelCase")]
pub struct DatasetProblem {
    /// The file it came from, or the model id when the failure was in the merge.
    pub source: String,
    pub message: String,
}

/// What `roster_summary` returns (PRD § 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
#[serde(rename_all = "camelCase")]
pub struct RosterSummary {
    pub dataset_version: String,
    pub entries: Vec<RosterEntry>,
    pub problems: Vec<DatasetProblem>,
}

/// A completed startup load: the roster the UI lists, the resolved models the
/// generators read, and everything that was skipped on the way in.
#[derive(Debug, Clone)]
pub struct LoadedDataset {
    pub summary: RosterSummary,
    pub models: BTreeMap<String, StyleModel>,
}

/// Ids beginning with `_` are internal bases — `_defaults` is the root every
/// model inherits from, not something a user can generate from. They resolve
/// and are addressable by `resolve_model`; they are simply not offered.
fn is_internal(id: &str) -> bool {
    id.starts_with('_')
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The roster row for a model that has already resolved cleanly.
fn roster_entry(model: &StyleModel, own: &Value) -> RosterEntry {
    RosterEntry {
        id: model.id.clone(),
        name: model.name.clone(),
        model_type: model.model_type,
        aliases: string_list(own.get("aliases")),
        // The whole model parsed as a `StyleModel` to get here, so a `tier` that
        // is present is a valid one.
        tier: own
            .get("tier")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        genres: string_list(own.get("genres")),
        era: own.get("era").and_then(Value::as_str).map(str::to_owned),
    }
}

/// Load a whole dataset: parse, resolve, and build the roster.
///
/// Nothing here fails. A model that will not parse, resolve or lint is skipped
/// and recorded as a problem, because one bad file must not cost the user the
/// other nine hundred (FR-001) — `datasetc` is what makes that same file fail
/// CI, before it ever ships.
///
/// `dataset_version` is the caller's to supply: only the app knows where the
/// models came from.
pub fn load(
    dataset_version: impl Into<String>,
    files: impl IntoIterator<Item = (PathBuf, String)>,
) -> LoadedDataset {
    let registry = registry_from(files);
    let (models, errors) = registry.resolve_all();

    let mut problems: Vec<DatasetProblem> = registry
        .rejected()
        .iter()
        .map(|r| DatasetProblem {
            source: r.path.display().to_string(),
            message: r.error.to_string(),
        })
        .chain(errors.into_iter().map(|(id, error)| DatasetProblem {
            source: id,
            message: error.to_string(),
        }))
        .collect();
    // Rejections come in file order and resolution failures in id order; sorting
    // the union keeps the badge list stable between launches.
    problems.sort();

    let entries = models
        .iter()
        .filter(|(id, _)| !is_internal(id))
        .filter_map(|(id, model)| Some(roster_entry(model, registry.raw(id)?)))
        .collect();

    LoadedDataset {
        summary: RosterSummary {
            dataset_version: dataset_version.into(),
            entries,
            problems,
        },
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, json: &str) -> (PathBuf, String) {
        (PathBuf::from(name), json.to_owned())
    }

    #[test]
    fn a_registry_indexes_by_the_models_own_id() {
        let reg = registry_from(vec![entry(
            "anything.json",
            r#"{"id":"trap","type":"genre","name":"Trap"}"#,
        )]);
        assert_eq!(reg.len(), 1);
        assert!(reg.raw("trap").is_some());
    }

    #[test]
    fn invalid_json_is_rejected_with_its_path_and_line() {
        let reg = registry_from(vec![entry("broken.json", "{ not json")]);
        assert_eq!(reg.len(), 0);
        assert_eq!(reg.rejected().len(), 1);
        let msg = reg.rejected()[0].error.to_string();
        assert!(msg.contains("broken.json"), "{msg}");
    }

    #[test]
    fn a_model_without_an_id_is_rejected() {
        let reg = registry_from(vec![entry("x.json", r#"{"type":"genre","name":"X"}"#)]);
        assert_eq!(reg.rejected().len(), 1);
        assert!(reg.rejected()[0].error.to_string().contains("no `id`"));
    }

    #[test]
    fn a_duplicate_id_is_rejected_rather_than_silently_overwriting() {
        let reg = registry_from(vec![
            entry("a.json", r#"{"id":"trap","type":"genre","name":"First"}"#),
            entry("b.json", r#"{"id":"trap","type":"genre","name":"Second"}"#),
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.rejected().len(), 1);
        assert!(reg.rejected()[0].error.to_string().contains("duplicate"));
        // The first one wins and keeps its name.
        assert_eq!(reg.raw("trap").unwrap()["name"], "First");
    }

    #[test]
    fn one_broken_model_does_not_hide_the_others() {
        let reg = registry_from(vec![
            entry("ok.json", r#"{"id":"good","type":"genre","name":"Good"}"#),
            entry("bad.json", "{{{"),
        ]);
        let (ok, errors) = reg.resolve_all();
        assert_eq!(ok.len(), 1);
        assert!(errors.is_empty());
        assert_eq!(reg.rejected().len(), 1);
    }

    #[test]
    fn resolve_runs_the_lints() {
        let reg = registry_from(vec![entry(
            "bad.json",
            r#"{"id":"x","type":"genre","name":"X","melody":{"register":[60,200]}}"#,
        )]);
        match reg.resolve("x") {
            Err(DatasetError::Lint(m)) => assert!(m.contains("0–127"), "{m}"),
            other => panic!("expected a lint error, got {other:?}"),
        }
    }

    #[test]
    fn resolve_all_reports_every_failure_without_stopping() {
        let reg = registry_from(vec![
            entry(
                "a.json",
                r#"{"id":"a","type":"genre","name":"A","melody":{"register":[0,999]}}"#,
            ),
            entry("b.json", r#"{"id":"b","type":"genre","name":"B"}"#),
            entry(
                "c.json",
                r#"{"id":"c","type":"artist","name":"C","extends":["nope"]}"#,
            ),
        ]);
        let (ok, errors) = reg.resolve_all();
        assert_eq!(ok.len(), 1, "only b is clean");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn the_roster_lists_every_usable_model_and_hides_the_internal_bases() {
        let loaded = load(
            "0.1.0",
            vec![
                entry(
                    "_defaults.json",
                    r#"{"id":"_defaults","type":"genre","name":"Defaults"}"#,
                ),
                entry(
                    "trap.json",
                    r#"{"id":"trap","type":"genre","name":"Trap","extends":["_defaults"]}"#,
                ),
            ],
        );

        let ids: Vec<&str> = loaded
            .summary
            .entries
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, ["trap"], "_defaults is a base, not a choice");
        // It still resolves, so anything asking for it by id gets it.
        assert!(loaded.models.contains_key("_defaults"));
        assert_eq!(loaded.summary.dataset_version, "0.1.0");
    }

    #[test]
    fn roster_metadata_is_the_models_own_and_never_its_parents() {
        // The bug this prevents: an artist inheriting their genre's aliases, so
        // searching one alias returns every artist who extends that genre.
        let loaded = load(
            "0.1.0",
            vec![
                entry(
                    "trap.json",
                    r#"{"id":"trap","type":"genre","name":"Trap","aliases":["trap music"],
                        "genres":["trap"],"era":"2010s","tier":"standard"}"#,
                ),
                entry(
                    "artist.json",
                    r#"{"id":"osamason","type":"artist","name":"OsamaSon","extends":["trap"],
                        "aliases":["osama"],"tier":"flagship"}"#,
                ),
            ],
        );

        let artist = loaded
            .summary
            .entries
            .iter()
            .find(|e| e.id == "osamason")
            .expect("the artist should be in the roster");

        assert_eq!(artist.aliases, ["osama"]);
        assert_eq!(artist.model_type, ModelType::Artist);
        assert_eq!(artist.tier, Some(Tier::Flagship));
        assert_eq!(artist.era, None, "era is a claim, not an inheritance");
        assert!(artist.genres.is_empty());

        // The resolved model, by contrast, does inherit — that is what it is
        // for, and it is why the roster cannot be built from it.
        assert_eq!(loaded.models["osamason"].genres, ["trap"]);
        assert_eq!(loaded.models["osamason"].aliases, ["osama"]);
    }

    #[test]
    fn a_broken_model_is_a_problem_and_the_rest_still_load() {
        let loaded = load(
            "0.1.0",
            vec![
                entry("good.json", r#"{"id":"good","type":"genre","name":"Good"}"#),
                entry("torn.json", "{ not json"),
                entry(
                    "cyclic.json",
                    r#"{"id":"cyclic","type":"genre","name":"C","extends":["nope"]}"#,
                ),
            ],
        );

        assert_eq!(loaded.summary.entries.len(), 1);
        assert_eq!(loaded.summary.entries[0].id, "good");
        assert_eq!(loaded.summary.problems.len(), 2);

        // A parse failure can only name the file; a merge failure names the
        // model, because by then the file it came from is behind us.
        let sources: Vec<&str> = loaded
            .summary
            .problems
            .iter()
            .map(|p| p.source.as_str())
            .collect();
        assert!(sources.contains(&"torn.json"), "{sources:?}");
        assert!(sources.contains(&"cyclic"), "{sources:?}");
    }

    #[test]
    fn problems_are_ordered_so_the_badge_does_not_reshuffle() {
        let files = vec![
            entry("z.json", "{{{"),
            entry("a.json", "{{{"),
            entry("m.json", "{{{"),
        ];
        let sources: Vec<String> = load("0.1.0", files)
            .summary
            .problems
            .into_iter()
            .map(|p| p.source)
            .collect();
        assert_eq!(sources, ["a.json", "m.json", "z.json"]);
    }

    #[test]
    fn errors_render_for_humans() {
        assert_eq!(
            DatasetError::UnknownParent("rage".into()).to_string(),
            "unknown parent model `rage`"
        );
        assert_eq!(
            DatasetError::Cycle("a -> b -> a".into()).to_string(),
            "inheritance cycle: a -> b -> a"
        );
    }
}
