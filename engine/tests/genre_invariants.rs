//! One test per authored genre, asserting the thing that makes it that genre.
//!
//! These are the tests that fail when a model is edited carelessly or the
//! engine changes underneath it. Each names the research the claim comes from,
//! and each is statistical — 100 seeds — because a genre is a distribution, not
//! a pattern. **A failure here means the model or the engine is wrong, not the
//! test**: the numbers come from the research, so moving a bound to make it
//! pass is moving the genre.
//!
//! The three Phase 0 genres (trap, uk-drill, rage) are covered in
//! `drums_core.rs`, `drums_hats.rs`, `rolls.rs` and `bass808.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::context::SessionContext;
use engine::generators::drums::generate;
use engine::generators::grid;
use engine::pattern::{Articulation, Lane, LaneTrack, Note};
use engine::StyleModel;
use serde_json::Value;

const SEEDS: u64 = 100;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

fn shipped() -> BTreeMap<String, StyleModel> {
    let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
    let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
    assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
    models
}

fn model(id: &str) -> StyleModel {
    shipped()
        .remove(id)
        .unwrap_or_else(|| panic!("`{id}` must ship"))
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

/// Every note of a lane across `SEEDS` patterns.
fn sweep(model: &StyleModel, lane: Lane, bars: u16) -> Vec<(u64, Note)> {
    let context = ctx(bars);
    (0..SEEDS)
        .flat_map(|seed| {
            notes(&generate(model, &context, seed), lane)
                .into_iter()
                .map(move |note| (seed, note))
        })
        .collect()
}

/// Main hits only — not ghosts, and not the fill that ends the bar.
fn is_backbeat(note: &Note) -> bool {
    !matches!(
        note.articulation,
        Some(Articulation::Ghost) | Some(Articulation::Roll)
    )
}

fn beat(context: &SessionContext) -> u32 {
    grid::ticks_per_beat(context)
}

// ---------------------------------------------------------------- drill family

#[test]
fn chicago_drill_is_straighter_than_the_uk_strain() {
    // Research ch. 1 §2: Chicago is "sparser, straighter" — the UK's triplet
    // groupings and heavy offbeat lean are what it is defined against.
    let context = ctx(4);
    let offbeat_share = |m: &StyleModel| {
        let kicks = sweep(m, Lane::Kick, 4);
        let offbeat = kicks
            .iter()
            .filter(|(_, n)| {
                grid::is_offbeat_eighth((n.start_tick % context.ticks_per_bar()) / grid::SIXTEENTH)
            })
            .count();
        offbeat as f64 / kicks.len() as f64
    };

    let chicago = offbeat_share(&model("chicago-drill"));
    let uk = offbeat_share(&model("uk-drill"));
    assert!(
        chicago < uk,
        "chicago ({chicago:.2}) should lean offbeat less than the UK ({uk:.2})"
    );
}

#[test]
fn chicago_drills_808_mostly_holds_one_pitch() {
    // "808 mostly static pitch, few slides" — the opposite of the UK marker.
    let chicago = sweep(&model("chicago-drill"), Lane::Bass808, 4);
    let slides = chicago
        .iter()
        .filter(|(_, n)| n.slide_to_pitch.is_some())
        .count();
    let share = slides as f64 / chicago.len() as f64;
    assert!(
        share < 0.15,
        "chicago drill slid on {share:.2} of its 808 notes — that is the UK's marker"
    );
    assert!(!chicago.is_empty());
}

#[test]
fn ny_drill_moves_its_snare_from_three_to_four_across_two_bars() {
    // Research ch. 1 §2 NY/Brooklyn: "snare beat 3 (variant: 3 in bar 1 / 4 in
    // bar 2)". This is the two-bar form the whole genre is heard through.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();

    for (seed, note) in sweep(&model("ny-drill"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let bar = note.start_tick / bar_ticks;
        let within = note.start_tick % bar_ticks;
        let expected = if bar.is_multiple_of(2) {
            beat(&context) * 2
        } else {
            beat(&context) * 3
        };
        // NY quantizes hard but still nudges; allow the authored 0–2 ms.
        assert!(
            within.abs_diff(expected) <= 8,
            "seed {seed}, bar {bar}: snare at {within}, expected near {expected}"
        );
    }
}

// ----------------------------------------------------------------- plugg family

#[test]
fn plugg_lets_the_open_hats_carry_the_pattern() {
    // Research ch. 1 §4: "minimal closed (plugg near hat-less); OPEN hats carry
    // the pattern". So plugg is the one genre where open hats are not a garnish.
    let plugg = model("plugg");
    let open = sweep(&plugg, Lane::OpenHat, 4).len();
    let closed = sweep(&plugg, Lane::ClosedHat, 4).len();

    assert!(open > 0, "plugg produced no open hats at all");
    assert!(
        open * 3 > closed,
        "open hats ({open}) should be a real share against closed ({closed})"
    );

    // ...and against a genre where they are a garnish.
    let trap_open = sweep(&model("trap"), Lane::OpenHat, 4).len();
    assert!(
        open > trap_open * 2,
        "plugg ({open}) should open its hats far more than trap ({trap_open})"
    );
}

#[test]
fn pluggs_808_bounces_rather_than_sustains() {
    // The "Light 808": 0 ms attack, ~200 ms release — staccato, not legato.
    // Running it legato is what would make plugg sound like trap.
    for (seed, note) in sweep(&model("plugg"), Lane::Bass808, 4) {
        assert_eq!(
            note.articulation,
            Some(Articulation::Staccato),
            "seed {seed}: plugg's 808 sustained"
        );
        assert!(note.len_ticks <= grid::SIXTEENTH, "seed {seed}: too long");
    }
}

#[test]
fn plugg_keeps_its_low_passed_clap_flag_for_the_kit() {
    // Not a note-level property — the flag is what tells the kit to muffle the
    // clap, and losing it in an edit would be silent.
    let plugg = model("plugg");
    assert_eq!(
        plugg.blocks["drums"].pointer("/snare/lowPassedClap"),
        Some(&Value::Bool(true))
    );
    // And the clap is actually layered, or there is nothing to low-pass.
    assert!(!sweep(&plugg, Lane::Clap, 2).is_empty());
}

#[test]
fn pluggnb_switches_to_the_r_and_b_backbone() {
    // "Full-time variant (pluggnb 130+): kick 1&3, snare 2&4, offbeat hats".
    // It inherits everything else from plugg; this is the difference.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    for (seed, note) in sweep(&model("pluggnb"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % bar_ticks;
        assert!(
            backbeat.contains(&within),
            "seed {seed}: pluggnb snare at {within} is not on 2 or 4"
        );
    }
}

// ------------------------------------------------------------------------ jerk

#[test]
fn jerk_displaces_its_backbeat_off_the_grid() {
    // Research ch. 1 §5, the marker: "backbeat displaced ±1/32–1/16 off-grid".
    // Alone among these genres, jerk wants its timing loosened, so this is a
    // grammar displacement rather than humanizer jitter — it survives even a
    // hard-quantized session.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    let mut displaced = 0;
    let mut total = 0;
    for (_, note) in sweep(&model("jerk"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % bar_ticks;
        let nearest = backbeat
            .iter()
            .map(|b| within.abs_diff(*b))
            .min()
            .unwrap_or(u32::MAX);
        total += 1;
        if nearest > 0 {
            displaced += 1;
        }
        // Off-grid, not in a different bar: 4–12 ms at 140 BPM is 9–27 ticks.
        assert!(nearest <= 40, "a {nearest}-tick displacement is a new beat");
    }

    assert!(total > 0);
    let share = displaced as f64 / total as f64;
    assert!(
        share > 0.9,
        "jerk's snare should almost always sit off the grid, got {share:.2}"
    );
}

#[test]
fn jerk_is_the_one_genre_that_asks_to_be_loosened() {
    // `quantizeStrength` well below every other genre — "intentionally 'off'
    // quantization; DO randomize snare timing".
    let jerk = model("jerk");
    let strength = jerk
        .session
        .as_ref()
        .and_then(|s| s.humanize.as_ref())
        .and_then(|h| h.quantize_strength)
        .expect("jerk states a quantize strength");
    assert!(strength < 0.7, "jerk quantized at {strength}");

    for (id, other) in shipped() {
        if id == "jerk" || id == "_defaults" {
            continue;
        }
        if let Some(theirs) = other
            .session
            .as_ref()
            .and_then(|s| s.humanize.as_ref())
            .and_then(|h| h.quantize_strength)
        {
            assert!(theirs > strength, "{id} is looser than jerk ({theirs})");
        }
    }
}

// ----------------------------------------------------------------------- phonk

#[test]
fn phonk_drives_a_denser_kick_than_trap() {
    // "Driving, denser than trap (4–6/bar incl. offbeats)".
    let per_bar = |id: &str| {
        let m = model(id);
        sweep(&m, Lane::Kick, 4).len() as f64 / (SEEDS as f64 * 4.0)
    };
    let phonk = per_bar("phonk");
    let trap = per_bar("trap");

    assert!(
        (4.0..=6.0).contains(&phonk),
        "phonk should run 4–6 kicks a bar, got {phonk:.2}"
    );
    assert!(phonk > trap, "phonk ({phonk:.2}) vs trap ({trap:.2})");
}

#[test]
fn phonks_808_doubles_every_kick_and_glides_by_octaves() {
    // "808 doubled on EVERY kick" and "OCTAVE glides are the signature".
    let phonk = model("phonk");
    let context = ctx(4);

    for seed in 0..SEEDS {
        let lanes = generate(&phonk, &context, seed);
        let kicks = notes(&lanes, Lane::Kick).len();
        let bass = notes(&lanes, Lane::Bass808).len();
        assert_eq!(bass, kicks, "seed {seed}: the 808 left a kick undoubled");
    }

    let slides: Vec<i16> = sweep(&phonk, Lane::Bass808, 4)
        .iter()
        .filter_map(|(_, n)| {
            n.slide_to_pitch
                .map(|t| (i16::from(t) - i16::from(n.pitch)).abs())
        })
        .collect();
    assert!(!slides.is_empty(), "phonk never slid");

    let octaves = slides.iter().filter(|d| **d % 12 == 0).count();
    let share = octaves as f64 / slides.len() as f64;
    assert!(
        share > 0.4,
        "octaves should dominate phonk's glides, got {share:.2}"
    );
}

// -------------------------------------------------------------- west coast club

#[test]
fn west_coast_turns_over_on_a_clap_roll() {
    // Research ch. 1 §6: "16th CLAP ROLLS as turnaround fills (named
    // Mustard-era device)". Not a snare fill with a different sample — the roll
    // is on the clap.
    let west = model("west-coast-club");
    let clap_rolls = sweep(&west, Lane::Clap, 8)
        .iter()
        .filter(|(_, n)| n.articulation == Some(Articulation::Roll))
        .count();
    let snare_rolls = sweep(&west, Lane::Snare, 8)
        .iter()
        .filter(|(_, n)| n.articulation == Some(Articulation::Roll))
        .count();

    assert!(clap_rolls > 0, "no clap rolls at all");
    assert_eq!(snare_rolls, 0, "the turnaround should be on the clap");
}

#[test]
fn west_coast_keeps_a_full_time_backbeat() {
    // "Snare/clap: FULL-TIME 2 & 4" — the thing that separates it from the
    // half-time trap family it shares a tempo range with.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    for (seed, note) in sweep(&model("west-coast-club"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % bar_ticks;
        assert!(
            backbeat.contains(&within),
            "seed {seed}: snare at {within} is not the 2 or the 4"
        );
    }
}

// -------------------------------------------------------------------- boom bap

#[test]
fn boom_bap_swings_its_hats_at_the_classic_mpc_setting() {
    // 58% — the classic MPC shuffle. Applied by the humanizer, so this asserts
    // the model states it and the value is the researched one.
    let swing = model("boom-bap")
        .session
        .as_ref()
        .and_then(|s| s.swing.as_ref())
        .map(|s| s.amount)
        .expect("boom bap states its swing");
    assert!(
        (0.575..=0.585).contains(&swing),
        "boom bap should sit at the 58% MPC setting, got {swing}"
    );
}

#[test]
fn boom_bap_is_a_kit_and_not_a_sub() {
    // "bass808": null — a sampled break with a sub under it is a different
    // record. The null has to survive inheritance from `_defaults`, which does
    // define an 808.
    assert!(sweep(&model("boom-bap"), Lane::Bass808, 4).is_empty());
    assert!(sweep(&model("rnb-2000s"), Lane::Bass808, 4).is_empty());
    assert!(sweep(&model("country-train"), Lane::Bass808, 4).is_empty());
    // ...while the genres that do want one still have it.
    assert!(!sweep(&model("trap"), Lane::Bass808, 4).is_empty());
}

#[test]
fn boom_bap_fills_the_e_and_a_slots_with_ghosts() {
    // "Ghost snares 20–40% on e/a slots" — the detail that makes a boom bap
    // loop breathe rather than march.
    let ghosts: Vec<u32> = sweep(&model("boom-bap"), Lane::Snare, 4)
        .iter()
        .filter(|(_, n)| n.articulation == Some(Articulation::Ghost))
        .map(|(_, n)| (n.start_tick % 3840) / grid::SIXTEENTH)
        .collect();

    assert!(!ghosts.is_empty(), "boom bap produced no ghost snares");
    for index in &ghosts {
        assert!(
            grid::is_sixteenth_offbeat(*index),
            "a ghost landed on 16th {index}, which is not an e or an a"
        );
    }
}

// ----------------------------------------------------------------------- r&b

#[test]
fn rnb_adds_its_and_of_four_every_other_bar() {
    // "Kick: 1 + and-of-2 base; add and-of-4 every other bar."
    let context = ctx(8);
    let bar_ticks = context.ticks_per_bar();
    let and_of_four = beat(&context) * 3 + grid::SIXTEENTH * 2;

    let mut odd_bars = 0;
    let mut even_bars = 0;
    for (_, note) in sweep(&model("rnb-2000s"), Lane::Kick, 8) {
        if note.start_tick % bar_ticks != and_of_four {
            continue;
        }
        if (note.start_tick / bar_ticks) % 2 == 1 {
            odd_bars += 1;
        } else {
            even_bars += 1;
        }
    }

    assert!(odd_bars > 0, "the and-of-4 lead-in never happened");
    assert!(
        odd_bars > even_bars * 2,
        "it should land on every *other* bar: {odd_bars} vs {even_bars}"
    );
}

// -------------------------------------------------------------------- dnb

#[test]
fn liquid_dnb_locks_its_snares_to_two_and_four() {
    // "Core two-step: snares LOCKED 2 & 4" — locked, so not even the off-grid
    // nudge other genres take.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    for (seed, note) in sweep(&model("liquid-dnb"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % bar_ticks;
        assert!(
            backbeat.contains(&within),
            "seed {seed}: a locked snare moved to {within}"
        );
    }
}

