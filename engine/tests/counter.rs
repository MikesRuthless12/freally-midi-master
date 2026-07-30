//! The countermelody generator against the data that ships (TASK-036, FR-006).
//!
//! The unit tests in `engine/src/generators/counter.rs` prove each style in
//! isolation. These prove what only holds end to end: every authored style name
//! is one a reader knows, every parameter is read or excused, the gap rule holds
//! against real melodies rather than fixtures, and how many *different* counters
//! an artist can reach.
//!
//! Same four questions as `melody.rs`, asked through `common` so the parts
//! cannot drift into different standards.

use std::collections::BTreeSet;

use engine::context::{SessionContext, SessionOverrides};
use engine::generators::{chords, counter, drums, melody};
use engine::pattern::LaneTrack;
use serde_json::Value;

mod common;
use common::{
    authored_blocks, covers, leaf_paths, models_authoring, shape, variety_seeds, DISTINCT_SHARE,
    MIN_DISTINCT,
};

fn counter_models() -> Vec<(String, engine::StyleModel, Value)> {
    models_authoring("countermelody")
}

/// The counter a model generates for one seed, with the parts it answers.
fn counter_for(model: &engine::StyleModel, seed: u64) -> (LaneTrack, LaneTrack) {
    let ctx = SessionContext::from_model(model, &SessionOverrides::default(), seed);
    let harmony = chords::generate(model, &ctx, seed);
    let kit = drums::generate(model, &ctx, seed);
    let lead = melody::generate(model, &ctx, seed, &harmony, &kit);
    let track = counter::generate(model, &ctx, seed, &harmony, &lead);
    (track, lead)
}

/// Parameters under `countermelody` that no generator reads, with the reason.
///
/// Empty, and deliberately kept rather than deleted: every authored parameter is
/// currently read, and the pair of tests below is the same pair `melody.rs` and
/// `chords.rs` carry. The moment a parameter arrives that has no note-level
/// meaning, the place to say so already exists — which is the difference between
/// a gate and a habit.
const NOT_NOTE_LEVEL: &[(&str, &str)] = &[];

#[test]
fn every_authored_countermelody_parameter_is_read_or_documented_as_unread() {
    let known: Vec<&str> = counter::READ_KEYS
        .iter()
        .copied()
        .chain(NOT_NOTE_LEVEL.iter().map(|(key, _)| *key))
        .collect();

    // The model's own block and every mode's override of it — see
    // `authored_blocks`.
    let mut unread = Vec::new();
    for (id, model, _) in counter_models() {
        for (where_, block) in authored_blocks(&model, "countermelody") {
            for path in leaf_paths(&block, "") {
                if !known.iter().any(|prefix| covers(prefix, &path)) {
                    unread.push(format!("{id}: {where_}.{path}"));
                }
            }
        }
    }
    unread.sort();
    unread.dedup();

    assert!(
        unread.is_empty(),
        "these parameters are authored and nothing reads them. Either read them \
         in generators::counter and add them to READ_KEYS, or add them to \
         NOT_NOTE_LEVEL with the reason: {unread:#?}"
    );
}

#[test]
fn the_unread_list_does_not_outlive_the_keys_it_excuses() {
    let authored: BTreeSet<String> = counter_models()
        .iter()
        .flat_map(|(_, _, block)| leaf_paths(block, ""))
        .collect();

    let stale: Vec<&str> = NOT_NOTE_LEVEL
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !authored.iter().any(|path| covers(key, path)))
        .collect();

    assert!(
        stale.is_empty(),
        "nothing authors these any more, so the excuse should go: {stale:#?}"
    );
}

