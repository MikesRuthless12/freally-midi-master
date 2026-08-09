use baseview::{Event, Size, Window, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use nih_plug::prelude::{Editor, GuiContext, ParamSetter};
use serde_json::Value;
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};
#[cfg(not(target_os = "linux"))]
use wry::WebView;
use wry::{
    http::{Request, Response},
    WebContext, WebViewBuilder,
};

use crossbeam::channel::{unbounded, Receiver, Sender};

// The Linux editor, which is not upstream. GTK may only be initialised once per
// process and only ever touched from the thread that did it, so the webview
// cannot live on baseview's per-window thread the way it does elsewhere. See
// the module for the panic that rule exists to prevent.
#[cfg(target_os = "linux")]
mod linux;

// Only Linux reads the parent handle itself; elsewhere it is passed straight to
// baseview and wry without ever being matched on.
#[cfg(target_os = "linux")]
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

pub use wry::http;

pub use baseview::{DropData, DropEffect, EventStatus, MouseEvent};
pub use keyboard_types::*;

type EventLoopHandler = dyn Fn(&WindowHandler, ParamSetter, &mut Window) + Send + Sync;
type KeyboardHandler = dyn Fn(KeyboardEvent) -> bool + Send + Sync;
type MouseHandler = dyn Fn(MouseEvent) -> EventStatus + Send + Sync;
type CustomProtocolHandler =
    dyn Fn(&Request<Vec<u8>>) -> wry::Result<Response<Cow<'static, [u8]>>> + Send + Sync;

pub struct WebViewEditor {
    source: Arc<HTMLSource>,
    width: Arc<AtomicU32>,
    height: Arc<AtomicU32>,
    event_loop_handler: Arc<EventLoopHandler>,
    keyboard_handler: Arc<KeyboardHandler>,
    mouse_handler: Arc<MouseHandler>,
    custom_protocol: Option<(String, Arc<CustomProtocolHandler>)>,
    developer_mode: bool,
    background_color: (u8, u8, u8, u8),
}

