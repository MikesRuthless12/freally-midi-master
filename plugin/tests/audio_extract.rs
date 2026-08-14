//! The audio extractor against real files (TASK-058F / TASK-058G / TASK-058I).
//!
//! ⛔⛔ **The unit tests are synthetic and that is not enough.** Every threshold
//! in `plugin/src/extract` was chosen against a fixture this repo generates, and
//! a fixture generated to match a rule cannot tell you the rule is wrong. The
//! MIDI splitter learned this the hard way in 2026-08: *"they broke the MIDI
//! splitter three times out of three on the first pass"*, and every threshold in
//! `engine/src/smf_read.rs` is now a measurement of Mike's own files rather than
//! an assumption.
//!
//! ⚠⚠ **`audioTest/` is gitignored, so this SKIPS when it is absent.** That is
//! the deliberate trade: CI has no corpus (the files are commercial sample-pack
//! output and are not ours to redistribute), so the gates that run on every push
//! are the synthetic ones, and this is what a developer with the folder runs. A
//! skipped test that says why is better than either a test that cannot run or a
//! corpus in the repository.
//!
//! ▶ **Run it with the folder present**: `cargo test -p freally-midi-master-plugin
//! --test audio_extract -- --nocapture`. It prints what it measured, because the
//! numbers are the point — a future change to a threshold wants to be checked
//! against them.

use std::path::{Path, PathBuf};

use engine::pattern::{Lane, Part};
use freally_midi_master_plugin::audio::import::{decode_file_within, MAX_EXTRACT_SECONDS};
use freally_midi_master_plugin::extract::{self, AudioSplit};

/// Where Mike's own files live. Gitignored — see the module note.
fn corpus() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../audioTest");
    dir.is_dir().then_some(dir)
}

/// Read and split one file, or `None` if the corpus is not on this machine.
fn split_of(name: &str, session_bpm: f32) -> Option<AudioSplit> {
    let path = corpus()?.join(name);
    if !path.is_file() {
        eprintln!("skipping: {name} is not in audioTest/");
        return None;
    }
    let audio = decode_file_within(&path, MAX_EXTRACT_SECONDS)
        .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
    let split = extract::split(&audio, name, session_bpm, 4, 4)
        .unwrap_or_else(|error| panic!("{name} must split: {error}"));
    eprintln!(
        "{name}: {} BPM, vocal_left_alone={}, parts={:?}",
        split.bpm,
        split.vocal_left_alone,
        split
            .parts
            .iter()
            .map(|part| (part.part, part.reason, part.notes, part.pattern.bars))
            .collect::<Vec<_>>()
    );
    Some(split)
}

#[test]
fn the_starlight_loop_reads_its_own_tempo() {
    // ⛔ **Ground truth, and it is not a guess.** TASK-058E measured this file
    // when the MIDI half shipped: `Starlight.wav` is **15.966 s** and its own
    // `.mid` arranges to **8 bars at 120 BPM = 16.000 s**, which is 0.2%. So the
    // tempo this must read is 120, and the session is deliberately told 90 —
    // the tie-break may only choose between readings that fit the hits, and it
    // must not be able to drag the answer to a tempo the file does not have.
    let Some(split) = split_of("Starlight.wav", 90.0) else {
        return;
    };
    assert!(
        (split.bpm - 120.0).abs() < 3.0,
        "Starlight is 8 bars at 120; read as {}",
        split.bpm
    );
}

#[test]
fn the_starlight_mix_finds_the_drums_its_own_midi_carries() {
    // ⛔ `Starlight-Drums.mid` is 192 notes over **three** GM pitches — kick,
    // closed hat and snare. Those three lanes are what a reading of the mixed
    // audio has to find; anything it adds beyond them is a lane the record does
    // not play.
    let Some(split) = split_of("Starlight.wav", 120.0) else {
        return;
    };
    let drums = split
        .parts
        .iter()
        .find(|part| part.part == Part::Drums)
        .expect("a trap loop has drums in it");

    let lanes: Vec<Lane> = drums.pattern.lanes.iter().map(|track| track.lane).collect();
    for wanted in [Lane::Kick, Lane::ClosedHat] {
        assert!(lanes.contains(&wanted), "no {wanted:?} in {lanes:?}");
    }
    assert!(
        lanes
            .iter()
            .any(|lane| matches!(lane, Lane::Snare | Lane::OffSnare | Lane::GhostSnare)),
        "no snare of any kind in {lanes:?}"
    );
}

#[test]
fn the_bass_stem_reads_as_a_bassline_and_not_as_drums() {
    // ⛔⛔ **The one that proves the low-pass trick.** Mike asked whether the bass
    // could come from *"splitting the audio into all midi then taking the lowest
    // part"*; `StarlightBassOnly.wav` is that stem, and what must come back is a
    // Bass part rather than a page of kick hits.
    let Some(split) = split_of("StarlightBassOnly.wav", 120.0) else {
        return;
    };
    let bass = split
        .parts
        .iter()
        .find(|part| part.part == Part::Bass)
        .unwrap_or_else(|| panic!("no bass in {:?}", split.parts));
    assert!(bass.notes > 0);
}

#[test]
fn the_anthem_vocal_stem_is_left_alone() {
    // ⛔⛔ **The ground truth TASK-058G was written for.** Mike: *"and leave the
    // vocals alone."* `Cymatics - Anthem` ships as four stems and one of them is
    // literally named Vocal, so this is the one case where what the file *is* is
    // known independently of anything measured.
    //
    // ⚠ **Asserted as "not routed into Melody" rather than as
    // `vocal_left_alone`.** Either outcome satisfies the requirement: the line
    // may be rejected by the four discriminators, or it may be too breathy to
    // track at all. What must not happen is a vocal arriving in the Melody
    // generator.
    let Some(split) = split_of("Cymatics - Anthem - 144 BPM D Min Vocal.wav", 144.0) else {
        return;
    };
    assert!(
        !split.parts.iter().any(|part| part.part == Part::Melody),
        "a vocal was routed into Melody: {:?}",
        split.parts
    );
}

#[test]
fn the_anthem_keys_stem_reads_as_harmony() {
    // ⚠ The counterpart to the vocal: a stem that genuinely *is* a chord part has
    // to come back as one, or "leave the vocals alone" has been implemented as
    // "leave everything alone".
    let Some(split) = split_of("Cymatics - Anthem - 144 BPM D Min Keys.wav", 144.0) else {
        return;
    };
    assert!(
        split
            .parts
            .iter()
            .any(|part| matches!(part.part, Part::Chords | Part::Melody)),
        "the keys stem produced {:?}",
        split.parts
    );
}
