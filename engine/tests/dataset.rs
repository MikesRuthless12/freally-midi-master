//! Loads the real `data/` directory — the models that actually ship — and
//! checks they resolve, inherit and lint cleanly. A broken model must fail here
//! rather than in a user's session.

use std::fs;
use std::path::{Path, PathBuf};

use engine::dataset::{registry_from, DatasetError};

/// `data/` sits beside the engine crate, at the repo root.
fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model file: `data/_defaults.json` plus `data/genres/*.json`.
fn shipped_models() -> Vec<(PathBuf, String)> {
    let root = data_dir();
    let mut out = vec![];

    let defaults = root.join("_defaults.json");
    let text = fs::read_to_string(&defaults)
        .unwrap_or_else(|e| panic!("{} is required: {e}", defaults.display()));
    out.push((defaults, text));

    let genres = root.join("genres");
    let mut paths: Vec<PathBuf> = fs::read_dir(&genres)
        .unwrap_or_else(|e| panic!("{} is required: {e}", genres.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    // Deterministic order so a failure is reproducible.
    paths.sort();

    for path in paths {
        let text = fs::read_to_string(&path).unwrap();
        out.push((path, text));
    }
    out
}

#[test]
fn the_shipped_dataset_loads_without_rejections() {
    let registry = registry_from(shipped_models());
    assert!(
        registry.rejected().is_empty(),
        "rejected: {:?}",
        registry.rejected()
    );
    assert!(registry.len() >= 4, "expected _defaults plus three genres");
}

#[test]
fn every_shipped_model_resolves_and_lints_clean() {
    let registry = registry_from(shipped_models());
    let (resolved, errors) = registry.resolve_all();
    assert!(errors.is_empty(), "models failed to resolve: {errors:#?}");
    assert_eq!(resolved.len(), registry.len());
}

#[test]
fn the_three_seed_genres_are_present() {
    let registry = registry_from(shipped_models());
    for id in ["_defaults", "trap", "uk-drill", "rage"] {
        assert!(registry.raw(id).is_some(), "missing model `{id}`");
    }
}

#[test]
fn a_genre_inherits_what_it_does_not_declare() {
    let registry = registry_from(shipped_models());
    let trap = registry.resolve("trap").unwrap();

    // trap.json declares no `arrangement.structures`; _defaults does.
    let structures = trap.blocks["arrangement"]
        .get("structures")
        .expect("structures should have been inherited from _defaults");
    assert!(structures.as_array().is_some_and(|a| !a.is_empty()));

    // ...but it does declare its own sectionBars, which must win.
    assert_eq!(trap.blocks["arrangement"]["dropByBar"], 5);
}

#[test]
fn a_genre_overrides_what_it_does_declare() {
    let registry = registry_from(shipped_models());

    let defaults = registry.resolve("_defaults").unwrap();
    let trap = registry.resolve("trap").unwrap();

    // _defaults is a generic backbeat; trap is half-time with the snare on 3.
    assert_eq!(
        defaults.blocks["drums"]["snare"]["placement"],
        "backbeat_24"
    );
    assert_eq!(trap.blocks["drums"]["snare"]["placement"], "halftime_3");

    assert_eq!(defaults.session.as_ref().unwrap().half_time, Some(false));
    assert_eq!(trap.session.as_ref().unwrap().half_time, Some(true));
}

#[test]
fn identity_survives_inheritance() {
    let registry = registry_from(shipped_models());
    for id in ["trap", "uk-drill", "rage"] {
        let model = registry.resolve(id).unwrap();
        assert_eq!(model.id, id, "a model must keep its own id");
        assert_ne!(model.name, "Defaults", "a model must keep its own name");
    }
}

#[test]
fn the_genres_are_musically_distinct_where_the_research_says_they_should_be() {
    let registry = registry_from(shipped_models());
    let trap = registry.resolve("trap").unwrap();
    let drill = registry.resolve("uk-drill").unwrap();
    let rage = registry.resolve("rage").unwrap();

    // UK drill's 808 is a counter-riff; trap's doubles the bassline roots.
    assert_eq!(drill.blocks["drums"]["bass808"]["role"], "counter_riff");
    assert_eq!(trap.blocks["drums"]["bass808"]["role"], "bassline");

    // Drill mutes the 808 under the snare — the genre's signature gap.
    assert_eq!(drill.blocks["drums"]["bass808"]["muteUnderSnare"], true);

    // Rage's hats are bursts, not a continuous stream. That is the whole
    // aesthetic; a regression here would make it sound like trap.
    assert_eq!(rage.blocks["drums"]["hihat"]["continuous"], false);

    // Tempo centres differ: drill sits in the 140s, rage trends faster.
    let bpm = |m: &engine::StyleModel| m.session.as_ref().unwrap().bpm.as_ref().unwrap().nominal();
    assert_eq!(bpm(&drill), 141.0);
    assert_eq!(bpm(&rage), 150.0);
    assert!(bpm(&rage) > bpm(&trap));
}

#[test]
fn a_planted_cycle_is_rejected() {
    // Take the real dataset and add two models that point at each other.
    let mut entries = shipped_models();
    entries.push((
        PathBuf::from("cycle-a.json"),
        r#"{"id":"cycle-a","type":"genre","name":"A","extends":["cycle-b"]}"#.into(),
    ));
    entries.push((
        PathBuf::from("cycle-b.json"),
        r#"{"id":"cycle-b","type":"genre","name":"B","extends":["cycle-a"]}"#.into(),
    ));

    let registry = registry_from(entries);

    match registry.resolve("cycle-a") {
        Err(DatasetError::Cycle(path)) => {
            assert!(
                path.contains("cycle-a"),
                "the path should name the loop: {path}"
            );
        }
        other => panic!("expected a cycle error, got {other:?}"),
    }

    // And the healthy models must still resolve — one bad model cannot take
    // the dataset down with it.
    let (ok, errors) = registry.resolve_all();
    assert!(ok.contains_key("trap"));
    assert_eq!(errors.len(), 2, "only the two cycle models should fail");
}

#[test]
fn a_planted_bad_model_is_rejected_without_hiding_the_rest() {
    let mut entries = shipped_models();
    entries.push((
        PathBuf::from("bad-register.json"),
        r#"{"id":"bad","type":"genre","name":"Bad","extends":["_defaults"],
            "melody":{"register":[60,200]}}"#
            .into(),
    ));

    let registry = registry_from(entries);
    let (ok, errors) = registry.resolve_all();

    assert!(ok.contains_key("trap"), "healthy models still resolve");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].1.to_string().contains("0–127"), "{:?}", errors[0]);
}

#[test]
fn the_json_schema_file_is_valid_json_and_describes_the_model() {
    let path = data_dir().join("schema").join("artist-style.schema.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(schema["type"], "object");
    for required in ["id", "type", "name"] {
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == required),
            "`{required}` should be required"
        );
    }
    // Every shipped model carries a `$schema` pointer, so the schema has to
    // permit it — `additionalProperties: false` would otherwise reject them all.
    assert!(schema["properties"]["$schema"].is_object());
}

#[test]
fn every_shipped_model_points_at_the_schema_and_cites_its_sources() {
    for (path, text) in shipped_models() {
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();

        assert!(
            value.get("$schema").is_some(),
            "{name} should point at the schema so editors can complete it"
        );
        // Sourcing is the dataset's legal backbone: a model with no cited
        // research is not something we can defend or maintain.
        let sources = value.get("sources").and_then(|s| s.as_array());
        assert!(
            sources.is_some_and(|s| !s.is_empty()),
            "{name} must cite its sources"
        );
    }
}
