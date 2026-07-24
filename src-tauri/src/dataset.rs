//! The style dataset, loaded once at startup (TASK-016, FR-001).
//!
//! `data/` is bundled as a Tauri resource, read from the resource directory on
//! launch, and resolved into memory. Everything after this — search, the detail
//! pane, every generator — reads what this module loaded, so the app pays the
//! cost once rather than on each keystroke.
//!
//! Nothing here can fail the launch. A model that will not parse is skipped and
//! listed as a problem; a whole dataset that will not load leaves an empty
//! roster and one problem saying why. `datasetc` in CI is what stops a broken
//! model from ever reaching a user (FR-001 AC).

use std::collections::BTreeMap;
use std::path::Path;

use engine::dataset::{files, DatasetProblem, LoadedDataset, RosterSummary, StyleModel};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};

/// The dataset ships inside the app, so its version is the app's version.
///
/// This stops being true the moment user-supplied models land (Phase 5), which
/// is why the roster carries the field at all rather than the UI assuming it.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The resolved dataset, held for the life of the process.
pub struct Dataset(LoadedDataset);

impl Dataset {
    /// An empty dataset that says why it is empty.
    ///
    /// Reported as a problem rather than logged and forgotten: an app showing an
    /// empty roster with nothing wrong anywhere is indistinguishable from one
    /// whose dataset failed to load, and the second is a bug someone has to be
    /// able to see.
    fn unavailable(message: String) -> Self {
        eprintln!("dataset: {message}");
        Dataset(LoadedDataset {
            summary: RosterSummary {
                dataset_version: VERSION.into(),
                entries: Vec::new(),
                problems: vec![DatasetProblem {
                    source: "data".into(),
                    message,
                }],
            },
            models: BTreeMap::new(),
        })
    }

    fn summary(&self) -> &RosterSummary {
        &self.0.summary
    }

    /// One resolved model, inheritance already applied.
    fn model(&self, id: &str) -> Result<StyleModel, String> {
        self.0
            .models
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no style model with id `{id}`"))
    }
}

/// Load the bundled `data/` directory.
pub fn load(app: &AppHandle) -> Dataset {
    match app.path().resolve("data", BaseDirectory::Resource) {
        Ok(dir) => load_from(&dir),
        Err(e) => Dataset::unavailable(format!("could not locate the bundled dataset: {e}")),
    }
}

fn load_from(dir: &Path) -> Dataset {
    match files::load_dir(VERSION, dir) {
        Ok(loaded) => {
            eprintln!(
                "dataset: {} models, {} in the roster, {} problem(s), from {}",
                loaded.models.len(),
                loaded.summary.entries.len(),
                loaded.summary.problems.len(),
                dir.display()
            );
            for problem in &loaded.summary.problems {
                eprintln!("dataset: skipped {} — {}", problem.source, problem.message);
            }
            Dataset(loaded)
        }
        Err(e) => Dataset::unavailable(format!("{e}")),
    }
}

/// The roster the UI searches and browses (PRD § 4).
#[tauri::command]
pub fn roster_summary(dataset: State<'_, Dataset>) -> RosterSummary {
    dataset.summary().clone()
}

/// One model, resolved through its inheritance chain, for the detail pane.
#[tauri::command]
pub fn resolve_model(id: String, dataset: State<'_, Dataset>) -> Result<StyleModel, String> {
    dataset.model(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The repo's `data/`, which is what gets bundled as the resource.
    fn repo_data() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
    }

    #[test]
    fn the_shipped_dataset_loads_into_a_roster() {
        let dataset = load_from(&repo_data());
        let summary = dataset.summary();

        assert!(summary.problems.is_empty(), "{:?}", summary.problems);
        assert!(
            summary.entries.iter().any(|e| e.id == "trap"),
            "the seed genres should be offered: {:?}",
            summary.entries
        );
        assert_eq!(summary.dataset_version, VERSION);
    }

    #[test]
    fn the_defaults_base_is_loadable_but_not_offered() {
        let dataset = load_from(&repo_data());

        assert!(
            !dataset
                .summary()
                .entries
                .iter()
                .any(|e| e.id == "_defaults"),
            "`_defaults` is the root every model inherits from, not a style"
        );
        // It still resolves by id, which is what makes it debuggable.
        assert!(dataset.model("_defaults").is_ok());
    }

    #[test]
    fn a_missing_dataset_is_reported_rather_than_fatal() {
        // The launch path when the resource is missing from the bundle. It must
        // produce an app with an empty roster and a visible reason, never a
        // panic on the way to the first frame.
        let dataset = load_from(Path::new("definitely/not/here"));

        assert!(dataset.summary().entries.is_empty());
        assert_eq!(dataset.summary().problems.len(), 1);
        assert!(
            dataset.summary().problems[0]
                .message
                .contains("not a directory"),
            "{:?}",
            dataset.summary().problems
        );
    }

    #[test]
    fn the_dataset_is_bundled_as_a_resource() {
        // `load` reads BaseDirectory::Resource, which only has `data/` in it
        // because tauri.conf.json says to put it there. Delete that entry and
        // every test above still passes — they read the repo copy directly —
        // while the shipped app launches with an empty roster. Nothing else in
        // the suite can see that, so this reads the config.
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(manifest_dir.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();

        let resources = config["bundle"]["resources"]
            .as_object()
            .expect("bundle.resources must map source paths to targets");

        let (source, target) = resources
            .iter()
            .find(|(_, target)| target.as_str() == Some("data"))
            .expect("`data` must be bundled as a resource, or the app ships no models");
        assert_eq!(target, "data", "the app resolves the resource by that name");

        // And the source has to be a real directory of models, not a path that
        // merely looks right.
        let dir = manifest_dir.join(source);
        assert!(
            dir.join("_defaults.json").is_file(),
            "{} does not hold the dataset",
            dir.display()
        );
    }

    #[test]
    fn an_unknown_id_is_an_error_and_not_an_empty_model() {
        let dataset = load_from(&repo_data());
        let err = dataset.model("no-such-artist").unwrap_err();
        assert!(err.contains("no-such-artist"), "{err}");
    }
}
