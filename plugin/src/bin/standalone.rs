// ⛔ **No console window behind the app in release** — Mike, 2026-08-09:
// *"rebuild the standalone release without it opening the terminal window with
// it."* A console subsystem binary gets one whether it wants it or not, and for
// something a producer double-clicks it looks like a program that is still
// installing itself.
//
// ⚠ **Release only, and `debug_assertions` rather than a feature is the switch.**
// `npm run plugin:standalone` is a debug build and is how this is developed —
// keeping its console is the whole point of that path, because everything below
// prints there. In release there is nowhere for `eprintln!` to go, which is why
// [`crash_log`] exists: a crash reporter that writes to a stream nobody attached
// is not a reporter.
//
// ⚠ Logging still works in release: `NIH_LOG` takes a **file path** as well as
// `stderr` and `windbg`, so `set NIH_LOG=C:\some\file.log` brings the whole log
// back without rebuilding.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! The plugin as an ordinary application, with no DAW involved.
//!
//! This exists so that testing does not mean "copy the binary into a plugin
//! folder, rescan in the DAW, insert it on a track, and do it again next
//! build". It runs the *same* `Plugin` impl the CLAP exports — same editor,
//! same bridge, same engine — in a native window with its own audio and MIDI
//! I/O, and it rebuilds and relaunches in seconds.
//!
//! What it can and cannot tell you:
//!
//! - **It proves** the UI renders, the bridge answers, generation works, the
//!   notes are scheduled, and nothing panics. That is most of what breaks.
//! - **It cannot sync to your DAW, and it never will.** There is no host here,
//!   so there is nothing to sync *to*: `--tempo` sets a fixed tempo that the
//!   transport reports, which exercises the plumbing (tempo → `HostSession` →
//!   `SessionContext` → generation, all of which `host.rs` already unit-tests)
//!   but is not your project's tempo. **Real host tempo, meter and playhead
//!   require a real host.** No standalone, validator or emulator substitutes
//!   for that, and pretending otherwise is how a plugin ships broken sync.
//!
//! So the split is: **standalone for iteration, a DAW for host integration** —
//! and `npm run plugin:install` links the build into the CLAP folder once so
//! the DAW leg costs a rebuild rather than a copy-and-rescan every time.
//! `clap-validator` sits between them and checks the contract.
//!
//! **A standalone that works is necessary and not sufficient.** This project
//! has already shipped one phase on automated evidence alone and written the
//! gap down in capitals; do not do it twice.
//!
//! Run it with `npm run plugin:standalone`. `--help` lists the audio, MIDI and
//! tempo flags nih-plug provides.
//!
//! ## ⛔ Why that script passes `--period-size 2048`
//!
//! nih-plug's default is **512**, and its cpal backend asserts that the buffer
//! WASAPI hands the callback is no larger than the one it configured:
//!
//! ```text
//! thread 'cpal_wasapi_out' panicked at
//! 'Received 1056 samples, while the configured buffer size is 512'
//! nih-plug/src/wrapper/standalone/backend/cpal.rs:832
//! ```
//!
//! A shared-mode WASAPI period is the device's, not ours, and 1056 is what this
//! hardware gives. ⚠ **The panic is on the audio thread and does not close the
//! window**, which is what makes it nasty: `log_panics` catches it, that thread
//! dies still owning the raw channel pointers nih-plug's buffer manager holds,
//! and the process runs on looking healthy until the next real interaction
//! faults it with `STATUS_ACCESS_VIOLATION`. It was mistaken for a drag bug on
//! 2026-08-06 for exactly that reason — the crash arrived on a drag, minutes
//! after the cause.
//!
//! ⚠ **Standalone only.** A DAW hands the plugin its own buffers and never goes
//! near this backend, so nothing about it applies to the hosted path.

use freally_midi_master_plugin::FreallyMidiMaster;
use nih_plug::prelude::*;

