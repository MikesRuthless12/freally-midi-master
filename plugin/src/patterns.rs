//! The pattern library (TASK-045A).
//!
//! The problem it solves, in the owner's words: *"you generate something you
//! like, you have to leave, and when you come back it should still be there —
//! usable with whatever kit you feel like putting under it."*
//!
//! ## ⛔ What is stored, and the one thing that deliberately is not
//!
//! A saved pattern is the **notes**, plus everything needed to make sense of
//! them: seed, artist id, bars, key, scale, tempo, time signature and the
//! per-lane assignment. [`engine::pattern::Pattern`] already serialises to
//! exactly that, so the format is the one the engine already speaks and there is
//! no second shape to keep in step.
//!
//! **The kit is not stored.** That is what makes *"use it with any sounds you
//! want"* true rather than a claim: load the pattern, swap the kit, and the same
//! performance plays through different samples. Kits and one-shot configs have
//! their own preset system (TASK-060/061) and the two are joined only at
//! playback.
//!
//! ## ⛔ Why this is not `state.rs`, and not `presets.rs`
//!
//! - [`crate::state`] persists the *current* session **with the host's project**,
//!   so reopening a song restores what was on the track. A library that lived
//!   there would be per-project, which is the opposite of the point.
//! - [`crate::presets`] saves a `PluginSession` — an artist, a seed and pins — as
//!   a *starting point*. It stores no notes at all, because the engine
//!   regenerates them. This stores notes precisely because an edited pattern is
//!   **not** reproducible from its seed, and because a producer wants the take
//!   they kept rather than the one the seed would rebuild after an engine change.
//!
//! A pattern saved in one session loads in any other, in any host. It is also
//! how a pattern survives a plugin crash, which is worth saying out loud: a
//! plugin lives in someone else's process and dies with it.

use std::fs;
use std::path::{Path, PathBuf};

use engine::pattern::Pattern;
use serde::{Deserialize, Serialize};

/// One saved pattern, as it sits on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPattern {
    /// What the producer typed, unslugged, so the browser shows their name.
    pub name: String,
    /// When it was saved, epoch milliseconds.
    ///
    /// ⚠ **Written by the *page*, not by this module.** The engine and this
    /// store are clock-free on purpose — nothing about generation may depend on
    /// the time — and the frontend is already where an entry is created
    /// (TASK-045's history makes the same argument). `0` is what a file written
    /// before this field means, and the browser shows nothing rather than 1970.
    #[serde(default)]
    pub saved_at: i64,
    pub pattern: Pattern,
}

/// A row in the browser: enough to choose by, without loading the notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PatternSummary {
    /// The file stem, which is also what [`load`] and [`delete`] take.
    pub id: String,
    pub name: String,
    pub artist_id: String,
    pub part: String,
    pub bars: u16,
    pub bpm: f32,
    pub saved_at: i64,
    /// How busy each sixteenth of the clip is, 0–1 — the mini grid preview.
    ///
    /// ⛔ **Computed here rather than in the browser**, because the browser
    /// would have to load every pattern's notes to draw one row each. A
    /// thirty-two-number histogram is a few hundred bytes against a clip's few
    /// hundred *kilo*bytes, and the list is the thing that has to stay quick.
    pub density: Vec<f32>,
}

/// Where saved patterns live.
///
/// ⛔ **Individual files rather than one library blob**, so a corrupt save costs
/// one pattern instead of all of them — and so a producer can back them up, sync
/// them, or hand one to someone else by copying a file.
fn user_dir() -> Option<PathBuf> {
    crate::presets::data_dir().map(|base| base.join("patterns"))
}

/// A filename from a display name — [`crate::presets::slug`], with this
/// module's fallback prefix.
///
/// ⛔⛔ **This was a copy, and the copy had already drifted.** Both functions
/// carried the same doc comment claiming the same algorithm, and this one's
/// hash multiplied by `0x1000_0000_01b3` where `presets` uses the real FNV-1a
/// prime `0x100_0000_01b3` — so two "identical" functions produced *different*
/// filenames for the same non-ASCII name. It is a security boundary, which is
/// the worst possible thing to keep two versions of: hardening one would have
/// silently left the other on the old rule.
fn slug(name: &str) -> String {
    crate::presets::slug_with(name, "pattern")
}

