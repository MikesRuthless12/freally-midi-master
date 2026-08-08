//! The Arrangement Creator (TASK-065, FR-008).
//!
//! Every other suite proves one pattern. This one proves a *song*: that the form
//! a model says it writes is the form it writes, that the sections line up end
//! to end with no gap and no overlap, that every reference resolves, and that
//! the whole thing is reproducible from its seed.
//!
//! The statistics test is the one that matters most and is the easiest to get
//! wrong. A structure sampler that always returns the first form passes every
//! assertion about section order; only counting a hundred runs against the
//! authored weights catches it.

use std::collections::BTreeMap;

use engine::arrange::{self, ArrangeError};
use engine::context::SessionContext;
use engine::generators::grid;
use engine::pattern::{Part, SectionKind};
use engine::{parts, StyleModel};

mod common;
use common::shipped_models;

/// Models a song is asked for across the suite: a flagship artist, a genre with
/// its own arrangement authoring, and one with none of its own so the
/// `_defaults` inheritance path is exercised too.
const SAMPLE: [&str; 4] = ["trap", "osamason", "uk-drill", "boom-bap"];

fn model(id: &str) -> StyleModel {
    shipped_models()
        .remove(id)
        .unwrap_or_else(|| panic!("the shipped dataset has no `{id}`"))
}

fn ctx() -> SessionContext {
    SessionContext::default()
}

/// The name a [`SectionKind`] is compared under.
fn kind_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Intro => "intro",
        SectionKind::Verse => "verse",
        SectionKind::PreChorus => "prechorus",
        SectionKind::Hook => "hook",
        SectionKind::Bridge => "bridge",
        SectionKind::Outro => "outro",
    }
}

/// An authored section name reduced to the kind it means.
///
/// ⛔ Both sides of the statistics comparison go through this. `chorus` and
/// `hook` are one kind under two genres' names for it, so a model authoring
/// `chorus` would otherwise produce an observed form that matches no authored
/// key — and the test would report every form as 0% rather than as a mismatch,
/// which reads as the sampler being broken when it is the comparison.
fn canonical_section(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "intro" => "intro",
        "verse" => "verse",
        "prechorus" | "pre-chorus" => "prechorus",
        "hook" | "chorus" => "hook",
        "bridge" => "bridge",
        "outro" => "outro",
        other => panic!("`{other}` is not a section name the engine knows"),
    }
}

// ---------------------------------------------------------------------------
// engine::parts — the shared renderer Song Mode and the pattern path both use
// ---------------------------------------------------------------------------

#[test]
fn a_part_renders_the_same_notes_for_the_same_seed() {
    // The determinism the product rests on, asserted at the seam Song Mode
    // calls a few dozen times per song.
    let trap = model("trap");
    for part in [
        Part::Drums,
        Part::Chords,
        Part::Melody,
        Part::Counter,
        Part::Bass,
    ] {
        let a = parts::render(&trap, &ctx(), parts::Seeds::shared(7), part);
        let b = parts::render(&trap, &ctx(), parts::Seeds::shared(7), part);
        assert_eq!(a, b, "{part:?} is not reproducible from its seed");
    }
}

// ---------------------------------------------------------------------------
// TASK-141 — the two-seed design
// ---------------------------------------------------------------------------

#[test]
fn a_new_take_changes_the_part_and_leaves_the_record_alone() {
    // ⛔ **Both halves at once, which is the entire claim of TASK-141.** The
    // Defect 2 fix made Generate reroll — and in doing so made the ordinary
    // workflow (Generate on Drums, switch tab, Generate on Melody) draw two
    // unrelated seeds, so the melody was written against a harmony the chords
    // tab had never seen. One seed could give variation *or* coherence, never
    // both.
    let trap = model("trap");

    // Half one: a different take is a different part.
    let take_a = parts::render(
        &trap,
        &ctx(),
        parts::Seeds {
            song: 7,
            part: 1,
            drums: None,
        },
        Part::Drums,
    );
    let take_b = parts::render(
        &trap,
        &ctx(),
        parts::Seeds {
            song: 7,
            part: 2,
            drums: None,
        },
        Part::Drums,
    );
    assert_ne!(
        take_a, take_b,
        "rerolling the take must change the drums — this is Defect 2"
    );

    // Half two: the record survives it. Chords are the harmonic plan, so they
    // are a property of the song seed and must not move when a take rerolls.
    //
    // ⛔ **Compared as harmony — pitch and time — not as whole notes, and the
    // distinction is the design rather than a weakened assertion.** `humanize`
    // runs on the **part** seed, so a new take genuinely does play the same
    // chords with a different velocity shape. That is what a take *is*: same
    // record, different performance. Asserting on the full `Note` made this
    // fail on `vel` alone while every pitch and tick matched exactly.
    let harmony_of = |seeds| {
        parts::render(&trap, &ctx(), seeds, Part::Chords)
            .iter()
            .flat_map(|track| track.notes.iter())
            .map(|note| (note.start_tick, note.len_ticks, note.pitch))
            .collect::<Vec<_>>()
    };

    let chords_a = harmony_of(parts::Seeds {
        song: 7,
        part: 1,
        drums: None,
    });
    let chords_b = harmony_of(parts::Seeds {
        song: 7,
        part: 99,
        drums: None,
    });
    assert_eq!(
        chords_a, chords_b,
        "the harmonic plan belongs to the record, not to a take"
    );

    // And a different record really is a different record.
    let other = harmony_of(parts::Seeds {
        song: 8,
        part: 1,
        drums: None,
    });
    assert_ne!(chords_a, other, "a new song seed must write new harmony");
}