/// Print a stack trace when the process is about to die of a hardware fault.
///
/// ⛔⛔ **Because a panic tells you where it happened and an access violation
/// does not.** TASK-063D crashed the standalone three times with nothing but
/// `exit code: 0xc0000005, STATUS_ACCESS_VIOLATION` — no message, no frames, and
/// no debugger on the machine to attach. `nih_plug`'s `log_panics` covers
/// panics; nothing covered this, so every crash cost a reproduction and bought
/// no information.
///
/// ⚠ **`SetUnhandledExceptionFilter`, not a vectored handler.** A vectored
/// handler also sees *first-chance* exceptions, which several runtimes throw and
/// handle as a matter of course — it would print stacks for faults that were
/// never going to be fatal. This fires once, on the way down.
///
/// ⚠ Standalone only, and deliberately: a DAW installs its own handler and ours
/// would fight it.
#[cfg(windows)]
fn report_hardware_faults() {
    use std::ffi::c_void;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        next: *mut ExceptionRecord,
        address: *mut c_void,
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    const ACCESS_VIOLATION: u32 = 0xC000_0005;
    const CONTINUE_SEARCH: i32 = 0;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(
            filter: Option<unsafe extern "system" fn(*mut ExceptionPointers) -> i32>,
        ) -> *mut c_void;
    }

    unsafe extern "system" fn filter(info: *mut ExceptionPointers) -> i32 {
        // SAFETY: Windows hands this pointer to the filter it called, and the
        // records live for the duration of the call.
        unsafe {
            if !info.is_null() && !(*info).record.is_null() {
                let record = &*(*info).record;
                let what = if record.code == ACCESS_VIOLATION {
                    "ACCESS VIOLATION"
                } else {
                    "hardware fault"
                };
                let report = format!(
                    "\n[crash] {what} (0x{:08X}) at {:?}\n{}",
                    record.code,
                    record.address,
                    std::backtrace::Backtrace::force_capture()
                );
                eprintln!("{report}");
                crash_log(&report);
            }
        }
        // ⚠ Let Windows carry on killing us. This is a reporter, not a recovery
        // — swallowing a fault leaves the process running on corrupted state.
        CONTINUE_SEARCH
    }

    // SAFETY: a documented entry point taking a function pointer with the
    // signature Windows specifies.
    unsafe {
        SetUnhandledExceptionFilter(Some(filter));
    }
}

/// Append a crash report to `%APPDATA%\Freally MIDI Master\standalone-crash.log`.
///
/// ⛔⛔ **Because the release build has no console, so `eprintln!` goes nowhere.**
/// Taking the terminal window away took the crash reporter's only output with
/// it, and this project currently has **an unreproduced standalone crash** — so
/// shipping a silent one would have thrown away the evidence for exactly the
/// question that is open. TASK-063D's whole point was that
/// `STATUS_ACCESS_VIOLATION` with no frames costs a reproduction and buys
/// nothing.
///
/// ⚠ **Appends, and says nothing when it cannot.** This runs inside an unhandled
/// exception filter on a process Windows is already killing: there is no one to
/// report a failed write to, and doing more work on the way down risks losing
/// the report that did land. `Err` is discarded deliberately.
#[cfg(windows)]
fn crash_log(report: &str) {
    use std::io::Write;

    let Some(base) = std::env::var_os("APPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(base)
        .join("Freally MIDI Master")
        .join("standalone-crash.log");
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{report}");
    }
}

