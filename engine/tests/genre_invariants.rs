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
use std::sync::OnceLock;

use engine::context::SessionContext;
use engine::generators::drums::{generate, PERC_LANES};
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

/// The whole resolved roster, read from disk **once per test binary**.
///
/// ⛔ **This used to re-read and re-resolve the entire dataset on every call**,
/// and `model()` below calls it for a single id — so 62 of the 63 models were
/// parsed, inheritance-merged and thrown away each time. Closures that call
/// `model(id)` inside a 100-seed loop turned three lines into 600 full dataset
/// loads. Memoised, the file's ~390 effective loads collapse to one.
fn shipped() -> &'static BTreeMap<String, StyleModel> {
    static MODELS: OnceLock<BTreeMap<String, StyleModel>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
        let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
        assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
        models
    })
}

fn model(id: &str) -> StyleModel {
    shipped()
        .get(id)
        .cloned()
        .unwrap_or_else(|| panic!("`{id}` must ship"))
}

/// The share of this model's chord events voiced as a seventh or richer.
///
/// ⚠ **One definition of "extended harmony" for the whole suite.** `>= 4` tones
/// is the bar, and it was written out five times before this — so a change to
/// what counts as extended had to be found in five places.
fn extended_share(id: &str) -> f64 {
    let m = model(id);
    let context = ctx(4);
    let mut total = 0usize;
    let mut extended = 0usize;
    for seed in 0..SEEDS {
        for event in engine::generators::chords::generate(&m, &context, seed).events {
            total += 1;
            if event.tones.len() >= 4 {
                extended += 1;
            }
        }
    }
    assert!(total > 0, "{id} generated no chords at all");
    extended as f64 / total as f64
}

/// The share of this model's 808 notes that slide.
fn slide_share(id: &str) -> f64 {
    let notes = sweep(&model(id), Lane::Sub, 4);
    assert!(!notes.is_empty(), "{id} generated no 808 at all");
    let slid = notes
        .iter()
        .filter(|(_, n)| n.slide_to_pitch.is_some())
        .count();
    slid as f64 / notes.len() as f64
}

/// The swing amount a model states, which every genre in this file must.
fn swing_of(id: &str) -> f64 {
    model(id)
        .session
        .as_ref()
        .and_then(|s| s.swing.as_ref())
        .map(|s| s.amount)
        .unwrap_or_else(|| panic!("`{id}` must state its swing"))
}

/// The swing grid and amount together, for the one claim that is about *which*
/// subdivision swings — a shuffle swings the 8ths, not the 16ths.
fn swing_grid_of(id: &str) -> (String, f64) {
    let m = model(id);
    let swing = m
        .session
        .as_ref()
        .and_then(|s| s.swing.as_ref())
        .unwrap_or_else(|| panic!("`{id}` must state its swing"));
    (swing.grid.to_string(), swing.amount)
}

