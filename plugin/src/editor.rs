//! The plugin's window: the existing React UI, in a webview.
//!
//! This is the part of the pivot that is *not* free. `nih-plug-webview` is
//! explicitly work-in-progress, so this module owns more of the integration
//! than a dependency normally would. Upstream is macOS/Windows only; the X11 +
//! WebKitGTK path is ours, in `plugin/vendor/nih-plug-webview/src/linux.rs`
//! (TASK-P12), and `VENDORED.md` records what that cost.
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
use serde::Deserialize;
use serde_json::{json, Value};

use engine::pattern::{Lane, Pattern};

use crate::bridge::{self, Request};
use crate::shared::SharedState;
use crate::state;
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

/// The layout the UI is always given, in CSS pixels, whatever size the window
/// is on screen.
///
/// 1440 is the width at which the right rail stays open; below it the kit and
/// session panels collapse and have to be summoned with K. **This number does
/// not change when the window gets smaller** — that is the whole point of
/// [`SCALES`]. Shrinking the window by shrinking the layout is what would take
/// the rail away again.
const LAYOUT: (u32, u32) = (1440, 900);

/// The smallest the standalone frame may be dragged to, in physical pixels.
///
/// ⛔⛔ **This is not a layout minimum — it is the guarantee that the window can
/// be made big again.** Mike, 2026-08-12: *"i want the end user to be able to
/// resize my window to whatever size they want … if they size it too small, then
/// it's their own fault, as long as you can ensure that they can resize to make
/// it bigger."* Both halves are real. Left at zero, Windows will hand back a
/// client area with no room for the resize grips, and a frame nobody can grab is
/// a frame nobody can grow — the one outcome he ruled out. This is small enough
/// to be "whatever size they want" and large enough that every edge and corner
/// stays draggable.
///
/// ⚠ **Deliberately unrelated to [`LAYOUT`] and to the display scale.** It
/// answers "can you still grab it", which is a question about window chrome and
/// the same handful of pixels on every monitor — not about how much of the app
/// fits, which `WindowFit` now answers by zooming the page to whatever the frame
/// became.
///
/// ⚠ **Why the app no longer needs a layout floor at all:** the reason one
/// existed — *"a little smaller and it ends up clipping the right panel"* — was
/// a page pinned at `LAYOUT`. It reflows now.
const MIN_FRAME: (u32, u32) = (360, 240);

/// How large that layout is *drawn*, smallest first.
///
/// The page always lays out at [`LAYOUT`] and always shows every panel, because
/// the page is zoomed by the same factor the window is — a preset changes how
/// big the picture is drawn, never how much of it you get.
///
/// **Presets rather than a draggable edge**, because the vendored adapter does
/// not forward `Event::Window(Resized)` — its `on_event` handles keyboard and
/// mouse and nothing else — so a window the host resized would leave the page
/// inside it laid out at the old size. Teaching it to is a change to someone
/// else's crate, and every such change is one more line `VENDORED.md` has to
/// account for on the next rebase.
///
/// ⛔ **Both are at or above 1:1, and the default is the larger of the two sizes
/// that came before.** Mike, 2026-08-06, in three steps as he tried each one:
/// *"ensure that the app gets bigger one time and not smaller then back to
/// bigger"* … *"it needs to start off bigger and get bigger instead of starting
/// off smaller and get a little bigger"* … *"it needs to be the second size for
/// the default and bigger for the bigger size."*
///
/// ⚠ **This reverses the same day's earlier instruction and that is deliberate.**
/// `large` was removed that morning — *"it should only have 2 sizes, a smaller
/// version and a medium/large version"* — because it left dead black space
/// around the UI in Ableton. ▶ **That dead space was a bug, not a property of
/// the size**: the display scale was being read at two different moments (see
/// [`system_scale`]), so the window and the page disagreed. With that fixed the
/// larger sizes are honest, and bigger is the direction he actually wanted.
///
/// ⛔ **1.1 rather than something rounder, and the ceiling is real.** The window
/// is `LAYOUT * system_scale * factor`, so on a 150% display in Ableton `xl` is
/// **2376x1485** against a **2582x1550** work area — it fits with room to spare,
/// where 1.15 would have overrun the height and been clamped to a size nobody
/// picked. [`fit`] would handle that, but a preset that silently is not its own
/// factor is the thing this file has already been confused by once.
///
/// ⚠ **A project saved at `small` or `medium` still opens.** `current_scale`
/// falls back to [`DEFAULT_SCALE`] for any name `preset` does not recognise —
/// which is exactly this case, and the reason that fallback was written rather
/// than an `expect`.
const SCALES: &[(&str, f32)] = &[("large", 1.0), ("xl", 1.1)];

/// What the editor opens at. ⛔ **The *smaller* of the two, so the button's first
/// press grows the window and its second returns it here** — which is the cycle
/// Mike asked for by name. It is nonetheless the *larger* of the pair that
/// shipped before this change: 1:1, where the page is drawn at exactly the size
/// it lays out at and nothing is zoomed at all.
const DEFAULT_SCALE: &str = "large";

/// A named scale: the window in physical pixels, and the zoom the page applies.
///
/// The two must agree. The window is `LAYOUT * system_scale * factor` and the
/// page is zoomed by `factor`, so the CSS viewport works out at `LAYOUT` again
/// — a smaller window showing the same layout, rather than less of it.
fn preset(name: &str) -> Option<((u32, u32), f32)> {
    SCALES
        .iter()
        .find(|(id, _)| *id == name)
        .map(|(_, factor)| physical(*factor))
}

/// The next scale that is actually a *different size*, wrapping, and whether it
/// is smaller.
///
/// Both live here rather than in the frontend so the names exist in exactly one
/// place. A list mirrored in TypeScript would let a rename here leave the button
/// asking for a size the plugin rejects, and the "smaller" flag is what lets the
/// button show the right icon without knowing which name is the smallest.
///
/// ⛔ **Skipping equal sizes is not tidiness.** `SCALES` holds *nominal*
/// factors; `fit` returns clamped ones, and on a screen too small for the larger
/// preset both can collapse onto the same effective size — so the button would
/// change nothing at all on one press, while still flipping its icon as though
/// it had. Comparing what the window will actually be is the only way to know.
fn next_scale(name: &str) -> (&'static str, bool) {
    next_among(name, |factor| physical(factor).0)
}

/// The choice behind [`next_scale`], with the display taken out of it.
///
/// Separated for the same reason [`fit`] is, and the separation was paid for in
/// a red CI run. The tests below called [`next_scale`] and [`physical`], both of
/// which read the real desktop through [`work_area`] — so they asserted a
/// property of whatever machine ran them. They passed on Mike's monitor and
/// **failed on CI's `windows-latest` runner**, whose work area is about
/// 1040x650: every preset clamps to that same window, so the `stay put` branch
/// below fired and the cycle answered `large` where the test wanted `xl`.
///
/// ⚠ **Nothing was wrong with the sizing** — that branch is the documented
/// behaviour and the runner is simply a small screen. Taking the window in as
/// an argument is what lets a test say which screen it means instead of
/// inheriting one. ⛔ The Windows leg was the only one that could ever catch
/// this: [`work_area`] answers `None` off Windows, so the ubuntu and macOS
/// `quality` jobs clamp nothing and were green throughout.
fn next_among(name: &str, window: impl Fn(f32) -> (u32, u32)) -> (&'static str, bool) {
    let at = SCALES.iter().position(|(id, _)| *id == name).unwrap_or(0);
    let here = window(SCALES[at].1);

    for step in 1..=SCALES.len() {
        let (id, factor) = SCALES[(at + step) % SCALES.len()];
        if window(factor) != here {
            return (id, factor < SCALES[at].1);
        }
    }

    // Every preset clamps to the same window — a screen so small that nothing
    // fits. Stay put rather than pretend the button does something.
    (SCALES[at].0, false)
}

/// The window in physical pixels, and the factor the page must actually zoom by.
///
/// ⛔ The size handed to `WebViewEditor` is consumed as *physical* pixels while
/// the page inside is laid out in *CSS* pixels, and on a scaled display those
/// are not the same number. The vendored adapter says as much: its
/// `set_scale_factor` is a stub returning `false` with "TODO: implement for
/// Windows and Linux", so nih-plug's own DPI plumbing never reaches it. The
/// effect on an ordinary 150% laptop was that asking for 1280 gave the UI
/// **853** CSS pixels — below its own minimum, so the layout ran cramped and
/// the right rail auto-collapsed, with nothing anywhere saying why.
///
/// ⛔ **The two are one number, and clamping has to move both.** The page is
/// zoomed by the factor so the layout comes back out at [`LAYOUT`]; clamp the
/// window to the screen without clamping the factor and the CSS viewport is
/// suddenly smaller than the layout, which crops the app and collapses the
/// right rail — the exact failure this design exists to prevent.
///
/// So a screen too small for the asked-for size does not get a clipped window:
/// it gets a *smaller scale*, which still shows everything. A 1366x768 laptop
/// asking for `large` is the case that matters, and it is not an exotic one.
fn physical(factor: f32) -> ((u32, u32), f32) {
    // ⛔⛔ **The scale is applied by `baseview`, not here, and applying it in both
    // places counted it twice** (2026-08-09). `nih_plug_webview` opens the window
    // with `WindowScalePolicy::SystemScaleFactor`, which means the size it is
    // given is **logical** and baseview multiplies by the system DPI itself.
    //
    // ⚠ **This was invisible until the process became DPI-aware.** While it was
    // not, `GetDpiForSystem` answered 96 — so `system_scale()` was 1.0 and
    // multiplying by it changed nothing. `become_dpi_aware` in
    // `bin/standalone.rs` made it answer 144, and the same expression suddenly
    // asked for 1.5² of the layout: the UI overflowed the window, which is the
    // exact mirror of the dead margin it had just fixed. Mike, immediately:
    // *"my size of my gui outshined the window's size."*
    //
    // So the size handed out is `LAYOUT * factor`, in logical pixels, and the
    // one place that knows about DPI is baseview.
    fit(
        LAYOUT,
        system_scale(),
        factor,
        work_area().unwrap_or((u32::MAX, u32::MAX)),
    )
}

/// The arithmetic behind [`physical`], with the platform taken out of it.
///
/// Separated purely so it can be tested. The bug this shape exists to prevent
/// was live and invisible: the window was clamped to the screen while the zoom
/// was not, so a display too small for the asked-for size cropped the app
/// instead of scaling it.
fn fit(
    (w, h): (u32, u32),
    scale: f32,
    factor: f32,
    (max_w, max_h): (u32, u32),
) -> ((u32, u32), f32) {
    let want_w = w as f32 * scale * factor;
    let want_h = h as f32 * scale * factor;

    // How much of what was asked for actually fits. Capped at 1.0 because this
    // may only ever shrink: a small window on a big screen is a choice, and
    // blowing it up to fill the display would be overriding it.
    let fits = (max_w as f32 / want_w)
        .min(max_h as f32 / want_h)
        .clamp(f32::MIN_POSITIVE, 1.0);

    let effective = factor * fits;
    (
        (
            (w as f32 * scale * effective) as u32,
            (h as f32 * scale * effective) as u32,
        ),
        effective,
    )
}

/// Separate from the module's other `tests` below, which cover the bridge and
/// the served assets rather than the geometry.
#[cfg(test)]
mod sizing {
    use super::*;

    const SCREEN: (u32, u32) = (u32::MAX, u32::MAX);

    #[test]
    fn the_window_is_the_layout_times_the_display_scale() {
        // 1440x900 at 150% is 2160x1350 physical, and the page is not zoomed
        // beyond the factor asked for.
        let ((w, h), zoom) = fit(LAYOUT, 1.5, 1.0, SCREEN);
        assert_eq!((w, h), (2160, 1350));
        assert_eq!(zoom, 1.0);
    }

    #[test]
    fn a_smaller_factor_shrinks_the_window_and_the_zoom_together() {
        // The invariant the whole design rests on: window / (scale * zoom) is
        // always the layout, so the page always has 1440x900 to lay out in.
        // ⚠ Driven from `SCALES` rather than a copied list, so a preset added
        // or removed is covered here without anyone remembering to come back.
        for (_, factor) in SCALES {
            let factor = *factor;
            let ((w, _), zoom) = fit(LAYOUT, 1.5, factor, SCREEN);
            let css = w as f32 / (1.5 * zoom);
            assert!(
                (css - 1440.0).abs() < 2.0,
                "factor {factor} gave the page {css} CSS px, not 1440"
            );
        }
    }

