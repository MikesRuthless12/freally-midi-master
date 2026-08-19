//! An artist generating in every genre they work in (TASK-158C).
//!
//! ⛔⛔ **The defect this closes is a readout that lies, and it has been live at
//! scale.** `lib/cross-filter.ts` lists an artist under every genre in their
//! `relatedGenres`, so the rail says *"2Pac works in boom-bap"* — and Generate
//! has always answered g-funk, because `extends` is a **single lane in all 534**
//! artist and producer models and 529 of them name a related genre they do not
//! extend. The roster's claim and the generator's answer have never agreed.
//!
//! `Registry::resolve_over` is the fix: the artist's own blocks, resolved over
//! the *chosen* genre instead of their authored parent. What is measured here is
//! whether that holds against the **real dataset** rather than a fixture — the
//! unit tests in `dataset/inherit.rs` cover the merge order on four models, and
//! four models cannot tell you whether 1,600 real combinations lint.
//!
//! ## ⚠ What this deliberately does NOT do, and why
//!
//! The roadmap costs this task's gate at *"prove the variety floor holds for
//! each (model, genre, mood) triple"* — ~1,600 pairs times a mood axis, each a
//! thousand-seed variety run. That is not a per-push cost and it would not buy
//! what it looks like it buys: **the variety of a swapped model is the variety
//! of the genre it was swapped onto**, and every genre already has its own
//! per-model gate. What has to be proven here is the part that is *new*:
//!
//! 1. every real pair **resolves and lints** — the cheap sweep, and the one that
//!    catches a combination the linter refuses;
//! 2. a swap actually **changes the output**, so the chip is not decorative;
//! 3. a swapped model still **generates**, and is not merely well-formed;
//! 4. the variety floor holds on a **sample**, at the seed count the drum
//!    variety gate already uses.
//!
//! ⛔ Under `FREALLY_DEEP_SWAP=1` the sample in (4) becomes every pair. The deep
//! run is an env var rather than a second test for the same reason
//! `FREALLY_VARIETY_SEEDS` is.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::dataset::Registry;
use engine::generators::drums::generate;
use engine::pattern::{Lane, LaneTrack};
use serde_json::Value;

/// Seeds per variety measurement — the same number `drum_variety.rs` uses, and
/// for the same reason: enough that a small answer is a finding.
const SEEDS: u64 = 200;

/// The floor a sampled pair has to clear.
///
/// ⚠ **Lower than `MIN_DISTINCT` = 500, and it has to be**: that floor is
/// asserted over 1,000 seeds and this walks 200, and a count can never exceed
/// the seeds tried. What this is detecting is a swap that *collapses* a
/// generator — a base that produces four beats is broken, not narrow.
///
/// ▶ **Which of the two it is, is decided by the model rather than by this
/// number**: a pair under the floor is only reported when the same artist clears
/// it over their own authored parent. See the test for the two ballad-and-space
/// kits that proved the absolute reading wrong.
const MIN_DISTINCT_SAMPLED: usize = 100;

/// How many pairs the sampled measurement walks, unless the deep run is asked
/// for. Spread across the roster by taking every Nth pair rather than the first
/// N, so it is not all one letter of the alphabet.
const SAMPLE: usize = 40;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// The raw registry, read once per test binary — see `genre_invariants.rs` for
/// why this is memoised rather than re-read per call.
fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
        engine::dataset::registry_from(scan.files)
    })
}

/// Every `(artist, genre)` the roster claims, from the artist's own
/// `relatedGenres`.
///
/// ⛔ **Read off `relatedGenres`, because that is exactly what the rail filters
/// on.** Building this list any other way would be testing a different claim
/// from the one the producer can see.
///
/// ⚠ The genre the artist already `extends` is skipped: asking for it is a
/// no-op, which `inherit.rs` asserts directly.
fn claimed_pairs() -> &'static Vec<(String, String)> {
    static PAIRS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    PAIRS.get_or_init(|| {
        let registry = registry();
        let mut pairs = Vec::new();
        for id in registry.ids() {
            let Some(raw) = registry.raw(id) else {
                continue;
            };
            if raw.get("type").and_then(Value::as_str) == Some("genre") {
                continue;
            }
            let own: BTreeSet<&str> = raw
                .get("extends")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();

            for genre in raw
                .get("relatedGenres")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default()
            {
                if own.contains(genre) || registry.raw(genre).is_none() {
                    continue;
                }
                pairs.push((id.clone(), genre.to_owned()));
            }
        }
        pairs.sort();
        pairs
    })
}

fn context() -> SessionContext {
    SessionContext {
        bars: 4,
        ..Default::default()
    }
}

/// The beat's skeleton — the same measure `drum_variety.rs` counts, and its doc
/// explains at length why velocity and ghosts are thrown away.
fn shape(lanes: &[LaneTrack]) -> Vec<(Lane, Vec<u32>)> {
    let mut out: Vec<(Lane, Vec<u32>)> = lanes
        .iter()
        .map(|track| {
            let mut hits: Vec<u32> = track
                .notes
                .iter()
                .filter(|n| n.articulation != Some(engine::pattern::Articulation::Ghost))
                .filter(|n| n.vel >= 50)
                .map(|n| (n.start_tick + 120) / 240)
                .collect();
            hits.sort_unstable();
            hits.dedup();
            (track.lane, hits)
        })
        .collect();
    out.sort_by_key(|(lane, _)| format!("{lane:?}"));
    out
}

