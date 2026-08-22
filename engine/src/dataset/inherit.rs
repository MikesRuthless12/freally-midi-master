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
///
/// `base` is consumed rather than borrowed. Cloning it instead cost a full deep
/// copy of the accumulated model at every step of every chain — with 1,000
/// models over three ancestors that was over half of the whole startup load
/// (FR-001's 300 ms budget). Only what `over` contributes is cloned now, which
/// is the part genuinely being copied out of the registry.
pub fn deep_merge(base: Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(mut b), Value::Object(o)) => {
            for (k, v_over) in o {
                match b.get_mut(k) {
                    // Take the existing value out of its slot and put the merge
                    // back in the same place, rather than remove-then-insert:
                    // with `preserve_order` that would move the key to the end.
                    Some(slot) => {
                        let taken = std::mem::replace(slot, Value::Null);
                        *slot = deep_merge(taken, v_over);
                    }
                    None => {
                        b.insert(k.clone(), v_over.clone());
                    }
                }
            }
            Value::Object(b)
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
    resolve_over(id, None, registry)
}

/// Resolve `id` as if its `extends` named `base` instead of what it authors
/// (TASK-158C).
///
/// ⛔⛔ **"Drake, but in R&B."** 529 of the 534 artist and producer models
/// declare `relatedGenres` naming a genre they do **not** `extend`, and
/// `extends` is a single lane in all 534 — so `cross-filter.ts` has always
/// listed 2Pac under boom-bap while Generate answered g-funk. This is the
/// function that makes the roster's claim true: the artist's own blocks,
/// resolved over a *different* foundation.
///
/// ⛔ **NOT multi-`extends`, and that is a decision rather than an
/// implementation detail.** Drake's own `notes` say the OVO ballad, dancehall
/// and club modes *"are separate lanes and are not what this model
/// generates"* — blending two bases at once gives mud, not two modes. One base
/// at a time is what a producer actually means.
///
/// ⛔ **And it does NOT clone the registry.** The obvious implementation —
/// rewrite the child's `extends` and re-resolve — needs a mutable copy of 590
/// JSON documents per generation, on the editor thread. Instead only the
/// *walk* changes: the child's own body is untouched and its ancestors are
/// linearized from `base` rather than from `parents_of(child)`. The cost is one
/// linearize and a handful of merges, which is what resolving **one** model has
/// always cost.
///
/// ⚠ `None` is exactly [`resolve`] — the same walk, the same order, the same
/// output. `a_base_of_none_is_the_model_as_authored` holds that byte for byte,
/// because every model in the product goes through this path now.
pub fn resolve_over(
    id: &str,
    base: Option<&str>,
    registry: &BTreeMap<String, Value>,
) -> Result<Value, DatasetError> {
    let order = ancestor_order(id, base, registry)?;
    let merged = merge_ancestors(&order, registry)?;
    child_over(id, merged, registry)
}

/// `id`'s ancestors, lowest precedence first — the list the merge walks.
///
/// Split out of [`resolve_over`] so [`resolve_memoized`] can use it as a cache
/// key. It is exactly the ordering [`resolve_over`] always did, comments and
/// all; nothing about the walk changed when it moved.
fn ancestor_order(
    id: &str,
    base: Option<&str>,
    registry: &BTreeMap<String, Value>,
) -> Result<Vec<String>, DatasetError> {
    let mut seen = BTreeMap::new();
    let mut visiting = Vec::new();
    let mut next_index = 0;

    match base {
        None => linearize(id, registry, &mut visiting, &mut seen, 0, &mut next_index)?,
        Some(base) => {
            // The root's own bookkeeping, spelled out here because this is the
            // one call whose parents are not the ones it authors.
            //
            // ⚠ **Fetched first**, so an unknown `id` is the same error it is on
            // the ordinary path rather than a confusing one about the base.
            registry
                .get(id)
                .ok_or_else(|| DatasetError::UnknownParent(id.to_owned()))?;
            seen.insert(id.to_owned(), (0, 0));
            next_index = 1;
            // ⚠ On the stack before the walk, so `base == id` — or a base that
            // reaches back to this model — is reported as the cycle it is
            // rather than recursing.
            visiting.push(id.to_owned());
            linearize(base, registry, &mut visiting, &mut seen, 1, &mut next_index)?;
            visiting.pop();
        }
    }

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
    Ok(order.into_iter().cloned().collect())
}

