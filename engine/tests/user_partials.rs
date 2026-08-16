//! What the style editor writes, merged over every base it can be opened on.
//!
//! ⛔⛔ **The gap this file exists to close is a real one that shipped.** The
//! editor writes *partial* blocks — `drums.snare = { placement }`, and nothing
//! else — because `inherit::deep_merge` has an authored value replace the base's,
//! so a field written unasked is the base quietly overruled. That rule is right,
//! and it is also why a partial can be wrong in a way neither side can see
//! alone: the producer's fragment is valid, the base is valid, and the *merge*
//! is what the linter refuses.
//!
//! The Rolls control shipped exactly that. It wrote `rolls.vocab = { values }`;
//! `deep_merge` merges `vocab` key by key, so the producer's list arrived
//! against the **parent's** `weights`, and 566 of the 600 shipped models author
//! that pair. Ticking one subdivision over a base with five weights made
//! `check_weighted` refuse the save with `5 weights for 1 values`, naming a
//! field the producer never touched. Every style, every time.
//!
//! ⚠ **The e2e could not have caught it and still cannot.** `ipc-mock.ts` has no
//! linter, so `style-editor.spec.ts` round-trips the draft through a mock that
//! accepts anything — the mock-fixture-hides-the-bug failure this repo has
//! recorded before. The lint is Rust; the check belongs here.
//!
//! ⚠ **Kept in step by hand.** These fragments mirror `modelFrom` in
//! `src/components/StyleEditor/StyleEditor.tsx`. A control added there without a
//! case here is a control this file does not cover — the tradeoff `PLACEMENTS`
//! and its siblings already make one layer up.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::dataset::inherit::deep_merge;
use engine::dataset::validate;
use serde_json::{json, Value};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("data")
}

/// Every shipped model, resolved to the JSON the linter sees, read **once**.
///
/// ⚠ Resolved rather than raw: a partial is merged over what the producer
/// *opened*, which is the model with its whole inheritance chain already
/// applied. Linting the raw file would ask a different question and miss the
/// weights entirely — most models inherit `rolls.vocab` from `_defaults` rather
/// than authoring it, so the raw file has no `vocab` for a partial to collide
/// with and every case would pass.
///
/// ⚠ Loaded through `resolve_all` and serialised back, rather than through a
/// second hand-rolled reader: one answer to "what is in this model", for the
/// reason `rolls.rs` gives at its own `shipped()`.
fn resolved_models() -> BTreeMap<String, Value> {
    static MODELS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let scan = engine::dataset::files::scan(&data_dir()).expect("data/ must be readable");
            let (models, errors) = engine::dataset::registry_from(scan.files).resolve_all();
            assert!(errors.is_empty(), "the dataset must resolve: {errors:#?}");
            let out: BTreeMap<String, Value> = models
                .iter()
                .map(|(id, model)| {
                    (
                        id.clone(),
                        serde_json::to_value(model).expect("a resolved model must serialise"),
                    )
                })
                .collect();
            assert!(out.len() > 500, "only {} models resolved", out.len());
            out
        })
        .clone()
}

/// The fragments `modelFrom` writes, one per control, at their widest.
///
/// ⚠ The vocabularies are the editor's own lists, not the engine's — `ROLLS`
/// offers four subdivisions where `_defaults` authors five, and that mismatch is
/// the entire bug. Trimming these to match the base would test nothing.
fn editor_partials() -> Vec<(&'static str, Value)> {
    vec![
        // One ticked and all four ticked: the failure was length-dependent, so a
        // single case could have passed by luck on a base that authors four.
        (
            "rolls, one ticked",
            json!({ "drums": { "hihat": { "rolls": { "vocab": {
                "values": ["16T"], "weights": [1]
            } } } } }),
        ),
        (
            "rolls, all four ticked",
            json!({ "drums": { "hihat": { "rolls": { "vocab": {
                "values": ["16", "32", "16T", "8T"], "weights": [1, 1, 1, 1]
            } } } } }),
        ),
        (
            "snare placement",
            json!({ "drums": { "snare": { "placement": "downbeat_1_3" } } }),
        ),
        (
            "808 role and slide",
            json!({ "drums": { "bass808": { "role": "bassline", "slideProb": 0.4 } } }),
        ),
        (
            "progression families",
            json!({ "chords": { "progressionFamilies": [
                { "roman": ["i", "iv"], "weight": 1 }
            ] } }),
        ),
        // ⚠ Carries `weights` because `modelFrom` now does. Without them this
        // case failed against 599 of the 620 model files — which is how the
        // *pre-existing* scales defect was found, in a control that shipped long
        // before the four this file was written for.
        (
            "scales, one ticked",
            json!({ "session": { "scales": { "values": ["dorian"], "weights": [1] } } }),
        ),
        (
            "scales, three ticked",
            json!({ "session": { "scales": {
                "values": ["dorian", "natural_minor", "phrygian"], "weights": [1, 1, 1]
            } } }),
        ),
        (
            "melody density",
            json!({ "melody": { "densityPerBar": [2, 6] } }),
        ),
    ]
}

