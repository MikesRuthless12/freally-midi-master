//! Every key the dataset authors must be read by something.
//!
//! ⛔⛔ **This gate exists because the same defect shipped twice in one day**,
//! 2026-08-07, and neither instance was visible to any other test:
//!
//! - **`drums.percs`** — 15 of 30 models authored a percussion block, complete
//!   with `lanes`, `densityPerBar`, `placement` and `gainOffsetDb`, and **no
//!   generator read a byte of it**. `uk-drill` asked for a `woodblock`, which
//!   was not even a `Lane`, so the request vanished twice over.
//! - **`kit.preview`** — 8 models named the kit they should be heard through.
//!   `rage` asked for `rage-default` and `uk-drill` for `drill-default`; only
//!   `trap-default` existed on disk, and `preview_kit()` was hardcoded to it
//!   anyway. Both silently played trap samples.
//!
//! ▶ **Why nothing caught either.** `dataset:validate` checks the *schema*, and
//! `$defs/partBlock` is deliberately open — its own description says each
//! generator's fields are locked down "as that generator lands". So an authored
//! key that no code reads is, to every existing gate, indistinguishable from
//! one that works. The only way to see it is to compare what the data says
//! against what the source names, which is what this does.
//!
//! ## How it works, and what it cannot see
//!
//! Every key in every model is collected, then compared against every string
//! literal in the engine, the plugin and the tools. That is deliberately
//! **loose**: a key named anywhere at all counts as read. A tighter match
//! (`block(x, "key")` and friends) would be more truthful and would also fail
//! on every legitimate spelling nobody predicted, so this errs towards silence.
//!
//! ⚠ **It therefore proves the weak direction only.** A key that appears in no
//! reader is certainly dead; a key that appears somewhere might still be dead
//! if the literal it matched was a coincidence. That is the same trap the
//! shortcut catalog's guard hit, where `Ctrl+3` "passed" because the digit `3`
//! occurred elsewhere — so the weak direction is the honest claim to make.
//!
//! ⚠ **Four keys are known to be authored, unread, and invisible to this**, and
//! they are named here rather than lost: `layered`, `stack`, `tritone` and
//! `build`. Each is an ordinary English word that occurs as an
//! unrelated identifier somewhere in the scanned trees, so the loose match
//! treats them as read. They cannot go in [`DEFERRED`] — the staleness check
//! would reject them for exactly that reason — and a second list that nothing
//! asserts on is what this file already had and deleted, because it rotted.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Keys that are authored and genuinely not read yet, with why.
///
/// ⛔ **This list is a debt register, not a permission slip.** Every entry is a
/// model asking for something the engine does not do — a producer's authored
/// intent, doing nothing. Entries come *off* this list by being implemented,
/// never by being deleted because the test was noisy.
///
/// ⚠ **Adding to it should feel wrong.** The whole point of the gate is that a
/// *new* unread key fails; an entry here is an admission that one already
/// shipped. Anything added needs a reason a reader would accept.
const DEFERRED: &[(&str, &str)] = &[
    // Not a model field at all.
    (
        "$schema",
        "the JSON Schema pointer, read by editors rather than by us",
    ),
    // ── The 808's tone. Authored heavily, implemented not at all ──────────
    //
    // The generator writes notes; none of these describe notes. They describe
    // the *sound*, which the sampler would have to apply, and the preview
    // sampler has no per-lane processing at all yet.
    (
        "distortionWet",
        "808 tone: the wet/dry for the drive that does not exist",
    ),
    // ⚠ **152 models, and the `true` half of it already works.** Every shipped
    // kit gives its 808 pad `chokeGroup: 2`, so the sampler does cut the lane
    // against itself — the register said it had "no per-lane choke beyond the
    // hats" and that was read off the wrong file. What is genuinely unread is
    // the **four models that author `false`**, and honouring those means the
    // flag reaching the sampler, which today knows a `Pattern` and not a model.
    (
        "monoCutSelf",
        "808 choke. The kit already chokes it; the four models asking NOT to are unread",
    ),
    // ── The snare's tone and layering ────────────────────────────────────
    ("lowPassedClap", "a dulled clap layer"),
    (
        "lowPassed",
        "a dulled layer, the same idea as `lowPassedClap` elsewhere",
    ),
    (
        "reverb",
        "snare room. There is no effects chain to put it through",
    ),
    // ── The kick ─────────────────────────────────────────────────────────
    (
        "sidechainToKick",
        "ducking. Needs a mixer the plugin does not have",
    ),
    // ── The hats ─────────────────────────────────────────────────────────
    //
    // ⚠ **A design call, not a patch.** `trap` authors `freqPerBar: 0.8` *and*
    // `mutatedBeatsPerBar: [1, 3]`, and those are two answers to one question —
    // how much of the bar the roll engine takes. Reading the second as a cap is
    // inert under the first; reading it as the count triples trap's hat rolls
    // and every model that inherits them. Which one Mike wants is the question.
    (
        "mutatedBeatsPerBar",
        "how much of a bar is re-rolled. Conflicts with `freqPerBar`",
    ),
    // ── Melody and harmony ───────────────────────────────────────────────
    // ⚠ **1,234 authorings across `melody.timbreHint` and `modes[].melody`, not
    // the 21 this entry used to claim** — the largest authored-but-unread key in
    // the dataset now that `portamentoMs` is read.
    //
    // ⛔ **There is nothing to choose from, and that is the blocker.** Every kit
    // ships exactly one melodic pad per lane — `lead`, `bell`, `bass`, `keys` —
    // so "rhodes" and "bright_bell" have the same voice to land on. Reading it
    // means either a melodic sample library the app does not have, or a General
    // MIDI program change in the exported `.mid`, which is a decision about what
    // a producer's DAW does with a dragged clip rather than a patch.
    (
        "timbreHint",
        "1,234 authorings name a sound. Every kit has one melodic pad per lane to play it on",
    ),
    // ⚠ **A `drums.bass808` key, not a countermelody one.** It asks for an 808
    // that tracks the lead's root, and the drums have no upstream to read it
    // from. `drums::generate_in` now takes the *section*, so the signature is no
    // longer the obstacle — the harmony is: `parts::upstream` answers
    // `Upstream::None` for `Part::Drums`, and giving it one makes generating a
    // beat also build and hand back the chords, which changes what the Drums tab
    // returns over IPC. Three models, and a change to how a part is built.
    (
        "followsLeadRoot",
        "an 808 that tracks the lead's root. The drums have no upstream",
    ),
    // ⚠ **`uk-drill` alone, and the numbers are proportions of the harmony.**
    // Read at the source it caps `susOrDimProb: 0.2` and vetoes part of the
    // `["i", "bII", "i"]` family — one key overruling two the same model
    // authors. Read as a post-pass over the voiced chords it needs a definition
    // of "this chord carries a tritone" that nothing in `chords.rs` has, and a
    // rule for what to do with the ones over budget. **A question for Mike.**
    (
        "dissonanceBudget",
        "how much dissonance drill tolerates. Both readings overrule another authored key",
    ),
    // ⚠ **A weight map of scale degrees, and it is authored in `_defaults.json`**
    // — so this is not "one model", it is all 590. It is not "how often chords
    // change" either; that is `harmonicRhythm`, which is read. Weighting family
    // sampling by it double-counts every model's own `progressionFamilies`
    // weights, and using it only where a family is emptied by `avoid` is inert,
    // because no shipped family contains the one numeral `_defaults` avoids.
    // **A question for Mike**, and the widest blast radius of anything here.
    (
        "chordFrequency",
        "the corpus prior over degrees, inherited by all 590. Every reading double-counts",
    ),
    // ── Arrangement and structure ────────────────────────────────────────
    // ⚠ **`drums.minimalism`, not an arrangement key** — `rage`, one model, one
    // number, 0.75. Both readings are wrong rather than merely hard: scaled
    // through the density keys it fights `percs.densityPerBar: [0, 1]` and
    // `fills`, which the same model authors explicitly, and rage at a quarter of
    // its density is not rage; applied per section it duplicates
    // `sectionRules.density`, which `_defaults` already gives every model.
    // **A question for Mike, not a patch**: what does a kit at 0.75 minimal
    // leave out that its own density parameters do not already say?
    (
        "minimalism",
        "how sparse rage's kit is. Every reading fights a key the same model authors",
    ),
    // ── Newly visible once the scanner stopped reading its own comments ──
    //
    // ⚠ These sat in a second list that nothing asserted on, on the theory that
    // the matcher could not see them because their names collide with ordinary
    // identifiers. That was wrong: they were passing because the scanner read
    // whole files *including prose*, and all three are discussed in comments
    // and used in no code at all. Stripping comments made them visible, so they
    // belong in the register that is actually checked.
    (
        "distortion",
        "808 tone: 25 models ask for it; the sampler has no drive stage",
    ),
    ("clipping", "hard clip on the 808 — the same missing stage"),
    (
        "transient",
        "snare attack shaping. There is no envelope stage",
    ),
];

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `engine/`; the data and the plugin are beside it.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the engine lives inside the repo")
        .to_path_buf()
}

