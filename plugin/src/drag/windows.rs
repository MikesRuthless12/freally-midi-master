//! The Windows drag source (TASK-063C / FMM-S03).
//!
//! `DoDragDrop` with a `CF_HDROP` data object — the same thing a file dragged
//! out of Explorer carries, which is why every DAW's drop target already
//! handles it. [`super`] carries the note on why this is a real file rather
//! than a promised one.
//!
//! ## ⛔ Where this runs, and why there was no choice
//!
//! On the thread that answers `POST /__rpc` — the host's editor thread. That is
//! not a preference:
//!
//! - `DoDragDrop` runs a **modal loop** and must be called from the thread that
//!   owns the gesture. Handing it to a worker would leave the mouse captured by
//!   one thread and the drag driven by another.
//! - The custom-protocol handler is the **only** code path that runs in a
//!   hosted plugin window. `with_event_loop` looked like the tidier home for
//!   this right up until you read `plugin/vendor/VENDORED.md`: a window
//!   parented into Ableton Live never receives a frame tick, so that loop never
//!   runs there at all.
//!
//! ✅ **No edit to the vendored adapter was needed, and the handoff budgeted
//! for one.** The reason is small and worth writing down: `DoDragDrop` takes no
//! `HWND`. It wants an `IDataObject`, an `IDropSource` and the thread's mouse
//! state, none of which the editor window has to hand over — so the drag source
//! never has to reach inside `nih-plug-webview` for a window handle, and
//! `VENDORED.md` has nothing new to account for on the next rebase.
//!
//! ## ⛔⛔ THE REENTRANCY, AND WHY THE STANDALONE MUST NOT DO THIS
//!
//! `DoDragDrop` runs a modal loop that pumps and dispatches the thread's
//! messages — **including the editor window's own**. `VENDORED.md` records that
//! dispatching a message back to that window while baseview is holding a
//! `RefCell` borrow **aborts the process**, and that it was observed rather than
//! theorised.
//!
//! **In a hosted plugin that does not happen.** The call arrives on WebView2's
//! resource handler, which COM delivers through its own message-only window;
//! baseview's window procedure is not on the stack and nothing of its is
//! borrowed.
//!
//! ⛔ **In the standalone it happens every time, and an earlier version of this
//! comment claimed otherwise.** `plugin/src/bin/standalone.rs` calls
//! `own_message_queue()`, which turns on `windows_pump`; `windows_pump::drain`
//! is called **from `on_frame`**, which is itself called from inside baseview's
//! window procedure with that borrow live. `drain` dispatches exactly the
//! message-only-window messages WebView2's COM completions ride on — so in the
//! standalone the RPC handler, and therefore this function, runs *inside* the
//! window procedure. `DoDragDrop` then dispatches a `WM_TIMER` straight back
//! into it, the `RefCell` panics inside an `extern "system"` frame, and the
//! process aborts.
//!
//! That is why [`crate::drag::supported_in`] refuses the standalone. The
//! condition is **"this process pumps its own queue"**, which today is exactly
//! "this is the standalone" — ⛔ if anything else ever calls
//! `own_message_queue()`, it has to be refused here too.

use std::mem::ManuallyDrop;
use std::path::PathBuf;

use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{implement, HRESULT};
use windows::Win32::Foundation::{
    COLORREF, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND,
    POINT, SIZE, S_OK,
};
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::Com::{
    CoCreateInstance, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC, STGMEDIUM,
    TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, CF_HDROP, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_NONE,
};

use windows::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_RBUTTON, MODIFIERKEYS_FLAGS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, ReleaseCapture, VK_LBUTTON,
};
use windows::Win32::UI::Shell::{
    CLSID_DragDropHelper, IDragSourceHelper, SHCreateDataObject, DROPFILES, SHDRAGIMAGE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, IsWindowVisible, SetWindowPos, ShowWindow, GA_ROOT, HWND_TOP, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOW,
};

use super::{Dropped, Preview};

pub const SUPPORTED: bool = true;

