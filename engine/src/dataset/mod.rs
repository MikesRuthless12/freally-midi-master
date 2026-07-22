//! The style dataset: loading `data/`, resolving inheritance, validating.
//!
//! Models are read as plain JSON, deep-merged along their `extends` chain, and
//! only then parsed into typed form. Merging before typing is what lets a genre
//! archetype and an artist share one shape without every field being optional
//! twice over.

pub mod inherit;
pub mod schema;
pub mod validate;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

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
        validate::parse(&merged)
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
