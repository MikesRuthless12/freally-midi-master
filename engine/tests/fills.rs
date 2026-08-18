//! Fills and pattern length.
//!
//! Consensus formula #20: variation events land at phrase boundaries and the
//! densest bars are the ones that close a phrase. That is checkable as a
//! histogram — which bars get a fill, over many seeds — and a histogram is
//! what this file mostly is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::generators::grid;
use engine::pattern::{Articulation, Lane, LaneTrack, Note};
use engine::StyleModel;
use serde_json::{json, Value};

const SEEDS: u64 = 60;

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

fn model(drums: Value) -> StyleModel {
    serde_json::from_value(json!({
        "id": "test", "type": "genre", "name": "Test", "drums": drums,
    }))
    .expect("the test model must parse")
}

fn ctx(bars: u16) -> SessionContext {
    SessionContext {
        bars,
        ..Default::default()
    }
}

fn notes(lanes: &[LaneTrack], want: Lane) -> Vec<Note> {
    lanes
        .iter()
        .find(|l| l.lane == want)
        .map(|l| l.notes.clone())
        .unwrap_or_default()
}

/// How many fill notes land in each bar, summed over seeds.
///
/// Counts the snare *and* the clap: west-coast club turns its fills over on the
/// clap rather than the snare, and a histogram that only looked at one lane
/// reported that genre as having no fills at all.
fn histogram(model: &StyleModel, bars: u16) -> Vec<usize> {
    let context = ctx(bars);
    let bar_ticks = context.ticks_per_bar();
    let mut counts = vec![0; usize::from(bars)];

    for seed in 0..SEEDS {
        let lanes = generate(model, &context, seed);
        for lane in [Lane::Snare, Lane::Clap] {
            for note in notes(&lanes, lane) {
                if note.articulation == Some(Articulation::Roll) {
                    counts[(note.start_tick / bar_ticks) as usize] += 1;
                }
            }
        }
    }
    counts
}

#[test]
fn a_small_fill_lands_every_second_bar_and_nowhere_else() {
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    for (bar, count) in counts.iter().enumerate() {
        let position = bar + 1;
        if position % 2 == 0 {
            assert!(*count > 0, "bar {position} should carry a fill");
        } else {
            assert_eq!(*count, 0, "bar {position} should be plain");
        }
    }
}

#[test]
fn the_eighth_bar_gets_the_big_one() {
    // The phrase-boundary shape: every second bar varies, and the bar that
    // closes the eight-bar phrase is the densest of them.
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    let biggest = counts.iter().max().unwrap();
    assert_eq!(
        counts[7], *biggest,
        "bar 8 should be the densest: {counts:?}"
    );
    assert!(
        counts[7] > counts[1] * 3 / 2,
        "the big fill should be clearly bigger than a small one: {counts:?}"
    );
}

#[test]
fn a_different_cycle_moves_the_fills() {
    // Rage fills every four bars rather than every two, and the difference has
    // to show in the output or the parameter is decoration.
    let m = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 4, "bigEveryBars": 8, "fillBeforeSection": false }
    }));
    let counts = histogram(&m, 8);

    for (bar, count) in counts.iter().enumerate() {
        let position = bar + 1;
        if position % 4 == 0 {
            assert!(*count > 0, "bar {position} should carry a fill: {counts:?}");
        } else {
            assert_eq!(*count, 0, "bar {position} should be plain: {counts:?}");
        }
    }
}

#[test]
fn the_last_bar_fills_so_the_loop_leads_somewhere() {
    // `fillBeforeSection` — a pattern should end *into* whatever comes next
    // rather than stopping dead at the loop point. Three bars is the test:
    // bar 3 is not on the two-bar cycle, so only the flag can fill it.
    let with_flag = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": true }
    }));
    let without = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": false }
    }));

    assert!(
        histogram(&with_flag, 3)[2] > 0,
        "the flag should fill bar 3"
    );
    assert_eq!(histogram(&without, 3)[2], 0, "without it, bar 3 is plain");
}

