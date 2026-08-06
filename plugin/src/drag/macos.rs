//! The macOS drag source (TASK-063C / FMM-S03).
//!
//! `NSDraggingSession` carrying file URLs on the pasteboard — the same thing a
//! file dragged out of Finder carries, so it is what every macOS DAW's drop
//! target has already had to handle. That is the argument [`super`] makes for
//! `CF_HDROP` on Windows and `text/uri-list` on Linux, a third time.
//!
//! ## ⛔⛔ THIS HAS NEVER RUN. Read before trusting it.
//!
//! There is no Apple SDK on the machine this was written on, so unlike the
//! Windows and Linux sources **this has never been dropped into a DAW**. It is
//! written to be iterated on by CI's macOS runner (see the `plugin (macos)`
//! job) and then by a human with a Mac.
//!
//! ✅ **It does now type-check against the real bindings**, which it did not
//! when it was first written. `cargo check --target aarch64-apple-darwin`
//! compiles Rust for macOS without an Apple toolchain — only *linking* needs
//! one — so the objc2 surface here is checked by a compiler rather than by
//! eye. `docs/runbooks/macos-typecheck.md` is how to run it from Windows, and
//! it is worth running before every push that touches this file: the first
//! version of it had four hard errors and two `-D warnings` failures, and CI's
//! macOS runner would have been the first thing to notice.
//!
//! ⚠ **What that does NOT prove is behaviour.** Nothing here has spoken to a
//! window server. It is switched ON anyway — see `SUPPORTED` below: Mike has
//! testers on real Macs, and a flag that hides the handle would leave them with
//! nothing to try.
//!
//! ## Where the view comes from, and why nothing had to be plumbed
//!
//! A dragging session is begun *on an `NSView`* — unlike `DoDragDrop`, which
//! needs no window at all. Rather than plumb the editor's view down here (which
//! would mean a public accessor on the vendored adapter, exactly what the Linux
//! source avoided), this asks AppKit: the key window's `contentView` is the
//! editor, because the producer just pressed the mouse in it. That is the same
//! move `restore_editor` makes on Windows with `GetFocus`/`GetAncestor` — take
//! it from the OS, which already knows.
//!
//! ## ⛔ The main thread
//!
//! AppKit must be used from the main thread. The RPC handler runs on the host's
//! editor thread, which in a hosted plugin *is* the main thread — that is why
//! the editor can be built there at all — but it is asserted rather than
//! assumed: [`MainThreadMarker::new`] returns `None` anywhere else and the drag
//! is refused rather than corrupting AppKit's state.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSPasteboardWriting,
};
use objc2_foundation::{
    NSArray, NSDate, NSDefaultRunLoopMode, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop,
    NSSize, NSString, NSURL,
};

use super::{Dropped, Preview};

/// ⛔⛔ **ON, and it has never been run — that combination is deliberate.**
///
/// The usual rule in this project is that a handle which drops nothing is worse
/// than no handle, and it is why Linux and macOS drew nothing for months. It
/// does not apply here, and Mike said why on 2026-08-06: he has testers on real
/// Macs waiting to try this. With the flag off they would see no drag handle at
/// all, so there would be nothing to report on — the caution would prevent the
/// very thing that resolves it.
///
/// ⚠ So the honest warning lives in the handoff and the release notes rather
/// than in a switch that makes the feature untestable. Releases are suspended
/// until v1.0.0 (TASK-075), so no unsuspecting end user meets this first.
pub const SUPPORTED: bool = true;

/// ✅ **Yes, once `SUPPORTED` is.** `own_message_queue()` is a no-op here and a
/// dragging session is driven by AppKit's run loop, not by re-entering a window
/// procedure — so the standalone is no different from a host.
pub const STANDALONE_SAFE: bool = true;

/// How long to wait for a session that never reports back.
///
/// ⚠ A backstop for a lost callback, not a limit on the gesture — the same
/// reasoning as the Linux source's, and for the same reason: the thread that
/// blocks here is the one a DAW draws its editor from.
const LOST_CALLBACK: Duration = Duration::from_secs(600);

/// How long each turn of the nested run loop waits before coming back.
///
/// ⚠ **Not a poll interval.** [`NSRunLoop::runMode_beforeDate`] returns as soon
/// as it has handled an event, so in a live drag it comes back constantly; this
/// only bounds how long it sits when *nothing at all* is happening, which is
/// what makes [`LOST_CALLBACK`] reachable rather than decorative.
const TICK: f64 = 0.1;

