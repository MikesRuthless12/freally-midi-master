//! End-to-end checks on the `datasetc` binary.
//!
//! These run the real executable against the real `data/` directory, because
//! the thing being guaranteed is "CI fails when the dataset is broken" — and
//! that is a property of the process's exit code, not of any function.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // tools/datasetc -> tools -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root should resolve")
}

fn datasetc(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_datasetc"))
        .args(args)
        .output()
        .expect("datasetc should run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A scratch copy of the dataset that cleans itself up.
struct Scratch(PathBuf);

impl Scratch {
    fn with_dataset(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("datasetc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(&repo_root().join("data"), &dir);
        Scratch(dir)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().filter_map(Result::ok) {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn validate_passes_on_the_shipped_dataset() {
    let root = repo_root();
    let data = root.join("data");
    let (ok, stdout, stderr) = datasetc(&["validate", data.to_str().unwrap()]);
    assert!(
        ok,
        "validate should pass on data/\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("ok:"), "{stdout}");
}

#[test]
fn validate_fails_and_names_the_file_and_pointer() {
    let scratch = Scratch::with_dataset("bad");
    scratch.write(
        "genres/broken.json",
        r#"{
          "$schema": "../schema/artist-style.schema.json",
          "id": "broken", "type": "genre", "name": "Broken",
          "extends": ["_defaults"],
          "melody": { "register": [60, 200] }
        }"#,
    );

    let (ok, _stdout, stderr) = datasetc(&["validate", scratch.path()]);
    assert!(!ok, "a corrupt model must fail validation");
    assert!(
        stderr.contains("broken.json"),
        "should name the file: {stderr}"
    );
    assert!(
        stderr.contains("/melody/register/1"),
        "should give the JSON pointer: {stderr}"
    );
}

#[test]
fn a_bad_enum_is_caught_by_the_schema() {
    let scratch = Scratch::with_dataset("enum");
    scratch.write(
        "genres/wrong-type.json",
        r#"{"id":"wrong-type","type":"subgenre","name":"Wrong"}"#,
    );

    let (ok, _out, stderr) = datasetc(&["validate", scratch.path()]);
    assert!(!ok);
    assert!(stderr.contains("/type"), "{stderr}");
}

#[test]
fn an_inheritance_cycle_fails_validation() {
    let scratch = Scratch::with_dataset("cycle");
    scratch.write(
        "genres/cycle-a.json",
        r#"{"id":"cycle-a","type":"genre","name":"A","extends":["cycle-b"]}"#,
    );
    scratch.write(
        "genres/cycle-b.json",
        r#"{"id":"cycle-b","type":"genre","name":"B","extends":["cycle-a"]}"#,
    );

    let (ok, _out, stderr) = datasetc(&["validate", scratch.path()]);
    assert!(!ok, "a cycle must fail validation");
    assert!(stderr.to_lowercase().contains("cycle"), "{stderr}");
}

#[test]
fn a_file_is_reported_once_however_many_checks_it_fails() {
    let scratch = Scratch::with_dataset("grouped");
    scratch.write(
        "genres/many.json",
        r#"{
          "id": "Bad Id", "type": "subgenre", "name": "Many",
          "melody": { "register": [60, 200], "densityPerBar": [8, 3] }
        }"#,
    );

    let (ok, _out, stderr) = datasetc(&["validate", scratch.path()]);
    assert!(!ok);
    let headings = stderr.matches("many.json").count();
    assert_eq!(
        headings, 1,
        "the file should get one FAIL heading, not one per check phase:\n{stderr}"
    );
}

#[test]
fn stats_reports_counts() {
    let root = repo_root();
    let (ok, stdout, _err) = datasetc(&["stats", root.join("data").to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("By type"), "{stdout}");
    assert!(stdout.contains("By genre"), "{stdout}");
    assert!(stdout.contains("cites its sources"), "{stdout}");
}

#[test]
fn coverage_shows_declared_versus_inherited() {
    let scratch = Scratch::with_dataset("coverage");
    // A model that declares only drums must show the rest as inherited.
    scratch.write(
        "genres/sparse.json",
        r#"{"id":"sparse","type":"genre","name":"Sparse","extends":["_defaults"],
            "drums":{"kick":{"syncopation":0.4}}}"#,
    );

    let (ok, stdout, err) = datasetc(&["coverage", scratch.path()]);
    assert!(ok, "{err}");
    let line = stdout
        .lines()
        .find(|l| l.starts_with("sparse"))
        .unwrap_or_else(|| panic!("no row for sparse:\n{stdout}"));
    assert!(line.contains('●'), "should mark what it declares: {line}");
    assert!(line.contains('○'), "should mark what it inherits: {line}");
}

#[test]
fn help_succeeds_and_lists_the_commands() {
    for args in [vec!["--help"], vec!["-h"], vec![]] {
        let (ok, stdout, _err) = datasetc(&args);
        assert!(ok, "help should exit 0 for {args:?}");
        for command in ["validate", "lint", "stats", "coverage"] {
            assert!(stdout.contains(command), "help should list {command}");
        }
    }
}

#[test]
fn an_unknown_command_fails_rather_than_doing_something_surprising() {
    let (ok, _out, stderr) = datasetc(&["destroy", "data"]);
    assert!(!ok);
    assert!(stderr.contains("unknown command"), "{stderr}");
}

#[test]
fn a_missing_directory_is_an_error_not_a_pass() {
    let (ok, _out, stderr) = datasetc(&["validate", "definitely/not/here"]);
    assert!(!ok, "a missing dataset must not silently pass");
    assert!(stderr.contains("not a directory"), "{stderr}");
}