/// The merged bodies of an ancestor list, lowest precedence first.
//
// ⛔⛔ Merging fully *resolved* parents instead is the subtle way to get this
// wrong, and it was: with `"extends": ["p1", "p2"]` where both descend from
// `_defaults`, resolved-p2 carries `_defaults`' values for everything p2
// never mentions — so merging it over resolved-p1 let p2's *inherited
// defaults* silently overwrite p1's *explicit* declarations. An artist
// model with two parents got `_defaults`' straight timing and generic BPM
// back, with nothing reported anywhere.
//
// ⛔ **This is also what makes the ancestor list a sound cache key.** The
// result depends on the ordered list and on the ancestors' own bodies —
// never on which child asked — so two models with the same ancestors have
// the same base by construction.
fn merge_ancestors(
    order: &[String],
    registry: &BTreeMap<String, Value>,
) -> Result<Value, DatasetError> {
    let mut merged = Value::Object(Map::new());
    for ancestor in order {
        let model = registry
            .get(ancestor)
            .ok_or_else(|| DatasetError::UnknownParent(ancestor.clone()))?;
        merged = deep_merge(merged, model);
        // ⛔ Per STEP, not once at the end (TASK-172). A genre that states its
        // own `anchors` over a parent genre that states a `secondaryAnchor` is
        // THIS merge, not `child_over`'s — `trap-soul` over `trap` is exactly
        // that, and it is extended by 70 models.
        drop_orphaned_companions(&mut merged, model);
    }
    Ok(merged)
}

/// The child's own body over its merged ancestors, with identity restored.
fn child_over(
    id: &str,
    merged: Value,
    registry: &BTreeMap<String, Value>,
) -> Result<Value, DatasetError> {
    let model = registry
        .get(id)
        .ok_or_else(|| DatasetError::UnknownParent(id.to_owned()))?;

    // The child last, on top of every ancestor. It is merged here rather than
    // as the tail of `order` because `order` is sorted by depth and the child
    // is not an ancestor of itself.
    let mut merged = deep_merge(merged, model);

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

    drop_orphaned_companions(&mut merged, model);

    Ok(merged)
}

/// Keys that are part of the array beside them, and must go when it is replaced
/// (TASK-172).
///
/// Each row is `(block path, the array, the companions it owns)`.
///
/// ⛔⛔ **The hazard, in one sentence.** An array in a child *replaces* the
/// parent's — that is `deep_merge`'s rule and it is what lets a model narrow
/// what it inherits. A **scalar sibling that means the same thing** does not:
/// it survives the replacement and is read back alongside the child's list. So
/// a model saying *"my anchors are 1 and 3"* was given 1, 2& and 3, because
/// `drums::anchor_ticks` unions `secondaryAnchor` into `anchors`.
///
/// ▶ **Measured before it was changed.** `darius-rucker` authors
/// `anchors: ["1", "3"]` with `densityPerBar: [2, 3]`; over `rnb-2000s`, whose
/// `secondaryAnchor` is `"2&"`, all three hits became guaranteed and the kick
/// fell from **92 distinct shapes in 200 seeds to 4** — every other lane
/// unchanged.
///
/// ⛔ **Authorised by the owner on 2026-08-22**, on the condition that the
/// result *"ends up still sounding and adhering to the artist/producer's
/// genre/db that i researched"* — which is the direction this moves.
///
/// ⛔⛔ **ONE ROW, AND THAT IS DELIBERATE.** A review of this fix found at least
/// five more pairs with the same shape — `percs.lanes` + `tambourine`,
/// `arrangement…parts` + `addLayers`, `chords.progressionFamilies` +
/// `maxChords` (**395 models**), `hihat.rolls.vocab` + `burstOnly`,
/// `bass808.slidePositions` + `slidesPer4Bars`. Every one is a candidate and
/// **none of them is authorised**: adding a row here changes what shipped models
/// generate, and that is the owner's call and his ears. They are written up in
/// the roadmap under TASK-172 with their counts. **This table is the mechanism;
/// the rows are a product decision.**
const COMPANIONS: &[(&str, &str, &[&str])] = &[("/drums/kick", "anchors", &["secondaryAnchor"])];

