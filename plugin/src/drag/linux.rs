//! The Linux drag source (TASK-063C / FMM-S03).
//!
//! GTK's own drag, carrying `text/uri-list` — which is what a file dragged out
//! of Nautilus carries, so it is the format every Linux DAW's drop target has
//! already had to handle. That is the same argument [`super`] makes for
//! `CF_HDROP` over a promised file on Windows, and it is the reason this is not
//! hand-rolled XDND: GTK is already in the process, already running a main
//! loop, and already knows the protocol.
//!
//! ## ⛔ Why this needs no edit to the vendored adapter
//!
//! The obvious route is the webview's own `GtkWidget`, and it is a trap: wry's
//! webviews live in a `thread_local` registry *inside* `nih_plug_webview`,
//! addressed by an id this crate never sees. Reaching one would mean adding a
//! public accessor to somebody else's crate and carrying it across every
//! rebase — the cost `VENDORED.md` exists to account for.
//!
//! None of that is necessary. A GTK drag needs *a realized widget* to own the
//! drag context and answer `drag-data-get`; it does not need to be the widget
//! under the cursor. So this makes its own [`gtk::Invisible`], starts the drag
//! from that, and destroys it when the drag ends. The adapter is untouched.
//!
//! ## ⛔⛔ Synchronous out here, asynchronous in there — and the trap in between
//!
//! [`super::Drags::start`] blocks for the length of the gesture and returns a
//! [`Dropped`]. That shape comes from `DoDragDrop`, which is genuinely modal. A
//! GTK drag is **not**: `drag_begin` returns at once and the gesture finishes
//! later, on `drag-end`, delivered *by the GTK main loop*.
//!
//! ⛔⛔ **So this must never simply block waiting for that signal, and the first
//! cut did.** It posted the start with `MainContext::invoke`, then waited on a
//! channel for ten minutes. Two facts from the vendored adapter make that a
//! deadlock rather than a wait: `nih_plug_webview::linux` answers `/__rpc`
//! **on the GTK thread**, and `g_main_context_invoke` runs its job *inline*
//! when the calling thread already owns the context. So the drag started on the
//! right thread and was then parked by the very call that was waiting for it —
//! the main loop could not deliver `drag-end` because this was sitting on it.
//! A producer got a frozen plugin window for ten minutes and no drop.
//!
//! ▶ **What it does instead: a nested main loop, which is what GTK itself does
//! for exactly this** — `gtk_dialog_run` blocks a caller while keeping the loop
//! turning. [`drag`] pumps `gtk::main_iteration` until the outcome arrives, so
//! the caller blocks *and* the drag it started keeps running.
//!
//! ⚠ **Both threads are still handled**, because which one calls is the
//! adapter's business and not ours: on the GTK thread the drag starts inline and
//! pumps; anywhere else the old post-and-wait is correct, precisely because the
//! GTK thread is then free to run the drag.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use super::{Dropped, Preview, NO_DRAG_SOURCE};

pub const SUPPORTED: bool = true;

/// ✅ **Yes.** `own_message_queue()` is a no-op off Windows, and a GTK drag runs
/// on the GTK thread's own main loop rather than re-entering anybody's window
/// procedure. The reentrancy that forces the Windows refusal has no analogue.
pub const STANDALONE_SAFE: bool = true;

/// The one format offered. See the module header for why it is this one.
const URI_LIST: &str = "text/uri-list";

/// How long to wait for a drag that never reports back.
///
/// ⚠ **A backstop for a lost signal, not a limit on the gesture.** A producer
/// may hold a drag over their arrangement for as long as they like; what this
/// catches is a `drag-end` that never arrives because the GTK thread died or
/// the widget was destroyed underneath it. Without it the RPC thread — which is
/// the thread the DAW draws its editor from — would block forever.
const LOST_SIGNAL: Duration = Duration::from_secs(600);

