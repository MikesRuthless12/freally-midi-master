//! Presets: a session with a name, kept somewhere it outlives one project.
//!
//! [`crate::state`] persists the *current* session into the host's project
//! file, so reopening a song restores the beat it was saved with. A preset is
//! the same [`PluginSession`], named, stored outside any project — the
//! difference between "this song" and "any song".
//!
//! **The plugin owns its presets, with its own UI.** CLAP has a
//! preset-discovery factory that would surface them in the host's browser
//! instead; Mike ruled that out on 2026-07-29 and it is not built here.
//!
//! ## Where they live, which is the only part that cannot be embedded
//!
//! Factory presets are compiled in, exactly like the dataset and the UI — a
//! plugin is a shared library in someone else's process, with no resource
//! directory it chose and no working directory it can trust.
//!
//! User presets have to be written somewhere, and that somewhere is the
//! platform's per-user data directory. It is the one part of this plugin that
//! touches the filesystem at all, which is why the path is built here, once,
//! and why [`slug`] exists.
//!
//! ## What a factory preset deliberately does not do
//!
//! **It never pins a tempo.** Precedence is user pin > host > model, so a
//! factory preset carrying `bpm` would override the DAW on load — in a plugin
//! whose whole reason for existing is following the host's tempo. A pin is an
//! instruction from the user, and a shipped file is not the user.
//! `no_factory_preset_pins_a_tempo` fails if one ever does.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dataset;
use crate::state::PluginSession;

/// A preset as it is stored, and as a factory file is authored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preset {
    /// What the user sees. Not a slug: the id is derived from this, and the two
    /// are allowed to disagree so that renaming is a display change.
    pub name: String,
    pub session: PluginSession,
}

/// A preset as the UI lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSummary {
    pub id: String,
    pub name: String,
    /// Shipped rather than saved — the UI refuses to delete these.
    pub factory: bool,
}

const FACTORY: &str = "factory/";
const USER: &str = "user/";

/// The longest a slug may be, so a pasted paragraph cannot become a filename.
const MAX_SLUG: usize = 64;

/// A name turned into something safe to be a filename.
///
/// ⛔ **This is a security boundary rather than tidiness.** The name arrives
/// from a text box in a webview and ends up as a path. `../../.ssh/authorized_keys`,
/// a drive letter, a NUL — every one of them is a write outside the preset
/// directory, performed by a process that is someone else's DAW. Only ASCII
/// letters and digits survive, joined by `-`, so there is nothing left to
/// traverse with: the result cannot contain `/`, `\`, `:` or `.`.
///
/// A name with no ASCII alphanumerics at all — which is most of a Japanese or
/// Arabic name, and this app ships 18 locales — would slug to nothing, so it
/// falls back to a hash of the name instead of being refused. The display name
/// is unaffected either way.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }

    let capped: String = out.trim_matches('-').chars().take(MAX_SLUG).collect();
    let capped = capped.trim_matches('-').to_owned();

    if capped.is_empty() {
        format!("preset-{:016x}", hash(name))
    } else {
        capped
    }
}

/// FNV-1a, so a name that slugs to nothing still gets a **stable** filename.
///
/// Hand-rolled rather than `DefaultHasher`, whose output std explicitly does
/// not promise to keep across releases — a preset whose filename changed with
/// the compiler would go missing on upgrade.
fn hash(text: &str) -> u64 {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        acc ^= u64::from(*byte);
        acc = acc.wrapping_mul(0x100_0000_01b3);
    }
    acc
}

/// True when `stem` is something [`slug`] could have produced.
///
/// Every path is built from an id that came back over the bridge, and the
/// bridge is reachable from the page. This is the check that makes joining it
/// to a directory safe, and it is deliberately a whitelist.
fn is_safe_stem(stem: &str) -> bool {
    !stem.is_empty()
        && stem.len() <= MAX_SLUG + 24
        && stem
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// The per-user data directory, or `None` where there is not one to speak of.
///
/// Built from the environment rather than a crate: three `join`s against a
/// variable every one of these platforms defines is not worth a dependency,
/// a licence to clear in `deny.toml` and another thing to audit.
pub(crate) fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("Freally MIDI Master"))
    }

    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Freally MIDI Master")
        })
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|base| base.join("freally-midi-master"))
    }
}

/// Where user presets are written.
fn user_dir() -> Option<PathBuf> {
    data_dir().map(|base| base.join("presets"))
}

/// Every preset, factory first, each group by name.
pub fn list() -> Vec<PresetSummary> {
    let mut all = factory_summaries();
    all.extend(user_dir().map(|dir| list_user_in(&dir)).unwrap_or_default());
    all
}

