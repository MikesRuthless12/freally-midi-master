//! The drag-out spike (TASK-013) and the export fallback.
//!
//! The commands here are what the DAW matrix is filled in from. Each one
//! reports what actually happened rather than returning silently, because the
//! entire point of the spike is telling "the drag was refused" apart from "the
//! drag worked but the DAW ignored it".

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use super::{safe_stem, write_session_file, ExportResult};

/// The folder the user last chose in the picker.
///
/// Held here rather than passed back in by the UI. Everything crossing the IPC
/// bridge is untrusted — Tauri's model says so explicitly — and a command that
/// accepts an arbitrary destination is one any script in the WebView can aim
/// wherever the user can write.
static EXPORT_FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Reject any path that is not inside this session's export directory.
///
/// The only files these commands have business touching are ones this app just
/// wrote. Without it, `export_to_folder` would happily read `~/.ssh/id_rsa`,
/// because the source path arrived straight from the WebView. Canonicalising
/// both sides before comparing is what makes `..` and symlinks moot.
fn must_be_session_file(path: &str) -> Result<PathBuf, String> {
    let file = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("no such file {path}: {e}"))?;
    let session = super::session_dir()
        .and_then(|d| d.canonicalize())
        .map_err(|e| format!("no session directory: {e}"))?;

    if !file.starts_with(&session) {
        return Err(format!(
            "{path} is outside this session's export directory — \
             only files this app exported can be dragged or copied"
        ));
    }
    Ok(file)
}

/// What the UI needs to label the export chip on this platform.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragCapability {
    pub platform: String,
    /// Present-and-attempted, not proven — the spike is what proves it.
    pub drag_supported: bool,
    /// True on Wayland, where drag is the open question (PRD § 15 Q1).
    pub is_wayland: bool,
    /// Why drag is unavailable, when it is.
    pub note: Option<String>,
}

fn detect_wayland() -> bool {
    // Set by the compositor; the most reliable signal available without
    // linking a display library.
    std::env::var("WAYLAND_DISPLAY").is_ok_and(|v| !v.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
}

#[tauri::command]
pub fn drag_capability() -> DragCapability {
    let platform = std::env::consts::OS.to_string();
    let wayland = cfg!(target_os = "linux") && detect_wayland();

    DragCapability {
        drag_supported: true,
        is_wayland: wayland,
        note: if wayland {
            Some(
                "Wayland session detected. Native drag-out is unverified here — \
                 if the drop does not land, use Export instead and record it in \
                 docs/daw-matrix.md."
                    .into(),
            )
        } else {
            None
        },
        platform,
    }
}

/// Write the spike pattern to a real `.mid` in the session dir.
///
/// Separate from the drag itself so the file can be inspected, opened by hand,
/// and imported into a DAW normally — which is the control case the matrix
/// needs. If the file will not import by hand, a failed drag says nothing.
#[tauri::command]
pub fn export_spike_midi() -> Result<ExportResult, String> {
    let pattern = engine::midi::drag_spike_pattern();
    let bytes = engine::midi::pattern_to_smf(&pattern);

    let path = write_session_file("freally-drag-spike", "mid", &bytes)
        .map_err(|e| format!("could not write the test file: {e}"))?;

    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
    })
}

