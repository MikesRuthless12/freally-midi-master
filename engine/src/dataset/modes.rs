//! Modes: one artist, more than one kind of record (TASK-040V).
//!
//! Without this a model is a single parameter set, so "Metro Boomin" means one
//! *kind* of beat. The seed varies which notes come out; it cannot vary what
//! kind of thing is being written, and a dark 140 trap beat and a melodic 128
//! one are both Metro with no seed between them.
//!
//! A mode is a **named partial override of its own model** — a name, a sampling
//! weight, and any block it wants to change. The override is merged into the
//! model *before* any generator runs, which is the whole design:
//!
//! > **Every generator gets modes for free and none of them knows modes exist.**
//! > `drums`, `chords`, `melody`, `counter` and `bass` all read their blocks off
//! > the model they are handed. Hand them a mode-applied model and they generate
//! > that mode. There is no per-generator code, no flag to thread through five
//! > call sites, and no way for one part to honour a mode while another ignores
//! > it — which is what a per-generator implementation would eventually do.
//!
//! ## Modes are inherited, and that is what makes moods authentic
//!
//! `modes` is an ordinary block, so [`crate::dataset::inherit`] already carries
//! it down `extends` — and because arrays replace rather than append, an artist
//! that authors none inherits its genre's list whole, while one that authors its
//! own replaces it entirely. Two consequences worth stating, because they are
//! requirements rather than side effects:
//!
//! - **An artist can only offer moods from its own lineage.** There is no path
//!   by which one artist's modes reach another, or a genre's reach a different
//!   genre. That is TASK-040U's "an artist's moods are only the ones that artist
//!   actually does", and it is structural rather than checked.
//! - **A mode cannot change what genre a model is.** It overrides parameters
//!   inside the model; it does not touch `extends`.

use rand::Rng;
use serde_json::Value;

use crate::dataset::inherit::deep_merge;
use crate::dataset::schema::StyleModel;
use crate::rng;

/// The block a model's modes are authored under.
const MODES: &str = "modes";

/// A mode entry's own fields, which are not overrides to merge.
///
/// ⛔ `name` in particular: a model has a `name` too, and merging the mode's
/// over it would rename the artist to "dark".
const META: &[&str] = &["name", "weight"];

/// A mode a model offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub name: String,
    /// Sampling weight. Zero means "only if pinned".
    pub weight: f64,
}