    #[test]
    fn a_screen_too_small_shrinks_the_scale_rather_than_cropping_the_app() {
        // ⛔ The bug this test exists for. Clamping the window to the screen
        // without clamping the zoom leaves the page laying out at 1440 inside
        // something narrower — the app is cropped and the right rail vanishes,
        // which is the failure the scales were introduced to avoid.
        //
        // A 1366x768 laptop at 100%, asking for the largest size.
        let ((w, h), zoom) = fit(LAYOUT, 1.0, 1.0, (1366, 728));

        assert!(w <= 1366 && h <= 728, "the window must fit the screen");
        assert!(zoom < 1.0, "the zoom must have come down with it");

        let css_w = w as f32 / zoom;
        let css_h = h as f32 / zoom;
        assert!(
            css_w >= 1439.0 && css_h >= 899.0,
            "the page still needs 1440x900 to lay out in, got {css_w}x{css_h}"
        );
    }

    /// ⛔ Mike, 2026-08-06: *"it needs to be a little bigger than the default
    /// for the first click of the resize button and then back to the default
    /// size."* The default must therefore be the **smaller** of the two.
    #[test]
    fn there_are_two_sizes_and_the_default_is_the_smaller_of_them() {
        assert_eq!(SCALES.len(), 2, "the size button offers exactly two");
        assert_eq!(
            SCALES.first().unwrap().0,
            DEFAULT_SCALE,
            "the editor must open at the smaller one so the button's first press grows it"
        );
        assert!(
            SCALES[0].1 < SCALES[1].1,
            "`SCALES` is smallest-first, and the rest of this module reads it that way"
        );
    }

    #[test]
    fn a_project_saved_at_a_size_that_no_longer_exists_still_opens() {
        // ⛔ `small` and `medium` were both real presets and are both in real
        // project files — they were dropped when the two sizes moved up to 1.0
        // and 1.1. `preset` answering `None` is what `current_scale` falls back
        // on, so this is the assertion that keeps that fallback honest rather
        // than decorative.
        assert!(preset("small").is_none());
        assert!(preset("medium").is_none());
        assert!(preset(DEFAULT_SCALE).is_some());
    }

    /// ⛔ One press bigger, the next press back — never bigger twice. Mike,
    /// 2026-08-06: *"ensure that the app gets bigger one time and not smaller
    /// then back to bigger."*
    /// A display with room for every preset, stated rather than inherited from
    /// whatever machine is running the suite. ⛔ See `next_among`: reading the
    /// real desktop here is what passed on Mike's monitor and failed on CI.
    fn roomy(factor: f32) -> (u32, u32) {
        fit(LAYOUT, 1.0, factor, SCREEN).0
    }

    #[test]
    fn the_button_grows_the_window_then_returns_it_to_the_default() {
        let (next, shrinks) = next_among(DEFAULT_SCALE, roomy);
        assert_eq!(next, "xl");
        assert!(
            !shrinks,
            "the first press must grow the window, not shrink it"
        );

        // ⛔ **Nothing is drawn below 1:1 any more.** Every factor under 1.0 put
        // the page through a zoom round trip, and that is what cost the right
        // rail its one pixel — see `state/ui.ts::isWide`.
        assert!(
            SCALES.iter().all(|(_, factor)| *factor >= 1.0),
            "a preset below 1:1 reintroduces the zoom rounding that hid the Stems panel"
        );

        let (back, shrinks_back) = next_among("xl", roomy);
        assert_eq!(back, DEFAULT_SCALE);
        assert!(
            shrinks_back,
            "the second press must come back down to the default"
        );
    }

    /// ⛔ The third size, and it was never a third *preset*. Mike, 2026-08-06:
    /// *"it only does it once and then back to the default size not twice and
    /// then back to the medium size."*
    #[test]
    fn the_button_offers_exactly_as_many_distinct_windows_as_there_are_presets() {
        // `system_scale` used to answer 1.0 before baseview made the process
        // DPI-aware and 1.5 after, so `medium` was 1224 at open and 1836 at the
        // first press — two windows from one preset. It is pinned now, so a
        // preset resolves to one size however many times it is asked.
        //
        // ⚠ On a screen with room for both. A display too small for either
        // legitimately collapses them onto one window — CI's runner does
        // exactly that — so this claim has to name the screen it holds on.
        let sizes: Vec<(u32, u32)> = SCALES.iter().map(|(_, f)| roomy(*f)).collect();
        let mut distinct = sizes.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            SCALES.len(),
            "two presets must be two windows, got {sizes:?}"
        );

        // And asking twice must answer the same, which is the property that
        // actually broke — the cycle read it once per press. ⚠ This half reads
        // the real display on purpose: it is what pins the `OnceLock`, and it
        // holds on any screen, small or large.
        for (name, _) in SCALES {
            assert_eq!(
                preset(name),
                preset(name),
                "`{name}` must be one window for the whole session"
            );
        }
    }

    /// ⛔ The branch CI found, on a machine rather than in review: a screen too
    /// small for any preset. `windows-latest` reports a work area of about
    /// 1040x650, both presets clamp to exactly that, and the button then has no
    /// second window to offer.
    ///
    /// It must **stay put** rather than flip its icon while changing nothing —
    /// see `next_among`. Untested until the runner exercised it, and the two
    /// tests above could not cover it because they now state a roomy screen.
    #[test]
    fn a_screen_too_small_for_either_preset_leaves_the_button_where_it_is() {
        let cramped = |factor: f32| fit(LAYOUT, 1.0, factor, (1040, 650)).0;

        // The premise: every preset really does collapse onto one window here.
        let sizes: Vec<(u32, u32)> = SCALES.iter().map(|(_, f)| cramped(*f)).collect();
        assert!(
            sizes.windows(2).all(|p| p[0] == p[1]),
            "this test is only meaningful if the presets collapse, got {sizes:?}"
        );

        let (next, shrinks) = next_among(DEFAULT_SCALE, cramped);
        assert_eq!(
            next, DEFAULT_SCALE,
            "with no distinct window to move to, the button must not move"
        );
        assert!(!shrinks, "and it must not claim it is shrinking anything");
    }

    #[test]
    fn a_big_screen_does_not_inflate_a_small_window() {
        // `fits` is capped at 1.0. Asking for small on a 4K display means
        // small, not "as big as the screen allows".
        let ((w, _), zoom) = fit(LAYOUT, 1.0, 0.7, (3840, 2160));
        assert_eq!(zoom, 0.7);
        assert_eq!(w, 1008);
    }
}

/// The name of the size this session is at.
///
/// Saved with the project, so reopening a song gives back the window it was
/// closed at rather than the default. An unrecognised name — a project written
/// by a build whose presets have since been renamed — falls back rather than
/// refusing to open an editor.
fn current_scale(shared: &SharedState) -> String {
    // One field, so `with` rather than a clone of the whole session.
    let saved = state::with(&shared.session, |s| s.window_size.clone()).flatten();
    match saved {
        Some(name) if preset(&name).is_some() => name,
        _ => DEFAULT_SCALE.to_owned(),
    }
}

/// The size the editor opens at.
///
/// `current_scale` only ever returns a name `SCALES` contains, so `preset`
/// cannot fail here — hence `expect` rather than a fallback that could only
/// ever return a *logical* size from a function that promises physical pixels.
fn window_size(shared: &SharedState) -> (u32, u32) {
    let (size, _) = preset(&current_scale(shared)).expect("current_scale returns a known scale");
    size
}

/// Commands the *window* owns, rather than the engine.
///
/// [`bridge::dispatch`] answers for the dataset and the generator and knows
/// nothing about a window — it cannot, because the window exists only inside
/// the frame loop. Handled before dispatch so that the bridge's "an unknown
/// command fails loudly" rule keeps meaning what it says: this is a *known*
/// command that simply lives on the other side of the seam.
///
/// The `lane` argument of a one-shot command.
///
/// ⛔ **Refused rather than defaulted.** These arrive from the webview, so a
/// lane the engine does not have means the page and the plugin disagree — and
/// defaulting to `Kick` would silently assign somebody's sample to the wrong
/// pad, which is far harder to notice than an error.
fn lane_arg(request: &Request) -> Result<engine::pattern::Lane, String> {
    serde_json::from_value(request.args["lane"].clone())
        .map_err(|_| format!("{} is not a lane", request.args["lane"]))
}