/// Declare this process per-monitor DPI aware, before any window exists.
///
/// ⛔⛔ **THE DEAD MARGIN.** Mike, 2026-08-09, across five rounds of it: black or
/// white space outside the UI that no amount of resizing the frame would close.
/// The page's own numbers ended it — `screen=1707x1067` with `dpr=1`, on a
/// display that is 2560x1600. 1707x1067 *is* 2560x1600 ÷ 1.5, so the desktop is
/// at **150%** while the page believes it is at 100%. Both are only possible if
/// Windows is **virtualizing this process**.
///
/// ▶ **What that does.** A DPI-unaware window asking for 1440x900 is stretched
/// by the desktop compositor to 2160x1350 on screen. WebView2 is per-monitor
/// aware and renders at *true* pixels — 1440x900 — so its content covers
/// `1440/2160 = 0.667` of the window in each axis and the remaining third is
/// bare window. Two components, two different ideas of what a pixel is.
///
/// ⚠ **`main`, and as early as possible.** Awareness is a property of the
/// process that can only be set before the first window is created;
/// `system_scale`'s own doc already recorded that baseview makes the process
/// aware *after* `create()` has sized the editor, which is too late for the
/// sizing and too late for this. Doing it here means every measurement anyone
/// takes afterwards is in the same units.
///
/// ⚠ **Standalone only, and that is not a limitation.** In a plugin the *host*
/// owns the process and has already chosen its awareness; a plugin that changed
/// it would be reaching into Ableton's own rendering. If the same margin shows
/// up in a DAW it is that host's awareness against WebView2's, and it needs a
/// different answer.
///
/// ⚠ Failure is ignored on purpose: the call does not exist before Windows 10
/// 1703, and it fails harmlessly if awareness was somehow already set. Neither
/// is a reason to refuse to start.
#[cfg(windows)]
fn become_dpi_aware() {
    use std::ffi::c_void;

    // `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`, which is a sentinel pointer
    // rather than an enum — this is how the Windows headers define it.
    const PER_MONITOR_AWARE_V2: isize = -4;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetProcessDpiAwarenessContext(context: *mut c_void) -> i32;
    }

    // SAFETY: a documented entry point taking a documented sentinel value.
    unsafe {
        SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2 as *mut c_void);
    }
}

fn main() {
    // ⛔ **First, before anything opens a window.** See the doc above: this is
    // the fix for the dead margin, and it only works from here.
    #[cfg(windows)]
    become_dpi_aware();

    // TASK-063D. Must be first: it is what makes a crash before this point the
    // only kind that still reports nothing.
    #[cfg(windows)]
    report_hardware_faults();

    // TASK-P16. **This binary owns its thread's Windows message queue; a DAW
    // does not.** `baseview`'s standalone loop pumps with an `hwnd` filter, which
    // never retrieves thread messages — and WebView2 delivers its COM completions
    // as exactly those, so without this the window renders nothing at all. The
    // call is a no-op off Windows.
    //
    // ⛔ It belongs *here* and nowhere else. Calling it from the plugin would
    // drain the host's queue out from under Ableton or FL. `main` is the gate:
    // a host loads the library and never runs this function.
    nih_plug_webview::own_message_queue();

    // TASK-041T. **There is no host here, so the transport is ours to run.**
    // nih-plug's cpal backend reports `transport.playing = true` on every
    // block and offers no way to stop it — so without this a pattern loops
    // from the moment it is generated, Stop rewinds to zero and keeps playing,
    // and there is no pause at all. `main` is the gate for the same reason as
    // the line above: a host loads the library and never runs this function.
    freally_midi_master_plugin::mark_standalone();

    nih_export_standalone_with_args::<FreallyMidiMaster, _>(period_size_defaulted());
}

/// This process's arguments, with `--period-size 2048` added when nobody asked
/// for one.
///
/// ⛔⛔ **Because double-clicking the exe crashed it, every time.** Mike,
/// 2026-08-09: *"every time i try to open the exe it crashes."* Everything above
/// explains the mechanism — nih-plug's cpal backend defaults to **512** and
/// asserts when WASAPI hands it the 1056 this hardware actually gives — and the
/// answer had been *"pass `--period-size 2048`"*, carried by `package.json`.
///
/// ⚠ **That made the npm script load-bearing for correctness**, which is the
/// actual defect. A binary that aborts unless it is launched a particular way is
/// broken for everyone who launches it the obvious way, and the obvious way is
/// double-clicking it. Worse in `--release`, where `panic = "abort"` turns the
/// backend's panic from a dead audio thread into a dead process: the window gets
/// as far as appearing and then the whole thing goes.
///
/// ⚠ **A default, not an override.** An explicit `--period-size` still wins —
/// including a deliberately small one, because someone measuring latency has to
/// be able to ask for it. This only fills the gap when the flag is absent.
fn period_size_defaulted() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().collect();
    let asked = args
        .iter()
        .any(|arg| arg == "--period-size" || arg.starts_with("--period-size="));
    if !asked {
        args.push("--period-size".to_owned());
        args.push("2048".to_owned());
    }
    args
}
