//! The Linux editor: one GTK thread for the whole process.
//!
//! **Not upstream.** Added for TASK-P12 — `VENDORED.md` records why, and this
//! file is the whole of it, so a future rebase is a diff against `lib.rs` plus
//! one new module rather than a rewrite.
//!
//! ## Why a dedicated thread rather than `gtk::init()` where the window opens
//!
//! wry's WebKitGTK backend reaches for `gdk::Display::default()`, which is
//! `None` until somebody has called `gtk_init`. A plugin cannot require that of
//! its host — Reaper and Bitwig are not GTK applications — so the editor has to
//! initialise GTK itself.
//!
//! The obvious place is baseview's window thread, and it is wrong. **baseview
//! spawns a fresh thread per editor**, and gtk-rs *panics* on the second
//! `init()` from a different thread:
//!
//! ```text
//! panicked at 'Attempted to initialize GTK from two different threads.'
//! ```
//!
//! A panic in a plugin takes the DAW down. That is not an exotic path either:
//! closing the plugin window and reopening it is a new baseview thread, and so
//! is a second instance on a second track. It would have passed a single-window
//! screenshot test and destroyed a real session — the exact shape of failure
//! this project already has five gates written about.
//!
//! So GTK is initialised exactly once, on a thread this module owns, and every
//! webview lives there for as long as it exists. Other threads address a
//! webview by id and post work to it; nothing outside this file touches a GTK
//! object. gtk-rs asserts that on every call, so the rule is enforced by the
//! binding rather than by us remembering it.
//!
//! ## What that buys beyond not crashing
//!
//! The GTK thread runs a real `gtk::main()`, so WebKit paints and answers
//! `/__rpc` on its own schedule instead of waiting for a frame tick. That is the
//! opposite failure to the one Ableton produced on Windows and macOS, where the
//! bridge had to stop depending on `on_frame` because a hosted window never got
//! one.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use gtk::glib;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle, XlibWindowHandle};
use wry::WebView;

thread_local! {
    /// Every open editor's webview — on the one thread allowed to touch them.
    ///
    /// A `thread_local` rather than a `Mutex<HashMap>` on purpose: `WebView` is
    /// not `Send`, and this is the type system agreeing with GTK's own rule
    /// rather than a lock that would let the rule be broken quietly.
    static WEBVIEWS: RefCell<HashMap<u64, WebView>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static GTK: OnceLock<bool> = OnceLock::new();

/// Start the process's GTK thread, once, and answer whether it is usable.
///
/// ⛔ **Waiting for it is the whole point, and skipping the wait is not a
/// slower start — it is a crash.** `g_main_context_invoke` does not simply
/// queue: if the calling thread can *acquire* the context, it runs the job
/// inline, right there. So posting work before the GTK thread has claimed the
/// default context runs that work on baseview's thread instead, where gtk-rs
/// was never initialised, and the first GTK call aborts the process:
///
/// ```text
/// panicked at 'GTK has not been initialized. Call `gtk::init` first.'
///   webkit2gtk::ApplicationInfo::new
///   wry::webkitgtk::web_context::WebContextImpl::new
/// ```
///
/// That is not hypothetical — it is exactly what the first Xvfb run did, and
/// the two symptoms pointed away from the cause: `gtk_init` reported "Failed to
/// acquire default main context" because the *other* thread had just taken it.
///
/// Once `gtk::init()` returns Ok the GTK thread holds the default context for
/// good (gtk-rs acquires and deliberately leaks it), so from that moment no
/// other thread can acquire it and every later `invoke` genuinely queues.
fn gtk_ready() -> bool {
    *GTK.get_or_init(|| {
        let (announce, wait) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("freally-gtk".to_owned())
            .spawn(move || match gtk::init() {
                Ok(()) => {
                    let _ = announce.send(true);
                    gtk::main();
                }
                Err(error) => {
                    // No display, or a host whose own main thread already owns
                    // the default main context — see the module comment. Logged
                    // rather than panicked: the host's process is not ours to
                    // end, and no editor is a better outcome than no DAW.
                    eprintln!("nih_plug_webview: gtk_init failed, no editor can open: {error}");
                    let _ = announce.send(false);
                }
            });

        match thread {
            Ok(_) => wait.recv().unwrap_or(false),
            Err(error) => {
                eprintln!("nih_plug_webview: could not start the GTK thread: {error}");
                false
            }
        }
    })
}

/// Run `job` on the GTK thread.
///
/// Jobs run in the order they are posted, which is what lets a webview be
/// addressed by id immediately after the job that creates it has been queued
/// but before it has run.
///
/// When GTK is unavailable the job is **dropped rather than run**: `invoke`
/// would fall back to running it on this thread, and every GTK call inside it
/// would take the host down.
pub(crate) fn on_gtk<F: FnOnce() + Send + 'static>(job: F) {
    if !gtk_ready() {
        return;
    }
    glib::MainContext::default().invoke(job);
}

