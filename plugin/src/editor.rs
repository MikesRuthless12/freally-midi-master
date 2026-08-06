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

use engine::pattern::Pattern;

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
    let at = SCALES.iter().position(|(id, _)| *id == name).unwrap_or(0);
    let here = physical(SCALES[at].1).0;

    for step in 1..=SCALES.len() {
        let (id, factor) = SCALES[(at + step) % SCALES.len()];
        if physical(factor).0 != here {
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
    #[test]
    fn the_button_grows_the_window_then_returns_it_to_the_default() {
        let (next, shrinks) = next_scale(DEFAULT_SCALE);
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

        let (back, shrinks_back) = next_scale("xl");
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
        let sizes: Vec<(u32, u32)> = SCALES.iter().map(|(_, f)| physical(*f).0).collect();
        let mut distinct = sizes.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            SCALES.len(),
            "two presets must be two windows, got {sizes:?}"
        );

        // And asking twice must answer the same, which is the property that
        // actually broke — the cycle read it once per press.
        for (name, _) in SCALES {
            assert_eq!(
                preset(name),
                preset(name),
                "`{name}` must be one window for the whole session"
            );
        }
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
        "audition_note" => {
            // Clamped once, in `request_audition`, which is where the test that
            // proves it lives. A second clamp here would be a second authority
            // on the same rule.
            let pitch = request.args["pitch"].as_u64().unwrap_or(60);
            shared.request_audition(pitch.try_into().unwrap_or(127));
            return Some(Ok(Value::Null));
        }

        // ⛔ Stop is a seek to zero, and inside a DAW Pause is *nothing at
        // all* — which is the whole distinction. The host owns whether time is
        // running; the plugin only owns where in the pattern it is. So Pause
        // has nothing to do there (the host stops calling `process` with a
        // moving transport, and the marker simply stays where it was), while
        // Stop has to move the marker back itself.
        //
        // The standalone has no host to own it, so it owns it here: Stop also
        // holds the transport, or the marker snaps to zero and the pattern
        // carries straight on playing from the top (TASK-041T).
        // ⛔ `stop_playback`, not `transport_stop`. The name is the one the
        // frontend already invokes and the one the mock answers — and a bridge
        // that answers a *different* name fails in the quietest possible way:
        // the unknown command rejects, `stop()` swallows it, the marker snaps
        // to zero locally, and the audio thread never rewinds. The beat keeps
        // playing from the middle of the pattern while the playhead says it
        // stopped.
        "stop_playback" => {
            shared.request_seek(0.0);
            // `set_running` is itself gated on being the standalone, so this
            // needs no guard of its own.
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
        "playback_status" => {
            return Some(Ok(json!({
                "standalone": shared.standalone,
                "reason": if shared.standalone {
                    Value::Null
                } else {
                    Value::String(
                        "Press play in your DAW — the plugin puts the notes on the track."
                            .into(),
                    )
                },
            })));
        }

        // ⛔ Both refused outside the standalone, rather than silently doing
        // nothing. In a host these controls are not ours to offer — the UI
        // does not render them — so a call arriving here means the page and
        // the plugin disagree about which shell they are in, and that is worth
        // an error rather than a no-op that looks like a broken button.
        "transport_play" | "transport_pause" => {
            if !shared.standalone {
                return Some(Err(
                    "the host owns the transport — press play in your DAW".into()
                ));
            }
            shared.set_running(request.command == "transport_play");
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
            let base = crate::audio::preview_kit();
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

        #[test]
        fn a_host_is_told_who_owns_the_transport() {
            let shared: SharedState = Arc::new(Shared::default());
            let reply = window_command(&command("playback_status"), &shared)
                .expect("playback_status is a window command")
                .expect("it answers rather than failing");

            assert_eq!(reply["standalone"], json!(false));
            assert!(
                reply["reason"].as_str().unwrap_or_default().contains("DAW"),
                "a host needs a reason a human can act on: {reply}"
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

        #[test]
        fn a_host_is_refused_the_transport_rather_than_quietly_ignored() {
            // ⛔ An error, not a no-op. A `transport_play` arriving here means the
            // page and the plugin disagree about which shell they are in, and a
            // silent success would leave that disagreement invisible.
            let shared: SharedState = Arc::new(Shared::default());
            for name in ["transport_play", "transport_pause"] {
                let error = window_command(&command(name), &shared)
                    .expect("the command is known")
                    .expect_err("a host must be refused");
                assert!(error.contains("host owns the transport"), "{name}: {error}");
            }
            assert!(shared.running(), "a refusal must not have moved anything");
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

        #[test]
        fn stop_rewinds_and_holds_in_the_standalone_but_only_rewinds_in_a_host() {
            // The two halves of Stop. In a host the DAW owns whether time runs,
            // so Stop may only move the marker; in the standalone it must also
            // hold, or the pattern carries straight on playing from the top.
            let standalone: SharedState = Arc::new(Shared::standalone_for_test());
            standalone.set_running(true);
            window_command(&command("stop_playback"), &standalone)
                .expect("known")
                .expect("answers");
            assert_eq!(standalone.take_seek(), Some(0.0));
            assert!(
                !standalone.running(),
                "stop holds the standalone's transport"
            );

            let hosted: SharedState = Arc::new(Shared::default());
            window_command(&command("stop_playback"), &hosted)
                .expect("known")
                .expect("answers");
            assert_eq!(hosted.take_seek(), Some(0.0));
            assert!(hosted.running(), "stop must not stop a DAW");
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

            // ⚠ And the one lane it has never covered. The drum generator can
            // write `Snap` and no pad has ever played it, so it is silent — the
            // panel has to be able to say that rather than drawing it like the
            // others.
            assert_eq!(of("snap")["shipped"], json!(false));
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
