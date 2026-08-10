//! The producer's own style models — "Original Workflow" (TASK-040U).
//!
//! A shipped model is compiled into the binary ([`crate::dataset`]). A user
//! model is a file the producer saved, in the **same schema `datasetc`
//! validates**, so there is one format and one validator rather than a
//! second-class one for users. It appears in the roster beside the shipped
//! ones, generates through the same code, and inherits through `extends` like
//! everything else — which is what makes a user's "dark trap" gain whatever the
//! shipped `trap` learns later.
//!
//! ## Why this is a layer over the dataset rather than part of it
//!
//! [`crate::dataset::loaded`] is a `OnceLock` behind a `&'static`: parsed once,
//! never rebuilt, no lock on any read. Everything **mutable** lives here
//! instead, so that stays true — a producer saving a style must not be able to
//! take a write lock over the map every generation reads.
//!
//! ⚠ **This module's first draft justified the split as a real-time
//! constraint**, on `dataset.rs`'s claim that a host may call `initialize` on
//! the audio thread. That claim is retired (see the note on `LOADED`): it
//! contradicted `lib.rs`, which reads files from disk on the same path and says
//! so. The split is still right — a lock on the hot read path is worth avoiding
//! on its own merits — but nothing here should be read as a real-time guarantee,
//! because it is not one.
//!
//! ## Two rules that are invariants rather than validations
//!
//! - **A user model may not take an id a shipped model already uses.** Both
//!   ways of resolving that collision are wrong: letting the user model win
//!   silently replaces an artist the producer believes they are generating, and
//!   letting the shipped one win silently ignores work they saved. Refusing the
//!   save is the only answer that tells anybody anything.
//! - **An id must already be filename-safe**, and the editor derives it from the
//!   display name. That is what stops two ids slugging to one file — the failure
//!   the pattern library shipped and had to fix, where saving `Take 1` and
//!   `Take-1` left one file and lost a take. If the id *is* the stem, there is
//!   no mapping to collide.
//!
//! ## Resolution happens against the shipped set, not after it
//!
//! `extends` is most of the point: a user model that extends `trap` and
//! overrides four keys is exactly what the inheritance resolver was built for.
//! So a rebuild hands **embedded and user files together** to
//! `engine::dataset::load` and keeps the user half of the answer. Re-parsing the
//! shipped ~536 KB costs a few milliseconds and happens only on a save, a delete
//! or an import — never on a generation, and never on the audio thread.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use engine::dataset::{DatasetProblem, RosterEntry, StyleModel};
use engine::pattern::Lane;
use serde_json::Value;

use crate::presets::{data_dir, is_safe_stem};

/// The resolved user models, and everything that was skipped on the way in.
#[derive(Debug, Default)]
pub struct UserModels {
    /// Resolved, ready to generate from.
    pub models: BTreeMap<String, StyleModel>,
    /// How the roster lists them.
    pub entries: Vec<RosterEntry>,
    /// The ones that would not load, each with a reason a producer can act on.
    ///
    /// ⛔ Reported rather than dropped, exactly as a shipped model is: a style
    /// that silently vanishes from the roster is indistinguishable from one that
    /// was never saved, and the second is a bug someone has to be able to see.
    pub problems: Vec<DatasetProblem>,
}

/// The cached answer, rebuilt whenever the directory changes underneath it.
static CACHE: OnceLock<RwLock<Option<Arc<UserModels>>>> = OnceLock::new();

fn cache() -> &'static RwLock<Option<Arc<UserModels>>> {
    CACHE.get_or_init(|| RwLock::new(None))
}

/// Where user models are written.
pub fn user_dir() -> Option<PathBuf> {
    data_dir().map(|base| base.join("models"))
}

/// Every user model, resolved. Cheap after the first call.
pub fn all() -> Arc<UserModels> {
    if let Ok(guard) = cache().read() {
        if let Some(loaded) = guard.as_ref() {
            return Arc::clone(loaded);
        }
    }

    let built = Arc::new(user_dir().map(|dir| build_from(&dir)).unwrap_or_default());
    if let Ok(mut guard) = cache().write() {
        *guard = Some(Arc::clone(&built));
    }
    built
}

/// Drop the cache so the next read rebuilds.
///
/// Called after every write. Not public beyond the crate: a caller that could
/// invalidate without having written is a caller that can make the roster
/// flicker for no reason.
fn invalidate() {
    if let Ok(mut guard) = cache().write() {
        *guard = None;
    }
}

/// One resolved user model.
pub fn model(id: &str) -> Option<StyleModel> {
    all().models.get(id).cloned()
}

/// Read every `.json` in `dir` and resolve it against the shipped dataset.
fn build_from(dir: &Path) -> UserModels {
    let files = read_files(dir);
    if files.is_empty() {
        return UserModels::default();
    }

    let mine: Vec<String> = files
        .iter()
        .filter_map(|(_, text)| id_of(text))
        .filter(|id| !crate::dataset::loaded().models.contains_key(id))
        .collect();

    // ⛔ Loaded **with** the shipped set rather than beside it, so `extends`
    // reaches `trap`. Keeping only `mine` afterwards is what makes this a layer:
    // the shipped half of the answer is thrown away, because `dataset::loaded()`
    // already holds it and is the copy the audio path reads.
    let combined = engine::dataset::load(
        env!("CARGO_PKG_VERSION"),
        crate::dataset::entries().into_iter().chain(files),
    );

    let models = combined
        .models
        .into_iter()
        .filter(|(id, _)| mine.contains(id))
        .collect();
    let entries = combined
        .summary
        .entries
        .into_iter()
        .filter(|entry| mine.contains(&entry.id))
        // The loader cannot know where a file came from and deliberately does
        // not guess; `mine` is the one place that does.
        .map(mine_entry)
        .collect();
    // A problem is only ours if it names one of our ids or one of our files —
    // the shipped half's problems belong to `dataset::loaded()`, and reporting
    // them twice would double every badge.
    let problems = combined
        .summary
        .problems
        .into_iter()
        .filter(|problem| mine.contains(&problem.source) || problem.source.contains("<user>"))
        .collect();

    UserModels {
        models,
        entries,
        problems,
    }
}

