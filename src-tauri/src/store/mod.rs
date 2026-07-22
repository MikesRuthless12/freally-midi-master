//! Persisted application settings.
//!
//! Lives in the OS app-data dir as `settings.json` (PRD § 3), not in the
//! WebView's localStorage — settings must survive a profile reset, be
//! inspectable, and be backed up with everything else.

pub mod settings;

pub use settings::{Settings, SettingsError};