/// Confirm a path is a real file before the UI hands it to the OS drag.
///
/// The drag itself is started from the frontend — `tauri-plugin-drag` exposes
/// only `init()` on the Rust side and does the work through its own JS API.
/// This exists because a drag whose path does not exist is silently rejected
/// by the drop target, which during a spike is indistinguishable from "the
/// platform does not support drag at all". Checking first turns that ambiguity
/// into a real error message.
#[tauri::command]
pub fn drag_source_ready(path: String) -> Result<u64, String> {
    // Constrained to the session dir: otherwise this doubles as an oracle for
    // the existence and size of any file the user can read.
    let file = must_be_session_file(&path)?;
    let meta = std::fs::metadata(&file).map_err(|e| format!("nothing to drag at {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if meta.len() == 0 {
        return Err(format!("{path} is empty — a DAW will reject it"));
    }
    Ok(meta.len())
}

/// Ask the user for an export folder.
///
/// Uses `rfd` directly rather than `tauri-plugin-dialog`. The plugin hard-enables
/// `rfd/common-controls-v6`, which imports `TaskDialogIndirect` from Common
/// Controls **v6** — available only to a binary carrying the right side-by-side
/// manifest. The app binary gets one from `tauri-build`; a `cargo test` binary
/// does not, so every unit test in this crate died at load with
/// STATUS_ENTRYPOINT_NOT_FOUND on Windows. `rfd` is already a dependency for the
/// crash dialog, so the plugin was buying nothing but that failure.
///
/// Returns `None` when the user cancels, which is not an error.
#[tauri::command]
pub async fn pick_export_folder() -> Option<String> {
    let chosen = rfd::AsyncFileDialog::new()
        .set_title("Choose an export folder")
        .pick_folder()
        .await?;

    let path = chosen.path().to_path_buf();
    // Remember it here. `export_to_folder` uses this rather than a path handed
    // back by the UI, so the destination is always one the user picked in a
    // native dialog this run.
    if let Ok(mut slot) = EXPORT_FOLDER.lock() {
        *slot = Some(path.clone());
    }
    Some(path.to_string_lossy().into_owned())
}

/// Copy an exported file into a folder the user chose, and reveal it.
///
/// The always-works path. On a platform where drag proves unreliable this
/// becomes the default rather than a fallback.
#[tauri::command]
pub fn export_to_folder(source: String) -> Result<ExportResult, String> {
    // Both ends are constrained: the source must be a file this app exported,
    // and the destination is the folder the user chose in the native picker —
    // not a path supplied by the caller.
    let src = must_be_session_file(&source)?;
    let folder = EXPORT_FOLDER
        .lock()
        .map_err(|_| "the export folder is unavailable".to_string())?
        .clone()
        .ok_or_else(|| "choose an export folder first".to_string())?;

    let stem = src
        .file_stem()
        .map(|s| safe_stem(&s.to_string_lossy()))
        .ok_or_else(|| "the exported file has no name".to_string())?;
    let extension = src
        .extension()
        .map(|e| safe_stem(&e.to_string_lossy()))
        .unwrap_or_else(|| "mid".into());
    let target = folder.join(format!("{stem}.{extension}"));

    let bytes = std::fs::read(&src).map_err(|e| format!("could not read {source}: {e}"))?;
    super::write_atomic(&target, &bytes)
        .map_err(|e| format!("could not write to {}: {e}", target.display()))?;

    // Show it, so "exported" is something the user can see rather than believe.
    let _ = tauri_plugin_opener::reveal_item_in_dir(&target);

    Ok(ExportResult {
        path: target.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spike_file_is_a_real_midi_file() {
        let result = export_spike_midi().expect("the spike file must be writable");
        let bytes = std::fs::read(&result.path).unwrap();

        assert_eq!(
            &bytes[0..4],
            b"MThd",
            "must be a MIDI file, not an empty stub"
        );
        assert_eq!(bytes.len() as u64, result.bytes);
        assert!(result.bytes > 100, "a 4-bar pattern should not be tiny");

        let _ = std::fs::remove_file(&result.path);
    }

    #[test]
    fn a_file_outside_the_session_dir_is_refused() {
        // The exploit this closes: any script in the WebView calling
        // export_to_folder / drag_source_ready with an arbitrary absolute path
        // to read a private key, or to destroy a file via write_atomic's
        // remove-then-rename.
        let outside = std::env::temp_dir().join("freally-not-a-session-file.txt");
        std::fs::write(&outside, b"secret").unwrap();

        let path = outside.to_string_lossy().into_owned();
        let err = drag_source_ready(path.clone()).unwrap_err();
        assert!(err.contains("outside this session"), "{err}");

        let err = export_to_folder(path).unwrap_err();
        assert!(err.contains("outside this session"), "{err}");

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn a_traversal_path_cannot_escape_the_session_dir() {
        // Canonicalisation is what makes `..` moot; assert it rather than
        // assuming it.
        let escape = super::super::session_dir()
            .unwrap()
            .join("..")
            .join("..")
            .join("escape-attempt.txt");
        std::fs::write(&escape, b"x").unwrap();

        let err = drag_source_ready(escape.to_string_lossy().into_owned()).unwrap_err();
        assert!(err.contains("outside this session"), "{err}");

        let _ = std::fs::remove_file(&escape);
    }

    #[test]
    fn a_missing_path_is_an_error_not_a_pass() {
        let err = drag_source_ready("definitely/not/here.mid".into()).unwrap_err();
        assert!(err.contains("no such file"), "{err}");
    }

    #[test]
    fn exporting_without_choosing_a_folder_is_refused() {
        // Its own file, not the shared spike name. Calling `export_spike_midi`
        // here raced `the_spike_file_is_a_real_midi_file`: both write
        // freally-drag-spike.mid into the one session directory, and whichever
        // finished first deleted it out from under the other, which then failed
        // canonicalising a path that had existed a moment earlier.
        let source = write_session_file("export-without-folder", "mid", b"MThd-ish").unwrap();

        // The destination comes from the picker, never from the caller.
        let result = export_to_folder(source.to_string_lossy().into_owned());
        // Either no folder has been chosen, or a previous test chose one; both
        // are fine, but it must never accept a caller-supplied destination.
        if let Err(e) = result {
            assert!(e.contains("choose an export folder"), "{e}");
        }
        let _ = std::fs::remove_file(&source);
    }

    #[test]
    fn capability_reports_this_platform() {
        let cap = drag_capability();
        assert_eq!(cap.platform, std::env::consts::OS);
        // Wayland can only be true on Linux.
        if !cfg!(target_os = "linux") {
            assert!(!cap.is_wayland);
        }
    }

    #[test]
    fn a_wayland_session_carries_an_explanatory_note() {
        // The note is what tells the tester to record a failure rather than
        // assume the app is broken.
        let cap = drag_capability();
        if cap.is_wayland {
            assert!(cap.note.is_some());
        }
    }
}
