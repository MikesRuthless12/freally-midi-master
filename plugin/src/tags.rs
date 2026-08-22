//! Tags on samples, one-shots and MIDI files (TASK-058C).
//!
//! The other half of the entry favourites closed in August. A star answers
//! *"come back to this one"*; a tag answers *"show me the ones like this"*, and
//! a producer with four thousand files needs both.
//!
//! ## ⛔ Keyed by path, exactly as [`crate::favourites`] is, and for one more reason
//!
//! The entry specifies **content hash** so that moving a file does not lose its
//! tags. `favourites.rs` records at length why that is unaffordable there — the
//! tree draws a star on every row, so deciding by content means hashing every
//! file in a folder on the host's editor thread. Tags inherit that argument
//! whole, because the entry also asks to **filter the tree by tag**: a row that
//! can be filtered out is a row whose tags had to be known.
//!
//! ⚠ **So a tagged file that is moved loses its tags**, and that is the same
//! stated limitation the star carries. The cure is the same one too: apply the
//! import pipeline's hash when a *lookup fails*, rather than on every row.
//!
//! ## ⚠ What is bounded here, and why each bound exists
//!
//! This file is read on the page's mount and rewritten on every edit, and it
//! arrives from disk where anything could have edited it:
//!
//! - [`MAX_TAGGED`] files, so the whole map can live on the page and the tree
//!   can filter without asking the plugin per row.
//! - [`MAX_PER_FILE`] tags on one file. A file with two hundred tags has no tags.
//! - [`MAX_TAG_LEN`] characters, so a tag stays a chip rather than a paragraph.
//!
//! ⛔ **Nothing here launches a process or opens a file.** Unlike
//! `favourites::reveal`, a tag is a string beside a path — so the guard surface
//! is the path itself: [`crate::oneshot::refuse_remote`] before the store is
//! touched, and containment applied by the caller that holds the explorer.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// How many files may carry tags.
const MAX_TAGGED: usize = 500;

/// How many tags one file may carry.
const MAX_PER_FILE: usize = 12;

/// How long one tag may be, in characters.
///
/// ⚠ Characters rather than bytes: a producer tagging in Japanese gets the same
/// budget as one tagging in English, and truncating a `char_indices` boundary is
/// how a store ends up holding invalid UTF-8.
const MAX_TAG_LEN: usize = 32;

/// One tagged file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tagged {
    /// The full path, which is the identity — see the module header.
    pub path: String,
    /// Its tags, in the order the producer added them.
    pub tags: Vec<String>,
}

/// What is written to `tags.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    tagged: Vec<Tagged>,
}

fn path_of_store() -> Option<std::path::PathBuf> {
    crate::presets::data_dir().map(|dir| dir.join("tags.json"))
}

/// Every tagged file on this machine.
///
/// ⚠ **Per user, not per project** — the same distinction favourites, the EULA
/// and the library roots all make. A tag is a property of this person's library,
/// and shipping one inside a project sent to a label would say something about
/// their folders they did not mean to send.
pub fn list() -> Vec<Tagged> {
    path_of_store()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|stored| stored.tagged)
        .map(|mut tagged| {
            // ⛔ **Cleaned on the way IN, not only on the way out.** The file is
            // ordinary JSON on the producer's disk; a hand-edited one holding a
            // thousand tags on a file must not become a thousand chips on
            // screen, and must not be written back that way by the next edit.
            for entry in &mut tagged {
                entry.tags = clean(&entry.tags);
            }
            tagged.retain(|entry| !entry.tags.is_empty());
            tagged.truncate(MAX_TAGGED);
            tagged
        })
        .unwrap_or_default()
}

/// The tags a producer typed, as they will be stored.
///
/// Trimmed, empties dropped, over-long ones cut at a character boundary, and
/// deduplicated **case-insensitively while keeping the first spelling** — so
/// `808` and `808 ` are one tag, and a producer who wrote `Vocal` first does not
/// find it renamed to `vocal` by typing it again later.
fn clean(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag: String = tag.trim().chars().take(MAX_TAG_LEN).collect();
        if tag.is_empty() {
            continue;
        }
        if out.iter().any(|held| held.eq_ignore_ascii_case(&tag)) {
            continue;
        }
        out.push(tag);
        if out.len() == MAX_PER_FILE {
            break;
        }
    }
    out
}