/// Is this a stem this module wrote? [`crate::presets::is_safe_stem`].
///
/// The gate on every path that comes back *in* from the page — `load` and
/// `delete` both take an id, and an id is a filename.
fn is_safe_stem(stem: &str) -> bool {
    crate::presets::is_safe_stem(stem)
}

/// How busy each of `columns` slices of the clip is, 0–1.
///
/// The same shape `cells.ts::columnDensity` draws the generation ripple from,
/// normalised against the busiest column so a sparse boom-bap pattern reads as
/// clearly as a dense drill one.
fn density(pattern: &Pattern, columns: usize) -> Vec<f32> {
    // The clip's own length: bars × beats × ticks-per-beat, from the pattern's
    // own meter rather than an assumed 4/4 — a 6/8 clip is three quarters as
    // long and a preview that assumed common time would draw its notes off the
    // end.
    // ⚠ **`den == 0` falls back to 4, not to 1**, which is the rule
    // `engine::context::normalise_meter` states: 1 is a *legal* denominator, so
    // `.max(1)` would silently accept a malformed project and make the clip
    // four times too long — collapsing every preview into column 0.
    let den = match pattern.time_sig_den {
        0 => 4,
        den => u32::from(den),
    };
    // ⛔⛔ **Saturating, in `u64`, and floored at 1 — because this arithmetic
    // runs on numbers read straight off a producer's disk.** The module's own
    // doc invites people to back these files up, sync them and hand one to
    // someone else, so `bars`, `ppq` and the meter arrive as whatever survived
    // that trip. Two ways this used to end the process, both with
    // `panic = "abort"` under a host: `ppq * 4` overflowed a `u32` for a large
    // `ppq`, and `ppq * 4 / den` floored to **0** for anything with `ppq` under
    // `den / 4` — after which `/ total` is a divide by zero. One corrupt file
    // must cost the producer that file, not the DAW; the `den == 0` guard just
    // above shows one of these was already considered, and the rest were not.
    let total = u64::from(pattern.bars)
        .max(1)
        .saturating_mul(u64::from(pattern.time_sig_num).max(1))
        .saturating_mul((u64::from(pattern.ppq).saturating_mul(4) / u64::from(den)).max(1))
        .max(1);
    let mut counts = vec![0.0f32; columns.max(1)];
    for track in &pattern.lanes {
        for note in &track.notes {
            let column = (u64::from(note.start_tick) * counts.len() as u64 / total)
                .min(counts.len() as u64 - 1) as usize;
            counts[column] += 1.0;
        }
    }
    let busiest = counts.iter().copied().fold(0.0f32, f32::max);
    if busiest <= 0.0 {
        return counts;
    }
    counts.iter().map(|count| count / busiest).collect()
}

/// How many columns a preview is drawn in.
const PREVIEW_COLUMNS: usize = 32;

fn summarise(id: &str, saved: &SavedPattern) -> PatternSummary {
    PatternSummary {
        id: id.to_owned(),
        name: saved.name.clone(),
        artist_id: saved.pattern.artist_id.clone(),
        // `Part` serialises camelCase; the page filters on the same strings it
        // uses for its tabs.
        part: serde_json::to_value(saved.pattern.part)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
        bars: saved.pattern.bars,
        bpm: saved.pattern.bpm,
        saved_at: saved.saved_at,
        density: density(&saved.pattern, PREVIEW_COLUMNS),
    }
}

/// Every saved pattern, newest first.
pub fn list() -> Vec<PatternSummary> {
    user_dir().map(|dir| list_in(&dir)).unwrap_or_default()
}