#[test]
fn every_part_of_one_record_is_written_against_the_same_harmony() {
    // ⛔ **The property `arrange::render_section`'s own doc records losing.**
    // An earlier cut called `parts::render` per part with a per-part seed and
    // every melody's internal harmony came out a different voicing from the
    // chords beside it — "both clips were individually correct and the pair had
    // never been written against each other".
    //
    // Five parts, five *different* takes, one song seed. The melodic parts are
    // each written against `chords::generate(model, ctx, song)`, so generating
    // them at different moments cannot put them on different progressions.
    let trap = model("trap");
    let song = 4_242;

    let melody_now = parts::render(
        &trap,
        &ctx(),
        parts::Seeds {
            song,
            part: 11,
            drums: None,
        },
        Part::Melody,
    );
    // The same take, asked for again after four other parts have been
    // generated at their own take seeds. Nothing about the order may matter.
    for take in [12, 13, 14, 15] {
        let _ = parts::render(
            &trap,
            &ctx(),
            parts::Seeds {
                song,
                part: take,
                drums: None,
            },
            Part::Counter,
        );
    }
    let melody_again = parts::render(
        &trap,
        &ctx(),
        parts::Seeds {
            song,
            part: 11,
            drums: None,
        },
        Part::Melody,
    );
    assert_eq!(
        melody_now, melody_again,
        "a part is a pure function of its two seeds, whatever was generated between"
    );
}

#[test]
fn one_seed_for_both_is_exactly_what_it_always_was() {
    // ⚠ The compatibility claim, and it is load-bearing: every saved project
    // written before TASK-141 carries one seed, and `Seeds::shared` is what it
    // means. If this drifts, reopening an old project stops reproducing the
    // beat it was saved with — which is US-004, the promise the seed chip makes.
    let trap = model("trap");
    for part in [Part::Drums, Part::Chords, Part::Melody, Part::Bass] {
        let shared = parts::render(&trap, &ctx(), parts::Seeds::shared(7), part);
        let spelled = parts::render(
            &trap,
            &ctx(),
            parts::Seeds {
                song: 7,
                part: 7,
                drums: None,
            },
            part,
        );
        assert_eq!(shared, spelled, "{part:?}");
    }
}

/// A session with the feel switched off, so a tick is a tick.
///
/// `quantize_strength: 1.0` scales the per-lane jitter to nothing and swing at
/// 0.5 is straight, which is what lets the assertion below be about *positions*
/// rather than about a tolerance. The kick and the bass are humanized on
/// different streams, so with the shipped defaults they drift a millisecond or
/// two apart and "is this note on that kick" stops having an exact answer.
fn on_the_grid() -> SessionContext {
    SessionContext {
        humanize: engine::Humanize {
            quantize_strength: 1.0,
            velocity_var: 0.0,
            timing_jitter_ms: BTreeMap::new(),
        },
        ..Default::default()
    }
}