/// Every mode this model offers, in authored order.
///
/// Authored order rather than sorted, because the weights are authored against
/// it and a producer reading a list expects the order the model states.
pub fn modes_of(model: &StyleModel) -> Vec<Mode> {
    model
        .blocks
        .get(MODES)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let name = entry.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    Some(Mode {
                        name: name.to_owned(),
                        weight: entry
                            .get("weight")
                            .and_then(Value::as_f64)
                            .unwrap_or(1.0)
                            .max(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Does this model offer a mode by this name?
pub fn offers(model: &StyleModel, name: &str) -> bool {
    modes_of(model).iter().any(|mode| mode.name == name)
}

/// Pick a mode for this seed, weighted. `None` when the model offers none.
///
/// Its own seeded stream, so choosing a mode cannot move the notes of a part
/// that was not re-rolled — the same rule every other domain follows.
pub fn pick(model: &StyleModel, seed: u64) -> Option<String> {
    let modes = modes_of(model);
    let first = modes.first()?;

    let total: f64 = modes.iter().map(|mode| mode.weight).sum();
    if total <= 0.0 {
        // Every weight zero is an authoring mistake rather than "never pick a
        // mode"; falling back to the first keeps the model generatable and
        // `every_shipped_model_with_modes_can_pick_one` reports it.
        return Some(first.name.clone());
    }

    let mut point = rng::stream(seed, "model/mode").random_range(0.0..total);
    for mode in &modes {
        point -= mode.weight;
        if point < 0.0 {
            return Some(mode.name.clone());
        }
    }
    modes.last().map(|mode| mode.name.clone())
}

/// The model a generator should read when `name` is the chosen mode.
///
/// The mode's blocks are deep-merged onto the model, so a mode that changes one
/// field of `melody` leaves the rest of `melody` alone. Arrays replace, which is
/// what lets a mode *narrow* a progression family list rather than only add to
/// it — the same rule `extends` follows.
pub fn apply(model: &StyleModel, name: &str) -> Result<StyleModel, String> {
    let entry = model
        .blocks
        .get(MODES)
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
                .cloned()
        })
        .ok_or_else(|| {
            let offered: Vec<String> = modes_of(model).into_iter().map(|m| m.name).collect();
            if offered.is_empty() {
                format!("`{}` offers no modes, so `{name}` cannot be one", model.id)
            } else {
                format!(
                    "`{}` has no mode `{name}` — it offers {}",
                    model.id,
                    offered.join(", ")
                )
            }
        })?;

    // Only the override half of the entry. `name` and `weight` describe the
    // mode; everything else is what it changes.
    let Value::Object(fields) = entry else {
        return Err(format!("mode `{name}` is not an object"));
    };
    let overrides: Value = Value::Object(
        fields
            .into_iter()
            .filter(|(key, _)| !META.contains(&key.as_str()))
            .collect(),
    );

    let base = serde_json::to_value(model).map_err(|error| error.to_string())?;
    let mut merged = deep_merge(base, &overrides);
    // ⛔ **The third door the companion rule has to be at** (TASK-172). A mode is
    // a child of the model in every sense that matters — its arrays replace, so
    // a mode stating its own `kick.anchors` must not keep the model's
    // `secondaryAnchor` beside them. Nine mode blocks author `anchors`, and four
    // of them sit on a kick that declares a secondary: `blake-shelton`'s
    // *traditional* mode reproduced the exact case TASK-172 was opened for.
    // ⚠ `inherit` owns the rule; this calls it rather than repeating it, because
    // a rule installed at one of three doors is the failure this repo has
    // already recorded four times in one branch.
    crate::dataset::inherit::drop_orphaned_companions(&mut merged, &overrides);

    serde_json::from_value(merged)
        .map_err(|error| format!("mode `{name}` produced a model that will not parse: {error}"))
}

/// The model to generate from, and which mode it came out as.
///
/// `pinned` is the user's choice; `None` means "let the seed pick", which is
/// what walks an artist's whole range as Generate is pressed repeatedly. A
/// pinned mode the model does not offer is an **error rather than a fallback**:
/// silently generating a different mood than the one asked for is the readout
/// lying, and this project has a rule about that.
pub fn resolve(
    model: &StyleModel,
    seed: u64,
    pinned: Option<&str>,
) -> Result<(StyleModel, Option<String>), String> {
    let chosen = match pinned {
        Some(name) => Some(name.to_owned()),
        None => pick(model, seed),
    };

    match chosen {
        Some(name) => {
            let applied = apply(model, &name)?;
            Ok((applied, Some(name)))
        }
        None => Ok((model.clone(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(extra: Value) -> StyleModel {
        let mut value = json!({
            "id": "test",
            "type": "genre",
            "name": "Test",
            "melody": { "densityPerBar": [4, 6], "register": [60, 84] },
            "drums": { "kick": { "densityPerBar": 3 } }
        });
        if let (Value::Object(base), Value::Object(more)) = (&mut value, extra) {
            for (key, item) in more {
                base.insert(key, item);
            }
        }
        serde_json::from_value(value).expect("the test model must parse")
    }

    fn with_modes() -> StyleModel {
        model(json!({
            "modes": [
                { "name": "dark", "weight": 3, "melody": { "densityPerBar": [2, 3] } },
                { "name": "melodic", "weight": 1, "melody": { "register": [72, 96] } }
            ]
        }))
    }

    #[test]
    fn a_model_with_no_modes_offers_none_and_resolves_to_itself() {
        let plain = model(json!({}));
        assert!(modes_of(&plain).is_empty());
        assert_eq!(pick(&plain, 1), None);

        let (resolved, chosen) = resolve(&plain, 1, None).unwrap();
        assert_eq!(chosen, None);
        assert_eq!(resolved, plain);
    }

    #[test]
    fn modes_are_listed_in_the_order_they_are_authored() {
        let names: Vec<String> = modes_of(&with_modes())
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["dark", "melodic"]);
    }

    #[test]
    fn applying_a_mode_overrides_only_what_it_mentions() {
        // The whole point of a *partial* override: `dark` changes the melody's
        // density and must leave its register and the drums alone.
        let dark = apply(&with_modes(), "dark").unwrap();

        assert_eq!(dark.blocks["melody"]["densityPerBar"], json!([2, 3]));
        assert_eq!(
            dark.blocks["melody"]["register"],
            json!([60, 84]),
            "the register was not mentioned and must survive"
        );
        assert_eq!(dark.blocks["drums"]["kick"]["densityPerBar"], json!(3));
    }

    #[test]
    fn a_mode_does_not_rename_the_artist() {
        // ⛔ A mode entry has a `name` and so does a model. Merging the first
        // over the second would make every dark trap beat come from an artist
        // called "dark".
        let dark = apply(&with_modes(), "dark").unwrap();
        assert_eq!(dark.name, "Test");
        assert_eq!(dark.id, "test");
        assert!(
            !dark.blocks.contains_key("weight"),
            "a mode's weight is not a block"
        );
    }

    #[test]
    fn a_mode_the_model_does_not_offer_is_an_error_naming_the_ones_it_does() {
        let error = apply(&with_modes(), "uptempo").unwrap_err();
        assert!(error.contains("uptempo"), "{error}");
        assert!(error.contains("dark"), "{error}");
        assert!(error.contains("melodic"), "{error}");

        // And through `resolve`, because that is what the bridge calls.
        assert!(resolve(&with_modes(), 1, Some("uptempo")).is_err());
    }

    #[test]
    fn a_pinned_mode_is_the_one_that_is_used_whatever_the_seed_says() {
        for seed in 1..30u64 {
            let (_, chosen) = resolve(&with_modes(), seed, Some("melodic")).unwrap();
            assert_eq!(chosen.as_deref(), Some("melodic"));
        }
    }

    #[test]
    fn the_seed_picks_a_mode_and_the_same_seed_picks_the_same_one() {
        let model = with_modes();
        for seed in 1..30u64 {
            assert_eq!(pick(&model, seed), pick(&model, seed));
        }
    }

    #[test]
    fn pressing_generate_repeatedly_walks_every_mode_the_model_offers() {
        // The reason `Any` exists: over a run of seeds an artist's whole range
        // should be reachable, not just its heaviest mode.
        let model = with_modes();
        let mut seen: Vec<String> = (1..200u64).filter_map(|seed| pick(&model, seed)).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen, vec!["dark", "melodic"]);
    }

    #[test]
    fn weights_are_honoured_rather_than_ignored() {
        // `dark` is weighted 3 to `melodic`'s 1, so it must dominate — otherwise
        // the weights are decoration and an artist's typical sound is not its
        // typical sound.
        let model = with_modes();
        let dark = (1..400u64)
            .filter(|seed| pick(&model, *seed).as_deref() == Some("dark"))
            .count();
        assert!(
            (240..=360).contains(&dark),
            "dark came up {dark} times in 399, expected about 300"
        );
    }

    #[test]
    fn every_weight_zero_still_generates() {
        let model = model(json!({
            "modes": [{ "name": "only", "weight": 0 }]
        }));
        assert_eq!(pick(&model, 7).as_deref(), Some("only"));
    }

    #[test]
    fn a_mode_may_narrow_a_list_rather_than_only_add_to_it() {
        // Arrays replace, as they do through `extends`. A mode that names one
        // progression family means that one, not that one plus the parent's.
        let model = model(json!({
            "chords": { "progressionFamilies": [{ "roman": ["i", "VI"], "weight": 1 }] },
            "modes": [{
                "name": "minimal",
                "chords": { "progressionFamilies": [{ "roman": ["i"], "weight": 1 }] }
            }]
        }));

        let minimal = apply(&model, "minimal").unwrap();
        let families = minimal.blocks["chords"]["progressionFamilies"]
            .as_array()
            .unwrap();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0]["roman"], json!(["i"]));
    }

    #[test]
    fn a_mode_can_change_the_session_a_model_asks_for() {
        // "uptempo" is a tempo statement, and the tempo lives in `session`,
        // which is a typed field rather than a block — so this proves the merge
        // survives the round trip through `StyleModel` rather than only touching
        // the untyped half.
        let model = model(json!({
            "session": { "bpm": { "min": 130, "max": 150, "mode": 140 } },
            "modes": [{
                "name": "uptempo",
                "session": { "bpm": { "min": 150, "max": 170, "mode": 160 } }
            }]
        }));

        let fast = apply(&model, "uptempo").unwrap();
        let bpm = fast.session.as_ref().and_then(|s| s.bpm.clone()).unwrap();
        assert_eq!(bpm.mode, Some(160.0));
    }
}