fn list_in(dir: &Path) -> Vec<PatternSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        // No directory yet is an empty library, not an error: it is what every
        // producer has before they save their first pattern.
        return Vec::new();
    };

    let mut found: Vec<PatternSummary> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_owned();
            // ⚠ Parity with `presets::list_in`. Not a traversal risk — a
            // `DirEntry` stem can never hold a separator, and `load`/`delete`
            // re-validate — but a hand-placed `My Pattern.json` would otherwise
            // show up as a library row that errors the moment it is clicked.
            if !is_safe_stem(&stem) {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;
            // ⚠ **A file that will not parse is skipped, not fatal.** One
            // corrupt save must not cost the producer the rest of the library —
            // which is the whole reason these are individual files.
            let saved: SavedPattern = serde_json::from_str(&text).ok()?;
            Some(summarise(&stem, &saved))
        })
        .collect();

    // Newest first, then by name so the order is total — two patterns saved in
    // the same millisecond must not swap places between listings.
    found.sort_by(|a, b| {
        b.saved_at
            .cmp(&a.saved_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    found
}

/// The name a saved file holds, if it is one we can read.
fn name_in(dir: &Path, stem: &str) -> Option<String> {
    let text = fs::read_to_string(dir.join(format!("{stem}.json"))).ok()?;
    let saved: SavedPattern = serde_json::from_str(&text).ok()?;
    Some(saved.name)
}

/// How many `-2`, `-3`… stems to try before giving the file a hashed name.
const MAX_COLLISIONS: u32 = 64;

/// The filename to save `name` under, given what `dir` already holds.
///
/// ⛔⛔ **Saving over *your own* pattern is the promise; saving over somebody
/// else's is the bug.** The stem came straight from `slug`, which strips
/// punctuation and case — so "Take 1", "Take-1", "take 1" and "Take  1" are one
/// filename, and the second save silently deleted the first take. In a feature
/// whose entire purpose is that what you liked is still there when you come
/// back, that is the worst thing it could do, and it did it with no warning and
/// no way back. A stem is reused only when the file under it carries the *same
/// display name*; otherwise the next free `-2`, `-3`… is taken.
///
/// ⚠ **A file we cannot read counts as taken, not as free.** It might be a
/// corrupt save the producer still wants to recover by hand, and `list_in`
/// already skips it rather than deleting it.
fn free_stem(dir: &Path, name: &str) -> String {
    let base = slug(name);
    for suffix in 1..=MAX_COLLISIONS {
        let stem = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        if !dir.join(format!("{stem}.json")).exists() {
            return stem;
        }
        if name_in(dir, &stem).is_some_and(|existing| existing == name) {
            return stem;
        }
    }
    // Sixty-four patterns whose names all slug alike is not a producer's
    // library, but it still has to land somewhere that is not on top of one of
    // them. `slug_with`'s own fallback shape, so the name stays stable.
    format!("{base}-{:016x}", crate::presets::hash(name))
}

/// Save a pattern under a name, replacing only a pattern saved under that same
/// name — never one that merely slugs to the same filename.
pub fn save(name: &str, saved_at: i64, pattern: Pattern) -> Result<PatternSummary, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to save patterns to".to_owned())?;
    save_in(&dir, name, saved_at, pattern)
}

fn save_in(
    dir: &Path,
    name: &str,
    saved_at: i64,
    pattern: Pattern,
) -> Result<PatternSummary, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a pattern needs a name".to_owned());
    }
    if pattern.lanes.iter().all(|track| track.notes.is_empty()) {
        // ⛔ Refused rather than written. A library row that loads to silence is
        // a saved pattern the producer will click, hear nothing from, and
        // reasonably conclude the feature is broken.
        return Err("this pattern has no notes in it".to_owned());
    }

    fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    // ⚠ After `create_dir_all`, because `free_stem` asks the directory what is
    // already in it and a directory that does not exist answers "nothing".
    let stem = free_stem(dir, name);

    let saved = SavedPattern {
        name: name.to_owned(),
        saved_at,
        pattern,
    };
    let text = serde_json::to_string_pretty(&saved)
        .map_err(|error| format!("could not serialise the pattern: {error}"))?;

    write_atomic(&dir.join(format!("{stem}.json")), &text)?;
    Ok(summarise(&stem, &saved))
}

/// Write via a temporary file and a rename.
///
/// ⛔ **A clip is kilobytes and a plain `fs::write` truncates before it writes.**
/// A crash — or a DAW being force-quit — between those two leaves a zero-length
/// file where a pattern was, and the producer loses the take rather than the
/// save. Rename is atomic on every platform this ships to.
///
/// ⚠ **Windows will not rename onto an existing file**, so the destination is
/// removed first. That is a real window, and it is the reason the temporary
/// carries the same stem: if the rename fails, the `.tmp` beside it is the
/// pattern, recoverable by hand.
/// `pub(crate)` for [`crate::models`], which writes a producer's own style model
/// and needs exactly this dance rather than a second copy of it.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, text)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not save {}: {error}", path.display()))
}

/// Load one saved pattern's notes.
pub fn load(id: &str) -> Result<Pattern, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to load patterns from".to_owned())?;
    load_in(&dir, id)
}

