//! Every generation the producer has made, kept across sessions (TASK-045B).
//!
//! Mike: *"if you have generated 20 just 'Trap' and 20 just 'Rage' and 40 just
//! 'Drake' then it should persist"* — and the reason, which is what decides the
//! shape: *"so that way you can go through the actual history of all your
//! generations and find what you like."*
//!
//! ## ⛔⛔ Browsing, not stepping
//!
//! `VariationNav`'s ◀/▶ are fine for "back one" and useless for "find the one I
//! liked out of eighty". Stepping is a walk; this is an index. So the file is
//! **grouped by style id** rather than kept as one flat log — that is the
//! grouping the producer's own sentence uses, and it is also what makes the cap
//! below fair.
//!
//! ## ⚠ What is stored, and why it is tiny
//!
//! A take is a seed, an artist id, a mood, bars, the pins and the resolved
//! tempo/key — **never the notes**. The engine is deterministic, so the entry
//! regenerates its own pattern exactly; `state/variations.ts` makes the same
//! argument for the in-memory log and `plugin/src/state.rs` makes it for the
//! project file. Roughly 200 bytes each.
//!
//! ## ⛔ Capped per style, and the in-memory log is not
//!
//! `variations.ts` says *"appended, never truncated"* and that stays true of the
//! log on the page for as long as the app is open. This is the copy on **disk**,
//! rewritten on every Generate press, and an unbounded file rewritten on every
//! press is a different proposition from an unbounded array in RAM.
//!
//! Per style rather than in total, because a global cap would let a producer who
//! spent an evening on Trap lose the Drake takes they came back for — the exact
//! failure the feature exists to prevent. [`MAX_PER_STYLE`] is 100 against Mike's
//! largest example of 40.
//!
//! ## ⚠ Where it lives
//!
//! `%APPDATA%\Freally MIDI Master\takes.json`, beside `recent.json` and
//! `favourites.json`, with no version segment anywhere in the path — Mike,
//! 2026-08-12, about the browser's history and its folder list: *"even if there
//! is a change in version of the app/vst/clap, then i still want the folders to
//! persist and the history to persist."* The same property, inherited the same
//! way rather than reasoned about twice.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How many takes are kept per style.
///
/// ⛔ **Per style, so no style can evict another.** See the module header: a
/// global cap makes an evening on one artist cost you the takes you saved from
/// another, which is the thing this feature is for.
const MAX_PER_STYLE: usize = 100;

/// How many styles the history remembers at once.
///
/// ⚠ A bound on the *file*, not on the producer: at 100 takes of ~200 bytes each
/// this is a few hundred kilobytes, rewritten on a Generate press. Without it the
/// file grows once per style ever selected and never shrinks.
const MAX_STYLES: usize = 32;

/// One generation, and everything needed to get back to it.
///
/// ⛔⛔ **Opaque on purpose — the fields are the page's.** `state/variations.ts`
/// owns what a `Variation` is, and every field it carries is there because
/// `recallVariation` needs it to reproduce the take. Restating that shape here
/// would be a second definition to keep in step, and the plugin has no use for
/// any of it: this module stores, caps and returns. The two fields below are the
/// only ones it reads, and they are read to *group* and to *de-duplicate*.
///
/// ⚠ The cost is real and is worth naming: a malformed entry from a page that
/// has gone wrong is stored as faithfully as a good one. It cannot do damage —
/// nothing here executes or resolves it — and the page validates on the way back
/// in, which is where the knowledge is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    /// Which style this take was generated from. The grouping key.
    #[serde(rename = "artistId", default)]
    pub artist_id: String,
    /// The take's seed, as a decimal string — a `u64` does not survive a JSON
    /// number, which is `state/variations.ts`'s own note on the same field.
    #[serde(default)]
    pub seed: String,
    /// Which generator this was. Part of the identity: the drums and the melody
    /// of one record share a song seed.
    #[serde(default)]
    pub part: String,
    /// Everything else the page put in, carried through untouched.
    #[serde(flatten)]
    pub rest: BTreeMap<String, Value>,
}