#[test]
fn two_four_and_eight_bar_patterns_all_end_on_a_fill() {
    // The three lengths the UI offers (FR-003). Whatever the length, the last
    // bar leads out of it.
    let m = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 2, "bigEveryBars": 8, "fillBeforeSection": true }
    }));

    for bars in [2u16, 4, 8] {
        let counts = histogram(&m, bars);
        assert!(
            *counts.last().unwrap() > 0,
            "a {bars}-bar pattern should fill its last bar: {counts:?}"
        );
        // ...and every note still belongs to the pattern.
        let context = ctx(bars);
        for seed in 0..SEEDS {
            for track in generate(&m, &context, seed) {
                for note in &track.notes {
                    assert!(
                        note.start_tick < context.total_ticks(),
                        "{bars} bars, seed {seed}: a fill ran past the end"
                    );
                }
            }
        }
    }
}

#[test]
fn a_fill_takes_the_end_of_its_bar_and_leaves_the_backbeat_alone() {
    // A fill is a run-up, not a replacement for the bar. The backbeat that the
    // fill is leading away from has to survive it.
    let m = model(json!({
        "snare": { "placement": "backbeat_24" },
        "fills": { "smallEveryBars": 1, "bigEveryBars": 99, "fillBeforeSection": false }
    }));
    let context = ctx(4);
    let beat = grid::ticks_per_beat(&context);

    for seed in 0..SEEDS {
        let snares = notes(&generate(&m, &context, seed), Lane::Snare);
        for bar in 0..4u32 {
            let bar_start = bar * context.ticks_per_bar();
            let backbeat = snares.iter().any(|n| {
                n.start_tick == bar_start + beat && n.articulation != Some(Articulation::Roll)
            });
            assert!(
                backbeat,
                "seed {seed}, bar {bar}: the 2 was eaten by the fill"
            );
        }
    }
}

#[test]
fn a_fill_keeps_the_ghost_it_plays_over() {
    // Drill's and-of-4 ghost answers the backbeat and lives in exactly the beat
    // a fill lands on. Clearing it cost the genre half of them.
    let m = model(json!({
        "snare": {
            "placement": "halftime_3",
            "ghost": { "prob": 1.0, "pos": ["4&"], "vel": [0.45, 0.45] }
        },
        "fills": { "smallEveryBars": 1, "bigEveryBars": 99, "fillBeforeSection": false }
    }));
    let context = ctx(4);

    for seed in 0..SEEDS {
        let ghosts = notes(&generate(&m, &context, seed), Lane::Snare)
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Ghost))
            .count();
        assert_eq!(ghosts, 4, "seed {seed}: a fill ate a ghost");
    }
}

#[test]
fn a_model_with_no_fills_block_gets_none() {
    let m = model(json!({ "snare": { "placement": "backbeat_24" } }));
    for seed in 0..SEEDS {
        assert!(
            notes(&generate(&m, &ctx(8), seed), Lane::Snare)
                .iter()
                .all(|n| n.articulation != Some(Articulation::Roll)),
            "seed {seed}: an unasked-for fill"
        );
    }
}

