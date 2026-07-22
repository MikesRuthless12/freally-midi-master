//! `settings.json` in the OS app-data dir.
//!
//! Every field has a default, and an unreadable or partly-unknown file falls
//! back to defaults rather than refusing to start — a settings file is not
//! worth losing a session over. Writes are atomic, so a crash mid-write cannot
//! leave a truncated file that the next launch then discards.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    NoDataDir,
    Io(String),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::NoDataDir => write!(f, "no app-data directory available"),
            SettingsError::Io(m) => write!(f, "{m}"),
        }
    }
}

/// Theme preference. Mirrors `ThemePreference` in `src/state/theme.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Minimising sends the window to the system tray instead of the taskbar.
    pub minimize_to_tray: bool,
    /// Closing the window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Show the tray icon at all. Off means the two options above cannot apply.
    pub show_tray_icon: bool,
    pub theme: ThemePreference,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Off by default: a window that vanishes from the taskbar when a
            // user minimises it is alarming if they did not ask for it.
            minimize_to_tray: false,
            close_to_tray: false,
            show_tray_icon: true,
            theme: ThemePreference::default(),
        }
    }
}

fn settings_path() -> Result<PathBuf, SettingsError> {
    let dirs = directories::ProjectDirs::from("com", "Havoc Software", "Freally MIDI Master")
        .ok_or(SettingsError::NoDataDir)?;
    let dir = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&dir).map_err(|e| SettingsError::Io(e.to_string()))?;
    Ok(dir.join("settings.json"))
}

impl Settings {
    /// Read from disk, falling back to defaults for anything missing or broken.
    pub fn load() -> Self {
        let Ok(path) = settings_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        // `#[serde(default)]` on the struct means an older file missing a field
        // gains that field's default rather than failing the whole parse.
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), SettingsError> {
        let path = settings_path()?;
        let text =
            serde_json::to_string_pretty(self).map_err(|e| SettingsError::Io(e.to_string()))?;

        // Temp + rename, so an interrupted write cannot truncate the real file.
        //
        // Nothing is deleted first. `fs::rename` replaces the destination on
        // every platform this ships to — including Windows, where std uses
        // SetFileInformationByHandle with ReplaceIfExists — so a pre-emptive
        // `remove_file` buys nothing and costs the atomicity this comment
        // claims: between the delete and the rename there is a window where
        // settings.json simply does not exist, and any failure inside it (a
        // rename refused by an antivirus handle, a full disk, a kill) loses
        // every preference silently, because `load` treats a missing file as
        // "use the defaults".
        let tmp = path.with_extension("json.part");
        fs::write(&tmp, format!("{text}\n")).map_err(|e| SettingsError::Io(e.to_string()))?;
        match fs::rename(&tmp, &path) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Never leave our temp file behind.
                let _ = fs::remove_file(&tmp);
                Err(SettingsError::Io(e.to_string()))
            }
        }
    }
}

#[tauri::command]
pub fn settings_get() -> Settings {
    Settings::load()
}

#[tauri::command]
pub fn settings_set(app: tauri::AppHandle, settings: Settings) -> Result<Settings, String> {
    settings.save().map_err(|e| e.to_string())?;
    // The tray follows the setting immediately. Leaving it until the next
    // launch is what allowed "show a tray icon" to be on with no icon anywhere
    // — and close-to-tray in that state hides the window with no way back.
    crate::tray::sync(&app).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_do_not_surprise_the_user() {
        let s = Settings::default();
        assert!(!s.minimize_to_tray, "a window must not vanish unasked");
        assert!(
            !s.close_to_tray,
            "close must mean close until asked otherwise"
        );
        assert!(s.show_tray_icon);
        assert_eq!(s.theme, ThemePreference::System);
    }

    #[test]
    fn settings_round_trip_through_json() {
        let s = Settings {
            minimize_to_tray: true,
            close_to_tray: true,
            show_tray_icon: true,
            theme: ThemePreference::Dark,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("minimizeToTray"),
            "wire format is camelCase: {json}"
        );
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
    }

    #[test]
    fn an_older_file_missing_fields_gains_their_defaults() {
        // A settings file written by a previous version must not be discarded
        // wholesale just because a field was added since.
        let s: Settings = serde_json::from_str(r#"{"minimizeToTray":true}"#).unwrap();
        assert!(s.minimize_to_tray);
        assert!(s.show_tray_icon, "a missing field takes its default");
        assert_eq!(s.theme, ThemePreference::System);
    }

    #[test]
    fn an_unknown_field_does_not_break_the_parse() {
        // Downgrading after a newer version wrote extra keys must still work.
        let s: Settings =
            serde_json::from_str(r#"{"minimizeToTray":true,"somethingNew":42}"#).unwrap();
        assert!(s.minimize_to_tray);
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults_rather_than_panicking() {
        let s: Settings = serde_json::from_str("{ not json").unwrap_or_default();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn saving_over_an_existing_file_never_leaves_it_missing() {
        // The remove-then-rename this replaces had a window where settings.json
        // did not exist at all; a failure inside it silently reverted every
        // preference, because `load` reads a missing file as "use the defaults".
        let Ok(path) = settings_path() else {
            return; // no app-data dir on this runner; nothing to assert
        };
        let restore = fs::read(&path).ok();

        Settings {
            minimize_to_tray: true,
            close_to_tray: false,
            show_tray_icon: true,
            theme: ThemePreference::Light,
        }
        .save()
        .unwrap();
        assert!(path.exists());

        // Overwrite: the file must never stop existing, and the new value must
        // be what comes back.
        Settings {
            theme: ThemePreference::Dark,
            ..Default::default()
        }
        .save()
        .unwrap();
        assert!(path.exists(), "the file vanished during an overwrite");
        assert_eq!(Settings::load().theme, ThemePreference::Dark);

        // And no temp file survives a successful write.
        assert!(!path.with_extension("json.part").exists());

        match restore {
            Some(bytes) => fs::write(&path, bytes).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    #[test]
    fn theme_serializes_the_same_way_the_ui_spells_it() {
        // Must match ThemePreference in src/state/theme.ts, or the two sides
        // disagree about what "system" means.
        for (value, expected) in [
            (ThemePreference::System, "\"system\""),
            (ThemePreference::Dark, "\"dark\""),
            (ThemePreference::Light, "\"light\""),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), expected);
        }
    }
}