/// ⛔ **No.** This is the one platform where a standalone drag aborts the
/// process — the module header above has the whole mechanism, and it is the
/// reason `supported_in` refuses at all. Fixing it means getting `DoDragDrop`
/// off the stack that baseview's window procedure is already on; see TASK-063D.
pub const STANDALONE_SAFE: bool = false;

/// Hand `paths` to the OS and block until the producer lets go.
///
/// `stacked` is the alternative set to hand over **while Ctrl is held**, or
/// empty where the gesture has only one meaning. See [`DropSource`] for why the
/// choice is made during the drag rather than before it.
pub fn drag(
    paths: &[PathBuf],
    stacked: &[PathBuf],
    preview: Option<&Preview>,
) -> Result<Dropped, String> {
    if paths.is_empty() {
        return Err("there are no files to drag".to_owned());
    }
    // ⛔ **The button has to still be down, and this is not defensive noise.**
    // Preparing a drag renders and writes files, so the page can only call this
    // a little *after* the gesture began — and if the producer let go in the
    // meantime, `DoDragDrop` sees no button on its very first
    // `QueryContinueDrag`, answers `DRAGDROP_S_DROP` immediately, and drops the
    // files wherever the cursor happens to be sitting. A stray loop landing on
    // a random track is the worst possible reading of "the drag did nothing".
    if !button_is_down() {
        return Ok(Dropped::Cancelled);
    }
    ensure_ole()?;

    // SAFETY: every call below is a documented Win32/COM entry point, given
    // arguments this function owns. The data object and the drop source are
    // refcounted by the `windows` crate's generated implementations; the one
    // raw allocation is the `HGLOBAL`, whose ownership passes to the data
    // object in `SetData` and is described at `hdrop`.
    unsafe {
        // ⛔ **Before `DoDragDrop`, not after.** The webview still holds the
        // mouse from the `mousedown` that started this, and a captured pointer
        // means the drag loop never sees the cursor leave the plugin window —
        // the drag appears to start and then goes nowhere.
        let _ = ReleaseCapture();

        // ⛔⛔ **Remembered *before* the drag, because after it the focus is the
        // DAW's.** See [`restore_editor`] for what this is for. Taken from the
        // OS rather than from the vendored adapter, so `VENDORED.md` still has
        // nothing to account for — this module's header makes a point of
        // `DoDragDrop` needing no `HWND`, and that stays true of the *drag*.
        let editor = GetAncestor(GetFocus(), GA_ROOT);

        let data: IDataObject = SHCreateDataObject(None, None, None)
            .map_err(|e| format!("could not create the drag payload: {e}"))?;
        let medium = hdrop(paths)?;
        let format = hdrop_format();
        // `true` hands the medium over: the data object frees the `HGLOBAL`,
        // and a `false` here would leak it on every drag.
        data.SetData(&format, &medium, true)
            .map_err(|e| format!("could not attach the files to the drag: {e}"))?;

        // ⚠ **After `SetData`, before `DoDragDrop`.** The helper writes its own
        // private formats onto this same data object, so it needs the object to
        // exist and the drag not to have started. A failure here loses the
        // picture and keeps the drag: a producer would rather move a file with
        // a plain cursor than not move it at all.
        if let Some(preview) = preview {
            if let Err(error) = attach_drag_image(&data, preview) {
                nih_plug::nih_log!("[drag] no drag image: {error}");
            }
        }

        let source: IDropSource = DropSource {
            data: data.clone(),
            plain: paths.to_vec(),
            stacked: stacked.to_vec(),
            holding_stacked: AtomicBool::new(false),
        }
        .into();
        let mut effect = DROPEFFECT_NONE;
        // ⛔ **Copy only.** `DROPEFFECT_MOVE` would invite the drop target to
        // tell us to delete the source afterwards, and the source is the
        // producer's spooled loop — which their DAW may still be referencing.
        let result = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);

        // ⛔ **However the drag ended.** A cancelled or refused drag hides the
        // window exactly as a successful one does, and leaving the producer to
        // find it themselves is the complaint either way.
        restore_editor(editor);

        match result {
            DRAGDROP_S_DROP if effect != DROPEFFECT_NONE => Ok(Dropped::Copied),
            // ⛔ **A drop that reported no effect is `Refused`, NOT `Cancelled`,
            // and the difference decides whether the files are deleted.** A
            // target may take the data and still leave the effect at `NONE` —
            // `IDataObjectAsyncCapability` extracts after this call returns.
            // Reporting it as a cancel is what let `Drags::start` delete a `.wav`
            // a DAW had just referenced.
            DRAGDROP_S_DROP => Ok(Dropped::Refused),
            // Nothing ever received it: the producer let go over nothing, or
            // pressed Escape. Neither is a fault worth a red message.
            DRAGDROP_S_CANCEL => Ok(Dropped::Cancelled),
            other => Err(format!("the drag failed ({:#010x})", other.0)),
        }
    }
}

