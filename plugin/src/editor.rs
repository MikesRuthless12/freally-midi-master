//! The plugin's window: the existing React UI, in a webview.
//!
//! This is the part of the pivot that is *not* free. `nih-plug-webview` is
//! explicitly work-in-progress and macOS/Windows only, so this module owns
//! more of the integration than a dependency normally would, and Linux is not
//! answered here yet.
//!
//! What it buys is the whole frontend: the same React app, the same 18 locale
//! catalogs, the same design tokens, drum grid and session chips the desktop
//! app shipped. `src/lib/ipc.ts` was always the single seam between the UI and
//! the core, and [`crate::bridge`] is simply what sits behind it now.
//!
//! The built app is compiled in for the same reason the dataset is: a plugin
//! is a shared library in someone else's process, with no resource directory
//! and no working directory it chose. **`npm run build` must therefore run
//! before `cargo build`** — the same ordering Tauri required.

use std::borrow::Cow;

use include_dir::{include_dir, Dir};
use nih_plug::prelude::*;
use nih_plug_webview::{HTMLSource, WebViewEditor};
use serde_json::{json, Value};

use crate::bridge::{self, Request};
use crate::shared::SharedState;
use crate::voice::Schedule;

/// The built frontend. Vite writes `index.html` plus hashed assets here.
static UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

/// The scheme the webview loads from.
///
/// A custom protocol rather than `HTMLSource::String`, because Vite emits a
/// separate JS bundle, a CSS file and the font files — an inlined single page
/// would mean either a second build pipeline or shipping the app without its
/// fonts, and the font fallback chain is something this project has already
/// had to fix once.
const SCHEME: &str = "freally";

/// The window the UI is designed for, in **CSS pixels**.
///
/// 1440x900 rather than the 1280x760 this used to ask for, and the width is the
/// number that matters: the right rail collapses below **1440**, so at 1280 the
/// kit and session panels were hidden the moment the editor opened and had to
/// be summoned with K. 1280 is the UI's hard *minimum* (PRD § 8), not the size
/// it was drawn against — asking for the minimum meant every host got the
/// degraded layout.
/// The layout the UI is always given, in CSS pixels, whatever size the window
/// is on screen.
///
/// 1440 is the width at which the right rail stays open; below it the kit and
/// session panels collapse and have to be summoned with K. **This number does
/// not change when the window gets smaller** — that is the whole point of
/// [`SCALES`]. Shrinking the window by shrinking the layout is what would take
/// the rail away again.
const LAYOUT: (u32, u32) = (1440, 900);

/// How large that layout is *drawn*, smallest first.
///
/// A 1440-wide layout at 100% is nearly a whole 1707-wide desktop once the
/// display is scaled — so "bigger" was never the useful direction. These shrink
/// the picture instead: the window takes less of the screen while the page
/// still lays out at [`LAYOUT`] and still shows every panel, because the page
/// is zoomed by the same factor the window is.
///
/// **Presets rather than a draggable edge**, because the vendored adapter does
/// not forward `Event::Window(Resized)` — its `on_event` handles keyboard and
/// mouse and nothing else — so a window the host resized would leave the page
/// inside it laid out at the old size. Teaching it to would mean editing
/// `src/lib.rs`, which `VENDORED.md` deliberately keeps byte-for-byte upstream.
const SCALES: &[(&str, f32)] = &[("small", 0.7), ("medium", 0.85), ("large", 1.0)];

/// What the editor opens at. Not the largest: a window that fills the host on
/// first insert is a window the user has to deal with before they can work.
const DEFAULT_SCALE: &str = "medium";

/// A named scale: the window in physical pixels, and the zoom the page applies.
///
/// The two must agree. The window is `LAYOUT * system_scale * factor` and the
/// page is zoomed by `factor`, so the CSS viewport works out at `LAYOUT` again
/// — a smaller window showing the same layout, rather than less of it.
fn preset(name: &str) -> Option<((u32, u32), f32)> {
    SCALES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, factor)| (physical(LAYOUT, *factor), *factor))
}