/// Every key name any model authors, and one example of where.
fn authored() -> BTreeMap<String, String> {
    let root = repo_root();
    let mut out: BTreeMap<String, String> = BTreeMap::new();

    let mut files: Vec<PathBuf> = Vec::new();
    for dir in ["data/artists", "data/genres"] {
        if let Ok(read) = fs::read_dir(root.join(dir)) {
            files.extend(read.flatten().map(|e| e.path()));
        }
    }
    files.push(root.join("data/_defaults.json"));

    for file in files {
        if file.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let model = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_owned();
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        collect(&value, &model, String::new(), &mut out);
    }
    out
}

fn collect(value: &Value, model: &str, path: String, out: &mut BTreeMap<String, String>) {
    // ⛔ **Arrays are walked too.** This returned on anything that was not an
    // object, so every key inside an array element was invisible to the gate:
    // `arrangement.structures[]`, `chords.progressionFamilies[]` and — the
    // largest by far — `modes[]`, which carries a whole nested model per entry,
    // were never collected at all. A new unread key authored inside one of those
    // blocks could never fail this.
    // ⚠ The four names above went missing from this comment when it was written
    // and it read "`, `, and ` were never collected", which said nothing at all.
    if let Some(items) = value.as_array() {
        for item in items {
            collect(item, model, path.clone(), out);
        }
        return;
    }
    let Some(map) = value.as_object() else { return };
    for (key, child) in map {
        let at = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        out.entry(key.clone())
            .or_insert_with(|| format!("{model}:{at}"));
        collect(child, model, at, out);
    }
}