/// Returns `None` when the command is not one of these, so the caller falls
/// through to the bridge.
fn window_command(request: &Request, shared: &SharedState) -> Option<Result<Value, String>> {
    // `editor_size` reads, `set_editor_size` writes, and both answer with the
    // same thing — so the reply is built once. Asked on mount because the
    // window already opens at the saved size but the *page* has no way to know
    // what zoom that implies, and at 1:1 inside a scaled window it is cropped.
    // The transport (TASK-041T). Handled here rather than in `bridge::dispatch`
    // for the reason every command in this function is: they need `shared`, and
    // the playhead lives there because the audio thread publishes it and the
    // editor may not take a lock to read it.
    match request.command.as_str() {
        // Polled by the editor at frame rate, so it stays a bare number rather
        // than a struct — this is the hottest command in the bridge.
        "playhead" => return Some(Ok(json!(shared.playhead()))),

        // Click anywhere on the timeline. The audio thread picks it up on its
        // next block and rewinds its own cursor to match.
        "seek" => {
            let to = request.args["progress"].as_f64().unwrap_or(0.0) as f32;
            shared.request_seek(to);
            return Some(Ok(Value::Null));
        }

        // The piano roll's keyboard gutter (TASK-041). A window command rather
        // than a bridge one for the same reason `seek` is: it needs the
        // `Shared` the audio thread reads, which `bridge::dispatch` has no
        // access to.
        //
        // ⛔ Answers `Ok` even when nothing will sound — the preview switched
        // off, no kit, no tuned pad. An audition is feedback on a click that has
        // already landed, so there is nothing for the page to do with a failure
        // (see `audition.ts`), and reporting one would put an error banner under
        // the Generate button for a preview a producer deliberately turned off.
        // ── The sample explorer (TASK-132) ──────────────────────────────
        //
        // ⚠ Three commands rather than one, because they have three different
        // costs: picking opens a modal on its own thread, opening is a cheap
        // write, and reading is a filesystem walk the page asks for when it
        // wants to redraw. Folding them together would make the page pay for a
        // dialog every time it refreshed a list.
        // ── The audition player (TASK-132) ──────────────────────────────
        //
        // ⛔ Six of Mike's eight preview items are ONE number — the read
        // position. The playhead marker, the progress fill, the time readout,
        // click-to-seek, reverse and loop all resolve to it, which is why
        // `preview_position` is a poll rather than six channels.
        "preview_load" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            let file = std::path::Path::new(path);
            // ⛔⛔ **The same two guards `waveform` has, and they must come
            // before `is_file()`.** Without them this took a raw webview path
            // and touched it: `is_file()` on a UNC string makes the SMB
            // redirector resolve the host, connect out and authenticate, which
            // `oneshot::refuse_remote` documents as a real vulnerability rather
            // than hardening. And with no containment it would decode *any*
            // audio file on disk, up to the 128 MB import bound.
            if let Err(reason) = crate::oneshot::refuse_remote(file) {
                return Some(Err(reason));
            }
            if !shared.explorer.contains(file) {
                return Some(Err("that sample is not in your sample library".into()));
            }
            if !file.is_file() {
                return Some(Err("that sample is not there".into()));
            }
            return Some(
                match crate::audio::import::decode_file(std::path::Path::new(path)) {
                    Ok(audio) => {
                        shared.preview.load(audio.samples, audio.sample_rate);
                        // ⛔ **Recorded here, because this is the moment a
                        // producer actually looked at a file.** `select`
                        // auditions on a click, an arrow key and a starred row
                        // alike, and all three arrive as `preview_load` — so one
                        // call covers every way into the browser. Recording on
                        // *drop* instead would miss everything auditioned and
                        // rejected, which is most of what browsing is.
                        //
                        // ⚠ **A failure here must not fail the audition.** The
                        // sample has already been decoded and handed to the
                        // player; a producer whose `%APPDATA%` is read-only
                        // should still hear it. The history is a convenience and
                        // refusing the preview to protect the bookkeeping would
                        // be the wrong trade.
                        let _ = crate::recent::note(path);
                        Ok(Value::Null)
                    }
                    Err(reason) => Err(reason),
                },
            );
        }

        "preview_play" => {
            shared.preview.play();
            return Some(Ok(Value::Null));
        }

        "preview_pause" => {
            shared.preview.pause();
            return Some(Ok(Value::Null));
        }

        // ⛔ Rewinds. Pause holds position; stop does not.
        "preview_stop" => {
            shared.preview.stop();
            return Some(Ok(Value::Null));
        }

        "preview_seek" => {
            let at = request.args["seconds"].as_f64().unwrap_or(0.0) as f32;
            shared.preview.seek(at);
            return Some(Ok(Value::Null));
        }

        "preview_loop" => {
            shared
                .preview
                .set_looping(request.args["on"].as_bool().unwrap_or(false));
            return Some(Ok(Value::Null));
        }

        "preview_reverse" => {
            shared
                .preview
                .set_reverse(request.args["on"].as_bool().unwrap_or(false));
            return Some(Ok(Value::Null));
        }

        // Polled while a sample is auditioning — the one number everything
        // else is drawn from. `collect` rides along because this is the
        // editor thread and it is what frees a buffer the callback parked.
        "preview_position" => {
            shared.preview.collect();
            return Some(
                serde_json::to_value(shared.preview.position()).map_err(|e| e.to_string()),
            );
        }

        "explorer_pick" => return Some(shared.explorer.pick().map(|()| Value::Null)),

        "explorer_remove" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            shared.explorer.remove(path);
            shared.store_sample_folders();
            return Some(Ok(Value::Null));
        }

        // The waveform the preview player draws (TASK-132).
        //
        // ⚠ Peaks, never the audio: a four-second sample is megabytes as JSON,
        // serialized on the editor thread inside somebody's DAW.
        "explorer_waveform" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            return Some(
                crate::explorer::waveform(&shared.explorer, path)
                    .and_then(|w| serde_json::to_value(w).map_err(|e| e.to_string())),
            );
        }

        // ⛔ **A `.mid` the producer already has, as a trainable pattern**
        // (TASK-040T). Mike: *"you should be able to drag in MIDI from the file
        // explorer to train your original artist/workflow."* It answers a
        // `Pattern` and nothing else, which is the requirement rather than a
        // convenience: the fit reads `Pattern`, so a model trained from files
        // cannot drift from one trained from generations.
        "explorer_midi" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            let part = match serde_json::from_value(request.args["part"].clone()) {
                Ok(part) => part,
                Err(_) => return Some(Err("that is not a part".to_owned())),
            };
            return Some(
                crate::explorer::midi_pattern(&shared.explorer, path, part)
                    .inspect(|_| {
                        // ⚠ The MIDI half of the same moment `preview_load`
                        // records. A `.mid` is never decoded by the audio
                        // player, so without this the history would show only
                        // the samples a producer auditioned and none of the
                        // loops they opened.
                        let _ = crate::recent::note(path);
                    })
                    .and_then(|p| serde_json::to_value(p).map_err(|e| e.to_string())),
            );
        }

        // ⛔⛔ **The sample-copy pair, and the paths come from the PLUGIN.**
        // These two shipped in `bridge.rs` for an afternoon taking a page-supplied
        // list of arbitrary filesystem paths, with `refuse_remote` applied and
        // **containment not** — which a security review found. That is a clean
        // per-path existence-and-exact-size oracle over the whole disk handed
        // straight back to an untrusted page, and then an arbitrary local file
        // read into a known folder. It is the same defect `explorer_drop` was
        // found with, arriving through two more doors.
        //
        // ▶ **The fix is also the simpler design**: the plugin already holds the
        // assignments, and the page was fetching them from `kit_state` only to
        // hand them straight back. Sourced here, there is no path to validate
        // because none crosses the boundary — the same rule `kits_save` states
        // below.
        //
        // ⚠ The two stay **separate commands**, because that split is the
        // consent: asking what a copy would cost cannot copy anything, and
        // `user_model_save` calls neither.
        "user_model_sample_cost" => {
            return Some(
                serde_json::to_value(crate::models::sample_cost(&assigned_paths(shared)))
                    .map_err(|e| e.to_string()),
            );
        }

        "user_model_copy_samples" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            return Some(
                crate::models::copy_samples(id, &assigned_pairs(shared))
                    .and_then(|landed| serde_json::to_value(landed).map_err(|e| e.to_string())),
            );
        }

        // ⛔⛔ **The other half of the copy, and without it the consent text was
        // false.** It told the producer their samples *"still work if you move
        // or delete the originals"*; nothing read `models/<id>/samples/` back,
        // so the copies were bytes on a drive that no code path could reach.
        // Mike found it by hand: clear the kick on a saved style, select
        // something else, come back, and the kick did not return.
        //
        // ⛔ **Through `load_kit`, so it is on the loader thread.** This decodes
        // up to a dozen files; doing that here would be doing it on the host's
        // editor thread, which is § 4.8's freeze — the failure this session has
        // already had to fix once, in `user_model_train`. The page waits on
        // `one_shot_status` exactly as it does for a folder re-roll.
        //
        // ⚠ **Silent when the style owns nothing**, which is most styles. A
        // producer selecting a style they never copied samples for must not be
        // told about a file they never made — so an empty list is `Ok(())` here
        // rather than `load_kit`'s "that kit has no samples in it".
        "user_model_load_samples" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            let pairs = crate::models::samples_for(id);
            if pairs.is_empty() {
                return Some(Ok(Value::Bool(false)));
            }
            return Some(
                shared
                    .one_shots
                    .load_kit(pairs, &shared.kits, &shared.session)
                    .map(|()| Value::Bool(true)),
            );
        }

        // Named kits (TASK-051). ⛔ **`kits_save` reads the assignments from the
        // plugin rather than taking them from the page**, because the plugin is
        // what holds them — `OneShots::snapshot` is the truth, and a page that
        // sent its own idea of the kit could save one that never played.
        "kits_list" => {
            return Some(serde_json::to_value(crate::kits::list()).map_err(|e| e.to_string()))
        }

        "kits_save" => {
            let name = request.args["name"].as_str().unwrap_or_default();
            let lanes = shared
                .one_shots
                .snapshot()
                .into_iter()
                .map(|(lane, (path, _))| (lane, path))
                .collect();
            return Some(
                crate::kits::save(name, lanes)
                    .and_then(|k| serde_json::to_value(k).map_err(|e| e.to_string())),
            );
        }

        "kits_load" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            return Some(crate::kits::load(id).and_then(|pairs| {
                shared
                    .one_shots
                    .load_kit(pairs, &shared.kits, &shared.session)
                    .map(|()| Value::Null)
            }));
        }

        "kits_rename" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            let name = request.args["name"].as_str().unwrap_or_default();
            return Some(
                crate::kits::rename(id, name)
                    .and_then(|k| serde_json::to_value(k).map_err(|e| e.to_string())),
            );
        }

        "kits_duplicate" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            let name = request.args["name"].as_str().unwrap_or_default();
            return Some(
                crate::kits::duplicate(id, name)
                    .and_then(|k| serde_json::to_value(k).map_err(|e| e.to_string())),
            );
        }

        "kits_delete" => {
            let id = request.args["id"].as_str().unwrap_or_default();
            return Some(crate::kits::delete(id).map(|()| Value::Null));
        }

        // ⛔ **Re-roll pads from the folder being browsed** (TASK-050A). The
        // page sends the lanes, because it is what knows which pads are locked —
        // TASK-044's rule applied to pads, kept in one place rather than
        // mirrored here where the two could disagree. It sends the seed too, for
        // the same reason `variations.ts` sends a timestamp: nothing below the
        // page may read a clock.
        "kit_randomize" => {
            let lanes: Vec<engine::pattern::Lane> =
                match serde_json::from_value(request.args["lanes"].clone()) {
                    Ok(lanes) => lanes,
                    Err(_) => return Some(Err("those are not lanes".to_owned())),
                };
            let seed = request.args["seed"]
                .as_str()
                .and_then(|text| text.parse::<u64>().ok())
                .unwrap_or(0);

            // ⛔⛔ **THE FOLDER THE PRODUCER IS STANDING IN, WHICH THE PAGE HAS
            // TO SAY** — Mike, 2026-08-11: *"if i am on an actual sample in the
            // file explorer, or i am on a folder in file explorer, either way, it
            // should remember the folder's name that i am in, and it should
            // randomize a sample from that specific folder for the 'Re-roll'."*
            //
            // ▶ **`state()` could not answer that any more.** It reports the
            // explorer's own "current folder", which the *tree* view stopped
            // maintaining — expanding a branch and clicking a file inside it
            // never moves it. So the dice was re-rolling from whatever folder was
            // last opened the old way, or from nothing at all.
            //
            // ⚠ **Which folder a *file* means is the page's question**, because
            // the page is what holds the selection: it sends the parent for a
            // file and the folder itself for a folder. Deriving it here would be
            // a second idea of where the producer is standing.
            //
            // ⛔ **`list_one` re-applies containment**, so an arbitrary path from
            // the webview cannot read outside the sample library — the same
            // boundary `explorer_drop` leans on, and the reason this is not just
            // a `read_dir`. Falling back to `state()` keeps the old behaviour for
            // a page that sends nothing.
            let files: Vec<String> = match request.args["folder"].as_str() {
                Some(dir) if !dir.is_empty() => shared
                    .explorer
                    .list_one(dir)
                    .map(|state| state.entries)
                    .unwrap_or_default(),
                _ => shared.explorer.state().entries,
            }
            .into_iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.path)
            .collect();

            return Some(
                shared
                    .one_shots
                    .randomize(lanes, files, seed, &shared.kits, &shared.session)
                    .map(|()| Value::Null),
            );
        }

        "explorer_open" => {
            let dir = request.args["path"].as_str().unwrap_or_default();
            return Some(shared.explorer.open(dir).map(|()| Value::Null));
        }

        // One folder's rows, for a node the producer expanded in the tree.
        //
        // ⚠ **Does not move the browse location**, which is the whole difference
        // from `explorer_open` — see `Explorer::list_one`. A tree has several
        // folders open and only one of them is the folder being worked from.
        "explorer_list" => {
            let dir = request.args["path"].as_str().unwrap_or_default();
            return Some(
                shared
                    .explorer
                    .list_one(dir)
                    .and_then(|state| serde_json::to_value(state).map_err(|e| e.to_string())),
            );
        }

        // Separating a layered `.mid` into the parts its voices belong to.
        //
        // ⛔ Mike, 2026-08-10: *"split it into the proper generators if it is a
        // layered melody file with the bass and countermelody included."* Each
        // result carries **why** it was routed where it was — `engine::smf_read`
        // states the rule: never present a guess as a transcription.
        "explorer_midi_split" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            return Some(
                crate::explorer::midi_split(&shared.explorer, path)
                    .and_then(|parts| serde_json::to_value(parts).map_err(|e| e.to_string())),
            );
        }

        // A whole `.mid` as an arrangement, for the Song tab (TASK-058D).
        //
        // ⛔ Mike, 2026-08-10 — the file lands in the Song tab and the producer
        // drills out the parts they want, rather than one drop overwriting a
        // generator. ⚠ **The drag-in path only**; generating a song is untouched.
        "explorer_song" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            return Some(
                crate::explorer::midi_song(&shared.explorer, path)
                    .and_then(|song| serde_json::to_value(song).map_err(|e| e.to_string())),
            );
        }

        // Starred samples, one-shots and MIDI files (TASK-058C).
        "favourites_list" => {
            return Some(
                serde_json::to_value(crate::favourites::list()).map_err(|e| e.to_string()),
            );
        }

        // ── The browser's history (TASK-058) ─────────────────────────────
        //
        // ⛔ Read-only from the page. Nothing adds to the history over the
        // bridge: entries only ever come from `preview_load` and `explorer_midi`
        // above, which are already bounded by `Explorer::contains`. A
        // `recent_add` would be a way for the page to name an arbitrary path and
        // have it stored and shown as somewhere the producer had been.
        "recent_list" => {
            return Some(serde_json::to_value(crate::recent::list()).map_err(|e| e.to_string()));
        }

        "recent_clear" => {
            return Some(
                crate::recent::clear()
                    .and_then(|list| serde_json::to_value(list).map_err(|e| e.to_string())),
            );
        }

        // ⛔⛔ **Containment is applied HERE, where the explorer is in scope.**
        // `favourites::add` cannot take it — it has no reference to the library —
        // so a star is only allowed on a file the browser would actually list.
        // Without this the page could star any local path and then use `reveal`
        // to launch a shell at it, which is the one command in this plugin that
        // starts a process.
        "favourites_add" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            if !shared.explorer.contains(std::path::Path::new(path)) {
                return Some(Err("that file is not in your sample library".into()));
            }
            return Some(
                crate::favourites::add(path)
                    .and_then(|list| serde_json::to_value(list).map_err(|e| e.to_string())),
            );
        }

        // ⚠ **No containment on the way out.** A folder removed from the library
        // must still be un-starrable, or a favourite could become permanent by
        // the producer tidying their roots.
        "favourites_remove" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            return Some(
                crate::favourites::remove(path)
                    .and_then(|list| serde_json::to_value(list).map_err(|e| e.to_string())),
            );
        }

        // ⛔ Mike, 2026-08-10: *"if it's not a folder that you still have in the
        // 'File Explorer' then it should take you there in Windows Explorer or
        // the macOS Explorer."* `favourites::reveal` refuses anything that is not
        // already starred, which is what bounds the process launch.
        "favourites_reveal" => {
            let path = request.args["path"].as_str().unwrap_or_default();
            return Some(crate::favourites::reveal(path).map(|()| Value::Null));
        }

        "explorer_state" => {
            // ⚠ Written back on every read, because the only thing that knows
            // a dialog finished is the dialog thread — and it cannot touch the
            // session. The page polls this after asking for a folder, so this
            // is where a newly added root becomes something the host will save.
            shared.store_sample_folders();
            return Some(serde_json::to_value(shared.explorer.state()).map_err(|e| e.to_string()));
        }

        // Dropping a sample from the explorer onto a lane.
        //
        // ⛔ **Routed through `restore`, which is the no-dialog load a reopened
        // project already uses.** It decodes, refuses a remote path, records a
        // file that will not load, and rebuilds the kit — every one of which a
        // second loader here would have to repeat and would eventually get
        // wrong. This command is a name for an existing path, not a new one.
        "explorer_drop" => {
            let Some(lane) = request.args.get("lane").and_then(|lane| {
                serde_json::from_value::<engine::pattern::Lane>(lane.clone()).ok()
            }) else {
                return Some(Err("that is not a lane".into()));
            };
            let path = request.args["path"].as_str().unwrap_or_default();
            // ⛔⛔ **Contained, like its two siblings.** `restore` refuses a
            // remote path — that is the SMB-authentication guard — but it does
            // *not* ask whether the file is in the producer's library, because
            // its other caller is a reopened project restoring a one-shot the
            // producer picked through a native dialog from anywhere on disk.
            // `preview_load` and `explorer_waveform` both apply this check and
            // this arm did not, so of the three commands that take a raw path
            // from the webview, one would decode any local file it was named —
            // and `decode_file`'s error strings tell missing from unreadable,
            // which is the filesystem oracle `explorer::waveform`'s own comment
            // exists to close.
            //
            // ⚠ Nothing legitimate is refused by this: the only thing that
            // sends `explorer_drop` is a row the explorer listed, and the
            // explorer only lists what is inside a saved root. Assigning a
            // sample from outside the library is `one_shot_assign`'s job, which
            // opens a dialog and is therefore the producer choosing rather than
            // the page naming.
            let file = std::path::Path::new(path);
            if !shared.explorer.contains(file) {
                return Some(Err("that sample is not in your sample library".into()));
            }
            // ⛔ **Backwards when the page says so** — Mike, 2026-08-11: *"'Ctrl +
            // left arrow' … should add the sample to that selected drum pad lane
            // in reverse and 'Ctrl + right arrow' should add the sample to that
            // selected drum pad lane playing regularly."*
            //
            // ⚠ **Absent means forwards**, so the drag-onto-a-pad and
            // click-to-assign routes — which send no such flag and have no
            // reverse gesture — keep behaving exactly as they did.
            let reversed = request.args["reversed"].as_bool().unwrap_or(false);
            return Some(
                shared
                    .one_shots
                    .restore(lane, path, reversed, &shared.kits, &shared.session)
                    .map(|()| Value::Null),
            );
        }

        "audition_note" => {
            // Clamped once, in `request_audition`, which is where the test that
            // proves it lives. A second clamp here would be a second authority
            // on the same rule.
            let pitch = request.args["pitch"].as_u64().unwrap_or(60);
            shared.request_audition(pitch.try_into().unwrap_or(127));
            return Some(Ok(Value::Null));
        }

        // Hear one drum lane on its own (TASK-043) — "which pad am I about to
        // edit", answered without soloing and pressing play.
        //
        // ⛔ **The lane is resolved to its GM note *here*, on the editor
        // thread, and never on the frontend.** `gm_drum_note` is the engine's,
        // and a JavaScript copy of it would be a second authority on which pad
        // is the rim — the drift class this project has been bitten by. An
        // unknown lane name is refused rather than defaulted, because guessing
        // would audition a drum the producer did not click.
        "audition_lane" => {
            let Some(name) = request.args["lane"].as_str() else {
                return Some(Err("audition_lane needs a lane".into()));
            };
            let Ok(lane) = serde_json::from_value::<engine::pattern::Lane>(json!(name)) else {
                return Some(Err(format!("`{name}` is not a lane")));
            };
            shared.request_lane_audition(lane);
            return Some(Ok(Value::Null));
        }

        // ⛔ Stop is a seek to zero **and** a hold, in both shells since
        // TASK-138. It used to be a seek alone in a host, because there was no
        // transport of ours to hold there; the preview transport is ours to
        // hold everywhere, and without this Stop would rewind the marker and
        // leave the preview playing on from the top (TASK-041T).
        //
        // ⚠ **This still cannot stop a DAW, and does not try.** `lib.rs` gates
        // on `host_playing || preview`: taking the preview down leaves the
        // host's own playback exactly as it was, which is the separation the
        // old standalone-only behaviour was protecting.
        // ⛔ `stop_playback`, not `transport_stop`. The name is the one the
        // frontend already invokes and the one the mock answers — and a bridge
        // that answers a *different* name fails in the quietest possible way:
        // the unknown command rejects, `stop()` swallows it, the marker snaps
        // to zero locally, and the audio thread never rewinds. The beat keeps
        // playing from the middle of the pattern while the playhead says it
        // stopped.
        "stop_playback" => {
            shared.request_seek(0.0);
            shared.set_running(false);
            return Some(Ok(Value::Null));
        }

        // Who owns the transport, and why it cannot be driven from the page if
        // it cannot (TASK-041T).
        //
        // ⛔ **Both in one reply, because they are one fact.** They were briefly
        // two commands — this and a `shell_info` — computed from the same source
        // eight lines apart in `bridge.rs`. Two answers to one question is two
        // answers that can drift, and the page recombined them into a single
        // decision anyway: an enabled Play button whose tooltip said to press
        // play in your DAW was one dropped reply away.
        //
        // ⛔ **Here rather than in `bridge::dispatch` for the reason every
        // command in this function is here: it needs `shared`.** Reading a
        // process-wide flag instead is what made it untestable — one global
        // cannot be both a host and a standalone in the same test binary.
        //
        // `reason` is a *string for a human*, never a decision on its own. In a
        // host it says who owns the transport; in the standalone nothing is
        // wrong, so it is null — and if that ever stops being true (no output
        // device, a kit that failed to decode) it can say so without the page
        // mistaking it for "this is a plugin".
        // ⛔⛔ **`reason` is null in a host too now (TASK-138).** It used to read
        // *"Press play in your DAW — the plugin puts the notes on the track"*,
        // and that string was the whole reason Play was disabled there. The
        // plugin drives its own preview transport now, so nothing is wrong in a
        // host and there is nothing to explain.
        //
        // ⚠ **The field stays**, and so does `standalone`. `reason` was always a
        // string for a human rather than a decision, and it still has real work:
        // no output device, or a kit that failed to decode, are refusals the page
        // must be able to show without mistaking them for "this is a plugin".
        "playback_status" => {
            return Some(Ok(json!({
                "standalone": shared.standalone,
                "reason": Value::Null,
                // ⛔ **The page had no way to learn this and defaulted to `true`.**
                // `Shared` outlives the webview, so turning Loop off and then
                // closing and reopening the plugin window in a host left the
                // button lit and `aria-pressed="true"` while the schedule was
                // not looping — a control reporting the opposite of the truth.
                //
                // ⚠ One command carrying all three, which is the argument
                // `standalone`/`reason` already make for each other: they are
                // one fact about the transport, asked once on mount.
                "looping": shared.looping(),
            })));
        }

        // ⛔⛔ **Answered in a host now, and the refusal that stood here is gone
        // (TASK-138).** It read *"the host owns the transport — press play in
        // your DAW"*, which was right about the DAW's timeline and wrong about
        // auditioning. Mike, 2026-08-04: *"i do not want to just use Ableton's
        // transpose play button."*
        //
        // ▶ This drives a **preview** transport that is explicitly not the
        // host's: `lib.rs` gates on `host_playing || preview` and drops the
        // preview the moment the host's transport starts, so the two can never
        // both drive the schedule. `Shared::set_running` carries the full
        // reasoning and the reason the old gate does not apply.
        "transport_play" | "transport_pause" => {
            shared.set_running(request.command == "transport_play");
            return Some(Ok(Value::Null));
        }

        // Whether the clip repeats at its end (TASK-138).
        //
        // ⛔ **Answered in a host too, unlike `transport_play` above, and the
        // difference is the point.** Play is a claim on the *host's* timeline
        // and is refused there. Looping is a property of our own schedule over
        // our own clip — a DAW that is rolling still expects a plugin's loop to
        // turn over — so this is ours to answer wherever it is asked. Mike,
        // 2026-08-06: *"can you have the 'Loop' button toggle off and on."*
        "transport_loop" => {
            let on = request.args["on"].as_bool().unwrap_or(true);
            shared.set_looping(on);
            return Some(Ok(Value::Null));
        }

        // ---- The KIT panel and one-shot assignment (TASK-131B, TASK-136) ----
        //
        // ⛔ **Here rather than in `bridge::dispatch` for the reason every
        // command in this function is: they need `shared`.** The kit lives
        // behind a handoff the audio thread reads, and the assignment map is
        // per instance — neither is reachable from the bridge.

        // What the KIT panel draws: every lane the engine has, what plays it,
        // and whether that is the producer's own sample.
        //
        // ⛔ **Built from the kit and the assignment map, never from a table
        // written here.** `RightRail` used to render eight hardcoded disabled
        // buttons and a static "No kit yet" while a twelve-pad kit was loaded
        // and audibly playing (TASK-136) — a readout that lies, and it lied
        // because it was connected to nothing. A second list of lanes in this
        // file would be the same defect with an extra step.
        "kit_state" => {
            let assigned = shared.one_shots.snapshot();
            // ⚠ The **model's** kit, so the panel names the voices the producer
            // is actually hearing. Reading the trap kit here would put this
            // panel back to describing a kit that is not playing, which is the
            // exact defect TASK-136 fixed one layer down.
            let model_id = crate::state::with(&shared.session, |s| s.selected_id.clone())
                .flatten()
                .unwrap_or_default();
            let base = crate::audio::kit_for_model(&model_id);
            let lanes: Vec<Value> = crate::shared::ALL_LANES
                .iter()
                .map(|lane| {
                    let one_shot = assigned.get(lane);
                    json!({
                        "lane": lane,
                        // ⚠ A lane with neither is silent, and the panel has to
                        // say so. `Lane::Snap` is that lane today: the drum
                        // generator can write it and the shipped kit has never
                        // carried a pad for it.
                        "shipped": base.is_some_and(|kit| kit.pad_for(*lane).is_some()),
                        "name": one_shot.map(|(_, name)| name.clone()),
                        "path": one_shot.map(|(path, _)| path.clone()),
                    })
                })
                .collect();
            return Some(Ok(json!({
                "id": base.map(|kit| kit.id.clone()),
                "lanes": lanes,
            })));
        }

        // Open a dialog and put what is picked on a lane. Returns immediately;
        // the outcome arrives through `one_shot_status`. See `crate::oneshot`
        // for why it cannot block here.
        "one_shot_assign" => {
            return Some(
                lane_arg(request)
                    .and_then(|lane| shared.assign_one_shot(lane).map(|()| Value::Null)),
            );
        }

        "one_shot_clear" => {
            return Some(lane_arg(request).map(|lane| {
                shared.clear_one_shot(lane);
                Value::Null
            }));
        }

        "export_pattern_stems" => {
            return Some((|| -> Result<Value, String> {
                let patterns: Vec<Pattern> =
                    Vec::<engine::pattern::Pattern>::deserialize(&request.args["patterns"])
                        .map_err(|e| format!("bad patterns: {e}"))?;
                crate::bridge::check_patterns(&patterns)?;
                let audio = request.args["audio"].as_bool().unwrap_or(false);
                let lanes = request.args["lanes"].as_bool().unwrap_or(false);
                let first = &patterns[0];
                let folder = format!(
                    "{}-{}-{}",
                    first.artist_id,
                    first.seed,
                    if audio { "audio" } else { "midi" }
                );
                shared
                    .exports
                    .start_pattern_stems(patterns, &folder, audio, lanes, shared.current_kit())
                    .map(|()| Value::Null)
            })());
        }

        "one_shot_status" => {
            return Some(
                serde_json::to_value(shared.one_shots.take_status()).map_err(|e| e.to_string()),
            );
        }

        // ---- Dragging a part out into the DAW (TASK-063C / FMM-S03) --------
        //
        // ⛔ **Three commands, not one, and `crate::drag`'s header explains the
        // split in full.** Rendering eight lanes of audio on the frame the page
        // is waiting on is exactly the stall `export.rs` exists to avoid, so
        // `drag_prepare` returns immediately, the page polls `drag_status`, and
        // only `drag_start` — cheap once the bytes are on disk — enters the
        // platform's modal drag loop.

        // Whether this build, in this shell, can start an OS file drag.
        //
        // ⛔ **Here rather than on `app_info`, because it needs `shared`.** The
        // answer is not just "which OS" — the standalone pumps its own message
        // queue, so a drag there re-enters baseview's window procedure and
        // aborts the process. `crate::drag::supported_in` is where that rule
        // lives and this is the only thing that asks it.
        "drag_supported" => {
            return Some(Ok(
                json!({ "supported": crate::drag::supported_in(shared.standalone) }),
            ));
        }

        // Render and spool what is about to be dragged.
        //
        // ⚠ **Checked exactly as the equivalent export is**, and with the same
        // function: these arrive as JSON from the webview through the same
        // door, and this path renders audio from their bar count and meter just
        // as that one does.
        "drag_prepare" => {
            return Some((|| -> Result<Value, String> {
                let args = DragArgs::deserialize(&request.args)
                    .map_err(|e| format!("bad drag request: {e}"))?;
                // ⛔ The kit that is PLAYING, exactly as the export resolves it:
                // a producer who assigned their own snare must drag out the one
                // they heard, not the shipped one.
                let kit = if args.audio {
                    let kit = shared.current_kit();
                    if kit.is_none() {
                        return Err(
                            "the preview kit did not load, so there is no audio to drag".to_owned()
                        );
                    }
                    kit
                } else {
                    None
                };

                // ⚠ **A song travels as a song.** Flattening it is what makes
                // every part the whole timeline with one part in it — the
                // property that lets a producer drop all of them at bar 1 and
                // get the arrangement back — and it happens on the spooling
                // thread rather than here. See `drag::Subject`: doing it here
                // walked the whole record five times on the thread the DAW
                // draws its editor from, for every press of the chip.
                let subject = match args.song {
                    Some(song) => {
                        // ⛔⛔ **The refusal that stood here is gone, and what
                        // replaced it is the thing it asked for.** It read: *"a
                        // whole arrangement drags out as MIDI — render audio
                        // stems from Export instead"*, because "a song is
                        // minutes long, and rendering one to audio needs
                        // progress a producer can watch and a cancel they can
                        // press". Both now exist — `drag::Progress` publishes
                        // how far the render has got and stops it the moment the
                        // slot is disowned — so the request is answered rather
                        // than turned away. Mike asked for exactly this on
                        // 2026-08-06: *"we need to do progress-with-cancel so
                        // that way we can drag the entire song arrangement to
                        // the DAW all at once."*
                        // ⛔ **`check_song`, NOT `check_patterns`.** A flattened
                        // arrangement is as long as the arrangement, and
                        // `MAX_BARS` bounds the four- or eight-bar loop on
                        // screen — running it over a song would refuse any
                        // record past 128 bars, which is most of them.
                        crate::bridge::check_song_for_export(&song)?;
                        crate::drag::Subject::Song(song)
                    }
                    None => {
                        crate::bridge::check_patterns(&args.patterns)?;
                        crate::drag::Subject::Patterns {
                            patterns: args.patterns,
                            // ⚠ **The plugin cuts, not the page.** The page used
                            // to slice a lane out of the pattern itself and ask
                            // for "split by lane", which put "what a lane stem
                            // is" on both sides of the bridge.
                            // ⛔ **"All Tracks" spools the *sequential* layout,
                            // and the stacked one rides along as the Ctrl
                            // alternative** (Mike, 2026-08-06: *"it has to be
                            // separate midi clips one after the other, but on
                            // the same line unless you hold ctrl … then it
                            // stacks them"*). `render_and_spool` builds both
                            // from this one cut; `drag/windows.rs` picks
                            // between them from inside the drag.
                            // ⛔⛔ **MORE THAN ONE PART LANDS TOGETHER**
                            // (Mike, 2026-08-11): *"all parts of a song should be
                            // able to press 'Ctrl+Drag in' to put them on
                            // separate lanes, but together if you don't press
                            // ctrl"* … *"for audio and midi."* `Cut::Parts` — one
                            // file each, and therefore one DAW lane each — is now
                            // what the modifier reaches; `drag::render_and_spool`
                            // spools both.
                            //
                            // ⚠ **One pattern is `Parts`, not `Together`.** They
                            // would produce the same single file, but `Together`
                            // names it *All Parts*, which on a lone drum loop is
                            // a label that lies about what is in it. A single
                            // part also has nothing for Ctrl to choose between.
                            // ⚠ **How many patterns there are does not enter
                            // into it**, and it used to: a `several` guard
                            // selected between two arms that had become the same
                            // answer, which rustc cannot warn about through a
                            // guard. See `drag::render_and_spool` — a multi-part
                            // drag is plain `Cut::Parts`, one clip per part at
                            // bar 1, because anything offset arrives as a
                            // staircase of over-long clips.
                            cut: match (args.lane, args.lanes) {
                                (Some(lane), _) => crate::export::Cut::OneLane(lane),
                                (None, true) => crate::export::Cut::EveryLaneInSequence,
                                (None, false) => crate::export::Cut::Parts,
                            },
                        }
                    }
                };
                shared
                    .drags
                    .prepare(subject, kit, shared.standalone)
                    .map(|()| Value::Null)
            })());
        }

        "drag_status" => {
            return Some(serde_json::to_value(shared.drags.status()).map_err(|e| e.to_string()));
        }

        // ⛔ **This one blocks for the whole gesture**, and that is not a bug to
        // fix later: `DoDragDrop` owns a modal loop and has to run on the thread
        // the drag started from, which is the thread answering this call. The
        // page's `fetch` stays outstanding until the producer lets go, which is
        // correct — there is nothing for it to do in the meantime.
        "drag_start" => {
            return Some((|| -> Result<Value, String> {
                // What rides on the cursor (Mike: "ensure it shows a preview of
                // what you are dragging"). ⚠ **Optional on every path** — a drag
                // with no picture still moves the file, and refusing one because
                // the page could not draw would trade the feature for the
                // decoration.
                //
                // ⛔ **Here, not on `drag_prepare`.** Prepare runs on every
                // press, including the ones that are ordinary clicks, and this
                // is ~96 KB of pixels through JSON each time.
                let preview = Option::<PreviewArgs>::deserialize(&request.args["preview"])
                    .map_err(|e| format!("bad drag image: {e}"))?
                    .map(PreviewArgs::decode)
                    .transpose()?;
                shared
                    .drags
                    .start(preview)
                    .and_then(|dropped| serde_json::to_value(dropped).map_err(|e| e.to_string()))
            })());
        }

        // A `mousedown` that never became a drag. Without this the next drag
        // would start from a stale selection.
        "drag_cancel" => {
            shared.drags.cancel();
            return Some(Ok(Value::Null));
        }

        _ => {}
    }

    let (name, resize) = match request.command.as_str() {
        "editor_size" => (current_scale(shared), false),
        "set_editor_size" => (
            request.args["size"].as_str().unwrap_or_default().to_owned(),
            true,
        ),
        _ => return None,
    };

    let Some(((width, height), zoom)) = preset(&name) else {
        let known: Vec<&str> = SCALES.iter().map(|(id, _)| *id).collect();
        return Some(Err(format!(
            "`{name}` is not a window size — expected one of {}",
            known.join(", ")
        )));
    };

    if resize {
        shared.request_resize(width, height);

        // Remembered here rather than by the UI, so the size is saved with the
        // project by the same host state call as everything else and there is
        // one place that knows what size this window is. One guard, not a
        // read-clone-write across two.
        state::update(&shared.session, |session| {
            session.window_size = Some(name.clone());
        });
    }

    // `zoom` is what the page must apply for the layout to come out at `LAYOUT`
    // inside a window this size; `next` is what the button cycles to and
    // `nextShrinks` which icon it should wear. All of it is sent rather than
    // duplicated in the frontend, so none of it can drift.
    let (next, shrinks) = next_scale(&name);
    Some(Ok(json!({
        "size": name,
        "next": next,
        "nextShrinks": shrinks,
        "width": width,
        "height": height,
        "zoom": zoom,
        // ⛔ **What the page must end up laying out in, so it can check rather
        // than trust.** `zoom` above is correct only if the window really became
        // `width` — and the resize is queued for the frame loop while this reply
        // goes back immediately, so the page can apply a zoom for a window it
        // never got. That mismatch is dead space around the UI (window bigger
        // than the layout) or a cropped app (smaller), which is what Mike
        // reported on 2026-08-06. With this the page divides by the window it
        // can measure and lands on `LAYOUT` whatever actually happened.
        "layoutWidth": LAYOUT.0,
    })))
}