/// Drop companions the child orphaned by replacing their array.
///
/// ⛔⛔ **Applied at EVERY merge step, which the first cut was not.** It lived
/// only in [`child_over`], and a review found two live holes: `trap-soul`
/// authors `anchors: ["1"]` over `trap`'s `secondaryAnchor: "3"` and is extended
/// by **70 models** — a genre-over-genre merge, which goes through
/// [`merge_ancestors`] and never reached the rule — and
/// [`crate::dataset::modes::apply`] merges a mode's overrides with a bare
/// `deep_merge`, where **nine mode blocks** author `anchors` and four of them sit
/// on a kick that declares a secondary. `blake-shelton`'s *traditional* mode
/// reproduced the headline case exactly. A rule installed at one of three doors
/// is the failure this repo already records four times in one branch.
///
/// ⚠ **The over-model's own authorship decides, never the accumulated base.**
/// A child that authors the companion itself keeps it — it has answered the
/// question — and one that authors neither inherits both, which is what an
/// inherited-tier model wants.
pub(crate) fn drop_orphaned_companions(merged: &mut Value, over: &Value) {
    for (block, array, companions) in COMPANIONS {
        // ⚠ A JSON Pointer, which this crate already uses — `validate.rs` reads
        // `model.pointer("/session/bpm")`. A hand-rolled key walk needed two
        // mutable borrows of the same value and did not compile.
        let Some(authored) = over.pointer(block) else {
            continue;
        };
        if authored.get(array).is_none() {
            continue;
        }
        let orphaned: Vec<&str> = companions
            .iter()
            .copied()
            .filter(|companion| authored.get(companion).is_none())
            .collect();
        if orphaned.is_empty() {
            continue;
        }
        if let Some(Value::Object(target)) = merged.pointer_mut(block) {
            for companion in orphaned {
                target.remove(companion);
            }
        }
    }
}

