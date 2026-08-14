//! The style dataset: loading `data/`, resolving inheritance, validating.
//!
//! Models are read as plain JSON, deep-merged along their `extends` chain, and
//! only then parsed into typed form. Merging before typing is what lets a genre
//! archetype and an artist share one shape without every field being optional
//! twice over.

pub mod files;
pub mod inherit;
pub mod modes;
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
        self.resolve_over(id, None)
    }

    /// Resolve one model over `base` instead of over what it `extends`
    /// (TASK-158C) — see [`inherit::resolve_over`] for what that is and is not.
    ///
    /// ⚠ **The same lint and the same parse.** A swapped model is a model: if
    /// the combination produces something the linter refuses, it is refused
    /// here rather than generated from. That is the whole reason this goes
    /// through `resolve` rather than merging blocks somewhere downstream.
    pub fn resolve_over(&self, id: &str, base: Option<&str>) -> Result<StyleModel, DatasetError> {
        let merged = inherit::resolve_over(id, base, &self.models)?;
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
    /// Ids of the genre models this one works in, for cross-filtering the roster.
    ///
    /// Empty for a genre, and for an artist nobody has curated yet. ⛔ Not the
    /// same thing as `genres`, which is free-text tags in a vocabulary of its
    /// own — `rap`, `drill` — that name no model at all. Every id here has been
    /// checked to name a real `genre`; see [`unknown_related_genres`].
    pub related_genres: Vec<String>,
    pub era: Option<String>,
    /// The producer's own model rather than a shipped one (TASK-040U).
    ///
    /// ⛔ **Set by whoever loaded it, not by the loader.** The engine has no idea
    /// where a file came from — `load` is handed `(path, text)` pairs and its
    /// rules are the same for all of them, which is exactly what makes a user
    /// model first-class. The app knows which half it read from disk, so the app
    /// is what marks them; this defaults to `false` and the plugin flips it.
    ///
    /// It carries no behaviour. A user model searches, generates, locks and
    /// re-rolls identically — this is only so the roster can *say* it is yours.
    #[serde(default)]
    pub mine: bool,
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
    /// The **unresolved** models, kept so a base swap costs one resolve.
    ///
    /// ⛔⛔ **This is what stops TASK-158C being unaffordable.** Resolving an
    /// artist over a different genre needs the child's *own* body, and a
    /// resolved model no longer has one — its parent is already merged in. The
    /// alternative the roadmap costed it at was a deep clone of all 590
    /// documents per generation, on the editor thread; keeping the raw registry
    /// once turns that into a single linearize and a handful of merges.
    ///
    /// ⚠ **Smaller than [`Self::models`], not larger.** These are the authored
    /// bodies — an artist file is a few dozen lines — where the resolved ones
    /// each carry the whole of `_defaults` and their genre merged in.
    pub registry: Registry,
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
        related_genres: string_list(own.get("relatedGenres")),
        era: own.get("era").and_then(Value::as_str).map(str::to_owned),
        // Not the loader's to know — see the field's own note.
        mine: false,
    }
}