/// The desktop scale factor, as a multiplier (1.5 at 150%).
///
/// ⛔ **Read once and pinned for the process, and that is a bug fix rather than
/// an optimisation.** `GetDpiForSystem` answers **96** — so 1.0 — while the
/// process is still DPI-unaware, and **baseview makes it per-monitor aware when
/// it opens its first window**, which happens *after* `create()` has already
/// sized the editor. So the same call answered 1.0 at open and 1.5 at every
/// resize afterwards, and since a window is `LAYOUT * system_scale * factor`,
/// each of the two [`SCALES`] factors silently meant two different windows.
///
/// ▶ **That is where the third size came from**, measured on Mike's machine on
/// 2026-08-06: the editor opened `medium` at **1224**, the first press made
/// `small` **1512** — *bigger* — and the second made `medium` **1836**, bigger
/// again, so the button appeared to grow twice before coming back. `SCALES`
/// held two entries the whole time. Mike: *"it only does it once and then back
/// to the default size not twice."*
///
/// ⚠ **Consistency is the property that matters here, not the value.** Pinned
/// at 1.0 the presets are 1224 and 1008; pinned at 1.5 they are 1836 and 1512.
/// Either is self-consistent and neither leaves dead space — what cannot work
/// is one preset measured against each. A host that is already DPI-aware when
/// it loads the plugin simply pins the other value, and both stay correct.
///
/// ⚠ **Known limit: this is the *system* DPI, not the monitor the editor opens
/// on.** `GetSystemMetrics` agrees with it, so any single-monitor machine is
/// correct — but a mixed-DPI pair is not, and producers run those. Opening the
/// editor on a 150% secondary while the primary is 100% sizes the window for
/// 100% and the page then zooms inside too few pixels, which crops it; the
/// reverse leaves the UI small in a window with dead space around it.
///
/// Not fixed here because there is no window handle at `create()` time, so
/// `GetDpiForWindow` is not available — it needs the DPI re-read from the frame
/// loop and `fit` re-applied when it changes, which is TASK-P12's neighbourhood
/// rather than a one-line change.
///
/// Windows only, because Windows is where the adapter's TODO bites. macOS
/// reports a backing scale factor through Cocoa that baseview already applies.
/// **Linux falls through to 1.0 and that is a known limit rather than a
/// finding**: GTK reads its own scale from `GDK_SCALE`/Xft, so a HiDPI Linux
/// desktop gets a window sized for 100% — the same shape of gap `work_area`
/// leaves there and on macOS, and no worse than either.
#[cfg(target_os = "windows")]
fn system_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        // `user32` is already linked by the window this plugin opens; declaring
        // the one call is cheaper than taking a dependency on `windows` for it.
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
    })
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
///
/// ⛔ **Pinned for the process for the same reason [`system_scale`] is, and it
/// must be pinned *with* it.** `GetSystemMetrics` answers in whatever units the
/// caller's DPI awareness implies, and it flips in the very same instant:
/// measured on Mike's machine, **1723x1035 while unaware** and **2582x1550**
/// once baseview has made the process per-monitor aware. [`physical`] compares
/// a want built from `system_scale` against this bound, so reading one before
/// the flip and the other after would compare a logical bound to a physical
/// want — a clamp to a scale nobody asked for. Reading both once keeps the pair
/// honest whichever side of the flip they land on.
#[cfg(target_os = "windows")]
fn work_area() -> Option<(u32, u32)> {
    static AREA: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *AREA.get_or_init(|| {
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
    })
}

