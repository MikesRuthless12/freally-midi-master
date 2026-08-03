//! Getting a song out of the plugin as a file (TASK-073).
//!
//! ## ⛔ Why this is a thread and a mailbox rather than one blocking call
//!
//! A native Save As dialog is **modal and blocking**. The bridge answers an
//! HTTP call from the webview over the custom protocol, and that handler runs on
//! a frame the page is waiting on — `editor.rs` already documents that it sits
//! in an `extern "C"` frame where a panic cannot even unwind. Opening a modal
//! dialog there stalls the webview for as long as the producer is browsing for
//! a folder, and in a host that is the DAW's own editor thread: no crash, no
//! error, and no way out but killing the DAW. That is the exact failure class
//! this project has been bitten by twice and writes down in capitals.
//!
//! So the command **starts** an export and returns immediately. A detached
//! thread owns the dialog and the write, and drops its outcome into a one-slot
//! mailbox the page reads on its next poll. The page shows "Choose a
//! folder…" while that is happening, which is also the honest readout — the
//! dialog really is what it is waiting for.
//!
//! ## One at a time
//!
//! A second export while one is open is refused rather than queued. Two native
//! dialogs from one plugin is a window a producer cannot explain, and "export
//! twice quickly" is not a thing anybody means to do.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use engine::pattern::Song;
use serde::Serialize;

/// How an export ended, for the page to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Nothing has been exported since the last time the page looked.
    Idle,
    /// A dialog is open, or the bytes are being written.
    Running,
    /// Written. The path is shown to the producer, because a file they cannot
    /// find is a file they did not get.
    Done {
        path: String,
    },
    /// The producer closed the dialog. **Not an error** — it is the ordinary
    /// way out of a Save As, and reporting it as a failure would train people
    /// to ignore the one message that matters.
    Cancelled,
    Failed {
        reason: String,
    },
}

/// The one slot. See the module header for why there is only one.
static STATUS: Mutex<Status> = Mutex::new(Status::Idle);

/// Start writing `song` to a file the producer picks.
///
/// Returns as soon as the thread is running. `suggested` is the file name the
/// dialog opens with — the artist and the seed, so a folder full of exports is
/// readable without opening any of them.
pub fn start_song_midi(song: &Song, suggested: &str) -> Result<(), String> {
    // Encoded **here, on the caller's thread**, and deliberately: the bytes are
    // a pure function of the song and this is the last place that still has a
    // borrow of it. Sending the `Song` to the thread instead would mean cloning
    // an arrangement that can be megabytes, and doing the encoding somewhere
    // nothing is waiting for it buys nothing.
    let bytes = engine::midi::song_to_smf(song);
    let name = sanitize(suggested);

    {
        let mut slot = STATUS
            .lock()
            .map_err(|_| "the export state is unusable".to_owned())?;
        if *slot == Status::Running {
            return Err("an export is already open — finish that one first".to_owned());
        }
        *slot = Status::Running;
    }

    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_file_name(&name)
            .add_filter("MIDI file", &["mid"])
            .save_file();
        let status = match picked {
            Some(path) => write(&with_extension(path), &bytes),
            None => Status::Cancelled,
        };
        if let Ok(mut slot) = STATUS.lock() {
            *slot = status;
        }
    });

    Ok(())
}

/// Write one file per part into a folder the producer picks (TASK-069).
///
/// ## ⛔ MIDI stems, not audio stems, and the reason is measured
///
/// TASK-069 asks for per-part **wavs**. The plugin cannot honestly produce them
/// yet: `audio::Kit::pad_for` returns `None` for Melody, Counter, Bass and
/// Chords, because the preview kit is a *drum* kit — `lib.rs` says so in as
/// many words and points at FMM-N15/FMM-N16 for the pitched voices. Rendering
/// the four melodic parts today would write four silent files and call them
/// stems, which is worse than not offering them: a producer would import them,
/// hear nothing, and blame their DAW.
///
/// So this writes what the plugin actually has — one **type-0 `.mid` per
/// part**, which the roadmap's own entry lists beside the wavs ("per-part
/// type-0 clips alongside"). The audio half arrives with the pitched voices.
///
/// ## What "aligned to bar 1, identical lengths" means here
///
/// Every file is the whole song's timeline with one part in it, so they line up
/// by construction: dropping all five onto a DAW at bar 1 reassembles the
/// arrangement. That is the property the wav half would have needed too, and
/// flattening is what gives it — see `Song::flatten_parts`.
pub fn start_song_stems(song: &Song, folder_name: &str) -> Result<(), String> {
    // Encoded on the caller's thread, for the same reason `start_song_midi`
    // does it: the bytes are a pure function of the song, and cloning a whole
    // arrangement onto the dialog thread would buy nothing.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for part in engine::pattern::PART_ORDER {
        let flat = song.flatten_parts(Some(&[part]));
        // A part nothing plays gets no file. An empty stem is a file a producer
        // imports, hears nothing from, and has to work out was always empty.
        if flat.note_count() == 0 {
            continue;
        }
        files.push((
            // The names US-011 already gives the SMF's tracks, so a folder of
            // stems and a multi-track export read the same.
            format!("FMM {}.mid", part_name(part)),
            engine::midi::pattern_to_smf(&flat),
        ));
    }
    if files.is_empty() {
        return Err("this song plays nothing, so there are no stems to write".to_owned());
    }

    let stem_dir = sanitize(folder_name);

    {
        let mut slot = STATUS
            .lock()
            .map_err(|_| "the export state is unusable".to_owned())?;
        if *slot == Status::Running {
            return Err("an export is already open — finish that one first".to_owned());
        }
        *slot = Status::Running;
    }

    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new().pick_folder();
        let status = match picked {
            Some(root) => write_stems(&root.join(&stem_dir), &files),
            None => Status::Cancelled,
        };
        if let Ok(mut slot) = STATUS.lock() {
            *slot = status;
        }
    });

    Ok(())
}