#[test]
fn liquid_dnb_puts_its_second_kick_on_the_and_of_three() {
    // The second-kick position is what sets the flavour: straight is beat 3,
    // classic step is the and-of-3, neuro is 2a. Liquid takes the classic step.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let and_of_three = beat(&context) * 2 + grid::SIXTEENTH * 2;

    let hits = sweep(&model("liquid-dnb"), Lane::Kick, 4);
    let stepped = hits
        .iter()
        .filter(|(_, n)| n.start_tick % bar_ticks == and_of_three)
        .count();
    let downbeats = hits
        .iter()
        .filter(|(_, n)| n.start_tick % bar_ticks == 0)
        .count();

    assert!(downbeats > 0, "the 1 is the anchor");
    assert!(
        stepped as f64 > downbeats as f64 * 0.7,
        "the and-of-3 should be near-constant: {stepped} against {downbeats} downbeats"
    );
}

// ------------------------------------------------------------------- country

#[test]
fn the_train_beat_is_a_sixteenth_stream_over_walking_quarters() {
    // Research ch. 1 §11: "continuous 1/16 snare stream, accents 2&4, kick
    // quarters". The densest snare lane in the dataset by a wide margin.
    let country = model("country-train");
    let context = ctx(4);

    for seed in 0..SEEDS {
        let lanes = generate(&country, &context, seed);
        let snares = notes(&lanes, Lane::Snare);
        // 16 a bar, minus whatever the fill replaced at the end of bar 4.
        assert!(
            snares.len() >= 60,
            "seed {seed}: a train beat should be a stream, got {}",
            snares.len()
        );

        let kicks: Vec<u32> = notes(&lanes, Lane::Kick)
            .iter()
            .map(|n| n.start_tick % context.ticks_per_bar())
            .collect();
        for kick in &kicks {
            assert!(
                kick % beat(&context) == 0,
                "seed {seed}: a country kick left the quarters at {kick}"
            );
        }
    }
}