#[cfg(not(target_os = "windows"))]
fn work_area() -> Option<(u32, u32)> {
    None
}

/// Where the page is loaded from.
///
/// One string for every platform: wry rewrites a custom scheme into
/// `http://<scheme>.<host>` on Windows itself, so asking for that form directly
/// changes nothing. It was tried — see `VENDORED.md` on the standalone's blank
/// window, which is a message-pump problem rather than a URL one.
const PAGE: &str = "freally://localhost/index.html";

pub fn create(shared: SharedState) -> Option<Box<dyn Editor>> {
    let editor = WebViewEditor::new(HTMLSource::URL(PAGE), window_size(&shared))
        // ⛔⛔ **The floor is a grab handle, not a layout rule** (2026-08-12).
        //
        // It used to be `physical(1.0)` — the whole `large` preset, ~1440×900 —
        // from Mike, 2026-08-09: *"a little smaller and it ends up clipping the
        // right panel."* He reversed it: *"i want the end user to be able to
        // resize my window to whatever size they want … if they size it too
        // small, then it's their own fault, as long as you can ensure that they
        // can resize to make it bigger."*
        //
        // ▶ **Clipping is no longer the price of a small window.** `WindowFit`
        // re-reads `window.innerWidth` on every `resize` and zooms the root, so
        // the page reflows to whatever the frame becomes rather than being cut
        // off at a fixed layout — which is why this can be relaxed now and could
        // not have been then.
        //
        // ⚠ **What the remaining floor buys is the second half of his sentence.**
        // At zero, Windows will happily hand back a client area with no room for
        // the resize grips, and a window nobody can grab is a window nobody can
        // make bigger again — the one outcome he ruled out. [`MIN_FRAME`] is
        // small enough to be "any size you want" and large enough that every
        // edge and corner stays draggable.
        //
        // ⚠ **Standalone only, and that is not a gap.** The floor is enforced
        // through `WM_GETMINMAXINFO` on our own frame; inside a DAW the window
        // belongs to the host, so VST3 and CLAP were never constrained by this
        // and are as resizable as the host allows.
        .with_minimum_size(MIN_FRAME)
        // ⛔⛔ **So the caption says it once** — Mike, 2026-08-11: *"can you
        // replace the window's title bar after the vst3/clap file opens … so it
        // just says it once?"* Ableton names a fresh track after the instrument
        // dropped on it and then builds the plugin window's caption from the
        // device *and* the track, so the name lands on both sides of a slash:
        // `Freally MIDI Master By: Mike Weaver/1-Freally MIDI Master By: Mike
        // Weaver`.
        //
        // ⚠ **`Plugin::NAME`, not a second string.** The whole point is that the
        // caption agrees with what the host calls us; a literal here would be a
        // rename waiting to disagree with the plugin browser.
        //
        // ⛔ The adapter only rewrites a caption that **already contains** this
        // name — `windows_pump::retitle` explains why, and the short version is
        // that a docked FL Studio editor's top-level window is FL's own frame.
        .with_window_title(<crate::FreallyMidiMaster as Plugin>::NAME)
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
            // ...including a kit a one-shot assignment replaced (TASK-131B),
            // which is megabytes rather than a schedule's few kilobytes and so
            // is the one that would be most audible to free in the callback.
            shared.kits.collect();

            // A size the UI asked for. This is the only place it can be applied:
            // `resize` needs the window, and the window only exists here.
            if let Some((width, height)) = shared.take_resize() {
                ctx.resize(window, width, height);
            }

            // **Nothing reads `ctx.next_event()`, deliberately.** This loop used to
            // answer commands off the webview's IPC channel, and that path is dead:
            // the UI posts everything to `/__rpc` (see [`rpc`]), because wry's IPC
            // is one-way and a window parented into Ableton never gets a frame tick
            // to push a reply from. `window.sendToPlugin` survives only as the
            // marker `isPlugin()` checks for; it is never called.
            //
            // The handler was kept here for a while and answered nothing, a
            // line-for-line copy of `rpc` that no request could reach — so the copy
            // a reader met first was the one that could never run. Deleted rather
            // than commented out, because a second answering path is exactly what
            // this bridge already lost an evening to.
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

/// What the page sends to start preparing a drag (TASK-063C).
///
/// ⛔ **A struct, like every other structured argument here** — `GenerateArgs`,
/// `ArmSongArgs`, `RerollArgs`. The first cut read these fields off the raw
/// `Value` with `as_bool().unwrap_or(false)` and `as_u64().unwrap_or(0)`, which
/// turns a renamed field into a silent default: a `lanes` the page spelled
/// differently would have exported per part while the UI said per lane, and
/// said nothing. Deserializing names the field that is wrong.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DragArgs {
    #[serde(default)]
    patterns: Vec<Pattern>,
    /// Present when the whole arrangement is being dragged.
    #[serde(default)]
    song: Option<Box<engine::pattern::Song>>,
    #[serde(default)]
    audio: bool,
    /// One file per lane rather than per part.
    #[serde(default)]
    lanes: bool,
    /// Just this one lane — *"i can just drag the hihats out"*.
    #[serde(default)]
    lane: Option<engine::pattern::Lane>,
}

