//! Starred samples, one-shots and MIDI files (TASK-058C).
//!
//! Mike, 2026-08-10: *"i think we need a 'Favorites' tag with a 'Star' so that
//! way if we star one, it adds to a list that we can see so that way we can click
//! just on the name of that 'Starred Favorite' and it will take us to the exact
//! spot of that exact sample in the actual 'File Explorer' browser and if it's
//! not a folder that you still have in the 'File Explorer' then it should take
//! you there in Windows Explorer or the macOS Explorer."*
//!
//! ## ⛔ Keyed by path, and TASK-058C says hash — here is why it is not
//!
//! The roadmap entry specifies favourites keyed by **content hash**, so that
//! moving or renaming a file on disk does not lose the star. That is the better
//! property and it is not affordable *here*:
//!
//! - **The tree draws a star on every row.** Deciding starred-or-not by content
//!   would mean hashing every file in the folder being listed — up to
//!   [`crate::explorer::MAX_ENTRIES`] reads, on the editor thread, every time a
//!   branch opens. The listing is deliberately a `read_dir` and nothing more.
//! - **Mike's own description is about the path.** *"you can unstar them too and
//!   it will delete the memory of what folder they are in"* — the folder is the
//!   thing being remembered, which is what makes "take me to the exact spot" and
//!   the Explorer fallback possible at all.
//!
//! ⚠ **So a starred file that is moved loses its star**, and that is a real
//! limitation rather than an oversight. Recovering it needs the hash the import
//! pipeline already computes, applied when a star *fails* to resolve rather than
//! on every row — worth doing, and it is a different piece of work.
//!
//! ## ⛔⛔ Revealing a file launches a process out of somebody's DAW
//!
//! ⚠ **One of two places that does** — `crash::reveal` opens the crash folder
//! (TASK-093). The difference is what makes this one the dangerous half: **the
//! page supplies the path here** and cannot there, where the folder is computed.
//! Two guards, both structural:
//!
//! 1. **Only a path that is already starred can be revealed.** The page cannot
//!    name an arbitrary string; it can only ask for one of its own entries, and
//!    an entry only got there from a row the explorer listed — which is itself
//!    bounded by the library.
//! 2. **[`crate::oneshot::refuse_remote`] first**, before any syscall, for the
//!    reason that guard documents: on Windows a UNC path handed to the shell is
//!    an outbound authentication, not a read.
//!
//! ⚠ And no shell. `Command::new("explorer")` passes its arguments as argv, so
//! there is no string for a quote or an `&` in a filename to break out of.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many favourites a producer may keep.
///
/// ⚠ A bound for the same reason `MAX_ROOTS` is one: this file is read on every
/// listing and written on every star, and it arrives from disk where anything
/// could have edited it.
const MAX_FAVOURITES: usize = 500;

/// One starred file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Favourite {
    /// The full path, which is both the identity and the way back to it.
    pub path: String,
    /// The file's own name, so the list reads without parsing paths on the page.
    pub name: String,
    /// `"audio"` or `"midi"` — the list shows the same two kinds the tree does.
    ///
    /// ⛔ Mike asked for all three: *"for samples/one shots/midi"*. A starred
    /// `.mid` has to be as reachable as a starred one-shot.
    pub kind: String,
}

/// What is written to `favourites.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    favourites: Vec<Favourite>,
}

fn path_of_store() -> Option<PathBuf> {
    crate::presets::data_dir().map(|dir| dir.join("favourites.json"))
}

/// Everything starred on this machine.
///
/// ⚠ **Per user, not per project** — the same distinction `eula.rs` records and
/// `explorer::library_path` now follows. A favourite is a property of this
/// person's library, and shipping one inside a project sent to a label would say
/// something about their folders that they did not mean to send.
pub fn list() -> Vec<Favourite> {
    path_of_store()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|stored| stored.favourites)
        .map(|mut favourites| {
            favourites.truncate(MAX_FAVOURITES);
            favourites
        })
        .unwrap_or_default()
}

