//! `deep_merge` was changed from borrowing its base to consuming it, for a
//! large startup win (330 ms → 219 ms for 1,000 models). That is shared
//! inheritance code every model in the dataset passes through, so "it still
//! behaves the same" is a claim that has to be checked rather than asserted.
//!
//! This compares the shipped implementation against a deliberately naive
//! reference — the obvious recursive clone-and-overwrite — over a few thousand
//! generated JSON shapes.

use engine::dataset::inherit::deep_merge;
use engine::rng::root_stream;
use rand::Rng;
use serde_json::{json, Map, Value};

/// The obvious implementation, written for clarity and nothing else.
fn reference(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out: Map<String, Value> = b.clone();
            for (k, v_over) in o {
                let merged = match b.get(k) {
                    Some(v_base) => reference(v_base, v_over),
                    None => v_over.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => over.clone(),
    }
}

/// A random JSON value, biased toward the shapes real models have: nested
/// objects, arrays, numbers, and the odd explicit null.
fn any_json(rng: &mut impl Rng, depth: u8) -> Value {
    match rng.random_range(0..if depth == 0 { 5 } else { 7 }) {
        0 => Value::Null,
        1 => json!(rng.random_range(-100i32..100)),
        2 => json!(rng.random_bool(0.5)),
        3 => json!(["a", "b", "c"][rng.random_range(0..3)]),
        4 => json!([rng.random_range(0..8), rng.random_range(0..8)]),
        5 => {
            let mut map = Map::new();
            for _ in 0..rng.random_range(0..4) {
                let key = ["x", "y", "z", "w", "kick", "snare"][rng.random_range(0..6)];
                map.insert(key.to_owned(), any_json(rng, depth - 1));
            }
            Value::Object(map)
        }
        _ => Value::Array(
            (0..rng.random_range(0..3))
                .map(|_| any_json(rng, depth - 1))
                .collect(),
        ),
    }
}

#[test]
fn the_in_place_merge_matches_the_naive_one_on_generated_shapes() {
    let mut rng = root_stream(20_260_724);
    for case in 0..4000 {
        let base = any_json(&mut rng, 3);
        let over = any_json(&mut rng, 3);
        assert_eq!(
            deep_merge(base.clone(), &over),
            reference(&base, &over),
            "case {case} diverged\nbase: {base}\nover: {over}"
        );
    }
}

#[test]
fn the_cases_the_dataset_actually_relies_on() {
    // Spelled out as well as generated, so a failure names the rule it broke.
    let cases = [
        // Objects merge key by key.
        (json!({"a": {"b": 1, "c": 2}}), json!({"a": {"b": 9}})),
        // Arrays replace outright — a child narrowing what it inherits.
        (json!({"g": ["trap", "drill"]}), json!({"g": ["rage"]})),
        // A child switching a block off with an explicit null.
        (
            json!({"bass808": {"role": "bassline"}}),
            json!({"bass808": null}),
        ),
        // A key only the base has survives.
        (json!({"only_base": 1}), json!({"other": 2})),
        // A key only the override has arrives.
        (json!({}), json!({"new": {"deep": [1, 2]}})),
        // Scalar over object and object over scalar.
        (json!({"x": {"deep": 1}}), json!({"x": 5})),
        (json!({"x": 5}), json!({"x": {"deep": 1}})),
        // Null in the base being overwritten.
        (json!({"x": null}), json!({"x": {"deep": 1}})),
        // Non-object roots.
        (json!([1, 2]), json!({"a": 1})),
        (json!({"a": 1}), json!([1, 2])),
        (json!(null), json!({"a": 1})),
    ];

    for (base, over) in cases {
        assert_eq!(
            deep_merge(base.clone(), &over),
            reference(&base, &over),
            "diverged on base {base} / over {over}"
        );
    }
}

#[test]
fn key_order_survives_the_in_place_merge() {
    // The in-place version mutates through `get_mut` rather than
    // remove-then-insert precisely so an existing key keeps its position. That
    // is invisible with serde_json's default BTreeMap, but it is the reason the
    // code is written that way, and it is what would break first if the crate
    // ever gained `preserve_order`.
    let base = json!({"zebra": 1, "alpha": {"inner": 1}, "mid": 3});
    let over = json!({"alpha": {"inner": 2}, "new": 4});

    let merged = deep_merge(base.clone(), &over);
    let expected = reference(&base, &over);

    let keys = |v: &Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&merged), keys(&expected));
    assert_eq!(merged["alpha"]["inner"], json!(2));
}