fn load_in(dir: &Path, id: &str) -> Result<Pattern, String> {
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a pattern id"));
    }
    let path = dir.join(format!("{id}.json"));
    let text =
        fs::read_to_string(&path).map_err(|error| format!("could not read `{id}`: {error}"))?;
    serde_json::from_str::<SavedPattern>(&text)
        .map(|saved| saved.pattern)
        .map_err(|error| format!("pattern `{id}` is malformed: {error}"))
}

/// Delete a saved pattern.
pub fn delete(id: &str) -> Result<(), String> {
    let dir = user_dir()
        .ok_or_else(|| "there is no user directory to delete patterns from".to_owned())?;
    delete_in(&dir, id)
}

fn delete_in(dir: &Path, id: &str) -> Result<(), String> {
    if !is_safe_stem(id) {
        return Err(format!("`{id}` is not a pattern id"));
    }
    let path = dir.join(format!("{id}.json"));
    // A pattern that is already gone is not an error: the producer wanted it
    // gone and it is.
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|error| format!("could not delete `{id}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale, PPQ};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-patterns-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn pattern(start: u32) -> Pattern {
        Pattern {
            id: "p".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 7,
            song_seed: 7,
            bars: 4,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 6,
            scale: Scale::NaturalMinor,
            ppq: PPQ,
            lanes: vec![LaneTrack {
                lane: Lane::Kick,
                notes: vec![Note {
                    start_tick: start,
                    len_ticks: 120,
                    pitch: 36,
                    vel: 100,
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                    reversed: false,
                }],
            }],
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    #[test]
    fn two_names_that_slug_alike_do_not_destroy_each_other() {
        // ⛔⛔ **The library's one promise, and it was breaking it silently.**
        // `slug` strips punctuation and case, so "Take 1" and "Take-1" were one
        // filename and the second save deleted the first take — no warning, no
        // prompt, no way back. Both must survive, and both must load as what
        // they were saved as.
        let dir = temp_dir("collide");
        let first = save_in(&dir, "Take 1", 1, pattern(0)).unwrap();
        let second = save_in(&dir, "Take-1", 2, pattern(960)).unwrap();

        assert_ne!(first.id, second.id, "two patterns must not share a file");
        assert_eq!(load_in(&dir, &first.id).unwrap(), pattern(0));
        assert_eq!(load_in(&dir, &second.id).unwrap(), pattern(960));
        assert_eq!(list_in(&dir).len(), 2);

        // ⚠ And saving over *your own* name still replaces it, which is what
        // the feature promises — otherwise re-saving a take you kept editing
        // would litter the library with `-2`, `-3`, `-4`.
        let again = save_in(&dir, "Take 1", 3, pattern(1920)).unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(load_in(&dir, &first.id).unwrap(), pattern(1920));
        assert_eq!(list_in(&dir).len(), 2);
    }

    #[test]
    fn a_clip_with_an_impossible_meter_is_summarised_rather_than_aborting() {
        // ⛔⛔ **This crashed the host, not the panel.** `density` divided by
        // `bars × num × (ppq * 4 / den)`, and that inner term floors to **0**
        // whenever `ppq * 4 < den` — so a file holding `ppq: 1, den: 8` was a
        // divide by zero, and the crate builds with `panic = "abort"`. The
        // module invites producers to sync these files and hand them to each
        // other, so the numbers arrive as whatever survived the trip. A file we
        // cannot make sense of costs the producer *that file*.
        let mut broken = pattern(0);
        broken.ppq = 1;
        broken.time_sig_den = 8;
        let counts = density(&broken, PREVIEW_COLUMNS);
        assert_eq!(counts.len(), PREVIEW_COLUMNS);

        // The other end of the same arithmetic: `ppq * 4` used to overflow a
        // `u32` before it was ever divided.
        let mut huge = pattern(0);
        huge.ppq = u32::MAX;
        huge.bars = u16::MAX;
        huge.time_sig_num = u8::MAX;
        assert_eq!(density(&huge, PREVIEW_COLUMNS).len(), PREVIEW_COLUMNS);

        // And it still reaches the panel rather than taking the DAW with it.
        let dir = temp_dir("meter");
        let saved = save_in(&dir, "Broken", 1, broken).unwrap();
        assert_eq!(list_in(&dir).len(), 1);
        assert_eq!(saved.density.len(), PREVIEW_COLUMNS);
    }

    #[test]
    fn a_saved_pattern_comes_back_note_for_note() {
        // ⛔ **The claim the whole task rests on**, and the roadmap's own verify
        // line: save, come back, and the notes are identical. The kit is not in
        // the file at all, which is what makes "with any sounds you want" true.
        let dir = temp_dir("roundtrip");
        let original = pattern(480);

        let summary = save_in(&dir, "My Beat", 1_700_000_000_000, original.clone()).unwrap();
        assert_eq!(summary.id, "my-beat");
        assert_eq!(summary.artist_id, "trap");
        assert_eq!(summary.bars, 4);

        assert_eq!(load_in(&dir, &summary.id).unwrap(), original);
        assert!(
            !fs::read_to_string(dir.join("my-beat.json"))
                .unwrap()
                .contains("kit"),
            "a saved pattern must carry no kit"
        );
    }

    #[test]
    fn saving_the_same_name_twice_replaces_rather_than_accumulates() {
        let dir = temp_dir("replace");
        save_in(&dir, "Take", 1, pattern(0)).unwrap();
        save_in(&dir, "Take", 2, pattern(960)).unwrap();

        assert_eq!(list_in(&dir).len(), 1, "one name, one file");
        assert_eq!(
            load_in(&dir, "take").unwrap().lanes[0].notes[0].start_tick,
            960
        );
        // ⚠ And the temporary is gone — a `.tmp` left beside it would show up
        // in a producer's folder and in any backup they take.
        assert!(!dir.join("take.json.tmp").exists());
    }

    #[test]
    fn a_name_that_could_be_a_path_cannot_write_outside_the_library() {
        // The same boundary `presets::slug` documents: the name comes from a
        // text box in a webview, in someone else's DAW.
        let dir = temp_dir("traversal");
        let summary = save_in(&dir, "../../../etc/passwd", 1, pattern(0)).unwrap();
        assert_eq!(summary.id, "etc-passwd");
        assert!(dir.join("etc-passwd.json").exists());

        // ...and the same rule on the way back in.
        assert!(load_in(&dir, "../../../etc/passwd").is_err());
        assert!(delete_in(&dir, "..").is_err());
    }

    #[test]
    fn a_name_with_no_ascii_in_it_still_gets_a_stable_file() {
        // This app ships 18 locales, and most of a Japanese name slugs to
        // nothing. Refusing it would make the feature unusable in half of them.
        let dir = temp_dir("unicode");
        let first = save_in(&dir, "ビート", 1, pattern(0)).unwrap();
        let second = save_in(&dir, "ビート", 2, pattern(0)).unwrap();
        assert_eq!(
            first.id, second.id,
            "the same name must reach the same file"
        );
        assert_eq!(first.name, "ビート", "the display name is untouched");
    }

    #[test]
    fn a_pattern_with_no_notes_is_refused_rather_than_saved() {
        let dir = temp_dir("empty");
        let mut silent = pattern(0);
        silent.lanes[0].notes.clear();
        assert!(save_in(&dir, "Nothing", 1, silent).is_err());
        assert!(list_in(&dir).is_empty());
    }

    #[test]
    fn the_listing_survives_a_corrupt_file_and_is_newest_first() {
        // ⛔ **The reason these are individual files.** One unreadable save must
        // cost one pattern, not the library.
        let dir = temp_dir("corrupt");
        save_in(&dir, "Older", 100, pattern(0)).unwrap();
        save_in(&dir, "Newer", 200, pattern(240)).unwrap();
        fs::write(dir.join("broken.json"), "{ not json").unwrap();

        let listed = list_in(&dir);
        assert_eq!(
            listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Newer", "Older"]
        );
    }

    #[test]
    fn the_preview_says_where_the_notes_are() {
        // The browser draws this instead of loading every clip's notes to show
        // one row each.
        let dir = temp_dir("preview");
        let summary = save_in(&dir, "Downbeat", 1, pattern(0)).unwrap();
        assert_eq!(summary.density.len(), PREVIEW_COLUMNS);
        assert_eq!(summary.density[0], 1.0, "the only note is at the start");
        assert!(summary.density[1..].iter().all(|value| *value == 0.0));
    }

    #[test]
    fn deleting_something_that_is_already_gone_is_not_an_error() {
        let dir = temp_dir("delete");
        save_in(&dir, "Gone", 1, pattern(0)).unwrap();
        assert!(delete_in(&dir, "gone").is_ok());
        assert!(
            delete_in(&dir, "gone").is_ok(),
            "the producer wanted it gone"
        );
        assert!(list_in(&dir).is_empty());
    }
}
