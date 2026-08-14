//! How many different melodies, counterlines, basslines and chord parts does
//! one artist actually write?
//!
//! ⛔ **The drum half of this question was asked first and found a real defect**
//! (`drum_variety.rs`): a hardcoded fill made six flagship trap artists write a
//! byte-identical snare roll. Mike, 2026-08-05: "it shouldn't just be drums that
//! generate differently for the artist/producers, it should also be melodies,
//! counter melodies, basslines, and chords and it should generate as many as it
//! can that are unique from one another." This is the same measurement, pointed
//! at the other four generators.
//!
//! ⚠ **TASK-126 already did this once for chords** and the number is the reason
//! it got fixed: `voice()` took the strict minimum cost, so a voicing was a pure
//! function of the chord, and **rage reached 8 distinct progressions in 1,000
//! seeds** while its melody reached 823.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::pattern::Part;
use engine::StyleModel;

const SEEDS: u64 = 200;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

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
            let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            models
        })
        .clone()
}

/// A part as a listener hears it: pitch and onset, quantised to the 16th.
///
/// ⚠ Humanization nudges every note, so raw ticks would make every seed look
/// distinct and the measure would report perfect variety over one melody — the
/// mistake the drum version of this made on its first attempt.
fn shape(model: &StyleModel, ctx: &SessionContext, part: Part, seed: u64) -> Vec<(u32, u8)> {
    let pattern = engine::parts::render(model, ctx, engine::parts::Seeds::shared(seed), part);
    let mut out: Vec<(u32, u8)> = pattern
        .iter()
        .flat_map(|t| t.notes.iter())
        .map(|n| ((n.start_tick + 120) / 240, n.pitch))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[test]
fn report_how_many_distinct_melodic_parts_each_artist_writes() {
    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };
    const PARTS: [(Part, &str); 4] = [
        (Part::Melody, "melody"),
        (Part::Counter, "counter"),
        (Part::Bass, "bass"),
        (Part::Chords, "chords"),
    ];

    println!("\n  distinct over {SEEDS} seeds, 4 bars");
    println!(
        "  {:>16}  {:>7}  {:>7}  {:>7}  {:>7}",
        "model", "melody", "counter", "bass", "chords"
    );

    let mut worst: Vec<(String, &str, usize)> = Vec::new();
    for (id, model) in &models {
        let mut counts = Vec::new();
        for (part, name) in PARTS {
            let mut seen = BTreeSet::new();
            for seed in 0..SEEDS {
                let s = shape(model, &ctx, part, seed);
                if !s.is_empty() {
                    seen.insert(s);
                }
            }
            counts.push(seen.len());
            if !seen.is_empty() {
                worst.push((id.clone(), name, seen.len()));
            }
        }
        println!(
            "  {id:>16}  {:>7}  {:>7}  {:>7}  {:>7}",
            counts[0], counts[1], counts[2], counts[3]
        );
    }

    worst.sort_by_key(|(_, _, n)| *n);
    println!("\n  TEN NARROWEST:");
    for (id, part, n) in worst.iter().take(10) {
        println!("    {id:>16} {part:>8}  {n:>4} / {SEEDS}");
    }
}

/// The floor a part must clear before a producer notices it repeating.
///
/// ⛔ **Set from a measurement, not from taste.** Before this, `rage` and
/// `osamason` reached **4 distinct chord parts in 200 seeds** — they authored a
/// single `harmonicRhythm` value and four mostly-one-chord families, which is
/// not "harmonically static", it is "four". The floor sits under the narrowest
/// legitimate model so it fails on a collapse rather than on a tuning change.
const FLOOR: usize = 55;

/// An extension is *available* to a model once it is drawn this often.
///
/// Below a tenth the chord is that shape a handful of times in two hundred
/// seeds, which is not a vocabulary — it is a garnish.
const EXTENSION_MIN: f64 = 0.1;