/// Put the editor back once the drag is over.
///
/// ⛔⛔ **Mike, 2026-08-06, in Ableton:** *"it disappears when i go to drag midi
/// or audio to the arrangement view … and then it never appears unless i go
/// back to the other view and restore the window myself."* Dragging into the
/// Arrangement means Ableton changes view, and the plugin window goes with it —
/// so the producer lands the clip and is left with no plugin, having to go back
/// to Session view and reopen it to drag the next lane. With eight drum lanes
/// that is eight round trips through the host's own UI.
///
/// ⚠ **Restore, not "always on top", and Mike chose this himself:** *"can you
/// just have it disappear while you are dragging into the DAW and have it show
/// on top again after you get done dragging?"* A `WS_EX_TOPMOST` window floats
/// over every application on the desktop for the rest of the session and fights
/// the host's own plugin-window management — Ableton ships a preference for it,
/// so it is already the producer's call. This only undoes what the drag did.
///
/// ⚠ **`SWP_NOACTIVATE`, so the DAW keeps the keyboard.** They have just
/// dropped a clip and the next thing they do is in the arrangement; stealing
/// focus back would put their next keystroke in the wrong application.
///
/// A window we cannot find is left alone rather than guessed at: raising the
/// wrong top-level window would be worse than raising none.
unsafe fn restore_editor(window: HWND) {
    if window.is_invalid() {
        return;
    }
    if !IsWindowVisible(window).as_bool() {
        let _ = ShowWindow(window, SW_SHOW);
    }
    let _ = SetWindowPos(
        window,
        Some(HWND_TOP),
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

/// The `CF_HDROP` format, as every drop target asks for it.
fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// The paths as a `DROPFILES` block in movable global memory.
///
/// The layout is a `DROPFILES` header followed by the file names as UTF-16,
/// each `NUL`-terminated, with a second `NUL` ending the list. ⚠ **Both
/// terminators are required** — a list with only one is read past its end by
/// the drop target, which is a crash in the *host*, not here.
unsafe fn hdrop(paths: &[PathBuf]) -> Result<STGMEDIUM, String> {
    let mut names: Vec<u16> = Vec::new();
    for path in paths {
        // A path that is not valid UTF-16 cannot exist on Windows; this is the
        // encode, not a validation.
        names.extend(path.as_os_str().encode_wide());
        names.push(0);
    }
    names.push(0);

    let header = std::mem::size_of::<DROPFILES>();
    let bytes = header + names.len() * 2;
    let handle: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes)
        .map_err(|e| format!("out of memory for the drag: {e}"))?;

    let base = GlobalLock(handle);
    if base.is_null() {
        return Err("the drag payload could not be locked".to_owned());
    }
    base.cast::<DROPFILES>().write(DROPFILES {
        // Where the names start, relative to the beginning of this block.
        pFiles: header as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: false.into(),
        // ⛔ Wide. Saying `false` here with UTF-16 in the buffer is how a
        // dropped file arrives at the DAW with a name of one character.
        fWide: true.into(),
    });
    std::ptr::copy_nonoverlapping(names.as_ptr(), base.add(header).cast::<u16>(), names.len());
    let _ = GlobalUnlock(handle);

    Ok(STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
        // The data object owns the handle once `SetData` has taken it.
        pUnkForRelease: ManuallyDrop::new(None),
    })
}

