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

/// The window the UI was designed against (PRD § 8's minimum).
///
/// Smaller than the desktop app's window because a plugin lives inside a
/// host's frame, and every DAW gives it less room than a full screen.
const SIZE: (u32, u32) = (1280, 760);

pub fn create(shared: SharedState) -> Option<Box<dyn Editor>> {
    let editor = WebViewEditor::new(HTMLSource::URL("freally://localhost/index.html"), SIZE)
        // The app's own background, so a slow first paint is the app's colour
        // rather than a white flash inside a dark DAW.
        .with_background_color((11, 11, 13, 255))
        .with_developer_mode(cfg!(debug_assertions))
        .with_custom_protocol(SCHEME.into(), serve)
        .with_event_loop(move |ctx, _setter, _window| {
            // Free anything the audio thread parked. This is the thread that
            // is allowed to.
            shared.handoff.collect();

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
                let reply = match bridge::dispatch(&request, &host) {
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

/// Serve one file out of the compiled-in frontend.
fn serve(
    request: &nih_plug_webview::http::Request<Vec<u8>>,
) -> wry::Result<nih_plug_webview::http::Response<Cow<'static, [u8]>>> {
    use nih_plug_webview::http::{header::CONTENT_TYPE, Response, StatusCode};

    let path = request.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

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
