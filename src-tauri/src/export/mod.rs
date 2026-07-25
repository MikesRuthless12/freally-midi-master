//! Getting a generated file out of the app and into a DAW.
//!
//! Two routes, and the second is not a consolation prize:
//!
//! 1. **Native drag** — pick the clip up and drop it on the timeline. Proven on
//!    Windows, macOS and X11; Wayland is the open question TASK-013 exists to
//!    settle (PRD § 15 Q1).
//! 2. **Export folder** — write the file somewhere the user chose and reveal it
//!    in their file manager. Always present, always works, and on a platform
//!    where drag turns out to be unreliable it becomes the default with the
//!    export chip relabelled rather than a feature quietly failing.

pub mod drag;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Where a file was written, and by which route.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    /// Bytes written — a zero-length export is a bug worth seeing.
    pub bytes: u64,
}

/// The per-session temp directory drag sources are written to.
///
/// Drag needs a real file on disk that outlives the IPC call, and it must not
/// litter the user's own folders. Scoped per process so two running copies
/// cannot fight over one path.
pub fn session_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join("freally-midi-master")
        .join(format!("session-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Strip anything that cannot safely be a filename.
///
/// Names are built from artist ids and user text, so this is the boundary
/// between "a name" and "a path". `..` and separators must never survive it.
pub fn safe_stem(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | ' ' => c,
            _ => '-',
        })
        .collect();
    // Trim after substitution, not before: every disallowed character has
    // already become '-', so a name of "..." arrives here as "---". Trimming
    // only dots would leave that as the filename.
    let trimmed = cleaned.trim().trim_matches(|c| c == '-' || c == '.').trim();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// How many exports the session directory keeps.
///
/// A drop hands the DAW a path in a temp directory, and some hosts reference
/// that file rather than copying it — so the last few exports have to outlive
/// the drag. Twenty is enough that going back to something from earlier in a
/// session works, and few enough that a long session cannot fill a disk.
pub const KEEP_EXPORTS: usize = 20;

/// Write bytes into the session dir under a safe name.
pub fn write_session_file(stem: &str, extension: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = session_dir()?;
    let path = dir.join(format!("{}.{}", safe_stem(stem), safe_stem(extension)));
    write_atomic(&path, bytes)?;
    // Pruned here rather than on quit: a crash or a kill never reaches a quit
    // handler, and that is exactly the session that generated hundreds of
    // exports. Bounded at all times costs one directory read per export.
    prune_exports(&dir, KEEP_EXPORTS);
    Ok(path)
}

/// Keep the newest `keep` exports and delete the rest.
///
/// Best-effort by design: a file another process still holds open cannot be
/// removed on Windows, and failing an export because tidying failed would be
/// the wrong trade entirely.
pub fn prune_exports(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "mid"))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();

    if files.len() <= keep {
        return;
    }

    // Newest first, then drop everything past the limit.
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
}

/// Write via a temp file and rename, so a crash mid-write cannot leave a
/// half-written `.mid` that a DAW will happily try to open.
///
/// The temp name is unique per writer. A fixed `.part` suffix looks fine until
/// two writers target the same file: the first one's rename consumes the temp
/// file out from under the second, which then fails with ENOENT. That is not
/// hypothetical — it turned up as a macOS CI failure the moment two tests
/// exported the same spike file concurrently, and the app will eventually
/// export from more than one place at once.
///
/// Nothing is deleted first, either. `fs::rename` replaces the destination on
/// every platform this ships to — on Windows std uses SetFileInformationByHandle
/// with ReplaceIfExists — so removing the target beforehand only opens a window
/// where the user's file is gone and the replacement is not yet in place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = format!(
        "{}.{}.{}.part",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = path.with_file_name(unique);

    fs::write(&tmp, bytes)?;

    // Windows refuses a replace with ERROR_ACCESS_DENIED or a sharing violation
    // whenever anything else holds a handle on the destination for a moment —
    // another writer mid-replace, the search indexer, an antivirus scanner. It
    // is a timing artefact rather than a permissions problem: the identical call
    // succeeds milliseconds later. POSIX rename has no such window, so this loop
    // is a no-op there.
    //
    // Found by the concurrency test above, on Windows CI, against the fix that
    // introduced it — eight threads replacing one path is exactly the contention
    // that provokes it.
    let mut attempt = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt += 1;
                if attempt >= RENAME_ATTEMPTS {
                    // Never leave our temp file behind if the rename lost a race.
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
                std::thread::sleep(RENAME_BACKOFF);
            }
        }
    }
}

