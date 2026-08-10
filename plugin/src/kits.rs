//! Named kits: a set of one-shot assignments, saved under a name (TASK-051).
//!
//! A producer's one-shots already persist **with the project** — reopen a song
//! and the samples come back. This is the other half: the same set of
//! assignments, named, kept outside any project, so a kit built on one track can
//! be put on the next one. The difference is exactly the one
//! [`crate::presets`] draws between "this song" and "any song", and this file
//! follows it deliberately.
//!
//! ## What a saved kit is, and what it is not
//!
//! ⛔ **It stores paths, not audio.** A kit is a few hundred bytes naming files
//! that already exist on the producer's disk. Copying the samples is a separate,
//! *consented* thing that belongs to a style — see
//! [`crate::models::copy_samples`] — and doing it here silently would spend
//! somebody's disk space on a Save button, which is the thing the owner
//! explicitly ruled out on 2026-08-09.
//!
//! ⚠ **A kit that has lost a sample still loads the rest.** A producer who moved
//! one folder should get eleven of their twelve pads back rather than a refusal,
//! which is the same rule a reopened project already follows.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use engine::pattern::Lane;
use serde::{Deserialize, Serialize};

use crate::presets::{data_dir, is_safe_stem, slug_with};

/// A kit as it is stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Kit {
    /// What the producer reads. Not the id: renaming is a display change, and
    /// the two are allowed to disagree.
    pub name: String,
    /// Lane to the sample assigned to it.
    pub lanes: BTreeMap<Lane, String>,
}

/// A kit as the panel lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KitSummary {
    pub id: String,
    pub name: String,
    /// How many lanes it carries, so a producer can tell two apart at a glance.
    pub lanes: usize,
}

/// Where named kits are written.
fn kits_dir() -> Option<PathBuf> {
    data_dir().map(|base| base.join("kits"))
}

/// Every saved kit, by name.
pub fn list() -> Vec<KitSummary> {
    kits_dir().map(|dir| list_in(&dir)).unwrap_or_default()
}

fn list_in(dir: &Path) -> Vec<KitSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        // No directory yet simply means nothing has been saved.
        return Vec::new();
    };

    let mut found: Vec<KitSummary> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "json" {
                return None;
            }
            let id = path.file_stem()?.to_string_lossy().into_owned();
            if !is_safe_stem(&id) {
                return None;
            }
            let kit: Kit = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            Some(KitSummary {
                id,
                name: kit.name,
                lanes: kit.lanes.len(),
            })
        })
        .collect();

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The assignments a kit restores.
pub fn load(id: &str) -> Result<Vec<(Lane, String)>, String> {
    let dir =
        kits_dir().ok_or_else(|| "there is no user directory to read kits from".to_owned())?;
    load_in(&dir, id)
}

fn load_in(dir: &Path, id: &str) -> Result<Vec<(Lane, String)>, String> {
    if !is_safe_stem(id) {
        // Refused before it is ever joined to a path.
        return Err(format!("`{id}` is not a kit name"));
    }
    let text = fs::read_to_string(dir.join(format!("{id}.json")))
        .map_err(|error| format!("no kit `{id}`: {error}"))?;
    let kit: Kit =
        serde_json::from_str(&text).map_err(|error| format!("kit `{id}` is malformed: {error}"))?;
    Ok(kit.lanes.into_iter().collect())
}

/// Save these assignments under a name, replacing any kit with the same slug.
pub fn save(name: &str, lanes: BTreeMap<Lane, String>) -> Result<KitSummary, String> {
    let dir = kits_dir().ok_or_else(|| "there is no user directory to save kits to".to_owned())?;
    save_in(&dir, name, lanes)
}

fn save_in(dir: &Path, name: &str, lanes: BTreeMap<Lane, String>) -> Result<KitSummary, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a kit needs a name".to_owned());
    }
    if lanes.is_empty() {
        // ⛔ Refused rather than saved empty. A kit with nothing in it looks
        // identical to one whose samples all failed to load, and loading it
        // would silently do nothing — the readout-that-lies failure, in a list.
        return Err("there are no samples of your own to save yet".to_owned());
    }

    // ⚠ The same slug the presets and the pattern library use, because it is a
    // **security boundary** rather than tidiness: this name comes from a text
    // box in a webview and becomes a path inside somebody else's DAW.
    let id = slug_with(name, "kit");

    // ⛔⛔ **Two names that slug alike must not silently become one kit.**
    // `Take 1` and `Take-1` both slug to `take-1`, and an unconditional write
    // meant the second save deleted the first with nothing said — which is
    // word-for-word the failure the pattern library shipped and had to fix, and
    // which `models.rs` avoids by making the id *be* the name. A kit's name is
    // free text, so it cannot do that; refusing is the honest alternative.
    //
    // ⚠ Saving over your **own** name still replaces it, which is the point of
    // a Save button. Only a *different* name landing on the same slug is
    // refused — and `rename` and `duplicate` inherit the protection, which is
    // what stops either destroying an unrelated kit.
    if let Ok(existing) = fs::read_to_string(dir.join(format!("{id}.json"))) {
        if let Ok(kit) = serde_json::from_str::<Kit>(&existing) {
            if kit.name != name {
                return Err(format!(
                    "a kit called `{}` already goes by that name — pick another",
                    kit.name
                ));
            }
        }
    }

    fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;

    let kit = Kit {
        name: name.to_owned(),
        lanes,
    };
    let text = serde_json::to_string_pretty(&kit)
        .map_err(|error| format!("could not serialise the kit: {error}"))?;
    crate::patterns::write_atomic(&dir.join(format!("{id}.json")), &text)?;

    Ok(KitSummary {
        id,
        name: kit.name,
        lanes: kit.lanes.len(),
    })
}