/// How often the nested loop is woken so it can check the clock.
///
/// ⚠ **Without this the backstop above is unreachable.** `gtk::main_iteration`
/// blocks until there *is* an event, so a drag whose signal went missing and
/// whose pointer has stopped moving would park here forever — the exact hang
/// [`LOST_SIGNAL`] exists to bound. A source that does nothing but tick
/// guarantees the loop comes back around to look at the time.
const TICK: Duration = Duration::from_millis(100);

/// Hand `paths` to the desktop and block until the producer lets go.
///
/// `stacked` is the alternative set to offer **while Ctrl is held**, or empty
/// where the gesture has only one meaning — the same contract the Windows side
/// documents at [`super::platform::drag`].
pub fn drag(
    paths: &[PathBuf],
    stacked: &[PathBuf],
    _preview: Option<&Preview>,
) -> Result<Dropped, String> {
    if paths.is_empty() {
        return Err("there are no files to drag".to_owned());
    }
    // ⛔ **GTK may never have started.** `nih_plug_webview::linux` gives up
    // rather than taking the host down when `gtk_init` fails, and in that case
    // there is no editor either — so this is the same refusal every other
    // platform without a drag source gives, rather than a panic.
    if !gtk::is_initialized() {
        return Err(NO_DRAG_SOURCE.to_owned());
    }

    let plain = uris(paths)?;
    let held = uris(stacked)?;

    // ⛔ The ordinary case: the adapter answers `/__rpc` on the GTK thread, so
    // the drag starts inline and this pumps the loop that runs it. See the
    // module header for why waiting on a channel here is a deadlock.
    if gtk::is_initialized_main_thread() {
        return pump(plain, held);
    }

    // Off the GTK thread, posting and waiting is right: that thread is free to
    // run the drag while this one blocks.
    let (tx, rx) = mpsc::channel::<Dropped>();
    glib::MainContext::default().invoke(move || {
        begin(plain, held, move |dropped| {
            let _ = tx.send(dropped);
        });
    });

    // ⚠ A closed channel means the GTK thread dropped the sender without
    // sending — the widget went away before the drag ended. Reported as
    // cancelled, which is what the producer saw.
    match rx.recv_timeout(LOST_SIGNAL) {
        Ok(dropped) => Ok(dropped),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Dropped::Cancelled),
        Err(mpsc::RecvTimeoutError::Timeout) => Err("the drag never reported back".to_owned()),
    }
}

/// Start the drag on this thread and turn the main loop until it ends.
///
/// ⚠ **A nested main loop, exactly as `gtk_dialog_run` runs one.** Re-entering
/// the loop is how GTK itself offers a blocking call to code that is already on
/// its thread, and it is the only way to honour [`super::Drags::start`]'s
/// synchronous shape without stopping the drag from happening.
fn pump(plain: Vec<String>, stacked: Vec<String>) -> Result<Dropped, String> {
    // ⚠ `Rc<Cell<_>>` and not a channel: everything here is on one thread, and
    // a channel would only add a second way to describe the same handoff.
    let outcome: Rc<Cell<Option<Dropped>>> = Rc::new(Cell::new(None));
    begin(plain, stacked, {
        let outcome = Rc::clone(&outcome);
        move |dropped| outcome.set(Some(dropped))
    });

    let ticker = glib::timeout_add_local(TICK, || glib::ControlFlow::Continue);
    let deadline = Instant::now() + LOST_SIGNAL;
    let dropped = loop {
        if let Some(dropped) = outcome.get() {
            break Some(dropped);
        }
        if Instant::now() >= deadline {
            break None;
        }
        gtk::main_iteration();
    };
    ticker.remove();

    dropped.ok_or_else(|| "the drag never reported back".to_owned())
}