/// The window size to ask the host for, in **physical** pixels.
///
/// ⛔ The size handed to `WebViewEditor` is consumed as *physical* pixels while
/// the page inside is laid out in *CSS* pixels, and on a scaled display those
/// are not the same number. The vendored adapter says as much: its
/// `set_scale_factor` is a stub returning `false` with "TODO: implement for
/// Windows and Linux", so nih-plug's own DPI plumbing never reaches it.
///
/// The effect on a 150% display — which is an ordinary laptop, not an exotic
/// setup — was that asking for 1280 gave the UI **853** CSS pixels. Below its
/// own minimum, so the layout ran cramped and the right rail auto-collapsed,
/// and nothing anywhere said why. Multiplying here is what makes the request
/// mean what it says.
///
/// Scaling up is safe because the result is clamped to the work area below: on
/// a 100% display this is exactly [`LOGICAL_SIZE`], and on a small screen it
/// shrinks to fit rather than opening a window with its controls off-screen.
fn physical((w, h): (u32, u32), factor: f32) -> (u32, u32) {
    let scale = system_scale() * factor;
    let (max_w, max_h) = work_area().unwrap_or((u32::MAX, u32::MAX));

    (
        ((w as f32 * scale) as u32).min(max_w),
        ((h as f32 * scale) as u32).min(max_h),
    )
}

/// The size the editor opens at.
fn window_size() -> (u32, u32) {
    preset(DEFAULT_SCALE)
        .map(|(size, _)| size)
        .unwrap_or(LAYOUT)
}

/// Commands the *window* owns, rather than the engine.
///
/// [`bridge::dispatch`] answers for the dataset and the generator and knows
/// nothing about a window — it cannot, because the window exists only inside
/// the frame loop. Handled before dispatch so that the bridge's "an unknown
/// command fails loudly" rule keeps meaning what it says: this is a *known*
/// command that simply lives on the other side of the seam.
///
/// Returns `None` when the command is not one of these, so the caller falls
/// through to the bridge.
fn window_command(request: &Request, shared: &SharedState) -> Option<Result<Value, String>> {
    if request.command != "set_editor_size" {
        return None;
    }

    let name = request.args["size"].as_str().unwrap_or_default();
    Some(match preset(name) {
        Some(((width, height), factor)) => {
            shared.request_resize(width, height);
            // `zoom` is what the page must apply for the layout to come out at
            // `LAYOUT` inside a window this size. Sent back rather than
            // duplicated in the frontend, so the two cannot drift apart.
            Ok(json!({ "width": width, "height": height, "zoom": factor }))
        }
        None => {
            let known: Vec<&str> = SCALES.iter().map(|(id, _)| *id).collect();
            Err(format!(
                "`{name}` is not a window size — expected one of {}",
                known.join(", ")
            ))
        }
    })
}

/// The desktop scale factor, as a multiplier (1.5 at 150%).
///
/// Windows only, because Windows is where the adapter's TODO bites. macOS
/// reports a backing scale factor through Cocoa that baseview already applies,
/// and Linux has no editor at all until TASK-P12 — both fall through to 1.0,
/// which is the same behaviour as before this function existed.
#[cfg(target_os = "windows")]
fn system_scale() -> f32 {
    // `user32` is already linked by the window this plugin opens; declaring the
    // one call is cheaper than taking a dependency on `windows` for it.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForSystem() -> u32;
    }

    // 96 DPI is 100%. A zero would mean the call failed, and dividing by it
    // would hand the host a window of size NaN.
    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 {
        return 1.0;
    }
    (dpi as f32 / 96.0).clamp(1.0, 4.0)
}

#[cfg(not(target_os = "windows"))]
fn system_scale() -> f32 {
    1.0
}

/// The usable desktop, in physical pixels, so the window cannot open larger
/// than the screen it appears on.
///
/// `SM_CXMAXIMIZED`/`SM_CYMAXIMIZED` rather than the raw screen size: it
/// excludes the taskbar, which is what "as big as it can usefully be" means.
#[cfg(target_os = "windows")]
fn work_area() -> Option<(u32, u32)> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }

    const SM_CXMAXIMIZED: i32 = 61;
    const SM_CYMAXIMIZED: i32 = 62;

    let (w, h) = unsafe {
        (
            GetSystemMetrics(SM_CXMAXIMIZED),
            GetSystemMetrics(SM_CYMAXIMIZED),
        )
    };
    (w > 0 && h > 0).then_some((w as u32, h as u32))
}