fn write(favourites: &[Favourite]) -> Result<(), String> {
    let path = path_of_store().ok_or("this platform has no per-user data directory")?;
    let text = serde_json::to_string_pretty(&Stored {
        favourites: favourites.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    // ⛔ **Temp-and-rename, not a bare write.** This rewrites the whole list on
    // every star, and a crash mid-write leaves a truncated file — which
    // `list()`'s `unwrap_or_default` then reads as *no favourites at all*. The
    // producer would lose every star with nothing to say why. `kits`, `models`
    // and `patterns` already go through this for the same shape of data.
    crate::patterns::write_atomic(&path, &text)
}

/// Star `path`. Idempotent — starring twice is one entry, not two.
///
/// ⚠ **Containment is the caller's**, because it holds the [`crate::explorer::Explorer`]:
/// `editor::rpc` applies `contains` before calling this, so only a file the
/// browser would list can be starred. Re-taking that check here would need a
/// second reference to the explorer for no extra safety.
pub fn add(path: &str) -> Result<Vec<Favourite>, String> {
    let file = Path::new(path);
    crate::oneshot::refuse_remote(file)?;

    let kind = match engine::formats::kind_of(file) {
        Some(engine::formats::Kind::Audio) => "audio",
        Some(engine::formats::Kind::Midi) => "midi",
        // ⛔ Refused rather than starred as "something": a favourites list that
        // can hold a `.txt` is one that can send you to a file the app cannot
        // open, which is the readout-that-lies failure with a star on it.
        None => return Err("that is not a file this plugin can use".into()),
    };

    let mut favourites = list();
    if favourites.iter().any(|held| held.path == path) {
        return Ok(favourites);
    }
    if favourites.len() >= MAX_FAVOURITES {
        return Err(format!(
            "you can keep {MAX_FAVOURITES} favourites — unstar one to add another"
        ));
    }

    favourites.push(Favourite {
        path: path.to_owned(),
        name: file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_owned(),
        kind: kind.to_owned(),
    });
    write(&favourites)?;
    Ok(favourites)
}

/// Unstar `path`.
///
/// ⛔ Mike: *"you can unstar them too and it will delete the memory of what
/// folder they are in for that specific folder."* The entry goes; nothing is
/// tombstoned.
pub fn remove(path: &str) -> Result<Vec<Favourite>, String> {
    let mut favourites = list();
    favourites.retain(|held| held.path != path);
    write(&favourites)?;
    Ok(favourites)
}

/// Show `path` in the OS file manager.
///
/// ⛔⛔ **Only a path that is already starred.** See the module header: this
/// launches a process, and the page must not be able to name the target. A path
/// that is not in the list is refused before anything runs.
pub fn reveal(path: &str) -> Result<(), String> {
    if !list().iter().any(|held| held.path == path) {
        return Err("that file is not one of your favourites".into());
    }
    let file = Path::new(path);
    // ⛔ Before any syscall, including the one the shell would make.
    crate::oneshot::refuse_remote(file)?;
    reveal_in_shell(file)
}

#[cfg(target_os = "windows")]
fn reveal_in_shell(file: &Path) -> Result<(), String> {
    // ⚠ **`/select,<path>` as ONE argument**, which is the form Explorer wants —
    // and passed through argv rather than a shell, so a filename containing a
    // quote or an `&` is a filename and not syntax.
    let mut argument = std::ffi::OsString::from("/select,");
    argument.push(file);
    std::process::Command::new("explorer")
        .arg(argument)
        // ⚠ Explorer answers non-zero even when it succeeds, so the exit status
        // is deliberately not checked — only whether the process started.
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open Explorer: {error}"))
}

#[cfg(target_os = "macos")]
fn reveal_in_shell(file: &Path) -> Result<(), String> {
    // `-R` reveals the file in Finder rather than opening it, which is the
    // difference between showing somebody where a sample is and playing it in
    // whatever app owns `.wav`.
    std::process::Command::new("open")
        .arg("-R")
        .arg(file)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open Finder: {error}"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_in_shell(file: &Path) -> Result<(), String> {
    // ⚠ No portable "reveal and select" on Linux, so the containing folder is
    // opened instead — the honest approximation rather than a claim to select a
    // row in a file manager this code cannot know.
    let folder = file.parent().unwrap_or(file);
    std::process::Command::new("xdg-open")
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open the file manager: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_path_is_refused_before_anything_runs() {
        // ⛔⛔ The guard that matters most in this module: `reveal` starts a
        // process, and on Windows handing the shell a UNC path is an outbound
        // authentication rather than a read.
        let error = add(r"\\evil.example.com\share\kick.wav")
            .expect_err("a network path must not be starred");
        assert!(error.contains("network path"), "{error}");

        let error = reveal(r"\\evil.example.com\share\kick.wav").expect_err("nor revealed");
        // Refused as "not a favourite" — it could never have been added, so it
        // does not reach the remote check, and either refusal is correct.
        assert!(!error.is_empty(), "it must be refused with a reason");
    }

    #[test]
    fn something_that_is_not_a_sample_or_a_midi_file_cannot_be_starred() {
        let error = add("C:/notes.txt").expect_err("a text file is not starrable");
        assert!(error.contains("can use"), "{error}");
    }

    #[test]
    fn a_path_that_was_never_starred_cannot_be_revealed() {
        // ⛔ The containment for the process launch. The page can only ask for
        // one of its own entries, and an entry only got there from a row the
        // explorer listed.
        let error =
            reveal("C:/somewhere/else/secret.wav").expect_err("an unstarred path must be refused");
        assert!(error.contains("not one of your favourites"), "{error}");
    }

    #[test]
    fn the_store_survives_a_round_trip_and_an_older_file_still_reads() {
        let text = serde_json::to_string(&Stored {
            favourites: vec![Favourite {
                path: "C:/samples/kick.wav".into(),
                name: "kick.wav".into(),
                kind: "audio".into(),
            }],
        })
        .expect("it serialises");
        let back: Stored = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back.favourites.len(), 1);
        assert_eq!(back.favourites[0].name, "kick.wav");

        let empty: Stored = serde_json::from_str("{}").expect("an older file still reads");
        assert!(empty.favourites.is_empty());
    }
}