fn factory_summaries() -> Vec<PresetSummary> {
    let mut found: Vec<PresetSummary> = dataset::factory_presets()
        .into_iter()
        .filter_map(|(stem, text)| {
            // A factory preset that will not parse is a build-time mistake, and
            // `every_factory_preset_parses` fails on it. Skipped rather than
            // fatal at runtime so one bad file cannot cost the user the rest.
            let preset: Preset = serde_json::from_str(text).ok()?;
            Some(PresetSummary {
                id: format!("{FACTORY}{stem}"),
                name: preset.name,
                factory: true,
            })
        })
        .collect();

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn list_user_in(dir: &Path) -> Vec<PresetSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        // No directory yet simply means nothing has been saved.
        return Vec::new();
    };

    let mut found: Vec<PresetSummary> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "json" {
                return None;
            }
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            if !is_safe_stem(&stem) {
                return None;
            }
            let preset: Preset = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            Some(PresetSummary {
                id: format!("{USER}{stem}"),
                name: preset.name,
                factory: false,
            })
        })
        .collect();

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The session a preset restores.
pub fn load(id: &str) -> Result<PluginSession, String> {
    if let Some(stem) = id.strip_prefix(FACTORY) {
        return dataset::factory_presets()
            .into_iter()
            .find(|(name, _)| name == stem)
            .ok_or_else(|| format!("no factory preset `{stem}`"))
            .and_then(|(_, text)| {
                serde_json::from_str::<Preset>(text)
                    .map(|preset| preset.session)
                    .map_err(|error| format!("factory preset `{stem}` is malformed: {error}"))
            });
    }

    let stem = id
        .strip_prefix(USER)
        .ok_or_else(|| format!("`{id}` is not a preset id"))?;
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to read presets from".to_owned())?;
    load_user_in(&dir, stem)
}

fn load_user_in(dir: &Path, stem: &str) -> Result<PluginSession, String> {
    if !is_safe_stem(stem) {
        // Refused before it is ever joined to a path.
        return Err(format!("`{stem}` is not a preset name"));
    }

    let path = dir.join(format!("{stem}.json"));
    let text = fs::read_to_string(&path).map_err(|error| format!("no preset `{stem}`: {error}"))?;
    serde_json::from_str::<Preset>(&text)
        .map(|preset| preset.session)
        .map_err(|error| format!("preset `{stem}` is malformed: {error}"))
}

/// Save the session under a name, replacing any preset with the same slug.
pub fn save(name: &str, session: PluginSession) -> Result<PresetSummary, String> {
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to save presets to".to_owned())?;
    save_in(&dir, name, session)
}

fn save_in(dir: &Path, name: &str, session: PluginSession) -> Result<PresetSummary, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a preset needs a name".to_owned());
    }

    let stem = slug(name);
    fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;

    let preset = Preset {
        name: name.to_owned(),
        session,
    };
    let text = serde_json::to_string_pretty(&preset)
        .map_err(|error| format!("could not serialise the preset: {error}"))?;

    fs::write(dir.join(format!("{stem}.json")), text)
        .map_err(|error| format!("could not write the preset: {error}"))?;

    Ok(PresetSummary {
        id: format!("{USER}{stem}"),
        name: name.to_owned(),
        factory: false,
    })
}

/// Delete a user preset. Factory presets are shipped and cannot be removed.
pub fn delete(id: &str) -> Result<(), String> {
    if id.starts_with(FACTORY) {
        return Err("a factory preset cannot be deleted".to_owned());
    }

    let stem = id
        .strip_prefix(USER)
        .ok_or_else(|| format!("`{id}` is not a preset id"))?;
    let dir =
        user_dir().ok_or_else(|| "there is no user directory to delete presets from".to_owned())?;
    delete_in(&dir, stem)
}

