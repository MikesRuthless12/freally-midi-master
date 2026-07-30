// Each integration test is its own binary and compiles this module fresh, so a
// helper `modes.rs` does not need looks unused there even though `melody.rs`
// relies on it. Allowed once here rather than annotated per function.
#![allow(dead_code)]

//! Shared by the per-generator dataset gates.
//!
//! Each part has its own test file — `melody.rs`, `counter.rs`, `bass.rs` — and
//! each asks the same four questions of the shipped data: is every authored
//! parameter read, is every authored string one a reader knows, does the part
//! stay in its key and register, and how many *different* patterns can an artist
//! actually reach. The questions live here so the four files cannot drift into
//! four different answers, which is the same reason `NON_MODEL_DIRS` is one
//! constant rather than a list restated per loader.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::pattern::LaneTrack;
use serde_json::Value;

pub fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

pub fn shipped_models() -> BTreeMap<String, engine::StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

/// Every model that authors `block_name`, with that block.
pub fn models_authoring(block_name: &str) -> Vec<(String, engine::StyleModel, Value)> {
    shipped_models()
        .into_iter()
        .filter_map(|(id, model)| {
            let block = model
                .blocks
                .get(block_name)
                .filter(|value| !value.is_null())
                .cloned()?;
            Some((id, model, block))
        })
        .collect()
}

/// Every place `block_name` is authored for this model, labelled.
///
/// ⛔ **The model's own block *and* every mode's override of it.** Modes
/// (TASK-040V) can change any block, so a mode authoring `melody.swagger` would
/// be an unread parameter that the plain `blocks["melody"]` walk never sees — a
/// hole in the one discipline this dataset most needs. The label distinguishes
/// them so a failure says *where*: `melody` or `modes[dark].melody`.
pub fn authored_blocks(model: &engine::StyleModel, block_name: &str) -> Vec<(String, Value)> {
    let mut found = Vec::new();

    if let Some(block) = model.blocks.get(block_name).filter(|v| !v.is_null()) {
        found.push((block_name.to_owned(), block.clone()));
    }

    if let Some(modes) = model.blocks.get("modes").and_then(Value::as_array) {
        for mode in modes {
            let name = mode
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>");
            if let Some(block) = mode.get(block_name).filter(|v| !v.is_null()) {
                found.push((format!("modes[{name}].{block_name}"), block.clone()));
            }
        }
    }

    found
}

/// Does `prefix` name this path, or an object containing it?
///
/// `"intervals"` covers `intervals.step` because the generator reads that
/// block's own fields; `"interval"` does not, or a prefix would quietly excuse
/// every key that merely starts with the same letters.
pub fn covers(prefix: &str, path: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}."))
}

/// Every leaf parameter path in a block, as `a.b.c`.
///
/// Arrays are leaves: `colorTones` is one parameter, not one per entry, and
/// walking into it would make the list depend on how many a model happens to
/// have.
pub fn leaf_paths(value: &Value, prefix: &str) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, child)| {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_paths(child, &path)
            })
            .collect(),
        _ if prefix.is_empty() => Vec::new(),
        _ => vec![prefix.to_owned()],
    }
}

/// The note content of a lane, for telling two generations apart.
///
/// The notes rather than the id: two seeds that produce the same notes are one
/// generation wearing two names, and counting them twice would let a narrow
/// grammar claim variety it does not have.
pub fn shape(track: &LaneTrack) -> Vec<(u32, u32, u8, u8)> {
    track
        .notes
        .iter()
        .map(|note| (note.start_tick, note.len_ticks, note.pitch, note.vel))
        .collect()
}

/// How many seeds a variety measurement walks.
///
/// **1,000 by default, and that number is chosen rather than round.** The gate
/// below asserts an absolute floor of [`MIN_DISTINCT`] = 500 per model per part,
/// and a count can never exceed the seeds tried — so at 500 seeds a model that
/// collides at all could not possibly prove it. 1,000 leaves room for the
/// deliberately narrow styles (the rage family collides about 22% of the time) to
/// clear 500 on their own merits.
///
/// `FREALLY_VARIETY_SEEDS` raises it; the deep run (10,000) is that env var
/// rather than a separate test, because 10,000 x every model x every part is well
/// over a million generations and not a per-push cost.
pub fn variety_seeds() -> u64 {
    std::env::var("FREALLY_VARIETY_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000)
}

/// The share of seeds that must reach a pattern no other seed reached.
///
/// ⛔ **A collision rate, not a count, and it is the claim that the output is
/// unbounded.** Distinct patterns can never outnumber the seeds tried, so
/// "infinite generations per genre" cannot be a stored number — it is the
/// statement that the grammar has *not saturated*: walk more seeds, get more
/// patterns. At 99% there is no ceiling in sight and the only limit is how many
/// seeds the caller asks for. Measured at 3,000 seeds on 2026-07-29, 24 of 26
/// models returned 3000/3000 — no saturation whatsoever.
///
/// A raw floor on its own would pass a grammar that reaches 501 and then repeats
/// itself forever, which is why both this and [`MIN_DISTINCT`] are asserted.
///
/// **0.98 rather than 1.0, because this is a saturation detector and not a
/// perfection target.** A grammar that is running out shows a share that *falls*
/// as seeds grow; one with a space far larger than any session will exhaust still
/// produces the occasional coincidence, and demanding zero of them would be
/// demanding a perfect hash rather than a varied instrument. Measured
/// 2026-07-29 at 1,000 seeds: melodies land at 0.998–0.999 across the roster, and
/// the lowest counter is uk-drill at 0.984 — which weights `sustain_pad` at 60%,
/// and a held string voicing has genuinely fewer shapes than a lead line.
pub const DISTINCT_SHARE: f64 = 0.98;

/// The fewest distinct patterns every model must reach for every part.
///
/// ⛔ **Mike's floor, 2026-07-29:** *"I want infinite generations per genre, but
/// I want at least 500 if not more generations per artist within a specific
/// genre."* So this one is absolute and admits **no exceptions** — a style may be
/// deliberately narrow enough to collide (rage's two-note hook is the genre), and
/// it still has to offer a producer five hundred different patterns before it
/// starts repeating itself.
pub const MIN_DISTINCT: usize = 500;
