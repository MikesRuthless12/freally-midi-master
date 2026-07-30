//! Generation and export — the commands that turn a style model into notes and
//! then into a file a DAW can open (TASK-026, FR-003 / FR-015).
//!
//! The pipeline is three steps and the order matters: generate on the grid,
//! humanize, then write. `engine/tests/golden.rs` pins exactly this sequence,
//! so a change here that is not a change there is a change nobody meant.

use engine::context::{SessionContext, SessionDefaults, SessionOverrides};
use engine::generators::{chords, drums};
use engine::humanize::humanize;
use engine::midi::pattern_to_smf;
use engine::pattern::{Part, Pattern, PPQ};
use engine::StyleModel;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::dataset::Dataset;
use crate::export::{write_session_file, ExportResult};

/// What the UI asks for when the user presses Generate (PRD § 4).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub style_id: String,
    #[serde(default)]
    pub part: Option<Part>,
    #[serde(default)]
    pub session: Option<SessionOverrides>,
    #[serde(default)]
    pub bars: Option<u16>,
    /// Decimal string, like `Pattern::seed` — a u64 does not survive a JSON
    /// number in the WebView (see `pattern.rs`).
    #[serde(default)]
    pub seed: Option<String>,
}

/// Build the pattern for a request. Separated from the command so it can be
/// tested without a Tauri app.
fn render(model: &StyleModel, request: &GenerateRequest) -> Result<Pattern, String> {
    let part = request.part.unwrap_or(Part::Drums);
    if !matches!(part, Part::Drums | Part::Chords) {
        // The rest of the melodic generators are still to come in Phase 2.
        // Saying so is better than returning an empty pattern the UI would
        // draw as a working result.
        return Err(format!(
            "the {part:?} generator is not implemented yet — only drums and chords are"
        ));
    }

    let seed = match &request.seed {
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| format!("`{text}` is not a seed"))?,
        // A fresh seed, and the one place in the engine's reach where system
        // entropy is allowed: the *choice* of seed is not part of generation,
        // and everything downstream of it is reproducible from the value.
        None => rand::random::<u64>(),
    };

    let mut overrides = request.session.clone().unwrap_or_default();
    if let Some(bars) = request.bars {
        overrides.bars = Some(bars);
    }

    let ctx = SessionContext::from_model(model, &overrides, seed);
    let mut lanes = match part {
        Part::Chords => vec![chords::generate(model, &ctx, seed).track],
        // Drums by construction — the match above rejected everything else.
        _ => drums::generate(model, &ctx, seed),
    };
    // Chords are humanized too: a pad swings with the track and breathes with
    // it. No lane authors `timingJitterMs` for chords, so the jitter term is
    // zero and only the swing and the velocity spread apply — which is what a
    // held chord should get and what a hat should not.
    humanize(&mut lanes, &ctx, seed);

    if lanes.iter().all(|lane| lane.notes.is_empty()) {
        // A model with no `chords` block generates nothing, and an empty grid
        // reads as a failed generation rather than as a style that does not
        // author harmony.
        return Err(format!(
            "{} has no {part:?} part authored — nothing to generate",
            model.name
        ));
    }

    Ok(Pattern {
        id: format!("{}-{seed}", model.id),
        part,
        artist_id: model.id.clone(),
        seed,
        bars: ctx.bars,
        bpm: ctx.bpm,
        time_sig_num: ctx.time_sig_num,
        time_sig_den: ctx.time_sig_den,
        key_root: ctx.key_root,
        scale: ctx.scale,
        lanes,
        ppq: PPQ,
    })
}

/// Generate one pattern.
#[tauri::command]
pub fn generate_pattern(
    request: GenerateRequest,
    dataset: State<'_, Dataset>,
) -> Result<Pattern, String> {
    let model = dataset.model(&request.style_id)?;
    render(&model, &request)
}

