//! The first-run licence gate.
//!
//! The shared bug-reporter/updater standard requires every app to show its
//! agreement before it can be used ("any consent gate (EULA/first-run)"). This
//! is that gate, for the plugin.
//!
//! **The text is compiled in from the repo-root `EULA.md`**, for the same reason
//! the dataset and the UI are: a plugin is a shared library in someone else's
//! process with no resource directory it can trust — and, more importantly here,
//! because the agreement a user accepts must be *exactly* the one this build
//! ships. A file read at runtime could have been edited underneath it.
//!
//! ## ⛔ Decline disables the plugin; it does not close the DAW
//!
//! This is the one place the plugin cannot copy the desktop apps. A standalone
//! app answers Decline by quitting. **A plugin has no such option** — it is a
//! guest inside Ableton or FL Studio, and a plugin that terminated its host over
//! a licence prompt would be the single most destructive thing in this codebase.
//!
//! So Decline leaves the plugin **inert**: [`accepted`] stays false, and the
//! RPC boundary in [`crate::editor`] refuses every command that would generate,
//! play, export or save. The editor still opens, and the agreement can be reopened and
//! accepted at any time — at which point the plugin works immediately, with
//! nothing lost. `EULA.md` states this in its own opening, because a gate whose
//! terms are not on the page is not consent.
//!
//! ## Where acceptance is stored, and where it deliberately is not
//!
//! In the per-user data directory beside the presets. ⛔ **Not in
//! [`crate::state::PluginSession`]**, which the host writes into the *project
//! file* — acceptance is a property of this person on this machine, not of a
//! song. Storing it there would ask a collaborator to re-accept because they
//! opened your `.als`, and would ship your acceptance inside a project you sent
//! to a label.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::presets::data_dir;

/// The agreement, exactly as this build ships it.
pub const TEXT: &str = include_str!("../../EULA.md");

/// The version a user's acceptance is recorded against.
///
/// **Bump this whenever `EULA.md` changes in a way that affects the user's
/// rights.** A stored version that no longer matches re-shows the gate, which is
/// the whole mechanism for re-consent — so bumping it for a typo fix is a way to
/// interrupt every user for nothing, and *not* bumping it for a real change is a
/// way to claim consent nobody gave.
pub const VERSION: &str = "2026-07-29";

/// What the UI needs to render the gate and decide whether to show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub version: String,
    pub text: String,
    pub accepted: bool,
}

/// What is written to disk. A struct rather than a bare string so a later field
/// (the accepting build, say) does not need a new file and a migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Record {
    accepted_version: Option<String>,
}

fn path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("eula.json"))
}

fn stored() -> Record {
    path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Set once this process has seen acceptance on disk. See [`accepted`].
static SEEN_ACCEPTED: AtomicBool = AtomicBool::new(false);

/// Whether this machine has accepted the version this build ships.
///
/// ⛔ Everything gates on this, and it runs on the host's UI thread once per
/// command — so the read it used to do every time is a file system call between
/// a producer and their own plugin. On a local disk that is microseconds; on a
/// roaming or OneDrive-backed profile it is however long that redirect takes.
///
/// ⛔ **Only `true` is remembered, and that asymmetry is the whole design.**
/// Caching both answers would need invalidating from wherever the file is
/// written, which is the staleness that turns "I accepted it" into "it still
/// will not generate". Caching only the positive cannot do that:
///
/// - **Not yet accepted** still reads the disk every time, so an acceptance
///   written by another instance — a second DAW, another track — is picked up
///   the moment it happens. That path is also cold: while the gate is up the
///   only commands running are the handful it allows.
/// - **Accepted** is remembered, which is the hot path and the only one worth
///   removing. Within this process the single thing that can revoke it is
///   [`decline`], which clears the flag itself.
///
/// The one behaviour this gives up: declining in *another* running host does not
/// reach back into this one. That is the right outcome anyway — a decision made
/// in Ableton should not silently disable a plugin mid-session in FL.
pub fn accepted() -> bool {
    if SEEN_ACCEPTED.load(Ordering::Relaxed) {
        return true;
    }

    let accepted = stored().accepted_version.as_deref() == Some(VERSION);
    if accepted {
        SEEN_ACCEPTED.store(true, Ordering::Relaxed);
    }
    accepted
}

pub fn status() -> Status {
    Status {
        version: VERSION.to_owned(),
        text: TEXT.to_owned(),
        accepted: accepted(),
    }
}

/// Record acceptance of the shipped version. Idempotent.
pub fn accept() -> Result<(), String> {
    let path =
        path().ok_or("this platform has no per-user data directory to record acceptance in")?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {parent:?}: {error}"))?;
    }

    let record = Record {
        accepted_version: Some(VERSION.to_owned()),
    };
    let text = serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?;
    fs::write(&path, text).map_err(|error| format!("could not write {path:?}: {error}"))?;

    // After the write, never before: a failed write must not leave this process
    // believing an acceptance that is not on disk and will not survive a reload.
    SEEN_ACCEPTED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Withdraw acceptance, leaving the plugin inert until it is accepted again.
///
/// The record is *cleared* rather than the file deleted, so the difference
/// between "declined" and "never asked" stays visible on disk to anyone
/// diagnosing a support report.
pub fn decline() -> Result<(), String> {
    let path =
        path().ok_or("this platform has no per-user data directory to record the decision in")?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {parent:?}: {error}"))?;
    }

    let text =
        serde_json::to_string_pretty(&Record::default()).map_err(|error| error.to_string())?;
    fs::write(&path, text).map_err(|error| format!("could not write {path:?}: {error}"))?;

    // ⛔ Cleared *before* returning, so the very next command through the RPC
    // boundary is refused. This is the only thing that revokes the memo in
    // [`accepted`], and a decline that left it standing would leave the plugin
    // fully working until the host was restarted.
    SEEN_ACCEPTED.store(false, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agreement_is_compiled_in_and_is_the_real_one() {
        // A gate showing placeholder text is a gate that has not been wired to
        // the document it claims to present.
        assert!(TEXT.contains("End User License Agreement"), "not the EULA");
        assert!(
            TEXT.contains("All Rights Reserved"),
            "the licence terms are missing from the text users accept"
        );
    }

    #[test]
    fn the_agreement_says_the_app_cannot_be_used_until_it_is_accepted() {
        // ⛔ Mike's requirement, and it is a consent property rather than copy:
        // the gate's own terms have to be *in* the text being agreed to.
        assert!(
            TEXT.contains("cannot use Freally MIDI Master until you have read"),
            "EULA.md must state that the app is unusable until accepted"
        );
        assert!(
            TEXT.contains("Decline"),
            "EULA.md must explain what declining does"
        );
    }

    #[test]
    fn the_status_carries_the_version_the_ui_gates_on() {
        let status = status();
        assert_eq!(status.version, VERSION);
        assert_eq!(status.text, TEXT);
    }
}
