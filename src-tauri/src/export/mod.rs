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

/// Write bytes into the session dir under a safe name.
pub fn write_session_file(stem: &str, extension: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = session_dir()?;
    let path = dir.join(format!("{}.{}", safe_stem(stem), safe_stem(extension)));
    write_atomic(&path, bytes)?;
    Ok(path)
}

/// Write via a temp file and rename, so a crash mid-write cannot leave a
/// half-written `.mid` that a DAW will happily try to open.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("part");
    fs::write(&tmp, bytes)?;
    // Windows will not rename onto an existing file.
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(
            !path.with_extension("part").exists(),
            "the temp file must be gone"
        );

        // Overwriting an existing file must work — Windows rename does not
        // clobber by default.
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");

        let _ = fs::remove_file(&path);
    }
}