fn write(tagged: &[Tagged]) -> Result<(), String> {
    let path = path_of_store().ok_or("this platform has no per-user data directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {parent:?}: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&Stored {
        tagged: tagged.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    // Temp-and-rename, for the reason `favourites::write` gives: this rewrites
    // the whole file on every edit, and `list()`'s `unwrap_or_default` would read
    // a truncated one as *no tags at all*.
    crate::patterns::write_atomic(&path, &text)
}

/// Set `path`'s tags to exactly `tags`, and answer the whole store.
///
/// ⛔ **An empty list removes the entry rather than storing one.** The same rule
/// unstarring follows: nothing is tombstoned, so a producer who clears a file's
/// tags gets that file's row back out of the store and out of the count.
///
/// ⚠ **Containment is the caller's**, because it holds the
/// [`crate::explorer::Explorer`] — `editor::rpc` applies `contains` before
/// calling this, so only a file the browser would list can be tagged.
pub fn set(path: &str, tags: &[String]) -> Result<Vec<Tagged>, String> {
    let file = Path::new(path);
    crate::oneshot::refuse_remote(file)?;
    if engine::formats::kind_of(file).is_none() {
        // Refused rather than tagged as "something", for the reason
        // `favourites::add` gives: a tag on a file the app cannot open sends the
        // producer to a dead end with a label on it.
        return Err("that is not a file this plugin can use".into());
    }

    let tags = clean(tags);
    let mut tagged = list();
    // ⚠ The index, not an `iter_mut().find()`: the empty case removes the row,
    // and holding a mutable borrow into the vector while shortening it does not
    // compile — which is the honest reading of "these two arms do opposite
    // things to the same list".
    let at = tagged.iter().position(|held| held.path == path);
    match (at, tags.is_empty()) {
        (Some(index), true) => {
            tagged.remove(index);
        }
        (Some(index), false) => tagged[index].tags = tags,
        // Clearing the tags of a file that has none is not a write.
        (None, true) => return Ok(tagged),
        (None, false) => {
            if tagged.len() >= MAX_TAGGED {
                return Err(format!(
                    "you can tag {MAX_TAGGED} files — clear one file's tags to tag another"
                ));
            }
            tagged.push(Tagged {
                path: path.to_owned(),
                tags,
            });
        }
    }
    write(&tagged)?;
    Ok(tagged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_path_is_refused_before_the_store_is_touched() {
        // ⛔ The same first guard every path command in this plugin takes. On
        // Windows a UNC path is an outbound authentication rather than a read,
        // and `set` would otherwise reach it through `kind_of`.
        let error = set(r"\\evil.example.com\share\kick.wav", &["drums".into()])
            .expect_err("a network path must not be tagged");
        assert!(
            error.contains("network path"),
            "it must be refused as remote, not as something else: {error}"
        );
    }

    #[test]
    fn something_that_is_not_a_sample_or_a_midi_file_cannot_be_tagged() {
        let error = set("C:/notes.txt", &["drums".into()]).expect_err("a text file is not taggable");
        assert!(error.contains("can use"), "{error}");
    }

    #[test]
    fn tags_are_trimmed_deduplicated_case_insensitively_and_bounded() {
        let cleaned = clean(&[
            "  808  ".into(),
            "808".into(),
            "808".into(),
            "".into(),
            "   ".into(),
            "Vocal".into(),
            "vocal".into(),
        ]);
        // ⚠ The FIRST spelling survives: a producer who wrote `Vocal` does not
        // find it renamed by typing `vocal` later.
        assert_eq!(cleaned, vec!["808".to_owned(), "Vocal".to_owned()]);

        let many: Vec<String> = (0..40).map(|n| format!("tag{n}")).collect();
        assert_eq!(clean(&many).len(), MAX_PER_FILE);
    }

    #[test]
    fn an_over_long_tag_is_cut_on_a_character_boundary() {
        // ⛔ Characters, not bytes. Cutting a multi-byte character in half is how
        // a store ends up holding a string that will not deserialize.
        let long = "あ".repeat(MAX_TAG_LEN + 10);
        let cleaned = clean(&[long]);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].chars().count(), MAX_TAG_LEN);
    }

    #[test]
    fn the_store_survives_a_round_trip_and_an_older_file_still_reads() {
        let text = serde_json::to_string(&Stored {
            tagged: vec![Tagged {
                path: "C:/samples/kick.wav".into(),
                tags: vec!["drums".into(), "808".into()],
            }],
        })
        .expect("it serialises");
        let back: Stored = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back.tagged[0].tags, vec!["drums".to_owned(), "808".to_owned()]);

        let empty: Stored = serde_json::from_str("{}").expect("an older file still reads");
        assert!(empty.tagged.is_empty());
    }
}