#[test]
fn a_mirrored_bass_lands_on_the_kick_the_drums_are_actually_playing() {
    // ⛔⛔ **The two parts that are supposed to read as one instrument played
    // twice were landing in different places.** `bassline.rhythm =
    // "mirror_kick"` is the roster default and it copies the kick's ticks
    // *verbatim* — but the reference kit it copied was generated at the SONG
    // seed, while the drums the producer can see came from their own take. With
    // one seed, 13 of 13 boom-bap bass notes sat on a real kick; with the two
    // seeds the ordinary workflow produces, 9 of 13, and `uk-drill` fell to 1
    // of 14.
    //
    // ⚠ **Split seeds on purpose.** `Seeds::shared` cannot fail this — the
    // record and the take being one number is precisely the case that always
    // worked. The take numbers below are arbitrary and unequal, which is what
    // "Generate on Drums, switch tab, Generate on Bass" actually sends.
    const RECORD: u64 = 7;
    const DRUMS_TAKE: u64 = 3141;
    const BASS_TAKE: u64 = 2718;

    let ctx = on_the_grid();
    let mut measured: Vec<String> = Vec::new();

    for (id, model) in shipped_models() {
        // Only the models this rule is about: a bass that mirrors, and that is
        // not deferring to an 808 (FR-007). Read off the resolved model, so a
        // roster edit changes what is covered rather than what is asserted.
        let mirrors = model
            .blocks
            .get("bassline")
            .and_then(|b| b.get("rhythm"))
            .and_then(|r| r.as_str())
            // Absent is `mirror_kick` — `bass::generate`'s own default, and
            // what most of the roster inherits rather than states.
            .unwrap_or("mirror_kick")
            == "mirror_kick";
        if !mirrors {
            continue;
        }

        // The drums on screen: their own take, exactly as the Drums tab sent.
        let drums = parts::render(
            &model,
            &ctx,
            parts::Seeds {
                song: RECORD,
                part: DRUMS_TAKE,
                drums: None,
            },
            Part::Drums,
        );
        let kicks: Vec<u32> = drums
            .iter()
            .filter(|track| track.lane == engine::pattern::Lane::Kick)
            .flat_map(|track| track.notes.iter().map(|note| note.start_tick))
            .collect();
        if kicks.is_empty() {
            continue;
        }

        // ...and the bass generated afterwards, told which take to mirror.
        let bass = parts::render(
            &model,
            &ctx,
            parts::Seeds {
                song: RECORD,
                part: BASS_TAKE,
                drums: Some(DRUMS_TAKE),
            },
            Part::Bass,
        );
        if parts::is_silent(&bass) {
            continue;
        }

        measured.push(id.clone());
        for note in bass.iter().flat_map(|track| track.notes.iter()) {
            // ⚠ Two legal positions, not one. `anticipationProb` pushes a note
            // one 16th early on purpose — that is drill's lean — so "on the
            // kick" means the kick's tick or the 16th before it. Asserting only
            // the first would fail on a genre that is behaving correctly.
            let on_kick = kicks.contains(&note.start_tick);
            let anticipating = kicks.contains(&(note.start_tick + grid::SIXTEENTH));
            assert!(
                on_kick || anticipating,
                "{id}: a mirrored bass note at tick {} is on no kick the drums play \
                 — the kicks are {kicks:?}",
                note.start_tick
            );
        }
    }

    // The filter above is data-driven, so it could quietly select nothing and
    // report success. This is what stops that.
    assert!(
        measured.len() >= 3,
        "only {} shipped model(s) exercised the mirror — {measured:?}",
        measured.len()
    );
}

#[test]
fn a_style_whose_808_is_the_bassline_renders_a_silent_bass() {
    // Not a bug and not a failure: trap's 808 plays the bass inside the drums,
    // and a second bass under it is a muddier low end rather than a fuller one
    // (FR-007). `is_silent` is how a caller tells this apart from lanes it did
    // not ask for.
    let trap = model("trap");
    assert!(
        parts::is_silent(&parts::render(
            &trap,
            &ctx(),
            parts::Seeds::shared(7),
            Part::Bass
        )),
        "trap authors no separate bass part"
    );
    assert!(
        !parts::is_silent(&parts::render(
            &trap,
            &ctx(),
            parts::Seeds::shared(7),
            Part::Drums
        )),
        "its drums are where the 808 is"
    );
}

// ---------------------------------------------------------------------------
// The shape of a song
// ---------------------------------------------------------------------------

#[test]
fn every_shipped_model_builds_a_song() {
    // `_defaults` carries an arrangement block and everything inherits it, so a
    // model that cannot build one has broken that chain — which is invisible
    // until somebody presses Generate in Song Mode on that one artist.
    let models = shipped_models();
    let mut failed: Vec<String> = Vec::new();

    for (id, model) in &models {
        if id.starts_with('_') {
            continue;
        }
        if let Err(error) = arrange::generate(model, &ctx(), 4242) {
            failed.push(format!("{id}: {error}"));
        }
    }

    assert!(
        failed.is_empty(),
        "every shipped model must produce a song:\n{}",
        failed.join("\n")
    );
}

#[test]
fn sections_tile_the_song_with_no_gap_and_no_overlap() {
    // A gap is silence the timeline does not draw; an overlap is two sections
    // claiming the same bar. Either one makes the ruler disagree with the
    // export, and neither is visible in a single section.
    for id in SAMPLE {
        let model = model(id);
        for seed in [1u64, 2, 99, 100_000] {
            let song = arrange::generate(&model, &ctx(), seed).expect("builds");
            let mut expected = 0u32;
            for section in &song.sections {
                assert_eq!(
                    section.start_bar, expected,
                    "{id} seed {seed}: {:?} starts at {} not {expected}",
                    section.kind, section.start_bar
                );
                assert!(section.bars >= 1, "{id}: a zero-bar section is a hole");
                expected += u32::from(section.bars);
            }
            assert_eq!(song.total_bars(), expected);
        }
    }
}

#[test]
fn every_reference_resolves_to_a_pattern_in_the_store() {
    // The failure this catches draws as an empty row and exports as silence,
    // with nothing anywhere saying why.
    for id in SAMPLE {
        let model = model(id);
        for seed in [7u64, 8, 5_000] {
            let song = arrange::generate(&model, &ctx(), seed).expect("builds");
            assert!(
                song.dangling_refs().is_empty(),
                "{id} seed {seed}: {:?} name no pattern",
                song.dangling_refs()
            );
            assert!(
                !song.patterns.is_empty(),
                "{id}: a song with no patterns is silence"
            );
        }
    }
}