/// Up to ~100 ms of retrying, which is far longer than any real contention
/// window and still imperceptible to a user waiting on an export.
const RENAME_ATTEMPTS: u32 = 20;
const RENAME_BACKOFF: std::time::Duration = std::time::Duration::from_millis(5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writers_to_one_path_do_not_trip_over_each_other() {
        // Two separate bugs have lived here, so the contention is deliberately
        // heavier than anything the app does:
        //
        //  1. A fixed `.part` name — the first writer's rename consumed the temp
        //     file the second was still using (ENOENT, first seen on macOS CI).
        //  2. No retry around the rename — Windows refuses a replace with
        //     ERROR_ACCESS_DENIED while another writer holds the destination
        //     (seen on Windows CI, against the fix for the first bug).
        //
        // 16 writers × 8 replaces each provokes both within a second.
        let dir = session_dir().unwrap();
        let path = dir.join("concurrent-test.mid");

        std::thread::scope(|scope| {
            for i in 0..16 {
                let path = path.clone();
                scope.spawn(move || {
                    for round in 0..8 {
                        write_atomic(&path, format!("payload {i}").as_bytes())
                            .unwrap_or_else(|e| panic!("writer {i} round {round} failed: {e}"));
                    }
                });
            }
        });

        // Whichever won, the file must exist and hold one complete payload.
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("payload "), "got {content:?}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_session_directory_stays_bounded() {
        // A long session generating hundreds of clips must not fill the disk,
        // and the newest must survive — a prune that kept the *oldest* twenty
        // would delete the clip the user just dragged.
        let dir = session_dir().unwrap().join("prune-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        for i in 0..(KEEP_EXPORTS + 15) {
            let path = dir.join(format!("clip-{i:03}.mid"));
            fs::write(&path, b"MThd").unwrap();
            // Timestamps have coarse resolution on some filesystems, so order
            // is established explicitly rather than trusted to write speed.
            let when = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 60);
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(when).unwrap();
        }

        // A file that is not an export is not ours to delete.
        fs::write(dir.join("notes.txt"), b"keep me").unwrap();

        prune_exports(&dir, KEEP_EXPORTS);

        let mut remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        remaining.sort();

        assert!(
            remaining.contains(&"notes.txt".to_string()),
            "{remaining:?}"
        );
        let mids: Vec<&String> = remaining.iter().filter(|n| n.ends_with(".mid")).collect();
        assert_eq!(mids.len(), KEEP_EXPORTS, "{mids:?}");
        // The newest survived and the oldest did not.
        assert!(
            mids.iter().any(|n| n.as_str() == "clip-034.mid"),
            "{mids:?}"
        );
        assert!(
            !mids.iter().any(|n| n.as_str() == "clip-000.mid"),
            "{mids:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_separators_cannot_survive_a_stem() {
        for nasty in ["../../etc/passwd", "a/b", "a\\b", "C:evil"] {
            let s = safe_stem(nasty);
            assert!(!s.contains('/'), "{s}");
            assert!(!s.contains('\\'), "{s}");
            assert!(!s.contains(':'), "{s}");
            assert!(!s.contains(".."), "{s}");
        }
    }

    #[test]
    fn ordinary_names_survive_intact() {
        assert_eq!(safe_stem("osamason drums 4bar"), "osamason drums 4bar");
        assert_eq!(safe_stem("uk-drill_808"), "uk-drill_808");
    }

    #[test]
    fn an_empty_or_dotty_name_becomes_untitled() {
        assert_eq!(safe_stem(""), "untitled");
        assert_eq!(safe_stem("   "), "untitled");
        assert_eq!(safe_stem("..."), "untitled");
    }

    #[test]
    fn names_are_bounded() {
        assert!(safe_stem(&"x".repeat(500)).len() <= 64);
    }

    #[test]
    fn writing_is_atomic_and_leaves_no_part_file() {
        let dir = session_dir().unwrap();
        let path = dir.join("atomic-test.mid");
        write_atomic(&path, b"MThd-ish").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"MThd-ish");

        // No temp file for THIS path may survive, whatever it was called.
        // Scoped to our own filename: other tests share this directory and may
        // legitimately have a `.part` file in flight while we look.
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("atomic-test.mid.") && name.ends_with(".part"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        // Overwriting an existing file must work, and the file must never stop
        // existing on the way — no delete-then-rename gap.
        write_atomic(&path, b"second").unwrap();
        assert!(path.exists());
        assert_eq!(fs::read(&path).unwrap(), b"second");

        let _ = fs::remove_file(&path);
    }
}