#[test]
fn the_ladder_flag_decides_what_the_big_fill_is_made_of() {
    // Trap asks for the subdivision ladder; drill does not. The two must not
    // produce the same fill, or the flag means nothing.
    let ladder = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 8, "bigEveryBars": 8, "snareRollLadder": true },
        "snareRoll": { "ladder": ["4", "8", "16", "32"], "velocityRampRange": [1, 127] }
    }));
    let plain = model(json!({
        "snare": { "placement": "halftime_3" },
        "fills": { "smallEveryBars": 8, "bigEveryBars": 8, "snareRollLadder": false }
    }));

    let context = ctx(8);
    let last_bar = |m: &StyleModel, seed: u64| -> Vec<u32> {
        let start = 7 * context.ticks_per_bar();
        notes(&generate(m, &context, seed), Lane::Snare)
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Roll) && n.start_tick >= start)
            .map(|n| n.start_tick - start)
            .collect()
    };

    // The ladder accelerates: its gaps get smaller. A plain roll does not.
    let rungs = last_bar(&ladder, 1);
    let flat = last_bar(&plain, 1);
    assert!(!rungs.is_empty() && !flat.is_empty());

    let gaps = |ticks: &[u32]| -> Vec<u32> { ticks.windows(2).map(|p| p[1] - p[0]).collect() };
    let ladder_gaps = gaps(&rungs);
    let flat_gaps = gaps(&flat);

    assert!(
        ladder_gaps.first() > ladder_gaps.last(),
        "the ladder should accelerate: {ladder_gaps:?}"
    );
    // ⛔ **A multiple of one subdivision, not all equal — and the difference is
    // TASK-131C.** A plain fill used to be a hardcoded, gapless 16th run, which
    // is why every model on the roster wrote one to four distinct fills and six
    // flagship trap artists wrote a byte-identical one. It now cuts a hole
    // (`gapProb`), so a gap of 480 appears in a 240 run — the roll is still on
    // *one grid*, which is the thing separating it from the ladder, but it is no
    // longer a machine.
    //
    // ⚠ The ladder is still distinguished by the assertion above: its
    // subdivision genuinely accelerates, so its first gap exceeds its last. A
    // hole cannot produce that, because a hole makes a gap *larger* later in the
    // run rather than smaller.
    // ⛔ **`step` or exactly `2 × step`, not "any multiple".** The first cut
    // asserted `g % step == 0` with `step` taken as the *minimum* gap — which is
    // true of almost any set of gaps and so gated nothing. `with_gaps` removes
    // at most one note, so a plain fill has one grid and at most one doubled
    // gap; a run that changed subdivision, or dropped two notes, would show a
    // gap this rejects.
    let step = *flat_gaps.iter().min().expect("a fill has gaps");
    assert!(
        flat_gaps.iter().all(|g| *g == step || *g == step * 2),
        "a plain fill is one grid with at most one hole: {flat_gaps:?} (step {step})"
    );
    assert!(
        flat_gaps.iter().filter(|g| **g == step * 2).count() <= 1,
        "at most one hole is cut: {flat_gaps:?}"
    );
}

#[test]
fn every_shipped_genre_fills_at_the_cycle_it_authors() {
    for (id, model) in shipped() {
        let Some(fills) = model.blocks.get("drums").and_then(|d| d.get("fills")) else {
            continue;
        };
        // `.max(1)` for the same reason `generators::drums::fills` clamps it: a
        // cycle of zero is a modulo by zero here and a fill in every bar there.
        let small = (fills
            .get("smallEveryBars")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize)
            .max(1);
        // ⛔ **The big cycle is half of "the cycle it authors" and was missing.**
        // Six shipped models — `dan-shay`, `byron-gallimore`, `eddie-bayers`,
        // `frank-rogers`, `mikey-reaves`, `peter-collins` — author a *bigger*
        // fill more often than a small one (16 and 8), and `generators::drums::
        // fills` writes a bar when **either** cycle lands. Reading only `small`
        // left those bars covered by the `position == 8` clause below rather
        // than by the model's own number, which is the same lie in the other
        // direction.
        let big = (fills
            .get("bigEveryBars")
            .and_then(Value::as_u64)
            .unwrap_or(8) as usize)
            .max(1);
        // ⛔ **Read, not assumed.** This used to say "the last bar always fills
        // — `fillBeforeSection` defaults on", and the default is only what a
        // model gets when it says nothing: ten shipped kits author it **off**
        // (`aaron-dessner`, `deana-carter`, `little-big-town`, `shane-mcanally`,
        // `deadmau5` and five more, every one of them at `smallEveryBars: 16`),
        // and for those `every_shipped_genre_fills_at_the_cycle_it_authors` was
        // demanding a fill the cycle they authored does not have. The unit test
        // `a_small_fill_lands_every_second_bar_and_nowhere_else` states the same
        // rule on a fixture and has always passed `fillBeforeSection: false`.
        let before_section = fills
            .get("fillBeforeSection")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let counts = histogram(&model, 8);
        for (bar, count) in counts.iter().enumerate() {
            let position = bar + 1;
            let last_bar = position == 8;
            if position % small == 0 || position % big == 0 || (last_bar && before_section) {
                assert!(*count > 0, "{id}: bar {position} should fill: {counts:?}");
            } else {
                // The negative half, without which the authored cycle is not
                // tested at all: filling *every* bar also satisfied "fills at
                // least on the cycle".
                assert_eq!(
                    *count, 0,
                    "{id}: bar {position} is off the {small}/{big}-bar cycle and should be plain: {counts:?}"
                );
            }
        }
    }
}

