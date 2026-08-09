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
                eprintln!(
                    "\n[crash] {what} (0x{:08X}) at {:?}\n{}",
                    record.code,
                    record.address,
                    std::backtrace::Backtrace::force_capture()
                );
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

fn main() {
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

    nih_export_standalone::<FreallyMidiMaster>();
}