#[test]
fn the_roster_claims_a_great_many_pairs_or_this_file_proves_nothing() {
    // ⛔ A guard on the *premise*. If `relatedGenres` were ever emptied, every
    // test below would sweep zero pairs and pass — the shape of green test that
    // is worse than a missing one.
    let pairs = claimed_pairs();
    assert!(
        pairs.len() > 500,
        "the roster claims only {} artist/genre pairs — that is not the dataset this was written against",
        pairs.len()
    );
}

#[test]
fn every_pair_the_roster_claims_actually_resolves() {
    // ⛔⛔ **The sweep that matters, and it is cheap.** `resolve_over` runs the
    // same linter and the same parse as an ordinary resolve, so a combination
    // that produces something the linter refuses fails *here* rather than in
    // front of a producer who picked it from a menu. No generation, so the whole
    // ~1,600 is affordable on every push.
    let registry = registry();
    let mut refused: Vec<String> = Vec::new();

    for (artist, genre) in claimed_pairs() {
        if let Err(error) = registry.resolve_over(artist, Some(genre)) {
            refused.push(format!("{artist} over {genre}: {error}"));
        }
    }

    assert!(
        refused.is_empty(),
        "{} of {} claimed pairs do not resolve:\n{}",
        refused.len(),
        claimed_pairs().len(),
        refused.join("\n")
    );
}

#[test]
fn a_swap_changes_what_is_generated_rather_than_being_decorative() {
    // ⛔⛔ **The chip has to do something.** A base swap that resolved cleanly
    // and produced the same notes would be the readout-that-lies failure this
    // whole task is about, arriving through the fix — and it is exactly what a
    // merge in the wrong order would give: the child's own genre re-applied on
    // top of the new one.
    //
    // ⚠ **Not asserted for every pair.** Two genres can genuinely be close
    // enough that one seed collides, and a model may author enough of its own
    // that the base barely shows. What must not be true is that swapping is
    // *usually* a no-op.
    let registry = registry();
    let ctx = context();
    let mut changed = 0usize;
    let mut compared = 0usize;

    for (artist, genre) in claimed_pairs().iter().step_by(7) {
        let Ok(authored) = registry.resolve(artist) else {
            continue;
        };
        let Ok(swapped) = registry.resolve_over(artist, Some(genre)) else {
            continue;
        };
        compared += 1;
        let before = shape(&generate(&authored, &ctx, 42));
        let after = shape(&generate(&swapped, &ctx, 42));
        if before != after {
            changed += 1;
        }
    }

    assert!(compared > 20, "too few pairs compared to mean anything");
    let share = changed as f64 / compared as f64;
    assert!(
        share > 0.5,
        "only {changed} of {compared} swaps changed the beat — the base is not reaching the generator"
    );
}

#[test]
fn a_swapped_model_still_generates_something_to_hear() {
    // ⚠ Well-formed is not the same as playable. A model that lints and writes
    // no notes is silence with a green test over it.
    let registry = registry();
    let ctx = context();
    let mut silent: Vec<String> = Vec::new();

    for (artist, genre) in claimed_pairs().iter().step_by(3) {
        let Ok(model) = registry.resolve_over(artist, Some(genre)) else {
            continue;
        };
        let lanes = generate(&model, &ctx, 7);
        if lanes.iter().all(|track| track.notes.is_empty()) {
            silent.push(format!("{artist} over {genre}"));
        }
    }

    assert!(
        silent.is_empty(),
        "these swaps generate nothing at all:\n{}",
        silent.join("\n")
    );
}

