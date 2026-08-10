//! What the File Explorer shows, and what the loader accepts — **one list**.
//!
//! ⛔⛔ **Three copies of this is a defect class, not a tidiness question**, and
//! TASK-058 names it: *"the tree's filter, the importer's accepted extensions and
//! the decoder's enabled `symphonia` features must come from a single place in
//! the engine. Three copies is how the explorer ends up showing a file the
//! importer then refuses."* That is the readout-that-lies failure wearing a
//! different hat — a row that does nothing when you drop it, with nothing on
//! screen able to say why.
//!
//! It had already drifted into two: `explorer::AUDIO` and the Open dialog's
//! filter in `oneshot.rs` were separate literals that happened to agree. Nothing
//! made them agree, and nothing would have noticed when they stopped.
//!
//! ⛔ **A format is listed BECAUSE it decodes.** [`AUDIO`] is not a wish list —
//! every entry has a decoder compiled in (see the `symphonia` features in
//! `plugin/Cargo.toml`) and `engine`'s own test walks the list. Adding an
//! extension here without the feature behind it is how the explorer starts
//! offering files the loader refuses.
//!
//! ⚠ **In `engine` rather than in `plugin`** because both the plugin and the
//! offline tools have to agree about it, and because it is a fact about the
//! product rather than about the editor.

/// Audio a producer's one-shots may be in.
///
/// ⛔ **`.m4a`/`.aac` is a DECISION, not an omission, and it is currently made
/// the other way from what TASK-058 records.** The task says AAC's
/// patent-licensing position is less clear-cut than the rest and it should be
/// *"flagged for Mike rather than quietly included"* — and `m4a` is in this list
/// with `aac` and `alac` enabled in symphonia. It is flagged in
/// `docs/product-roadmap.md`; removing it is one line here and it takes the
/// dialog filter and the tree with it, which is the whole point of this module.
///
/// ⚠ Lower-case, and compared case-insensitively — a sample library is full of
/// `.WAV`.
pub const AUDIO: &[&str] = &["wav", "aif", "aiff", "flac", "mp3", "m4a", "ogg", "oga"];

/// MIDI a producer may train from, drag onto a generator, or split into parts.
///
/// ⛔⛔ **These are NOT sound files and nothing may treat them as any.** A `.mid`
/// has no waveform to draw, no PCM to cache and nothing to assign to a drum pad.
/// Offering a waveform for one, or a pad assignment, is a control that can only
/// fail — TASK-058 states this as a rule and [`Kind`] is what keeps the two
/// apart everywhere rather than at each call site.
pub const MIDI: &[&str] = &["mid", "midi"];

/// Which of the two a file is, decided once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Decodes to PCM: waveform, audition, one-shot assignment, audio→MIDI.
    Audio,
    /// Reads as notes: piano-roll thumbnail, playback, drag-to-generator.
    Midi,
}

/// What `path`'s extension says it is, or `None` if the explorer should not
/// show it at all.
pub fn kind_of(path: &std::path::Path) -> Option<Kind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if AUDIO.contains(&extension.as_str()) {
        return Some(Kind::Audio);
    }
    if MIDI.contains(&extension.as_str()) {
        return Some(Kind::Midi);
    }
    None
}

/// Every extension the explorer lists, for a file dialog's filter.
pub fn all() -> Vec<&'static str> {
    AUDIO.iter().chain(MIDI.iter()).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_two_kinds_never_overlap() {
        // ⛔ An extension in both lists would make `kind_of` decide by the order
        // the two are checked in, which is an accident rather than a rule.
        for extension in AUDIO {
            assert!(
                !MIDI.contains(extension),
                "{extension} cannot be both audio and MIDI"
            );
        }
    }

    #[test]
    fn an_extension_is_classified_whatever_its_case() {
        // A sample library is full of `.WAV` and the occasional `.MID`.
        assert_eq!(kind_of(Path::new("kick.wav")), Some(Kind::Audio));
        assert_eq!(kind_of(Path::new("kick.WAV")), Some(Kind::Audio));
        assert_eq!(kind_of(Path::new("loop.MIDI")), Some(Kind::Midi));
        assert_eq!(kind_of(Path::new("loop.mid")), Some(Kind::Midi));
    }

    #[test]
    fn everything_else_is_not_shown_at_all() {
        // ⚠ Including a file with no extension: a browser that offered it would
        // be offering the loader something it has no way to classify.
        assert_eq!(kind_of(Path::new("notes.txt")), None);
        assert_eq!(kind_of(Path::new("cover.png")), None);
        assert_eq!(kind_of(Path::new("README")), None);
        assert_eq!(kind_of(Path::new("")), None);
    }

    #[test]
    fn the_dialog_filter_offers_both_kinds() {
        let all = all();
        assert!(all.contains(&"wav"));
        assert!(all.contains(&"mid"));
        assert_eq!(all.len(), AUDIO.len() + MIDI.len());
    }
}
