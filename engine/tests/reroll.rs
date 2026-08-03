//! Re-rolling one section of a song (TASK-067).
//!
//! ⛔ **Every test here asserts the sections that were *not* named**, not the
//! one that was. A re-roll that quietly regenerated the whole song still
//! changes the section it was pointed at, and would pass any assertion that
//! only looked there — which is the shape of the bug this task's own acceptance
//! criterion ("re-roll section 2 leaves other sections byte-stable") describes.
//!
//! The sharing rule is what makes that non-trivial: verse 1 and verse 2 play
//! the *same* clip, so a re-roll done in place would change every verse in the
//! song and nothing about the section it was asked for would look wrong.

use engine::arrange::{self, ArrangeError};
use engine::context::SessionContext;
use engine::pattern::{Part, Song};
use engine::StyleModel;

mod common;
use common::shipped_models;

/// The same four models the arrangement suite works over: a flagship artist, a
/// genre with its own arrangement authoring, and one with none of its own so
/// the `_defaults` inheritance path is exercised too.
const SAMPLE: [&str; 4] = ["trap", "osamason", "uk-drill", "boom-bap"];

fn model(id: &str) -> StyleModel {
    shipped_models()
        .remove(id)
        .unwrap_or_else(|| panic!("the shipped dataset has no `{id}`"))
}

fn ctx() -> SessionContext {
    SessionContext::default()
}

/// The first section playing more than one part, so a partial lock has
/// something to lock. Which sections exist varies by seed, so this is found
/// rather than assumed.
fn multi_part_section(song: &Song) -> usize {
    song.sections
        .iter()
        .position(|s| s.patterns.len() > 1)
        .expect("no section in this song plays more than one part")
}

#[test]
fn rerolling_one_section_leaves_every_other_section_byte_stable() {
    for id in SAMPLE {
        let model = model(id);
        let song = arrange::generate(&model, &ctx(), 4_242).expect("builds");
        let target = multi_part_section(&song);

        let rerolled =
            arrange::reroll_section(&model, &ctx(), &song, target, 99, &[]).expect("re-rolls");

        assert_eq!(
            rerolled.sections.len(),
            song.sections.len(),
            "{id}: a re-roll changed the number of sections"
        );

        for (index, (before, after)) in song
            .sections
            .iter()
            .zip(rerolled.sections.iter())
            .enumerate()
        {
            assert_eq!(
                before.kind, after.kind,
                "{id}: section {index} changed kind"
            );
            assert_eq!(
                (before.start_bar, before.bars),
                (after.start_bar, after.bars),
                "{id}: section {index} moved"
            );
            if index == target {
                continue;
            }
            for (part, reference) in &before.patterns {
                let was = song.pattern(reference).expect("resolves");
                let now = rerolled
                    .pattern(&after.patterns[part])
                    .expect("still resolves");
                assert_eq!(
                    was.lanes, now.lanes,
                    "{id}: re-rolling section {target} changed section {index}'s {part:?}"
                );
            }
        }
    }
}

#[test]
fn a_reroll_actually_changes_the_section_it_names() {
    // The other half, and the reason the test above is not sufficient on its
    // own: a "re-roll" that returned the song untouched would pass it perfectly.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 8).expect("builds");
    let target = multi_part_section(&song);

    let rerolled =
        arrange::reroll_section(&trap, &ctx(), &song, target, 5_150, &[]).expect("re-rolls");

    let after = &rerolled.sections[target];
    let changed = song.sections[target].patterns.iter().any(|(part, was)| {
        song.pattern(was).expect("resolves").lanes
            != rerolled
                .pattern(&after.patterns[part])
                .expect("resolves")
                .lanes
    });
    assert!(changed, "the re-rolled section plays exactly what it did");
}