/// What a style asks for, for the session chips (FR-002).
///
/// Answered on selection rather than after a generation, because the chips have
/// to show what pressing Generate *would* do — and because the keep-or-adopt
/// prompt has to name the incoming artist's values while the outgoing artist's
/// pattern is still the one on screen.
#[tauri::command]
pub fn session_defaults(
    style_id: String,
    dataset: State<'_, Dataset>,
) -> Result<SessionDefaults, String> {
    Ok(SessionDefaults::of(&dataset.model(&style_id)?))
}

/// Where an export may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    /// One pattern as a type-0 SMF.
    Clip,
}

/// Write a pattern to the session directory as a `.mid`.
///
/// Takes the pattern rather than an id: the UI may have edited it, and
/// exporting a freshly regenerated one would silently discard those edits.
#[tauri::command]
pub fn export_midi(pattern: Pattern, mode: Option<ExportMode>) -> Result<ExportResult, String> {
    let _ = mode.unwrap_or(ExportMode::Clip);
    if pattern.note_count() == 0 {
        return Err("there is nothing in this pattern to export".into());
    }

    let bytes = pattern_to_smf(&pattern);
    let stem = format!("{}-{}", pattern.artist_id, pattern.seed);
    let path = write_session_file(&stem, "mid", &bytes)
        .map_err(|e| format!("could not write the MIDI file: {e}"))?;

    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::Lane;
    use std::path::Path;

    fn shipped(id: &str) -> StyleModel {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).unwrap();
        let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();
        models.get(id).cloned().unwrap()
    }

    fn request(style: &str, seed: Option<&str>) -> GenerateRequest {
        GenerateRequest {
            style_id: style.into(),
            part: None,
            session: None,
            bars: Some(4),
            seed: seed.map(str::to_owned),
        }
    }

    #[test]
    fn a_pattern_carries_the_session_its_model_asks_for() {
        let trap = shipped("trap");
        let pattern = render(&trap, &request("trap", Some("7"))).unwrap();

        assert_eq!(pattern.artist_id, "trap");
        assert_eq!(pattern.seed, 7);
        assert_eq!(pattern.bars, 4);
        assert_eq!(pattern.ppq, PPQ);
        // trap authors bpm 130–170 with a mode of 140.
        assert_eq!(pattern.bpm, 140.0);
        assert!(pattern.note_count() > 0);
        assert!(pattern.lanes.iter().any(|l| l.lane == Lane::Kick));
    }

    #[test]
    fn the_same_seed_reproduces_the_same_pattern() {
        // US-004: paste a seed, get the same beat.
        let trap = shipped("trap");
        let a = render(&trap, &request("trap", Some("2024"))).unwrap();
        let b = render(&trap, &request("trap", Some("2024"))).unwrap();
        let c = render(&trap, &request("trap", Some("2025"))).unwrap();
        assert_eq!(a, b);
        assert_ne!(a.lanes, c.lanes);
    }

    #[test]
    fn omitting_the_seed_produces_a_new_one_each_time() {
        let trap = shipped("trap");
        let a = render(&trap, &request("trap", None)).unwrap();
        let b = render(&trap, &request("trap", None)).unwrap();
        assert_ne!(a.seed, b.seed, "a fresh generation should be fresh");
        // ...and the seed it chose reproduces it.
        let again = render(&trap, &request("trap", Some(&a.seed.to_string()))).unwrap();
        assert_eq!(a, again);
    }

    #[test]
    fn an_override_beats_the_model_and_absence_does_not() {
        let trap = shipped("trap");
        let mut req = request("trap", Some("7"));
        req.session = Some(SessionOverrides {
            bpm: Some(88.0),
            key_root: Some(5),
            bars: None,
            ..Default::default()
        });

        let pinned = render(&trap, &req).unwrap();
        assert_eq!(pinned.bpm, 88.0, "the user's tempo wins");
        assert_eq!(pinned.key_root, 5);
        // `bars` was left absent in the overrides, so the request's own value
        // still applies rather than being reset to a default.
        assert_eq!(pinned.bars, 4);
    }

    #[test]
    fn a_nonsense_pinned_tempo_never_reaches_the_pattern() {
        // Nothing downstream would crash on it: the MIDI writer and the audio
        // transport each substitute a tempo of their own for a non-positive
        // one. That is the problem — the app would play and export at their
        // fallback while the chip, the grid header and the file all said 0.
        let trap = shipped("trap");
        let mut req = request("trap", Some("7"));
        req.session = Some(SessionOverrides {
            bpm: Some(0.0),
            ..Default::default()
        });
        let floored = render(&trap, &req).unwrap();
        assert!(floored.bpm >= 20.0, "got {}", floored.bpm);

        // The ceiling is Ableton Live's 999, not a bespoke number: the plugin
        // is a guest in someone else's project and has to be able to say
        // whatever that project says.
        req.session = Some(SessionOverrides {
            bpm: Some(100_000.0),
            ..Default::default()
        });
        let capped = render(&trap, &req).unwrap();
        assert!(capped.bpm <= 999.0, "got {}", capped.bpm);
        assert!(
            capped.bpm >= 900.0,
            "clamped to something arbitrary: {}",
            capped.bpm
        );
    }

    #[test]
    fn the_defaults_command_reports_what_the_model_asks_for() {
        // trap authors bpm 130–170 with a mode of 140, and a set of keys to
        // pick from. The chips show the tempo as a value and the keys as a
        // list, because only the second is chosen by the seed.
        let defaults = SessionDefaults::of(&shipped("trap"));
        assert_eq!(defaults.bpm, 140.0);
        assert!(
            !defaults.keys.is_empty(),
            "trap authors session.keys, so the chip has something to offer"
        );
        assert!(!defaults.scales.is_empty());
    }

    #[test]
    fn a_bad_seed_is_refused_rather_than_silently_replaced() {
        let trap = shipped("trap");
        let err = render(&trap, &request("trap", Some("not-a-number"))).unwrap_err();
        assert!(err.contains("not a seed"), "{err}");
    }

    #[test]
    fn a_generator_that_does_not_exist_says_so() {
        // Better than an empty pattern the UI would draw as a working result.
        let trap = shipped("trap");
        let mut req = request("trap", Some("1"));
        req.part = Some(Part::Melody);
        let err = render(&trap, &req).unwrap_err();
        assert!(err.contains("not implemented"), "{err}");
    }

    #[test]
    fn the_chords_part_generates_in_the_session_key() {
        // The whole point of one SessionContext: the chords land in the key
        // the pattern says it is in, not one of their own.
        let trap = shipped("trap");
        let mut req = request("trap", Some("7"));
        req.part = Some(Part::Chords);
        req.session = Some(SessionOverrides {
            key_root: Some(5),
            scale: Some(engine::pattern::Scale::NaturalMinor),
            ..Default::default()
        });

        let pattern = render(&trap, &req).unwrap();
        assert_eq!(pattern.part, Part::Chords);
        assert_eq!(pattern.key_root, 5);
        assert_eq!(pattern.lanes.len(), 1);
        assert_eq!(pattern.lanes[0].lane, Lane::Chords);
        assert!(pattern.note_count() > 0);
    }

    #[test]
    fn every_shipped_genre_generates_and_exports() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).unwrap();
        let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();

        for (id, model) in &models {
            if id.starts_with('_') {
                continue;
            }
            let pattern = render(model, &request(id, Some("11"))).unwrap();
            assert!(pattern.note_count() > 0, "{id} generated nothing");

            let result = export_midi(pattern, None).unwrap();
            let bytes = std::fs::read(&result.path).unwrap();
            assert_eq!(&bytes[0..4], b"MThd", "{id} did not export a MIDI file");
            assert_eq!(bytes.len() as u64, result.bytes);
            let _ = std::fs::remove_file(&result.path);
        }
    }

    #[test]
    fn an_empty_pattern_is_refused_rather_than_written() {
        let trap = shipped("trap");
        let mut pattern = render(&trap, &request("trap", Some("1"))).unwrap();
        pattern.lanes.clear();
        let err = export_midi(pattern, None).unwrap_err();
        assert!(err.contains("nothing in this pattern"), "{err}");
    }
}
