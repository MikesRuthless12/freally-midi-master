//! Inheritance: `extends` resolution by ordered deep-merge, with cycle
//! detection (PRD § 3 Relationships).
//!
//! A model inherits from zero or more parents. Precedence runs left to right
//! and then the child on top: given `"extends": ["rage", "dark-plugg"]`,
//! `dark-plugg` overrides `rage`, and the model itself overrides both. Genre
//! archetypes extend `_defaults`, so every model bottoms out at one place.

use std::collections::BTreeMap;

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
    let mut seen = BTreeMap::new();
    let mut visiting = Vec::new();
    let mut next_index = 0;
    linearize(id, registry, &mut visiting, &mut seen, 0, &mut next_index)?;

    // Deepest first, then by the order they were declared.
    //
    // A plain depth-first post-order is not enough. With `extends: [p1, p2]`
    // where p2 extends `base`, the post-order is [p1, base, p2] — so `base`,
    // reachable only THROUGH p2, ends up outranking p1's own explicit
    // declarations. Ordering by depth puts every ancestor below every model
    // that inherits from it, so a direct parent always beats a grandparent
    // reached via a sibling, and the left-to-right rule still decides between
    // two parents at the same depth.
    let mut order: Vec<&String> = seen.keys().filter(|k| *k != id).collect();
    order.sort_by_key(|k| {
        let (depth, index) = seen[*k];
        (std::cmp::Reverse(depth), index)
    });

    // Merge each ancestor's OWN body, lowest precedence first.
    //
    // Merging fully *resolved* parents instead is the subtle way to get this
    // wrong, and it was: with `"extends": ["p1", "p2"]` where both descend from
    // `_defaults`, resolved-p2 carries `_defaults`' values for everything p2
    // never mentions — so merging it over resolved-p1 let p2's *inherited
    // defaults* silently overwrite p1's *explicit* declarations. An artist
    // model with two parents got `_defaults`' straight timing and generic BPM
    // back, with nothing reported anywhere.
    let mut merged = Value::Object(Map::new());
    for ancestor in order {
        let model = registry
            .get(ancestor)
            .ok_or_else(|| DatasetError::UnknownParent(ancestor.clone()))?;
        merged = deep_merge(&merged, model);
    }

    let model = registry
        .get(id)
        .ok_or_else(|| DatasetError::UnknownParent(id.to_owned()))?;

    // The child last, on top of every ancestor. It is merged here rather than
    // as the tail of `order` because `order` is sorted by depth and the child
    // is not an ancestor of itself.
    let mut merged = deep_merge(&merged, model);

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

/// Append `id` and everything it inherits from to `order`, lowest precedence
/// first: every model lands after all of its own parents, and parents land
/// left to right.
///
/// The first placement is the one kept. A model reached twice through a diamond
/// therefore sits at its *deepest* position, below both of the paths that
/// reached it — which is what makes a shared ancestor lose to the parents that
/// extend it, rather than winning by arriving last.
fn linearize(
    id: &str,
    registry: &BTreeMap<String, Value>,
    visiting: &mut Vec<String>,
    seen: &mut BTreeMap<String, (usize, usize)>,
    depth: usize,
    next_index: &mut usize,
) -> Result<(), DatasetError> {
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

    // Keep the DEEPEST depth and the EARLIEST discovery. Reaching a model again
    // by a longer path must push it further down the stack, never pull it up.
    match seen.get_mut(id) {
        Some(entry) => {
            if depth <= entry.0 {
                // Already placed at least this deep; its ancestors are too.
                return Ok(());
            }
            entry.0 = depth;
        }
        None => {
            seen.insert(id.to_owned(), (depth, *next_index));
            *next_index += 1;
        }
    }

    visiting.push(id.to_owned());
    for parent in parents_of(model) {
        linearize(&parent, registry, visiting, seen, depth + 1, next_index)?;
    }
    visiting.pop();
    Ok(())
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
    fn a_parents_explicit_value_beats_a_siblings_inherited_default() {
        // The shape every artist model has: two parents that both extend
        // `_defaults`. p1 explicitly overrides swing; p2 says nothing about it.
        // p2 must NOT drag `_defaults`' 0.5 back over p1's explicit 0.62 just
        // by being listed second — an inherited default is not a declaration.
        let reg = registry(vec![
            (
                "_defaults",
                json!({ "id": "_defaults", "type": "genre", "name": "D",
                        "session": { "swing": { "amount": 0.5 } } }),
            ),
            (
                "p1",
                json!({ "id": "p1", "type": "genre", "name": "P1", "extends": ["_defaults"],
                        "session": { "swing": { "amount": 0.62 } } }),
            ),
            (
                "p2",
                json!({ "id": "p2", "type": "genre", "name": "P2", "extends": ["_defaults"] }),
            ),
            (
                "artist",
                json!({ "id": "artist", "type": "artist", "name": "A", "extends": ["p1", "p2"] }),
            ),
        ]);
        let out = resolve("artist", &reg).unwrap();
        assert_eq!(out["session"]["swing"]["amount"], json!(0.62));
    }

    #[test]
    fn between_two_explicit_parents_the_later_still_wins() {
        // The rule the module documents: precedence runs left to right. Only
        // *inherited* values lose to declaration order, never declared ones.
        let reg = registry(vec![
            (
                "_defaults",
                json!({ "id": "_defaults", "type": "genre", "name": "D", "v": 1 }),
            ),
            (
                "p1",
                json!({ "id": "p1", "type": "genre", "name": "P1", "extends": ["_defaults"], "v": 2 }),
            ),
            (
                "p2",
                json!({ "id": "p2", "type": "genre", "name": "P2", "extends": ["_defaults"], "v": 3 }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["p1", "p2"] }),
            ),
        ]);
        assert_eq!(resolve("c", &reg).unwrap()["v"], json!(3));
    }

    #[test]
    fn a_subclass_parent_outranks_its_own_superclass() {
        // `c` lists p2 first, but p2 extends p1 — a model must never be beaten
        // by something it inherits from, whatever order the child names them.
        let reg = registry(vec![
            (
                "p1",
                json!({ "id": "p1", "type": "genre", "name": "P1", "v": 1 }),
            ),
            (
                "p2",
                json!({ "id": "p2", "type": "genre", "name": "P2", "extends": ["p1"], "v": 2 }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["p2", "p1"] }),
            ),
        ]);
        assert_eq!(resolve("c", &reg).unwrap()["v"], json!(2));
    }

    #[test]
    fn a_grandparent_reached_through_a_sibling_loses_to_a_direct_parent() {
        // `c` extends [p1, p2]; only p2 extends `base`. A depth-first order
        // places base between p1 and p2, so base's value overwrote p1's own —
        // even though p1 is a direct parent and base is not.
        let reg = registry(vec![
            (
                "base",
                json!({ "id": "base", "type": "genre", "name": "Base", "v": 1 }),
            ),
            (
                "p1",
                json!({ "id": "p1", "type": "genre", "name": "P1", "v": 2 }),
            ),
            (
                "p2",
                json!({ "id": "p2", "type": "genre", "name": "P2", "extends": ["base"] }),
            ),
            (
                "c",
                json!({ "id": "c", "type": "artist", "name": "C", "extends": ["p1", "p2"] }),
            ),
        ]);
        assert_eq!(
            resolve("c", &reg).unwrap()["v"],
            json!(2),
            "p1 declares v explicitly; base is only reachable through p2"
        );
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