// ── The hi-hat's fill (TASK-043H) ────────────────────────────────────────────
//
// The hat is where trap, drill and plugg do their talking, and until this the
// engine gave it rolls but no *fill* — a phrase-end figure that breaks the
// stream and hands over to the next bar. These are the two questions that
// matter: does it land where the model said, and is every string it names one
// the engine can read.

/// Every model that authors a hat fill, with the block.
fn fill_models() -> Vec<(String, StyleModel, Value)> {
    shipped()
        .into_iter()
        .filter_map(|(id, model)| {
            let fill = model
                .blocks
                .get("drums")?
                .get("hihat")?
                .get("fill")
                .filter(|value| !value.is_null())?
                .clone();
            Some((id, model, fill))
        })
        .collect()
}

#[test]
fn the_hat_fill_vocabulary_is_one_the_engine_can_read() {
    // ⛔ **The gate the roadmap asked for in the same breath as the feature:**
    // `fill` is a new string vocabulary, and a typo costs that genre its fill
    // in silence — an unknown figure falls back to a plain roll and an unknown
    // landing point is dropped, so the model's authored intent simply stops
    // happening with nothing to see.
    let authored = fill_models();
    assert!(
        !authored.is_empty(),
        "no model authors a hat fill, so this gate is asserting nothing"
    );

    for (id, _, fill) in authored {
        for name in string_values(fill.get("at")) {
            assert!(
                engine::generators::rolls::can_read_fill_at(&name),
                "{id}: `{name}` is not a landing point the fill generator knows"
            );
        }
        for name in string_values(fill.get("figure")) {
            assert!(
                engine::generators::rolls::can_read_fill_figure(&name),
                "{id}: `{name}` is not a figure the fill generator knows"
            );
        }
        for name in string_values(fill.get("subdivision")) {
            assert!(
                grid::note_value_ticks(&name).is_some(),
                "{id}: `{name}` is not a note value"
            );
        }
    }
}