/// ⚠ **Into a sub-folder of the one that was picked, not into it directly.**
/// Five files dropped into somebody's Desktop is a mess they have to clean up
/// by hand, and it makes "which of these go together" unanswerable once a
/// second song is exported.
fn write_stems(dir: &Path, files: &[(String, Vec<u8>)]) -> Status {
    if let Err(error) = std::fs::create_dir_all(dir) {
        return Status::Failed {
            reason: format!("could not create {}: {error}", dir.display()),
        };
    }
    for (name, bytes) in files {
        if let Err(error) = std::fs::write(dir.join(name), bytes) {
            return Status::Failed {
                reason: format!("could not write {name}: {error}"),
            };
        }
    }
    Status::Done {
        path: dir.display().to_string(),
    }
}

/// The name US-011 gives this part's track, so a stem file and an SMF track
/// agree.
fn part_name(part: engine::pattern::Part) -> &'static str {
    use engine::pattern::Part;
    match part {
        Part::Drums => "Drums",
        Part::Chords => "Chords",
        Part::Melody => "Melody",
        Part::Counter => "Counter",
        Part::Bass => "Bass",
    }
}

/// Read the outcome, and clear it if there is one.
///
/// ⛔ **Taken rather than merely read.** The page polls this, so a terminal
/// status left in place would re-announce the same successful export on every
/// tick — a toast that will not go away. `Running` stays, because it is not an
/// outcome.
pub fn take_status() -> Status {
    let Ok(mut slot) = STATUS.lock() else {
        return Status::Failed {
            reason: "the export state is unusable".to_owned(),
        };
    };
    match &*slot {
        Status::Running => Status::Running,
        other => {
            let taken = other.clone();
            *slot = Status::Idle;
            taken
        }
    }
}

fn write(path: &PathBuf, bytes: &[u8]) -> Status {
    match std::fs::write(path, bytes) {
        Ok(()) => Status::Done {
            path: path.display().to_string(),
        },
        Err(error) => Status::Failed {
            reason: format!("could not write {}: {error}", path.display()),
        },
    }
}

/// Make sure the file really is a `.mid`.
///
/// ⚠ Every platform's dialog *offers* the extension from the filter and none of
/// them guarantees it: a producer who types `verse-idea` gets a file with no
/// extension, which a DAW's import browser then hides. Added only when it is
/// missing, so `beat.mid` does not become `beat.mid.mid`.
fn with_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mid"))
    {
        return path;
    }
    path.with_extension("mid")
}