/// The `.json` files in `dir`, as the loader wants them.
///
/// Paths are tagged `<user>` so a problem naming a file says which half of the
/// roster it came from — the shipped ones are `data/artists/…`.
fn read_files(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = fs::read_dir(dir) else {
        // No directory yet simply means nothing has been saved.
        return Vec::new();
    };

    let mut found: Vec<(PathBuf, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "json" {
                return None;
            }
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            if !is_safe_stem(&stem) {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;

            // ⛔ **The filename and the model's own id must agree.** Everything
            // downstream keys on the id read from the *content* — `delete` and
            // `export` both join `{id}.json` — so a hand-placed `backup.json`
            // declaring `"id": "dark-trap"` would appear in the roster,
            // generate happily, and then refuse to be deleted or exported: a
            // style you can see and use and never remove. Skipped here, so it
            // never reaches the roster in the first place.
            //
            // ⚠ Nothing `save` or `import` writes can hit this — they name the
            // file after the id — but a model file is explicitly something a
            // producer backs up and hands over, so it is reachable by hand.
            //
            // ⛔ **Only when it names a *different* id, never when it names
            // none.** A file that will not parse, or has no `id` at all, must
            // still go through so the loader reports it as a problem — dropping
            // it here would make an unreadable style vanish in silence, which is
            // the very thing `UserModels::problems` exists to prevent. Skipping
            // is for the one case that would otherwise list and then refuse to
            // be deleted.
            if id_of(&text).is_some_and(|id| id != stem) {
                return None;
            }
            Some((PathBuf::from(format!("<user>/{stem}.json")), text))
        })
        .collect();

    // `read_dir` promises no order and inheritance must not depend on one.
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The `id` a model file declares, if it declares one.
fn id_of(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// Save a model, replacing any user model with the same id.
///
/// Validated **before** anything is written, so a model that would be skipped on
/// the way back in is refused on the way out — with the reason, while the
/// producer is still looking at the editor that produced it.
pub fn save(raw: Value) -> Result<RosterEntry, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to save models to".to_owned())?;
    save_in(&dir, raw)
}

/// The id a candidate declares, once it has earned the right to be one.
///
/// ⛔ **Shared by save and train rather than checked twice.** Mike's rule for
/// training — *"you should only be able to train original artists/workflows"* —
/// is the same shipped-id refusal a save already makes, and two copies of a rule
/// that must never disagree is how they start to.
fn usable_id(raw: &Value) -> Result<String, String> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "a model needs an `id`".to_owned())?
        .to_owned();

    if !is_safe_stem(&id) {
        return Err(format!(
            "`{id}` cannot be a model id — use lower-case letters, digits and hyphens"
        ));
    }
    if id.starts_with('_') {
        return Err(format!(
            "`{id}` is reserved: ids beginning with `_` are internal"
        ));
    }
    if crate::dataset::loaded().models.contains_key(&id) {
        return Err(format!(
            "`{id}` is a shipped model — choose another id, or your version would replace it \
             everywhere without saying so"
        ));
    }
    Ok(id)
}

fn save_in(dir: &Path, raw: Value) -> Result<RosterEntry, String> {
    let id = usable_id(&raw)?;

    // ⛔ Resolved against the shipped set *and* every other user model before a
    // byte is written. This is what catches a bad `extends`, a cycle through
    // another saved model, and every lint `datasetc` runs in CI — at the moment
    // the producer can still fix it.
    let (entry, _) = resolve_one(dir, &id, &raw)?;
    write_model(dir, &id, &raw)?;
    invalidate();
    Ok(entry)
}

/// Serialise a validated model and put it on disk.
///
/// ⛔ **Through `patterns::write_atomic` rather than a second copy of it.** A
/// user model is authored work, not a cache, so a crash mid-write must not leave
/// half a file where a whole one was — and `patterns.rs` already owns that
/// dance, including the Windows caveat that a rename will not land on an
/// existing file. This module shipped with a verbatim copy for an afternoon;
/// `patterns.rs`'s own notes record what happened the last time a filesystem
/// helper was copied here rather than called, which was a hash constant
/// silently drifting.
fn write_model(dir: &Path, id: &str, raw: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(raw)
        .map_err(|error| format!("could not serialise the model: {error}"))?;
    fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    crate::patterns::write_atomic(&dir.join(format!("{id}.json")), &text)
}