/// Give a saved kit a different name.
///
/// ⛔ **Writes the new file before removing the old one**, so a failure leaves
/// the producer with their kit under its old name rather than with neither.
pub fn rename(id: &str, name: &str) -> Result<KitSummary, String> {
    let dir =
        kits_dir().ok_or_else(|| "there is no user directory to rename kits in".to_owned())?;
    rename_in(&dir, id, name)
}

fn rename_in(dir: &Path, id: &str, name: &str) -> Result<KitSummary, String> {
    let lanes: BTreeMap<Lane, String> = load_in(dir, id)?.into_iter().collect();
    refuse_landing_on_another(dir, id, name)?;

    let saved = save_in(dir, name, lanes)?;
    if saved.id != id {
        let _ = fs::remove_file(dir.join(format!("{id}.json")));
    }
    Ok(saved)
}

/// Refuse a rename or copy that would land on a kit other than `from`.
///
/// ⛔ **`save_in`'s own collision check is not enough here, and the difference
/// is subtle enough to be worth stating.** That one refuses a *different name*
/// landing on an existing slug — which is right for a Save button, where
/// re-using your own name means "replace mine". But renaming `Trap A` to
/// `Trap B` when a `Trap B` already exists arrives with the names *matching*,
/// so it sails through and overwrites a kit the producer never mentioned — and
/// `rename` then deletes the source, turning two kits into one with no error.
fn refuse_landing_on_another(dir: &Path, from: &str, name: &str) -> Result<(), String> {
    let target = slug_with(name, "kit");
    if target == from {
        return Ok(());
    }
    if dir.join(format!("{target}.json")).exists() {
        return Err(format!(
            "there is already a kit called `{name}` — pick another name"
        ));
    }
    Ok(())
}

/// Copy a saved kit under a new name, leaving the original alone.
pub fn duplicate(id: &str, name: &str) -> Result<KitSummary, String> {
    let dir = kits_dir().ok_or_else(|| "there is no user directory to copy kits in".to_owned())?;
    duplicate_in(&dir, id, name)
}

fn duplicate_in(dir: &Path, id: &str, name: &str) -> Result<KitSummary, String> {
    let lanes: BTreeMap<Lane, String> = load_in(dir, id)?.into_iter().collect();
    // ⛔ A copy that lands on the original is not a copy — it is an overwrite
    // reported as success, with nothing new created.
    refuse_landing_on_another(dir, "", name)?;
    save_in(dir, name, lanes)
}

/// Forget a saved kit. The samples themselves are untouched — they are the
/// producer's files, and this only ever stored their names.
pub fn delete(id: &str) -> Result<(), String> {
    let dir =
        kits_dir().ok_or_else(|| "there is no user directory to delete kits from".to_owned())?;
    delete_in(&dir, id)
}

fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a kit name"));
    }
    fs::remove_file(dir.join(format!("{id}.json")))
        .map_err(|error| format!("could not delete `{id}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-kits-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a temp dir must be creatable");
        dir
    }

    fn two_lanes() -> BTreeMap<Lane, String> {
        BTreeMap::from([
            (Lane::Kick, "C:/packs/kick.wav".to_owned()),
            (Lane::Snare, "C:/packs/snare.wav".to_owned()),
        ])
    }

    #[test]
    fn a_saved_kit_comes_back_with_the_lanes_it_was_saved_with() {
        let dir = temp_dir("roundtrip");
        let saved = save_in(&dir, "My Kit", two_lanes()).expect("save");
        assert_eq!(saved.name, "My Kit");
        assert_eq!(saved.lanes, 2);

        let back: BTreeMap<Lane, String> = load_in(&dir, &saved.id).unwrap().into_iter().collect();
        assert_eq!(back, two_lanes());
    }

    #[test]
    fn an_empty_kit_is_refused_rather_than_saved_as_nothing() {
        // ⛔ A kit with nothing in it looks identical to one whose samples all
        // failed to load, and loading it would silently do nothing.
        let dir = temp_dir("empty");
        let error = save_in(&dir, "Nothing", BTreeMap::new()).unwrap_err();
        assert!(error.contains("no samples"), "{error}");
        assert!(list_in(&dir).is_empty());
    }

    #[test]
    fn a_name_that_could_not_be_a_filename_never_becomes_a_path() {
        // ⛔ A security boundary: the name arrives from a text box in a webview
        // and ends up joined to a directory inside somebody else's DAW.
        let dir = temp_dir("traversal");
        let saved = save_in(&dir, "../../.ssh/authorized_keys", two_lanes()).expect("save");
        assert!(is_safe_stem(&saved.id), "{}", saved.id);
        assert!(dir.join(format!("{}.json", saved.id)).exists());
        assert_eq!(list_in(&dir).len(), 1);
    }

    #[test]
    fn renaming_keeps_the_samples_and_leaves_one_kit_behind() {
        let dir = temp_dir("rename");
        let first = save_in(&dir, "Old Name", two_lanes()).expect("save");

        let renamed = rename_in(&dir, &first.id, "New Name").expect("rename");
        assert_eq!(renamed.name, "New Name");

        let all = list_in(&dir);
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].name, "New Name");

        let back: BTreeMap<Lane, String> =
            load_in(&dir, &renamed.id).unwrap().into_iter().collect();
        assert_eq!(back, two_lanes(), "a rename must not touch the samples");
    }

    #[test]
    fn duplicating_leaves_the_original_alone() {
        let dir = temp_dir("duplicate");
        let first = save_in(&dir, "Original", two_lanes()).expect("save");
        let copy = duplicate_in(&dir, &first.id, "Copy").expect("duplicate");

        assert_ne!(copy.id, first.id);
        assert_eq!(list_in(&dir).len(), 2);
        assert!(
            load_in(&dir, &first.id).is_ok(),
            "the original must survive"
        );

        // ⛔ And copying onto its own name is refused rather than reported as a
        // success that created nothing.
        assert!(duplicate_in(&dir, &first.id, "Original").is_err());
    }

    #[test]
    fn two_names_that_slug_alike_cannot_become_one_kit() {
        // ⛔⛔ **The pattern library's own failure, arriving in a new store.**
        // `808 Kit` and `808-Kit` both slug to `808-kit`; an unconditional write
        // meant the second save deleted the first silently. Refused by name now.
        let dir = temp_dir("collide");
        save_in(&dir, "808 Kit", two_lanes()).expect("the first name is free");

        let error = save_in(&dir, "808-Kit", two_lanes()).unwrap_err();
        assert!(error.contains("808 Kit"), "{error}");

        let all = list_in(&dir);
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].name, "808 Kit", "the first kit must survive");
    }

    #[test]
    fn renaming_onto_another_kits_name_is_refused_rather_than_destroying_it() {
        // ⛔ `rename` writes then deletes, so without the refusal above it would
        // overwrite the other kit's lanes *and* delete the source — two kits
        // becoming one, with no error.
        let dir = temp_dir("rename-collide");
        let a = save_in(&dir, "Trap A", two_lanes()).expect("a");
        save_in(
            &dir,
            "Trap B",
            BTreeMap::from([(Lane::Kick, "C:/packs/b.wav".to_owned())]),
        )
        .expect("b");

        assert!(rename_in(&dir, &a.id, "Trap B").is_err());

        let all = list_in(&dir);
        assert_eq!(all.len(), 2, "both kits must still be there: {all:?}");
        assert_eq!(
            load_in(&dir, "trap-b").unwrap().len(),
            1,
            "Trap B must keep its own lanes"
        );
    }

    #[test]
    fn saving_the_same_name_twice_replaces_it_rather_than_leaving_two() {
        let dir = temp_dir("replace");
        save_in(&dir, "One Kit", two_lanes()).expect("first");
        save_in(
            &dir,
            "One Kit",
            BTreeMap::from([(Lane::Kick, "C:/packs/other.wav".to_owned())]),
        )
        .expect("second");

        let all = list_in(&dir);
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].lanes, 1, "the second save is the one that stands");
    }

    #[test]
    fn a_deleted_kit_is_gone_and_a_missing_one_says_so() {
        let dir = temp_dir("delete");
        let saved = save_in(&dir, "Doomed", two_lanes()).expect("save");

        assert!(delete_in(&dir, &saved.id).is_ok());
        assert!(list_in(&dir).is_empty());
        assert!(load_in(&dir, &saved.id).is_err());
    }

    #[test]
    fn a_broken_file_is_skipped_and_the_others_still_list() {
        // FR-001's rule again: one bad file must not cost the producer the rest.
        let dir = temp_dir("broken");
        save_in(&dir, "Good", two_lanes()).expect("save");
        fs::write(dir.join("broken.json"), "{ not json").unwrap();

        let all = list_in(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Good");
    }
}
