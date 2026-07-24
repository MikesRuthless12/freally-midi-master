//! Golden determinism: a fixed `(model, seed, context)` must always produce
//! byte-identical output (NFR determinism, US-004).
//!
//! This is the test that makes the seed chip's promise real — "paste a seed,
//! get the same beat" — and the one that will notice an engine change nobody
//! meant to make. Everything else here asserts a *property*; this asserts the
//! exact bytes.
//!
//! When a change to the engine is intentional, regenerate the snapshots in the
//! same commit and say what moved in `CHANGELOG.md`:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p engine --test golden
//! ```
//!
//! Then **read the diff**. The JSON is committed alongside the `.mid` precisely
//! so that a regeneration is reviewable: if the diff is bigger than the change
//! that was made, something else moved too.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use engine::context::{Humanize, SessionContext, Swing, SwingGrid};
use engine::generators::drums::generate;
use engine::humanize::humanize;
use engine::midi::pattern_to_smf;
use engine::pattern::{Lane, Part, Pattern, Scale, PPQ};
use engine::StyleModel;

/// The cases pinned by a snapshot: `(model, seed, bars)`.
///
/// Three genres so a change that only affects one is still caught, and one
/// eight-bar case because fills and phrase boundaries only exist at length.
const CASES: &[(&str, u64, u16)] = &[
    ("trap", 7, 4),
    ("uk-drill", 7, 4),
    ("rage", 7, 4),
    ("trap", 2024, 8),
];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn shipped() -> BTreeMap<String, StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

/// The context every snapshot is taken in.
///
/// Written out rather than defaulted, and deliberately *not* neutral: a swung
/// grid with real jitter and velocity variance means the humanizer is part of
/// what is pinned. A golden taken at quantize 1.0 would not notice the feel
/// changing at all.
fn golden_context(bars: u16) -> SessionContext {
    SessionContext {
        bpm: 140.0,
        time_sig_num: 4,
        time_sig_den: 4,
        key_root: 0,
        scale: Scale::NaturalMinor,
        swing: Swing {
            grid: SwingGrid::Sixteenth,
            amount: 0.54,
        },
        bars,
        half_time: true,
        humanize: Humanize {
            quantize_strength: 0.85,
            velocity_var: 0.15,
            timing_jitter_ms: [
                (Lane::Kick, 2.0),
                (Lane::Snare, 4.0),
                (Lane::ClosedHat, 3.0),
                (Lane::Bass808, 1.0),
            ]
            .into_iter()
            .collect(),
        },
    }
}

/// The whole pipeline, exactly as `generate_pattern` will run it: generate on
/// the grid, then humanize.
fn render(model: &StyleModel, seed: u64, bars: u16) -> Pattern {
    let ctx = golden_context(bars);
    let mut lanes = generate(model, &ctx, seed);
    humanize(&mut lanes, &ctx, seed);

    Pattern {
        id: format!("{}-{seed}", model.id),
        part: Part::Drums,
        artist_id: model.id.clone(),
        seed,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
    }
}

fn updating() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1")
}

#[test]
fn every_case_matches_its_snapshot() {
    let models = shipped();
    let dir = snapshot_dir();
    if updating() {
        fs::create_dir_all(&dir).expect("the snapshot directory must be creatable");
    }

    let mut stale = Vec::new();

    for (id, seed, bars) in CASES {
        let model = models
            .get(*id)
            .unwrap_or_else(|| panic!("`{id}` must ship"));
        let pattern = render(model, *seed, *bars);

        let json = serde_json::to_string_pretty(&pattern).expect("a pattern must serialize");
        let smf = pattern_to_smf(&pattern);

        let json_path = dir.join(format!("{id}-{seed}-{bars}bar.json"));
        let midi_path = dir.join(format!("{id}-{seed}-{bars}bar.mid"));

        if updating() {
            // Written with LF so the file is identical on every OS; the repo
            // normalises text to LF and a CRLF snapshot would fail on the
            // Windows leg only.
            fs::write(&json_path, json.replace("\r\n", "\n")).expect("write the JSON snapshot");
            fs::write(&midi_path, &smf).expect("write the MIDI snapshot");
            continue;
        }

        let expected_json = fs::read_to_string(&json_path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nRun `UPDATE_GOLDEN=1 cargo test -p engine --test golden` to create it.",
                json_path.display()
            )
        });
        let expected_smf =
            fs::read(&midi_path).unwrap_or_else(|e| panic!("{}: {e}", midi_path.display()));

        if expected_json.replace("\r\n", "\n") != json {
            stale.push(format!(
                "{id} seed {seed} ({bars} bars): the pattern changed"
            ));
        }
        if expected_smf != smf {
            stale.push(format!(
                "{id} seed {seed} ({bars} bars): the exported MIDI changed"
            ));
        }
    }

    assert!(
        stale.is_empty(),
        "the engine no longer reproduces its snapshots:\n  {}\n\n\
         If the change was intended, regenerate with \
         `UPDATE_GOLDEN=1 cargo test -p engine --test golden`, read the JSON diff, \
         and note it in CHANGELOG.md. If it was not, this is the regression.",
        stale.join("\n  ")
    );
}

#[test]
fn a_snapshot_is_reproducible_within_one_run_too() {
    // The snapshot files catch drift between runs; this catches a generator
    // that reads the clock, hashes a pointer, or otherwise varies inside a
    // single process — where the committed files would still match.
    let models = shipped();
    for (id, seed, bars) in CASES {
        let model = models.get(*id).unwrap();
        let first = render(model, *seed, *bars);
        let second = render(model, *seed, *bars);
        assert_eq!(first, second, "{id} seed {seed} varied inside one run");
        assert_eq!(pattern_to_smf(&first), pattern_to_smf(&second));
    }
}

#[test]
fn a_different_seed_produces_a_different_pattern() {
    // The other half of the promise: a seed has to *mean* something. A golden
    // suite passes trivially if the engine ignores its seed.
    let models = shipped();
    let trap = models.get("trap").unwrap();
    assert_ne!(render(trap, 7, 4), render(trap, 8, 4));
}

#[test]
fn the_snapshots_on_disk_are_the_ones_the_cases_name() {
    // A renamed or deleted case would otherwise leave its snapshot behind,
    // where it proves nothing and looks like coverage.
    let dir = snapshot_dir();
    let expected: Vec<String> = CASES
        .iter()
        .flat_map(|(id, seed, bars)| {
            [
                format!("{id}-{seed}-{bars}bar.json"),
                format!("{id}-{seed}-{bars}bar.mid"),
            ]
        })
        .collect();

    let mut found: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();

    let mut expected = expected;
    expected.sort();
    assert_eq!(
        found, expected,
        "the snapshot directory does not match the case list"
    );
}