/// Every identifier-shaped string literal in the code that reads the dataset.
fn named_in_source() -> BTreeSet<String> {
    let root = repo_root();
    let mut text = String::new();
    for dir in ["engine/src", "plugin/src", "tools"] {
        read_rust(&root.join(dir), &mut text);
    }
    // The schema names the fields it constrains, which is a reader too.
    if let Ok(schema) = fs::read_to_string(root.join("data/schema/artist-style.schema.json")) {
        text.push_str(&schema);
    }

    // ⛔ **Identifiers as well as string literals, and this was a real gap.**
    // A great many keys are read through serde's derive rather than by name:
    // `Lane::Clap` becomes `"clap"` only because of `rename_all`, and
    // `SessionDefaults.scales` is a struct field, not a literal. The first cut
    // of this gate reported `clap`, `snap` and `scales` as dead while all three
    // were read on every generation — a false alarm that would have trained
    // whoever met it to add things to `DEFERRED` to make the noise stop.
    //
    // ⚠ **Compared with case and underscores removed**, because that is exactly
    // what serde does between the two worlds: `time_sig_num` ⇄ `timeSigNum`,
    // `Clap` ⇄ `clap`.
    let mut out = BTreeSet::new();
    let mut word = String::new();
    let mut in_string = false;
    for c in text.chars() {
        if c == '"' {
            in_string = !in_string;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.insert(normalise(&word));
            word.clear();
        }
    }
    if !word.is_empty() {
        out.insert(normalise(&word));
    }
    out
}