fn delete_in(dir: &Path, stem: &str) -> Result<(), String> {
    if !is_safe_stem(stem) {
        return Err(format!("`{stem}` is not a preset name"));
    }

    fs::remove_file(dir.join(format!("{stem}.json")))
        .map_err(|error| format!("could not delete `{stem}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A directory of this test's own.
    ///
    /// The real directory comes from the environment, and an env var is process
    /// global — two tests setting it would race. The core functions therefore
    /// take the directory, and only the thin public wrappers know where it is.
    fn scratch() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "freally-presets-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(unique);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_preset_comes_back_with_the_session_that_was_saved() {
        let dir = scratch();
        let session = PluginSession {
            selected_id: Some("travis-scott".into()),
            seed: "4242".into(),
            bars: Some(8),
            ..PluginSession::default()
        };

        let saved = save_in(&dir, "My Beat", session.clone()).unwrap();
        assert_eq!(saved.id, "user/my-beat");
        assert!(!saved.factory);

        assert_eq!(load_user_in(&dir, "my-beat").unwrap(), session);
        assert_eq!(list_user_in(&dir).len(), 1);
        assert_eq!(list_user_in(&dir)[0].name, "My Beat");
    }

    #[test]
    fn saving_the_same_name_twice_replaces_rather_than_multiplies() {
        let dir = scratch();
        save_in(&dir, "Take", PluginSession::default()).unwrap();
        save_in(
            &dir,
            "Take",
            PluginSession {
                seed: "9".into(),
                ..PluginSession::default()
            },
        )
        .unwrap();

        assert_eq!(list_user_in(&dir).len(), 1);
        assert_eq!(load_user_in(&dir, "take").unwrap().seed, "9");
    }

    #[test]
    fn a_deleted_preset_is_gone_and_deleting_it_again_says_so() {
        let dir = scratch();
        save_in(&dir, "Gone", PluginSession::default()).unwrap();
        delete_in(&dir, "gone").unwrap();

        assert!(list_user_in(&dir).is_empty());
        assert!(delete_in(&dir, "gone").is_err());
    }

    #[test]
    fn a_name_cannot_escape_the_preset_directory() {
        // ⛔ The name is typed into a webview and becomes a filename. Every one
        // of these used to be a write outside the preset directory.
        for attempt in [
            "../../etc/passwd",
            "..\\..\\Windows\\System32",
            "C:/Windows/system.ini",
            "/etc/shadow",
            "with\0nul",
            "..",
        ] {
            let stem = slug(attempt);
            assert!(
                !stem.contains('/')
                    && !stem.contains('\\')
                    && !stem.contains(':')
                    && !stem.contains('\0'),
                "{attempt} slugged to {stem}"
            );
            assert!(
                stem != ".." && !stem.contains(".."),
                "{attempt} slugged to {stem}"
            );
            assert!(
                is_safe_stem(&stem),
                "{attempt} slugged to an unusable {stem}"
            );
        }
    }

    #[test]
    fn an_id_that_is_not_a_slug_is_refused_before_it_reaches_the_filesystem() {
        let dir = scratch();
        for attempt in ["../secret", "..", "a/b", "A", "with space", ""] {
            assert!(
                load_user_in(&dir, attempt).is_err(),
                "`{attempt}` should not be loadable"
            );
            assert!(
                delete_in(&dir, attempt).is_err(),
                "`{attempt}` should not be deletable"
            );
        }
    }

    #[test]
    fn a_name_with_no_ascii_in_it_still_gets_a_stable_filename() {
        // 18 locales. A name that is entirely Japanese must save, and must save
        // to the same place next time.
        let stem = slug("ビート");
        assert!(is_safe_stem(&stem), "{stem}");
        assert_eq!(stem, slug("ビート"));
        assert_ne!(stem, slug("비트"));
    }

    #[test]
    fn a_factory_preset_cannot_be_deleted() {
        assert!(delete("factory/trap").is_err());
    }

    #[test]
    fn every_factory_preset_parses() {
        let presets = dataset::factory_presets();
        assert!(!presets.is_empty(), "no factory presets were embedded");

        for (stem, text) in presets {
            serde_json::from_str::<Preset>(text)
                .unwrap_or_else(|error| panic!("data/presets/{stem}.json does not parse: {error}"));
        }
    }

    #[test]
    fn every_factory_preset_names_a_model_that_exists() {
        // ⛔ The dataset's recurring failure, applied here: a preset naming an
        // artist id that is not in the roster selects nothing, generates
        // nothing, and says nothing. A typo would cost that preset silently.
        for summary in factory_summaries() {
            let session = load(&summary.id).unwrap();
            let id = session
                .selected_id
                .unwrap_or_else(|| panic!("{} selects no artist at all", summary.id));
            dataset::model(&id).unwrap_or_else(|error| {
                panic!("{} names `{id}`, which is not a model: {error}", summary.id)
            });
        }
    }

    #[test]
    fn no_factory_preset_pins_a_tempo() {
        // ⛔ Precedence is user pin > host > model. A shipped preset that pins
        // `bpm` overrides the DAW the moment it is loaded — in the plugin whose
        // entire reason for existing is following the host's tempo. A pin is
        // the user's instruction, and a file we ship is not the user.
        for summary in factory_summaries() {
            let session = load(&summary.id).unwrap();
            assert_eq!(
                session.pins.bpm, None,
                "{} pins a tempo, which would beat the host",
                summary.id
            );
        }
    }

    #[test]
    fn the_factory_list_is_ordered_and_marked() {
        let all = factory_summaries();
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "the list a user reads must be ordered");
        assert!(all.iter().all(|p| p.factory));
        assert!(all.iter().all(|p| p.id.starts_with(FACTORY)));
    }
}
