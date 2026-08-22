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
// the fault reporter exists: a crash reporter that writes to a stream nobody attached
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
                // ⛔ **The one crash folder** — see `crash.rs`, whose own header
                // states the rule this used to break: a producer asked for
                // "the crash folder" should not have to be told which of two to
                // look in. This wrote its own `standalone-crash.log` beside it,
                // with a third hand-rolled `%APPDATA%` join to get there.
                freally_midi_master_plugin::crash::write("fault", &report);
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

/// How large the log may grow before a launch starts it over, in bytes.
///
/// ⚠ It is appended to on every run. Unbounded, a file written for one
/// unreproduced crash becomes a megabyte a producer never asked for; a megabyte
/// is many launches' worth of the log that actually matters, which is the last
/// one.
#[cfg(all(windows, not(debug_assertions)))]
const MAX_LOG_BYTES: u64 = 1_000_000;

/// Send nih-plug's log — panics included — to a file, because release has no
/// console (TASK-159).
///
/// ⛔⛔ **THE HALF OF THE CRASH REPORTER THAT WAS MISSING.** The fault filter above
/// catches *hardware faults* and this project's open standalone bug is not one:
/// Mike, *"opens up the console with the app and crashes before it even loads"*,
/// and three launches off a fresh build could not reproduce it. A **panic** is
/// the documented failure on this path — the module header spells out cpal's
/// buffer assert — and `nih_plug` does report panics, through `log_panics`, into
/// the `log` crate.
///
/// ▶ **Whose sink is `stderr`, and `windows_subsystem = "windows"` means there
/// is no stderr.** So in the build a producer actually double-clicks, a panic
/// wrote its message to a handle nobody owns and the process died silently. The
/// one failure mode most likely to be behind an unreproduced crash was the one
/// leaving no evidence.
///
/// ⚠ **The env var rather than a panic hook**, and that is not a style choice:
/// `nih_export_standalone` installs `log_panics` itself, which calls
/// `panic::set_hook` and would replace any hook set here. `NIH_LOG` is read
/// *inside* that setup and already accepts a file path — the module header says
/// so — so this routes the whole log, panics included, through the mechanism
/// nih-plug already has rather than racing it.
///
/// ⚠ **Release only**, matching `windows_subsystem` exactly and for the same
/// reason: a debug build has a console, and `npm run plugin:standalone` is a
/// debug build whose whole point is that everything prints there.
///
/// ⚠ **An explicit `NIH_LOG` still wins.** Someone debugging with
/// `set NIH_LOG=windbg` must not have it quietly overridden.
#[cfg(all(windows, not(debug_assertions)))]
fn log_to_a_file_since_there_is_no_console() {
    if std::env::var_os("NIH_LOG").is_some() {
        return;
    }
    let Some(base) = std::env::var_os("APPDATA") else {
        return;
    };
    let path = std::path::PathBuf::from(base)
        .join("Freally MIDI Master")
        .join("standalone.log");
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    // Started over rather than rotated: one previous run's log is worth keeping
    // and a numbered archive of them is not, for a file that exists to answer
    // "what happened the last time it died".
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(path) = path.to_str() {
        std::env::set_var("NIH_LOG", path);
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

    // TASK-159. **The other half of that, and the half the open crash needs.**
    // A hardware fault writes into the crash folder (`crash.rs`); a *panic* went
    // to a stderr that a `windows_subsystem = "windows"` build does not have.
    // ⚠ Panics now reach the crash folder too — `install_panic_hook` chains
    // rather than replaces — but this file is still what carries nih-plug's
    // ordinary log, which a panic's surrounding context lives in. Before
    // `nih_export_standalone_with_args` below, because that is what reads
    // `NIH_LOG`.
    #[cfg(all(windows, not(debug_assertions)))]
    log_to_a_file_since_there_is_no_console();

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

#[cfg(test)]
mod tests {
    /// The defaulting, over a supplied argument list.
    ///
    /// ⚠ **Split from [`super::period_size_defaulted`] so it can be tested at
    /// all.** That one reads `std::env::args()`, which a test cannot set — and
    /// the rule it applies is the whole of the fix, so leaving it unreachable
    /// meant the one thing standing between a producer and *"every time i try to
    /// open the exe it crashes"* had no gate on it.
    fn defaulted(args: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let asked = args
            .iter()
            .any(|arg| arg == "--period-size" || arg.starts_with("--period-size="));
        if !asked {
            args.push("--period-size".to_owned());
            args.push("2048".to_owned());
        }
        args
    }

    #[test]
    fn the_default_matches_what_the_binary_actually_does() {
        // ⛔ The two implementations must not drift: this is the gate, and a copy
        // that had diverged would be a gate on nothing. Compared over the one
        // input `period_size_defaulted` can be called with in a test — the
        // process's own arguments, whatever the harness passed.
        let real = super::period_size_defaulted();
        let own: Vec<String> = std::env::args().collect();
        let borrowed: Vec<&str> = own.iter().map(String::as_str).collect();
        assert_eq!(real, defaulted(&borrowed));
    }

    #[test]
    fn double_clicking_the_exe_gets_a_buffer_big_enough_to_survive_wasapi() {
        // ⛔⛔ **Mike, 2026-08-09: *"every time i try to open the exe it
        // crashes."*** nih-plug's cpal backend defaults to 512 and asserts when
        // WASAPI hands it the 1056 this hardware actually gives — and in release,
        // where `panic = "abort"`, that assert takes the whole process rather
        // than one audio thread. The answer had been "launch it through the npm
        // script", which makes a build correct only when started a particular
        // way; the obvious way is double-clicking it.
        assert_eq!(
            defaulted(&["standalone.exe"]),
            vec!["standalone.exe", "--period-size", "2048"],
        );
    }

    #[test]
    fn an_explicit_period_size_still_wins() {
        // ⚠ **A default, not an override.** Somebody measuring latency has to be
        // able to ask for a small buffer, including one small enough to hit the
        // assert above — that is their call to make.
        assert_eq!(
            defaulted(&["standalone.exe", "--period-size", "256"]),
            vec!["standalone.exe", "--period-size", "256"],
        );
        // Both spellings, because nih-plug accepts both and a producer who wrote
        // the `=` form would otherwise get 2048 appended after their own value.
        assert_eq!(
            defaulted(&["standalone.exe", "--period-size=256"]),
            vec!["standalone.exe", "--period-size=256"],
        );
    }

    #[test]
    fn other_flags_are_left_alone() {
        // ⚠ The arguments are passed through, not rebuilt: `--tempo`, the audio
        // and MIDI device flags and `--help` are all nih-plug's, and this
        // function's only business is the one it is named for.
        assert_eq!(
            defaulted(&["standalone.exe", "--tempo", "92"]),
            vec!["standalone.exe", "--tempo", "92", "--period-size", "2048"],
        );
    }
}
