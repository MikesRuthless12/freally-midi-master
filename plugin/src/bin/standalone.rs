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

use freally_midi_master_plugin::FreallyMidiMaster;
use nih_plug::prelude::*;

fn main() {
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

    nih_export_standalone::<FreallyMidiMaster>();
}