/// What the drag source object carries while a session is running.
struct DragState {
    /// Where the outcome goes when the session ends.
    ///
    /// ⚠ `Rc<Cell<_>>` rather than a channel: an `NSDraggingSource` is only ever
    /// spoken to on the main thread — the class is declared `MainThreadOnly`
    /// below, so the compiler holds that — and [`drag`] waits on the same
    /// thread. A channel would be a second mechanism describing one handoff.
    outcome: Rc<Cell<Option<Dropped>>>,
}

define_class!(
    /// The `NSDraggingSource` a session needs.
    ///
    /// ⚠ **A real Objective-C class rather than a closure**, because AppKit
    /// asks the *source* two questions during a drag and there is nowhere else
    /// to answer them from: what operations are allowed, and — the one this
    /// exists for — when the session ended and with what result.
    ///
    /// ⛔ **`MainThreadOnly` is required, not decorative.** `NSDraggingSource`
    /// is declared `pub unsafe trait NSDraggingSource: NSObjectProtocol +
    /// MainThreadOnly`, and a defined class otherwise inherits its superclass's
    /// thread kind — `NSObject`'s is `AnyThread`. Without this the `unsafe impl`
    /// below fails its bound and `Self::alloc(mtm)` resolves to the zero-argument
    /// `AnyThread` overload: two hard errors, and the reason this file did not
    /// compile when it was first written.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FreallyDragSource"]
    #[ivars = DragState]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        /// ⛔ **Copy only, exactly as the Windows source offers only
        /// `DROPEFFECT_COPY`.** A move would invite the target to tell us to
        /// delete the source afterwards, and the source is the producer's
        /// spooled loop — which their DAW may still be referencing by path.
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }

        /// ⛔ **`Copied` only when a target actually took it.** The distinction
        /// decides whether `Drags::start` deletes the spooled folder, and for
        /// audio it must not delete one a DAW is referencing — the same rule
        /// the Windows source spells out at `DRAGDROP_S_DROP`.
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _point: NSPoint, operation: NSDragOperation) {
            // ⚠ **No `Refused` leg here, unlike the Linux source, and that is
            // the API's own shape rather than an omission.** `operation` is what
            // the target settled on; there is no separate signal saying it read
            // the pasteboard, because an `NSURL` item writes eagerly at the
            // start of the session and any target may have looked. `None` means
            // nothing accepted the drop, which is a cancel.
            let dropped = if operation.contains(NSDragOperation::Copy) {
                Dropped::Copied
            } else {
                Dropped::Cancelled
            };
            self.ivars().outcome.set(Some(dropped));
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker, outcome: Rc<Cell<Option<Dropped>>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DragState { outcome });
        unsafe { msg_send![super(this), init] }
    }
}