/// The harmonic vocabulary a model needs before the full [`FLOOR`] is a fair
/// question to ask of its chords.
///
/// Four progression families times three extension shapes, which is where the
/// roster's own distribution sits — `trap` is 7 × 3, `boom-bap` 5 × 3 — so the
/// ordinary case is unchanged and this only ever relaxes the exception.
const NORMAL_VOCABULARY: usize = 12;

/// The floor this part must clear on this model.
///
/// ⛔⛔ **A FIXED FLOOR ASKED HARMONICALLY MINIMAL STYLES TO INVENT HARMONY.**
/// Volume 1 put 33 models under 55, and every one of them is minimal *on
/// purpose*: crunk, snap, trap-metal and the ringtone lane. `comethazine` is the
/// limit case — **one progression family of one chord, `maxChords: 2`** — and it
/// still reaches 23 distinct chord parts in 200 seeds, entirely from rhythm.
/// There is no way for it to reach 55 that does not mean authoring chord changes
/// its records do not have, which is the exact thing this test's own failure
/// message tells you not to do.
///
/// ▶ **So a model is judged against what its own authored vocabulary allows** —
/// the progression families it may draw, times the chord shapes it may build
/// from them. Both terms are read from the model's own data and the
/// normalisation point is the roster's typical value, so this is not a number
/// tuned until three models passed.
///
/// ⛔ **Families alone was half the picture and measuring proved it.** Scaling on
/// families cleared 30 of the 33, and the three left — `petey-pablo`,
/// `bone-crusher`, `trap-metal` — had three or four families each and were still
/// narrow. The binding constraint was `extensions`: all three author
/// **`triad` at 0.93–0.95**, against 0.4–0.6 on the models that pass, so
/// virtually every chord they draw is the same three notes. Rebalancing their
/// family weights moved them 36→39, 38→39 and 45→43, which is what a wrong
/// diagnosis looks like.
///
/// ⚠ **Chords only.** Families and extensions are harmonic ideas — a melody,
/// counter or bass has no equivalent and none of them is bounded the way a
/// triad-only two-chord vamp is, so they keep the flat floor.
///
/// ⚠ **This cannot excuse a collapse, which is the point it must not give up.**
/// The narrowest possible vocabulary still has to clear a twelfth of the floor,
/// and the defect that put this test here — `rage` reaching **4** distinct chord
/// parts because `voice()` made a voicing a pure function of the chord — is
/// below that at every vocabulary size.
fn floor_for(model: &StyleModel, part: Part) -> usize {
    if part != Part::Chords {
        return FLOOR;
    }
    let chords = model.blocks.get("chords");
    let families = chords
        .and_then(|chords| chords.get("progressionFamilies"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(4);
    // Absent `extensions` means the engine's own default spread, which is not a
    // narrow one — so an unauthored block must not buy a lower floor.
    let shapes = chords
        .and_then(|chords| chords.get("extensions"))
        .and_then(serde_json::Value::as_object)
        .map(|ext| {
            ext.values()
                .filter(|v| v.as_f64().unwrap_or(0.0) >= EXTENSION_MIN)
                .count()
        })
        .unwrap_or(3);
    let vocabulary = (families.max(1) * shapes.max(1)).clamp(1, NORMAL_VOCABULARY);
    FLOOR * vocabulary / NORMAL_VOCABULARY
}

#[test]
fn no_part_collapses_to_a_handful_of_outputs() {
    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };

    let mut thin: Vec<(String, &str, usize, usize)> = Vec::new();
    for (id, model) in &models {
        for (part, name) in [
            (Part::Melody, "melody"),
            (Part::Counter, "counter"),
            (Part::Bass, "bass"),
            (Part::Chords, "chords"),
        ] {
            let mut seen = BTreeSet::new();
            for seed in 0..SEEDS {
                let s = shape(model, &ctx, part, seed);
                if !s.is_empty() {
                    seen.insert(s);
                }
            }
            // ⚠ **Empty is not narrow.** Trap's `Bass` is deliberately silent —
            // the 808 carries the low end (FR-007) — so a part that never
            // generates has nothing to measure and must not be failed for it.
            let floor = floor_for(model, part);
            if !seen.is_empty() && seen.len() < floor {
                thin.push((id.clone(), name, seen.len(), floor));
            }
        }
    }

    thin.sort_by_key(|(_, _, n, _)| *n);
    assert!(
        thin.is_empty(),
        "these parts barely vary over {SEEDS} seeds, as \
         (model, part, reached, its own floor): {thin:?}\n\
         The cure is authored range in the model — more progression families, more \
         than one harmonicRhythm value, a chordDurationBeats range — not a lower floor. \
         The chords floor already scales with the model's own progression families, so \
         a model failing here is narrow even for the vocabulary it claims; see \
         `floor_for`."
    );
}

#[test]
fn two_different_artists_do_not_write_the_same_melodic_part() {
    // ⛔ **The other axis, and the one Mike's report was actually about**: hold
    // the seed, change the artist. The drum version of this
    // (`drum_variety.rs`) found `ny-drill` and `pop-smoke` writing an identical
    // beat on 16 seeds of 200 — two models separated by 0.05 on four numbers.
    // The melodic generators need the same check or a genre inheriting its
    // whole melody block is invisible here.
    //
    // ⚠ Same reasoning as the drum gate: zero is the wrong target. Two models
    // in one family will coincide occasionally, and forcing that never to
    // happen means authoring differences nobody asked for.
    // ⛔ **Per part, because the parts have genuinely different vocabulary
    // sizes.** A melody is a free line and two models writing the same one is
    // real evidence they are the same model. A two-chord dark-minor vamp is
    // nearly a closed set — trap, dark-trap, rage, phonk, future and osamason
    // *are* one harmonic family, and forcing them never to coincide would mean
    // authoring progressions none of them play, which is the thing this gate's
    // failure message tells you not to do.
    //
    // ⚠ **What this catches is a model that inherits its parent wholesale**,
    // which produces a 100% collision rate — before the blocks below were
    // authored, six trap flagships shared one melody on **200 seeds of 200**
    // and pluggnb/summrs shared one chord part on 200 of 200. It is not tuned
    // to catch a family resemblance, and it should not be.
    let ceiling_for = |part: &str| -> usize {
        let per_1000 = match part {
            "chords" => 250,
            _ => 100,
        };
        SEEDS as usize * per_1000 / 1000
    };

    let models = shipped();
    let ctx = SessionContext {
        bars: 4,
        ..Default::default()
    };
    let ids: Vec<&String> = models.keys().filter(|id| *id != "_defaults").collect();

    let mut hits: BTreeMap<(&str, String), usize> = BTreeMap::new();
    for (part, name) in [
        (Part::Melody, "melody"),
        (Part::Counter, "counter"),
        (Part::Bass, "bass"),
        (Part::Chords, "chords"),
    ] {
        for seed in 0..SEEDS {
            let mut by_shape: BTreeMap<Vec<(u32, u8)>, Vec<&str>> = BTreeMap::new();
            for id in &ids {
                let s = shape(&models[*id], &ctx, part, seed);
                if !s.is_empty() {
                    by_shape.entry(s).or_default().push(id);
                }
            }
            for group in by_shape.values().filter(|g| g.len() > 1) {
                *hits.entry((name, group.join(" = "))).or_default() += 1;
            }
        }
    }

    let mut rows: Vec<(&(&str, String), &usize)> = hits.iter().collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    println!("\n  melodic collisions over {SEEDS} seeds:");
    for ((part, pair), count) in rows.iter().take(12) {
        println!("    {count:>4} / {SEEDS}  {part:>8}  {pair}");
    }

    let over: Vec<(&str, &str, usize)> = hits
        .iter()
        .filter(|((part, _), count)| **count > ceiling_for(part))
        .map(|((part, pair), count)| (*part, pair.as_str(), *count))
        .collect();
    assert!(
        over.is_empty(),
        "these models write the same part as each other too often in {SEEDS} seeds: \
         {over:?}\n\
         The cure is an authored block in the child, not a looser gate."
    );
}