pub enum HTMLSource {
    String(&'static str),
    URL(&'static str),
}

impl WebViewEditor {
    pub fn new(source: HTMLSource, size: (u32, u32)) -> Self {
        let width = Arc::new(AtomicU32::new(size.0));
        let height = Arc::new(AtomicU32::new(size.1));
        Self {
            source: Arc::new(source),
            width,
            height,
            developer_mode: false,
            background_color: (255, 255, 255, 255),
            event_loop_handler: Arc::new(|_, _, _| {}),
            keyboard_handler: Arc::new(|_| false),
            mouse_handler: Arc::new(|_| EventStatus::Ignored),
            custom_protocol: None,
        }
    }

    pub fn with_background_color(mut self, background_color: (u8, u8, u8, u8)) -> Self {
        self.background_color = background_color;
        self
    }

    pub fn with_custom_protocol<F>(mut self, name: String, handler: F) -> Self
    where
        F: Fn(&Request<Vec<u8>>) -> wry::Result<Response<Cow<'static, [u8]>>>
            + 'static
            + Send
            + Sync,
    {
        self.custom_protocol = Some((name, Arc::new(handler)));
        self
    }

    pub fn with_event_loop<F>(mut self, handler: F) -> Self
    where
        F: Fn(&WindowHandler, ParamSetter, &mut baseview::Window) + 'static + Send + Sync,
    {
        self.event_loop_handler = Arc::new(handler);
        self
    }

    pub fn with_developer_mode(mut self, mode: bool) -> Self {
        self.developer_mode = mode;
        self
    }

    pub fn with_keyboard_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(KeyboardEvent) -> bool + Send + Sync + 'static,
    {
        self.keyboard_handler = Arc::new(handler);
        self
    }

    pub fn with_mouse_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(MouseEvent) -> EventStatus + Send + Sync + 'static,
    {
        self.mouse_handler = Arc::new(handler);
        self
    }
}

pub struct WindowHandler {
    context: Arc<dyn GuiContext>,
    event_loop_handler: Arc<EventLoopHandler>,
    keyboard_handler: Arc<KeyboardHandler>,
    mouse_handler: Arc<MouseHandler>,
    // Elsewhere the webview is owned right here, on baseview's window thread.
    // On Linux it lives on the process's one GTK thread and this is its address
    // — same method names, so nothing below has to know which it is holding.
    #[cfg(not(target_os = "linux"))]
    webview: Option<WebView>,
    #[cfg(target_os = "linux")]
    webview: linux::WebViewHandle,
    events_receiver: Receiver<Value>,
    pub width: Arc<AtomicU32>,
    pub height: Arc<AtomicU32>,
}

impl WindowHandler {
    /// The webview, if this editor has one.
    ///
    /// ⛔ **`None` is a real state on Windows and macOS, not a placeholder.**
    /// When the webview cannot be constructed — no WebView2 runtime is the
    /// common case — `spawn` logs and carries on rather than panicking inside
    /// the host's editor-open callback. Everything below then quietly does
    /// nothing, which is a plugin with a blank editor rather than a DAW that
    /// has just closed on unsaved work.
    ///
    /// Linux always answers `Some`: its handle is an id, and a webview that
    /// failed to build is simply absent from the registry the id addresses.
    #[cfg(not(target_os = "linux"))]
    fn webview(&self) -> Option<&WebView> {
        self.webview.as_ref()
    }

    #[cfg(target_os = "linux")]
    fn webview(&self) -> Option<&linux::WebViewHandle> {
        Some(&self.webview)
    }

    pub fn resize(&self, window: &mut baseview::Window, width: u32, height: u32) {
        if let Some(webview) = self.webview() {
            webview.set_bounds(wry::Rect {
                x: 0,
                y: 0,
                width,
                height,
            });
        }
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);
        self.context.request_resize();
        window.resize(Size {
            width: width as f64,
            height: height as f64,
        });
    }

    pub fn send_json(&self, json: Value) {
        let Some(webview) = self.webview() else {
            return;
        };
        let json_str = json.to_string();
        let json_str_quoted =
            serde_json::to_string(&json_str).expect("Should not fail: the value is always string");
        // Upstream unwrapped. This is called from the editor's frame handler on
        // the host's UI thread, so a failed script is not grounds to end the
        // host's process — and a page that has navigated away or is being torn
        // down is a perfectly ordinary reason for one to fail.
        if let Err(error) =
            webview.evaluate_script(&format!("onPluginMessageInternal({});", json_str_quoted))
        {
            eprintln!("nih_plug_webview: could not deliver a message to the page: {error}");
        }
    }

    pub fn next_event(&self) -> Result<Value, crossbeam::channel::TryRecvError> {
        self.events_receiver.try_recv()
    }
}

/// Declare that this process owns its own Windows message queue (TASK-P16).
///
/// ⛔ **Only a standalone application may call this, and it must never be called
/// from a plugin.** Inside Ableton or FL the *host* owns the thread's message
/// queue; draining it from our frame handler would steal the host's messages and
/// break it — the "takes the DAW down" failure class this project has already
/// been bitten by. The flag therefore defaults to **off**, and a DAW has no code
/// path that turns it on: `plugin/src/bin/standalone.rs` is the only caller, and
/// a host never runs that binary's `main`.
///
/// It is deliberately *not* gated on "the host sends no frame ticks". That is a
/// quirk of one host, not a statement about who owns the queue.
///
/// # Why this is needed at all
///
/// `baseview`'s `open_blocking` pumps with `GetMessageW(&mut msg, hwnd, 0, 0)` —
/// a **non-NULL** `hwnd`, which retrieves only messages for that window and its
/// children and **never retrieves thread messages at all**. WebView2 is COM/STA
/// and delivers its async completions through a COM-owned message-only window
/// plus posted thread messages, neither of which is our window. So the loop spins
/// at 60 fps while every WebView2 callback sits unretrieved: the custom-protocol
/// handler is never dispatched, navigation never completes, and the window stays
/// on `about:blank`. `VENDORED.md` and `HANDOFF.md` carry the full diagnosis.
pub fn own_message_queue() {
    #[cfg(target_os = "windows")]
    windows_pump::enable();
}

/// The editor window of a process that pumps its own queue, or null.
///
/// ⛔ **The honest test for "am I the standalone?" from below the seam**, and
/// the handle is one the caller can trust: `on_frame` hands it over every frame.
/// `plugin/src/drag/windows.rs` needs it because `GetFocus()` answers `NULL`
/// when the focused window belongs to another thread's queue — which is what
/// WebView2 arranges — and a drag that cannot name our window cannot put it back
/// in front afterwards.
///
/// ⚠ Per thread, and only non-null on the editor thread. That is the only
/// thread allowed to ask, since it is the one a drag runs on.
#[cfg(target_os = "windows")]
pub fn own_queue_editor_window() -> *mut std::ffi::c_void {
    windows_pump::editor_window()
}

/// The pump itself. Windows-only, and inert unless [`own_message_queue`] ran.
#[cfg(target_os = "windows")]
mod windows_pump {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Declared rather than pulled in as a dependency, matching what
    // `plugin/src/editor.rs` does for `GetDpiForSystem`: three calls do not
    // justify the `windows` crate, and `user32` is already linked by the window
    // this adapter opens.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn IsChild(parent: *mut c_void, child: *mut c_void) -> i32;
        fn PostQuitMessage(exit_code: i32);
        fn RegisterClassW(class: *const WndClassW) -> u16;
        #[allow(clippy::too_many_arguments)]
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        fn DefWindowProcW(hwnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> isize;
        fn PostMessageW(hwnd: *mut c_void, msg: u32, w_param: usize, l_param: isize) -> i32;
        fn GetParent(hwnd: *mut c_void) -> *mut c_void;
        fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, new_long: isize) -> isize;
        fn GetClientRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
        fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex_style: u32) -> i32;
        fn SetWindowPos(
            hwnd: *mut c_void,
            insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn RevokeDragDrop(hwnd: *mut c_void) -> i32;
    }

    /// Win32 `WNDCLASSW`. `repr(C)` for the same reason [`Msg`] is.
    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    /// Win32 `RECT`.
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    /// Win32 `MSG`. `repr(C)` so the padding matches what `PeekMessageW` writes.
    #[repr(C)]
    struct Msg {
        hwnd: *mut c_void,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
    }

    const PM_NOREMOVE: u32 = 0x0000;
    const PM_REMOVE: u32 = 0x0001;

    static ENABLED: AtomicBool = AtomicBool::new(false);

    pub fn enable() {
        ENABLED.store(true, Ordering::Relaxed);
    }

    thread_local! {
        /// Belt to the braces below: nothing dispatched here should re-enter
        /// this function, and if a future message type ever does, it stops
        /// rather than growing the stack until it does not.
        static PUMPING: Cell<bool> = const { Cell::new(false) };
    }