/// Hand `paths` to the window server and block until the producer lets go.
///
/// `stacked` is the alternative set to offer **while Command is held**.
///
/// ⚠ **Command, not Control** — and that is a deliberate divergence from the
/// other two platforms rather than an oversight. Ctrl-click *is* right-click on
/// macOS, so binding the stacked layout to Ctrl would fire it every time
/// somebody tried to open a context menu mid-drag. Command is the modifier
/// macOS uses for the equivalent job everywhere else.
pub fn drag(
    paths: &[PathBuf],
    stacked: &[PathBuf],
    _preview: Option<&Preview>,
) -> Result<Dropped, String> {
    if paths.is_empty() {
        return Err("there are no files to drag".to_owned());
    }
    // ⛔ Not the main thread means AppKit is not ours to touch. Refused rather
    // than dispatched: a drag that began after the gesture it belongs to would
    // land wherever the cursor had wandered to.
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("a drag can only start on the main thread".to_owned());
    };

    let app = NSApplication::sharedApplication(mtm);
    // ⚠ The window the producer just pressed in. See the header for why this is
    // asked of AppKit rather than plumbed down from the editor.
    let Some(view) = app.keyWindow().and_then(|window| window.contentView()) else {
        return Err("there is no editor window to drag from".to_owned());
    };
    let Some(event) = app.currentEvent() else {
        return Err("there is no mouse event to attach the drag to".to_owned());
    };

    // ⛔⛔ **The modifier is read HERE, when the drag begins, and that is a
    // known difference from the other two platforms.** Windows swaps the
    // payload from inside `QueryContinueDrag` and Linux reads it in
    // `drag-data-get`, so both honour Mike's *"before or during"*. A dragging
    // session's items are fixed when it starts, so this reads the state at the
    // press only.
    // ⚠ TO FINISH: `draggingSession:movedToPoint:` can observe the modifier
    // mid-drag, but the pasteboard is already written by then — matching the
    // other two needs promise-based items, which is a larger change. Written
    // down rather than left as a silent difference in behaviour.
    let held = NSEvent::modifierFlags(&event);
    let command = held.0 & (1 << 20) != 0;
    let chosen = if command && !stacked.is_empty() {
        stacked
    } else {
        paths
    };

    let items = dragging_items(chosen);
    if items.is_empty() {
        return Err("none of those paths could be offered".to_owned());
    }

    let outcome: Rc<Cell<Option<Dropped>>> = Rc::new(Cell::new(None));
    let source = DragSource::new(mtm, Rc::clone(&outcome));

    let protocol: &ProtocolObject<dyn NSDraggingSource> = ProtocolObject::from_ref(&*source);
    let _session = view.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&items),
        &event,
        protocol,
    );

    // ⛔⛔ **The session is asynchronous and this call is not — and the first cut
    // bridged that with a blocking channel, which was a deadlock.** AppKit
    // drives a dragging session from *this thread's run loop*, and
    // `MainThreadMarker::new()` above has already established that this is that
    // thread. So parking it on `recv_timeout` stopped the very loop that had to
    // deliver `draggingSession:endedAtPoint:operation:`: the DAW's whole UI
    // froze for ten minutes, nothing was ever dropped, and the spooled folder
    // was then deleted. The comment there claimed both things at once — "AppKit
    // keeps running the drag on this thread's run loop, so the block has to be
    // brief" — over a block of 600 seconds.
    //
    // ▶ **A nested run loop instead**, which is what AppKit itself does for a
    // modal session: keep turning the loop so the drag runs, and come back when
    // the source says it ended.
    //
    // ⚠ **Correct whether or not `beginDraggingSession` blocks internally.**
    // Which of the two it does is not something this machine can establish, and
    // it does not have to: if it already ran the drag to completion then
    // `outcome` is set before the first turn and this returns immediately.
    let run_loop = NSRunLoop::currentRunLoop();
    let deadline = Instant::now() + LOST_CALLBACK;
    loop {
        if let Some(dropped) = outcome.get() {
            return Ok(dropped);
        }
        if Instant::now() >= deadline {
            return Err("the drag never reported back".to_owned());
        }
        let until = NSDate::dateWithTimeIntervalSinceNow(TICK);
        // ⚠ The `unsafe` is for `NSDefaultRunLoopMode` being an extern static,
        // not for the call — turning the loop is safe. ⚠ The *default* mode and
        // not event-tracking: the window server's event source belongs to the
        // common modes, so either delivers the drag, and this is the mode the
        // host's own loop would have been in had it not been re-entered here.
        let mode = unsafe { NSDefaultRunLoopMode };
        run_loop.runMode_beforeDate(mode, &until);
    }
}

/// One `NSDraggingItem` per file, each writing its own URL to the pasteboard.
///
/// ⚠ `NSURL` conforms to `NSPasteboardWriting`, so no promise and no delegate:
/// the bytes are already on disk by the time this runs — which is the same
/// decision, for the same reason, that [`super`] records for `CF_HDROP`.
fn dragging_items(paths: &[PathBuf]) -> Vec<Retained<NSDraggingItem>> {
    paths
        .iter()
        .filter_map(|path| {
            let text = NSString::from_str(path.to_str()?);
            let url = NSURL::fileURLWithPath(&text);
            // ⚠ **Through the protocol, not the class.** `initWithPasteboardWriter`
            // takes `&ProtocolObject<dyn NSPasteboardWriting>`; handing it the
            // `NSURL` directly is a type error, and it was one of the four that
            // stopped this file compiling at all.
            let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*url);
            let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
            // ⚠ A frame is required or the item has nowhere to draw. The
            // drag image itself is the page's job on the other platforms and
            // is not wired here yet — see `Preview`, which this ignores.
            unsafe {
                item.setDraggingFrame_contents(
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0)),
                    None::<&AnyObject>,
                );
            }
            Some(item)
        })
        .collect()
}