/// Resolve one candidate alongside the shipped set and the saved user models.
///
/// ⛔ **Hands back both halves of the one load.** It used to answer only the
/// roster entry, and `train_in` needed the resolved model too — so a second,
/// near-identical function did the whole thing again, and `train` ended up
/// re-parsing the embedded ~536 KB **three times** for one press. `load`
/// already returns `models` beside `summary`; taking both is free.
fn resolve_one(
    dir: &Path,
    id: &str,
    raw: &Value,
) -> Result<(RosterEntry, engine::StyleModel), String> {
    let mut files = read_files(dir);
    // The candidate replaces its own saved version rather than colliding with it.
    files.retain(|(_, text)| id_of(text).as_deref() != Some(id));
    files.push((PathBuf::from(format!("<user>/{id}.json")), raw.to_string()));

    let combined = engine::dataset::load(
        env!("CARGO_PKG_VERSION"),
        crate::dataset::entries().into_iter().chain(files),
    );

    if let Some(problem) = combined
        .summary
        .problems
        .iter()
        .find(|problem| problem.source == id || problem.source.contains(&format!("/{id}.json")))
    {
        return Err(problem.message.clone());
    }

    let entry = combined
        .summary
        .entries
        .into_iter()
        .find(|entry| entry.id == id)
        .map(mine_entry)
        .ok_or_else(|| format!("`{id}` resolved to nothing the roster can list"))?;
    let model = combined
        .models
        .get(id)
        .cloned()
        .ok_or_else(|| format!("`{id}` listed in the roster but did not resolve"))?;

    Ok((entry, model))
}

/// Mark an entry as the producer's own.
///
/// ⛔ **One function, because a test claims there is one place.**
/// `dataset::nothing_shipped_is_marked_as_the_producers_own` says `mine` is set
/// in exactly one place — and it was being constructed in two, so the claim was
/// already aspirational the day it was written. Structural beats asserted.
fn mine_entry(entry: RosterEntry) -> RosterEntry {
    RosterEntry {
        mine: true,
        ..entry
    }
}

/// Delete a user model. Shipped models are compiled in and cannot be removed.
pub fn delete(id: &str) -> Result<(), String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to delete models from".to_owned())?;
    delete_in(&dir, id)
}

fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a model id"));
    }
    if crate::dataset::loaded().models.contains_key(id) {
        return Err(format!("`{id}` is a shipped model and cannot be deleted"));
    }

    let path = dir.join(format!("{id}.json"));
    fs::remove_file(&path).map_err(|error| format!("could not delete `{id}`: {error}"))?;
    invalidate();
    Ok(())
}

/// A model as a single file, so a vibe can be backed up or handed to someone.
pub fn export(id: &str) -> Result<String, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to read models from".to_owned())?;
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a model id"));
    }
    fs::read_to_string(dir.join(format!("{id}.json")))
        .map_err(|error| format!("no model `{id}`: {error}"))
}

/// Take a model file in. Validated exactly as a save is, because a file handed
/// over by someone else is the *least* trustworthy way one can arrive.
pub fn import(text: &str) -> Result<RosterEntry, String> {
    let raw: Value =
        serde_json::from_str(text).map_err(|error| format!("that is not a model file: {error}"))?;
    save(raw)
}

/// What copying a style's samples would cost the producer's disk.
///
/// ⛔ **Answered before anything is copied, because the producer has to be able
/// to say no.** Mike, 2026-08-09: *"ensure that the end user knows that creating
/// their own original artist adds copies of samples … and ensure that they want
/// to do that before allowing the app to copy the samples."* A style that keeps
/// its sounds is a style that owns a second copy of every one of them — which is
/// the right default for a style you can hand to someone, and absolutely not
/// something to do to somebody's drive without asking.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleCost {
    /// How many files would be copied. Duplicates are counted once.
    pub count: usize,
    /// How many bytes they come to.
    pub bytes: u64,
}

/// The most a single style's samples may come to.
///
/// ⛔ A bound because the paths arrive from a page and the files from a
/// producer's library. A kit of one-shots is a few megabytes; a quarter of a
/// gigabyte is somebody having assigned a stem or a whole recording, and copying
/// it silently is the failure this whole gate exists to prevent.
const MAX_COPY_BYTES: u64 = 256 * 1024 * 1024;

/// What copying these samples would take, so the producer can be asked first.
///
/// Unreadable and remote paths are skipped rather than refused: the question is
/// *"how much disk"*, and a file that cannot be read will not be copied either.
pub fn sample_cost(paths: &[String]) -> SampleCost {
    let mut seen: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for path in paths {
        let path = Path::new(path);
        if crate::oneshot::refuse_remote(path).is_err() {
            continue;
        }
        // ⚠ Keyed by path so the same sample on two lanes — a producer using one
        // clap for the clap and the snap — is one copy and is counted once.
        if let Ok(meta) = fs::metadata(path) {
            seen.insert(path.to_path_buf(), meta.len());
        }
    }

    SampleCost {
        count: seen.len(),
        bytes: seen.values().sum(),
    }
}

/// A style's copied samples, by the lane each one plays on.
///
/// ⛔⛔ **The lane is the whole point of this file.** Without it the copies are
/// sixteen hex-named files in a folder with nothing saying which is the kick —
/// which is exactly what shipped on 2026-08-09, and what made the consent
/// checkbox's promise false: the producer agreed, the bytes landed, and no code
/// path could ever put them back. `assigned_paths` threw the lane away one line
/// before the copy, so the information was lost at the last possible moment.
///
/// Names, not paths, because the directory is ours: a copy that recorded an
/// absolute path would break the moment the style was moved to another machine,
/// which is the case the copies exist to survive.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SampleIndex {
    lanes: BTreeMap<Lane, String>,
}