#[test]
fn a_locked_part_survives_a_reroll_note_for_note() {
    // What "lock-respecting" means at this layer: the engine is handed the
    // parts the producer pinned, and must not touch them.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 21).expect("builds");
    let target = multi_part_section(&song);
    let locked: Vec<Part> = song.sections[target]
        .patterns
        .keys()
        .copied()
        .take(1)
        .collect();

    let rerolled =
        arrange::reroll_section(&trap, &ctx(), &song, target, 777, &locked).expect("re-rolls");

    for part in &locked {
        let was = song
            .pattern(&song.sections[target].patterns[part])
            .expect("resolves");
        let now = rerolled
            .pattern(&rerolled.sections[target].patterns[part])
            .expect("resolves");
        assert_eq!(was.lanes, now.lanes, "the locked {part:?} was re-rolled");
    }
}

#[test]
fn a_reroll_with_the_chords_locked_writes_its_melody_against_those_chords() {
    // ⛔ The coherence bug the per-section seed already fixed once, arriving
    // again through the back door. `render_section` derives the harmony from
    // the seed it is handed, so re-rolling the melody alone would write it
    // against a voicing from the *new* seed — a different harmony from the
    // chord clip the section still plays. `Carry` is what closes it.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 63).expect("builds");
    let target = song
        .sections
        .iter()
        .position(|s| {
            s.patterns.contains_key(&Part::Chords) && s.patterns.contains_key(&Part::Melody)
        })
        .expect("no section plays both chords and melody");

    // ⛔ **Mutation-tested by construction rather than by recomputing the
    // implementation.** The same song is re-rolled twice with the same re-roll
    // seed and the chords locked, differing only in the seed the *locked chord
    // clip* was made with. With the carry in place the re-rolled melody follows
    // that harmony; with it dropped, both re-rolls derive their harmony from the
    // re-roll seed and come out byte-identical whatever the locked chords say.
    let chords_id = song.sections[target].patterns[&Part::Chords]
        .pattern_id
        .clone();

    let melody_after = |chords_seed: u64| {
        let mut variant = song.clone();
        variant.patterns.get_mut(&chords_id).expect("resolves").seed = chords_seed;
        let rerolled =
            arrange::reroll_section(&trap, &ctx(), &variant, target, 1_234, &[Part::Chords])
                .expect("re-rolls");
        rerolled
            .pattern(&rerolled.sections[target].patterns[&Part::Melody])
            .expect("the section still plays a melody")
            .lanes
            .clone()
    };

    // ⚠ **Searched for rather than hard-coded, and the reason is measured:**
    // `melody::generate` reads the harmony it is handed for only about 8 seeds
    // in 20 — trap's melody often takes its pitches from the scale instead —
    // so a fixed pair of chord seeds lands on "no difference" more often than
    // not, and a bare `assert_ne!` over one pair would report a *working* carry
    // as broken. Finding any pair that moves proves the dependency exists;
    // finding none over the whole sweep is what a dropped carry looks like.
    let baseline = melody_after(1);
    let moved = (2..48u64).any(|seed| melody_after(seed) != baseline);

    assert!(
        moved,
        "no locked harmony in 47 changed the re-rolled melody, so the melody is \
         not being written against the chords the section actually plays"
    );
}

#[test]
fn locking_every_part_is_a_no_op_rather_than_a_regeneration() {
    // The UI's change detection compares by identity, so "nothing was unlocked"
    // has to come back as the same arrangement rather than as an equal-but-new
    // one — which would mark the song edited and turn a no-op into an undo step.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 12).expect("builds");
    let locked: Vec<Part> = song.sections[0].patterns.keys().copied().collect();

    let rerolled =
        arrange::reroll_section(&trap, &ctx(), &song, 0, 999, &locked).expect("re-rolls");
    assert_eq!(rerolled, song);
}

#[test]
fn rerolling_repeatedly_does_not_grow_the_pattern_store() {
    // ⛔ A `Song` crosses the bridge as JSON and is saved with the project.
    // Without pruning, twenty re-rolls of one section leave twenty abandoned
    // clips in the file — and `song_to_smf` is handed every one of them.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 77).expect("builds");
    let target = multi_part_section(&song);
    let before = song.patterns.len();

    let mut rolling = song;
    for seed in 0..20u64 {
        rolling =
            arrange::reroll_section(&trap, &ctx(), &rolling, target, seed, &[]).expect("re-rolls");
    }

    assert!(
        rolling.patterns.len() <= before + rolling.sections[target].patterns.len(),
        "twenty re-rolls grew the store from {before} to {}",
        rolling.patterns.len()
    );
    assert!(
        rolling.dangling_refs().is_empty(),
        "pruning removed a clip a section still names: {:?}",
        rolling.dangling_refs()
    );
}

