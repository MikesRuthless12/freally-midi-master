//! The platforms that have no native drag source yet (TASK-063C / FMM-S03).
//!
//! ⛔ **This is a refusal, not a stub that pretends.** macOS needs
//! `NSFilePromiseProvider` on the `NSView` and Linux needs XDND against the X11
//! window `linux.rs` owns; neither is written, and neither can be tested from
//! the machine this shipped from. What must not happen in the meantime is a
//! drag handle that the producer picks up and that drops nothing into their
//! DAW — they blame the DAW, and the plugin has told them a comfortable lie.
//! [`crate::drag::supported`] is how the page knows to keep offering Export
//! instead, which is the same call TASK-131F made for the audio stems.

use std::path::PathBuf;

use super::{Dropped, Preview, NO_DRAG_SOURCE};

pub const SUPPORTED: bool = false;

/// Moot — there is no drag source here to allow anywhere. Present so the seam
/// is the same shape on every platform and `supported_in` needs no `cfg`.
pub const STANDALONE_SAFE: bool = false;

pub fn drag(
    _paths: &[PathBuf],
    _stacked: &[PathBuf],
    _preview: Option<&Preview>,
) -> Result<Dropped, String> {
    Err(NO_DRAG_SOURCE.to_owned())
}