/// Hand a freshly built webview to the registry. Called on the GTK thread.
pub(crate) fn store(id: u64, webview: WebView) {
    WEBVIEWS.with(|open| {
        open.borrow_mut().insert(id, webview);
    });
}

fn with_webview(id: u64, act: impl FnOnce(&WebView)) {
    WEBVIEWS.with(|open| {
        if let Some(webview) = open.borrow().get(&id) {
            act(webview);
        }
    });
}

/// A webview that lives on the GTK thread, addressed from anywhere else.
///
/// The methods mirror the parts of [`wry::WebView`]'s surface that
/// [`crate::WindowHandler`] uses, so the call sites are identical on every
/// platform and `lib.rs` needs no `cfg` around them.
pub(crate) struct WebViewHandle {
    id: u64,
}

impl WebViewHandle {
    pub(crate) fn new() -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn set_bounds(&self, bounds: wry::Rect) {
        let id = self.id;
        on_gtk(move || with_webview(id, |webview| webview.set_bounds(bounds)));
    }

    /// **Posts the script; it has not run when this returns.**
    ///
    /// wry's own signature is fallible and synchronous. Crossing a thread makes
    /// it neither, and `Ok` here means "queued" rather than "evaluated". Nothing
    /// in this project reads the result — the bridge is a request/response over
    /// the custom protocol, not `evaluate_script` — so the distinction costs
    /// nothing today and is written down rather than discovered later.
    pub(crate) fn evaluate_script(&self, script: &str) -> wry::Result<()> {
        let id = self.id;
        let script = script.to_owned();
        on_gtk(move || {
            with_webview(id, |webview| {
                if let Err(error) = webview.evaluate_script(&script) {
                    eprintln!("nih_plug_webview: evaluate_script failed: {error}");
                }
            })
        });
        Ok(())
    }
}

impl Drop for WebViewHandle {
    /// Drop the webview **on the GTK thread**.
    ///
    /// The handle is dropped on baseview's thread when the host closes the
    /// editor. Dropping a `WebView` destroys GTK widgets, so the value itself
    /// has to go home to do it.
    fn drop(&mut self) {
        let id = self.id;
        on_gtk(move || {
            WEBVIEWS.with(|open| open.borrow_mut().remove(&id));
        });
    }
}

/// The parent window, as wry wants to receive it.
///
/// wry takes a `HasRawWindowHandle` rather than a bare id, and by the time the
/// creation job runs on the GTK thread the baseview window it came from is not
/// reachable — so the id travels alone and arrives wearing the trait again.
pub(crate) struct ParentXid(pub u64);

// SAFETY: the contract is that the handle describes a valid window of the
// declared kind. `window` is an X11 window id taken from the baseview window
// this editor was parented into, which outlives the webview built under it —
// `WindowHandler` owns the handle and is dropped with that window.
unsafe impl HasRawWindowHandle for ParentXid {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = XlibWindowHandle::empty();
        handle.window = self.0;
        RawWindowHandle::Xlib(handle)
    }
}