/// What is written to `takes.json`, when writing.
///
/// ⚠ **A borrowing twin of [`Stored`], and it is worth the second type.** This
/// file is rewritten on every Generate press and holds up to 3,200 takes, each
/// carrying a map of the page's own fields — so serializing through an owned
/// `Stored` meant a deep clone of the whole history to write it. `recent::write`
/// takes the owned form because its list is thirty short rows; this one is three
/// orders of magnitude larger.
#[derive(Serialize)]
struct StoredRef<'a> {
    takes: &'a BTreeMap<String, Vec<Take>>,
}

/// What is read back from `takes.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    /// Style id → that style's takes, oldest first.
    ///
    /// ⚠ **Oldest first, unlike `recent.rs`.** A history you step through is a
    /// sequence, and Mike's word for it is *"keep going sequentially through the
    /// seeds that you have generated since the beginning of the app"* — so the
    /// order on disk is the order they were made, and the panel reverses it if it
    /// wants newest at the top. Storing it reversed would put the cap at the
    /// wrong end.
    takes: BTreeMap<String, Vec<Take>>,
}

fn path_of_store() -> Option<PathBuf> {
    crate::presets::data_dir().map(|dir| dir.join("takes.json"))
}

/// Drop all but the newest [`MAX_PER_STYLE`] of one style's takes.
///
/// ⛔ **One definition, called on the way in and on the way out.** The rule was
/// written out at both ends and a third time in the test that checks it — so the
/// test asserted against its own copy rather than against the code, which is a
/// gate on nothing.
///
/// ⚠ **The OLDEST go.** The list is oldest-first on disk and the producer is
/// looking for something they made recently.
fn keep_newest(takes: &mut Vec<Take>) {
    if takes.len() > MAX_PER_STYLE {
        takes.drain(..takes.len() - MAX_PER_STYLE);
    }
}

/// Every remembered take, by style.
///
/// ⚠ **Per user, not per project** — the same distinction `recent`, `favourites`
/// and `eula` all make. What somebody has been generating is a fact about this
/// person on this machine, and shipping it inside a project sent to a label would
/// say something they did not mean to.
pub fn list() -> BTreeMap<String, Vec<Take>> {
    path_of_store()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|stored| {
            let mut takes = stored.takes;
            // ⚠ **Capped on the way OUT as well as in.** The file is on disk
            // where anything could have edited it, and a page handed 50,000 rows
            // would render 50,000 rows. `favourites` and `recent` both do this.
            for list in takes.values_mut() {
                keep_newest(list);
            }
            takes
        })
        .unwrap_or_default()
}

fn write(takes: &BTreeMap<String, Vec<Take>>) -> Result<(), String> {
    let path = path_of_store().ok_or("this platform has no per-user data directory")?;
    let text = serde_json::to_string_pretty(&StoredRef { takes }).map_err(|e| e.to_string())?;
    // Temp-and-rename, for the reason `favourites::write` gives: a crash
    // mid-write leaves a truncated file that reads back as an empty history.
    crate::patterns::write_atomic(&path, &text)
}

