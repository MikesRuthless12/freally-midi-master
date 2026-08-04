//! Song Mode for a **genre with no artist over it** (TASK-074, FR-008 AC).
//!
//! ⛔ **A genre archetype is a different code path from an artist, and nothing
//! else in the suite exercises it deliberately.** An artist model is an
//! `extends` chain resolved down onto a genre; a genre is the bottom of that
//! chain with only `_defaults` behind it. So a genre is the case where an
//! authored value is *missing* — no `arrangement` override, no `sectionRules`
//! for a kind the form names, no `switchUpProb` — and every one of those is a
//! fallback that only runs here. `arrange.rs`'s own SAMPLE reaches two genres
//! incidentally; this reaches five deliberately, one per family.
//!
//! The five are the families the roadmap names, and they are not
//! interchangeable: trap and uk-drill author their own forms, pop-2000s is the
//! only family with a **pre-chorus**, liquid-dnb has 16-bar cores, and
//! country-train is the one whose research states verse-16/chorus-8 with fills
//! every 8. A suite that ran five trap-shaped genres would pass while the
//! authored variety did nothing.

use engine::arrange;
use engine::context::SessionContext;
use engine::midi::song_to_smf;
use engine::pattern::{Part, SectionKind, Song};
use engine::StyleModel;

mod common;
use common::shipped_models;

/// One family each, per FR-008's acceptance criterion.
const FAMILIES: [&str; 5] = [
    "trap",
    "uk-drill",
    "pop-2000s",
    "liquid-dnb",
    "country-train",
];

fn model(id: &str) -> StyleModel {
    shipped_models()
        .remove(id)
        .unwrap_or_else(|| panic!("no `{id}` in the shipped dataset"))
}

fn song(id: &str, seed: u64) -> Song {
    arrange::generate(&model(id), &SessionContext::default(), seed)
        .unwrap_or_else(|error| panic!("{id} at seed {seed}: {error}"))
}

#[test]
fn every_family_builds_a_song_with_no_artist_over_it() {
    for id in FAMILIES {
        for seed in 0..12u64 {
            let song = song(id, seed);
            assert!(
                !song.sections.is_empty(),
                "{id}/{seed} built a song with no sections"
            );
            assert!(
                !song.patterns.is_empty(),
                "{id}/{seed} built a song that plays nothing"
            );
            assert!(
                song.dangling_refs().is_empty(),
                "{id}/{seed} names clips it does not carry: {:?}",
                song.dangling_refs()
            );
            // The artist is the genre itself, which is what "genre mode" means.
            assert_eq!(song.artist_id, id);
        }
    }
}

#[test]
fn every_familys_structure_survives_generation_intact() {
    // ⛔ The form is sampled from what the model authored, so every section it
    // produces has to be one the model asked for — in the same order, tiled end
    // to end. A generator that quietly substituted `_defaults`' form for a
    // genre whose own form failed to parse would pass every other test here.
    for id in FAMILIES {
        let forms = arrange::structures_of(&model(id)).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!forms.is_empty(), "{id} authors no song form");

        let authored: Vec<Vec<SectionKind>> = forms
            .iter()
            .map(|form| form.iter().filter_map(|name| kind_of(name)).collect())
            .collect();

        for seed in 0..24u64 {
            let song = song(id, seed);
            let built: Vec<SectionKind> = song.sections.iter().map(|s| s.kind).collect();
            assert!(
                authored.contains(&built),
                "{id}/{seed} built {built:?}, which is none of the forms it authors: {authored:?}"
            );

            // End to end, no gap and no overlap — the invariant everything else
            // in the timeline and the export rests on.
            let mut expected = 0u32;
            for section in &song.sections {
                assert_eq!(section.start_bar, expected, "{id}/{seed} has a gap");
                assert!(section.bars >= 1, "{id}/{seed} has a zero-bar section");
                expected += u32::from(section.bars);
            }
        }
    }
}

#[test]
fn a_forced_form_is_the_form_that_gets_built() {
    // The structure picker's whole contract (TASK-070), asserted per family
    // because the fallback paths differ: a genre with one authored form must
    // honour index 0, and one with several must honour each.
    for id in FAMILIES {
        let model = model(id);
        let forms = arrange::structures_of(&model).expect("authors forms");
        for (index, form) in forms.iter().enumerate() {
            let song = arrange::generate_with(&model, &SessionContext::default(), 7, Some(index))
                .unwrap_or_else(|e| panic!("{id} form {index}: {e}"));
            let built: Vec<SectionKind> = song.sections.iter().map(|s| s.kind).collect();
            let wanted: Vec<SectionKind> = form.iter().filter_map(|n| kind_of(n)).collect();
            assert_eq!(built, wanted, "{id} form {index} was not the form built");
        }
    }
}

