//! Inheritance: `extends` resolution by ordered deep-merge, with cycle
//! detection (PRD § 3 Relationships).
//!
//! A model inherits from zero or more parents. Precedence runs left to right
//! and then the child on top: given `"extends": ["rage", "dark-plugg"]`,
//! `dark-plugg` overrides `rage`, and the model itself overrides both. Genre
//! archetypes extend `_defaults`, so every model bottoms out at one place.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::dataset::DatasetError;

/// Deep-merge `over` onto `base`, returning the result.
///
/// Objects merge key by key. **Arrays and scalars replace outright** — a child
/// listing two progression families means exactly those two, not those two
/// appended to its parent's five. Appending would make it impossible for a
/// model to narrow what it inherits, which is most of what artist models do.
pub fn deep_merge(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out: Map<String, Value> = b.clone();
            for (k, v_over) in o {
                let merged = match b.get(k) {
                    Some(v_base) => deep_merge(v_base, v_over),
                    None => v_over.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        // Anything else: the overriding value wins whole.
        _ => over.clone(),
    }
}

fn parents_of(model: &Value) -> Vec<String> {
    model
        .get("extends")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve `id` against `registry`, merging its ancestors in precedence order.
///
/// The returned value keeps the child's own `id`, `type` and `name` — those
/// identify the model and must never be inherited from a parent.
pub fn resolve(id: &str, registry: &BTreeMap<String, Value>) -> Result<Value, DatasetError> {
    let mut visiting = Vec::new();
    let mut done = BTreeSet::new();
    resolve_inner(id, registry, &mut visiting, &mut done)
}

fn resolve_inner(
    id: &str,
    registry: &BTreeMap<String, Value>,
    visiting: &mut Vec<String>,
    done: &mut BTreeSet<String>,
) -> Result<Value, DatasetError> {
    if visiting.iter().any(|v| v == id) {
        // Report the loop as it was walked, so the author can see which edge to
        // cut rather than just being told one exists.
        let mut chain = visiting.clone();
        chain.push(id.to_owned());
        let start = chain.iter().position(|v| v == id).unwrap_or(0);
        return Err(DatasetError::Cycle(chain[start..].join(" -> ")));
    }

    let model = registry
        .get(id)
        .ok_or_else(|| DatasetError::UnknownParent(id.to_owned()))?;

    visiting.push(id.to_owned());
    let mut acc = Value::Object(Map::new());
    for parent in parents_of(model) {
        let resolved = resolve_inner(&parent, registry, visiting, done)?;
        acc = deep_merge(&acc, &resolved);
    }
    visiting.pop();
    done.insert(id.to_owned());

    let mut merged = deep_merge(&acc, model);

    // Identity is the child's, never an ancestor's — a merge must not be able
    // to rename a model or change its type.
    if let (Value::Object(out), Value::Object(own)) = (&mut merged, model) {
        for key in ["id", "type", "name"] {
            match own.get(key) {
                Some(v) => {
                    out.insert(key.to_owned(), v.clone());
                }
                None => {
                    out.remove(key);
                }
            }
        }
        // `extends` describes this model's own edges; a resolved model has none
        // left to walk.
        out.remove("extends");
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry(entries: Vec<(&str, Value)>) -> BTreeMap<String, Value> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect()
    }

    #[test]
    fn objects_merge_key_by_key() {
        let base = json!({ "session": { "bpm": { "min": 130 }, "halfTime": true } });
        let over = json!({ "session": { "bpm": { "min": 140 } } });
        assert_eq!(
            deep_merge(&base, &over),
            json!({ "session": { "bpm": { "min": 140 }, "halfTime": true } })
        );
    }

    #[test]
    fn arrays_replace_rather_than_append() {
        // A child narrowing its inherited list is the common case; appending
        // would make narrowing impossible.
        let base = json!({ "genres": ["trap", "drill", "rage"] });
        let over = json!({ "genres": ["rage"] });
        assert_eq!(deep_merge(&base, &over), json!({ "genres": ["rage"] }));
    }

    #[test]
    fn a_child_can_override_with_null() {
        let base = json!({ "drums": { "percs": { "lanes": ["rim"] } } });
        let over = json!({ "drums": { "percs": null } });
        assert_eq!(
            deep_merge(&base, &over),
            json!({ "drums": { "percs": null } })
        );
    }

    #[test]
    fn parents_apply_left_to_right_then_the_child() {
        let reg = registry(vec![
            (
                "a",
                json!({ "id": "a", "type": "genre", "name": "A", "v": 1, "onlyA": true }),
            ),
            (
                "b",
                json!({ "id": "b", "type": "genre", "name": "B", "v": 2 }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["a", "b"] }),
            ),
        ]);
        let out = resolve("c", &reg).unwrap();
        // b beats a.
        assert_eq!(out["v"], json!(2));
        // Anything only a declared still comes through.
        assert_eq!(out["onlyA"], json!(true));
    }

    #[test]
    fn the_child_beats_every_parent() {
        let reg = registry(vec![
            (
                "p",
                json!({ "id": "p", "type": "genre", "name": "P", "v": 1 }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["p"], "v": 99 }),
            ),
        ]);
        assert_eq!(resolve("c", &reg).unwrap()["v"], json!(99));
    }

    #[test]
    fn identity_is_never_inherited() {
        let reg = registry(vec![
            ("p", json!({ "id": "p", "type": "genre", "name": "Parent" })),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "Child", "extends": ["p"] }),
            ),
        ]);
        let out = resolve("c", &reg).unwrap();
        assert_eq!(out["id"], json!("c"));
        assert_eq!(out["name"], json!("Child"));
        assert_eq!(out["type"], json!("artist"));
    }

    #[test]
    fn a_resolved_model_has_no_extends_left() {
        let reg = registry(vec![
            ("p", json!({ "id": "p", "type": "genre", "name": "P" })),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["p"] }),
            ),
        ]);
        assert!(resolve("c", &reg).unwrap().get("extends").is_none());
    }

    #[test]
    fn grandparents_resolve_through() {
        let reg = registry(vec![
            (
                "_defaults",
                json!({ "id": "_defaults", "type": "genre", "name": "D", "deep": 1 }),
            ),
            (
                "g",
                json!({ "id": "g", "type": "genre", "name": "G", "extends": ["_defaults"] }),
            ),
            (
                "a",
                json!({ "id": "a", "type": "artist", "name": "A", "extends": ["g"] }),
            ),
        ]);
        assert_eq!(resolve("a", &reg).unwrap()["deep"], json!(1));
    }

    #[test]
    fn a_direct_cycle_is_rejected_with_the_path() {
        let reg = registry(vec![
            (
                "a",
                json!({ "id": "a", "type": "genre", "name": "A", "extends": ["b"] }),
            ),
            (
                "b",
                json!({ "id": "b", "type": "genre", "name": "B", "extends": ["a"] }),
            ),
        ]);
        match resolve("a", &reg) {
            Err(DatasetError::Cycle(path)) => {
                assert!(path.contains("a"), "path should name the loop: {path}");
                assert!(path.contains("->"), "path should be a chain: {path}");
            }
            other => panic!("expected a cycle error, got {other:?}"),
        }
    }

    #[test]
    fn a_self_cycle_is_rejected() {
        let reg = registry(vec![(
            "a",
            json!({ "id": "a", "type": "genre", "name": "A", "extends": ["a"] }),
        )]);
        assert!(matches!(resolve("a", &reg), Err(DatasetError::Cycle(_))));
    }

    #[test]
    fn a_long_cycle_is_rejected() {
        let reg = registry(vec![
            (
                "a",
                json!({ "id": "a", "type": "genre", "name": "A", "extends": ["b"] }),
            ),
            (
                "b",
                json!({ "id": "b", "type": "genre", "name": "B", "extends": ["c"] }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "genre", "name": "C", "extends": ["a"] }),
            ),
        ]);
        assert!(matches!(resolve("a", &reg), Err(DatasetError::Cycle(_))));
    }

    #[test]
    fn a_diamond_is_not_a_cycle() {
        // a -> b, c; b -> d; c -> d. `d` is visited twice but never re-entered
        // while on the stack, so this must resolve rather than false-positive.
        let reg = registry(vec![
            (
                "d",
                json!({ "id": "d", "type": "genre", "name": "D", "base": true }),
            ),
            (
                "b",
                json!({ "id": "b", "type": "genre", "name": "B", "extends": ["d"] }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "genre", "name": "C", "extends": ["d"] }),
            ),
            (
                "a",
                json!({ "id": "a", "type": "artist", "name": "A", "extends": ["b", "c"] }),
            ),
        ]);
        assert_eq!(resolve("a", &reg).unwrap()["base"], json!(true));
    }

    #[test]
    fn an_unknown_parent_is_named_in_the_error() {
        let reg = registry(vec![(
            "a",
            json!({ "id": "a", "type": "artist", "name": "A", "extends": ["nope"] }),
        )]);
        match resolve("a", &reg) {
            Err(DatasetError::UnknownParent(id)) => assert_eq!(id, "nope"),
            other => panic!("expected UnknownParent, got {other:?}"),
        }
    }
}