/// [`resolve`], reusing an ancestor merge an earlier model already paid for.
///
/// ⛔ **For bulk resolution only** — `resolve_all` walks 590 models and most of
/// them are artists over the same handful of genre archetypes, so the same
/// ancestor chain was being merged from scratch dozens of times. The cache is
/// the caller's, lives for one `resolve_all`, and is keyed on the ordered
/// ancestor list for the reason [`merge_ancestors`] states.
///
/// ⚠ **Not a global or a `OnceLock`.** The registry is what the merge reads,
/// and a cache that outlived one call would answer from a registry that has
/// since changed — which is precisely what a user model saved through the
/// editor does.
pub fn resolve_memoized(
    id: &str,
    registry: &BTreeMap<String, Value>,
    cache: &mut BTreeMap<Vec<String>, Value>,
) -> Result<Value, DatasetError> {
    let order = ancestor_order(id, None, registry)?;
    let merged = match cache.get(&order) {
        Some(merged) => merged.clone(),
        None => {
            let merged = merge_ancestors(&order, registry)?;
            cache.insert(order, merged.clone());
            merged
        }
    };
    child_over(id, merged, registry)
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
            deep_merge(base, &over),
            json!({ "session": { "bpm": { "min": 140 }, "halfTime": true } })
        );
    }

    #[test]
    fn arrays_replace_rather_than_append() {
        // A child narrowing its inherited list is the common case; appending
        // would make narrowing impossible.
        let base = json!({ "genres": ["trap", "drill", "rage"] });
        let over = json!({ "genres": ["rage"] });
        assert_eq!(deep_merge(base, &over), json!({ "genres": ["rage"] }));
    }

    #[test]
    fn a_child_can_override_with_null() {
        let base = json!({ "drums": { "percs": { "lanes": ["rim"] } } });
        let over = json!({ "drums": { "percs": null } });
        assert_eq!(
            deep_merge(base, &over),
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

    // ── The base swap (TASK-158C) ──────────────────────────────────────────

    /// A tiny world: two genres that disagree about everything, and an artist
    /// who authors one thing of their own.
    fn two_genres() -> BTreeMap<String, Value> {
        BTreeMap::from([
            (
                "_defaults".to_owned(),
                json!({ "id": "_defaults", "session": { "bpm": 100 }, "drums": { "swing": 0 } }),
            ),
            (
                "g-funk".to_owned(),
                json!({
                    "id": "g-funk",
                    "type": "genre",
                    "extends": ["_defaults"],
                    "session": { "bpm": 92 },
                    "drums": { "swing": 8, "kick": "laid-back" },
                }),
            ),
            (
                "boom-bap".to_owned(),
                json!({
                    "id": "boom-bap",
                    "type": "genre",
                    "extends": ["_defaults"],
                    "session": { "bpm": 88 },
                    "drums": { "swing": 55, "kick": "on-the-one" },
                }),
            ),
            (
                "2pac".to_owned(),
                json!({
                    "id": "2pac",
                    "type": "artist",
                    "name": "2Pac",
                    "extends": ["g-funk"],
                    "relatedGenres": ["g-funk", "boom-bap"],
                    "drums": { "kick": "his-own" },
                }),
            ),
        ])
    }

    #[test]
    fn a_base_of_none_is_the_model_as_authored() {
        // ⛔⛔ **Every model in the product goes through `resolve_over` now**, so
        // if `None` were not byte-identical to the old `resolve`, adding this
        // feature would have changed what the other 590 generate.
        let registry = two_genres();
        for id in registry.keys() {
            assert_eq!(
                resolve_over(id, None, &registry).unwrap(),
                resolve(id, &registry).unwrap(),
                "{id} resolves differently through the swap path"
            );
        }
    }

    #[test]
    fn an_artist_keeps_their_own_blocks_over_the_new_base() {
        // ⛔ The whole claim. 2Pac's own `kick` survives; everything he does not
        // author comes from **boom-bap** rather than from g-funk.
        let registry = two_genres();
        let swapped = resolve_over("2pac", Some("boom-bap"), &registry).unwrap();

        assert_eq!(swapped["drums"]["kick"], json!("his-own"), "his own wins");
        assert_eq!(
            swapped["drums"]["swing"],
            json!(55),
            "boom-bap's, not g-funk's"
        );
        assert_eq!(swapped["session"]["bpm"], json!(88));
        // ⚠ Identity is still the child's — a swap must not be able to rename a
        // model or turn an artist into a genre.
        assert_eq!(swapped["id"], json!("2pac"));
        assert_eq!(swapped["type"], json!("artist"));
        assert_eq!(swapped["name"], json!("2Pac"));
    }

    #[test]
    fn the_base_the_artist_already_extends_changes_nothing() {
        // ⚠ Asking for the genre they were authored in is a no-op, not a second
        // merge of it. The chip's "Any" and "g-funk" have to agree for 2Pac.
        let registry = two_genres();
        assert_eq!(
            resolve_over("2pac", Some("g-funk"), &registry).unwrap(),
            resolve("2pac", &registry).unwrap()
        );
    }

    #[test]
    fn the_grandparent_still_arrives_through_the_new_base() {
        // ⛔ `_defaults` is reached only *through* a genre. Swapping the genre
        // must not drop it — a model with no defaults is one the linter refuses
        // and the generators read holes out of.
        let registry = two_genres();
        let swapped = resolve_over("2pac", Some("boom-bap"), &registry).unwrap();
        // `_defaults` authors `drums.swing: 0`; boom-bap overrides it to 55, so
        // its presence shows in a key only `_defaults` has.
        assert!(swapped.get("session").is_some());
        assert_eq!(swapped["drums"]["swing"], json!(55));
    }

    #[test]
    fn a_base_that_is_the_model_itself_is_a_cycle_rather_than_a_recursion() {
        // ⛔ Reachable from the page: the roster lists a genre *and* artists, and
        // nothing stops a payload naming the same id twice.
        let registry = two_genres();
        assert!(matches!(
            resolve_over("2pac", Some("2pac"), &registry),
            Err(DatasetError::Cycle(_))
        ));
    }

    #[test]
    fn a_base_that_names_nothing_is_refused_rather_than_ignored() {
        // ⚠ Silently falling back to the authored base would be a chip that
        // reads "boom-bap" over a g-funk generation — the readout-that-lies
        // failure this whole task is about, arriving through the fix.
        let registry = two_genres();
        assert!(matches!(
            resolve_over("2pac", Some("no-such-genre"), &registry),
            Err(DatasetError::UnknownParent(_))
        ));
    }

    #[test]
    fn an_unknown_model_reports_itself_rather_than_the_base() {
        let registry = two_genres();
        match resolve_over("nobody", Some("boom-bap"), &registry) {
            Err(DatasetError::UnknownParent(id)) => assert_eq!(id, "nobody"),
            other => panic!("expected the model to be reported, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod companions {
    use super::*;
    use serde_json::json;

    fn registry(child: Value) -> BTreeMap<String, Value> {
        BTreeMap::from([
            (
                "parent".to_owned(),
                json!({
                    "id": "parent",
                    "type": "genre",
                    "name": "Parent",
                    "drums": { "kick": { "anchors": ["1"], "secondaryAnchor": "2&" } }
                }),
            ),
            ("child".to_owned(), child),
        ])
    }

    fn kick(resolved: &Value) -> &Value {
        &resolved["drums"]["kick"]
    }

    #[test]
    fn a_child_that_states_its_own_anchors_does_not_inherit_a_second_one() {
        // ⛔ TASK-172. `anchors` is an array and replaces; `secondaryAnchor` is a
        // scalar beside it and used to survive, so a model saying "1 and 3" got
        // 1, 2& and 3 — and where that filled its density budget the kick
        // stopped varying at all.
        let registry = registry(json!({
            "id": "child",
            "type": "artist",
            "name": "Child",
            "extends": ["parent"],
            "drums": { "kick": { "anchors": ["1", "3"] } }
        }));
        let resolved = resolve("child", &registry).expect("it resolves");
        assert_eq!(kick(&resolved)["anchors"], json!(["1", "3"]));
        assert!(
            kick(&resolved).get("secondaryAnchor").is_none(),
            "the parent's secondary anchor must not survive the child's own list: {:?}",
            kick(&resolved)
        );
    }

    #[test]
    fn a_child_that_states_neither_inherits_both() {
        // The inherited-tier case, and the reason this is not simply "never
        // inherit it": a model with no drum deltas wants its genre's whole kick
        // grammar, secondary anchor included.
        let registry = registry(json!({
            "id": "child",
            "type": "artist",
            "name": "Child",
            "extends": ["parent"],
            "session": { "bpm": { "min": 90, "max": 100 } }
        }));
        let resolved = resolve("child", &registry).expect("it resolves");
        assert_eq!(kick(&resolved)["anchors"], json!(["1"]));
        assert_eq!(kick(&resolved)["secondaryAnchor"], json!("2&"));
    }

    #[test]
    fn a_child_that_states_both_keeps_both() {
        // It has answered the question, so nothing is dropped — including when
        // it authors a *different* secondary from its parent's.
        let registry = registry(json!({
            "id": "child",
            "type": "artist",
            "name": "Child",
            "extends": ["parent"],
            "drums": { "kick": { "anchors": ["1", "3"], "secondaryAnchor": "4&" } }
        }));
        let resolved = resolve("child", &registry).expect("it resolves");
        assert_eq!(kick(&resolved)["secondaryAnchor"], json!("4&"));
    }

    #[test]
    fn a_swap_applies_the_same_rule_as_a_plain_resolve() {
        // ⚠ `resolve_over` is the path this was found on — "Darius Rucker, but
        // in R&B" — and it reaches `child_over` the same way, so the rule must
        // hold there or the fix misses the case that produced it.
        let mut registry = registry(json!({
            "id": "child",
            "type": "artist",
            "name": "Child",
            "extends": ["other"],
            "drums": { "kick": { "anchors": ["1", "3"] } }
        }));
        registry.insert(
            "other".to_owned(),
            json!({ "id": "other", "type": "genre", "name": "Other" }),
        );
        let resolved = resolve_over("child", Some("parent"), &registry).expect("it resolves");
        assert!(kick(&resolved).get("secondaryAnchor").is_none());
    }
}

#[cfg(test)]
mod companion_doors {
    //! The rule has to hold at every merge, not just the last one (TASK-172).
    //!
    //! ⛔ These exist because the first cut installed it in `child_over` alone
    //! and a review found two live holes in the shipped dataset: `trap-soul`
    //! authors `anchors` over `trap`'s `secondaryAnchor` and is extended by 70
    //! models, and `blake-shelton`'s *traditional* mode does the same thing to
    //! its own model. One test per door.
    use super::*;
    use serde_json::json;

    fn kick(resolved: &Value) -> &Value {
        &resolved["drums"]["kick"]
    }

    #[test]
    fn a_genre_over_a_genre_drops_it_too() {
        // The `trap-soul` over `trap` shape: the child here is a grandparent of
        // whatever finally resolves, so `child_over` never sees it.
        let registry = BTreeMap::from([
            (
                "grand".to_owned(),
                json!({ "id": "grand", "type": "genre", "name": "Grand",
                        "drums": { "kick": { "anchors": ["1"], "secondaryAnchor": "3" } } }),
            ),
            (
                "middle".to_owned(),
                json!({ "id": "middle", "type": "genre", "name": "Middle",
                        "extends": ["grand"],
                        "drums": { "kick": { "anchors": ["1", "4"] } } }),
            ),
            (
                "leaf".to_owned(),
                json!({ "id": "leaf", "type": "artist", "name": "Leaf",
                        "extends": ["middle"],
                        "session": { "bpm": { "min": 90, "max": 100 } } }),
            ),
        ]);
        let resolved = resolve("leaf", &registry).expect("it resolves");
        assert_eq!(kick(&resolved)["anchors"], json!(["1", "4"]));
        assert!(
            kick(&resolved).get("secondaryAnchor").is_none(),
            "the grandparent's secondary must not reach the leaf: {:?}",
            kick(&resolved)
        );
    }

    #[test]
    fn a_mode_that_states_its_own_anchors_drops_it_as_well() {
        // `blake-shelton`'s *traditional*: the model declares a secondary, the
        // mode declares its own anchors, and `modes::apply` merges with a bare
        // `deep_merge` that never reached `child_over`.
        let model: crate::StyleModel = serde_json::from_value(json!({
            "id": "m", "type": "artist", "name": "M",
            "drums": { "kick": { "anchors": ["1"], "secondaryAnchor": "2&" } },
            "modes": [{ "name": "traditional", "weight": 1,
                        "drums": { "kick": { "anchors": ["1", "3"] } } }]
        }))
        .expect("the fixture parses");

        let applied = crate::dataset::modes::apply(&model, "traditional").expect("it applies");
        let value = serde_json::to_value(&applied).expect("it serialises");
        assert_eq!(kick(&value)["anchors"], json!(["1", "3"]));
        assert!(
            kick(&value).get("secondaryAnchor").is_none(),
            "the model's secondary must not survive the mode's own list: {:?}",
            kick(&value)
        );
    }

    #[test]
    fn a_mode_that_states_neither_leaves_the_model_alone() {
        // A mood that only changes the melody must not disturb the kick.
        let model: crate::StyleModel = serde_json::from_value(json!({
            "id": "m", "type": "artist", "name": "M",
            "drums": { "kick": { "anchors": ["1"], "secondaryAnchor": "2&" } },
            "modes": [{ "name": "sparse", "weight": 1,
                        "melody": { "densityPerBar": [1, 3] } }]
        }))
        .expect("the fixture parses");

        let applied = crate::dataset::modes::apply(&model, "sparse").expect("it applies");
        let value = serde_json::to_value(&applied).expect("it serialises");
        assert_eq!(kick(&value)["secondaryAnchor"], json!("2&"));
    }
}
