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
        // Match arms of the shape `"name" =>`, and or-patterns of the shape
        // `"one" | "two" =>`.
        //
        // ⛔ **Both alternatives have to be seen.** An earlier version read only
        // the string immediately before the `=>`, so the first arm of
        // `"transport_play" | "transport_pause" =>` was invisible here — and an
        // invisible answer reads to this gate as a command the plugin does not
        // answer *and*, in the orphan test below, as one nothing invokes. A
        // forward split rather than a backwards walk, because a test whose whole
        // value is being obviously right should not need parsing in its head.
        for line in source.lines() {
            // ⛔ **Comments first.** Both dispatch tables are heavily commented,
            // and those comments quote command names — including, in this very
            // file's neighbourhood, an or-pattern written out to explain the
            // parsing below. A quoted name followed by `=>` inside a comment
            // would register here as a command the plugin answers, which can
            // only ever *mask* a missing handler: the page would invoke it, the
            // bridge would reject it at runtime, and this gate would say the two
            // sides agree.
            let code = line.split_once("//").map_or(line, |(before, _)| before);
            let Some((head, _)) = code.split_once("=>") else {
                continue;
            };
            for piece in head.split('|') {
                let Some(name) = piece
                    .trim()
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                else {
                    continue;
                };
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    found.insert(name.to_owned());
                }
            }
        }
    }
    found
}

/// Commands the page may invoke that the plugin answers on purpose by *not*
/// implementing them.
///
/// ⛔ **Emptied when `src-tauri` was removed, and that is the point of
/// keeping the list rather than the constant.** Every entry named a command the
/// desktop shell answered and the plugin did not — the settings store, the
/// export and drag-out path, the crash reporter, `play_pattern`. With that
/// shell gone the page no longer invokes any of them, so the exemption they
/// bought is not needed and would now hide a real gap: a command the page calls
/// and nothing answers is exactly what this gate exists to fail on.
///
/// Add a name here only with a written reason for why the plugin deliberately
/// does not answer it.
const DESKTOP_ONLY: &[&str] = &[];

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