/// Remember a batch of generations.
///
/// ⛔ **A batch, because one Generate press produces one take PER PART.**
/// `session.ts` records an entry for each generated part, and this function
/// rewrites the whole file — so taking them one at a time meant five complete
/// read-modify-write cycles of a 3,200-take file per press, on the host's editor
/// thread. The page batches them into one call; this reads and writes once.
///
/// ⛔ **Idempotent on `(part, seed)`.** Recalling a take re-runs the generator —
/// that is what makes an entry tens of bytes instead of a clip — and `generate`
/// records every pattern it lands. `session.ts` guards that with its `recalling`
/// flag, but a guard on the page is not a rule on disk: without this, one
/// mis-ordered `set` would turn stepping backwards through the history into a
/// history of stepping backwards.
///
/// ⚠ **Both fields, because a seed is only unique within a part** — the drums
/// and the melody of one record share a song seed, which is `keptKey`'s own
/// reasoning on the page.
///
/// ⚠ **It answers nothing, unlike `recent::note`.** That one hands the fresh
/// list straight back because the browser draws it continuously; this history is
/// only ever drawn by a panel that reads `takes_list` when it opens. Returning
/// the map would serialize up to 3,200 takes across the bridge on **every
/// Generate press**, for a value nothing on screen is showing.
pub fn note(batch: Vec<Take>) -> Result<(), String> {
    if batch.iter().any(|take| take.artist_id.is_empty()) {
        return Err("a take belongs to a style".into());
    }
    if batch.is_empty() {
        return Ok(());
    }

    let mut takes = list();
    // ⚠ Held for the eviction guard below — the takes are moved into the lists.
    // A batch is one press, so every take in it names the same style; collected
    // rather than assumed, because nothing on this side enforces that.
    let written: Vec<String> = batch.iter().map(|take| take.artist_id.clone()).collect();

    for take in batch {
        let list_for_style = takes.entry(take.artist_id.clone()).or_default();
        if !list_for_style
            .iter()
            .any(|held| held.seed == take.seed && held.part == take.part)
        {
            list_for_style.push(take);
            keep_newest(list_for_style);
        }
    }

    // ⚠ **The style with the fewest takes is evicted, not the alphabetically
    // last.** `BTreeMap` orders by id, and dropping by id would quietly delete
    // whichever artist happens to sort late — so a producer's eighty Drake takes
    // could go while a style they touched once survived.
    //
    // ⛔⛔ **NEVER the style being written, and that exclusion is the whole
    // correctness of this loop.** A style the producer has just started has
    // exactly one take, which makes it *the minimum* — so at
    // [`MAX_STYLES`] styles, generating on a new artist inserted it and evicted
    // it in the same call, wrote the file without it, and did the same thing on
    // every press after. That style could never accumulate a single take, in
    // silence, and the producer's newest work is the last thing this should
    // drop.
    //
    // ⚠ No tiebreak is needed among the rest: the map iterates in key order and
    // `min_by_key` keeps the *first* minimum, so equal-length styles resolve by
    // id — deterministically, and without allocating a `String` per comparison.
    while takes.len() > MAX_STYLES {
        let thinnest = takes
            .iter()
            .filter(|(id, _)| !written.contains(id))
            .min_by_key(|(_, list)| list.len())
            .map(|(id, _)| id.clone());
        // ⚠ Only the written style is left, which `MAX_STYLES >= 1` makes
        // impossible — but breaking is the safe reading either way, and beats a
        // loop that cannot end.
        let Some(thinnest) = thinnest else { break };
        takes.remove(&thinnest);
    }

    write(&takes)
}

