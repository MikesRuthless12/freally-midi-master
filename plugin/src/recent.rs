//! The files the browser has opened lately (TASK-058).
//!
//! Mike, 2026-08-12: *"even if there is a change in version of the app/vst/clap,
//! then i still want the folders to persist and the history to persist"*, and
//! then *"the history needs to persist with the new versions and so does the
//! file explorer's folder's list."*
//!
//! ⛔ **The folder list already survived a version change; the history did not
//! exist at all.** `explorer::library_path` writes `library.json` into
//! [`crate::presets::data_dir`], which is `%APPDATA%` plus the product name with
//! no version anywhere in it — so nothing had to be done for the folders. There
//! was no recent-files list anywhere in the plugin or the page to make persist.
//! This is it, written into the same directory so it inherits the same property
//! rather than being given its own.
//!
//! ## ⛔ Newest first, and the same file twice is one entry
//!
//! A history that grows a duplicate every time you audition the same kick is a
//! history you cannot read. Re-opening a file **moves it to the front** rather
//! than appending, which is what "recent" means everywhere else a producer uses
//! the word.
//!
//! ## ⚠ Recorded on *audition*, not on drop
//!
//! `explorer::select` is where a file is previewed — a click, an arrow key, or a
//! starred row — so that is the moment a producer has actually looked at
//! something. Recording on drop instead would miss everything they auditioned
//! and rejected, which is most of what browsing is.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many files the history keeps.
///
/// ⛔ Thirty, which is the roadmap's own number for this (TASK-058, *"Recent
/// Files (last 30)"*). It is also a bound for the reason `MAX_FAVOURITES` is
/// one: this file is rewritten on every audition and read back from disk, where
/// anything could have edited it.
const MAX_RECENT: usize = 30;

/// One file the browser opened.
///
/// The same shape as [`crate::favourites::Favourite`] on purpose — the page
/// draws both lists with one row component, and two shapes would be two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    /// The full path, which is both the identity and the way back to it.
    pub path: String,
    /// The file's own name, so the list reads without parsing paths on the page.
    pub name: String,
    /// `"audio"` or `"midi"`.
    pub kind: String,
}

/// What is written to `recent.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    recent: Vec<Recent>,
}

fn path_of_store() -> Option<PathBuf> {
    crate::presets::data_dir().map(|dir| dir.join("recent.json"))
}

/// The history, newest first.
///
/// ⚠ **Per user, not per project**, the distinction `eula.rs` wrote down and
/// `explorer::library_path` and `favourites` both follow: where somebody has
/// been browsing is a fact about this person on this machine, and shipping it
/// inside a project sent to a label would say something they did not mean to.
pub fn list() -> Vec<Recent> {
    path_of_store()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|stored| stored.recent)
        .map(|mut recent| {
            recent.truncate(MAX_RECENT);
            recent
        })
        .unwrap_or_default()
}

fn write(recent: &[Recent]) -> Result<(), String> {
    let path = path_of_store().ok_or("this platform has no per-user data directory")?;
    let text = serde_json::to_string_pretty(&Stored {
        recent: recent.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    // Temp-and-rename, for the reason `favourites::write` gives: a crash
    // mid-write leaves a truncated file that reads back as an empty history.
    crate::patterns::write_atomic(&path, &text)
}

/// Record that `path` was opened, moving it to the front.
///
/// ⚠ **Failure is deliberately swallowed by the caller.** This runs on every
/// audition, and a producer who cannot write to `%APPDATA%` should still be able
/// to browse and play their samples — a history is a convenience, and refusing
/// the preview because it could not be recorded would trade the feature for the
/// bookkeeping.
pub fn note(path: &str) -> Result<Vec<Recent>, String> {
    let file = Path::new(path);
    crate::oneshot::refuse_remote(file)?;

    let kind = match engine::formats::kind_of(file) {
        Some(engine::formats::Kind::Audio) => "audio",
        Some(engine::formats::Kind::Midi) => "midi",
        // Anything else is not a file the browser lists, so it cannot have been
        // auditioned — and a history that can hold a `.txt` is one that can send
        // you to a file the app cannot open.
        None => return Err("that is not a file this plugin can use".into()),
    };

    let mut recent = list();
    // Newest first, and one entry per file: re-auditioning promotes rather than
    // appends. See the module header.
    recent.retain(|held| held.path != path);
    recent.insert(
        0,
        Recent {
            path: path.to_owned(),
            name: file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned(),
            kind: kind.to_owned(),
        },
    );
    recent.truncate(MAX_RECENT);
    write(&recent)?;
    Ok(recent)
}

/// Forget everything.
///
/// ⚠ It is a list of where somebody has been. Being able to clear it is not a
/// convenience.
pub fn clear() -> Result<Vec<Recent>, String> {
    write(&[])?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_sits_beside_the_rest_of_the_user_data() {
        // ⛔ The whole point of the file: `%APPDATA%\Freally MIDI Master`, with no
        // version segment, so an update cannot orphan it. If `data_dir` ever
        // starts naming a version this fails rather than silently resetting
        // everybody's history on the next release.
        let Some(data) = crate::presets::data_dir() else {
            return;
        };
        let store = path_of_store().expect("there is a data dir, so there is a store");
        assert!(store.starts_with(&data));
        assert_eq!(store.file_name().unwrap(), "recent.json");
    }

    #[test]
    fn re_opening_a_file_promotes_it_rather_than_duplicating_it() {
        // Pure list arithmetic, so it does not touch the disk: `note` is the
        // same three lines over `list()`'s result.
        let mut recent = vec![
            Recent {
                path: "a".into(),
                name: "a".into(),
                kind: "audio".into(),
            },
            Recent {
                path: "b".into(),
                name: "b".into(),
                kind: "audio".into(),
            },
        ];
        let again = Recent {
            path: "b".into(),
            name: "b".into(),
            kind: "audio".into(),
        };
        recent.retain(|held| held.path != again.path);
        recent.insert(0, again);
        assert_eq!(
            recent.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"],
            "the file just opened leads, and appears once"
        );
    }

    #[test]
    fn the_history_is_capped_at_the_roadmaps_thirty() {
        let mut recent: Vec<Recent> = (0..40)
            .map(|i| Recent {
                path: format!("{i}"),
                name: format!("{i}"),
                kind: "audio".into(),
            })
            .collect();
        recent.truncate(MAX_RECENT);
        assert_eq!(recent.len(), 30);
    }
}