/// The `relatedGenres` ids in a model's own JSON that do not name a genre.
///
/// ⛔ **Read from the model's own file, never from its resolved form** — the
/// same rule, and the same reason, as [`roster_entry`]'s other metadata. A
/// resolved model is a deep merge of its parents, so every artist under `trap`
/// would otherwise claim whatever relations `trap` claimed.
///
/// Naming an *artist* is as wrong as naming nothing, and is the easier mistake
/// to make: `pluggnb` is a genre and `summrs` is not, and nothing in how an id
/// is spelled says which. Both come back here.
///
/// The **policy** is the caller's, which is why this only reports. `load` drops
/// the ids and records a problem, so one bad file cannot cost the user the rest;
/// `datasetc` fails CI on the same finding before it can ship.
pub fn unknown_related_genres(own: &Value, models: &BTreeMap<String, StyleModel>) -> Vec<String> {
    string_list(own.get("relatedGenres"))
        .into_iter()
        .filter(|id| {
            !matches!(
                models.get(id).map(|model| model.model_type),
                Some(ModelType::Genre)
            )
        })
        .collect()
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

    // ⛔ A `relatedGenres` id naming nothing is **dropped** from the roster
    // rather than carried into it. The rails filter on these, so a dangling id
    // arrives as a genre that hides every artist — which reads as the app being
    // broken rather than as a dataset mistake, and hides the mistake too.
    let mut dangling: Vec<DatasetProblem> = Vec::new();
    let entries: Vec<RosterEntry> = models
        .iter()
        .filter(|(id, _)| !is_internal(id))
        .filter_map(|(id, model)| {
            let own = registry.raw(id)?;
            let mut entry = roster_entry(model, own);
            let unknown = unknown_related_genres(own, &models);
            if !unknown.is_empty() {
                entry.related_genres.retain(|g| !unknown.contains(g));
                dangling.push(DatasetProblem {
                    source: id.clone(),
                    message: format!(
                        "`relatedGenres` names no genre model: {}",
                        unknown.join(", ")
                    ),
                });
            }
            Some(entry)
        })
        .collect();

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
        .chain(dangling)
        .collect();
    // Rejections come in file order and resolution failures in id order; sorting
    // the union keeps the badge list stable between launches.
    problems.sort();

    LoadedDataset {
        summary: RosterSummary {
            dataset_version: dataset_version.into(),
            entries,
            problems,
        },
        models,
        // ⚠ Moved rather than cloned — it has done its work above and nothing
        // else here needs it.
        registry,
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

    /// Two genres and one artist relating to both, as the shipped dataset does.
    fn cross_filtered(artist: &str) -> LoadedDataset {
        load(
            "0.1.0",
            vec![
                entry(
                    "plugg.json",
                    r#"{"id":"plugg","type":"genre","name":"Plugg"}"#,
                ),
                entry("rage.json", r#"{"id":"rage","type":"genre","name":"Rage"}"#),
                entry("artist.json", artist),
            ],
        )
    }

    fn related_of(loaded: &LoadedDataset, id: &str) -> Vec<String> {
        loaded
            .summary
            .entries
            .iter()
            .find(|e| e.id == id)
            .expect("the model should be in the roster")
            .related_genres
            .clone()
    }

    #[test]
    fn an_artist_carries_the_genres_it_works_in_in_authored_order() {
        // Order is the author's: the first entry is normally the model's own
        // `extends` parent, and the rail shows them in the order given.
        let loaded = cross_filtered(
            r#"{"id":"osamason","type":"artist","name":"OsamaSon","extends":["rage"],
                "relatedGenres":["rage","plugg"]}"#,
        );
        assert_eq!(related_of(&loaded, "osamason"), ["rage", "plugg"]);
        // A genre relates to nobody from its own side; the rail inverts the
        // artists' lists to answer "who works in this genre".
        assert!(related_of(&loaded, "rage").is_empty());
    }

    #[test]
    fn a_related_genre_that_names_nothing_is_dropped_and_reported() {
        // ⛔ Dropped rather than kept: the rails filter on these, so a dangling
        // id arrives in the UI as a genre that hides every artist — which reads
        // as the app being broken and hides the dataset mistake behind it.
        let loaded = cross_filtered(
            r#"{"id":"osamason","type":"artist","name":"OsamaSon",
                "relatedGenres":["rage","nope"]}"#,
        );
        assert_eq!(related_of(&loaded, "osamason"), ["rage"]);

        let problem = loaded
            .summary
            .problems
            .iter()
            .find(|p| p.source == "osamason")
            .expect("the dangling id should be reported");
        assert!(problem.message.contains("nope"), "{}", problem.message);
    }

    #[test]
    fn a_related_genre_pointing_at_an_artist_is_refused_like_a_missing_one() {
        // The easier mistake of the two, and invisible in the id: `pluggnb` is
        // a genre and `summrs` is not, and nothing in the spelling says which.
        let loaded = load(
            "0.1.0",
            vec![
                entry(
                    "plugg.json",
                    r#"{"id":"plugg","type":"genre","name":"Plugg"}"#,
                ),
                entry(
                    "summrs.json",
                    r#"{"id":"summrs","type":"artist","name":"Summrs"}"#,
                ),
                entry(
                    "artist.json",
                    r#"{"id":"osamason","type":"artist","name":"OsamaSon",
                        "relatedGenres":["plugg","summrs"]}"#,
                ),
            ],
        );

        assert_eq!(related_of(&loaded, "osamason"), ["plugg"]);
        assert!(loaded
            .summary
            .problems
            .iter()
            .any(|p| p.source == "osamason"));
    }

    #[test]
    fn related_genres_are_the_models_own_and_never_inherited() {
        // The same rule as every other roster field, and it matters more here:
        // inheriting would give every artist under `plugg` whatever relations
        // `plugg` carried, so one authored line would silently relate dozens.
        let loaded = load(
            "0.1.0",
            vec![
                entry("rage.json", r#"{"id":"rage","type":"genre","name":"Rage"}"#),
                entry(
                    "plugg.json",
                    r#"{"id":"plugg","type":"genre","name":"Plugg","relatedGenres":["rage"]}"#,
                ),
                entry(
                    "artist.json",
                    r#"{"id":"summrs","type":"artist","name":"Summrs","extends":["plugg"]}"#,
                ),
            ],
        );

        assert!(
            related_of(&loaded, "summrs").is_empty(),
            "the artist authored none of its own"
        );
        // The resolved model does inherit, which is exactly why the roster is
        // built from the raw file rather than from this.
        assert_eq!(loaded.models["summrs"].related_genres, ["rage"]);
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