#[test]
fn the_same_seed_rebuilds_the_identical_song() {
    for id in SAMPLE {
        let model = model(id);
        let a = arrange::generate(&model, &ctx(), 31_337).expect("builds");
        let b = arrange::generate(&model, &ctx(), 31_337).expect("builds");
        assert_eq!(a, b, "{id} is not reproducible from its seed");
    }
}

#[test]
fn a_different_seed_reaches_a_different_song() {
    // Song Mode is worthless if every seed lands on the same arrangement. This
    // is the coarse floor; the statistics test below is the real measurement.
    let trap = model("trap");
    let songs: Vec<String> = (0..40u64)
        .map(|seed| {
            let song = arrange::generate(&trap, &ctx(), seed).expect("builds");
            song.sections
                .iter()
                .map(|s| format!("{:?}:{}", s.kind, s.bars))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    let distinct: std::collections::BTreeSet<&String> = songs.iter().collect();
    assert!(
        distinct.len() > 1,
        "40 seeds produced one arrangement: {:?}",
        distinct
    );
}

#[test]
fn sections_of_the_same_kind_play_the_same_pattern() {
    // Verse 1 and verse 2 are the same beat — in these genres that is the rule,
    // and `switchUpProb` exists because it is. It is also what makes the store
    // hold one pattern rather than one per section.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 11).expect("builds");

    let mut by_kind: BTreeMap<(String, Part), String> = BTreeMap::new();
    for section in &song.sections {
        for (part, reference) in &section.patterns {
            let key = (format!("{:?}", section.kind), *part);
            match by_kind.get(&key) {
                Some(seen) => assert_eq!(
                    seen, &reference.pattern_id,
                    "two {:?} sections play different {part:?} patterns",
                    section.kind
                ),
                None => {
                    by_kind.insert(key, reference.pattern_id.clone());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The statistics gate the PRD names: structure counts match the authored
// weights within 10 points over 100 seeds (FR-008 AC).
// ---------------------------------------------------------------------------

#[test]
fn structure_sampling_matches_the_authored_weights() {
    // ⛔ The test that catches a sampler which always returns the first form.
    // Every assertion about section order passes under that bug; only counting
    // runs against the weights sees it.
    //
    // The comparison is on the *section order* rather than a structure index,
    // because that is the observable thing — two structures authored with the
    // same sections are the same form however they are indexed.
    const RUNS: u64 = 400;
    const TOLERANCE: f64 = 10.0;

    let trap = model("trap");
    let arrangement = trap.blocks.get("arrangement").expect("trap has one");
    let structures = arrangement
        .get("structures")
        .and_then(|v| v.as_array())
        .expect("inherited from _defaults");

    // Authored share per form, keyed the same way the observation is.
    let total: f64 = structures
        .iter()
        .map(|s| s.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0))
        .sum();
    let expected: BTreeMap<String, f64> = structures
        .iter()
        .map(|s| {
            let key = s
                .get("sections")
                .and_then(|v| v.as_array())
                .expect("a structure lists sections")
                .iter()
                .filter_map(|v| v.as_str())
                .map(canonical_section)
                .collect::<Vec<_>>()
                .join(",");
            let weight = s.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0);
            (key, 100.0 * weight / total)
        })
        .collect();

    assert!(
        expected.len() > 1,
        "a single-form model cannot measure sampling"
    );

    let mut seen: BTreeMap<String, f64> = BTreeMap::new();
    for seed in 0..RUNS {
        let song = arrange::generate(&trap, &ctx(), seed).expect("builds");
        let key = song
            .sections
            .iter()
            .map(|s| kind_name(s.kind))
            .collect::<Vec<_>>()
            .join(",");
        *seen.entry(key).or_default() += 100.0 / RUNS as f64;
    }

    for (form, want) in &expected {
        let got = seen.get(form).copied().unwrap_or(0.0);
        assert!(
            (got - want).abs() <= TOLERANCE,
            "form `{form}` came out {got:.1}% against an authored {want:.1}% \
             (tolerance {TOLERANCE} points over {RUNS} seeds)"
        );
    }

    let unauthored: Vec<&String> = seen.keys().filter(|k| !expected.contains_key(*k)).collect();
    assert!(
        unauthored.is_empty(),
        "sampled a form the model never authored: {unauthored:?}"
    );
}

// ---------------------------------------------------------------------------
// Section rules: part masks, added layers, density
// ---------------------------------------------------------------------------

#[test]
fn a_section_only_carries_the_parts_its_rule_lists() {
    // `_defaults` gives the intro melody only and the bridge chords and the low
    // end. A section carrying a full kit the rule never asked for is the mask
    // being ignored, which reads as "the arrangement does nothing".
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 21).expect("builds");

    let intro = song
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Intro)
        .expect("the shipped structures all open on an intro");

    assert!(
        !intro.patterns.contains_key(&Part::Drums),
        "the intro rule lists melody only, and it has drums"
    );
}

#[test]
fn a_sections_density_multiplier_thins_the_part_it_applies_to() {
    // ⛔ **The obvious version of this test asserts nothing.** Comparing every
    // note in the hook against every note in the intro passes whatever the
    // multiplier does, because `_defaults` gives the hook five parts and the
    // intro one — five parts outnumber one at any density. The confound is the
    // part count, so the comparison has to hold it fixed: the *melody*, which
    // both sections carry, at 1.0 in the hook and 0.6 in the intro.
    //
    // Averaged over many seeds rather than asserted on one. The two patterns
    // run on different streams, so a single pair can land either way on luck
    // and a one-seed version of this would be flaky rather than wrong.
    const SEEDS: u64 = 60;
    let trap = model("trap");

    let mut intro_total = 0.0;
    let mut hook_total = 0.0;
    let mut counted = 0.0;

    for seed in 0..SEEDS {
        let song = arrange::generate(&trap, &ctx(), seed).expect("builds");
        let melody_notes = |kind: SectionKind| -> Option<f64> {
            let section = song.sections.iter().find(|s| s.kind == kind)?;
            let pattern = song.pattern(section.patterns.get(&Part::Melody)?)?;
            Some(pattern.note_count() as f64)
        };
        let (Some(intro), Some(hook)) = (
            melody_notes(SectionKind::Intro),
            melody_notes(SectionKind::Hook),
        ) else {
            continue;
        };
        intro_total += intro;
        hook_total += hook;
        counted += 1.0;
    }

    assert!(
        counted >= 20.0,
        "only {counted} seeds carried a melody in both sections, which is too \
         few to measure"
    );
    let intro_mean = intro_total / counted;
    let hook_mean = hook_total / counted;

    // ⛔ **Asserting `hook_mean > intro_mean` is not enough, and this was
    // watched passing on the broken code before it was written this way.** With
    // the multiplier disconnected the two means come out 21.60 and 21.67 — the
    // hook is "denser" by six hundredths of a note, which is noise, and a bare
    // inequality calls that a pass. The authored ratio is the thing worth
    // measuring, and it lands within a hundredth of 0.6 when the multiplier is
    // actually reaching the generator.
    const AUTHORED: f64 = 0.6;
    const TOLERANCE: f64 = 0.15;
    let ratio = intro_mean / hook_mean;
    assert!(
        (ratio - AUTHORED).abs() <= TOLERANCE,
        "the intro carries {ratio:.2} of the hook's melody notes against an \
         authored density of {AUTHORED} ({intro_mean:.2} vs {hook_mean:.2} over \
         {counted} seeds) — a ratio near 1.0 means the multiplier never reached \
         the generator"
    );
}

#[test]
fn a_density_of_one_generates_the_same_notes_as_a_plain_request() {
    // ⛔ The property `with_density`'s early return exists for. A hook runs at
    // 1.0, so its notes must be exactly what the pattern path returns for the
    // same model, seed and part — if round-tripping the model through serde to
    // change nothing is lossy, this is where it shows.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 44).expect("builds");

    let hook = song
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Hook)
        .expect("has a hook");
    let reference = hook.patterns.get(&Part::Drums).expect("the hook has drums");
    let built = song.pattern(reference).expect("resolves");

    let direct = parts::render(&trap, &ctx(), parts::Seeds::shared(built.seed), Part::Drums);
    assert_eq!(
        built.lanes, direct,
        "a full-density section did not match a plain request for the same seed"
    );
}

#[test]
fn a_section_that_asks_for_the_low_end_gets_it_even_when_the_808_is_the_bass() {
    // ⛔ `_defaults`' bridge asks for `chords` and `bass808` and not `drums`.
    // For trap the 808 lives in the drums, so a naive part mask gives that
    // bridge chords over nothing — and the hole is only audible in one section
    // of one form.
    let trap = model("trap");
    assert!(
        engine::generators::bass::eight_o_eight_is_the_bass(&trap),
        "this test is about the styles where it is"
    );

    // Walk seeds until a form with a bridge turns up; not every structure has
    // one, and asserting on a seed that produced no bridge would pass forever.
    let bridge = (0..80u64).find_map(|seed| {
        let song = arrange::generate(&trap, &ctx(), seed).ok()?;
        let section = song
            .sections
            .iter()
            .find(|s| s.kind == SectionKind::Bridge)?
            .clone();
        Some((song, section))
    });
    let Some((song, bridge)) = bridge else {
        panic!("no seed in 80 produced a bridge, so this gate is asserting nothing");
    };

    let low_end: usize = bridge
        .patterns
        .values()
        .filter_map(|r| song.pattern(r))
        .flat_map(|p| p.lanes.iter())
        .filter(|lane| lane.lane == engine::pattern::Lane::Sub)
        .map(|lane| lane.notes.len())
        .sum();

    assert!(
        low_end > 0,
        "the bridge asked for the low end and has no 808 under it"
    );
}

#[test]
fn a_section_never_carries_a_drum_kit_it_did_not_ask_for() {
    // The other half of the rule above: pulling the drums in for the 808 must
    // not smuggle a full kit into a section whose rule never listed drums.
    let trap = model("trap");
    let song = (0..80u64)
        .find_map(|seed| {
            let song = arrange::generate(&trap, &ctx(), seed).ok()?;
            song.sections
                .iter()
                .any(|s| s.kind == SectionKind::Bridge)
                .then_some(song)
        })
        .expect("a form with a bridge");

    let bridge = song
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Bridge)
        .expect("found above");

    for reference in bridge.patterns.values() {
        let pattern = song.pattern(reference).expect("resolves");
        if pattern.part != Part::Drums {
            continue;
        }
        for lane in &pattern.lanes {
            assert_eq!(
                lane.lane,
                engine::pattern::Lane::Sub,
                "the bridge asked for the 808 and got a {:?} lane too",
                lane.lane
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

#[test]
fn a_model_with_no_arrangement_is_refused_by_name() {
    // Silently inventing a structure would be the alternative, and a song form
    // nobody authored is exactly the "generator hands you someone else's song"
    // failure TASK-063A exists to prevent.
    let mut trap = model("trap");
    trap.blocks.remove("arrangement");
    assert_eq!(
        arrange::generate(&trap, &ctx(), 1),
        Err(ArrangeError::NoArrangement("trap".to_owned()))
    );
}

#[test]
fn a_model_with_no_structures_is_refused_by_name() {
    let mut trap = model("trap");
    trap.blocks.insert(
        "arrangement".to_owned(),
        serde_json::json!({ "sectionBars": { "verse": 8 } }),
    );
    assert_eq!(
        arrange::generate(&trap, &ctx(), 1),
        Err(ArrangeError::NoStructures("trap".to_owned()))
    );
}

#[test]
fn a_section_name_the_engine_has_no_kind_for_is_refused_by_name() {
    // Dropping it silently would shorten the song with nothing saying so, and
    // the author would be looking for a section that never plays.
    //
    // The name is a *typo* of a real one rather than an invented word, because
    // that is the case this actually catches: `prechorus` is a section the
    // engine knows, and one character off it is the mistake an author makes.
    let mut trap = model("trap");
    trap.blocks.insert(
        "arrangement".to_owned(),
        serde_json::json!({
            "structures": [{ "sections": ["intro", "prehcorus", "hook"], "weight": 1 }],
            "sectionBars": { "intro": 4, "prehcorus": 4, "hook": 8 }
        }),
    );
    assert_eq!(
        arrange::generate(&trap, &ctx(), 1),
        Err(ArrangeError::UnknownSection {
            model: "trap".to_owned(),
            name: "prehcorus".to_owned(),
        })
    );
}

#[test]
fn chorus_is_read_as_a_hook_rather_than_refused() {
    // The same section under two genres' names for it. Pop authoring says
    // chorus; trap authoring says hook.
    let mut trap = model("trap");
    trap.blocks.insert(
        "arrangement".to_owned(),
        serde_json::json!({
            "structures": [{ "sections": ["verse", "chorus"], "weight": 1 }],
            "sectionBars": { "verse": 8, "chorus": 8 }
        }),
    );
    let song = arrange::generate(&trap, &ctx(), 1).expect("builds");
    assert_eq!(
        song.sections.iter().map(|s| s.kind).collect::<Vec<_>>(),
        vec![SectionKind::Verse, SectionKind::Hook]
    );
}

#[test]
fn a_structure_longer_than_the_cap_is_truncated_rather_than_generated() {
    // Generation runs on the thread the host draws its window from. A community
    // dataset authoring four hundred sections is a frozen DAW, not a long song.
    let mut trap = model("trap");
    let sections: Vec<&str> = std::iter::repeat_n("verse", 400).collect();
    trap.blocks.insert(
        "arrangement".to_owned(),
        serde_json::json!({
            "structures": [{ "sections": sections, "weight": 1 }],
            "sectionBars": { "verse": 4 }
        }),
    );
    let song = arrange::generate(&trap, &ctx(), 1).expect("builds");
    assert!(
        song.sections.len() <= 64,
        "{} sections got through the cap",
        song.sections.len()
    );
}

// ---------------------------------------------------------------------------
// TASK-066 — section transitions
// ---------------------------------------------------------------------------

/// Walk seeds until one produces a song the predicate accepts.
fn find_song(
    model: &StyleModel,
    limit: u64,
    accept: impl Fn(&engine::pattern::Song) -> bool,
) -> Option<(u64, engine::pattern::Song)> {
    (0..limit).find_map(|seed| {
        let song = arrange::generate(model, &ctx(), seed).ok()?;
        accept(&song).then_some((seed, song))
    })
}

#[test]
fn a_drop_out_lands_on_the_section_before_a_hook_and_never_on_the_hook() {
    // ⛔ The drop-out is the beat or two of nothing that makes a hook land, so
    // it belongs to whatever runs *into* the hook. On the hook itself it would
    // cut the top off the thing it is announcing — and that is an easy mistake
    // to make, because the rule that configures it is read while the hook is
    // being built.
    let trap = model("trap");
    let mut seen_any = false;

    for seed in 0..120u64 {
        let song = arrange::generate(&trap, &ctx(), seed).expect("builds");
        for (index, section) in song.sections.iter().enumerate() {
            if section.drop_out_beats == 0 {
                continue;
            }
            seen_any = true;
            let next = song.sections.get(index + 1).unwrap_or_else(|| {
                panic!("seed {seed}: the last section dropped out into nothing")
            });
            assert_eq!(
                next.kind,
                SectionKind::Hook,
                "seed {seed}: a {:?} dropped out into a {:?} rather than a hook",
                section.kind,
                next.kind
            );
            // A drop-out longer than the section is a section that does not play.
            let beats_in_section = u32::from(section.bars) * 4;
            assert!(
                u32::from(section.drop_out_beats) < beats_in_section,
                "seed {seed}: a {}-beat drop-out on a {}-bar section",
                section.drop_out_beats,
                section.bars
            );
        }
    }

    assert!(
        seen_any,
        "no seed in 120 produced a drop-out, so this gate is asserting nothing \
         — `_defaults` authors dropOutBeats at prob 0.6"
    );
}

#[test]
fn a_switch_up_varies_the_back_half_melody_and_holds_the_drums() {
    // The device these models author is the one where the beat stays and the
    // melody moves. A switch-up that changed the drums too would be a different
    // beat rather than a switch-up, and the producer would hear the song
    // restart rather than turn over.
    let trap = model("trap");

    let found = find_song(&trap, 200, |song| {
        // Two verses whose melodies differ is the observable switch-up.
        let verses: Vec<&engine::pattern::Section> = song
            .sections
            .iter()
            .filter(|s| s.kind == SectionKind::Verse)
            .collect();
        verses.len() >= 2
            && verses
                .first()
                .and_then(|f| f.patterns.get(&Part::Melody))
                .zip(verses.last().and_then(|l| l.patterns.get(&Part::Melody)))
                .is_some_and(|(a, b)| a.pattern_id != b.pattern_id)
    });

    let Some((seed, song)) = found else {
        panic!("no seed in 200 switched up, so this gate is asserting nothing");
    };

    let verses: Vec<&engine::pattern::Section> = song
        .sections
        .iter()
        .filter(|s| s.kind == SectionKind::Verse)
        .collect();
    let (front, back) = (verses[0], verses[verses.len() - 1]);

    let melody_a = song
        .pattern(&front.patterns[&Part::Melody])
        .expect("resolves");
    let melody_b = song
        .pattern(&back.patterns[&Part::Melody])
        .expect("resolves");
    assert_ne!(
        melody_a.lanes, melody_b.lanes,
        "seed {seed}: the switched-up verse plays the identical melody"
    );

    if let (Some(a), Some(b)) = (
        front.patterns.get(&Part::Drums),
        back.patterns.get(&Part::Drums),
    ) {
        assert_eq!(
            a.pattern_id, b.pattern_id,
            "seed {seed}: the switch-up changed the drums as well as the melody"
        );
    }
}

#[test]
fn most_songs_do_not_switch_up() {
    // `_defaults` authors `switchUpProb: 0.15`, and the value matters: a
    // generator that switched up every time could not repeat a hook, which is
    // most of what these genres are.
    const RUNS: u64 = 300;
    let trap = model("trap");

    let switched = (0..RUNS)
        .filter(|seed| {
            let song = arrange::generate(&trap, &ctx(), *seed).expect("builds");
            song.patterns.keys().any(|id| id.contains("switchup"))
        })
        .count();

    let share = switched as f64 / RUNS as f64;
    assert!(
        (0.05..=0.30).contains(&share),
        "{:.0}% of songs switched up against an authored 15% — a rate near 0 \
         means the roll never fires and one near 1 means it always does",
        share * 100.0
    );
}

#[test]
fn the_outro_is_marked_to_decay_and_other_sections_are_not() {
    // `_defaults` authors `decay: true` on the outro alone. The flag is read by
    // `midi::song_to_smf`, so this is a promise about the exported file rather
    // than a label.
    let trap = model("trap");
    let (_, song) = find_song(&trap, 60, |s| {
        s.sections.iter().any(|x| x.kind == SectionKind::Outro)
    })
    .expect("a form with an outro");

    for section in &song.sections {
        assert_eq!(
            section.decay,
            section.kind == SectionKind::Outro,
            "{:?} has decay = {}",
            section.kind,
            section.decay
        );
    }
}

// ---------------------------------------------------------------------------
// TASK-064 — the authored forms reach generation
// ---------------------------------------------------------------------------

#[test]
fn a_genre_that_authors_its_own_form_does_not_generate_the_default_one() {
    // ⛔ The failure this catches is silent and total: `structures` is an array,
    // and arrays *replace* under `extends` — but if a genre's block were merged
    // the wrong way round, or the key misspelled, every one of these would
    // quietly generate `_defaults`' trap form and nothing would say so. The
    // whole of TASK-064 would be a no-op that still passes `dataset:validate`.
    let defaults_form: Vec<String> = {
        let models = shipped_models();
        let trap = models.get("trap").expect("trap");
        // Trap authors its own now, so the check is that pop is not *this*.
        (0..40u64)
            .filter_map(|s| arrange::generate(trap, &ctx(), s).ok())
            .map(|song| {
                song.sections
                    .iter()
                    .map(|x| kind_name(x.kind))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect()
    };

    // Pop's form is stated outright in the research as V-PC-C-V2-PC-C-B-C, and
    // it is the one genre whose form no other genre shares.
    let pop = model("pop-2000s");
    let pop_forms: std::collections::BTreeSet<String> = (0..40u64)
        .filter_map(|s| arrange::generate(&pop, &ctx(), s).ok())
        .map(|song| {
            song.sections
                .iter()
                .map(|x| kind_name(x.kind))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();

    assert!(
        pop_forms.iter().all(|f| !defaults_form.contains(f)),
        "pop generated a form trap also generates: {pop_forms:?}"
    );
    assert!(
        pop_forms.iter().all(|f| f.contains("prechorus")),
        "every pop form must carry the pre-chorus its research states: {pop_forms:?}"
    );
}

#[test]
fn every_authored_section_name_is_one_the_engine_knows() {
    // A typo in a structure is refused at generation time by name — which is
    // correct behaviour and a terrible way to find out, because it only fires
    // for the artist somebody happens to pick. This is the gate that finds it
    // in CI instead, across the whole shipped dataset.
    let mut problems: Vec<String> = Vec::new();

    for (id, model) in shipped_models() {
        let Some(block) = model.blocks.get("arrangement") else {
            continue;
        };
        let Some(structures) = block.get("structures").and_then(|v| v.as_array()) else {
            continue;
        };
        for structure in structures {
            let Some(sections) = structure.get("sections").and_then(|v| v.as_array()) else {
                continue;
            };
            for name in sections.iter().filter_map(|v| v.as_str()) {
                let known = matches!(
                    name.trim().to_ascii_lowercase().as_str(),
                    "intro"
                        | "verse"
                        | "prechorus"
                        | "pre-chorus"
                        | "hook"
                        | "chorus"
                        | "bridge"
                        | "outro"
                );
                if !known {
                    problems.push(format!("{id}: `{name}`"));
                }
            }
        }
    }

    assert!(
        problems.is_empty(),
        "structures name sections the engine has no kind for:\n{}",
        problems.join("\n")
    );
}

#[test]
fn plugg_opens_on_chords_alone_the_way_its_research_states() {
    // `style-research.md` ch. 1 L82 is the only per-section instrumentation map
    // in the chapter — "intro 4 bars chords-only; verse 8 full; bridge =
    // kick+bass only" — so it is the one place a section rule can be checked
    // against a sentence rather than against itself.
    let plugg = model("plugg");
    let song = arrange::generate(&plugg, &ctx(), 5).expect("builds");
    let intro = song
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Intro)
        .expect("every plugg form opens on an intro");

    assert_eq!(
        intro.patterns.keys().copied().collect::<Vec<_>>(),
        vec![Part::Chords],
        "plugg's intro is chords-only"
    );
    assert_eq!(intro.bars, 4, "plugg's intro is four bars");
}

#[test]
fn a_sections_melody_is_written_against_the_chords_playing_beside_it() {
    // ⛔ **The bug this was written to catch, and it was watched failing.**
    // `parts::render` builds a melody *against* a harmony it generates
    // internally from the seed it is handed. If each part of a section is given
    // its own derived seed, that internal harmony is a different voicing from
    // the Chords clip sitting in the same section — so the two clips a producer
    // hears together were never written against each other. That is precisely
    // the "five parts versus one part played five times" failure the dependency
    // order in `engine::parts` exists to prevent, and nothing else in the suite
    // would see it: both clips are individually in key, in register and
    // reproducible.
    //
    // Asserted by regenerating the section's melody from the *chords clip's own
    // seed* and requiring the same notes — which holds exactly when both parts
    // of a section were derived from one seed.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 7).expect("builds");

    let section = song
        .sections
        .iter()
        .find(|s| s.patterns.contains_key(&Part::Melody) && s.patterns.contains_key(&Part::Chords))
        .expect("a section carrying both a melody and chords");

    let melody = song
        .pattern(&section.patterns[&Part::Melody])
        .expect("resolves");
    let chords = song
        .pattern(&section.patterns[&Part::Chords])
        .expect("resolves");

    assert_eq!(
        melody.seed, chords.seed,
        "the melody and the chords in one section came from different seeds, so \
         the harmony the melody was written against is not the harmony playing \
         under it"
    );

    // And the stronger form: the melody really is the one that seed produces.
    let expected = parts::render(
        &trap,
        &ctx(),
        parts::Seeds::shared(chords.seed),
        Part::Melody,
    );
    assert_eq!(
        melody.lanes, expected,
        "the melody is not what the section's own seed generates against its chords"
    );
}
