//! The style dataset, embedded in the plugin binary.
//!
//! The desktop app read `data/` from Tauri's resource directory. A plugin has
//! no such thing — it is a shared library loaded into someone else's process,
//! with no install layout it can rely on and no working directory it chose.
//! So the dataset is compiled in.
//!
//! That is affordable here and would not be for audio: 28 JSON files, ~536 KB,
//! parsed once on first use. Phase 5's user-supplied models will need a real
//! search path beside this one, and this module is where that goes.

use std::path::PathBuf;
use std::sync::OnceLock;

use engine::dataset::files::NON_MODEL_DIRS;
use engine::dataset::{DatasetProblem, LoadedDataset, RosterSummary, StyleModel};
use include_dir::{include_dir, Dir};

/// The dataset's version is the plugin's, until user models exist (Phase 5).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Compiled in at build time. The path is relative to this crate's root.
static DATA: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../data");

/// Parsed once, on whichever thread asks first.
///
/// A `OnceLock` rather than eager work in `initialize`: hosts call that on the
/// audio thread in at least one DAW, and parsing 28 JSON files there would be
/// a dropout. The editor is what needs the roster, and it asks from the UI
/// thread.
static LOADED: OnceLock<LoadedDataset> = OnceLock::new();

/// Every embedded model file, as the registry wants them.
fn entries() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect(&DATA, &mut files);

    // `include_dir` makes no ordering promise, and inheritance resolution must
    // not depend on which file the compiler happened to walk first — the
    // on-disk loader sorts for exactly this reason.
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Walk the embedded tree, taking every `.json` that is a model.
///
/// Hand-rolled rather than `Dir::find`, which needs `include_dir`'s `glob`
/// feature: one recursive function is cheaper than a dependency feature and
/// does not silently change behaviour if that feature is ever dropped.
///
/// `NON_MODEL_DIRS` comes from the engine rather than being restated here.
/// The first version of this function filtered on `.json` alone and duly
/// loaded `data/schema/artist-style.schema.json` as a model — two problems in
/// the roster on the first run. `data/kits/` would have been next.
fn collect(dir: &Dir<'_>, out: &mut Vec<(PathBuf, String)>) {
    for file in dir.files() {
        if file.path().extension().is_some_and(|ext| ext == "json") {
            // A file that is not UTF-8 cannot be a model. Skipped rather than
            // panicking, which is what `files::scan` does with an unreadable
            // one.
            if let Some(text) = file.contents_utf8() {
                out.push((file.path().to_path_buf(), text.to_owned()));
            }
        }
    }

    for sub in dir.dirs() {
        let name = sub
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if !NON_MODEL_DIRS.contains(&name.as_str()) {
            collect(sub, out);
        }
    }
}

/// The resolved dataset, loading it on first use.
pub fn loaded() -> &'static LoadedDataset {
    LOADED.get_or_init(|| {
        let entries = entries();
        if entries.is_empty() {
            // Reported rather than logged and forgotten: a plugin showing an
            // empty roster with nothing wrong anywhere is indistinguishable
            // from one whose dataset failed to embed, and the second is a bug
            // someone has to be able to see.
            return LoadedDataset {
                summary: RosterSummary {
                    dataset_version: VERSION.into(),
                    entries: Vec::new(),
                    problems: vec![DatasetProblem {
                        source: "data".into(),
                        message: "no models were embedded in the plugin".into(),
                    }],
                },
                models: Default::default(),
            };
        }

        // The same loader the desktop app and `datasetc` use. Building the
        // roster here instead would be a second implementation of the rules
        // about which ids are offered and how problems are reported, and the
        // two would drift.
        engine::dataset::load(VERSION, entries)
    })
}

/// The factory presets, as `(file stem, contents)`.
///
/// Read straight out of the already-embedded `data/`, rather than a second
/// `include_dir!` over `data/presets` — a second one would compile the whole
/// dataset into the binary twice, and the binary is already 23 MB.
///
/// `data/presets/` is in [`NON_MODEL_DIRS`], so [`collect`] steps over it and
/// these files never reach the model loader. Without that they would be scanned
/// as style models and every one of them would show up as a dataset problem in
/// front of the user — which is exactly what `data/schema/` did the first time.
pub fn factory_presets() -> Vec<(String, &'static str)> {
    let Some(dir) = DATA.get_dir("presets") else {
        return Vec::new();
    };

    let mut found: Vec<(String, &'static str)> = dir
        .files()
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|file| {
            let stem = file.path().file_stem()?.to_string_lossy().into_owned();
            Some((stem, file.contents_utf8()?))
        })
        .collect();

    // `include_dir` promises no order, and the preset list is shown to a user.
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// One resolved model, inheritance already applied.
pub fn model(id: &str) -> Result<StyleModel, String> {
    loaded()
        .models
        .get(id)
        .cloned()
        .ok_or_else(|| format!("no style model with id `{id}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dataset_is_embedded_and_resolves() {
        // The failure this catches is a build one, not a logic one: get the
        // `include_dir!` path wrong and the plugin ships with an empty roster
        // and no error anywhere.
        let loaded = loaded();
        assert!(
            loaded.summary.problems.is_empty(),
            "{:?}",
            loaded.summary.problems
        );
        assert!(
            loaded.summary.entries.iter().any(|e| e.id == "trap"),
            "the shipped genres should be offered"
        );
        assert!(loaded.models.len() >= 26, "got {}", loaded.models.len());
    }

    #[test]
    fn the_embedded_copy_matches_the_one_on_disk() {
        // Two loaders for one dataset is how they start to disagree. The
        // on-disk path is what every engine test uses; this asserts the
        // compiled-in copy resolves to the same models.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).unwrap();
        let (on_disk, _) = engine::dataset::registry_from(scan.files).resolve_all();

        let embedded = &loaded().models;
        assert_eq!(
            on_disk.keys().collect::<Vec<_>>(),
            embedded.keys().collect::<Vec<_>>(),
            "the embedded dataset holds different models than data/"
        );
        for (id, model) in &on_disk {
            assert_eq!(embedded.get(id), Some(model), "{id} resolved differently");
        }
    }

    #[test]
    fn the_defaults_base_is_loadable_but_not_offered() {
        assert!(!loaded().summary.entries.iter().any(|e| e.id == "_defaults"));
        assert!(model("_defaults").is_ok());
    }

    #[test]
    fn an_unknown_id_is_an_error_and_not_an_empty_model() {
        let err = model("no-such-artist").unwrap_err();
        assert!(err.contains("no-such-artist"), "{err}");
    }
}
