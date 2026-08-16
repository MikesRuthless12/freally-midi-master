//! The novelty guard, against the shipped dataset (FR-011, TASK-039).
//!
//! `engine/src/novelty.rs`'s own unit tests pin the fingerprint — what a step
//! is, what collides with what, how the file parses. These ask the questions
//! only the real roster can answer: does a planted fragment actually get
//! regenerated out of a real melody, does the guard leave a clean take alone,
//! and does it cost less than the 5 ms FR-011 budgets.

mod common;

use std::time::Instant;

use engine::generators::bass;
use engine::novelty::{self, Outcome, Table, N_LOOSE, N_TIGHT};
use engine::parts::{self, Seeds};
use engine::pattern::{LaneTrack, Part};
use engine::SessionContext;

/// A model with a melody worth screening.
fn model(id: &str) -> engine::StyleModel {
    common::shipped_models()
        .remove(id)
        .unwrap_or_else(|| panic!("`{id}` must be in the shipped dataset"))
}

/// The notes a part generates at a seed, before the guard sees them.
fn take(id: &str, part: Part, seed: u64) -> Vec<LaneTrack> {
    parts::render(
        &model(id),
        &SessionContext::default(),
        Seeds::shared(seed),
        part,
    )
}

/// A table holding exactly the contour of one generated take.
///
/// This is the planted fragment: the guard is pointed at a table that says the
/// melody it is about to write is a known hook, so the only way past the screen
/// is to write a different one.
fn table_of(lanes: &[LaneTrack]) -> Table {
    let melodies: Vec<_> = lanes
        .iter()
        .map(|lane| novelty::steps(&lane.notes))
        .collect();
    Table::from_melodies(&melodies)
}

#[test]
fn a_planted_fragment_is_regenerated() {
    // The verify line from the roadmap, and the AC from FR-011.
    let id = "travis-scott";
    let seed = 20_260_808;
    let ctx = SessionContext::default();
    let model = model(id);

    let planted = {
        let lanes = parts::render(&model, &ctx, Seeds::shared(seed), Part::Melody);
        assert!(
            !novelty::steps(&lanes[0].notes).is_empty(),
            "the fixture needs a melody with notes in it"
        );
        table_of(&lanes)
    };

    let (screened, kept, report) = novelty::screen(&planted, Part::Melody, true, seed, |take| {
        engine::parts::render(&model, &ctx, Seeds::shared(take), Part::Melody)
    });

    assert_eq!(
        report.outcome,
        Outcome::Regenerated,
        "a take that quotes the table must not be the one returned"
    );
    assert!(report.takes > 1, "the guard has to have drawn again");
    assert_ne!(kept, seed, "the kept take came from a different seed");

    // And what came back is genuinely clear, not merely different.
    for lane in &screened {
        assert!(
            !planted.hits(&novelty::steps(&lane.notes), N_TIGHT),
            "the returned take still quotes the table"
        );
    }
}

#[test]
fn the_guard_is_deterministic_through_a_regeneration() {
    // ⛔ The property the whole design rests on: a rejected take is redrawn at a
    // *derived* seed, never a fresh one, so a saved seed still rebuilds the
    // pattern the producer heard — even when the pattern they heard was the
    // guard's third attempt.
    let id = "future";
    let seed = 999;
    let ctx = SessionContext::default();
    let model = model(id);
    let planted = table_of(&parts::render(
        &model,
        &ctx,
        Seeds::shared(seed),
        Part::Melody,
    ));

    let run = || {
        novelty::screen(&planted, Part::Melody, true, seed, |take| {
            parts::render(&model, &ctx, Seeds::shared(take), Part::Melody)
        })
    };
    let (first, first_seed, first_report) = run();
    let (second, second_seed, second_report) = run();

    assert_eq!(first_report, second_report);
    assert_eq!(first_seed, second_seed);
    assert_eq!(first, second, "two runs of one seed must be byte-identical");
}

#[test]
fn a_clean_take_is_left_exactly_as_it_was() {
    // The guard must be invisible when it finds nothing — same notes, one take,
    // no perturbed seed. This is what keeps the golden snapshots meaningful.
    let ctx = SessionContext::default();
    let model = model("metro-boomin");
    let unrelated = Table::parse("0x0000000000000001\n0x0000000000000002\n").unwrap();

    let (lanes, kept, report) = novelty::screen(&unrelated, Part::Melody, true, 4242, |take| {
        parts::render(&model, &ctx, Seeds::shared(take), Part::Melody)
    });

    assert_eq!(report.outcome, Outcome::Clear);
    assert_eq!(report.takes, 1);
    assert_eq!(kept, 4242);
    assert_eq!(
        lanes,
        parts::render(&model, &ctx, Seeds::shared(4242), Part::Melody)
    );
}