/// True when `name` is something [`copy_samples_in`] could have written.
///
/// ⛔ A whitelist, and it guards a **join**. The index is a file on disk, so it
/// is editable by anything running as the producer — an entry of `..\..\..` is
/// the obvious attempt, and the answer is to accept only the shape we emit:
/// `{16 hex}.{alphanumeric}`.
fn is_safe_sample_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    is_safe_stem(stem)
        && !extension.is_empty()
        && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
}

/// Where a style keeps the samples it owns.
fn samples_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join(id).join("samples")
}

/// The samples this style owns, ready for [`crate::oneshot::OneShots::load_kit`].
///
/// ⛔ **This is the half that was missing.** The copy landed and nothing read it
/// back, so a style's sounds did not come with it — Mike, 2026-08-09: *"when i
/// tried to clear the kick from the 'My EDM' drum lane, and tried to go to
/// something different, and then clicked on My EDM again, it didn't load the
/// kick drum back into the slot."*
///
/// Empty rather than an error when there is no index: most styles have no
/// samples of their own, and a producer selecting one should not be told about a
/// file they never asked to exist.
pub fn samples_for(id: &str) -> Vec<(Lane, String)> {
    let Some(dir) = user_dir() else {
        return Vec::new();
    };
    samples_for_in(&dir, id)
}

fn samples_for_in(dir: &Path, id: &str) -> Vec<(Lane, String)> {
    if !is_safe_stem(id) {
        return Vec::new();
    }
    let into = samples_dir(dir, id);
    let Ok(text) = fs::read_to_string(into.join("index.json")) else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_str::<SampleIndex>(&text) else {
        return Vec::new();
    };

    index
        .lanes
        .into_iter()
        .filter(|(_, name)| is_safe_sample_name(name))
        .map(|(lane, name)| (lane, into.join(name).to_string_lossy().into_owned()))
        // ⚠ A copy that is not there is dropped here rather than handed on, so
        // the loader's "every sample in that kit has moved" can only mean what
        // it says. `load_kit` skips unreadable files too; this keeps the two
        // from disagreeing about how many there were.
        .filter(|(_, path)| Path::new(path).exists())
        .collect()
}

/// Copy a style's samples into its own folder, and say where they landed.
///
/// ⛔ **Only ever called after the producer has agreed**, with the count and the
/// size in front of them. Nothing on this path asks; the caller does, and the
/// bridge command is separate from `user_model_save` for exactly that reason —
/// so that saving a style can never copy anything by accident.
///
/// Takes **(lane, path)** rather than paths: see [`SampleIndex`].
pub fn copy_samples(id: &str, pairs: &[(Lane, String)]) -> Result<Vec<String>, String> {
    let dir = user_dir().ok_or_else(|| "there is no user directory to copy into".to_owned())?;
    copy_samples_in(&dir, id, pairs)
}

fn copy_samples_in(dir: &Path, id: &str, pairs: &[(Lane, String)]) -> Result<Vec<String>, String> {
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a model id"));
    }

    let paths: Vec<String> = pairs.iter().map(|(_, path)| path.clone()).collect();
    let cost = sample_cost(&paths);
    if cost.bytes > MAX_COPY_BYTES {
        return Err(format!(
            "those samples come to {} MB, which is more than a style should carry",
            cost.bytes / (1024 * 1024)
        ));
    }

    let into = samples_dir(dir, id);
    fs::create_dir_all(&into)
        .map_err(|error| format!("could not create {}: {error}", into.display()))?;

    let mut landed = Vec::new();
    let mut index = SampleIndex::default();
    for (lane, path) in pairs {
        let from = Path::new(path);
        crate::oneshot::refuse_remote(from)?;
        let Ok(bytes) = fs::read(from) else {
            // ⚠ Skipped rather than fatal: one sample that has moved must not
            // cost the producer the other twelve.
            continue;
        };

        // ⛔ **Named by content, so the same file assigned twice is stored
        // once.** Dedupe is worth having *because we now own the copies* — when
        // the plugin merely referenced a path, the path was the identity and
        // there was nothing to dedupe. FNV rather than SHA-256, and the
        // difference matters enough to say: this is a *dedupe* key for our own
        // files, not a integrity check on somebody else's, and `presets::hash`
        // is already the stable one this codebase uses for filenames.
        let extension = from
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| ext.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("wav")
            .to_ascii_lowercase();
        let name = format!(
            "{:016x}.{extension}",
            crate::presets::hash(&String::from_utf8_lossy(&bytes))
        );

        let target = into.join(&name);
        if !target.exists() {
            fs::write(&target, &bytes)
                .map_err(|error| format!("could not copy {}: {error}", from.display()))?;
        }
        // ⚠ Recorded per lane, so two pads sharing one clap both point at the
        // single stored copy — the dedupe above and this are the same fact seen
        // from the two ends.
        index.lanes.insert(*lane, name.clone());
        if !landed.contains(&name) {
            landed.push(name);
        }
    }

    // ⛔ **Written even when nothing landed**, so a style whose every sample has
    // moved records an empty kit rather than keeping the last one that worked.
    // A stale index is the readout-that-lies failure in its file form.
    let text = serde_json::to_string_pretty(&index)
        .map_err(|error| format!("could not write the sample index: {error}"))?;
    crate::patterns::write_atomic(&into.join("index.json"), &text)?;

    Ok(landed)
}