/// The drag image, as the page's canvas produced it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewArgs {
    width: u32,
    height: u32,
    /// Straight RGBA, base64. ⚠ Untrusted: see [`crate::drag::Preview::new`].
    rgba: String,
}

impl PreviewArgs {
    fn decode(self) -> Result<crate::drag::Preview, String> {
        crate::drag::Preview::new(
            self.width,
            self.height,
            crate::drag::from_base64(&self.rgba)?,
        )
    }
}

/// Serve one file out of the compiled-in frontend, or answer a command.
///
/// ⛔ **Every `Response::builder()` here reports its error rather than
/// unwrapping it.** The builder is fallible — it validates header names and
/// values — and this function is called from the webview's custom-protocol
/// callback, which is an `extern "C"` frame: a panic there cannot unwind, and a
/// release build aborts the host's process. The adapter turns an `Err` into a
/// 500 the page can report. These particular builders take static headers and
/// so should never fail; `?` is what makes "should never" not mean "or else the
/// DAW closes".
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
            .body(Cow::Owned(rpc(request.body(), shared).into_bytes()))?);
    }

    match UI.get_file(path) {
        Some(file) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, mime_for(path))
            .body(Cow::Borrowed(file.contents()))?),
        // A 404 rather than falling back to `index.html`. SPA fallbacks make a
        // missing asset render as a blank page with no error — the frontend
        // has no router, so a miss here is a build that did not produce what
        // the HTML asks for, and it should say so.
        None => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, "text/plain")
            .body(Cow::Owned(
                format!("{path} is not in the bundled UI").into_bytes(),
            ))?),
    }
}

/// Answer one command, as JSON.
///
/// Never fails the HTTP request: a command that errors still returns 200 with
/// an `error` field, because a non-200 arrives at the page as a network
/// failure with no message in it — and "failed to fetch" is exactly the kind
/// of unattributable error this bridge has already cost an evening to.
/// The only commands answerable before the licence is accepted.
///
/// ⛔ **An allowlist, not a denylist.** A new command must be *added* here to be
/// reachable from an unaccepted plugin, so forgetting the gate fails closed.
/// Listing what to *block* instead would let every future command through by
/// default, which is how a consent gate quietly stops gating.
///
/// `app_info` is here because the About box and the bug reporter need a version
/// even when nothing else works; `editor_size` because a window that cannot be
/// resized to fit the screen is a window the notice cannot be read in.
const BEFORE_ACCEPTANCE: &[&str] = &[
    "eula_status",
    "eula_accept",
    "eula_decline",
    "app_info",
    "editor_size",
    "set_editor_size",
];

/// Whether this call may proceed, given the licence has not been accepted.
///
/// ⛔ **Enforced at the RPC boundary, not in the UI, and that is the point.**
/// Declining has to mean the plugin *cannot be used* — not that a dialog is
/// covering it. A page that was reloaded, bypassed, or driven from devtools
/// still arrives here, and this is what refuses it.
///
/// It sits here rather than in [`bridge::dispatch`] because this is the single
/// door the webview comes through: `window_command` and `dispatch` are both
/// behind it, so one check covers both and neither has to know a gate exists.
/// The sample paths this instance has assigned, from the plugin's own map.
///
/// ⛔ **The only source of paths for the copy pair, deliberately.** They used to
/// arrive from the page; see the note on `user_model_sample_cost`. Reading them
/// here means there is no untrusted path to validate, which is a stronger
/// position than validating one — the guard that cannot be forgotten is the one
/// that has nothing to guard.
fn assigned_paths(shared: &SharedState) -> Vec<String> {
    assigned_pairs(shared)
        .into_iter()
        .map(|(_, path)| path)
        .collect()
}

/// The same assignments, **with the lane each one is on**.
///
/// ⛔⛔ **Dropping the lane here is what orphaned every copied sample.** The
/// copy landed sixteen hex-named files in a folder with nothing recording which
/// was the kick, so the read-back could not exist — `assigned_paths` discarded
/// the one fact that made the copies usable, one call before they were written.
/// The cost half genuinely does not need lanes (it sums bytes); the copy half
/// always did.
fn assigned_pairs(shared: &SharedState) -> Vec<(Lane, String)> {
    shared
        .one_shots
        .snapshot()
        .into_iter()
        .map(|(lane, (path, _))| (lane, path))
        .collect()
}

fn licence_blocks(command: &str) -> bool {
    !BEFORE_ACCEPTANCE.contains(&command) && !crate::eula::accepted()
}

/// What the page is told when the gate refuses. Phrased as the next action,
/// because "not licensed" inside a DAW is not something anyone can act on.
const NOT_ACCEPTED: &str = "Freally MIDI Master is waiting for you to accept its licence \
     agreement. Read it in the window and choose Agree — everything works immediately after that.";