#[test]
fn the_train_beat_accents_the_backbeat() {
    let context = ctx(2);
    let backbeat = [beat(&context), beat(&context) * 3];
    let accents: Vec<u32> = sweep(&model("country-train"), Lane::Snare, 2)
        .iter()
        .filter(|(_, n)| n.articulation == Some(Articulation::Accent))
        .map(|(_, n)| n.start_tick % context.ticks_per_bar())
        .collect();

    assert!(
        !accents.is_empty(),
        "a train beat with no accents is a buzz"
    );
    for tick in &accents {
        assert!(backbeat.contains(tick), "an accent landed off the 2 and 4");
    }
}

// ----------------------------------------------------------------------- pop

#[test]
fn pop_stays_on_the_grid() {
    // "Straight, never swung: the pocket is the grid." Both halves are
    // checkable — the model must say straight, and the notes must land there.
    let pop = model("pop-2000s");
    let swing = pop
        .session
        .as_ref()
        .and_then(|s| s.swing.as_ref())
        .map(|s| s.amount)
        .unwrap();
    assert_eq!(swing, 0.5, "pop should be straight");

    for (seed, note) in sweep(&pop, Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        assert_eq!(
            note.start_tick % grid::SIXTEENTH,
            0,
            "seed {seed}: pop drifted off the grid"
        );
    }
}