#[test]
fn the_shipped_roster_walks_past_the_bundled_table() {
    // ⛔ **This is the test that says the guard is not silently rewriting the
    // record.** The bundled table is public-domain melodies; if a shipped
    // artist's melody collided with one, every generation of that artist would
    // be a retry and nobody would know. A failure here is not necessarily a
    // bug — it is news, and it needs a human to look at which contour matched.
    let table = novelty::bundled();
    assert!(!table.is_empty(), "the bundled table must have loaded");

    let ctx = SessionContext::default();
    for (id, model) in common::shipped_models() {
        for part in [Part::Melody, Part::Counter] {
            for seed in [1_u64, 7, 42, 1_000, 20_260_808] {
                let lanes = parts::render(&model, &ctx, Seeds::shared(seed), part);
                for lane in &lanes {
                    let steps = novelty::steps(&lane.notes);
                    assert!(
                        !table.hits(&steps, N_TIGHT),
                        "{id} {part:?} at seed {seed} quotes the reference table"
                    );
                }
            }
        }
    }
}

#[test]
fn screening_costs_less_than_the_five_millisecond_budget() {
    // FR-011: "< 5 ms overhead". Measured as the screen alone — the fingerprint
    // and the lookups — because that is the cost the guard adds to a generation
    // that passes, which is every generation in practice.
    let ctx = SessionContext::default();
    let model = model("travis-scott");
    let lanes = parts::render(&model, &ctx, Seeds::shared(3), Part::Melody);
    let table = novelty::bundled();

    // 100 screens, so a single fast one cannot be a timer artifact.
    let runs = 100;
    let started = Instant::now();
    let mut hits = 0;
    for _ in 0..runs {
        for lane in &lanes {
            let steps = novelty::steps(&lane.notes);
            hits += usize::from(table.hits(&steps, N_TIGHT));
            hits += usize::from(table.hits(&steps, N_LOOSE));
        }
    }
    let each = started.elapsed() / runs;
    assert_eq!(hits, 0, "the fixture is supposed to be clear");
    assert!(
        each.as_millis() < 5,
        "one screen took {each:?}, over FR-011's 5 ms"
    );
}

#[test]
fn the_parts_the_guard_skips_are_untouched_by_it() {
    // Drums and Chords are deliberately not screened — no contour, and a
    // polyphonic stack respectively. A **kick-locked** bass joins them, because
    // rerolling a line that copies the kick's ticks would trade the lock that
    // makes it sit with the drums for a match nobody can hear as a quotation.
    //
    // Proved by pointing the guard at a table built from the part's *own*
    // contour — which would force a retry if it were screening — and asserting
    // the first take comes straight back.
    let ctx = SessionContext::default();
    // ⚠ **`killer-mike`, not `pop-smoke`, and the swap is the finding.** The
    // first version of this case used a drill model on the assumption that a
    // bassline is kick-locked by default — `pop-smoke` authors
    // `independent_riff`, and so do 194 other shipped models. Seven models in the
    // whole roster pair a real bass part with `mirror_kick`; this is one, and the
    // assertion below fails loudly rather than quietly proving nothing if that
    // stops being true.
    let model = model("killer-mike");
    assert!(
        bass::follows_the_kick(&model),
        "this case needs a kick-locked bass; killer-mike's has stopped being one"
    );
    for part in [Part::Drums, Part::Chords, Part::Bass] {
        let planted = table_of(&parts::render(&model, &ctx, Seeds::shared(11), part));
        let (lanes, kept, report) = novelty::screen(&planted, part, true, 11, |take| {
            parts::render(&model, &ctx, Seeds::shared(take), part)
        });
        assert_eq!(report.outcome, Outcome::NotScreened, "{part:?}");
        assert_eq!(kept, 11, "{part:?}");
        assert_eq!(
            lanes,
            parts::render(&model, &ctx, Seeds::shared(11), part),
            "{part:?}"
        );
    }
}

