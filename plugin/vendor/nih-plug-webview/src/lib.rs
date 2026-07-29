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
    webview: WebView,
    #[cfg(target_os = "linux")]
    webview: linux::WebViewHandle,
    events_receiver: Receiver<Value>,
    pub width: Arc<AtomicU32>,
    pub height: Arc<AtomicU32>,
}

impl WindowHandler {
    pub fn resize(&self, window: &mut baseview::Window, width: u32, height: u32) {
        self.webview.set_bounds(wry::Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);
        self.context.request_resize();
        window.resize(Size {
            width: width as f64,
            height: height as f64,
        });
    }

    pub fn send_json(&self, json: Value) {
        let json_str = json.to_string();
        let json_str_quoted =
            serde_json::to_string(&json_str).expect("Should not fail: the value is always string");
        self.webview
            .evaluate_script(&format!("onPluginMessageInternal({});", json_str_quoted))
            .unwrap();
    }

    pub fn next_event(&self) -> Result<Value, crossbeam::channel::TryRecvError> {
        self.events_receiver.try_recv()
    }
}

impl baseview::WindowHandler for WindowHandler {
    fn on_frame(&mut self, window: &mut baseview::Window) {
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
        builder = builder.with_custom_protocol(scheme, move |request| handler(&request).unwrap());
    }

    match source {
        HTMLSource::String(html) => builder.with_html(*html),
        HTMLSource::URL(url) => builder.with_url(*url),
    }
    .expect("the source is a static string or a static URL")
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
                let mut web_context = WebContext::new(Some(std::env::temp_dir()));
                // ⛔ The context goes on *first* — see `configure`.
                configure(
                    WebViewBuilder::new_as_child(window).with_web_context(&mut web_context),
                    bounds,
                    &source,
                    background_color,
                    developer_mode,
                    custom_protocol,
                    events_sender,
                )
                .build()
                .unwrap_or_else(|e| panic!("Failed to construct webview. {}", e))
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
                            let mut web_context = WebContext::new(Some(std::env::temp_dir()));
                            // ⛔ The context goes on *first* — see `configure`.
                            configure(
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