/// How many seeds a trained model is measured over before it is saved.
///
/// ⛔⛔ **300, not the shipped roster's 1,000, and the reason is the thread.**
/// `user_model_train` is dispatched from `editor::rpc`, which answers the
/// webview **synchronously** — inside a host, that is the DAW's own editor
/// thread. At 1,000 seeds × one sweep per kept part this ran thousands of full
/// generations there: the host freezes for the duration, and the page's own
/// 15-second fetch timeout fires and throws the finished work away. That is
/// § 4.8's failure class — the one thing in this product that can take somebody's
/// session down — arriving through a button rather than a dialog.
///
/// ⚠ **The ratio is the claim, not the count.** `engine/tests/fit.rs` measures
/// at 300/150 for the same reason and says so; a grammar that reaches 150 in 300
/// has not saturated. [`engine::fit::verify_variety`] also stops the moment the
/// floor is cleared, so a healthy model costs far less than the worst case.
///
/// ▶ **The full 1,000/500 sweep belongs off-thread**, behind a status poll, the
/// way `export` and `one_shot_status` already do it. That is not built; until it
/// is, this number is what keeps the gate honest *and* the host alive.
const TRAIN_SEEDS: u64 = 300;
const TRAIN_FLOOR: usize = 150;

/// Fit a model to the generations a producer kept, and save it (TASK-040T).
///
/// ⛔ **Only a workflow they own, never a shipped artist.** That falls out of
/// [`usable_id`] rather than being a second rule here, and it is not a UI
/// nicety: a shipped model is a researched approximation of a real person, and
/// one retrained on a stranger's MIDI would carry that person's name while no
/// longer being that estimate of them. Shipped models are also replaced by
/// updates, so a producer's training would be silently overwritten on the next
/// release — the worst of both.
///
/// ⛔ **Measured before it is written.** A fit that cannot clear the variety
/// floor is reported back as something the producer can act on — keep more, or
/// keep more varied ones — rather than saved as a model that repeats itself.
pub fn train(
    id: &str,
    name: &str,
    base: &str,
    moods: &[String],
    kept: &[engine::pattern::Pattern],
) -> Result<RosterEntry, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to save models to".to_owned())?;
    train_in(&dir, id, name, base, moods, kept)
}