#[test]
fn a_bass_that_plays_its_own_figure_is_screened() {
    // ⛔⛔ **The gap the old rule left, and it was 207 models wide.** `screens`
    // excluded the bass outright on the argument that a bassline is locked to
    // the kick — true of `mirror_kick` and false of the other four rhythms
    // `bass.rs` reads. 194 shipped models author `independent_riff` alone, and
    // an independent riff is as recognisable as any topline: a great many
    // records are known by their bass figure.
    //
    // ⚠ **This matters more since the roster was returned to its researched
    // values** (owner's instruction, 2026-08-15): models may now reach the same
    // figure by design, so the guard — not model-to-model difference — is what
    // keeps the figure off something somebody already owns.
    let ctx = SessionContext::default();
    let model = model("afrobeats");
    assert!(
        !bass::follows_the_kick(&model),
        "this case needs a bass that places its own onsets; afrobeats' has stopped being one"
    );

    let first = parts::render(&model, &ctx, Seeds::shared(11), Part::Bass);
    let planted = table_of(&first);
    let (lanes, kept, report) = novelty::screen(
        &planted,
        Part::Bass,
        bass::follows_the_kick(&model),
        11,
        |take| parts::render(&model, &ctx, Seeds::shared(take), Part::Bass),
    );

    assert_ne!(
        report.outcome,
        Outcome::NotScreened,
        "an independent bass figure went past the guard untouched"
    );
    assert_ne!(kept, 11, "the guard kept the very take the table describes");
    assert_ne!(lanes, first, "the same notes came back after a rejection");
}

#[test]
fn a_take_that_cannot_escape_still_returns_notes() {
    // ⛔ **The producer pressed Generate, so notes come back whatever the guard
    // thinks.** A generator that ignores its seed cannot escape any table, and
    // the guard has to give up rather than loop or return nothing.
    let fixed = take("trap", Part::Melody, 5);
    let planted = table_of(&fixed);
    let (lanes, _, report) = novelty::screen(&planted, Part::Melody, true, 5, |_| fixed.clone());

    assert_eq!(report.outcome, Outcome::Exhausted);
    assert_eq!(report.takes, novelty::MAX_RETRIES + 1);
    assert_eq!(
        lanes, fixed,
        "the last take is returned rather than nothing"
    );
}

#[test]
fn song_mode_reaches_the_guard_at_all() {
    // ⛔⛔ **Song Mode was the one path that never ran the guard.** TASK-039
    // installed the screen in `parts::render`, which is what a single-pattern
    // request goes through — and `arrange::render_section` calls
    // `melody::generate` and `counter::generate` directly, so every melody and
    // countermelody in every section of every arrangement shipped unscreened,
    // while the changelog said the screen runs on "every melody and
    // countermelody… before you ever hear it". A rule installed at one door
    // rather than at the seam both doors go through.
    //
    // ⛔⛔ **Read off the source, and that is deliberate — the obvious test
    // could not fail.** The first version of this generated songs across the
    // roster and asserted none of them quoted the bundled table. It passed with
    // the fix reverted, for the same reason
    // `the_shipped_roster_walks_past_the_bundled_table` passes: no shipped
    // model collides with the table today, so screened and unscreened output
    // are identical and nothing observable separates them. Planting a fragment
    // is what proves the screen for the Melody tab, and `arrange` takes no
    // table to plant into — it reaches for `novelty::bundled()` itself.
    //
    // So what is asserted is the wiring, which is the thing that was missing
    // and the thing that can silently go missing again: a call to either
    // melodic generator inside `arrange.rs` must sit inside a `novelty::screen`
    // closure. Crude, and it cannot pass vacuously.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/arrange.rs"),
    )
    .expect("arrange.rs must be readable");

    let mut screened = 0;
    for (name, call) in [
        ("melody", "melody::generate("),
        ("counter", "counter::generate("),
        // ⛔ **The bass joined them on 2026-08-15**, when the guard stopped
        // excluding every bassline and started excluding only the kick-locked
        // ones. Song Mode is where a producer generates the most bars, so a
        // screen that ran on the pattern path and not here would be the same
        // one-door failure this whole test exists to catch.
        ("bass", "bass::generate("),
    ] {
        let mut from = 0;
        let mut seen = 0;
        while let Some(at) = source[from..].find(call) {
            let at = from + at;
            seen += 1;
            // The `novelty::screen(` that opens this closure, if there is one:
            // look back over the handful of lines a closure header occupies.
            let window = &source[at.saturating_sub(400)..at];
            assert!(
                window.contains("novelty::screen("),
                "`{name}::generate` is called at byte {at} of arrange.rs without a \
                 `novelty::screen` around it — Song Mode would ship an unscreened {name}"
            );
            screened += 1;
            from = at + call.len();
        }
        assert!(
            seen > 0,
            "no `{name}::generate` call in arrange.rs at all — if Song Mode stopped \
             generating one, this gate is now watching nothing and needs rewriting"
        );
    }
    assert_eq!(
        screened, 3,
        "expected exactly one screened call per screened part — melody, counter and bass"
    );
}