#[test]
fn the_same_reroll_seed_reproduces_the_same_section() {
    // The property the whole engine rests on, applied to the new entry point.
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 5).expect("builds");
    let target = multi_part_section(&song);

    let a = arrange::reroll_section(&trap, &ctx(), &song, target, 606, &[]).expect("re-rolls");
    let b = arrange::reroll_section(&trap, &ctx(), &song, target, 606, &[]).expect("re-rolls");
    assert_eq!(a, b);
}

#[test]
fn two_sections_of_one_kind_diverge_rather_than_moving_together() {
    // ⛔ The failure the index keying exists for. Verse 1 and verse 2 share one
    // clip by id; re-rolling verse 2 in place would rewrite the entry both are
    // looking at, so verse 1 would change with it and the producer would have
    // re-rolled a section they never selected.
    let trap = model("trap");
    // A pair of sections that genuinely share their clips — which is the only
    // case this test says anything about. Searched for rather than assumed,
    // because which sections a seed produces varies.
    let (song, first, second) = (0..64u64)
        .find_map(|seed| {
            let song = arrange::generate(&trap, &ctx(), seed).ok()?;
            let pair = (0..song.sections.len()).find_map(|second| {
                let later = &song.sections[second];
                if later.patterns.is_empty() {
                    return None;
                }
                (0..second).find(|&first| song.sections[first].patterns == later.patterns)
            })?;
            let second = (0..song.sections.len())
                .find(|&i| i > pair && song.sections[i].patterns == song.sections[pair].patterns)?;
            Some((song, pair, second))
        })
        .expect("no seed in 64 produced two sections sharing their clips");

    let rerolled =
        arrange::reroll_section(&trap, &ctx(), &song, second, 4_004, &[]).expect("re-rolls");

    for (part, reference) in &song.sections[first].patterns {
        assert_eq!(
            song.pattern(reference).expect("resolves").lanes,
            rerolled
                .pattern(&rerolled.sections[first].patterns[part])
                .expect("resolves")
                .lanes,
            "re-rolling section {second} moved its twin at {first}"
        );
    }
}

#[test]
fn a_reroll_past_the_end_of_the_song_is_refused_by_name() {
    let trap = model("trap");
    let song = arrange::generate(&trap, &ctx(), 3).expect("builds");
    let count = song.sections.len();

    let error = arrange::reroll_section(&trap, &ctx(), &song, count, 1, &[]).unwrap_err();
    assert_eq!(
        error,
        ArrangeError::NoSuchSection {
            song: song.id.clone(),
            index: count,
            sections: count,
        }
    );
    assert!(error.to_string().contains(&count.to_string()));
}

#[test]
fn every_section_of_every_sample_model_can_be_rerolled() {
    // ⛔ `section_name` is the inverse of `section_kind`, and a re-roll looks a
    // section's *rule* back up through it. A pair that disagreed would re-roll
    // at `_defaults` density instead of the section's own, silently — and an
    // unmapped kind would panic on the index rather than returning an error.
    for id in SAMPLE {
        let model = model(id);
        for seed in 0..8u64 {
            let song = arrange::generate(&model, &ctx(), seed).expect("builds");
            for index in 0..song.sections.len() {
                let rerolled = arrange::reroll_section(&model, &ctx(), &song, index, seed, &[])
                    .unwrap_or_else(|e| panic!("{id} section {index}: {e}"));
                assert_eq!(
                    rerolled.sections[index].kind, song.sections[index].kind,
                    "{id}: re-rolling a {:?} produced a {:?}",
                    song.sections[index].kind, rerolled.sections[index].kind
                );
                assert!(rerolled.dangling_refs().is_empty(), "{id} section {index}");
            }
        }
    }
}