#[test]
fn pop_authors_a_pre_chorus_and_a_generated_song_actually_gets_one() {
    // ⛔ **A claim about the *data*, and it is the reason `PreChorus` exists as
    // a kind at all.** `docs/style-research.md` ch.1 states pop's
    // V-PC-C-V2-PC-C-B-C outright, and there is no honest way to spell a
    // pre-chorus in the other five kinds. If this stops holding, either the
    // research was re-read or the authored form was lost — both are worth
    // failing a build over.
    let pop_forms = arrange::structures_of(&model("pop-2000s")).expect("pop authors forms");
    assert!(
        pop_forms.iter().any(|form| form
            .iter()
            .any(|name| kind_of(name) == Some(SectionKind::PreChorus))),
        "pop-2000s no longer authors a pre-chorus: {pop_forms:?}"
    );

    // And it really reaches a generated song, not just the authoring.
    let reached = (0..48u64).any(|seed| {
        song("pop-2000s", seed)
            .sections
            .iter()
            .any(|s| s.kind == SectionKind::PreChorus)
    });
    assert!(reached, "no pop-2000s song in 48 seeds had a pre-chorus");
}

#[test]
fn the_families_do_not_all_write_the_same_song() {
    // ⛔ Song Mode's product claim is that the form reads as *that* music. Five
    // genres producing one shape would satisfy every other test in this file —
    // the sections would tile, the refs would resolve, the export would open —
    // and the feature would be worthless.
    let shapes: Vec<String> = FAMILIES
        .iter()
        .map(|id| {
            let song = song(id, 11);
            song.sections
                .iter()
                .map(|s| format!("{:?}:{}", s.kind, s.bars))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();

    let distinct: std::collections::BTreeSet<&String> = shapes.iter().collect();
    assert!(
        distinct.len() >= 4,
        "five families produced {} distinct arrangements: {shapes:?}",
        distinct.len()
    );
}

#[test]
fn every_familys_song_exports_and_plays_the_same_notes() {
    // The two things a producer does with a song, over the archetypes rather
    // than over the flagship artists the rest of the suite uses.
    for id in FAMILIES {
        let song = song(id, 5);

        let bytes = song_to_smf(&song);
        assert_eq!(&bytes[..4], b"MThd", "{id}: not an SMF");
        assert!(bytes.len() > 64, "{id}: an empty-looking export");

        let flat = song.flatten();
        assert!(
            flat.note_count() > 0,
            "{id}: the arrangement plays no notes at all"
        );
        assert_eq!(
            u32::from(flat.bars),
            song.total_bars(),
            "{id}: the flattened clip is not as long as the song"
        );
    }
}

#[test]
fn a_family_whose_808_is_its_bassline_carries_no_separate_bass_row() {
    // FR-007, at the arrangement level. Trap's 808 *is* the bassline, so a bass
    // row would double it — a production mistake rather than a fuller sound.
    // Asserted here because a song is where the omission is visible as a
    // missing row rather than as a refused request.
    let trap = model("trap");
    assert!(engine::generators::bass::eight_o_eight_is_the_bass(&trap));

    let song = song("trap", 3);
    assert!(
        song.sections
            .iter()
            .all(|section| !section.patterns.contains_key(&Part::Bass)),
        "trap grew a separate bass row beside its 808"
    );
}

/// The engine's section vocabulary, for reading an authored name.
///
/// ⚠ Mirrors `arrange::section_kind`, which is private. `chorus` is `hook`
/// under pop's name for it; a name this does not know is dropped, which is what
/// makes the comparison in `every_familys_structure_survives_generation_intact`
/// fail loudly rather than silently pass on an unmapped kind.
fn kind_of(name: &str) -> Option<SectionKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "intro" => Some(SectionKind::Intro),
        "verse" => Some(SectionKind::Verse),
        "prechorus" | "pre-chorus" => Some(SectionKind::PreChorus),
        "hook" | "chorus" => Some(SectionKind::Hook),
        "bridge" => Some(SectionKind::Bridge),
        "outro" => Some(SectionKind::Outro),
        _ => None,
    }
}