fn rpc(body: &[u8], shared: &SharedState) -> String {
    let reply = match serde_json::from_slice::<Request>(body) {
        Ok(request) if licence_blocks(&request.command) => {
            json!({ "id": request.id, "error": NOT_ACCEPTED })
        }
        Ok(request) => {
            let host = shared.host.snapshot();
            let outcome = window_command(&request, shared).unwrap_or_else(|| {
                bridge::dispatch(&request, &host, &shared.session, &shared.exports)
            });
            match outcome {
                Ok(value) => {
                    // A generation is the one command with a side effect
                    // beyond its reply: the notes have to reach the audio
                    // thread. Arming happens here, off the audio thread,
                    // because that is where the allocation belongs.
                    // ⛔ Declining has to stop a pattern that is already armed.
                    // The gate blocks new commands, but the audio thread does
                    // not consult it — so without this the UI says "nothing in
                    // the plugin will generate, play, export or save" while the
                    // last generation keeps playing on every transport start.
                    // An empty schedule replaces the live one by the same
                    // handoff the arming below uses.
                    // ⛔ `disarm` joins it for a different reason with the same
                    // shape: leaving Song Mode has to take the arrangement off
                    // the transport, and a session that only ever used Song
                    // Mode has no clip to put back in its place — so without an
                    // explicit empty schedule the whole record kept playing
                    // under an empty drum grid.
                    if request.command == "eula_decline" || request.command == "disarm" {
                        shared.handoff.send(Schedule::default());
                        shared.disarmed();
                    }

                    // ⛔ **Armed from the *shape* of the reply, not from the
                    // command's name.** This used to test for
                    // `generate_pattern`, and TASK-041 is exactly the case that
                    // broke: the piano roll edits notes and sends them back
                    // through `arm_pattern`, so a name match meant an edited
                    // clip never reached the audio thread — the producer moved a
                    // note, saw it move, pressed play and heard the note they
                    // had just moved away from. Any command that answers with a
                    // `Pattern` is a command that changed what should be
                    // playing, and there is no fourth string to remember.
                    //
                    // Safe to key on the type: `Pattern` has twelve required
                    // fields, so no other reply in the bridge deserializes into
                    // one by accident.
                    // ⛔ Deserialized *from the reference*. `from_value` takes
                    // the `Value` by move, so testing the shape used to deep-
                    // clone every reply the bridge ever sends — the roster, the
                    // dataset problems, the lot — to throw all but one away.
                    if let Ok(pattern) = Pattern::deserialize(&value) {
                        let mut schedule = Schedule::default();
                        schedule.arm(&pattern, shared.sample_rate());
                        // ⛔ **Hold the playhead when this is the clip already
                        // playing.** A fresh `Schedule` has no `armed_id`, so
                        // `arm`'s own resume path could never fire here and
                        // every arm reset the position. That was invisible while
                        // only a
                        // generation re-armed — and then muting a part, soloing
                        // one, toggling a loop or starting an audition all began
                        // re-arming the song, so clicking any of them mid-record
                        // threw it back to bar 1.
                        if shared.arming(&pattern.id) {
                            schedule.seek(shared.playhead());
                        }
                        shared.handoff.send(schedule);
                    }

                    // ⛔ The audio thread cannot read the session — taking that
                    // lock inside `process` is the dropout this whole module
                    // avoids — so the two FMM-S02 switches live as atomics and
                    // are mirrored here, from the value that was just written.
                    // Mirrored from the *store* rather than from the request, so
                    // a project restore and a user toggle take the same path and
                    // cannot disagree.
                    if request.command == "save_session_state" {
                        shared.adopt_session();
                    }
                    json!({ "id": request.id, "ok": value })
                }
                Err(message) => {
                    // ⛔⛔ **A refused command is invisible otherwise, and that
                    // is how a real defect reads as "it does nothing".** The
                    // reply carries the reason to the page, but nothing puts it
                    // anywhere a developer reading the standalone's output can
                    // see — so on 2026-08-06 a dead Drums drag and a refused
                    // audio drag both presented as silence, and diagnosing them
                    // began by adding this line. Cheap: it only runs when a
                    // command has already failed.
                    nih_plug::nih_log!("[rpc] {} refused: {message}", request.command);
                    json!({ "id": request.id, "error": message })
                }
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
    fn the_webview_profile_lives_beside_the_rest_of_the_user_data() {
        // ⛔⛔ **Two spellings of one directory, made to agree.** The vendored
        // crate cannot reach `presets::data_dir()`, so it restates the
        // `%APPDATA%`/`Application Support`/`XDG_DATA_HOME` walk itself — and a
        // restatement nothing compares is how one of them drifts. The cost of
        // drifting here is silent and total: the profile would go back to
        // wherever the other spelling pointed, and every producer's rail layout,
        // pad assignments, theme and language would reset with no error.
        //
        // ⚠ The *reason* it may not sit under temp is `web_data_dir`'s own doc:
        // Windows deletes temp on a schedule, and 17.9 MB of `localStorage` was
        // living there.
        let Some(data) = crate::presets::data_dir() else {
            // No per-user directory on this machine; the fallback is deliberate
            // and there is nothing to compare against.
            return;
        };
        let profile = nih_plug_webview::web_data_dir();
        assert!(
            profile.starts_with(&data),
            "the webview profile ({}) must sit under the user data directory ({}) — \
             see `web_data_dir`",
            profile.display(),
            data.display()
        );
        assert!(
            !profile.starts_with(std::env::temp_dir()),
            "the webview profile is back under temp, which the OS deletes: {}",
            profile.display()
        );
    }

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

    /// The shared fixture, so this crate does not carry a sixth RIFF writer.
    use crate::preview::tests::wav_bytes;

    fn ramp_wav(frames: usize) -> Vec<u8> {
        wav_bytes(frames, 44_100)
    }

    fn ask(shared: &SharedState, command: &str, args: Value) -> Value {
        let request = Request {
            id: 1,
            command: command.into(),
            args,
        };
        window_command(&request, shared)
            .unwrap_or_else(|| panic!("`{command}` is not a command the editor answers"))
            .unwrap_or_else(|error| panic!("`{command}` was refused: {error}"))
    }

    #[test]
    fn auditioning_a_lane_asks_for_that_lane_and_refuses_a_name_it_does_not_know() {
        // TASK-043's "click a lane header to hear that pad". ⛔ The lane→GM
        // mapping is resolved *here* rather than on the page, so this is what
        // proves the page can send a lane name at all — and that an unknown one
        // is refused rather than defaulted into auditioning some other drum.
        let shared: SharedState = std::sync::Arc::new(crate::shared::Shared::default());

        ask(&shared, "audition_lane", json!({ "lane": "kick" }));
        assert_eq!(
            shared.take_audition(),
            Some(crate::shared::Audition::Lane(engine::pattern::Lane::Kick)),
            "the request must name the kick, and must say it is a lane"
        );

        // The serde alias the saved-project format depends on still resolves.
        ask(&shared, "audition_lane", json!({ "lane": "bass808" }));
        assert_eq!(
            shared.take_audition(),
            Some(crate::shared::Audition::Lane(engine::pattern::Lane::Sub))
        );

        // ⛔ **`subLow` is a distinct request from `sub`, and it was not.** Both
        // lanes are rows in the drum grid and both answer GM note 0, so while
        // the lane travelled as a note these two were the same message.
        ask(&shared, "audition_lane", json!({ "lane": "subLow" }));
        assert_eq!(
            shared.take_audition(),
            Some(crate::shared::Audition::Lane(engine::pattern::Lane::SubLow))
        );

        for bad in [json!({ "lane": "trumpet" }), json!({})] {
            let request = Request {
                id: 1,
                command: "audition_lane".into(),
                args: bad.clone(),
            };
            assert!(
                window_command(&request, &shared).is_some_and(|r| r.is_err()),
                "{bad} should be refused rather than guessed at"
            );
        }
        assert!(
            shared.take_audition().is_none(),
            "a refused request must not have queued a note"
        );
    }

    #[test]
    fn the_audition_plays_through_the_commands_the_panel_actually_sends() {
        // ⛔⛔ **The end-to-end gate for TASK-132's player.** `preview.rs` proves
        // the voice reads a buffer correctly and `explorer.rs` proves the
        // library is safe; neither can see whether the *commands the page sends*
        // reach them. Every one of these was written, tested from Rust and
        // called by nothing until this session, so "does the command surface
        // work" is exactly the question nothing was asking.
        //
        // ⚠ Driven through `window_command` with the same argument shapes
        // `src/state/explorer.ts` sends, so a renamed key fails here rather than
        // in somebody's DAW.
        let dir = std::env::temp_dir().join(format!("fmm-editor-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join("ramp.wav");
        std::fs::write(&path, ramp_wav(44_100)).expect("a temp wav");
        let path = path.to_str().unwrap().to_owned();

        let shared: SharedState = std::sync::Arc::new(crate::shared::Shared::default());
        // The producer added this folder to their library; without it every
        // read is refused, which is the containment boundary doing its job.
        shared.explorer.restore(&[dir.to_str().unwrap().to_owned()]);

        // ── The waveform the panel draws, and the sample it loads.
        let wave = ask(&shared, "explorer_waveform", json!({ "path": path }));
        assert!(
            wave["peaks"].as_array().is_some_and(|p| !p.is_empty()),
            "the panel has nothing to draw: {wave}"
        );
        ask(&shared, "preview_load", json!({ "path": path }));

        // ⚠ One render to pick the handoff up, exactly as a block would.
        let mut out = vec![0.0f32; 64];
        shared.preview.render(&mut out, 1, 44_100.0);

        // Loading is browsing, not auditioning.
        let at = ask(&shared, "preview_position", json!({}));
        assert_eq!(at["playing"], json!(false), "clicking a row must be silent");
        assert!(
            (at["total"].as_f64().unwrap() - 1.0).abs() < 0.01,
            "a second of audio: {at}"
        );

        // ── Play sounds.
        ask(&shared, "preview_play", json!({}));
        out.fill(0.0);
        shared.preview.render(&mut out, 1, 44_100.0);
        assert!(
            out.iter().any(|s| *s != 0.0),
            "Play produced silence — the audition never reached the callback"
        );
        assert!(
            out.windows(2).all(|w| w[1] >= w[0]),
            "a ramp played forwards climbs: {:?}",
            &out[..8]
        );

        // ── Backwards.
        ask(&shared, "preview_stop", json!({}));
        ask(&shared, "preview_reverse", json!({ "on": true }));
        ask(&shared, "preview_play", json!({}));
        out.fill(0.0);
        shared.preview.render(&mut out, 1, 44_100.0);
        assert!(
            out.windows(2).all(|w| w[1] <= w[0]),
            "the same ramp played backwards falls: {:?}",
            &out[..8]
        );
        assert_eq!(
            ask(&shared, "preview_position", json!({}))["reverse"],
            json!(true)
        );

        // ── A click in the waveform, while it is playing.
        //
        // ⚠ **Applied by the next block, not by the command**, and that is the
        // design rather than a delay worth removing: the audio thread owns the
        // cursor while it renders, so the seek is published as a request and
        // consumed at the top of the next callback. The page covers the gap by
        // writing the clicked position optimistically.
        ask(&shared, "preview_reverse", json!({ "on": false }));
        ask(&shared, "preview_seek", json!({ "seconds": 0.5 }));
        out.fill(0.0);
        shared.preview.render(&mut out, 1, 44_100.0);
        let at = ask(&shared, "preview_position", json!({}));
        assert!(
            (at["seconds"].as_f64().unwrap() - 0.5).abs() < 0.01,
            "a click has to land where it was clicked: {at}"
        );

        // ...and while it is paused, where nothing renders, the readout still
        // has to move — otherwise the marker snaps back on the next poll.
        ask(&shared, "preview_pause", json!({}));
        ask(&shared, "preview_seek", json!({ "seconds": 0.25 }));
        let at = ask(&shared, "preview_position", json!({}));
        assert!(
            (at["seconds"].as_f64().unwrap() - 0.25).abs() < 0.01,
            "a paused click must be visible immediately: {at}"
        );

        // ── Loop, which is the difference between running out and not.
        ask(&shared, "preview_loop", json!({ "on": true }));
        ask(&shared, "preview_play", json!({}));
        // Far more frames than the sample has.
        let mut long = vec![0.0f32; 44_100 * 3];
        shared.preview.render(&mut long, 1, 44_100.0);
        let at = ask(&shared, "preview_position", json!({}));
        assert_eq!(at["playing"], json!(true), "a loop never runs out: {at}");
        assert_eq!(at["looping"], json!(true));

        // ...and off, it stops at the end.
        ask(&shared, "preview_loop", json!({ "on": false }));
        ask(&shared, "preview_stop", json!({}));
        ask(&shared, "preview_play", json!({}));
        shared.preview.render(&mut long, 1, 44_100.0);
        assert_eq!(
            ask(&shared, "preview_position", json!({}))["playing"],
            json!(false),
            "without a loop it has to finish"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_three_commands_that_take_a_raw_path_apply_the_same_containment() {
        // ⛔⛔ **Asserted as a set, because the defect was one of three missing
        // it.** `preview_load` and `explorer_waveform` both refused a file
        // outside the library; `explorer_drop` refused only a *remote* path and
        // would decode any local file the page named. Naming them together is
        // what stops the next command from being added with two guards instead
        // of three.
        let dir = std::env::temp_dir().join(format!("fmm-editor-contain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("library")).expect("a temp dir");
        let outside = dir.join("secret.wav");
        std::fs::write(&outside, ramp_wav(64)).expect("a temp wav");
        let outside = outside.to_str().unwrap().to_owned();

        let shared: SharedState = std::sync::Arc::new(crate::shared::Shared::default());
        shared
            .explorer
            .restore(&[dir.join("library").to_str().unwrap().to_owned()]);

        for (command, args) in [
            ("preview_load", json!({ "path": outside })),
            ("explorer_waveform", json!({ "path": outside })),
            ("explorer_drop", json!({ "lane": "kick", "path": outside })),
        ] {
            let request = Request {
                id: 1,
                command: command.into(),
                args,
            };
            let error = window_command(&request, &shared)
                .unwrap_or_else(|| panic!("`{command}` is not a command the editor answers"))
                .expect_err("a file outside the library must be refused");
            assert!(
                error.contains("sample library"),
                "`{command}` let a file outside the library through: {error}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sample_outside_the_library_is_refused_by_the_loader_too() {
        // ⚠ The same boundary `explorer_waveform` has. Both take a raw path from
        // the webview, and the browser must only *play* what it would list.
        let dir = std::env::temp_dir().join(format!("fmm-editor-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("library")).expect("a temp dir");
        let outside = dir.join("secret.wav");
        std::fs::write(&outside, ramp_wav(64)).expect("a temp wav");

        let shared: SharedState = std::sync::Arc::new(crate::shared::Shared::default());
        shared
            .explorer
            .restore(&[dir.join("library").to_str().unwrap().to_owned()]);

        let request = Request {
            id: 1,
            command: "preview_load".into(),
            args: json!({ "path": outside.to_str().unwrap() }),
        };
        let error = window_command(&request, &shared)
            .expect("the command exists")
            .expect_err("a sample outside the library must be refused");
        assert!(error.contains("sample library"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_licence_gate_is_an_allowlist_and_covers_the_whole_bridge() {
        // ⛔ The property that matters is not "generate is blocked" — it is that
        // *everything* is, apart from the named few. A denylist would pass a
        // test naming generate and still let the next command through.
        for command in [
            "generate_pattern",
            "roster_summary",
            "session_state",
            "save_session_state",
            "preset_save",
            "host_session",
            "playback_status",
            "some_command_added_next_year",
        ] {
            assert!(
                BEFORE_ACCEPTANCE.contains(&command)
                    || licence_blocks(command)
                    || crate::eula::accepted(),
                "{command} must be refused until the licence is accepted"
            );
        }

        // The gate's own commands have to survive it, or it cannot be answered.
        for command in ["eula_status", "eula_accept", "eula_decline"] {
            assert!(
                !licence_blocks(command),
                "{command} is how the gate is answered and must never be blocked"
            );
        }
    }

    #[test]
    fn a_blocked_call_is_answered_rather_than_left_hanging() {
        // A refusal still has to be a well-formed reply carrying the request id.
        // A dropped call leaves the UI on a spinner, which is indistinguishable
        // from the plugin being broken — the failure `rpc` already exists to avoid.
        if crate::eula::accepted() {
            return; // This machine has accepted; the path under test is not reachable.
        }

        let body = br#"{"id":7,"command":"generate_pattern","args":{}}"#;
        let reply: Value = serde_json::from_str(&rpc(body, &SharedState::default())).unwrap();

        assert_eq!(reply["id"], 7);
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains("Agree"),
            "the refusal must say what to do about it"
        );
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

    /// A `Request` for one of the window commands.
    fn command(name: &str) -> Request {
        Request {
            id: 1,
            command: name.to_owned(),
            args: json!({}),
        }
    }

    /// The three standalone-only transport arms, on both sides of the gate.
    ///
    /// ⛔ **These had no test at all, and both obvious mutations stayed green:**
    /// inverting `playback_status`'s standalone branch, and dropping the
    /// `shared.standalone` guard on `transport_play`. `Shared::standalone_for_test`
    /// exists precisely so a host and a standalone can both be exercised in one
    /// test binary — the process-wide flag cannot be.
    mod transport {
        use super::*;
        use crate::shared::Shared;
        use std::sync::Arc;

        /// ⛔⛔ **INVERTED at TASK-138.** This asserted that a host is told
        /// *"Press play in your DAW"* — the string that disabled Play there. The
        /// plugin drives its own preview transport now, so nothing is wrong in a
        /// host and there is nothing to explain. Mike, 2026-08-04: *"i do not
        /// want to just use Ableton's transpose play button."*
        #[test]
        fn a_host_is_given_no_reason_now_that_it_has_a_transport_of_its_own() {
            let shared: SharedState = Arc::new(Shared::default());
            let reply = window_command(&command("playback_status"), &shared)
                .expect("playback_status is a window command")
                .expect("it answers rather than failing");

            // ⚠ Still reported: the page uses it for the standalone-only bits of
            // the UI, and it is not the same question as "may I press Play".
            assert_eq!(reply["standalone"], json!(false));
            assert_eq!(
                reply["reason"],
                Value::Null,
                "a host has its own preview transport, so nothing is refused: {reply}"
            );
        }

        #[test]
        fn the_standalone_is_given_no_reason_because_nothing_is_wrong() {
            let shared: SharedState = Arc::new(Shared::standalone_for_test());
            let reply = window_command(&command("playback_status"), &shared)
                .expect("playback_status is a window command")
                .expect("it answers rather than failing");

            assert_eq!(reply["standalone"], json!(true));
            assert_eq!(
                reply["reason"],
                Value::Null,
                "there is no DAW to press play in, so there is nothing to say"
            );
        }

        /// ⛔⛔ **INVERTED at TASK-138.** It asserted a host was *refused* Play
        /// with "the host owns the transport". That is right about the DAW's
        /// timeline and wrong about auditioning, which is what
        /// `Shared::set_running` now records at length.
        #[test]
        fn a_host_drives_its_own_preview_transport_rather_than_being_refused() {
            let shared: SharedState = Arc::new(Shared::default());
            // ⛔ Starts stopped. A `true` here would mean the preview sounds from
            // the moment the plugin loads — see `Shared::new`.
            assert!(!shared.running(), "a freshly loaded plugin is not playing");

            window_command(&command("transport_play"), &shared)
                .expect("the command is known")
                .expect("a host may drive its own preview");
            assert!(shared.running(), "Play must start the preview in a host");

            window_command(&command("transport_pause"), &shared)
                .expect("the command is known")
                .expect("a host may hold its own preview");
            assert!(!shared.running(), "Pause must hold it");
        }

        #[test]
        fn the_standalone_runs_and_holds_its_own_transport() {
            let shared: SharedState = Arc::new(Shared::standalone_for_test());
            assert!(!shared.running(), "stopped until someone presses Play");

            window_command(&command("transport_play"), &shared)
                .expect("known")
                .expect("the standalone owns this");
            assert!(shared.running());

            window_command(&command("transport_pause"), &shared)
                .expect("known")
                .expect("the standalone owns this");
            assert!(!shared.running());
        }

        /// ⛔⛔ **INVERTED at TASK-138.** Stop used to leave a host's flag alone,
        /// because that flag was a constant the DAW's transport overrode. It is
        /// the *preview* flag now, so Stop must hold it in both shells or the
        /// preview carries straight on playing from the top.
        ///
        /// ⚠ **Stop still cannot stop a DAW, and nothing here claims it can.**
        /// `lib.rs` gates on `host_playing || preview`: taking the preview down
        /// leaves the host's own playback untouched, which is exactly the
        /// separation the old test was protecting.
        #[test]
        fn stop_rewinds_and_holds_the_preview_in_both_shells() {
            let shells: [SharedState; 2] = [
                Arc::new(Shared::default()),
                Arc::new(Shared::standalone_for_test()),
            ];
            for shared in shells {
                let standalone = shared.standalone;
                shared.set_running(true);

                window_command(&command("stop_playback"), &shared)
                    .expect("known")
                    .expect("answers");

                assert_eq!(shared.take_seek(), Some(0.0), "standalone={standalone}");
                assert!(
                    !shared.running(),
                    "stop must hold the preview transport (standalone={standalone})"
                );
            }
        }
    }

    /// The KIT panel's window commands (TASK-131B, TASK-136).
    mod kit {
        use super::*;
        use crate::shared::Shared;
        use engine::pattern::Lane;
        use std::sync::Arc;

        fn with_args(name: &str, args: Value) -> Request {
            Request {
                id: 1,
                command: name.to_owned(),
                args,
            }
        }

        /// A real, decodable WAV on disk, for the paths that take a file name.
        ///
        /// Written rather than checked in: what these tests are for is that the
        /// whole path works, and a fixture in the repo would be a second copy of
        /// `kitgen`'s output with nothing keeping it in step.
        fn written_sample() -> std::path::PathBuf {
            let frames = 512usize;
            let mut wav = Vec::with_capacity(44 + frames * 2);
            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&((36 + frames * 2) as u32).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&44_100u32.to_le_bytes());
            wav.extend_from_slice(&88_200u32.to_le_bytes());
            wav.extend_from_slice(&2u16.to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&((frames * 2) as u32).to_le_bytes());
            for frame in 0..frames {
                let value = if frame % 2 == 0 { i16::MAX } else { i16::MIN };
                wav.extend_from_slice(&value.to_le_bytes());
            }

            let dir = std::env::temp_dir().join("fmm-one-shot-panel");
            std::fs::create_dir_all(&dir).expect("the temp folder must be creatable");
            let path = dir.join("one-shot-test.wav");
            std::fs::write(&path, &wav).expect("the sample must be writable");
            path
        }

        #[test]
        fn the_panel_is_told_what_is_actually_loaded() {
            // ⛔ **TASK-136's gate.** `RightRail` rendered eight hardcoded
            // disabled buttons and a static "No kit yet" while a twelve-pad kit
            // was loaded and audibly playing. The fix is not a better string —
            // it is that the panel is told, and this is what tells it.
            let shared: SharedState = Arc::new(Shared::default());
            let reply = window_command(&command("kit_state"), &shared)
                .expect("kit_state is a window command")
                .expect("it answers rather than failing");

            assert_eq!(reply["id"], json!("trap-default"));
            let lanes = reply["lanes"].as_array().expect("a lane list");
            assert_eq!(
                lanes.len(),
                crate::shared::ALL_LANES.len(),
                "every lane the engine has, not a list written in the panel"
            );

            let of = |name: &str| {
                lanes
                    .iter()
                    .find(|entry| entry["lane"] == json!(name))
                    .unwrap_or_else(|| panic!("{name} must be listed"))
                    .clone()
            };

            // A lane the shipped kit covers, with nothing assigned over it.
            assert_eq!(of("melody")["shipped"], json!(true));
            assert_eq!(of("melody")["name"], Value::Null);

            // ⛔ **`Snap` used to be the lane nothing played, and this asserted
            // `false` for it.** TASK-140 gave every lane a default voice, so
            // the honest assertion is now the stronger one: the panel reports
            // the shipped kit as covering *everything*, with no lane left
            // silent. A `false` here means a kit shipped with a hole in it.
            for entry in lanes {
                assert_eq!(
                    entry["shipped"],
                    json!(true),
                    "lane {} has no shipped voice",
                    entry["lane"]
                );
                assert_eq!(entry["name"], Value::Null, "nothing is assigned yet");
            }
        }

        #[test]
        fn an_assigned_sample_shows_up_in_the_panel_under_its_own_name() {
            let shared: SharedState = Arc::new(Shared::default());
            // Restoring is the no-dialog path, so it is the one a test can
            // drive; `assign` differs only in where the path comes from.
            let sample = written_sample();
            shared
                .one_shots
                .restore(
                    Lane::Melody,
                    sample.to_str().expect("a utf-8 temp path"),
                    false,
                    &shared.kits,
                    &shared.session,
                )
                .expect("a real WAV must load");

            let reply = window_command(&command("kit_state"), &shared)
                .expect("known")
                .expect("answers");
            let melody = reply["lanes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["lane"] == json!("melody"))
                .unwrap()
                .clone();

            assert_eq!(melody["name"], json!("one-shot-test.wav"));
            assert!(melody["path"]
                .as_str()
                .unwrap()
                .ends_with("one-shot-test.wav"));

            // ...and clearing it puts the lane back on the shipped voice.
            window_command(
                &with_args("one_shot_clear", json!({ "lane": "melody" })),
                &shared,
            )
            .expect("known")
            .expect("answers");
            let reply = window_command(&command("kit_state"), &shared)
                .expect("known")
                .expect("answers");
            let melody = reply["lanes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["lane"] == json!("melody"))
                .unwrap()
                .clone();
            assert_eq!(melody["name"], Value::Null);
            assert_eq!(melody["shipped"], json!(true));
        }

        #[test]
        fn a_lane_the_engine_does_not_have_is_refused_rather_than_defaulted() {
            // ⛔ These arrive from the webview. Defaulting to `Kick` would put
            // somebody's sample on the wrong pad, which is much harder to
            // notice than an error and impossible to explain.
            let shared: SharedState = Arc::new(Shared::default());
            for command_name in ["one_shot_assign", "one_shot_clear"] {
                let error = window_command(
                    &with_args(command_name, json!({ "lane": "kazoo" })),
                    &shared,
                )
                .expect("the command is known")
                .expect_err("an unknown lane must be refused");
                assert!(error.contains("is not a lane"), "{command_name}: {error}");

                // A missing argument is the same mistake with a different shape.
                assert!(
                    window_command(&command(command_name), &shared)
                        .expect("known")
                        .is_err(),
                    "{command_name} with no lane must be refused"
                );
            }
        }

        #[test]
        fn the_status_is_taken_once_so_a_poll_does_not_repeat_itself() {
            // The page polls this while a dialog is open. See
            // `oneshot::OneShots::take_status` — a terminal status left in the
            // slot is a toast that never goes away.
            let shared: SharedState = Arc::new(Shared::default());
            let reply = window_command(&command("one_shot_status"), &shared)
                .expect("known")
                .expect("answers");
            assert_eq!(reply["state"], json!("idle"));
        }
    }
}