// ------------------------------------------------------------------- the set

#[test]
fn no_two_genres_produce_the_same_drums() {
    // The point of a roster: if two archetypes generate the same notes from the
    // same seed, one of them is not earning its place.
    let context = ctx(4);
    let models = shipped();
    let ids: Vec<&String> = models.keys().filter(|id| !id.starts_with('_')).collect();

    for (i, left) in ids.iter().enumerate() {
        for right in ids.iter().skip(i + 1) {
            let a = generate(&models[*left], &context, 42);
            let b = generate(&models[*right], &context, 42);
            assert_ne!(a, b, "{left} and {right} generate identical drums");
        }
    }
}

#[test]
fn every_genre_in_the_roster_has_an_invariant_test() {
    // The guard on this file: a genre authored without a signature test would
    // be a model nothing checks. Update both when adding one.
    const COVERED: &[&str] = &[
        "trap",
        "uk-drill",
        "rage",
        "chicago-drill",
        "ny-drill",
        "plugg",
        "pluggnb",
        "jerk",
        "phonk",
        "west-coast-club",
        "boom-bap",
        "rnb-2000s",
        "liquid-dnb",
        "country-train",
        "pop-2000s",
    ];

    let shipped_ids: Vec<String> = shipped()
        .keys()
        .filter(|id| !id.starts_with('_'))
        .cloned()
        .collect();

    for id in &shipped_ids {
        assert!(
            COVERED.contains(&id.as_str()),
            "`{id}` ships with no invariant test — add one here and list it"
        );
    }
    for id in COVERED {
        assert!(
            shipped_ids.iter().any(|s| s == id),
            "`{id}` is listed as covered but no longer ships"
        );
    }
}
