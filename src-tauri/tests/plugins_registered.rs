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

/// lib.rs with every comment removed.
///
/// Searching the raw text was the hole in this guard: a substring match is
/// satisfied by a *commented-out* `.plugin(...)` line — which is exactly how a
/// developer disables one while debugging, and exactly the regression this file
/// exists to catch. A doc-comment naming a plugin that was since deleted does
/// the same. Only code counts.
fn lib_code() -> String {
    strip_comments(&read("src/lib.rs"))
}

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_block = false;
    for line in source.lines() {
        let mut rest = line;
        loop {
            if in_block {
                match rest.find("*/") {
                    Some(end) => {
                        in_block = false;
                        rest = &rest[end + 2..];
                    }
                    None => break,
                }
            } else if let Some(start) = rest.find("/*") {
                out.push_str(&rest[..start]);
                in_block = true;
                rest = &rest[start + 2..];
            } else {
                // A line comment ends the line. `//` inside a string literal
                // would be mis-cut, but lib.rs has none and a false *negative*
                // here can only make the test stricter, never weaker.
                out.push_str(rest.split("//").next().unwrap_or(""));
                break;
            }
        }
        out.push('\n');
    }
    out
}

/// Is this plugin actually registered — `.plugin(tauri_plugin_foo::…)` in code?
fn is_registered(code: &str, module: &str) -> bool {
    code.split(".plugin(")
        .skip(1)
        .any(|call| call.trim_start().starts_with(&format!("{module}::")))
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
    let code = lib_code();

    let declared = plugin_dependencies(&manifest);
    assert!(
        !declared.is_empty(),
        "no tauri plugins found in Cargo.toml — has the manifest format changed?"
    );

    let missing: Vec<&String> = declared
        .iter()
        .filter(|module| !is_registered(&code, module))
        .collect();

    assert!(
        missing.is_empty(),
        "these plugins are dependencies but are never initialised in lib.rs: {missing:?}\n\
         Either register them with .plugin(...) or remove them from Cargo.toml — \
         an unregistered plugin is a dead feature that still compiles."
    );
}

#[test]
fn a_commented_out_registration_does_not_count() {
    // The guard's own failure mode, asserted directly. Commenting the line out
    // is how a developer disables a plugin while debugging, and it is how the
    // original regression got in — so a substring search over the raw file
    // cannot be what this test relies on.
    //
    // Run through the real `strip_comments`, not a copy of it: a test that
    // re-implements the thing it is checking reports on the copy and can pass
    // while the shipped one is broken.
    let disabled = r#"
        tauri::Builder::default()
            // .plugin(tauri_plugin_updater::Builder::new().build())
            /* .plugin(tauri_plugin_drag::init()) */
            .plugin(tauri_plugin_opener::init())
    "#;
    let code = strip_comments(disabled);

    assert!(
        is_registered(&code, "tauri_plugin_opener"),
        "a real registration must still be found"
    );
    // Both of these appear verbatim in the text above; only the parse tells
    // them apart from the live call.
    assert!(!is_registered(&code, "tauri_plugin_updater"));
    assert!(!is_registered(&code, "tauri_plugin_drag"));
}

#[test]
fn the_updater_and_drag_plugins_are_registered() {
    // Named explicitly because these two are the ones that went missing, and
    // both are invisible to every other test: the updater only runs against a
    // real release, and drag-out only against a real DAW.
    let code = lib_code();
    for module in ["tauri_plugin_updater", "tauri_plugin_drag"] {
        assert!(
            is_registered(&code, module),
            "{module} is not initialised — that feature is dead"
        );
    }
}
