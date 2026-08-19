//! Drum hits move in pitch (TASK-131D).
//!
//! ⛔ **Mike, 2026-08-05, twice:** "you cannot be just stuck on one line for the
//! entire drum pattern" and "hihat rolls can go up and down so can kicks, 808s,
//! snares, etc. in actual song arrangements". Measured that day: every roll note
//! in the shipped roster was written at one fixed pitch, because
//! `rolls::Roll::render` took it from `midi::gm_drum_note(lane)` — a constant.
//!
//! ⚠ **Chromatic semitones, by Mike's decision**, not scale degrees. The 808 is
//! a melodic generator and already follows the key; this is the percussion case,
//! and a pitched hat run is chromatic in practice.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::pattern::{Articulation, Lane};
use engine::StyleModel;

/// Every shipped model, read and resolved **once per test binary**.
///
/// ⛔ **This used to re-read and re-resolve the entire dataset on every call.**
/// `files::scan` walks 609 JSON files and `resolve_all()` merges inheritance
/// across 590 models, and the callers below ask for it once per test — so the
/// same load ran over and over to answer questions about data that cannot have
/// changed. `plugin/tests/host_timeline.rs` measured the same mistake at
/// **1,300.91s → 1.70s** on one binary.
///
/// ⚠ **Cloned out rather than borrowed**, which keeps every call site unchanged:
/// a map copy is memory, and what this was costing was 609 file reads and 590
/// inheritance merges.
fn shipped() -> BTreeMap<String, StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, StyleModel>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("data");
            let scan = engine::dataset::files::scan(&dir).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            models
        })
        .clone()
}

/// The pitches a model's snare rolls actually reach.
fn roll_pitches(model: &StyleModel, seeds: u64) -> BTreeSet<u8> {
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };
    let mut seen = BTreeSet::new();
    for seed in 0..seeds {
        for track in generate(model, &ctx, seed) {
            if track.lane != Lane::Snare {
                continue;
            }
            for note in &track.notes {
                if note.articulation == Some(Articulation::Roll) {
                    seen.insert(note.pitch);
                }
            }
        }
    }
    seen
}

/// How far this model lets a roll travel, in semitones, if it says.
///
/// ⛔ **The authored span, not a list of ids — and that distinction is the
/// whole of this file's second bug.** The exemption below used to read
/// `id != "country-train"`, so the one genre that authors `pitchWalk: 0` was
/// excused and the **33 artists that extend it and author no `snareRoll` of
/// their own** were not: `bela-fleck`, `ricky-skaggs`, `travis-tritt` and the
/// rest of the bluegrass and country-train lane inherit that zero and were
/// reported as writing "a flat roll" for obeying it. The same rule the melody
/// variety gate states applies here — *the property is what is being excused,
/// so the property is what is tested* — and it also survives Phase 5's next
/// five hundred artists, which an id list would not.
fn authored_walk(model: &StyleModel) -> Option<u8> {
    model
        .blocks
        .get("drums")?
        .pointer("/snareRoll/pitchWalk")?
        .as_f64()
        .map(|walk| walk.round().clamp(0.0, 24.0) as u8)
}

#[test]
fn a_roll_climbs_and_falls_rather_than_sitting_on_one_note() {
    // ⛔ The gate for the whole task. Before it, this set had exactly one member
    // for every model on the roster.
    let models = shipped();
    let mut flat = Vec::new();

    for (id, model) in &models {
        // ⚠ **The floor is three pitches or whatever the model's own span can
        // reach, whichever is smaller.** `rolls::Roll::pitch_at` interpolates
        // across `0..=pitchWalk`, so a span of `w` can put a roll on exactly
        // `w + 1` notes and no arrangement of seeds makes it more: the six
        // models authoring `pitchWalk: 1` — `darude`, `armin-van-buuren`,
        // `alice-deejay`, `ian-van-dahl`, `danny-l-harle`, `anthony-dent` —
        // reach two, which is a roll that moves rather than one sitting on a
        // note. A model that authors nothing still owes the full three; three
        // is what `_defaults` gives it.
        let walk = authored_walk(model);
        let floor = walk.map_or(3, |walk| usize::from(walk).saturating_add(1).min(3));
        let pitches = roll_pitches(model, 60);
        if pitches.is_empty() {
            continue;
        }
        if pitches.len() < floor {
            flat.push((id.clone(), pitches.len(), floor));
        }
        // The positive half: a model that says its snare does not pitch — a
        // train beat does not — must actually stay on one note.
        if walk == Some(0) {
            assert_eq!(
                pitches.len(),
                1,
                "{id} authors pitchWalk 0 and must stay on one note"
            );
        }
    }

    assert!(
        flat.is_empty(),
        "these models still write a flat roll, as (model, pitches reached, its own floor): {flat:?}"
    );
}

#[test]
fn how_far_a_roll_travels_is_the_artists_own_decision() {
    // ⛔ Otherwise the walk is one global effect wearing a per-artist name.
    // rage authors 8 semitones and Drake 2; if those came out the same, nothing
    // here would be per artist.
    let models = shipped();
    let span = |id: &str| -> u8 {
        let p = roll_pitches(&models[id], 120);
        p.last().copied().unwrap_or(0) - p.first().copied().unwrap_or(0)
    };

    let (wide, narrow) = (span("rage"), span("drake"));
    assert!(
        wide > narrow,
        "rage authors a wider walk than drake, got {wide} vs {narrow}"
    );
}