    /// Resets the guard even if a dispatched message panics through us.
    struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            PUMPING.with(|p| p.set(false));
        }
    }

    /// Our own message id, on our own window class, so it collides with nothing.
    const WM_PUMP: u32 = 0x0400 + 42; // WM_USER + 42
    const WS_CHILD: u32 = 0x4000_0000;

    thread_local! {
        /// The window [`request`] posts to, and whose procedure runs [`drain`].
        /// Created once, on the thread that owns the queue.
        static PUMP_WINDOW: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
        /// The editor handle [`drain`] needs, handed over by [`request`] because
        /// a window procedure is given no context of its own.
        static PUMP_EDITOR: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
        /// One posted `WM_PUMP` in flight at a time. `on_frame` runs at frame
        /// rate and would otherwise queue a pump per frame faster than they are
        /// consumed, which is a queue that only grows.
        static PUMP_POSTED: Cell<bool> = const { Cell::new(false) };
    }

    /// ⛔⛔ **THE WHOLE POINT OF THIS WINDOW, AND IT IS NOT AN OPTIMISATION.**
    ///
    /// `on_frame` is called from *inside* `baseview`'s window procedure, which is
    /// holding a `RefCell` borrow on its own window state for the duration. Any
    /// message dispatched from there re-enters that procedure and panics with
    /// `RefCell already borrowed`, inside an `extern "system"` frame where the
    /// panic cannot unwind — **the process aborts**. That is why [`drain`] has to
    /// leave the editor's own messages alone, and it is why a drag started from
    /// the standalone used to abort: `DoDragDrop` runs a modal loop that
    /// dispatches a `WM_TIMER` straight back into that procedure.
    ///
    /// ▶ **The escape is that `GetMessageW(&mut msg, hwnd, 0, 0)` retrieves
    /// messages for `hwnd` AND ITS CHILDREN.** `baseview`'s `open_blocking` loop
    /// is *outside* the window procedure, so anything it dispatches runs with no
    /// borrow live. So the pump moves onto **a child window of the editor**:
    /// `on_frame` only posts (which is safe inside the procedure), `open_blocking`
    /// retrieves the post because the window is in the editor's subtree, and
    /// [`drain`] then runs from [`pump_proc`] one stack frame below the loop
    /// rather than three frames inside baseview's procedure.
    ///
    /// ⚠ **A message-only window (`HWND_MESSAGE`) does NOT work here**, and it is
    /// the obvious thing to reach for. It is not in the editor's subtree, so
    /// `open_blocking`'s filtered `GetMessageW` never retrieves it and the post
    /// would sit in the queue forever — or be dispatched by `drain`, which is
    /// exactly the stack being escaped.
    ///
    /// ⛔ Inert unless [`own_message_queue`] ran, so a host never creates this.
    pub fn request(editor: *mut c_void) {
        if !ENABLED.load(Ordering::Relaxed) || editor.is_null() {
            return;
        }
        PUMP_EDITOR.with(|e| e.set(editor));

        let window = PUMP_WINDOW.with(|w| w.get());
        let window = if window.is_null() {
            let created = create_pump_window(editor);
            PUMP_WINDOW.with(|w| w.set(created));
            stop_being_a_drop_target(editor);
            pin_the_frame(editor);
            created
        } else {
            window
        };
        if window.is_null() {
            // ⛔ **Fall back to draining in place rather than not at all.** A
            // standalone that cannot create a child window is a standalone whose
            // webview never renders, which is a blank window — strictly worse
            // than the abort this indirection exists to avoid, and that abort
            // only happens if the producer then starts a drag.
            drain(editor);
            return;
        }

        if PUMP_POSTED.with(|p| p.replace(true)) {
            return;
        }
        // SAFETY: `window` is a live HWND owned by this thread, and `WM_PUMP` is
        // our own id on our own class.
        if unsafe { PostMessageW(window, WM_PUMP, 0, 0) } == 0 {
            PUMP_POSTED.with(|p| p.set(false));
        }
    }

    /// Give up `baseview`'s OLE drop target on our own window.
    ///
    /// ⛔⛔ **A CRASH FIX, AND THE CRASH IS IN `baseview`.** TASK-063D. A drag
    /// begins with the cursor over the window it started from, so `DoDragDrop`
    /// calls `IDropTarget::DragEnter` on **us** first — and `baseview` registers
    /// one on every window it opens (`win/window.rs:764`). Its parser then does
    /// `*(*medium.u).hGlobal()`, which dereferences the `STGMEDIUM` union twice:
    /// that yields the *data pointer* of a movable block where `DragQueryFileW`
    /// wants the *handle*. `GlobalLock` of a data pointer is `NULL` and the
    /// shell faults reading it. Captured stack:
    ///
    /// ```text
    /// DragQueryFileW
    /// baseview::win::drop_target::DropTarget::parse_drop_data  drop_target.rs:140
    /// baseview::win::drop_target::DropTarget::drag_enter       drop_target.rs:209
    /// DoDragDrop
    /// ```
    ///
    /// ⚠ **No allocation choice on the source side fixes it** — with `GMEM_FIXED`
    /// the same double-dereference reads `pFiles` and hands *that* to the shell.
    /// It is `baseview`'s bug, and short of a third fork we cannot repair it.
    ///
    /// ▶ **Here, and not in the drag code, because here the handle is certain.**
    /// This is the window `baseview` registered — the pump was handed it by
    /// `on_frame`. An earlier cut revoked from `drag/windows.rs` using a handle
    /// fetched across the seam, it came back null, and nothing changed.
    ///
    /// ⛔⛔ **THE EDITOR ALONE IS NOT ENOUGH — WALK THE PARENTS.** OLE looks for
    /// a drop target on the window under the cursor and then **up its parent
    /// chain**, and there are *two* `baseview` windows in the standalone:
    /// `nih_plug`'s wrapper opens a top-level one with `open_blocking`, and this
    /// adapter opens the editor as a **child** of it with `open_parented`. Both
    /// get a drop target. Revoking only the editor's was measured to change
    /// nothing: the drag simply found the parent's and faulted there instead,
    /// with a stack identical to the first.
    ///
    /// ⛔⛔ **Only reachable when this process owns its queue**, i.e. the
    /// standalone: [`request`] returns before this if `ENABLED` is false. ⚠ That
    /// guard is what makes walking *upwards* safe — in a host the chain above
    /// our editor is the DAW's own windows, and revoking their drop targets
    /// would break drag-and-drop across the whole application.
    ///
    /// ⚠ Safe to give up: nothing in this application reads dropped files —
    /// `DropData` appears once in the tree, as a `pub use` nobody consumes.
    fn stop_being_a_drop_target(editor: *mut c_void) {
        let mut window = editor;
        // ⚠ Bounded rather than `while !null`. A parent chain is three windows
        // deep here; anything longer means the assumption is wrong, and looping
        // forever inside a frame handler is worse than missing a revoke.
        for _ in 0..8 {
            if window.is_null() {
                return;
            }
            // SAFETY: documented entry points on windows this process owns,
            // called from the thread that created them.
            let hr = unsafe { RevokeDragDrop(window) };
            // ⚠ `DRAGDROP_E_NOTREGISTERED` (0x80040100) is the ordinary answer
            // for the ancestors that never had one, and is not worth reporting.
            if hr == 0 {
                eprintln!("[pump] released baseview's drop target on {window:?}");
            } else if hr != 0x8004_0100u32 as i32 {
                // ⛔ Not silent. A failure here is a crash on the next drag, and
                // a quiet one is what made this look like a drag bug for an hour.
                eprintln!(
                    "[pump] could NOT release a drop target on {window:?}: 0x{hr:08X} \
                     — a drag from the standalone may crash"
                );
            }
            window = unsafe { GetParent(window) };
        }
    }

    /// The editor window this thread pumps for, or null if it does not pump.
    ///
    /// Recorded by [`request`] on every frame, so it is always the handle
    /// `baseview` is actually using. [`super::own_queue_editor_window`] is the
    /// only caller and its doc says what for.
    pub fn editor_window() -> *mut c_void {
        PUMP_EDITOR.with(|e| e.get())
    }

    /// The outermost window above `from`, or `from` if it is already top level.
    fn top_level(from: *mut c_void) -> *mut c_void {
        let mut window = from;
        // ⚠ Bounded for the same reason the revoke walk is: a chain this deep
        // means the assumption is wrong, and spinning inside a frame handler is
        // worse than stopping early.
        for _ in 0..8 {
            // SAFETY: a documented entry point on a window this process owns.
            let parent = unsafe { GetParent(window) };
            if parent.is_null() {
                break;
            }
            window = parent;
        }
        window
    }

    /// Give the standalone a fixed frame: no maximise, no sizing border.
    ///
    /// ⛔⛔ **Mike asked for this by name** (2026-08-06): *"can you ensure that
    /// the maximize/restore button is disabled like you can with C# WinForms?
    /// that would fix it and ensure it is fixed border for the form."* It is the
    /// WinForms `FormBorderStyle = FixedSingle` + `MaximizeBox = false` pair,
    /// spelled in Win32.
    ///
    /// ▶ **Why it is the right fix and not a dodge.** Maximising or dragging the
    /// frame left a black margin down the right and along the bottom: the window
    /// grew and the page did not. An earlier attempt answered
    /// `WindowEvent::Resized` by re-bounding the webview, and Mike reported the
    /// dead space unchanged — so either that event does not arrive here or the
    /// bounds are not the whole story. ⛔ **A window that cannot be resized
    /// cannot disagree with its contents**, which removes the failure rather
    /// than tracking it. The editor already offers real size presets through
    /// `set_editor_size`, and those go through the path that resizes *both*.
    ///
    /// ⛔ **Standalone only.** [`request`] returns before this when the pump is
    /// off, so a host is never touched — restyling a DAW's window would be an
    /// appalling thing to do.
    fn pin_the_frame(editor: *mut c_void) {
        const GWL_STYLE: i32 = -16;
        const WS_THICKFRAME: isize = 0x0004_0000;
        const WS_MAXIMIZEBOX: isize = 0x0001_0000;
        const SWP_NOSIZE: u32 = 0x0001;
        const SWP_NOMOVE: u32 = 0x0002;
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;
        const SWP_FRAMECHANGED: u32 = 0x0020;

        // ⚠ The top-level window, not the editor. The editor is a `WS_CHILD` of
        // nih_plug's wrapper window, and a child has no caption buttons and no
        // sizing border to take away.
        let frame = top_level(editor);
        if frame.is_null() {
            return;
        }

        const GWL_EXSTYLE: i32 = -20;

        // SAFETY: documented entry points on a window this process owns, called
        // from the thread that created it.
        unsafe {
            let style = GetWindowLongPtrW(frame, GWL_STYLE);
            let fixed = style & !(WS_THICKFRAME | WS_MAXIMIZEBOX);
            if fixed == style {
                return;
            }

            // ⛔⛔ **MEASURE THE CLIENT AREA FIRST, AND PUT IT BACK AFTERWARDS.**
            // Dropping `WS_THICKFRAME` makes the non-client border thinner, so
            // the *client* area grows by a few pixels on every edge while the
            // window stays the same size. Nothing tells `baseview`'s child or
            // the webview, and those pixels paint as the window's background —
            // Mike, 2026-08-06: *"when i drag it down and to the right, it ends
            // up showing my black dead parts down and to the right."* The first
            // cut of this function skipped this and caused exactly that.
            let mut client = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let measured = GetClientRect(frame, &mut client) != 0;
            let (width, height) = (client.right - client.left, client.bottom - client.top);

            SetWindowLongPtrW(frame, GWL_STYLE, fixed);

            // Ask Win32 what outer size now yields the client area we had.
            let mut want = Rect {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let ex_style = GetWindowLongPtrW(frame, GWL_EXSTYLE) as u32;
            let adjusted =
                measured && AdjustWindowRectEx(&mut want, fixed as u32, 0, ex_style) != 0;

            // ⛔ `SWP_FRAMECHANGED` is not optional: without it the style is
            // stored but the frame is never recalculated, so the grab border
            // keeps working until something else happens to invalidate it.
            let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
            if adjusted {
                SetWindowPos(
                    frame,
                    std::ptr::null_mut(),
                    0,
                    0,
                    want.right - want.left,
                    want.bottom - want.top,
                    flags,
                );
            } else {
                SetWindowPos(frame, std::ptr::null_mut(), 0, 0, 0, 0, flags | SWP_NOSIZE);
            }

            // ⛔ **DO NOT stretch `baseview`'s child to fill the client here.**
            // Tried on 2026-08-06 and reverted the same hour: it makes the child
            // cover the gap, so the dead margin turns from the frame's black to
            // the child's white and nothing is actually fixed — the *webview*
            // inside the child still has its old bounds. Whatever is leaving a
            // gap has to be fixed where the two numbers are decided, in
            // `editor.rs::fit`, not painted over from here.
        }
        eprintln!("[pump] fixed the standalone frame on {frame:?} — no maximise, no resize");
    }

    /// Register the class once per process, then make the window per thread.
    fn create_pump_window(editor: *mut c_void) -> *mut c_void {
        let class: Vec<u16> = "FreallyWebviewPump\0".encode_utf16().collect();

        // SAFETY: both calls are documented Win32 entry points given arguments
        // this function owns. `RegisterClassW` copies the name, and a second
        // registration of the same class simply fails — which is why the return
        // is ignored rather than checked: another instance in the same process
        // may have registered it already, and the `CreateWindowExW` below is the
        // real test of whether the class exists.
        unsafe {
            let mut spec: WndClassW = std::mem::zeroed();
            spec.wnd_proc = Some(pump_proc);
            spec.instance = GetModuleHandleW(std::ptr::null());
            spec.class_name = class.as_ptr();
            let _ = RegisterClassW(&spec);

            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                // ⚠ `WS_CHILD` without `WS_VISIBLE`, and 0×0. It must be a child
                // so `open_blocking`'s filter retrieves its messages; it must be
                // invisible and empty so it never hit-tests, never paints and
                // never takes a click away from the webview above it.
                WS_CHILD,
                0,
                0,
                0,
                0,
                editor,
                std::ptr::null_mut(),
                spec.instance,
                std::ptr::null_mut(),
            )
        }
    }

    /// Runs [`drain`], one frame below `open_blocking` and outside every borrow.
    unsafe extern "system" fn pump_proc(
        hwnd: *mut c_void,
        msg: u32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        if msg == WM_PUMP {
            // ⛔ Cleared **before** draining, not after. Draining dispatches the
            // webview's completions, which is where the RPC handler — and
            // therefore a drag — runs, and a drag blocks here for the whole
            // gesture. Clearing afterwards would mean no pump could be queued
            // for the entire drag, and the webview would freeze while the
            // producer held the button.
            PUMP_POSTED.with(|p| p.set(false));
            drain(PUMP_EDITOR.with(|e| e.get()));
            return 0;
        }
        // SAFETY: the standard fall-through every window procedure owes Win32.
        unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
    }

    /// Drain what `baseview`'s loop cannot see, and **only** that.
    ///
    /// ⚠ **Normally reached from [`pump_proc`], which is outside baseview's
    /// window procedure** — that is what [`request`] buys, and it is what lets a
    /// drag run from here at all. But [`request`] still falls back to calling
    /// this *in place* when no pump window could be made, and that call is
    /// inside the procedure. So the rule below is written for the worse caller
    /// and holds for both.
    ///
    /// ⛔ **Messages belonging to `editor` or its children are deliberately left
    /// in the queue.** In the fallback, `on_frame` is inside baseview's window
    /// procedure, which is holding a `RefCell` borrow on its own window state
    /// for the duration — so dispatching another message to that same procedure
    /// re-enters it and panics with `RefCell already borrowed`
    /// (`baseview/src/platform/win/window.rs:513`). That panic then crosses an
    /// `extern "system"` frame, where it cannot unwind, and aborts the process.
    ///
    /// Leaving them is also simply correct on either path: baseview's
    /// `GetMessageW` filter retrieves messages for that window and its children
    /// perfectly well, in order. The **only** things it cannot retrieve are
    /// thread messages (`hwnd == NULL`) and messages for windows outside that
    /// subtree — which is exactly where WebView2's COM completions live, and
    /// exactly what this drains.
    ///
    /// ⚠ The pump window is itself a child of `editor`, so it passes the `ours`
    /// test and is never re-dispatched from here. Its own `WM_PUMP` has already
    /// been taken off the queue by `open_blocking` before this runs.
    pub fn drain(editor: *mut c_void) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        if PUMPING.with(|p| p.replace(true)) {
            return;
        }
        // ⛔ No editor handle means we cannot tell whose messages are whose, and
        // the safe reading of "no window" is *drain nothing* — not "nothing
        // belongs to the editor", which would dispatch baseview's own messages
        // and re-enter its window procedure on the first mouse move.
        if editor.is_null() {
            return;
        }
        let _guard = Guard;

        let mut msg = Msg {
            hwnd: std::ptr::null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            pt: Point { x: 0, y: 0 },
        };

        // SAFETY: `msg` is a correctly laid out `MSG` we own for the whole call,
        // and the queue being drained belongs to this thread — which is what
        // `own_message_queue` asserts and what the flag above gates on.
        unsafe {
            loop {
                // Look before taking. A message for baseview's window has to
                // stay queued for baseview, and there is no un-remove.
                if PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) == 0 {
                    break;
                }

                let ours =
                    !msg.hwnd.is_null() && (msg.hwnd == editor || IsChild(editor, msg.hwnd) != 0);
                if ours {
                    // Stop at the first one rather than skipping past it: the
                    // queue is ordered, and baseview will take it on its very
                    // next turn. Whatever is behind it keeps until then, and
                    // `on_frame` comes back around at frame rate.
                    break;
                }

                // ⛔ Remove **exactly** the message just inspected, by window
                // and by id. Re-peeking with a wide filter is a race: between
                // the two calls another thread can post to the editor window,
                // and a posted message outranks the `WM_PAINT`/`WM_TIMER` this
                // peek may have returned — so the wide `PM_REMOVE` would hand
                // back a *different*, editor-owned message and dispatch it,
                // re-entering baseview's window procedure while it holds its
                // `RefCell` borrow. That aborts the process, which is the exact
                // failure the `ours` check above exists to prevent.
                //
                // A thread message (`hwnd == NULL`) cannot be filtered by window
                // — `NULL` there means "any window" — so it is narrowed with
                // `-1`, which Win32 defines as "thread messages only".
                let filter = if msg.hwnd.is_null() {
                    usize::MAX as *mut c_void // (HWND)-1
                } else {
                    msg.hwnd
                };
                let (id, target) = (msg.message, msg.hwnd);
                if PeekMessageW(&mut msg, filter, id, id, PM_REMOVE) == 0 {
                    // Something else took it first. Nothing was removed, so the
                    // queue is still consistent; come back on the next frame.
                    break;
                }
                debug_assert_eq!(msg.hwnd, target);

                // `WM_QUIT` must not be swallowed. It is a thread message, so it
                // fails the `ours` test and would be removed and then dropped by
                // `DispatchMessageW` — leaving `baseview`'s loop waiting forever
                // for a quit that has already been consumed, and the standalone
                // hanging instead of closing.
                const WM_QUIT: u32 = 0x0012;
                if id == WM_QUIT {
                    PostQuitMessage(msg.w_param as i32);
                    break;
                }

                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl baseview::WindowHandler for WindowHandler {
    fn on_frame(&mut self, window: &mut baseview::Window) {
        // TASK-P16 probe. Whether this is called at all decides where the
        // Windows message pump can live: a frame tick is the only thread-owned
        // callback the adapter gets, so if it never fires the pump cannot be
        // driven from here. Counted rather than logged per frame, or the trace
        // is unreadable.
        if std::env::var("FREALLY_TRACE_EDITOR").is_ok() {
            static FRAMES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 3 || n % 60 == 0 {
                eprintln!("[editor] on_frame #{n}");
            }
        }

        // TASK-P16. Off unless this process asked for it; see `own_message_queue`.
        // The handle is passed so the pump can leave this window's own messages
        // to baseview — see `windows_pump::drain` for why that is not optional.
        //
        // ⛔⛔ **`request`, NOT `drain`, and TASK-063D is why.** This function
        // runs inside baseview's window procedure with a `RefCell` borrow live,
        // so draining *here* dispatches the webview's completions — and therefore
        // the RPC handler, and therefore `DoDragDrop` — on a stack that cannot
        // survive being re-entered. `request` only posts to a child window, which
        // `open_blocking` then dispatches from *outside* the procedure.
        // `windows_pump::request` carries the full mechanism.
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
            let editor = match window.raw_window_handle() {
                RawWindowHandle::Win32(handle) => handle.hwnd,
                _ => std::ptr::null_mut(),
            };
            windows_pump::request(editor);
        }

        let setter = ParamSetter::new(&*self.context);
        (self.event_loop_handler)(&self, setter, window);
    }

    fn on_event(&mut self, _window: &mut baseview::Window, event: Event) -> EventStatus {
        match event {
            Event::Keyboard(event) => {
                if (self.keyboard_handler)(event) {
                    EventStatus::Captured
                } else {
                    EventStatus::Ignored
                }
            }
            Event::Mouse(mouse_event) => (self.mouse_handler)(mouse_event),

            // ⛔⛔ **`Event::Window(Resized)` IS DELIBERATELY NOT HANDLED, AND AN
            // ATTEMPT TO HANDLE IT WAS REVERTED ON 2026-08-06.** It is tempting:
            // maximising left black dead space, and re-bounding the webview here
            // looks like the fix. It is not, and `plugin/src/editor.rs` says why
            // in the doc on `SCALES` and `physical` — **the window size and the
            // page zoom are one number**. The window is `LAYOUT * system_scale *
            // factor` and the page is zoomed by `factor`, so the CSS viewport
            // comes back out at `LAYOUT` whatever the window is. Re-bounding the
            // webview without also moving the zoom breaks that invariant, and
            // the symptom is the app cropping and the right rail collapsing —
            // the exact failure the design exists to prevent.
            //
            // ▶ **The answer is that the standalone cannot be resized at all**:
            // `windows_pump::pin_the_frame` takes away the sizing border and the
            // maximise box, so the only route to a different size is
            // `set_editor_size`, which moves *both* numbers together.
            _ => EventStatus::Ignored,
        }
    }
}