/// `file://` URIs, which is what `text/uri-list` carries.
///
/// ⚠ **Not bare paths.** A drop target reading `text/uri-list` expects URIs and
/// a path with a space in it is not one — `%20` is the difference between a
/// loop landing on a track and nothing happening, and this app's stem names
/// contain spaces by design (`trap - Snare - 140 BPM - C# Minor`).
///
/// ⛔ **A failure is returned, not swallowed.** This used to `unwrap_or_default`
/// a `Result<Vec<_>, _>`, which short-circuits on the first bad path and yields
/// an **empty** list — so one unconvertible name turned the whole gesture into a
/// drag that offered nothing, silently, and looked to the producer like their
/// DAW ignoring the drop.
fn uris(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| {
            glib::filename_to_uri(path, None)
                .map(|uri| uri.to_string())
                .map_err(|error| format!("{} cannot be offered: {error}", path.display()))
        })
        .collect()
}

/// Start the drag. **Runs on the GTK thread.**
fn begin(plain: Vec<String>, stacked: Vec<String>, done: impl Fn(Dropped) + 'static) {
    // ⛔ **Our own widget, realized, rather than the webview's.** See the module
    // header: reaching the webview's would mean a public accessor on the
    // vendored crate. `Invisible` has a `GdkWindow` of its own once shown,
    // which is all `drag_begin` requires.
    let source = gtk::Invisible::new();
    source.show();

    let targets = gtk::TargetList::new(&[gtk::TargetEntry::new(
        URI_LIST,
        gtk::TargetFlags::OTHER_APP,
        0,
    )]);

    // ⛔⛔ **The modifier is read when the target ASKS for the data, not when
    // the drag starts.** Mike, 2026-08-06: *"press and hold ctrl either before
    // or during the drag."* `drag-data-get` fires at the moment of the drop, so
    // the state read here is the state at the drop — which is exactly what the
    // Windows side achieves by swapping the payload from inside
    // `QueryContinueDrag`, by a different route.
    // ⛔⛔ **Whether anybody ever ASKED for the files, which decides whether they
    // may be deleted.** `Drags::start` removes the spooled folder on
    // [`Dropped::Cancelled`] and keeps it on [`Dropped::Refused`], and `drag.rs`
    // spells out why: "one of them may have taken the files and the other cannot
    // have… Collapsing them is what made a successful drop delete the clip it
    // had just handed over." `drag-data-get` firing is precisely the evidence
    // that a target had the data — without this flag every non-COPY end looked
    // like a cancel, and a Linux DAW referencing a dropped `.wav` by path would
    // find it deleted underneath the clip.
    let asked = Rc::new(Cell::new(false));

    source.connect_drag_data_get({
        let asked = Rc::clone(&asked);
        move |_, _, selection, _, _| {
            asked.set(true);
            let ctrl = gtk::current_event_state()
                .is_some_and(|state| state.contains(gdk::ModifierType::CONTROL_MASK));
            let chosen = if ctrl && !stacked.is_empty() {
                &stacked
            } else {
                &plain
            };
            let borrowed: Vec<&str> = chosen.iter().map(String::as_str).collect();
            selection.set_uris(&borrowed);
        }
    });

    // ⚠ **`drag-failed` fires before `drag-end` when nothing took it**, and both
    // arrive — so the outcome is decided in one place, at the end, from the
    // action the target settled on. Sending from both would race.
    source.connect_drag_end(move |widget, context| {
        let dropped = if context.selected_action().contains(gdk::DragAction::COPY) {
            // A target claimed the copy and said so.
            Dropped::Copied
        } else if asked.get() {
            // It read the data and then settled on no action — it may still
            // have taken the bytes, so the folder stays.
            Dropped::Refused
        } else {
            // Nothing ever asked. These files were seen by no one.
            Dropped::Cancelled
        };
        done(dropped);
        // The widget existed only to own this drag.
        unsafe { widget.destroy() };
    });

    // ⚠ `None` for the event: GTK then uses the current event's time and the
    // pointer's own device, which is the press that started this gesture.
    // `-1, -1` asks it to take the hotspot from that pointer position.
    source.drag_begin_with_coordinates(&targets, gdk::DragAction::COPY, 1, None, -1, -1);
}