#[test]
fn every_authored_style_and_echo_offset_is_one_the_engine_can_read() {
    // An unknown style falls back to an octave echo and an unparseable
    // `echoOffset` falls back to an eighth, so in both cases the model's ask
    // silently does not happen. `"1/8"` is the case that found this: the note
    // value parser knew `"8th"` and `"16T"` and returned `None` for it.
    let mut unknown = Vec::new();
    for (id, _, block) in counter_models() {
        let names: Vec<String> = match block.get("styles") {
            Some(Value::String(one)) => vec![one.clone()],
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            // The weighted form: `{ "values": [...], "weights": [...] }`.
            Some(Value::Object(map)) => map
                .get("values")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        for name in names {
            if !counter::can_read_style(&name) {
                unknown.push(format!("{id}: style `{name}`"));
            }
        }

        if let Some(offset) = block.get("echoOffset").and_then(Value::as_str) {
            if engine::generators::grid::note_value_ticks(offset).is_none() {
                unknown.push(format!("{id}: echoOffset `{offset}`"));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "these names are authored and no reader knows them: {unknown:#?}"
    );
}

#[test]
fn every_shipped_counter_keeps_out_of_the_melodys_way() {
    // ⛔ FR-006's acceptance criterion against the real dataset: with
    // `fillGapsOnly`, at most 20% of the counter's onsets may coincide with the
    // melody's. Every shipped model authors it as true.
    for (id, model, block) in counter_models() {
        if !block
            .get("fillGapsOnly")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            continue;
        }

        for seed in 1..12u64 {
            let (track, lead) = counter_for(&model, seed);
            if track.notes.is_empty() {
                continue;
            }

            let clashing = track
                .notes
                .iter()
                .filter(|note| {
                    lead.notes
                        .iter()
                        .any(|source| source.start_tick.abs_diff(note.start_tick) < 240)
                })
                .count();
            let ratio = clashing as f64 / track.notes.len() as f64;

            assert!(
                ratio <= 0.2,
                "{id} seed {seed}: {ratio:.2} of the counter's onsets landed on the melody's"
            );
        }
    }
}

#[test]
fn a_counter_stays_inside_the_register_its_offset_puts_it_in() {
    for (id, model, block) in counter_models() {
        let offset = block
            .get("registerOffset")
            .and_then(Value::as_i64)
            .unwrap_or(12) as i32;
        let melody_register = model
            .blocks
            .get("melody")
            .and_then(|melody| melody.get("register"))
            .and_then(Value::as_array)
            .and_then(|values| Some((values[0].as_i64()? as i32, values[1].as_i64()? as i32)));

        let Some((low, high)) = melody_register else {
            continue;
        };
        let (low, high) = (
            (low + offset).clamp(0, 127) as u8,
            (high + offset).clamp(0, 127) as u8,
        );

        for seed in 1..12u64 {
            let (track, _) = counter_for(&model, seed);
            for note in &track.notes {
                assert!(
                    note.pitch >= low && note.pitch <= high,
                    "{id} seed {seed}: {} is outside {low}..={high}",
                    note.pitch
                );
            }
        }
    }
}

#[test]
fn an_artist_can_reach_a_different_counter_for_practically_every_seed() {
    // The same collision-rate gate the melody carries. A counter is derived from
    // a melody, so a narrow lead would show up here too — which is why the
    // exception is derived from the melody's grammar rather than restated.
    let seeds = variety_seeds();
    let mut failures = Vec::new();
    let mut report = Vec::new();

    for (id, model, _) in counter_models() {
        let mut seen = BTreeSet::new();
        for seed in 1..=seeds {
            seen.insert(shape(&counter_for(&model, seed).0));
        }

        let share = seen.len() as f64 / seeds as f64;
        let narrow = narrow_lead(&model);
        report.push(format!(
            "{id}: {}/{seeds} distinct ({share:.3}){}",
            seen.len(),
            if narrow {
                " — narrow lead by design"
            } else {
                ""
            }
        ));

        // The absolute floor, which admits no exceptions — see `MIN_DISTINCT`.
        if seen.len() < MIN_DISTINCT {
            failures.push(format!(
                "{id}: only {} distinct counters in {seeds} seeds, and every model \
                 must reach {MIN_DISTINCT}",
                seen.len()
            ));
        }

        if !narrow && share < DISTINCT_SHARE {
            failures.push(format!(
                "{id}: only {}/{seeds} counters were distinct ({share:.3}, needs \
                 {DISTINCT_SHARE}) — that model's grammar is saturating",
                seen.len()
            ));
        }
    }

    println!(
        "counter variety over {seeds} seeds:\n  {}",
        report.join("\n  ")
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Does this model's *melody* have a deliberately narrow grammar?
///
/// The counter echoes and answers the lead, so a two-note hypnotic lead caps the
/// counter's reachable shapes too. Read off the melody block for the same reason
/// `melody.rs` does it: the property is what is being excused, not a model id.
fn narrow_lead(model: &engine::StyleModel) -> bool {
    model
        .blocks
        .get("melody")
        .map(|melody| {
            melody
                .get("loopVerbatim")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && melody.get("motifPitchCount").is_some()
        })
        .unwrap_or(false)
}