/// What the OS asks about whether the drag is still going.
///
/// The whole of the interface is two questions, and the answers are the
/// standard ones: escape or the right button cancels, letting the left button
/// go drops, and the shell draws the cursors.
/// ⛔⛔ **Ctrl is read here, and this is the only place it can be.**
///
/// Mike, 2026-08-06: *"press and hold ctrl either before or during the drag"* —
/// so the answer cannot be settled when the drag starts. The obvious hook would
/// be `IDataObject::GetData`, which the drop target calls at the moment of the
/// drop; but the payload is the **shell's** data object (`SHCreateDataObject`),
/// not ours, so there is no `GetData` of ours to run. Writing one would mean
/// implementing the whole of `IDataObject` — and `IDragSourceHelper` writes its
/// own private formats onto that same object, so our implementation would have
/// to carry those too.
///
/// `QueryContinueDrag` is called repeatedly for the length of the drag and is
/// handed the live key state, and the data object is still ours to write to. So
/// the modifier is watched here and the `CF_HDROP` is **replaced** whenever it
/// changes — the target reads whatever was set last, which is the state at the
/// drop.
///
/// ⚠ **Only rewritten when it actually changes.** This runs on every mouse
/// message for the whole gesture; re-allocating the file list each time would
/// be a global alloc per pointer move inside somebody's DAW.
#[implement(IDropSource)]
struct DropSource {
    /// The object the payload is written to. Cloned handle, same COM object.
    data: IDataObject,
    /// What is handed over with no modifier held.
    plain: Vec<PathBuf>,
    /// What is handed over while Ctrl is held. Empty when Ctrl means nothing
    /// for this gesture — a single lane has only one arrangement.
    stacked: Vec<PathBuf>,
    holding_stacked: AtomicBool,
}

