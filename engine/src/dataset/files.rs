//! Reading a dataset directory off disk.
//!
//! The engine's core is filesystem-free — models arrive as `(path, text)` pairs
//! so tests can use string literals — but the rule for *which* files in a
//! directory are models has to live in exactly one place. `datasetc` validates
//! `data/` in CI and the app loads the same directory at startup; if those two
//! ever disagreed about the file set, CI's green tick would stop saying
//! anything about what the app actually loads.

use std::fs;
use std::path::{Path, PathBuf};

use super::{load, DatasetError, DatasetProblem, LoadedDataset};

/// Directories under a dataset root that hold something other than models:
/// the JSON Schema the models point at, and the sample kits.
const NON_MODEL_DIRS: &[&str] = &["schema", "kits"];

/// What a directory scan found.
#[derive(Debug)]
pub struct Scan {
    /// `(path, text)` for every model file, in path order.
    pub files: Vec<(PathBuf, String)>,
    /// Files that exist but could not be read.
    pub problems: Vec<DatasetProblem>,
}

/// Every `*.json` model under `dir`, recursively, in a deterministic order.
///
/// An unreadable *file* becomes a problem rather than a failure: one file the
/// antivirus has locked must not cost the user the rest of the roster. An
/// unreadable *directory* is a failure, because then there is no dataset at all
/// and the caller needs to say so rather than report an empty one.
pub fn scan(dir: &Path) -> Result<Scan, DatasetError> {
    if !dir.is_dir() {
        return Err(DatasetError::Io {
            path: dir.display().to_string(),
            message: "is not a directory".into(),
        });
    }

    let mut files = Vec::new();
    let mut problems = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).map_err(|e| DatasetError::Io {
            path: current.display().to_string(),
            message: e.to_string(),
        })?;

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !NON_MODEL_DIRS.contains(&name.as_ref()) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "json") {
                match fs::read_to_string(&path) {
                    Ok(text) => files.push((path, text)),
                    Err(e) => problems.push(DatasetProblem {
                        source: path.display().to_string(),
                        message: e.to_string(),
                    }),
                }
            }
        }
    }

    // Directory order is whatever the filesystem says; a stable order is what
    // makes a failure reproducible and the roster identical between launches.
    files.sort_by(|a, b| a.0.cmp(&b.0));
    problems.sort();
    Ok(Scan { files, problems })
}

/// Scan a directory and load everything in it — what the app does at startup.
pub fn load_dir(
    dataset_version: impl Into<String>,
    dir: &Path,
) -> Result<LoadedDataset, DatasetError> {
    let scan = scan(dir)?;
    let mut loaded = load(dataset_version, scan.files);
    // An unreadable file is the same kind of news as an invalid one, so it goes
    // in the same list rather than being dropped on the floor.
    loaded.summary.problems.extend(scan.problems);
    loaded.summary.problems.sort();
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo's real `data/`, which both the CLI and the app load.
    fn data_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
    }

    #[test]
    fn the_shipped_dataset_scans_to_models_only() {
        let scan = scan(&data_dir()).expect("data/ must be readable");
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert!(scan.files.len() >= 4, "expected _defaults plus the genres");

        for (path, _) in &scan.files {
            let text = path.to_string_lossy().replace('\\', "/");
            assert!(
                !text.contains("/schema/") && !text.contains("/kits/"),
                "{text} is not a model and must not be scanned"
            );
        }
    }

    #[test]
    fn the_kit_manifest_is_not_mistaken_for_a_model() {
        // `data/kits/trap-default/kit.json` is a `.json` file that is not a
        // style model. Scanning it would put a rejection in front of the user
        // on every launch, for a file that is doing nothing wrong.
        let scan = scan(&data_dir()).unwrap();
        assert!(
            !scan
                .files
                .iter()
                .any(|(p, _)| p.file_name().is_some_and(|n| n == "kit.json")),
            "kit.json should be excluded by NON_MODEL_DIRS"
        );
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let err = scan(Path::new("definitely/not/here")).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
    }

    #[test]
    fn scanning_is_ordered() {
        let scan = scan(&data_dir()).unwrap();
        let mut sorted: Vec<PathBuf> = scan.files.iter().map(|(p, _)| p.clone()).collect();
        let original = sorted.clone();
        sorted.sort();
        assert_eq!(original, sorted, "the scan must be in path order");
    }

    #[test]
    fn load_dir_produces_a_roster_from_the_real_dataset() {
        let loaded = load_dir("test", &data_dir()).unwrap();
        assert!(loaded.summary.problems.is_empty(), "{:?}", loaded.summary);
        assert_eq!(loaded.summary.dataset_version, "test");
        assert!(loaded.summary.entries.iter().any(|e| e.id == "trap"));
    }
}