#[cfg(not(target_os = "windows"))]
fn work_area() -> Option<(u32, u32)> {
    None
}

pub fn create(shared: SharedState) -> Option<Box<dyn Editor>> {
    let editor = WebViewEditor::new(
        HTMLSource::URL("freally://localhost/index.html"),
        window_size(),
    )
    // The app's own background, so a slow first paint is the app's colour
    // rather than a white flash inside a dark DAW.
    .with_background_color((11, 11, 13, 255))
    // On unless `FREALLY_NO_DEVTOOLS` is set, rather than only in debug
    // builds. A plugin runs inside someone else's process: there is no
    // console to read, no stderr anyone will see, and a release build is
    // the only build a DAW ever loads — so gating devtools on
    // `debug_assertions` means the one configuration that can fail in a
    // host is the one configuration that cannot be inspected. That is how
    // an afternoon goes into guessing at an IPC timeout.
    .with_developer_mode(std::env::var("FREALLY_NO_DEVTOOLS").is_err())
    .with_custom_protocol(SCHEME.into(), {
        let shared = shared.clone();
        move |request| serve(request, &shared)
    })
    .with_event_loop(move |ctx, _setter, window| {
        // Free anything the audio thread parked. This is the thread that
        // is allowed to.
        shared.handoff.collect();

        // A size the UI asked for. This is the only place it can be applied:
        // `resize` needs the window, and the window only exists here.
        if let Some((width, height)) = shared.take_resize() {
            ctx.resize(window, width, height);
        }

        while let Ok(message) = ctx.next_event() {
            let Ok(request) = serde_json::from_value::<Request>(message.clone()) else {
                // Not a request shape at all. Loud rather than ignored:
                // silence here is how a UI that is talking to nothing
                // looks exactly like a UI whose command failed.
                ctx.send_json(json!({
                    "type": "response",
                    "id": Value::Null,
                    "error": format!("the plugin could not read this message: {message}"),
                }));
                continue;
            };

            let host = shared.host.snapshot();
            let outcome = window_command(&request, &shared)
                .unwrap_or_else(|| bridge::dispatch(&request, &host, &shared.session));
            let reply = match outcome {
                Ok(value) => {
                    // A generation is the one command with a side effect
                    // beyond its reply: the notes have to reach the audio
                    // thread. Arming happens *here*, on the UI thread,
                    // because that is where the allocation belongs.
                    if request.command == "generate_pattern" {
                        if let Ok(pattern) = serde_json::from_value(value.clone()) {
                            let mut schedule = Schedule::default();
                            schedule.arm(&pattern, shared.sample_rate());
                            shared.handoff.send(schedule);
                        }
                    }
                    json!({ "type": "response", "id": request.id, "ok": value })
                }
                Err(message) => {
                    json!({ "type": "response", "id": request.id, "error": message })
                }
            };
            ctx.send_json(reply);
        }
    });

    Some(Box::new(editor))
}

/// The path the UI posts commands to.
///
/// **The bridge is an HTTP round trip over the custom protocol, not the
/// webview's IPC channel.** That is not the obvious choice, and it is the only
/// one that works in a hosted plugin window: wry's IPC is one-way, so a reply
/// has to be pushed back with `evaluate_script` from the editor's *frame loop*
/// — and a window parented into Ableton Live never gets a frame tick. Every
/// command queued forever and nothing was ever answered.
///
/// A custom-protocol request is called synchronously by the webview and
/// returns a body, so the request and its reply are one exchange that depends
/// on no tick at all. The page already loads over this protocol, which is what
/// makes it the proven path rather than the second guess.
const RPC_PATH: &str = "__rpc";

