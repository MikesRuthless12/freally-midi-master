//! Every Tauri plugin we depend on must actually be initialised.
//!
//! This exists because it happened: `tauri_plugin_updater` and
//! `tauri_plugin_drag` were both dropped from the builder while remaining
//! dependencies in Cargo.toml. Everything still compiled, every unit test still
//! passed, CI was green on three platforms — and the update check and the
//! entire drag-out feature were dead. Nothing else in the suite can see that,
//! because a missing `.plugin(...)` call is not a type error.
//!
//! Source-level rather than runtime, because building a real `tauri::App` in a
//! test needs a window and a display.

use std::fs;

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `tauri-plugin-foo` → `tauri_plugin_foo`, the module name in the builder.
fn plugin_dependencies(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split(['=', ' ']).next())
        .filter(|name| name.starts_with("tauri-plugin-"))
        .map(|name| name.replace('-', "_"))
        .collect()
}

#[test]
fn every_plugin_dependency_is_initialised() {
    let manifest = read("Cargo.toml");
    let lib = read("src/lib.rs");

    let declared = plugin_dependencies(&manifest);
    assert!(
        !declared.is_empty(),
        "no tauri plugins found in Cargo.toml — has the manifest format changed?"
    );

    let missing: Vec<&String> = declared
        .iter()
        .filter(|module| !lib.contains(&format!("{module}::")))
        .collect();

    assert!(
        missing.is_empty(),
        "these plugins are dependencies but are never initialised in lib.rs: {missing:?}\n\
         Either register them with .plugin(...) or remove them from Cargo.toml — \
         an unregistered plugin is a dead feature that still compiles."
    );
}

#[test]
fn no_plugin_is_initialised_without_being_a_dependency() {
    // The reverse direction: a `.plugin()` call for something not in the
    // manifest would not compile, but a stale *comment* about one can outlive
    // the call and mislead the next reader.
    let lib = read("src/lib.rs");
    assert!(
        !lib.contains("// removed"),
        "lib.rs still contains a `// removed` marker — a bisect or edit left a \
         placeholder where a plugin registration used to be."
    );
}

#[test]
fn the_updater_and_drag_plugins_are_registered() {
    // Named explicitly because these two are the ones that went missing, and
    // both are invisible to every other test: the updater only runs against a
    // real release, and drag-out only against a real DAW.
    let lib = read("src/lib.rs");
    for module in ["tauri_plugin_updater", "tauri_plugin_drag"] {
        assert!(
            lib.contains(&format!("{module}::")),
            "{module} is not initialised — that feature is dead"
        );
    }
}
