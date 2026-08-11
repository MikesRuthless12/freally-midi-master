//! Windows resources for the standalone executable, and the one dependency
//! cargo cannot work out for itself: the embedded dataset.
//!
//! Without this the standalone ships with the stock Rust/MSVC application icon —
//! a blank window frame in the taskbar, in Explorer and in Alt-Tab. The plugin
//! is the product's face; a producer should recognise it before they read the
//! filename.
//!
//! ## Scope, and what this deliberately does not do
//!
//! Windows only, and the whole file is a no-op elsewhere: macOS takes its icon
//! from the `.app` bundle's `Info.plist` and Linux from a `.desktop` entry, so
//! neither has anything to embed here.
//!
//! The resource is attached to **every target this crate builds**, which
//! includes the `cdylib` a DAW loads. That is harmless — a DLL may carry an icon
//! and nothing looks for one — and it is not worth splitting the crate to avoid.
//!
//! ⛔ **`icon.ico` is DIB entries, not PNG-in-ICO.** Windows has accepted
//! PNG-compressed icon entries since Vista, but the resource compiler and older
//! shell surfaces are less forgiving, and an icon that silently falls back to the
//! stock one is the kind of failure nobody notices until release day. Regenerate
//! it from `images/freally_app_icon.png` if the artwork changes.

fn main() {
    // Re-run only when the artwork or this file changes. Without these, cargo
    // conservatively re-runs the script whenever anything in the crate changes,
    // which relinks the standalone on every edit.
    println!("cargo:rerun-if-changed=icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    // ⛔ **The dataset is compiled into the plugin, and cargo cannot see that.**
    // `dataset.rs` does `include_dir!("$CARGO_MANIFEST_DIR/../data")`, which
    // reads every model at *compile* time. A macro's file reads are invisible to
    // cargo's dependency graph, so editing a model changes no Rust source, the
    // crate is judged fresh, and the plugin keeps serving the roster it was last
    // built with — a stale genre list with nothing anywhere saying so.
    //
    // Found on 2026-08-10 by `the_embedded_copy_matches_the_one_on_disk`, and
    // only because a model happened to be edited between two builds. Without
    // this line that test is load-bearing by luck: it compares the embedded copy
    // against `data/`, but a build that never re-embedded still passes whenever
    // the two happen to agree.
    //
    // ⚠ This does mean any model edit relinks the plugin. That is the cost of
    // the data being *in* the binary, and it is the correct trade against
    // shipping a roster that silently does not match the dataset.
    println!("cargo:rerun-if-changed=../data");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("icon.ico");
        resource.set("ProductName", "Freally MIDI Master");
        resource.set(
            "FileDescription",
            "Freally MIDI Master — artist-accurate MIDI generator",
        );
        resource.set("CompanyName", "Mike Weaver");
        resource.set("LegalCopyright", "© 2026 Mike Weaver — All Rights Reserved");

        // ⚠ A failure here must not fail the build. The resource compiler comes
        // from the Windows SDK, and a machine without it can still produce a
        // perfectly working plugin — it simply gets the stock icon. Losing the
        // ability to build over a cosmetic resource would be the wrong trade.
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {error}");
        }
    }
}