/// The quantize strength a model states.
fn quantize_of(id: &str) -> f64 {
    model(id)
        .session
        .as_ref()
        .and_then(|s| s.humanize.as_ref())
        .and_then(|h| h.quantize_strength)
        .unwrap_or_else(|| panic!("`{id}` must state a quantize strength"))
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
                grid::is_offbeat_eighth(
                    (n.start_tick % context.ticks_per_bar()) / grid::SIXTEENTH,
                    &context,
                )
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
    let chicago = sweep(&model("chicago-drill"), Lane::Sub, 4);
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
    for (seed, note) in sweep(&model("plugg"), Lane::Sub, 4) {
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
        let bass = notes(&lanes, Lane::Sub).len();
        assert_eq!(bass, kicks, "seed {seed}: the 808 left a kick undoubled");
    }

    let slides: Vec<i16> = sweep(&phonk, Lane::Sub, 4)
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
    assert!(sweep(&model("boom-bap"), Lane::Sub, 4).is_empty());
    assert!(sweep(&model("rnb-2000s"), Lane::Sub, 4).is_empty());
    assert!(sweep(&model("country-train"), Lane::Sub, 4).is_empty());
    // ...while the genres that do want one still have it.
    assert!(!sweep(&model("trap"), Lane::Sub, 4).is_empty());
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
            grid::is_sixteenth_offbeat(*index, &ctx(4)),
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

// ------------------------------------------------------------- trap family

#[test]
fn dark_trap_leaves_the_eighth_before_its_snare_clear() {
    // Research ch. 1 §1: trap avoids a kick immediately before the beat-3
    // snare, "leaving an 8th gap" — it is what lets the half-time backbeat land
    // in silence. Bouncy trap does not author it, because its whole point is
    // that the kick keeps moving, so the two make the claim checkable.
    let context = ctx(4);
    let bar = context.ticks_per_bar();
    let snare = beat(&context) * 2; // beat 3, half-time
    let gap = snare - beat(&context) / 2;

    let pre_snare_share = |id: &str| {
        let kicks = sweep(&model(id), Lane::Kick, 4);
        let inside = kicks
            .iter()
            .filter(|(_, n)| (gap..snare).contains(&(n.start_tick % bar)))
            .count();
        inside as f64 / kicks.len() as f64
    };

    let dark = pre_snare_share("dark-trap");
    let bouncy = pre_snare_share("bouncy-trap");
    assert!(
        dark < bouncy,
        "dark trap ({dark:.3}) should crowd the pre-snare 8th less than bouncy trap ({bouncy:.3})"
    );
}

#[test]
fn bouncy_trap_keeps_its_kick_moving_more_than_the_dark_lane() {
    // Research ch. 2 §1: the Pi'erre/Zaytoven lane is the bright exception, and
    // the bounce is in the kick — `syncopation` 0.55 against dark trap's 0.35,
    // with the tresillo lean to match.
    let context = ctx(4);
    let offbeat_share = |id: &str| {
        let kicks = sweep(&model(id), Lane::Kick, 4);
        let offbeat = kicks
            .iter()
            .filter(|(_, n)| {
                grid::is_offbeat_eighth(
                    (n.start_tick % context.ticks_per_bar()) / grid::SIXTEENTH,
                    &context,
                )
            })
            .count();
        offbeat as f64 / kicks.len() as f64
    };

    let bouncy = offbeat_share("bouncy-trap");
    let dark = offbeat_share("dark-trap");
    assert!(
        bouncy > dark,
        "bouncy trap ({bouncy:.3}) should lean offbeat more than dark trap ({dark:.3})"
    );
}

#[test]
fn trap_soul_carries_r_and_b_harmony_rather_than_trap_triads() {
    // ⛔ **The genre's actual marker, and the one that nearly got lost.** Trap
    // soul is trap drums under R&B harmony (research ch. 1 §9, ch. 2 §6) — the
    // m7/m9/maj9 stack is the whole point. The families were first authored as
    // `i7`/`iv7`/`IIImaj7`, which the numeral parser cannot read, so every one
    // of those chords was being **dropped silently**. Rewriting them as plain
    // degrees is only correct if `extensions` really does the colouring, and
    // nothing asserted that until this test.
    let soul = extended_share("trap-soul");
    let trap = extended_share("trap");
    assert!(
        soul > 0.75,
        "trap soul voiced only {soul:.2} of its chords as sevenths or richer — \
         that is a triad genre wearing the name"
    );
    assert!(
        soul > trap,
        "trap soul ({soul:.2}) must be more extended than trap ({trap:.2})"
    );
}

#[test]
fn trap_soul_thins_the_hats_out_to_leave_room_for_a_vocal() {
    // Research ch. 1 §9, trap-soul variant: trap grammar under R&B harmony,
    // with the hats pulled back — `continuous: false` at 0.3 against trap's
    // 0.55 carpet. A sung topline is what the space is for.
    let soul = sweep(&model("trap-soul"), Lane::ClosedHat, 4).len();
    let trap = sweep(&model("trap"), Lane::ClosedHat, 4).len();
    assert!(
        soul < trap,
        "trap soul played {soul} closed hats against trap's {trap} — it must be the thinner of the two"
    );
    assert!(soul > 0, "trap soul still has a hat part");
}

#[test]
fn cloud_rap_is_the_sparsest_kick_in_the_family() {
    // The lane is defined by what it removes (research ch. 4 taxonomy, cloud
    // end): no roll carpet, no distortion, and a kick that gets out of the way
    // of the reverb. `densityPerBar` [1,3] against trap's [2,5].
    let cloud = sweep(&model("cloud-rap"), Lane::Kick, 4).len();
    let trap = sweep(&model("trap"), Lane::Kick, 4).len();
    assert!(
        cloud < trap,
        "cloud rap played {cloud} kicks against trap's {trap} — it must be the sparser of the two"
    );
    assert!(cloud > 0, "cloud rap still keeps a pulse");
}

#[test]
fn emo_rap_glides_its_808_more_than_the_dark_lane_does() {
    // Research ch. 2 §5: the 808 "shadows the guitar-loop roots 1:1" and
    // **glides for emotion** — which is the opposite use of the same device from
    // dark trap, where a slide is an accent rather than the feeling.

    let emo = slide_share("emo-rap");
    let dark = slide_share("dark-trap");
    assert!(
        emo > dark,
        "emo rap slid {emo:.3} of its 808 notes against dark trap's {dark:.3}"
    );
}

// ---------------------------------------------------- the underground scenes

#[test]
fn dark_plugg_bends_its_808_far_more_than_plugg_does() {
    // Research ch. 4 taxonomy, dark plugg: "808s: long bending slides, punchy
    // kick pairing, restrained level (melody breathes)" — set against plugg's
    // own entry, where the 808 is the short "Light 808" and, in ch. 1 §4,
    // "slides rare". The two share a chassis, a tempo range and a clap on 3, so
    // the bend is what separates them: an absolute threshold on dark plugg
    // alone would pass just as happily if plugg slid as much.

    let dark = slide_share("dark-plugg");
    let plugg = slide_share("plugg");

    // `slidePositions` are bar-granular, so `phrase_end` reaches about half the
    // notes — an authored 0.68 lands near a third of the lane, not two thirds.
    assert!(
        dark > 0.2,
        "dark plugg slid only {dark:.3} of its 808 notes — the long bend is the marker"
    );
    assert!(
        dark > plugg * 3.0,
        "dark plugg ({dark:.3}) should bend far more than plugg ({plugg:.3})"
    );
}

#[test]
fn detroit_bounce_holds_its_808_still_over_a_full_time_backbeat() {
    // Research ch. 4, scene taxonomy Michigan/Detroit: "punchy static 808s ...
    // no half-time drop". Both halves are the genre and both are asserted
    // against the model that does the opposite — UK drill, whose 808 is a
    // sliding counter-riff under a half-time snare. An absolute threshold on
    // Detroit alone could pass for the wrong reason (an 808 that never
    // generates slides on none of nothing), so the comparison carries the claim
    // and the emptiness check guards the denominator.

    let detroit = slide_share("detroit-bounce");
    let drill = slide_share("uk-drill");
    assert!(
        detroit < 0.1,
        "detroit slid {detroit:.3} of its 808 notes — a static 808 is the marker"
    );
    assert!(
        detroit < drill,
        "detroit ({detroit:.3}) must hold its pitch far more than uk drill ({drill:.3})"
    );

    // ...and the snare is full-time on 2 and 4, not the half-time 3 every other
    // genre at this tempo takes. `offGridMs: 0`, so the positions are exact.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    let mut hits = 0;
    for (seed, note) in sweep(&model("detroit-bounce"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % bar_ticks;
        assert!(
            backbeat.contains(&within),
            "seed {seed}: detroit's snare at {within} is not the 2 or the 4"
        );
        hits += 1;
    }
    assert!(hits > 0, "detroit bounce produced no backbeat at all");
}

#[test]
fn jersey_club_runs_the_densest_kick_in_the_dataset() {
    // Research ch. 4 SCENE TAXONOMY, Jersey club-rap: "5-kick syncopated
    // pattern" — 1, the a of 1, the and of 2, then 3 and the a of 3. Five to a
    // bar is the genre, and it is only a claim as a comparison: an absolute
    // figure would pass on any model that happened to be busy. Phonk is the
    // roster's benchmark at "4–6/bar incl. offbeats" (research ch. 1 §7), which
    // averages five, so jersey club has to sit above it.
    let jersey_kicks = sweep(&model("jersey-club"), Lane::Kick, 4);
    let phonk_kicks = sweep(&model("phonk"), Lane::Kick, 4);
    let bars = SEEDS as f64 * 4.0;
    let jersey = jersey_kicks.len() as f64 / bars;
    let phonk = phonk_kicks.len() as f64 / bars;

    assert!(
        (5.0..=5.6).contains(&jersey),
        "the five-kick bar is the whole genre, got {jersey:.2} kicks a bar"
    );
    assert!(
        jersey > phonk,
        "jersey club ({jersey:.2}) must out-kick phonk ({phonk:.2}), the densest of the rest"
    );

    // ...and the lurch is in where they land, not only how many. The "a" of 1
    // is an anchor, so no bar of any seed is without it — that off-beat 16th is
    // what a four-on-the-floor club kick never plays.
    let bar = ctx(4).ticks_per_bar();
    let on_the_a = jersey_kicks
        .iter()
        .filter(|(_, n)| n.start_tick % bar == grid::SIXTEENTH * 3)
        .count();
    assert_eq!(
        on_the_a,
        (SEEDS * 4) as usize,
        "the a of 1 is half of what makes the pattern Jersey"
    );
}

#[test]
fn atl_swag_rap_loops_on_two_chords_where_trap_soul_keeps_moving() {
    // Research ch. 4 scene taxonomy, ATL swag rap new wave (ØWay/YSL-orbit):
    // "melody = vocal acrobatics over 2-chord loops". The loop is the marker, so
    // the claim is about how far a progression actually travels — measured
    // against trap soul, whose four-chord R&B cycle (ch. 2 §6) is the opposite
    // choice at the same tempo. Eight bars rather than four, because a two-bar
    // harmonic rhythm truncates *everyone's* progression over four and would
    // flatter the claim.
    let short_loop_share = |id: &str| {
        let model = model(id);
        let context = ctx(8);
        let mut short = 0usize;
        for seed in 0..SEEDS {
            let mut roots: Vec<u8> = engine::generators::chords::generate(&model, &context, seed)
                .events
                .iter()
                .map(|event| event.root)
                .collect();
            assert!(!roots.is_empty(), "{id} wrote no chords on seed {seed}");
            roots.sort_unstable();
            roots.dedup();
            if roots.len() <= 2 {
                short += 1;
            }
        }
        short as f64 / SEEDS as f64
    };

    let swag = short_loop_share("atl-swag-rap");
    let soul = short_loop_share("trap-soul");

    assert!(
        swag > 0.6,
        "atl swag rap held to two chords in only {swag:.2} of its loops — the vamp is the genre"
    );
    assert!(
        swag > soul * 2.0,
        "the two-chord loop is what separates it from a harmonically busy genre: \
         {swag:.2} against trap soul's {soul:.2}"
    );
}

#[test]
fn uk_underground_keeps_jerks_displaced_backbeat_but_nudges_where_jerk_staggers() {
    // Research ch. 4 SCENE TAXONOMY, "UK underground (jerk-drill hybrid)":
    // "jerk snare displacement + UK drill sub-bass discipline (less
    // distortion)". A hybrid is only itself if BOTH halves hold, so both are
    // asserted — and both against `jerk` by name, because an absolute
    // threshold on either one alone passes for the wrong reason.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    // How far each backbeat sits from where the grid would put it. This is a
    // grammar displacement (`offGridMs`), not humanizer jitter, so it is in
    // the notes the generator writes.
    let displacements = |id: &str| -> Vec<u32> {
        let mut out = Vec::new();
        for (_, note) in sweep(&model(id), Lane::Snare, 4) {
            if !is_backbeat(&note) {
                continue;
            }
            let within = note.start_tick % bar_ticks;
            let nearest = backbeat
                .iter()
                .map(|b| within.abs_diff(*b))
                .min()
                .unwrap_or(u32::MAX);
            // Further than half a 16th away is a fill hit, not a nudged
            // backbeat, and averaging the two together would measure neither.
            if nearest <= grid::SIXTEENTH / 2 {
                out.push(nearest);
            }
        }
        out
    };

    // Half one, the jerk half: the backbeat is displaced, essentially always.
    let ours = displacements("uk-underground");
    assert!(
        ours.len() > SEEDS as usize * 4,
        "uk-underground produced only {} backbeats over {SEEDS} seeds",
        ours.len()
    );
    let off_grid = ours.iter().filter(|d| **d > 0).count() as f64 / ours.len() as f64;
    assert!(
        off_grid > 0.9,
        "the displaced backbeat is what this inherits from jerk, got {off_grid:.2} off-grid"
    );

    // ...but a drill nudge rather than a jerk stagger. This is the half that
    // stops it being jerk with a different name on it.
    let theirs = displacements("jerk");
    assert!(
        !theirs.is_empty(),
        "jerk produced no backbeat to compare against"
    );
    let mean = |hits: &[u32]| hits.iter().map(|d| f64::from(*d)).sum::<f64>() / hits.len() as f64;
    let (nudge, stagger) = (mean(&ours), mean(&theirs));
    assert!(
        nudge < stagger,
        "uk-underground displaced by {nudge:.1} ticks against jerk's {stagger:.1} — \
         the drill half is the tighter of the two by definition"
    );

    // Half two: and the hand behind it is tighter. `jerk` is asserted
    // elsewhere to be the loosest-quantized model in the dataset; this says
    // which way the hybrid leans off it, and it is the musical point.
    let (tight, loose) = (quantize_of("uk-underground"), quantize_of("jerk"));
    assert!(
        tight > loose,
        "uk-underground quantizes at {tight} against jerk's {loose} — drill discipline \
         is the other half of the hybrid"
    );
}

#[test]
fn edm_rage_lands_on_all_four_quarters_where_rage_leaves_them_empty() {
    // Research ch. 4 scene taxonomy, "EDM-rage / electroclash-rap (2hollis
    // lane)": "four-on-floor ↔ trap grid alternation", against ch. 1 §3's rage,
    // whose kick is the half-time trap skeleton at 2–4 hits a bar. The
    // four-on-the-floor is the whole marker, so it is asserted as the
    // comparison it actually is — rage is the genre this one is built out of,
    // and the club kick is what makes it a different record.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let quarter = beat(&context);

    let four_on_the_floor = |id: &str| {
        let m = model(id);
        let mut per_bar: BTreeMap<(u64, u32), Vec<u32>> = BTreeMap::new();
        for (seed, note) in sweep(&m, Lane::Kick, 4) {
            per_bar
                .entry((seed, note.start_tick / bar_ticks))
                .or_default()
                .push(note.start_tick % bar_ticks);
        }
        let full = per_bar
            .values()
            .filter(|ticks| (0..4).all(|q| ticks.contains(&(q * quarter))))
            .count();
        full as f64 / (SEEDS as f64 * 4.0)
    };

    let edm = four_on_the_floor("edm-rage");
    let rage = four_on_the_floor("rage");

    assert!(
        edm > 0.9,
        "edm-rage should put a kick on every quarter of nearly every bar, got {edm:.2}"
    );
    assert!(
        edm > rage * 4.0,
        "edm-rage ({edm:.2}) should be four-on-the-floor far more often than rage ({rage:.2})"
    );
}

#[test]
fn digicore_is_the_rosters_major_key_lane() {
    // Research ch. 4 taxonomy, "digicore / hyperpop": "Keys: major/relative
    // minor — the roster's main source of major-key material", and ch. 3
    // glaive: "major/relative-minor pop harmony (distinct from the rage
    // cohort's static minor)". A model's keys and scales only become notes
    // through `SessionContext::from_model`, so the default C-minor session the
    // rest of this file uses would hide the entire claim — this test reads the
    // session the model itself asks for. The measure is the third the harmony
    // is stacked on: a major third above the key root against a minor one.
    let major_third_share = |id: &str| {
        let m = model(id);
        let (mut major, mut minor) = (0usize, 0usize);
        for seed in 0..SEEDS {
            let context =
                SessionContext::from_model(&m, &engine::context::SessionOverrides::default(), seed);
            for event in engine::generators::chords::generate(&m, &context, seed).events {
                for tone in &event.tones {
                    match (i32::from(*tone) - i32::from(context.key_root)).rem_euclid(12) {
                        4 => major += 1,
                        3 => minor += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(major + minor > 0, "{id} generated no thirds at all");
        major as f64 / (major + minor) as f64
    };

    let digicore = major_third_share("digicore");
    let dark = major_third_share("dark-trap");

    assert!(
        digicore > 0.6,
        "digicore stacked only {digicore:.2} of its thirds major — this lane is the \
         roster's major-key source, not another minor genre with a bright name"
    );
    assert!(
        dark < 0.25,
        "dark trap is the minor-locked control here, and it came out {dark:.2}"
    );
    assert!(
        digicore > dark * 3.0,
        "digicore ({digicore:.2}) must be unmistakably more major than dark trap ({dark:.2})"
    );
}

// -------------------------------------------------- drum & bass and uk dance

#[test]
fn jump_up_takes_the_straight_second_kick_where_liquid_steps() {
    // Research ch. 1 §10: "second-kick position sets flavor: straight (beat 3),
    // classic step (and-of-3), neuro (2a — last 16th before 3)". Jump-up is the
    // straight one and liquid is the classic step, so the claim is not a
    // threshold on either model on its own — it is that the two cross over on
    // exactly that 16th while sharing everything else about the two-step.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let beat_three = beat(&context) * 2;
    let and_of_three = beat_three + grid::SIXTEENTH * 2;

    // Measured per bar against the 1, which both genres anchor once a bar — so
    // the downbeat count is the number of bars and the share is "how often does
    // a bar play this position".
    let share = |id: &str, tick: u32| {
        let hits = sweep(&model(id), Lane::Kick, 4);
        let downbeats = hits
            .iter()
            .filter(|(_, n)| n.start_tick % bar_ticks == 0)
            .count();
        assert!(downbeats > 0, "{id}: the 1 is the anchor");
        let at = hits
            .iter()
            .filter(|(_, n)| n.start_tick % bar_ticks == tick)
            .count();
        at as f64 / downbeats as f64
    };

    let jump_straight = share("jump-up-dnb", beat_three);
    let jump_stepped = share("jump-up-dnb", and_of_three);
    let liquid_straight = share("liquid-dnb", beat_three);
    let liquid_stepped = share("liquid-dnb", and_of_three);

    assert!(
        jump_straight > 0.95,
        "jump-up should land beat 3 in essentially every bar, got {jump_straight:.2}"
    );
    assert!(
        jump_straight > jump_stepped * 2.0,
        "jump-up leaned on the and-of-3 ({jump_stepped:.2}) against its own beat 3 ({jump_straight:.2})"
    );
    assert!(
        liquid_stepped > 0.95 && liquid_stepped > liquid_straight * 2.0,
        "liquid must still be the stepped one: {liquid_stepped:.2} against {liquid_straight:.2}"
    );
    assert!(
        jump_straight > liquid_straight && liquid_stepped > jump_stepped,
        "the two flavours have to cross over on this 16th, not merely differ in kick density"
    );
}

#[test]
fn neurofunk_displaces_its_second_kick_to_the_last_sixteenth_before_three() {
    // Research ch. 1 §10: the second-kick position is what splits the drum &
    // bass family — straight is beat 3, the classic step is the and-of-3, and
    // "neuro: kick displaced to 2a, dense sound-design percs". Liquid takes the
    // step, so the two models together are what make the claim checkable: an
    // absolute threshold on neurofunk alone could pass for the wrong reason.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let two_a = beat(&context) + grid::SIXTEENTH * 3;

    let on_two_a = |id: &str| {
        let kicks = sweep(&model(id), Lane::Kick, 4);
        assert!(!kicks.is_empty(), "{id} generated no kick at all");
        let hits = kicks
            .iter()
            .filter(|(_, n)| n.start_tick % bar_ticks == two_a)
            .count();
        hits as f64 / kicks.len() as f64
    };

    let neuro = on_two_a("neurofunk");
    let liquid = on_two_a("liquid-dnb");

    // The anchor is unconditional and the bar runs 2–4 kicks, so roughly a
    // third of every neuro kick lands here.
    assert!(
        neuro > 0.2,
        "2a should carry neurofunk's pattern, got {neuro:.3} of its kicks"
    );
    assert!(
        neuro > liquid * 4.0,
        "neurofunk ({neuro:.3}) must sit on 2a far more often than liquid ({liquid:.3})"
    );
}

#[test]
fn jungle_chops_its_snare_where_liquid_locks_it() {
    // Research ch. 1 §10, the jungle bullet: "sliced/resequenced multi-break
    // polyrhythms" — set against "snares LOCKED 2 & 4" two bullets above it.
    // Both strains still land the 2 and the 4; what separates them is
    // everything between, so the claim is about the ghost hits and how far
    // across the 16th grid they are spread. It only means anything as a
    // comparison against the locked strain, which is why liquid-dnb is here.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();

    let ghosts = |id: &str| {
        let mut slots: Vec<u32> = sweep(&model(id), Lane::Snare, 4)
            .iter()
            .filter(|(_, n)| n.articulation == Some(Articulation::Ghost))
            .map(|(_, n)| (n.start_tick % bar_ticks) / grid::SIXTEENTH)
            .collect();
        let total = slots.len();
        slots.sort_unstable();
        slots.dedup();
        (total, slots.len())
    };

    let (jungle_hits, jungle_slots) = ghosts("jungle");
    let (liquid_hits, liquid_slots) = ghosts("liquid-dnb");

    assert!(
        liquid_hits > 0,
        "liquid-dnb is the locked strain, not a silent one — nothing to compare against"
    );
    assert!(
        jungle_slots >= liquid_slots * 4,
        "jungle spread its ghosts over {jungle_slots} sixteenths against liquid's \
         {liquid_slots} — that is a two-step wearing the name, not a chopped break"
    );
    assert!(
        jungle_hits > liquid_hits * 3,
        "jungle played {jungle_hits} ghost snares against liquid's {liquid_hits}"
    );
}

#[test]
fn pop_dnb_swings_and_breathes_where_liquid_is_straight_and_tight() {
    // Research ch. 1 §10, the pop-DnB bullets, and ch. 3 PinkPantheress: the
    // same two-step skeleton as liquid, but "UKG-adjacent shuffle", drums mixed
    // "quieter/softer", "velocity-varied ghosts, off-grid hats". Every one of
    // those is a *feel* against liquid rather than a threshold of its own, so
    // the claim is the comparison — pop-DnB shuffles further, quantizes looser
    // and jitters its hats harder than the genre it extends — and the last
    // assert is the thing none of that is allowed to move: the locked 2 & 4
    // that keeps it drum & bass at all.
    let pop = model("pop-dnb");
    let liquid = model("liquid-dnb");

    let swing = |m: &StyleModel| {
        m.session
            .as_ref()
            .and_then(|s| s.swing.as_ref())
            .map(|s| s.amount)
            .expect("both models state their swing")
    };
    let quantize = |m: &StyleModel| {
        m.session
            .as_ref()
            .and_then(|s| s.humanize.as_ref())
            .and_then(|h| h.quantize_strength)
            .expect("both models state a quantize strength")
    };
    let hat_jitter = |m: &StyleModel| {
        m.session
            .as_ref()
            .and_then(|s| s.humanize.as_ref())
            .and_then(|h| h.timing_jitter_ms.get("closedHat").copied())
            .expect("both models jitter their closed hats")
    };

    let (pop_swing, liquid_swing) = (swing(&pop), swing(&liquid));
    assert!(
        pop_swing > liquid_swing,
        "pop-dnb ({pop_swing}) must shuffle where liquid ({liquid_swing}) is near-straight"
    );
    assert!(
        (0.55..=0.62).contains(&pop_swing),
        "the UK garage shuffle sits either side of 58%, got {pop_swing}"
    );

    let (pop_quantize, liquid_quantize) = (quantize(&pop), quantize(&liquid));
    assert!(
        pop_quantize < liquid_quantize,
        "pop-dnb ({pop_quantize}) must sit looser than liquid ({liquid_quantize})"
    );

    let (pop_hats, liquid_hats) = (hat_jitter(&pop), hat_jitter(&liquid));
    assert!(
        pop_hats > liquid_hats,
        "off-grid hats are the marker: {pop_hats} ms against liquid's {liquid_hats} ms"
    );

    // ...and the backbeat still never moves. Statistical over every seed,
    // because a two-step that drifts on one of them is not locked.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let backbeat = [beat(&context), beat(&context) * 3];

    let mut hits = 0usize;
    for (seed, note) in sweep(&pop, Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        hits += 1;
        let within = note.start_tick % bar_ticks;
        assert!(
            backbeat.contains(&within),
            "seed {seed}: pop-dnb's locked backbeat moved to {within}"
        );
    }
    assert!(
        hits >= SEEDS as usize * 4,
        "the 2 and the 4 have to play every bar, got {hits} across the sweep"
    );
}

// ------------------------------------------------------------------ uk garage

#[test]
fn uk_garage_swings_its_sixteenths_harder_than_the_straight_genres() {
    // Research ch. 1 §10, the pop-DnB line ("UKG-adjacent shuffle"), and ch. 3
    // PinkPantheress ("shuffled swung hats"): the shuffle *is* the genre, and it
    // sits on the 16th grid. 0.64 is near the top of the legal 0.50–0.75 band —
    // a real shuffle, not the 0.52-ish nudge the trap family takes. Compared
    // against named models rather than asserted as a bare number, because a
    // threshold on one model can pass for the wrong reason.

    let garage = swing_of("uk-garage");
    assert!(
        (0.62..=0.66).contains(&garage),
        "uk garage should shuffle at the top of the band, got {garage}"
    );

    // The models the claim is made against. Two of the three are pinned by their
    // own tests in this file — pop at exactly straight, boom bap at the 58% MPC
    // setting — so this cannot quietly become true by the other end moving.
    let pop = swing_of("pop-2000s");
    let trap = swing_of("trap");
    let mpc = swing_of("boom-bap");
    assert!(
        garage > pop,
        "uk garage ({garage}) must out-swing pop ({pop})"
    );
    assert!(
        garage > trap,
        "uk garage ({garage}) must out-swing trap ({trap})"
    );
    assert!(
        garage > mpc,
        "uk garage ({garage}) should shuffle past the classic MPC setting ({mpc}), not up to it"
    );

    // ...and not merely past three hand-picked models. It has to sit well clear
    // of what the roster as a whole does, which is what "the shuffled one" means.
    let authored: Vec<f64> = shipped()
        .iter()
        .filter(|(id, _)| !id.starts_with('_'))
        .filter_map(|(_, m)| {
            m.session
                .as_ref()
                .and_then(|s| s.swing.as_ref())
                .map(|s| s.amount)
        })
        .collect();
    assert!(!authored.is_empty(), "no model states a swing at all");
    let mean = authored.iter().sum::<f64>() / authored.len() as f64;
    assert!(
        garage - mean > 0.05,
        "uk garage swings at {garage} against a roster mean of {mean:.3} — that is a nudge, not a shuffle"
    );
}

#[test]
fn house_kicks_every_quarter_and_opens_its_hats_between_them() {
    // Research ch. 2 §11 and ch. 1 §12: house is four-on-the-floor — "kick all
    // quarters + snare 2&4" — and "dance-pop = offbeat open hats + straight 8th
    // closed". The kick half is absolute rather than statistical: `densityPerBar`
    // [4, 4] over four anchored beats, so there is no seed on which a bar plays
    // three. The open hat is the comparative half, because an absolute count
    // could pass on a model that merely has hats — trap opens its own as a
    // garnish on the 2& and 4&, while house puts one between every kick.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let quarter = beat(&context);

    let kicks = sweep(&model("house"), Lane::Kick, 4);
    assert_eq!(
        kicks.len() as u64,
        SEEDS * 4 * 4,
        "house owes every bar of every seed exactly four kicks"
    );

    let mut per_beat = [0usize; 4];
    for (seed, note) in &kicks {
        let within = note.start_tick % bar_ticks;
        assert_eq!(
            within % quarter,
            0,
            "seed {seed}: a house kick left the quarters at {within}"
        );
        per_beat[(within / quarter) as usize] += 1;
    }
    for (index, hits) in per_beat.iter().enumerate() {
        assert_eq!(
            *hits as u64,
            SEEDS * 4,
            "beat {} is missing from some bars — the floor has a hole in it",
            index + 1
        );
    }

    let open = |id: &str| sweep(&model(id), Lane::OpenHat, 4).len();
    let house = open("house");
    let trap = open("trap");
    assert!(
        house > trap * 3,
        "house opened its hats {house} times against trap's {trap} — the offbeat \
         open hat is the other half of the signature, not a garnish"
    );
}

// -------------------------------------- pop, country and the live-band lanes

#[test]
fn dance_pop_puts_a_kick_on_all_four_where_pop_2000s_syncopates() {
    // Research ch. 1 §12: the 2008–2012 dance-pop era is the four-on-the-floor
    // one — "energy = kick all quarters + snare 2&4" — where the Max Martin era
    // it grows out of keeps its kick "sparse/syncopated (1 + and-of-2)". The
    // claim is about the difference between the two, so it is checked against
    // the parent rather than against a threshold this model could clear for
    // reasons of its own.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let quarters: Vec<u32> = (0..4).map(|i| beat(&context) * i).collect();

    // The floor itself: every bar of every seed plays all four of them.
    let dance = model("dance-pop");
    for seed in 0..SEEDS {
        let kicks = notes(&generate(&dance, &context, seed), Lane::Kick);
        for bar in 0..4 {
            let within: Vec<u32> = kicks
                .iter()
                .map(|n| n.start_tick)
                .filter(|tick| *tick / bar_ticks == bar)
                .map(|tick| tick % bar_ticks)
                .collect();
            for quarter in &quarters {
                assert!(
                    within.contains(quarter),
                    "seed {seed}, bar {bar}: the floor lost its kick on {quarter}"
                );
            }
        }
    }

    // ...and how much further onto the beat that puts it than its parent.
    let on_beat_share = |id: &str| {
        let kicks = sweep(&model(id), Lane::Kick, 4);
        let on_beat = kicks
            .iter()
            .filter(|(_, n)| {
                grid::is_downbeat((n.start_tick % bar_ticks) / grid::SIXTEENTH, &context)
            })
            .count();
        on_beat as f64 / kicks.len() as f64
    };

    let child = on_beat_share("dance-pop");
    let parent = on_beat_share("pop-2000s");
    assert!(
        child > parent + 0.15,
        "dance pop ({child:.2}) should sit far further on the beat than pop-2000s ({parent:.2})"
    );
}

#[test]
fn pop_2020s_takes_the_half_time_backbeat_its_parent_never_does() {
    // Research ch. 1 §12: the late-2010s+ record puts the snare/clap on "3 in
    // trap-pop", where the Max Martin era it grew out of keeps the full-time
    // 2 & 4. The claim is a *difference* between two models, so both halves
    // are asserted — an absolute bound on one of them could pass for the
    // wrong reason, and the two files are what make it checkable.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let three = beat(&context) * 2;
    let full_time = [beat(&context), beat(&context) * 3];

    let modern = sweep(&model("pop-2020s"), Lane::Snare, 4);
    assert!(!modern.is_empty(), "pop-2020s produced no snare at all");
    for (seed, note) in &modern {
        if !is_backbeat(note) {
            continue;
        }
        assert_eq!(
            note.start_tick % bar_ticks,
            three,
            "seed {seed}: a 2020s pop backbeat left beat 3"
        );
    }

    for (seed, note) in sweep(&model("pop-2000s"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        assert!(
            full_time.contains(&(note.start_tick % bar_ticks)),
            "seed {seed}: the 2000s parent should still be on the 2 and the 4"
        );
    }
}

#[test]
fn country_pop_swaps_the_train_stream_for_a_backbone() {
    // Research ch. 1 §11, the country-pop layering bullet: "kick-snare 2&4
    // backbone; claps/stomps stacked on snare" — set against the train beat's
    // "continuous 1/16 snare stream" in the same section. Same kit, same keys,
    // the same producers, so the snare lane is the thing that tells the two
    // apart and the claim only means anything measured against `country-train`
    // itself.
    let per_bar = |id: &str| sweep(&model(id), Lane::Snare, 4).len() as f64 / (SEEDS as f64 * 4.0);
    let pop = per_bar("country-pop");
    let train = per_bar("country-train");

    assert!(
        pop > 1.0,
        "country-pop still plays a snare, got {pop:.2} a bar"
    );
    assert!(
        pop * 3.0 < train,
        "country-pop hit {pop:.2} snares a bar against the train beat's {train:.2} — \
         that is a stream wearing a backbeat's name"
    );

    // ...and what is left is the backbone rather than a thinned-out stream:
    // every hit that is not a ghost or a fill is the 2 or the 4.
    let context = ctx(4);
    let backbeat = [beat(&context), beat(&context) * 3];
    for (seed, note) in sweep(&model("country-pop"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        let within = note.start_tick % context.ticks_per_bar();
        assert!(
            backbeat.contains(&within),
            "seed {seed}: a country-pop snare landed at {within}, off the 2 and the 4"
        );
    }
}

#[test]
fn the_country_shuffle_swings_where_the_train_beat_marches() {
    // Research ch. 1 §11: the shuffle is "12/8 or swung-8th ride, backbeat
    // 2&4", and the same section's swing bullet puts it at 58–66%. There is no
    // meter field, so the 12/8 feel *is* the swing amount — the marker is a
    // number. It has to beat the train beat's, because the two share a kit, a
    // tempo band and a backbeat, and the swing is what a listener separates
    // them by.

    let (grid, shuffle) = swing_grid_of("country-shuffle");
    let train = swing_of("country-train");

    assert_eq!(grid, "8th", "a shuffle swings the 8ths, not the 16ths");
    assert!(
        (0.58..=0.66).contains(&shuffle),
        "the researched shuffle band is 58–66%, got {shuffle}"
    );
    assert!(
        shuffle > train,
        "country-shuffle ({shuffle}) must swing harder than country-train ({train})"
    );

    // ...and both still play to the 2 and the 4, which is what makes the swing
    // the difference between them rather than one difference among several.
    let context = ctx(4);
    let backbeat = [beat(&context), beat(&context) * 3];

    let mut shuffled = 0;
    for (seed, note) in sweep(&model("country-shuffle"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        shuffled += 1;
        let within = note.start_tick % context.ticks_per_bar();
        assert!(
            backbeat.contains(&within),
            "seed {seed}: the shuffle's snare left the backbeat at {within}"
        );
    }
    assert!(shuffled > 0, "the shuffle kept no backbeat at all");

    let mut accents = 0;
    for (seed, note) in sweep(&model("country-train"), Lane::Snare, 4) {
        if note.articulation != Some(Articulation::Accent) {
            continue;
        }
        accents += 1;
        let within = note.start_tick % context.ticks_per_bar();
        assert!(
            backbeat.contains(&within),
            "seed {seed}: the train beat accented {within}"
        );
    }
    assert!(accents > 0, "the train beat accented nothing");
}

// ------------------------------------------------------------------ neo-soul

#[test]
fn neo_soul_is_looser_and_more_extended_than_the_r_and_b_it_came_from() {
    // Research ch. 1 §9 and ch. 2 §6, plus the Dilla "drunk" note in ch. 1 §8:
    // neo-soul is the R&B it extends pulled in two directions at once. The
    // harmony thickens — "extended chords standard (maj7/m7/9/m9/maj9/11/13)",
    // triads authored down to 0.06 against `rnb-2000s`' 0.32 — and the pocket
    // loosens, `quantizeStrength` 0.58 where the parent quantizes at 0.70.
    // **Either half alone is a different genre**: loose triads are boom bap and
    // tight extensions are trap soul, so both are asserted, and both against
    // the named parent rather than as absolutes, because the claim is a
    // direction of travel and an absolute bound can pass for the wrong reason.
    let neo = extended_share("neo-soul");
    let rnb = extended_share("rnb-2000s");
    assert!(
        neo > rnb,
        "neo-soul voiced {neo:.2} of its chords as sevenths or richer against \
         rnb-2000s' {rnb:.2} — the m9/maj9 stack is the half of this genre that is not drums"
    );
    assert!(
        neo > 0.85,
        "a triad should be the exception in neo-soul, got {neo:.2} extended"
    );

    let loose = quantize_of("neo-soul");
    assert!(
        loose < quantize_of("rnb-2000s"),
        "neo-soul quantized at {loose} — the drunk pocket has to be the looser of the two"
    );
    // ...and jerk keeps its title as the loosest model in the dataset.
    assert!(
        loose > quantize_of("jerk"),
        "neo-soul sits just above jerk, not below it, and went to {loose}"
    );
}

// ------------------------------------------------------------------------ funk

#[test]
fn funk_brushes_more_ghost_snare_than_boom_bap() {
    // Research ch. 1 §§8–9: boom bap and 2000s R&B both put "ghost snares
    // 20–40% on e/a slots", and both took the device from funk, where it is not
    // an ornament but the hand itself — a hard 2 and 4 with nearly every
    // remaining sixteenth brushed underneath it. So funk's claim is comparative
    // rather than absolute: it must out-ghost the dataset's ghost-heavy
    // reference, both as a share of its own snare lane and as a raw count.
    let ghosts = |id: &str| {
        let hits = sweep(&model(id), Lane::Snare, 4);
        assert!(!hits.is_empty(), "{id} generated no snare at all");
        let quiet = hits
            .iter()
            .filter(|(_, n)| n.articulation == Some(Articulation::Ghost))
            .count();
        (
            quiet as f64 / hits.len() as f64,
            quiet as f64 / (SEEDS as f64 * 4.0),
        )
    };

    let (funk_share, funk_per_bar) = ghosts("funk");
    let (bap_share, bap_per_bar) = ghosts("boom-bap");

    assert!(
        funk_share > 0.65,
        "only {funk_share:.2} of funk's snare lane was ghosted — that is a backbeat \
         with ornament on it, not the funk hand"
    );
    assert!(
        funk_share > bap_share * 1.5,
        "funk ({funk_share:.2}) must ghost far harder than boom bap ({bap_share:.2})"
    );
    assert!(
        funk_per_bar > bap_per_bar * 3.0,
        "funk plays {funk_per_bar:.1} ghost snares a bar against boom bap's {bap_per_bar:.1}"
    );
}

#[test]
fn future_bass_hangs_an_extension_heavy_major_harmony_off_a_backbeat_on_three() {
    // Research ch. 2 §11: future bass is the fast-tempo *half-time* lane —
    // ~140–160 notated with the backbeat on 3 — and the harmony is what the
    // genre actually is: "SPREAD voicings across 4–5 octaves with 7/9/11/13 +
    // supersaws ... add9/add11 on most chords". Both halves have to hold at
    // once, or what is left is dance-pop played at the wrong tempo — which is
    // why `dance-pop` is the control here rather than an absolute threshold
    // alone: same major-key EDM-pop family, triads for the most part.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let three = beat(&context) * 2;

    let mut backbeats = 0;
    for (seed, note) in sweep(&model("future-bass"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        assert_eq!(
            note.start_tick % bar_ticks,
            three,
            "seed {seed}: a half-time backbeat left beat 3"
        );
        backbeats += 1;
    }
    assert!(backbeats > 0, "future bass produced no backbeat at all");

    let future = extended_share("future-bass");
    let dance = extended_share("dance-pop");
    assert!(
        future > 0.8,
        "future bass voiced only {future:.2} of its chords past the triad — \
         the ninths are the genre, not a garnish"
    );
    assert!(
        future > dance,
        "future bass ({future:.2}) must be more extended than dance-pop ({dance:.2})"
    );
}

// ----------------------------------------------------- the global club lanes

// ------------------------------------------------------------------ afrobeats

#[test]
fn afrobeats_lets_the_percussion_lead_instead_of_the_snare() {
    // Research ch. 2 §10: "ALL parts lock to clave", and the interlocking-part
    // culture *is* the genre — a shaker on the 16ths with congas, bongos,
    // clave, side-stick and woodblock answering each other, while the backbeat
    // is a colour rather than an engine. So the perc lanes have to outweigh the
    // snare. Trap is the same sentence inverted: its `percs` is `["rim",
    // "snap"]` at [0, 2], a garnish over a backbeat — which is why the
    // comparison, and not a threshold on afrobeats alone, is what makes the
    // claim checkable.
    let percussion_per_snare = |id: &str| {
        let m = model(id);
        let context = ctx(4);
        let (mut percs, mut snare) = (0usize, 0usize);
        for seed in 0..SEEDS {
            for track in generate(&m, &context, seed) {
                // ⛔ **Derived from `PERC_LANES`, never retyped** — the same rule
                // `drums_core.rs` states. Writing the *complement* by hand was
                // worse than a copy: a lane added to `PERC_LANES` would have been
                // swept into this numerator silently, while the baile-funk test
                // below kept measuring its stale six.
                if track.lane == Lane::Snare {
                    snare += track.notes.len();
                } else if PERC_LANES.contains(&track.lane) {
                    percs += track.notes.len();
                }
            }
        }
        assert!(
            snare > 0,
            "{id} has no snare to weigh the percussion against"
        );
        percs as f64 / snare as f64
    };

    let afro = percussion_per_snare("afrobeats");
    assert!(
        afro > 3.0,
        "afrobeats played {afro:.2} perc hits per snare hit — that is a kit-led beat"
    );

    let trap = percussion_per_snare("trap");
    assert!(
        afro > trap * 3.0,
        "afrobeats ({afro:.2}) must be far more percussion-led than trap ({trap:.2})"
    );
}

#[test]
fn amapianos_log_drum_plays_its_own_figure_instead_of_the_kicks() {
    // Research ch. 2 §10: "amapiano log drum = tuned percussive bass carrying
    // melodic movement in sub range". ⛔ **That is the genre's whole marker**,
    // and it is authored as the *bassline* — `independent_riff`, low register,
    // `followRootsProb` 0.45 — so it must run far denser than a bass that
    // shadows the kit, which is what boom bap's `mirror_kick` is. The second
    // half is the other thing that keeps it off house's patch: house and pop
    // sit on a straight grid at a neighbouring tempo, and amapiano shuffles.
    let context = ctx(4);
    let bass_per_bar = |id: &str| {
        let m = model(id);
        let notes: usize = (0..SEEDS)
            .map(|seed| {
                let harmony = engine::generators::chords::generate(&m, &context, seed);
                let kit = generate(&m, &context, seed);
                engine::generators::bass::generate(&m, &context, seed, &harmony, &kit)
                    .notes
                    .len()
            })
            .sum();
        notes as f64 / (SEEDS as f64 * 4.0)
    };

    let log_drum = bass_per_bar("amapiano");
    let follower = bass_per_bar("boom-bap");
    assert!(
        follower > 1.0,
        "boom bap must generate a bass at all to be compared against"
    );
    assert!(
        log_drum > 5.0,
        "the log drum carries the groove rather than punctuating it — got {log_drum:.2} a bar"
    );
    assert!(
        log_drum > follower * 1.4,
        "amapiano ({log_drum:.2}/bar) must out-play a root-following bass ({follower:.2}/bar)"
    );

    let amapiano = swing_of("amapiano");
    assert!(
        (0.56..=0.60).contains(&amapiano),
        "amapiano should shuffle just past the MPC setting, got {amapiano}"
    );
    assert!(
        amapiano > swing_of("pop-2000s"),
        "amapiano swings where the straight-grid four-on-the-floor family does not"
    );
}

// ------------------------------------------------------------------ dancehall

#[test]
fn dancehalls_one_drop_leaves_the_downbeat_empty() {
    // Research ch. 2 §10, and the reggae/dancehall one-drop convention it sits
    // on: the kick does *not* play beat 1. It drops on the 3 with the rim, and
    // the hole where the downbeat should be is the whole feel — dancehall is
    // the only model in the dataset that anchors on the 3 alone. Stated against
    // trap, which anchors every bar on the 1, and house, which walks all four,
    // because an absolute threshold on its own would also pass for a model that
    // simply plays very few kicks anywhere.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let bars = SEEDS as f64 * 4.0;

    let share_on = |id: &str, offset: u32| {
        sweep(&model(id), Lane::Kick, 4)
            .iter()
            .filter(|(_, n)| n.start_tick % bar_ticks == offset)
            .count() as f64
            / bars
    };

    let dancehall = share_on("dancehall", 0);
    let trap = share_on("trap", 0);
    let house = share_on("house", 0);

    assert!(
        dancehall < 0.2,
        "dancehall kicked the 1 in {dancehall:.3} of its bars — the one drop leaves it empty"
    );
    assert!(
        dancehall * 3.0 < trap && dancehall * 3.0 < house,
        "dancehall ({dancehall:.3}) must sit far under trap ({trap:.3}) and house ({house:.3})"
    );

    // ...and the drop itself: beat 3 is the anchor, and it is there every bar.
    let on_three = share_on("dancehall", beat(&context) * 2);
    assert!(
        on_three > 0.95,
        "the one drop lands on the 3, got {on_three:.2}"
    );
}

#[test]
fn reggaetons_dembow_marks_the_offbeats_of_essentially_every_bar() {
    // The dembow *is* the genre: under a plain backbeat, a rim/snare figure
    // marks the "a" of 1 and the "&" of 2, then says it again across the second
    // half of the bar — boom-ch-boom-chick, bar after bar, with almost no
    // variation. The compendium reaches only as far as the dancehall-influenced
    // mainstream (research ch. 2 §10), so this placement is documented
    // convention rather than in-repo research; pinning it against `pop-2000s`,
    // whose single offbeat ghost on the 4a is a garnish, is what makes the claim
    // mean "near-invariant" rather than merely "present".
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();
    let bars = usize::from(context.bars);

    let marked_share = |m: &StyleModel| {
        let mut marked = vec![false; SEEDS as usize * bars];
        for (seed, note) in sweep(m, Lane::Snare, context.bars) {
            // The fill's roll notes are not the figure, and neither is the
            // backbeat the figure sits under.
            if note.articulation != Some(Articulation::Ghost) {
                continue;
            }
            let sixteenth = (note.start_tick % bar_ticks) / grid::SIXTEENTH;
            if grid::is_downbeat(sixteenth, &context) {
                continue;
            }
            marked[seed as usize * bars + (note.start_tick / bar_ticks) as usize] = true;
        }
        marked.iter().filter(|bar| **bar).count() as f64 / marked.len() as f64
    };

    let reggaeton = marked_share(&model("reggaeton"));
    let pop = marked_share(&model("pop-2000s"));

    assert!(
        reggaeton > 0.95,
        "the dembow should carry offbeat 16ths in essentially every bar, got {reggaeton:.3}"
    );
    assert!(
        reggaeton > pop * 3.0,
        "reggaeton marked {reggaeton:.3} of its bars against pop's {pop:.3} — \
         that is a straight backbeat genre wearing the name"
    );
}

#[test]
fn baile_funk_is_percussion_led_over_a_kick_that_leaves_the_quarters() {
    // The tamborzão, and the whole reason this genre is in the roster: funk
    // carioca is a *percussion* loop built from atabaque and tamborim voices,
    // not a kick-and-snare grid. Research ch. 2 §10 supplies the club grammar
    // around it; the pattern itself is documented outside the compendium,
    // which is why the model states `confidence: "medium"`.
    //
    // Both halves are comparative on purpose. An absolute perc count would
    // pass for any busy model, so the perc field is measured against this
    // model's own kick — jersey club, the other kick-cluster genre, runs about
    // three perc hits per kick and would fail this bound. And "syncopated" is
    // only meaningful against something that is not: house is the other
    // 120–135 club genre here and its kick *is* the quarters.
    let context = ctx(4);
    let baile = model("baile-funk");
    let per_bar =
        |m: &StyleModel, lane: Lane| sweep(m, lane, 4).len() as f64 / (SEEDS as f64 * 4.0);

    // ⛔ Derived from `PERC_LANES` rather than a hand-typed six, so a lane added
    // to the kit is counted here instead of quietly ignored.
    let percussion: f64 = PERC_LANES.iter().map(|lane| per_bar(&baile, *lane)).sum();
    let kick = per_bar(&baile, Lane::Kick);

    assert!(kick > 0.0, "the tamborzão still has a bass drum under it");
    assert!(
        percussion > kick * 4.0,
        "baile funk played {percussion:.2} perc hits a bar against {kick:.2} kicks — \
         the percussion is the lead voice here, not a garnish on a drum grid"
    );

    let off_the_quarters = |m: &StyleModel| {
        let kicks = sweep(m, Lane::Kick, 4);
        let off = kicks
            .iter()
            .filter(|(_, n)| {
                !grid::is_downbeat(
                    (n.start_tick % context.ticks_per_bar()) / grid::SIXTEENTH,
                    &context,
                )
            })
            .count();
        off as f64 / kicks.len() as f64
    };

    let baile_off = off_the_quarters(&baile);
    let house_off = off_the_quarters(&model("house"));
    assert!(
        baile_off > 0.55,
        "only {baile_off:.2} of baile funk's kicks left the quarters — that is a grid, not a tamborzão"
    );
    assert!(
        baile_off > house_off + 0.35,
        "baile funk ({baile_off:.2}) must sit off the quarters far further than house ({house_off:.2})"
    );
}

#[test]
fn afroswing_carries_an_808_where_afrobeats_carries_none() {
    // Research ch. 2 §10, the UK crossover: "afroswing kicks 1 & 2.75, snare
    // 2&4" is an afrobeats pocket, but everything built over it is trap
    // production — a real 808 underneath, which neither afrobeats nor dancehall
    // has, and swung 16ths where trap is "50% straight default" (ch. 1 §1).
    // Both halves together are what makes it the crossover rather than either
    // parent, so the test asserts both.
    let afroswing = model("afroswing");
    let context = ctx(4);

    let played_an_808 = (0..SEEDS)
        .filter(|seed| !notes(&generate(&afroswing, &context, *seed), Lane::Sub).is_empty())
        .count();
    assert_eq!(
        played_an_808 as u64, SEEDS,
        "the 808 is this genre's low end, not an ornament — it played on {played_an_808} seeds"
    );

    // Against the parent it is most often mistaken for. Five times over rather
    // than "none at all", because the claim is about which record the sub
    // belongs to and the looser bound still says it.
    let ours = sweep(&afroswing, Lane::Sub, 4).len();
    let afrobeats = sweep(&model("afrobeats"), Lane::Sub, 4).len();
    assert!(
        ours > afrobeats * 5,
        "afroswing played {ours} 808 notes against afrobeats' {afrobeats}"
    );

    // ...and it swings where trap does not (ch. 1 cross-genre constants: 50%
    // straight, 54% the modern groove, 58% the classic MPC shuffle).
    let pocket = swing_of("afroswing");
    assert!(
        (0.55..=0.58).contains(&pocket),
        "afroswing sits between straight and the MPC shuffle, got {pocket}"
    );
    assert!(
        pocket > swing_of("trap"),
        "trap is the straight one; afroswing is the swung one"
    );
}

// ---------------------------------------------------------- the rap lineages

#[test]
fn memphis_rap_is_the_sparse_ancestor_phonk_was_built_from() {
    // Research ch. 1 §7 holds two genres in one section, and the split is the
    // whole point: classic Memphis is the tape-era ancestor, while modern drift
    // phonk is the derivative that is "driving, denser than trap (4–6/bar incl.
    // offbeats)" with "OCTAVE glides" for its 808 signature. Both halves of the
    // Memphis claim are comparisons, so both are asserted against phonk itself —
    // an absolute threshold on Memphis alone would pass just as well for a model
    // that had quietly lost its kick.
    let per_bar = |id: &str| {
        let m = model(id);
        sweep(&m, Lane::Kick, 4).len() as f64 / (SEEDS as f64 * 4.0)
    };
    let memphis = per_bar("memphis-rap");
    let phonk = per_bar("phonk");
    let trap = per_bar("trap");

    assert!(
        memphis * 1.5 < phonk,
        "memphis ran {memphis:.2} kicks a bar against phonk's {phonk:.2} — \
         the ancestor is the sparse one"
    );
    // ...and under trap as well, since phonk is the one defined as beating it.
    assert!(
        memphis < trap,
        "memphis ({memphis:.2}) should sit below trap ({trap:.2})"
    );
    assert!(
        memphis > 1.5,
        "a Memphis beat still keeps a pulse, got {memphis:.2}"
    );

    // The 808 is a long subby boom layered onto the kick one-shot rather than a
    // gliding lead — `slideProb` 0.06 against phonk's 0.5.
    let memphis_slides = slide_share("memphis-rap");
    let phonk_slides = slide_share("phonk");
    assert!(
        memphis_slides * 3.0 < phonk_slides,
        "memphis slid {memphis_slides:.3} of its 808 notes against phonk's \
         {phonk_slides:.3} — the still sub is the marker"
    );
}

#[test]
fn g_funk_walks_a_bass_where_the_club_lane_plays_an_808() {
    // Research ch. 3, Dr. Dre's G-funk lane: "played/replayed Moog-style
    // basslines — melodic, syncopated, muted; no 808 slides; bass = second
    // melody", over a kit the live musicians re-play. So g-funk is a kit record
    // the way boom bap is — `bass808: null` — and its low end is a walking part
    // of its own rather than a shadow of the kick. `west-coast-club` is the
    // Mustard-era contrast from ch. 1 §6: same coast, overlapping tempo, but
    // there the bass *is* the 808 sitting on the drum kit.
    assert!(
        sweep(&model("g-funk"), Lane::Sub, 4).is_empty(),
        "g-funk grew an 808 — it is a sampled and replayed kit record"
    );
    assert!(
        !sweep(&model("west-coast-club"), Lane::Sub, 4).is_empty(),
        "the club lane g-funk is defined against must still keep its 808"
    );

    let context = ctx(4);
    let bass_notes = |id: &str| {
        let style = model(id);
        (0..SEEDS)
            .map(|seed| {
                let harmony = engine::generators::chords::generate(&style, &context, seed);
                let kit = generate(&style, &context, seed);
                engine::generators::bass::generate(&style, &context, seed, &harmony, &kit)
                    .notes
                    .len()
            })
            .sum::<usize>()
    };

    let walking = bass_notes("g-funk");
    let root_follower = bass_notes("boom-bap");
    assert!(
        walking > root_follower,
        "g-funk played {walking} bass notes against boom bap's kick-mirroring {root_follower} — \
         the whole point is that it does not follow the kick"
    );

    // ...and it actually walks: a figure on the 16ths, not a note a beat.
    let per_bar = walking as f64 / (SEEDS as f64 * 4.0);
    assert!(
        per_bar > 4.0,
        "a walking bass should outrun the quarter note, got {per_bar:.2} notes a bar"
    );
}

#[test]
fn lofi_hiphop_is_the_slower_swung_jazzier_boom_bap() {
    // Research ch. 1 §8, the Dilla/neo-soul "drunk" variant of boom bap: the
    // shuffle measures ~62.5% where the classic MPC setting is 58%, and ch. 2 §6
    // puts the extended stack — m7/m9/maj9 with ii–v–i motion — at the centre of
    // that harmony. Lo-fi is that variant grown into a genre of its own, so the
    // claim is a comparison against its parent rather than a threshold of its
    // own: a bound on lo-fi alone would pass for a model that had merely been
    // nudged, which is exactly the failure two models sharing a parent invite.
    let session = |id: &str| {
        model(id)
            .session
            .clone()
            .unwrap_or_else(|| panic!("`{id}` states a session"))
    };

    let tempo = |id: &str| {
        session(id)
            .bpm
            .and_then(|bpm| bpm.mode)
            .unwrap_or_else(|| panic!("`{id}` states a bpm mode"))
    };
    let (lofi_bpm, boom_bpm) = (tempo("lofi-hiphop"), tempo("boom-bap"));
    assert!(
        lofi_bpm < boom_bpm,
        "lo-fi centres at {lofi_bpm} BPM, which is not slower than boom bap's {boom_bpm}"
    );

    let (lofi_swing, boom_swing) = (swing_of("lofi-hiphop"), swing_of("boom-bap"));
    assert!(
        lofi_swing > boom_swing,
        "lo-fi shuffled at {lofi_swing} against boom bap's classic MPC {boom_swing} — \
         heavier swing is half of what makes it the softer record"
    );
    assert!(
        (0.6..=0.65).contains(&lofi_swing),
        "the heavy neo-soul shuffle sits near 62%, got {lofi_swing}"
    );

    // The other half, and the one a producer hears first: the harmony.
    let lofi = extended_share("lofi-hiphop");
    let boom = extended_share("boom-bap");
    assert!(
        lofi > boom,
        "lo-fi voiced {lofi:.2} of its chords as sevenths or richer against boom bap's \
         {boom:.2} — the jazzier of the two is the whole reason it is a separate model"
    );
    assert!(
        lofi > 0.75,
        "the sevenths and ninths are the genre, got {lofi:.2}"
    );
}

#[test]
fn sexy_drill_keeps_drills_two_bar_snare_while_voicing_r_and_b_harmony() {
    // Research ch. 3 (Cash Cobain — sexy drill / sample drill) with ch. 1 §2:
    // this is the Brooklyn chassis carrying a sped-up R&B sample, so the snare
    // still walks 3 in the first bar and 4 in the second while the harmony
    // turns over completely — "jazzy 7th chords" against NY drill's two-chord
    // minor vamp of triads. Both halves have to hold at once, or the model is
    // drill with the wrong chords, or R&B with the wrong drums.
    let context = ctx(4);
    let bar_ticks = context.ticks_per_bar();

    let mut walked = 0;
    for (seed, note) in sweep(&model("sexy-drill"), Lane::Snare, 4) {
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
        // The authored 0–3 ms of lay-back, and nothing like a moved beat.
        assert!(
            within.abs_diff(expected) <= 16,
            "seed {seed}, bar {bar}: snare at {within}, expected near {expected}"
        );
        walked += 1;
    }
    assert!(walked > 0, "sexy drill produced no snare to walk");

    let sexy = extended_share("sexy-drill");
    let drill = extended_share("ny-drill");
    assert!(
        sexy > 0.8,
        "sexy drill voiced only {sexy:.2} of its chords as sevenths or richer — \
         the sped-up R&B sample is the entire lane"
    );
    assert!(
        sexy > drill * 3.0,
        "sexy drill ({sexy:.2}) must be far more extended than ny drill ({drill:.2})"
    );
}

// -------------------------------------------------------------------- hyphy

#[test]
fn hyphy_bounces_harder_and_faster_than_the_la_club_lane() {
    // Research ch. 1 §6 documents the West Coast club grammar as the LA lane —
    // a sparse kick anchored on 1 and 3 at around 100 BPM. Hyphy is the Bay
    // cousin of that same family and is defined against it: the kick anchors
    // only the 1 and spends its density on the offbeat 8ths with a 3-3-2 lean,
    // and it runs materially faster. Both halves are comparisons, because
    // "busier" and "faster" mean nothing except next to the neighbour the genre
    // is actually heard against.
    let context = ctx(4);
    let offbeat_share = |id: &str| {
        let kicks = sweep(&model(id), Lane::Kick, 4);
        let offbeat = kicks
            .iter()
            .filter(|(_, n)| {
                grid::is_offbeat_eighth(
                    (n.start_tick % context.ticks_per_bar()) / grid::SIXTEENTH,
                    &context,
                )
            })
            .count();
        offbeat as f64 / kicks.len() as f64
    };

    let hyphy = offbeat_share("hyphy");
    let west = offbeat_share("west-coast-club");
    assert!(
        hyphy > west,
        "hyphy ({hyphy:.3}) should lean offbeat harder than west coast club ({west:.3})"
    );
    assert!(
        hyphy > 0.25,
        "the off-kilter bounce is the genre: only {hyphy:.3} of hyphy's kicks were offbeat 8ths"
    );

    let tempo_centre = |id: &str| {
        let m = model(id);
        m.session
            .as_ref()
            .and_then(|s| s.bpm.as_ref())
            .and_then(|b| b.mode)
            .unwrap_or_else(|| panic!("{id} must state a tempo centre"))
    };
    let bay = tempo_centre("hyphy");
    let la = tempo_centre("west-coast-club");
    assert!(
        bay >= la + 5.0,
        "hyphy sits at {bay} against west coast club's {la} — not a material difference"
    );
}

#[test]
fn crunk_strips_traps_hat_carpet_back_to_eighths() {
    // Crunk is trap's ancestor with the detail taken back out (research ch. 1
    // §1 as the descendant, §7 Memphis as the neighbour): plain 8ths that stop
    // and start — `continuous: false` — where trap carpets the 16ths and rolls
    // across them. What it keeps is the half-time backbeat both are built on,
    // so the difference between the two really is the hat part and nothing else.
    let per_bar =
        |id: &str| sweep(&model(id), Lane::ClosedHat, 4).len() as f64 / (SEEDS as f64 * 4.0);
    let crunk = per_bar("crunk");
    let trap = per_bar("trap");

    assert!(crunk > 0.0, "crunk still plays a hat part");
    assert!(
        crunk * 3.0 < trap * 2.0,
        "crunk played {crunk:.2} closed hats a bar against trap's {trap:.2} — \
         that is a carpet, not the 8ths crunk is"
    );

    // ...and the thing it kept: the backbeat hard on 3.
    let context = ctx(4);
    let three = beat(&context) * 2;
    for (seed, note) in sweep(&model("crunk"), Lane::Snare, 4) {
        if !is_backbeat(&note) {
            continue;
        }
        assert_eq!(
            note.start_tick % context.ticks_per_bar(),
            three,
            "seed {seed}: crunk's backbeat left beat 3"
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
        "dark-trap",
        "bouncy-trap",
        "trap-soul",
        "cloud-rap",
        "emo-rap",
        "dark-plugg",
        "detroit-bounce",
        "jersey-club",
        "atl-swag-rap",
        "uk-underground",
        "edm-rage",
        "digicore",
        "jump-up-dnb",
        "neurofunk",
        "jungle",
        "pop-dnb",
        "uk-garage",
        "house",
        "dance-pop",
        "pop-2020s",
        "country-pop",
        "country-shuffle",
        "neo-soul",
        "funk",
        "future-bass",
        "afrobeats",
        "amapiano",
        "dancehall",
        "reggaeton",
        "baile-funk",
        "afroswing",
        "memphis-rap",
        "g-funk",
        "lofi-hiphop",
        "sexy-drill",
        "hyphy",
        "crunk",
    ];

    // Genres only: the artists are covered by
    // `the_ten_flagship_artists_all_ship_and_generate` and
    // `every_flagship_artist_sounds_unlike_the_genre_it_extends`, which is a
    // stronger claim than a checklist because it compares output.
    let shipped_ids: Vec<String> = shipped()
        .iter()
        .filter(|(id, model)| {
            !id.starts_with('_') && model.model_type == engine::dataset::ModelType::Genre
        })
        .map(|(id, _)| id.clone())
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

// --------------------------------------------------------------- the artists

#[test]
fn every_flagship_artist_sounds_unlike_the_genre_it_extends() {
    // The product's whole premise (PRD § 1): "Trap is not Metro Boomin". An
    // artist model that generates what its genre already generates is a name
    // in a list, not a style.
    let models = shipped();
    let context = ctx(4);

    let mut checked = 0;
    for (id, model) in models {
        // ⚠ Producers are held to the same claim as artists — "Trap is not
        // Metro Boomin" is *about* a producer — so this asks whether the model
        // is a style rather than whether it is specifically an artist.
        if !model.model_type.is_style() {
            continue;
        }
        let parent = model
            .blocks
            .get("extends")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_owned);
        // `extends` is consumed by inheritance, so read it from the file the
        // roster kept rather than the resolved model.
        let Some(parent) = parent.or_else(|| {
            models
                .keys()
                .find(|candidate| model.genres.iter().any(|g| g == *candidate))
                .cloned()
        }) else {
            continue;
        };
        let Some(base) = models.get(&parent) else {
            continue;
        };

        let mut different = 0;
        for seed in 0..20u64 {
            if generate(model, &context, seed) != generate(base, &context, seed) {
                different += 1;
            }
        }
        assert_eq!(
            different, 20,
            "{id} generates the same drums as `{parent}` on some seeds"
        );
        checked += 1;
    }

    assert!(checked > 0, "no artist model was compared to its genre");
}

#[test]
fn the_ten_flagship_artists_all_ship_and_generate() {
    // Named explicitly (PRD § 5 US-001): these are the roster the magic moment
    // is demonstrated with, so a missing one is a missing demo.
    const FLAGSHIPS: &[&str] = &[
        "metro-boomin",
        "southside",
        "pierre-bourne",
        "osamason",
        "nettspend",
        "summrs",
        "pop-smoke",
        "travis-scott",
        "future",
        "drake",
    ];

    let models = shipped();
    let context = ctx(4);

    for id in FLAGSHIPS {
        let model = models
            .get(*id)
            .unwrap_or_else(|| panic!("`{id}` must ship — it is a named flagship"));
        assert_eq!(model.tier, Some(engine::dataset::Tier::Flagship), "{id}");
        assert!(!model.aliases.is_empty(), "{id} needs aliases to be found");

        for seed in 0..20u64 {
            let lanes = generate(model, &context, seed);
            assert!(!lanes.is_empty(), "{id} seed {seed}: generated nothing");
            for track in &lanes {
                for n in &track.notes {
                    assert!(
                        n.start_tick < context.total_ticks(),
                        "{id}: note past the end"
                    );
                    assert!(n.vel >= 1 && n.vel <= 127, "{id}: bad velocity");
                }
            }
        }
    }
}
