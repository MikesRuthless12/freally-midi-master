//! The session block's string vocabularies, against the data that ships.
//!
//! `session.keys` and `session.scales` are authored as strings and read by
//! `SessionContext::from_model` and `SessionDefaults::of`, both of which drop a
//! name they cannot parse and fall back. A typo therefore costs the model its
//! key or its scale in complete silence — the same failure mode the lane names
//! had before `humanize.rs` grew a test for them.
//!
//! This is that test for the session block. It is the fifth string vocabulary
//! in the dataset and the last one that had no gate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::context::SessionDefaults;
use engine::pattern::Scale;
use engine::theory::key_pitch_class;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model, resolved.
fn shipped_models() -> BTreeMap<String, engine::StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let registry = engine::dataset::registry_from(scan.files);
    let (models, errors) = registry.resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

#[test]
fn every_authored_key_name_parses_to_a_pitch_class() {
    let mut unreadable: Vec<String> = Vec::new();

    for (id, model) in shipped_models() {
        let Some(keys) = model.session.as_ref().and_then(|s| s.keys.as_ref()) else {
            continue;
        };
        for name in keys.options() {
            if key_pitch_class(&name).is_none() {
                unreadable.push(format!("{id}: session.keys \"{name}\""));
            }
        }
    }

    assert!(
        unreadable.is_empty(),
        "these key names are dropped and the model silently loses them: {unreadable:#?}"
    );
}

#[test]
fn every_authored_scale_name_parses_to_a_scale() {
    let mut unreadable: Vec<String> = Vec::new();

    for (id, model) in shipped_models() {
        let Some(scales) = model.session.as_ref().and_then(|s| s.scales.as_ref()) else {
            continue;
        };
        for name in scales.options() {
            let parsed: Option<Scale> =
                serde_json::from_value(serde_json::Value::String(name.clone())).ok();
            if parsed.is_none() {
                unreadable.push(format!("{id}: session.scales \"{name}\""));
            }
        }
    }

    assert!(
        unreadable.is_empty(),
        "these scale names are dropped and the model silently loses them: {unreadable:#?}"
    );
}

#[test]
fn the_defaults_a_model_reports_are_the_ones_it_generates_with() {
    // The chips read `SessionDefaults`; generation reads `from_model`. Two
    // readers of one block is how a readout starts lying, so the deterministic
    // fields are asserted equal across every shipped model.
    use engine::context::{SessionContext, SessionOverrides};

    for (id, model) in shipped_models() {
        let defaults = SessionDefaults::of(&model);
        // Any seed: the three fields compared here are not sampled.
        let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), 99);

        assert_eq!(defaults.bpm, ctx.bpm, "{id} bpm");
        assert_eq!(defaults.swing, ctx.swing, "{id} swing");
        assert_eq!(defaults.half_time, ctx.half_time, "{id} halfTime");
    }
}

#[test]
fn a_key_a_model_offers_is_a_key_the_engine_can_generate_in() {
    // The other half of the same claim: what the chip lists must round-trip
    // through the pitch class an override carries.
    for (id, model) in shipped_models() {
        let defaults = SessionDefaults::of(&model);
        for name in &defaults.keys {
            let pitch = key_pitch_class(name)
                .unwrap_or_else(|| panic!("{id} offers \"{name}\", which has no pitch class"));
            assert!(pitch < 12, "{id}: \"{name}\" is not a pitch class");
        }
    }
}
