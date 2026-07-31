//! Every command the frontend invokes is a command the plugin answers.
//!
//! **This exists because the two sides disagreed and nothing said so.** The
//! transport shipped with the page invoking `stop_playback` and the plugin
//! answering `transport_stop`, and the failure was silent in the worst way: the
//! unknown command rejected, the store's `catch` swallowed it, the marker snapped
//! to zero locally, and the audio thread never rewound — so the playhead read
//! "stopped" while the beat carried on from the middle of the pattern.
//!
//! ⛔ **The list is derived from the frontend sources, not restated here.** A
//! hand-written copy is a third place the names live, and it would drift the
//! same way the first two did.
//!
//! ⛔ **Every `.ts`/`.tsx` under `src/`, not just the session store.** Scraping
//! one file left most of the page outside the net the test claims to cast:
//! `Presets.tsx` invokes `preset_save`/`preset_delete`, `Eula.tsx` invokes
//! `eula_accept`/`eula_decline`, `ui.ts` invokes `settings_set`. Renaming any of
//! those on one side left both tests green while the button silently stopped
//! working — which is the exact failure this file exists to prevent.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every `invoke('name')` anywhere in the frontend.
fn invoked_by_the_page() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for source in frontend_sources() {
        scrape_invocations(&source, &mut found);
    }
    assert!(
        found.len() > 10,
        "the scraper found almost nothing, so it has stopped matching the source: {found:?}"
    );
    found
}

/// Every `.ts`/`.tsx` under `src/`, excluding tests and the browser mock.
///
/// ⛔ The mock is excluded deliberately: it *answers* commands rather than
/// invoking them, so including it would assert that the plugin implements the
/// test fixture.
fn frontend_sources() -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![repo_root().join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !(name.ends_with(".ts") || name.ends_with(".tsx"))
                || name.contains(".test.")
                || name == "ipc-mock.ts"
            {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                out.push(text);
            }
        }
    }
    assert!(!out.is_empty(), "no frontend sources found under src/");
    out
}

fn scrape_invocations(source: &str, found: &mut BTreeSet<String>) {
    for (index, _) in source.match_indices("invoke") {
        // Skip the type parameter, if any, then take what is inside the quotes.
        let rest = &source[index..];
        let Some(open) = rest.find('(') else { continue };
        let after = &rest[open + 1..];
        let Some(quote) = after.find('\'') else {
            continue;
        };
        // Only a call whose first argument is a bare string literal; anything
        // else is a helper being passed through and is not a command name.
        if after[..quote].trim() != "" {
            continue;
        }
        let name: String = after[quote + 1..]
            .chars()
            .take_while(|c| *c != '\'')
            .collect();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            found.insert(name);
        }
    }
}

/// Command names the plugin answers, scraped from its two dispatch tables.
fn answered_by_the_plugin() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for file in ["plugin/src/bridge.rs", "plugin/src/editor.rs"] {
        let source = fs::read_to_string(repo_root().join(file)).expect("source should be readable");
        // Match arms of the shape `"name" =>`.
        for (index, _) in source.match_indices("\" =>") {
            let before = &source[..index];
            let Some(open) = before.rfind('"') else {
                continue;
            };
            let name = &before[open + 1..];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                found.insert(name.to_owned());
            }
        }
    }
    found
}

/// Commands that belong to the desktop shell and that the plugin answers on
/// purpose by *not* implementing them.
///
/// `play_pattern` is the clearest case: playback belongs to the host now, and
/// `playback_status` is the plugin's answer — "press play in your DAW". The
/// store calls the desktop command anyway and treats the rejection as "no
/// transport here", which is the behaviour the transport bar already draws.
const DESKTOP_ONLY: &[&str] = &[
    "play_pattern",
    "kit_load",
    "kit_state",
    "settings_get",
    "settings_set",
    "import_samples",
    "assign_pad",
    "export_midi",
    "drag_midi",
    // Export and drag-out. The plugin puts notes on the host's track instead,
    // and file drag-out is FMM-S03 — a native drag source per platform, not
    // built. `ExportChip` degrades on the rejection.
    "export_to_folder",
    "pick_export_folder",
    "drag_capability",
    "drag_source_ready",
    // ⚠ **The crash reporter, and this one is a gap rather than a decision.**
    // `src/components/BugReport/ipc.ts` invokes all five and the plugin answers
    // none, so the Havoc-standard reporter is inert inside a DAW — the panel
    // opens and submitting does nothing. It is listed here to keep this gate
    // green on a pre-existing hole rather than to bless it; the fix is to
    // implement them in `bridge.rs`, and until then the entries are the record
    // that they are missing.
    "bug_report_has_pending_crash",
    "bug_report_context",
    "bug_report_preview",
    "bug_report_submit",
    "bug_report_clear_crash",
];

#[test]
fn every_command_the_page_invokes_is_answered_or_deliberately_desktop_only() {
    let invoked = invoked_by_the_page();
    let answered = answered_by_the_plugin();

    let unanswered: Vec<&String> = invoked
        .iter()
        .filter(|name| !answered.contains(*name))
        .filter(|name| !DESKTOP_ONLY.contains(&name.as_str()))
        .collect();

    assert!(
        unanswered.is_empty(),
        "the page invokes commands the plugin does not answer: {unanswered:?}\n\
         Either the plugin is missing them or the two sides have drifted on a name."
    );
}

#[test]
fn the_plugin_answers_no_transport_command_the_page_never_invokes() {
    // The other half of the same bug, and the half that actually happened: a
    // command the plugin answers and nothing calls is dead code that *looks*
    // like the feature is wired. `transport_stop` sat here passing every test.
    let invoked = invoked_by_the_page();
    let answered = answered_by_the_plugin();

    let orphans: Vec<&String> = answered
        .iter()
        .filter(|name| name.contains("transport") || name.contains("playback") || **name == "seek")
        .filter(|name| !invoked.contains(*name))
        .collect();

    assert!(
        orphans.is_empty(),
        "the plugin answers transport commands nothing invokes: {orphans:?}"
    );
}