/// Serve one file out of the compiled-in frontend, or answer a command.
fn serve(
    request: &nih_plug_webview::http::Request<Vec<u8>>,
    shared: &SharedState,
) -> wry::Result<nih_plug_webview::http::Response<Cow<'static, [u8]>>> {
    use nih_plug_webview::http::{header::CONTENT_TYPE, Response, StatusCode};

    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if path == RPC_PATH {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            // The page is served from this same origin, but WebView2 treats a
            // custom scheme as opaque for fetch unless it is told otherwise.
            .header("Access-Control-Allow-Origin", "*")
            .body(Cow::Owned(rpc(request.body(), shared).into_bytes()))
            .unwrap());
    }

    match UI.get_file(path) {
        Some(file) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, mime_for(path))
            .body(Cow::Borrowed(file.contents()))
            .unwrap()),
        // A 404 rather than falling back to `index.html`. SPA fallbacks make a
        // missing asset render as a blank page with no error — the frontend
        // has no router, so a miss here is a build that did not produce what
        // the HTML asks for, and it should say so.
        None => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, "text/plain")
            .body(Cow::Owned(
                format!("{path} is not in the bundled UI").into_bytes(),
            ))
            .unwrap()),
    }
}

/// Answer one command, as JSON.
///
/// Never fails the HTTP request: a command that errors still returns 200 with
/// an `error` field, because a non-200 arrives at the page as a network
/// failure with no message in it — and "failed to fetch" is exactly the kind
/// of unattributable error this bridge has already cost an evening to.
fn rpc(body: &[u8], shared: &SharedState) -> String {
    let reply = match serde_json::from_slice::<Request>(body) {
        Ok(request) => {
            let host = shared.host.snapshot();
            let outcome = window_command(&request, shared)
                .unwrap_or_else(|| bridge::dispatch(&request, &host, &shared.session));
            match outcome {
                Ok(value) => {
                    // A generation is the one command with a side effect
                    // beyond its reply: the notes have to reach the audio
                    // thread. Arming happens here, off the audio thread,
                    // because that is where the allocation belongs.
                    if request.command == "generate_pattern" {
                        if let Ok(pattern) = serde_json::from_value(value.clone()) {
                            let mut schedule = Schedule::default();
                            schedule.arm(&pattern, shared.sample_rate());
                            shared.handoff.send(schedule);
                        }
                    }
                    json!({ "id": request.id, "ok": value })
                }
                Err(message) => json!({ "id": request.id, "error": message }),
            }
        }
        Err(error) => json!({ "error": format!("the plugin could not read this call: {error}") }),
    };

    reply.to_string()
}

/// Content types for what Vite emits.
///
/// Explicit rather than a crate: the list is short, and a webview that guesses
/// `text/plain` for a module script refuses to run it — which presents as a
/// blank window with nothing in the console.
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_ui_is_embedded() {
        // The failure this catches is a build-order one: `cargo build` without
        // a prior `npm run build` embeds a stale or absent `dist/`, and the
        // plugin opens a blank window with no error anywhere.
        assert!(
            UI.get_file("index.html").is_some(),
            "dist/index.html is missing — run `npm run build` before `cargo build`"
        );
    }

    #[test]
    fn the_index_asks_for_assets_that_are_actually_bundled() {
        // A hashed asset name that does not exist serves a 404 into a webview
        // that will render nothing and say nothing. Checking the references
        // here turns that into a build failure.
        let index = UI
            .get_file("index.html")
            .and_then(|f| f.contents_utf8())
            .expect("index.html must be UTF-8");

        let mut referenced = 0;
        for chunk in index.split(&['"', '\''][..]) {
            let asset = chunk.trim_start_matches('/');
            if !(asset.ends_with(".js") || asset.ends_with(".css")) {
                continue;
            }
            referenced += 1;
            assert!(
                UI.get_file(asset).is_some(),
                "index.html references {asset}, which is not in the bundle"
            );
        }
        assert!(referenced > 0, "index.html references no scripts or styles");
    }

    #[test]
    fn a_module_script_is_served_as_javascript() {
        // A webview handed `text/plain` for a module refuses to run it, and
        // presents as a blank window with an empty console.
        assert!(mime_for("assets/index-abc123.js").starts_with("text/javascript"));
        assert!(mime_for("assets/index-abc123.css").starts_with("text/css"));
        assert_eq!(mime_for("fonts/NotoSans.woff2"), "font/woff2");
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
    }
}