/// Everything about the webview that does not depend on who is building it.
///
/// Factored out because Linux builds the webview on the GTK thread while the
/// other platforms build it inline. Without this the attributes would exist
/// twice, and the copy a reader met first would be the one that had drifted —
/// which is precisely how this bridge already lost an evening to a dead
/// line-for-line duplicate of its own handler.
///
/// `with_web_context` is deliberately **not** applied here: it borrows for the
/// builder's whole lifetime, so the context has to be a local of whichever
/// scope is doing the building.
///
/// ⛔ **The caller must apply it *before* calling this, and that ordering is
/// load-bearing.** wry registers a custom protocol against the web context, so a
/// context attached after `with_custom_protocol` does not carry the scheme — the
/// page then fails to load and the window shows the background colour and
/// nothing else. The first version of this refactor applied it afterwards and
/// did exactly that: a perfectly compiled, perfectly blank editor.
fn configure<'a>(
    builder: WebViewBuilder<'a>,
    bounds: wry::Rect,
    source: &HTMLSource,
    background_color: (u8, u8, u8, u8),
    developer_mode: bool,
    custom_protocol: Option<(String, Arc<CustomProtocolHandler>)>,
    events_sender: Sender<Value>,
) -> WebViewBuilder<'a> {
    let mut builder = builder
        .with_bounds(bounds)
        .with_accept_first_mouse(true)
        .with_devtools(developer_mode)
        .with_initialization_script(include_str!("script.js"))
        .with_ipc_handler(move |msg: String| {
            let Ok(json_value) = serde_json::from_str::<Value>(&msg) else {
                // Upstream panicked here. A panic on the UI thread of someone
                // else's DAW takes the host down with it, and the message it
                // carried is a bug in the *page*, not grounds to kill Ableton.
                eprintln!("nih_plug_webview: invalid JSON from the web view: {msg}");
                return;
            };

            let _ = events_sender.send(json_value);
        })
        .with_background_color(background_color);

    if let Some((scheme, handler)) = custom_protocol {
        builder = builder.with_custom_protocol(scheme, move |request| match handler(&request) {
            Ok(response) => response,
            // Upstream unwrapped here too, and this one is the worse of the
            // pair: the webview calls this closure from an `extern "C"` frame,
            // where a panic cannot unwind — release builds set
            // `panic = "abort"`, so it ends the *host's* process rather than
            // arriving anywhere the host could catch it.
            //
            // A 500 reaches the page as a failed request instead, which a page
            // can report and a user can act on. `Response::new` is used rather
            // than the builder because the builder is itself fallible, and the
            // error path is not a place to add a new way to fail.
            Err(error) => {
                eprintln!("nih_plug_webview: the custom-protocol handler failed: {error}");
                let mut response = Response::new(Cow::Borrowed(
                    b"the custom-protocol handler failed" as &[u8],
                ));
                *response.status_mut() = wry::http::StatusCode::INTERNAL_SERVER_ERROR;
                response
            }
        });
    }

    // ⛔ **`FREALLY_TRACE_EDITOR=1` traces the load, and it exists because a
    // plugin has no console.** When the editor comes up blank there is nothing to
    // look at: no stderr anyone reads, no page to open devtools on if the page
    // never arrived. These three handlers answer, in order, the only questions
    // that matter — was navigation *requested*, did the content *start*, did it
    // *finish* — and the difference between "no navigation line at all" and
    // "Started with no Finished" is the difference between two entirely
    // different bugs.
    //
    // Left in rather than removed after use: this is the second time an empty
    // editor has cost an evening, and the first time the answer came from
    // turning devtools on in release for exactly this reason.
    if std::env::var("FREALLY_TRACE_EDITOR").is_ok() {
        builder = builder
            .with_navigation_handler(|url| {
                eprintln!("[editor] navigation requested: {url}");
                true
            })
            .with_on_page_load_handler(|event, url| {
                // `PageLoadEvent` has no `Debug`, so it is named here.
                let phase = match event {
                    wry::PageLoadEvent::Started => "started",
                    wry::PageLoadEvent::Finished => "finished",
                };
                eprintln!("[editor] page {phase}: {url}");
            })
            .with_document_title_changed_handler(|title| {
                eprintln!("[editor] title: {title}");
            });
    }

    match source {
        HTMLSource::String(html) => builder.with_html(*html),
        HTMLSource::URL(url) => builder.with_url(*url),
    }
    .expect("the source is a static string or a static URL")
}