/// A suggested name that cannot be a path.
///
/// ⛔ The name comes from the page — the artist id and the seed — and a `Song`
/// arrives at the bridge as JSON from the webview, so `artist_id` is whatever
/// that JSON said. Without this, `../../..` in an artist id would move the
/// dialog's starting directory, and a name carrying a separator is refused
/// outright by some platform dialogs and silently truncated by others.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // `..` survives the filter above because `.` is legal in a file name — it
    // is the one sequence that means something to a path rather than to a name.
    let cleaned = cleaned.replace("..", "-");

    // ⚠ Runs are collapsed, and it is not only tidiness: each replaced
    // character leaves its own dash, so `a/../../b` came out as `a-----b`. A
    // suggested name is the first thing a producer reads in the dialog, and one
    // that looks corrupted reads as the export having gone wrong already.
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }

    let trimmed = out.trim_matches(['-', '.']).to_owned();
    if trimmed.is_empty() {
        "song.mid".to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_could_be_a_path_is_reduced_to_a_name() {
        // ⛔ The artist id reaches here from the webview, so this is a trust
        // boundary in the same sense `check_song` is. A separator in a dialog's
        // suggested name is refused outright by some platforms and silently
        // truncated by others; a traversal moves the starting directory.
        assert_eq!(sanitize("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize("trap/../../x"), "trap-x");
        assert_eq!(sanitize("osamason-2291.mid"), "osamason-2291.mid");
        assert_eq!(sanitize("uk drill 7"), "uk-drill-7");
    }

    #[test]
    fn an_unusable_name_still_produces_a_file_name() {
        // A dialog opened with an empty name shows the folder and no file, which
        // reads as the export having failed before it started.
        assert_eq!(sanitize(""), "song.mid");
        assert_eq!(sanitize("///"), "song.mid");
        assert_eq!(sanitize(".."), "song.mid");
    }

    #[test]
    fn the_extension_is_added_once_and_only_when_missing() {
        assert_eq!(
            with_extension(PathBuf::from("beat")),
            PathBuf::from("beat.mid")
        );
        assert_eq!(
            with_extension(PathBuf::from("beat.mid")),
            PathBuf::from("beat.mid")
        );
        // Case-insensitively, because Save As on Windows and macOS will hand
        // back whatever the producer typed.
        assert_eq!(
            with_extension(PathBuf::from("beat.MID")),
            PathBuf::from("beat.MID")
        );
        // A name with an unrelated extension keeps its stem and becomes a .mid,
        // rather than becoming `beat.wav.mid`.
        assert_eq!(
            with_extension(PathBuf::from("beat.wav")),
            PathBuf::from("beat.mid")
        );
    }

    #[test]
    fn a_terminal_status_is_taken_once_and_running_is_not() {
        // ⛔ The page polls, so a `Done` left in the slot would re-announce the
        // same export on every tick — a toast that never goes away.
        *STATUS.lock().unwrap() = Status::Done {
            path: "C:/x/beat.mid".into(),
        };
        assert!(matches!(take_status(), Status::Done { .. }));
        assert_eq!(take_status(), Status::Idle);

        *STATUS.lock().unwrap() = Status::Running;
        assert_eq!(take_status(), Status::Running);
        assert_eq!(take_status(), Status::Running, "running is not an outcome");
        *STATUS.lock().unwrap() = Status::Idle;
    }

    #[test]
    fn stems_land_in_their_own_folder_under_the_one_that_was_picked() {
        // ⚠ Five files dropped straight into somebody's Desktop is a mess they
        // clean up by hand, and it makes "which of these go together"
        // unanswerable the moment a second song is exported.
        let root = std::env::temp_dir().join("fmm-stem-test");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("trap-7-stems");

        let files = vec![
            ("FMM Drums.mid".to_owned(), vec![0x4d, 0x54, 0x68, 0x64]),
            ("FMM Melody.mid".to_owned(), vec![0x4d, 0x54, 0x68, 0x64]),
        ];
        let status = write_stems(&dir, &files);

        assert!(matches!(status, Status::Done { .. }), "{status:?}");
        assert!(dir.join("FMM Drums.mid").is_file());
        assert!(dir.join("FMM Melody.mid").is_file());
        // Nothing was written beside the folder.
        let loose = std::fs::read_dir(&root)
            .expect("the root exists")
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(loose, 0, "a stem landed outside its folder");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_song_that_plays_nothing_is_refused_rather_than_writing_an_empty_folder() {
        // A folder of nothing is a successful-looking export the producer then
        // has to work out was always empty.
        let empty = engine::pattern::Song {
            id: "s".into(),
            artist_id: "trap".into(),
            seed: 1,
            bpm: 140.0,
            key_root: 0,
            scale: engine::pattern::Scale::NaturalMinor,
            sections: vec![],
            time_sig_num: 4,
            time_sig_den: 4,
            patterns: Default::default(),
            ppq: engine::pattern::PPQ,
        };
        let err = start_song_stems(&empty, "trap-1-stems").unwrap_err();
        assert!(err.contains("plays nothing"), "{err}");
        // And it did not leave the mailbox claiming an export is running.
        assert_eq!(take_status(), Status::Idle);
    }

    #[test]
    fn cancelling_is_not_a_failure() {
        // Closing a Save As is the ordinary way out of it. Reporting it as an
        // error trains people to ignore the one message that matters.
        *STATUS.lock().unwrap() = Status::Cancelled;
        let taken = take_status();
        assert_eq!(taken, Status::Cancelled);
        assert!(!matches!(taken, Status::Failed { .. }));
        *STATUS.lock().unwrap() = Status::Idle;
    }
}