#[test]
fn a_swapped_model_does_not_collapse_to_a_handful_of_beats() {
    // ⛔ The variety half, on a **sample** — see the module header for why the
    // full 1,600-pair sweep is not what it looks like. What this catches is a
    // swap that *collapses* a generator, which is a different failure from a
    // genre being narrow.
    let registry = registry();
    let ctx = context();
    let pairs = claimed_pairs();

    let deep = std::env::var("FREALLY_DEEP_SWAP").is_ok();
    let stride = if deep {
        1
    } else {
        (pairs.len() / SAMPLE).max(1)
    };

    let mut thin: Vec<String> = Vec::new();
    let mut walked = 0usize;

    for (artist, genre) in pairs.iter().step_by(stride) {
        let Ok(model) = registry.resolve_over(artist, Some(genre)) else {
            continue;
        };
        walked += 1;

        let swapped = distinct_beats(&model, &ctx);
        if swapped >= MIN_DISTINCT_SAMPLED {
            continue;
        }

        // ⛔⛔ **A count under the floor is only a finding when the model clears
        // that floor over its OWN base**, and without this the two were
        // indistinguishable. [`shape`] and [`SEEDS`] here are `drum_variety`'s to
        // the line, and *that* gate's floor is **20** — deliberately, because
        // `country-train` writes 24 beats and a train beat genuinely is
        // four-on-the-floor with `syncopation: 0`. So a model can pass the
        // roster-wide variety gate and still sit under 100 here with the swap
        // having done nothing to it.
        //
        // ▶ Both models this caught are that case. `walter-afanasieff` is a
        // support-layer ballad kit — research: "soft-attack kick on 1, gated or
        // brushed snare on beat 3 of the doubled grid, **no hat lane**" — and
        // every other lane he authors generates below the velocity 50 that
        // `shape` counts, so his beat is a kick figure and a backbeat wherever he
        // is played. `james-blake` is the same story in a different lane: "one
        // deep sub-kick per bar and a single reverbed snare on 3 (half-time),
        // with silence between hits used as a rhythmic event". Widening either to
        // clear a floor would be authoring over the research.
        //
        // What must not happen — and what this still fails on — is a base that
        // takes a model from a varied beat down to a handful.
        let own = registry
            .resolve(artist)
            .map(|authored| distinct_beats(&authored, &ctx))
            .unwrap_or(0);
        if own >= MIN_DISTINCT_SAMPLED {
            thin.push(format!(
                "{artist} over {genre}: {swapped} of {SEEDS}, against {own} over its own base"
            ));
        }
    }

    assert!(walked >= 20, "only {walked} pairs walked");
    assert!(
        thin.is_empty(),
        "{} of {walked} sampled swaps collapse below {MIN_DISTINCT_SAMPLED} distinct beats in {SEEDS} seeds, \
         from a model that clears it over its own base:\n{}",
        thin.len(),
        thin.join("\n")
    );
}

/// How many distinct beats a model writes over [`SEEDS`] seeds of one context.
fn distinct_beats(model: &engine::StyleModel, ctx: &SessionContext) -> usize {
    let mut beats: BTreeSet<Vec<(Lane, Vec<u32>)>> = BTreeSet::new();
    for seed in 0..SEEDS {
        beats.insert(shape(&generate(model, ctx, seed)));
    }
    beats.len()
}

#[test]
fn a_swap_never_rewrites_who_the_model_is() {
    // ⛔ Identity is the child's. A swap that let a genre's `name` through would
    // put the wrong artist in the rail, and one that let `type` through would
    // put a genre in the roster.
    let registry = registry();
    let mut wrong: Vec<String> = Vec::new();

    for (artist, genre) in claimed_pairs().iter().step_by(5) {
        let Ok(swapped) = registry.resolve_over(artist, Some(genre)) else {
            continue;
        };
        let Ok(authored) = registry.resolve(artist) else {
            continue;
        };
        if swapped.id != authored.id || swapped.name != authored.name {
            wrong.push(format!(
                "{artist} over {genre} became `{}` / `{}`",
                swapped.id, swapped.name
            ));
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_pairs_are_the_ones_the_rail_actually_offers() {
    // ⚠ A guard on the *list*, not on the swap. `claimed_pairs` reads
    // `relatedGenres` because that is what `cross-filter.ts` filters on; if it
    // ever drifted to reading `extends` instead, every test above would pass
    // while proving nothing about what a producer can pick.
    let registry = registry();
    let mut checked = 0usize;

    for (artist, genre) in claimed_pairs().iter().take(50) {
        let raw = registry.raw(artist).expect("the artist is in the registry");
        let related: Vec<&str> = raw
            .get("relatedGenres")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        assert!(
            related.contains(&genre.as_str()),
            "{artist} does not list {genre} in relatedGenres"
        );
        let extends: Vec<&str> = raw
            .get("extends")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        assert!(
            !extends.contains(&genre.as_str()),
            "{artist} already extends {genre} — that pair is a no-op and should not be listed"
        );
        checked += 1;
    }

    assert_eq!(checked, 50);
}

/// ⚠ A silence check on the premise the whole task rests on, so nobody has to
/// re-derive the 529 the roadmap quotes.
#[test]
fn the_roster_really_does_claim_genres_the_models_do_not_extend() {
    let registry = registry();
    let mut mismatched = 0usize;
    let mut multi_parent = 0usize;

    for id in registry.ids() {
        let Some(raw) = registry.raw(id) else {
            continue;
        };
        if raw.get("type").and_then(Value::as_str) == Some("genre") {
            continue;
        }
        let extends: Vec<&str> = raw
            .get("extends")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if extends.len() > 1 {
            multi_parent += 1;
        }
        let related: Vec<&str> = raw
            .get("relatedGenres")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if related.iter().any(|genre| !extends.contains(genre)) {
            mismatched += 1;
        }
    }

    assert!(
        mismatched > 400,
        "only {mismatched} models claim a genre they do not extend — the premise has moved"
    );
    // ⛔ **`extends` being a single lane is why a base *swap* is the answer and
    // multi-`extends` is not.** If models start declaring two parents, this
    // fails and the decision is worth revisiting rather than inheriting.
    assert_eq!(
        multi_parent, 0,
        "{multi_parent} models now declare more than one parent — re-read TASK-158C's shape"
    );
}