#[test]
fn every_control_in_the_style_editor_merges_clean_over_every_shipped_base() {
    // ⛔ The claim: a producer can open ANY style, touch ONE control, and save.
    // Not "a partial is valid JSON" — that the merged result passes the same
    // lint `models::save` runs before it writes a byte.
    let models = resolved_models();
    let partials = editor_partials();
    let mut checked = 0usize;

    for (id, base) in &models {
        for (what, partial) in &partials {
            let merged = deep_merge(base.clone(), partial);
            let findings = validate::lint(&merged);
            assert!(
                findings.is_empty(),
                "{id}: saving `{what}` over this style is refused — {}",
                findings
                    .iter()
                    .map(|f| format!("{}: {}", f.pointer, f.message))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            checked += 1;
        }
    }

    assert!(checked > 3_000, "only {checked} merges were checked");
}

#[test]
fn no_number_of_ticked_subdivisions_is_safe_without_weights() {
    // ⛔⛔ **The bug itself, held as a test so the fix cannot be undone by
    // "simplifying" the weights back out.** These are the fragments the editor
    // used to write. They must be refused — if this ever passes, `deep_merge`
    // has changed its mind about arrays and `modelFrom` needs rereading, not
    // this.
    //
    // ⛔ **Every tick count, because the hazard is length-dependent and a single
    // count hides most of it.** The bases author `rolls.vocab.weights` at lengths
    // 1 through 5, so a fragment of length *n* is accepted by exactly those bases
    // authoring *n* weights and refused by the rest. Ticking one subdivision is
    // refused by ~310 of 602 — the smallest number of the four, because 259 bases
    // happen to author a single weight — and ticking three by ~540. The first cut
    // of this test asserted `> 400` off one fragment and failed on the honest
    // measurement; the shape of the claim was wrong, not the number.
    // ⛔⛔ **Both checkbox groups, because BOTH had the defect.** The Rolls block
    // was written by copying the Scales block, so it inherited a bug that had
    // been shipping in `session.scales` since long before TASK-040U — 599 of the
    // 620 model files author `scales` with weights, so ticking a scale and
    // pressing Save has been refused for almost every base. Neither reviewer
    // found that one; this test did, on its first run.
    let models = resolved_models();
    let ticked = ["dorian", "phrygian", "natural_minor", "harmonic_minor"];

    // ⚠ A `match` on the group name rather than an array of boxed closures: the
    // closure version was `[(&str, &dyn Fn(&[&str]) -> Value); 2]`, which clippy
    // refuses as `type_complexity` at `-D warnings`.
    for group in ["rolls", "scales"] {
        for n in 1..=ticked.len() {
            let values = &ticked[..n];
            let fragment = match group {
                "rolls" => {
                    json!({ "drums": { "hihat": { "rolls": { "vocab": { "values": values } } } } })
                }
                _ => json!({ "session": { "scales": { "values": values } } }),
            };
            let refused = models
                .values()
                .filter(|base| !validate::lint(&deep_merge((*base).clone(), &fragment)).is_empty())
                .count();

            assert!(
                refused > 250,
                "ticking {n} {group} without weights was refused by only {refused} of \
                 {} bases — the merge hazard this test guards has changed shape",
                models.len()
            );
        }
    }
}