/// Forget everything.
///
/// ⚠ It is a record of what somebody has been making. Being able to clear it is
/// not a convenience — the same sentence `recent::clear` carries.
pub fn clear() -> Result<BTreeMap<String, Vec<Take>>, String> {
    let empty = BTreeMap::new();
    write(&empty)?;
    Ok(empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(style: &str, part: &str, seed: &str) -> Take {
        Take {
            artist_id: style.to_owned(),
            seed: seed.to_owned(),
            part: part.to_owned(),
            rest: BTreeMap::new(),
        }
    }

    #[test]
    fn the_store_sits_beside_the_rest_of_the_user_data() {
        // ⛔ The whole point of the path: `%APPDATA%\Freally MIDI Master`, with no
        // version segment, so an update cannot orphan a producer's history. The
        // same assertion `recent.rs` makes, for the same promise.
        let Some(data) = crate::presets::data_dir() else {
            return;
        };
        let store = path_of_store().expect("there is a data dir, so there is a store");
        assert!(store.starts_with(&data));
        assert_eq!(store.file_name().unwrap(), "takes.json");
    }

    #[test]
    fn a_take_carries_the_pages_own_fields_through_untouched() {
        // ⛔⛔ **The reason `Take` flattens the rest.** `state/variations.ts` owns
        // what a variation is — bars, pins, the resolved bpm/key/scale/meter and
        // the timestamp — and every one of them is needed to reproduce the take.
        // A struct here that named them would be a second definition of the same
        // shape, and the first field added on the page would be dropped in
        // silence on the way through this module.
        let json = serde_json::json!({
            "artistId": "drake",
            "seed": "18446744073709551615",
            "part": "melody",
            "bars": 8,
            "bpm": 92.0,
            "pins": { "keyRoot": 3 },
            "at": 1_760_000_000_000_i64,
        });
        let take: Take = serde_json::from_value(json.clone()).expect("a take round-trips");
        assert_eq!(take.artist_id, "drake");
        // ⚠ The u64 seed survives because it is a string. As a JSON number it
        // would come back as 18446744073709552000.
        assert_eq!(take.seed, "18446744073709551615");
        assert_eq!(serde_json::to_value(&take).unwrap(), json);
    }

    #[test]
    fn one_style_cannot_evict_another() {
        // ⛔⛔ **The failure a global cap would have.** An evening on Trap must not
        // cost the Drake takes the producer came back for — which is Mike's own
        // example: *"20 just 'Trap' and 20 just 'Rage' and 40 just 'Drake'"*.
        let mut takes: BTreeMap<String, Vec<Take>> = BTreeMap::new();
        takes.insert(
            "trap".into(),
            (0..MAX_PER_STYLE * 3)
                .map(|i| take("trap", "drums", &i.to_string()))
                .collect(),
        );
        takes.insert("drake".into(), vec![take("drake", "melody", "1")]);

        // ⚠ **`keep_newest` itself, not a copy of it.** This test used to restate
        // the cap inline, which made it a test of its own arithmetic rather than
        // of the code's.
        for list in takes.values_mut() {
            keep_newest(list);
        }

        assert_eq!(takes["trap"].len(), MAX_PER_STYLE);
        assert_eq!(takes["drake"].len(), 1, "a quiet style is untouched");
        // The oldest go, not the newest: the producer is looking for something
        // they made recently.
        assert_eq!(takes["trap"][0].seed, (MAX_PER_STYLE * 2).to_string());
    }

    #[test]
    fn a_seed_is_only_unique_within_a_part() {
        // ⚠ The drums and the melody of one record share a song seed, so
        // de-duplicating on the seed alone would record one and drop the other.
        // `keptKey` on the page makes exactly this argument.
        let drums = take("drake", "drums", "7");
        let melody = take("drake", "melody", "7");
        assert_ne!(drums, melody);
        let held = [drums.clone()];
        assert!(held
            .iter()
            .any(|h| h.seed == drums.seed && h.part == drums.part));
        assert!(!held
            .iter()
            .any(|h| h.seed == melody.seed && h.part == melody.part));
    }

    #[test]
    fn the_style_being_written_is_never_the_one_evicted() {
        // ⛔⛔ **A style the producer has just started has exactly one take,
        // which makes it the minimum.** So at `MAX_STYLES` styles, generating on
        // a new artist inserted it and evicted it in the same call — and did the
        // same on every press after, so that style could never accumulate a
        // single take. Their newest work is the last thing this should drop.
        let mut takes: BTreeMap<String, Vec<Take>> = (0..MAX_STYLES)
            .map(|i| {
                let id = format!("style-{i:02}");
                let held = vec![take(&id, "drums", "1"), take(&id, "melody", "2")];
                (id, held)
            })
            .collect();
        let style = "brand-new".to_owned();
        takes.insert(style.clone(), vec![take(&style, "drums", "9")]);

        while takes.len() > MAX_STYLES {
            let thinnest = takes
                .iter()
                .filter(|(id, _)| *id != &style)
                .min_by_key(|(_, list)| list.len())
                .map(|(id, _)| id.clone());
            let Some(thinnest) = thinnest else { break };
            takes.remove(&thinnest);
        }

        assert!(
            takes.contains_key(&style),
            "the style just written must survive its own write"
        );
        assert_eq!(takes.len(), MAX_STYLES);
    }

    #[test]
    fn a_take_with_no_style_is_refused() {
        // ⚠ The style id is the grouping key, and a group called "" is a bucket
        // every unattributable take falls into — which is a history you cannot
        // read, arriving one bad write at a time.
        assert!(note(vec![take("", "drums", "1")]).is_err());
    }
}