fn train_in(
    dir: &Path,
    id: &str,
    name: &str,
    base: &str,
    moods: &[String],
    kept: &[engine::pattern::Pattern],
) -> Result<RosterEntry, String> {
    let raw = engine::fit::fit(id, name, base, moods, kept).map_err(|error| error.to_string())?;
    let id = usable_id(&raw)?;

    // ⛔ Resolved **once**, and both halves are used: the entry is what the
    // roster gets back, the model is what the variety check generates from — a
    // fitted block on its own has no parent and no generators to run.
    let (entry, resolved) = resolve_one(dir, &id, &raw)?;

    for part in engine::fit::kept_by_part(kept).keys() {
        engine::fit::verify_variety(&resolved, *part, TRAIN_SEEDS, TRAIN_FLOOR)?;
    }

    // ⚠ Written directly rather than through `save_in`, which would resolve the
    // whole dataset a second time to re-derive the entry already in hand.
    write_model(dir, &id, &raw)?;
    invalidate();
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-models-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a temp dir must be creatable");
        dir
    }

    fn dark_trap() -> Value {
        json!({
            "id": "my-dark-trap",
            "type": "artist",
            "name": "My Dark Trap",
            "extends": ["trap"],
            "genres": ["rap", "trap"],
            "session": { "bpm": { "min": 128, "max": 140, "mode": 132 } }
        })
    }

    #[test]
    fn a_saved_model_comes_back_resolved_against_the_shipped_one_it_extends() {
        let dir = temp_dir("extends");
        let entry = save_in(&dir, dark_trap()).expect("a well-formed model must save");
        assert_eq!(entry.id, "my-dark-trap");
        assert_eq!(entry.name, "My Dark Trap");

        // ⛔ The whole point of `extends`: what it did not author it inherits.
        // `drums` is trap's, and it has to be there or a user model would
        // generate nothing but the four numbers it typed.
        let built = build_from(&dir);
        let model = built.models.get("my-dark-trap").expect("it must resolve");
        assert!(
            model.blocks.contains_key("drums"),
            "a user model must inherit its parent's blocks"
        );
        assert!(
            !engine::dataset::modes::modes_of(model).is_empty(),
            "and its parent's moods, which is what keeps a user style generatable"
        );
    }

    #[test]
    fn an_id_a_shipped_model_already_uses_is_refused_rather_than_shadowing_it() {
        // ⛔ Neither resolution is acceptable, which is why this is refused
        // instead: letting the user model win replaces an artist the producer
        // thinks they are generating, and letting the shipped one win ignores
        // work they saved. Both are silent.
        let dir = temp_dir("collide");
        let mut raw = dark_trap();
        raw["id"] = json!("trap");

        let error = save_in(&dir, raw).unwrap_err();
        assert!(error.contains("trap"), "{error}");
        assert!(error.contains("shipped"), "{error}");
    }

    #[test]
    fn an_id_that_could_not_be_a_filename_is_refused_before_it_becomes_a_path() {
        // ⛔ A security boundary, not tidiness: the id arrives from a text box
        // in a webview and ends up joined to a directory, inside somebody
        // else's DAW.
        let dir = temp_dir("traversal");
        for bad in ["../escape", "C:/somewhere", "My Style", "_defaults"] {
            let mut raw = dark_trap();
            raw["id"] = json!(bad);
            assert!(
                save_in(&dir, raw).is_err(),
                "`{bad}` must not be accepted as a model id"
            );
        }
    }

    #[test]
    fn a_model_that_would_not_load_is_refused_while_the_editor_is_still_open() {
        // A bad `extends` is the ordinary way to get this wrong, and finding out
        // at save time is the difference between a message and a style that
        // quietly never appears.
        let dir = temp_dir("badparent");
        let mut raw = dark_trap();
        raw["extends"] = json!(["no-such-genre"]);

        let error = save_in(&dir, raw).unwrap_err();
        assert!(error.contains("no-such-genre"), "{error}");
        assert!(
            !dir.join("my-dark-trap.json").exists(),
            "nothing may be written when the model is refused"
        );
    }

    #[test]
    fn saving_the_same_id_twice_replaces_it_rather_than_leaving_two() {
        let dir = temp_dir("replace");
        save_in(&dir, dark_trap()).expect("the first save");

        let mut second = dark_trap();
        second["name"] = json!("Renamed");
        save_in(&dir, second).expect("the second save");

        let built = build_from(&dir);
        assert_eq!(built.entries.len(), 1, "{:?}", built.entries);
        assert_eq!(built.entries[0].name, "Renamed");
    }

    #[test]
    fn a_broken_file_is_skipped_with_a_reason_and_the_others_still_load() {
        // FR-001's rule, applied to the user's own directory: one bad file must
        // not cost them the rest, and it must not vanish without a word either.
        let dir = temp_dir("broken");
        save_in(&dir, dark_trap()).expect("a good one");
        fs::write(dir.join("broken.json"), "{ not json").expect("plant a bad one");

        let built = build_from(&dir);
        assert!(built.models.contains_key("my-dark-trap"));
        assert_eq!(built.problems.len(), 1, "{:?}", built.problems);
        assert!(built.problems[0].source.contains("broken"));
    }

    #[test]
    fn a_file_whose_name_disagrees_with_its_id_never_reaches_the_roster() {
        // ⛔ Everything downstream keys on the id from the *content*, and
        // `delete`/`export` join `{id}.json` — so a file named one thing and
        // declaring another would list, generate, and then refuse to be removed.
        // A style you can see and use and never delete is worse than one that
        // never appeared.
        let dir = temp_dir("mismatch");
        save_in(&dir, dark_trap()).expect("a well-named one");

        let mut planted = dark_trap();
        planted["id"] = serde_json::json!("something-else");
        fs::write(
            dir.join("backup.json"),
            serde_json::to_string_pretty(&planted).unwrap(),
        )
        .unwrap();

        let built = build_from(&dir);
        assert!(built.models.contains_key("my-dark-trap"));
        assert!(
            !built.models.contains_key("something-else"),
            "a file whose name disagrees with its id must not list"
        );
    }

    #[test]
    fn a_deleted_model_is_gone_and_a_shipped_one_cannot_be_deleted() {
        let dir = temp_dir("delete");
        save_in(&dir, dark_trap()).expect("save");
        assert!(delete_in(&dir, "my-dark-trap").is_ok());
        assert!(build_from(&dir).models.is_empty());

        let error = delete_in(&dir, "trap").unwrap_err();
        assert!(error.contains("shipped"), "{error}");
    }

    /// `n` melodies from one shipped model, as a producer's kept set would be.
    fn kept(model_id: &str, n: usize) -> Vec<engine::pattern::Pattern> {
        use engine::context::{SessionContext, SessionOverrides};
        use engine::generators::{chords, drums, melody};

        let model = crate::dataset::model(model_id).expect("a shipped model to keep from");
        (1..=n as u64)
            .map(|seed| {
                let ctx = SessionContext::from_model(&model, &SessionOverrides::default(), seed);
                let harmony = chords::generate(&model, &ctx, seed);
                let kit = drums::generate(&model, &ctx, seed);
                engine::pattern::Pattern {
                    id: format!("{model_id}-{seed}"),
                    part: engine::pattern::Part::Melody,
                    artist_id: model_id.to_owned(),
                    seed,
                    song_seed: seed,
                    bars: 4,
                    bpm: 140.0,
                    time_sig_num: 4,
                    time_sig_den: 4,
                    key_root: ctx.key_root,
                    scale: ctx.scale,
                    lanes: vec![melody::generate(&model, &ctx, seed, &harmony, &kit)],
                    ppq: 960,
                    mood: None,
                    loop_region: None,
                    clip_region: None,
                }
            })
            .collect()
    }

    #[test]
    fn a_shipped_artist_can_never_be_trained_over() {
        // ⛔⛔ Mike: *"you should only be able to train original artists /
        // workflows"* — not Drake, not Future. Enforced here rather than in the
        // UI, for two reasons that are both about what a shipped model *is*: it
        // is a researched approximation of a real person, and it is replaced by
        // the next release. Retraining one would put a stranger's taste behind
        // that person's name, and then lose it on update.
        let dir = temp_dir("train-shipped");
        let error = train_in(&dir, "trap", "Not Trap", "trap", &[], &kept("trap", 30)).unwrap_err();

        assert!(error.contains("shipped"), "{error}");
        assert!(!dir.join("trap.json").exists(), "and nothing was written");
    }

    #[test]
    fn training_below_the_floor_says_how_far_short_it_is() {
        let dir = temp_dir("train-short");
        let error = train_in(&dir, "mine", "Mine", "trap", &[], &kept("trap", 9)).unwrap_err();

        assert!(error.contains('9'), "{error}");
        assert!(
            error.contains(&engine::fit::MIN_KEPT.to_string()),
            "the message has to carry the target, not just the shortfall: {error}"
        );
    }

    #[test]
    fn nothing_is_copied_until_the_producer_has_been_told_what_it_costs() {
        // ⛔⛔ **Mike's instruction, 2026-08-09**: *"ensure that the end user
        // knows that creating their own original artist adds copies of samples
        // … and ensure that they want to do that before allowing the app to
        // copy the samples."* The gate is structural rather than a prompt
        // somebody could forget to show: asking what it costs and doing it are
        // two functions, and `save` calls neither.
        let dir = temp_dir("consent");
        let library = temp_dir("consent-library");
        let sample = library.join("kick.wav");
        fs::write(&sample, vec![7u8; 2048]).unwrap();
        let paths = vec![sample.display().to_string()];
        let pairs = vec![(Lane::Kick, sample.display().to_string())];

        // Asking costs nothing on disk.
        let cost = sample_cost(&paths);
        assert_eq!(cost.count, 1);
        assert_eq!(cost.bytes, 2048);

        // And saving a style does not copy, however many samples are assigned.
        save_in(&dir, dark_trap()).expect("save");
        assert!(
            !dir.join("my-dark-trap").join("samples").exists(),
            "a save must not copy anything by itself"
        );

        // Only the explicit copy does.
        let landed = copy_samples_in(&dir, "my-dark-trap", &pairs).expect("copy");
        assert_eq!(landed.len(), 1);
        assert!(dir
            .join("my-dark-trap")
            .join("samples")
            .join(&landed[0])
            .exists());
    }

    #[test]
    fn a_styles_samples_come_back_on_the_lanes_they_were_copied_from() {
        // ⛔⛔ **The half that was missing, and the reason the consent text was a
        // lie.** Mike, 2026-08-09: *"when i tried to clear the kick from the 'My
        // EDM' drum lane, and tried to go to something different, and then
        // clicked on My EDM again, it didn't load the kick drum back into the
        // slot."* The copy worked; nothing read it back.
        let dir = temp_dir("readback");
        let library = temp_dir("readback-library");
        let kick = library.join("kick.wav");
        let snare = library.join("snare.wav");
        fs::write(&kick, vec![1u8; 64]).unwrap();
        fs::write(&snare, vec![2u8; 64]).unwrap();

        copy_samples_in(
            &dir,
            "my-dark-trap",
            &[
                (Lane::Kick, kick.display().to_string()),
                (Lane::Snare, snare.display().to_string()),
            ],
        )
        .expect("copy");

        let back = samples_for_in(&dir, "my-dark-trap");
        assert_eq!(back.len(), 2, "both lanes must come back");
        assert_eq!(back[0].0, Lane::Kick);
        assert_eq!(back[1].0, Lane::Snare);

        // ⚠ The paths must resolve — a lane naming a file that is not there is
        // the same silent failure one layer along.
        for (lane, path) in &back {
            assert!(Path::new(path).exists(), "{lane:?} points at nothing");
        }

        // And the two lanes must not have been handed the same file: the dedupe
        // is by content, and these differ.
        assert_ne!(back[0].1, back[1].1);
    }

    #[test]
    fn a_style_with_no_samples_of_its_own_asks_for_nothing() {
        // The common case. A producer selecting a style they never copied
        // samples for must not be told about a file they never made.
        let dir = temp_dir("no-samples");
        save_in(&dir, dark_trap()).expect("save");
        assert!(samples_for_in(&dir, "my-dark-trap").is_empty());
        assert!(samples_for_in(&dir, "nothing-here").is_empty());
    }

    #[test]
    fn a_tampered_index_cannot_reach_outside_the_styles_own_folder() {
        // ⛔ The index is a file on disk, so it is editable by anything running
        // as the producer. It is read back into a `join`, which is what makes
        // this a boundary rather than a validation.
        let dir = temp_dir("tampered");
        let into = dir.join("my-dark-trap").join("samples");
        fs::create_dir_all(&into).unwrap();
        fs::write(
            into.join("index.json"),
            r#"{"lanes":{"Kick":"../../../../../../Windows/System32/drivers/etc/hosts",
                         "Snare":"..\\..\\secrets.wav",
                         "Clap":"no-extension"}}"#,
        )
        .unwrap();

        assert!(
            samples_for_in(&dir, "my-dark-trap").is_empty(),
            "no entry that is not the shape we write may survive the read"
        );
    }

    #[test]
    fn re_copying_a_style_whose_samples_have_all_moved_leaves_no_stale_kit() {
        // ⚠ A stale index is the readout-that-lies failure in file form: the
        // style would keep loading the kit it had before the producer changed
        // it, and nothing on screen would say so.
        let dir = temp_dir("stale");
        let library = temp_dir("stale-library");
        let kick = library.join("kick.wav");
        fs::write(&kick, vec![9u8; 32]).unwrap();

        copy_samples_in(
            &dir,
            "my-dark-trap",
            &[(Lane::Kick, kick.display().to_string())],
        )
        .expect("the first copy");
        assert_eq!(samples_for_in(&dir, "my-dark-trap").len(), 1);

        // Everything the producer had assigned is gone by the second copy.
        copy_samples_in(
            &dir,
            "my-dark-trap",
            &[(Lane::Kick, library.join("gone.wav").display().to_string())],
        )
        .expect("the second copy");
        assert!(
            samples_for_in(&dir, "my-dark-trap").is_empty(),
            "the index must forget a lane whose sample no longer exists"
        );
    }

    #[test]
    fn one_sample_on_two_lanes_is_counted_once_and_stored_once() {
        // A producer using one clap for the clap and the snap is ordinary, and
        // telling them it costs twice what it does — then storing it twice —
        // would be wrong in both directions.
        let dir = temp_dir("dedupe");
        let library = temp_dir("dedupe-library");
        let sample = library.join("clap.wav");
        fs::write(&sample, vec![3u8; 1024]).unwrap();

        let twice = vec![sample.display().to_string(), sample.display().to_string()];
        assert_eq!(
            sample_cost(&twice),
            SampleCost {
                count: 1,
                bytes: 1024
            }
        );

        let landed = copy_samples_in(
            &dir,
            "my-dark-trap",
            &[
                (Lane::Clap, sample.display().to_string()),
                (Lane::Snap, sample.display().to_string()),
            ],
        )
        .expect("copy");
        assert_eq!(landed.len(), 1);

        // ⚠ Stored once, but *both* lanes must still find it — the dedupe is an
        // implementation detail of the folder, not something the producer's kit
        // should feel as a missing pad.
        let back = samples_for_in(&dir, "my-dark-trap");
        assert_eq!(back.len(), 2, "both lanes point at the single stored copy");
        assert_eq!(back[0].1, back[1].1);
    }

    #[test]
    fn a_sample_that_has_moved_does_not_cost_the_producer_the_others() {
        let dir = temp_dir("partial");
        let library = temp_dir("partial-library");
        let here = library.join("snare.wav");
        fs::write(&here, vec![1u8; 512]).unwrap();

        let paths = vec![
            here.display().to_string(),
            library.join("gone.wav").display().to_string(),
        ];
        assert_eq!(sample_cost(&paths).count, 1, "a missing file costs nothing");

        let landed = copy_samples_in(
            &dir,
            "my-dark-trap",
            &[
                (Lane::Snare, here.display().to_string()),
                (Lane::Kick, library.join("gone.wav").display().to_string()),
            ],
        )
        .expect("copy");
        assert_eq!(landed.len(), 1, "the one that is there still lands");

        // ⚠ And the lane whose file had gone is simply absent, rather than
        // recorded pointing at nothing.
        let back = samples_for_in(&dir, "my-dark-trap");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0, Lane::Snare);
    }

    #[test]
    fn the_copy_commands_take_no_paths_from_the_page() {
        // ⛔⛔ **A security review found this, and the fix is structural.** Both
        // commands shipped in `bridge.rs` for an afternoon taking a page-supplied
        // list of arbitrary filesystem paths: `refuse_remote` was applied and
        // containment was not, which is a clean per-path existence-and-exact-size
        // oracle over the whole disk handed back to an untrusted page, plus an
        // arbitrary local file read into a known folder.
        //
        // The fix was to stop the paths crossing the boundary at all — the
        // plugin already holds the assignments. **The guard that cannot be
        // forgotten is the one that has nothing to guard**, so this asserts the
        // shape rather than a validation: neither arm may mention `paths`.
        let editor = include_str!("editor.rs");
        let arms = editor
            .split("\"user_model_sample_cost\"")
            .nth(1)
            .and_then(|rest| rest.split("// Named kits").next())
            .expect("both copy arms live in editor.rs, between the marker comments");

        assert!(
            !arms.contains("request.args[\"paths\"]"),
            "a copy command is reading paths from the page again:\n{arms}"
        );
        assert!(
            arms.contains("assigned_paths(shared)"),
            "the copy commands must source their paths from the plugin's own map"
        );
        assert!(
            !include_str!("bridge.rs").contains("crate::models::copy_samples"),
            "the copy commands must stay where `shared` is, so they cannot be \
             re-written to take a path list"
        );
    }

    #[test]
    fn a_remote_path_is_never_read_let_alone_copied() {
        // ⛔ The same guard every other path-taking command carries: a UNC
        // string makes the SMB redirector authenticate outward on the first
        // syscall, so it must be refused before `metadata` is ever called.
        let paths = vec!["\\\\evil.example.com\\share\\kick.wav".to_owned()];
        assert_eq!(sample_cost(&paths), SampleCost::default());

        let dir = temp_dir("remote");
        let pairs = vec![(Lane::Kick, paths[0].clone())];
        assert!(copy_samples_in(&dir, "my-dark-trap", &pairs).is_err());

        // ⚠ And a refusal must leave nothing behind for the read-back to find:
        // a half-written index naming a UNC path would put the outbound
        // authentication one selection away instead of one copy away.
        assert!(samples_for_in(&dir, "my-dark-trap").is_empty());
    }

    #[test]
    fn a_model_survives_the_round_trip_through_a_file() {
        // Export and import are how a vibe moves between machines, so the same
        // seed has to reach the same output on the far side — which it does
        // only if the file that comes back resolves to the same model.
        let dir = temp_dir("roundtrip");
        save_in(&dir, dark_trap()).expect("save");

        let text = fs::read_to_string(dir.join("my-dark-trap.json")).expect("read it back");
        let reimported: Value = serde_json::from_str(&text).expect("it must parse");
        assert_eq!(reimported, dark_trap());

        let elsewhere = temp_dir("roundtrip-2");
        save_in(&elsewhere, reimported).expect("and load on another install");
        assert_eq!(
            build_from(&elsewhere).models.get("my-dark-trap"),
            build_from(&dir).models.get("my-dark-trap"),
            "the same file must resolve to the same model"
        );
    }
}