/// Every string in a value, in any of the dataset's three authoring forms.
fn string_values(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::String(one) => vec![one.clone()],
        Value::Array(many) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::Object(spec) => spec
            .get("values")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[test]
fn a_hat_fill_lands_at_the_phrase_end_and_nowhere_else() {
    // The roadmap's verify line: generate 8 bars for a hat-led genre and assert
    // a fill lands at the phrase end and nowhere else.
    //
    // ⛔ **Measured against the same model with the fill removed, not against a
    // density threshold.** The first cut asked whether the last beat was busier
    // than the rest, and that is the wrong question in two directions: a hat
    // stream is dense there anyway, and `gap` — a real figure — makes the last
    // beat *emptier*, which is the fill working rather than failing. An A/B
    // says what actually happened.
    //
    // The other half of "and nowhere else" is
    // `re_rolling_the_fill_does_not_move_the_hats_it_interrupts` below, which
    // holds every tick outside the window byte-identical.
    let ctx = SessionContext {
        bars: 8,
        ..Default::default()
    };
    let models = shipped();
    let beat = grid::ticks_per_beat(&ctx);
    let bar = ctx.ticks_per_bar();

    for (id, _, _) in fill_models() {
        let model = &models[&id];
        let mut without = model.clone();
        without
            .blocks
            .get_mut("drums")
            .and_then(|d| d.get_mut("hihat"))
            .and_then(Value::as_object_mut)
            .expect("a model authoring a fill authors a hihat block")
            .remove("fill");

        let last_beats = |model: &StyleModel, seed: u64| -> Vec<u32> {
            notes(&generate(model, &ctx, seed), Lane::ClosedHat)
                .iter()
                .map(|note| note.start_tick)
                .filter(|tick| tick % bar >= bar.saturating_sub(beat))
                .collect()
        };

        let changed = (0..SEEDS)
            .filter(|seed| last_beats(model, *seed) != last_beats(&without, *seed))
            .count();
        assert!(
            changed > 0,
            "{id}: removing the authored fill changed nothing in any of {SEEDS} seeds,              so it is not reaching the pattern"
        );
    }
}

#[test]
fn no_hat_is_open_and_shut_at_the_same_instant() {
    // ⛔⛔ **The hat engine's one hard rule, and the fill was breaking it.**
    // `hats()` deletes the closed hit underneath every open hat it places —
    // "one hi-hat cannot be open and shut at the same instant" — and then
    // `hat_fills` cleared its window and wrote a fresh stream across it, putting
    // a closed hat straight back on the open hat's tick. Export fires GM 42 and
    // 46 together at that instant, which is not a hat sound at all, and the
    // regenerated trap golden carried it.
    //
    // ⚠ **Over every shipped model and every seed, not just the ones that
    // author a fill.** The invariant belongs to the hat engine; the fill is only
    // the way it was most recently broken, and the next way should fail here
    // too.
    let ctx = SessionContext {
        bars: 8,
        ..Default::default()
    };

    for (id, model) in shipped() {
        for seed in 0..SEEDS {
            let lanes = generate(&model, &ctx, seed);
            let open: Vec<u32> = notes(&lanes, Lane::OpenHat)
                .iter()
                .map(|note| note.start_tick)
                .collect();
            if open.is_empty() {
                continue;
            }
            for closed in notes(&lanes, Lane::ClosedHat) {
                assert!(
                    !open.contains(&closed.start_tick),
                    "{id} seed {seed}: a closed hat sits on the open hat at tick {}",
                    closed.start_tick
                );
            }
        }
    }
}

#[test]
fn re_rolling_the_fill_does_not_move_the_hats_it_interrupts() {
    // ⛔ **The reason the fill has its own `drums/hats/fill` stream.** A fill
    // drawn from the hat stream would shift every hat after it, so changing a
    // fill parameter would rewrite the whole hat part — which is the "rerolling
    // one part leaves every other byte-identical" property `rng.rs` is built
    // around, one level down.
    //
    // Proved by generating with and without the fill block and comparing the
    // hats *outside* every fill window.
    //
    // ⚠ **Placement, not velocity, and the difference is honest rather than
    // convenient.** `humanize` walks a lane in note order and draws per note,
    // so removing or adding notes reshuffles every later draw — the velocities
    // downstream of a fill do move, and the golden diff for `uk-drill` shows
    // exactly that. This is not new: hat rolls have always had it, for the same
    // reason. What the seeded stream buys is that no hat *changes position*,
    // which is the part a producer would hear as the pattern being rewritten.
    let ctx = SessionContext {
        bars: 8,
        ..Default::default()
    };
    let models = shipped();
    let model = &models["trap"];
    let beat = grid::ticks_per_beat(&ctx);
    let bar = ctx.ticks_per_bar();

    let mut without = model.clone();
    without
        .blocks
        .get_mut("drums")
        .and_then(|d| d.get_mut("hihat"))
        .and_then(Value::as_object_mut)
        .expect("trap authors a hihat block")
        .remove("fill");

    for seed in [1_u64, 7, 42] {
        let outside = |lanes: &[LaneTrack]| -> Vec<u32> {
            notes(lanes, Lane::ClosedHat)
                .iter()
                .map(|note| note.start_tick)
                .filter(|tick| tick % bar < bar.saturating_sub(beat))
                .collect()
        };
        assert_eq!(
            outside(&generate(model, &ctx, seed)),
            outside(&generate(&without, &ctx, seed)),
            "seed {seed}: the fill moved hats outside its own window"
        );
    }
}