/// Where the webview keeps its own profile.
///
/// ⛔ **A directory of our own, not the bare system temp.** Upstream passes
/// `std::env::temp_dir()`, which is shared with every other WebView2 and
/// WebKitGTK profile on the machine. WebView2 in particular refuses to create a
/// second environment over a user-data folder another environment already holds
/// with different options — and it does so *without* failing the create, which
/// presents as a webview that exists, reports success, and never navigates.
///
/// That is a live suspect for the blank Windows standalone (TASK-P16), and it is
/// poor hygiene regardless: a plugin should not be sharing its browser profile
/// with whatever else happens to be running.
fn web_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("freally-midi-master-webview")
}

struct Instance {
    window_handle: WindowHandle,
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.window_handle.close();
    }
}

unsafe impl Send for Instance {}

impl Editor for WebViewEditor {
    fn spawn(
        &self,
        parent: nih_plug::prelude::ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let options = WindowOpenOptions {
            scale: WindowScalePolicy::SystemScaleFactor,
            size: Size {
                width: self.width.load(Ordering::Relaxed) as f64,
                height: self.height.load(Ordering::Relaxed) as f64,
            },
            title: "Plug-in".to_owned(),
        };

        let width = self.width.clone();
        let height = self.height.clone();
        let developer_mode = self.developer_mode;
        let source = self.source.clone();
        let background_color = self.background_color;
        let custom_protocol = self.custom_protocol.clone();
        let event_loop_handler = self.event_loop_handler.clone();
        let keyboard_handler = self.keyboard_handler.clone();
        let mouse_handler = self.mouse_handler.clone();

        let window_handle = baseview::Window::open_parented(&parent, options, move |window| {
            let (events_sender, events_receiver) = unbounded();

            let bounds = wry::Rect {
                x: 0,
                y: 0,
                width: width.load(Ordering::Relaxed),
                height: height.load(Ordering::Relaxed),
            };

            #[cfg(not(target_os = "linux"))]
            let webview = {
                let mut web_context = WebContext::new(Some(web_data_dir()));
                // ⛔ The context goes on *first* — see `configure`.
                let built = configure(
                    WebViewBuilder::new_as_child(window).with_web_context(&mut web_context),
                    bounds,
                    &source,
                    background_color,
                    developer_mode,
                    custom_protocol,
                    events_sender,
                )
                .build();

                match built {
                    Ok(webview) => Some(webview),
                    // ⛔ Upstream panicked, and this closure is run by
                    // `baseview::Window::open_parented` **from inside the
                    // host's editor-open callback** — so the panic is the
                    // host's crash, and under `panic = "abort"` it cannot even
                    // be caught. It is not a remote failure either: a machine
                    // with no WebView2 Evergreen runtime, a read-only temp
                    // directory, or a `web_data_dir()` already held by another
                    // WebView2 environment with different options all land
                    // here. That last one is the standalone opened alongside
                    // the DAW.
                    //
                    // Logged, exactly as the Linux branch below does, and the
                    // editor stays inert. A blank editor is recoverable; a
                    // closed DAW with unsaved work is not.
                    Err(error) => {
                        eprintln!("nih_plug_webview: could not construct the webview: {error}");
                        None
                    }
                }
            };

            // Linux builds it on the GTK thread and keeps it there; see `linux`
            // for why it cannot be built here. The handle comes back straight
            // away and the window fills in a moment later, which is why nothing
            // below this point may assume the webview already exists.
            #[cfg(target_os = "linux")]
            let webview = {
                let handle = linux::WebViewHandle::new();
                let id = handle.id();

                match window.raw_window_handle() {
                    RawWindowHandle::Xlib(xlib) => {
                        let parent = xlib.window;
                        linux::on_gtk(move || {
                            let parent = linux::ParentXid(parent);
                            let mut web_context = WebContext::new(Some(web_data_dir()));
                            // ⛔ The context goes on *first* — see `configure`.
                            let built = configure(
                                WebViewBuilder::new_as_child(&parent)
                                    .with_web_context(&mut web_context),
                                bounds,
                                &source,
                                background_color,
                                developer_mode,
                                custom_protocol,
                                events_sender,
                            )
                            .build();

                            match built {
                                Ok(webview) => linux::store(id, webview),
                                // Logged, not panicked: this runs on a thread
                                // shared with the host's other plugins.
                                Err(error) => eprintln!(
                                    "nih_plug_webview: could not construct the webview: {error}"
                                ),
                            }
                        });
                    }
                    // wry's WebKitGTK backend is X11-only, and so is baseview.
                    // Wayland is out of scope (TASK-P12); say so rather than
                    // handing X an invalid parent and letting it abort.
                    other => eprintln!(
                        "nih_plug_webview: the editor needs an X11 window, got {other:?}; \
                         under Wayland, run the host with XWayland"
                    ),
                }

                handle
            };

            WindowHandler {
                context,
                event_loop_handler,
                webview,
                events_receiver,
                keyboard_handler,
                mouse_handler,
                width,
                height,
            }
        });
        return Box::new(Instance { window_handle });
    }

    fn size(&self) -> (u32, u32) {
        (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        )
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        // TODO: implement for Windows and Linux
        return false;
    }

    fn param_values_changed(&self) {}

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}
}