/// The spelling serde would compare: no case, no underscores.
fn normalise(word: &str) -> String {
    word.chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Drop `//` comments, so prose cannot pass for a reader.
///
/// ⛔ **The gate was counting its own documentation as code.** The module claims
/// it compares against what the *source* names, and it was reading whole files
/// including every comment — so `distortion`, `clipping`, `transient` and
/// `drumLoopBars` "passed" purely because they are discussed in prose, with
/// zero code occurrences between them. In a codebase whose comments are this
/// dense that is an enormous false-pass surface.
///
/// ⚠ It also armed a landmine in the other direction: writing the word "reverb"
/// in any doc comment under the scanned trees would flip that key to "named"
/// and fail `the_deferral_list_does_not_outlive_what_it_defers`, demanding the
/// deletion of a debt entry that is still owed.
///
/// ⚠ Block comments are left alone deliberately — this codebase does not use
/// them, and a naive `/* */` strip would corrupt any string containing those
/// two characters.
fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            // ⚠ Only when the `//` is not inside a string literal. Counting
            // quotes is crude but exact for this codebase, where the only `//`
            // inside a string is a URL — and a URL is not a dataset key.
            Some(at) if line[..at].matches('"').count() % 2 == 0 => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_rust(dir: &Path, into: &mut String) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_rust(&path, into);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(text) = fs::read_to_string(&path) {
                into.push_str(&strip_comments(&text));
                into.push('\n');
            }
        }
    }
}

#[test]
fn every_authored_key_is_read_by_something() {
    let authored = authored();
    let named = named_in_source();
    let deferred: BTreeSet<&str> = DEFERRED.iter().map(|(key, _)| *key).collect();

    let unread: Vec<String> = authored
        .iter()
        .filter(|(key, _)| !named.contains(&normalise(key)))
        .filter(|(key, _)| !deferred.contains(key.as_str()))
        .map(|(key, example)| format!("  {key}  (e.g. {example})"))
        .collect();

    assert!(
        unread.is_empty(),
        "these keys are authored in the dataset and read by nothing:\n{}\n\n\
         A model asking for something the engine does not do is authored intent \
         doing nothing, and no other gate can see it — `$defs/partBlock` is \
         deliberately open, so the schema cannot. Either read the key, or add it \
         to `DEFERRED` in this file with a reason.",
        unread.join("\n")
    );
}

#[test]
fn the_deferral_list_does_not_outlive_what_it_defers() {
    // ⛔ **The other direction, and it is what stops the list rotting.** A key
    // implemented later, or renamed, or removed from every model, leaves an
    // entry here claiming a debt that no longer exists — and the next person
    // reads it as a to-do that is already done.
    let authored = authored();
    let named = named_in_source();

    let stale: Vec<&str> = DEFERRED
        .iter()
        .filter(|(key, _)| !authored.contains_key(*key) || named.contains(&normalise(key)))
        .map(|(key, _)| *key)
        .collect();

    assert!(
        stale.is_empty(),
        "these are on the deferral list but are no longer unread-and-authored: {stale:?}\n\
         Remove them — an entry that claims a debt nobody owes is read as a \
         to-do that is already done."
    );
}

#[test]
fn every_deferral_carries_a_reason() {
    // A bare key on the list is a note to nobody. The reason is what lets the
    // next person decide whether to implement it.
    for (key, why) in DEFERRED {
        assert!(
            why.len() > 12,
            "`{key}` is deferred without saying why (`{why}`)"
        );
    }
}

#[test]
fn the_two_that_started_this_are_read_now() {
    // ⛔ Regression pins for the pair that made this file necessary. Both were
    // authored across the dataset with no reader; both are wired now, and this
    // is what says so if either is ever unwired.
    let named = named_in_source();
    assert!(named.contains("percs"), "the percussion block must be read");
    assert!(
        named.contains("preview"),
        "`kit.preview` must be read — models name the kit they are heard through"
    );
}