impl DropSource {
    /// Put the set `ctrl` asks for onto the data object, if it is not already on.
    ///
    /// ⚠ A failure is logged and swallowed: the drag is already in flight and
    /// the object still holds a perfectly good file list, so the honest outcome
    /// is the other layout rather than a cancelled gesture.
    fn follow_modifier(&self, ctrl: bool) {
        if self.stacked.is_empty() || ctrl == self.holding_stacked.load(Ordering::Relaxed) {
            return;
        }
        let want = if ctrl { &self.stacked } else { &self.plain };
        // SAFETY: `hdrop` allocates an `HGLOBAL` this call hands to the data
        // object (`true` transfers ownership, exactly as the initial `SetData`
        // does), and the object releases the medium it held before.
        unsafe {
            match hdrop(want) {
                Ok(medium) => {
                    if let Err(error) = self.data.SetData(&hdrop_format(), &medium, true) {
                        nih_plug::nih_log!("[drag] could not swap the payload: {error}");
                        return;
                    }
                }
                Err(error) => {
                    nih_plug::nih_log!("[drag] could not build the payload: {error}");
                    return;
                }
            }
        }
        self.holding_stacked.store(ctrl, Ordering::Relaxed);
    }
}

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows::core::BOOL,
        key_state: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if escape_pressed.as_bool() || key_state.contains(MK_RBUTTON) {
            return DRAGDROP_S_CANCEL;
        }
        // ⛔ **Before the drop is reported, not after.** Returning
        // `DRAGDROP_S_DROP` is what sends the target to read the data, so the
        // payload has to already say what the modifier asked for.
        self.follow_modifier(key_state.contains(MK_CONTROL));
        if !key_state.contains(MK_LBUTTON) {
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        // The shell's own cursors. Drawing our own would mean tracking the
        // target's answer to say "copy here" versus "no", which is exactly the
        // work this return value delegates.
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Put the page's picture under the cursor for the length of the drag.
///
/// The shell's own `IDragSourceHelper` does the drawing — it is what Explorer
/// uses, so the image follows the cursor across every window on the desktop
/// rather than stopping at the edge of the plugin, which is the entire reason
/// this is not an HTML5 `setDragImage`.
unsafe fn attach_drag_image(data: &IDataObject, preview: &Preview) -> Result<(), String> {
    let helper: IDragSourceHelper =
        CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("no drag helper: {e}"))?;

    // ⛔ **Taken together from the one constructor that bounds them.**
    // `Preview::new` is the only way to build one, and it refuses a buffer whose
    // length is not exactly `width * height * 4`. That is what makes the raw
    // slice below in bounds: a 32-bpp DIB's stride is always `width * 4`, with
    // no padding, so `CreateDIBSection` allocates exactly `rgba.len()` bytes.
    let (pixel_width, pixel_height, rgba) = preview.parts();
    let width = pixel_width as i32;
    let height = pixel_height as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // ⛔ **Negative, and this is the classic way a drag image ships
            // upside down.** A DIB is bottom-up unless the height says
            // otherwise, and a canvas is top-down.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pixels: *mut std::ffi::c_void = std::ptr::null_mut();
    let bitmap = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut pixels, None, 0)
        .map_err(|e| format!("no bitmap: {e}"))?;
    if pixels.is_null() {
        let _ = DeleteObject(bitmap.into());
        return Err("the bitmap has no pixels".to_owned());
    }

    // RGBA from a canvas to the BGRA the GDI wants, premultiplied.
    //
    // ⛔ **Premultiplied is not optional.** The shell alpha-blends this, and an
    // unpremultiplied buffer renders as a bright halo around every edge — which
    // reads as a rendering bug rather than as a rounded corner.
    let out = std::slice::from_raw_parts_mut(pixels.cast::<u8>(), rgba.len());
    for (source, dest) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let alpha = u32::from(source[3]);
        let mul = |channel: u8| ((u32::from(channel) * alpha + 127) / 255) as u8;
        dest[0] = mul(source[2]);
        dest[1] = mul(source[1]);
        dest[2] = mul(source[0]);
        dest[3] = source[3];
    }

    let image = SHDRAGIMAGE {
        sizeDragImage: SIZE {
            cx: width,
            cy: height,
        },
        // Where the cursor sits inside the picture. Just inside the top-left
        // corner, so the image trails below and to the right and never covers
        // the drop target the producer is aiming at.
        ptOffset: POINT { x: 12, y: 12 },
        hbmpDragImage: bitmap,
        // No colour key: the alpha channel is what makes this transparent.
        crColorKey: COLORREF(0xFFFF_FFFF),
    };

    match helper.InitializeFromBitmap(&image, data) {
        // ⛔ **The helper owns the bitmap once this succeeds** and deletes it
        // itself; deleting it here as well is a double free inside the host.
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = DeleteObject(bitmap.into());
            Err(format!("the helper refused the image: {error}"))
        }
    }
}

/// Whether the primary mouse button is held right now.
///
/// ⚠ `VK_LBUTTON` is the *primary* button, so this is still correct for a
/// producer who has swapped their mouse buttons — it is the one they dragged
/// with either way.
fn button_is_down() -> bool {
    // SAFETY: a pure query with no arguments to get wrong.
    let state = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) };
    // The high bit is "down now"; the low bit is "was pressed since last asked"
    // and is deliberately ignored — a click that has already finished is
    // exactly the case this guards against.
    (state as u16) & 0x8000 != 0
}

/// Make sure OLE is up on this thread.
///
/// ⛔ **Initialised and never uninitialised, deliberately.** `OleUninitialize`
/// tears down the apartment when the last reference goes, and this runs on a
/// thread the *host* owns — getting the pairing wrong by one, once, unloads COM
/// out from under Ableton. The asymmetry is the whole argument: an extra
/// reference on a thread that lives as long as the process is invisible, and
/// the alternative failure is the DAW closing.
///
/// ⚠ `OleInitialize` on a thread that already called `CoInitializeEx` with a
/// compatible model succeeds and adds a reference, which is the normal case
/// here — WebView2 requires an STA, so the host has already been through this.
fn ensure_ole() -> Result<(), String> {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    DONE.with(|done| {
        if done.get() {
            return Ok(());
        }
        // SAFETY: an ordinary COM entry point; the argument is reserved and
        // must be null.
        let result = unsafe { OleInitialize(None) };
        // `S_FALSE` means "already initialised on this thread", which is a
        // success and the expected answer inside a DAW.
        if result.is_err() {
            return Err(format!("this thread cannot start a drag ({result:?})"));
        }
        done.set(true);
        Ok(())
    })
}

/// `OsStr` as UTF-16, without pulling the whole `OsStrExt` name into scope
/// where it would read as a general utility.
use std::os::windows::ffi::OsStrExt;

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{IDataObject, DATADIR_GET};

    /// Read a `CF_HDROP` medium back into the paths it carries.
    ///
    /// ⛔ **The point of the test, and the only part of this module that can be
    /// checked without a DAW and a hand on a mouse.** Everything downstream —
    /// whether Ableton takes the drop, whether the clip lands on the right
    /// track — needs a human. What this proves is that the bytes handed to the
    /// OS are the paths that were spooled, in order, wide and doubly
    /// terminated, which is the half that fails silently.
    unsafe fn read_back(data: &IDataObject) -> Vec<String> {
        let medium = data
            .GetData(&hdrop_format())
            .expect("CF_HDROP must be there");
        let base = GlobalLock(medium.u.hGlobal);
        assert!(!base.is_null());

        let header = base.cast::<DROPFILES>().read();
        assert!(header.fWide.as_bool(), "the names must be UTF-16");

        let mut out = Vec::new();
        let mut at = base.add(header.pFiles as usize).cast::<u16>();
        loop {
            let mut len = 0usize;
            while at.add(len).read() != 0 {
                len += 1;
            }
            if len == 0 {
                // The second terminator: the end of the list.
                break;
            }
            out.push(String::from_utf16_lossy(std::slice::from_raw_parts(
                at, len,
            )));
            at = at.add(len + 1);
        }
        let _ = GlobalUnlock(medium.u.hGlobal);
        out
    }

    #[test]
    fn the_data_object_hands_back_the_paths_it_was_given() {
        ensure_ole().expect("the test thread must take OLE");
        let paths = vec![
            PathBuf::from(r"C:\Temp\trap - Snare - 140 BPM - C# Minor.mid"),
            PathBuf::from(r"C:\Temp\trap - Kick - 140 BPM - C# Minor.mid"),
        ];
        unsafe {
            let data: IDataObject = SHCreateDataObject(None, None, None).unwrap();
            let medium = hdrop(&paths).unwrap();
            data.SetData(&hdrop_format(), &medium, true).unwrap();

            let read = read_back(&data);
            assert_eq!(
                read,
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                "the drop target must see exactly the spooled files, in order"
            );
        }
    }

    #[test]
    fn a_drop_target_asking_what_is_on_offer_is_told_hdrop() {
        // ⚠ Some targets call `QueryGetData` before they will light up as a
        // drop zone at all, so a data object that only answers `GetData` looks
        // like nothing is being dragged.
        ensure_ole().unwrap();
        unsafe {
            let data: IDataObject = SHCreateDataObject(None, None, None).unwrap();
            let medium = hdrop(&[PathBuf::from(r"C:\Temp\a.mid")]).unwrap();
            data.SetData(&hdrop_format(), &medium, true).unwrap();
            assert_eq!(data.QueryGetData(&hdrop_format()), S_OK);

            // And it enumerates, which is the other way a target asks.
            let mut formats = [FORMATETC::default(); 8];
            let enumerator = data.EnumFormatEtc(DATADIR_GET.0 as u32).unwrap();
            let mut got = 0u32;
            enumerator
                .Next(&mut formats, Some(&mut got))
                .ok()
                .or(if got > 0 { Ok(()) } else { Err(()) })
                .expect("the formats must enumerate");
            assert!(
                formats[..got as usize]
                    .iter()
                    .any(|f| f.cfFormat == CF_HDROP.0),
                "CF_HDROP must be among the formats on offer"
            );
        }
    }

    #[test]
    fn dragging_nothing_is_refused_rather_than_starting_an_empty_drag() {
        assert!(drag(&[], &[], None).is_err());
    }
}
