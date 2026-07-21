// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use freally_midi_master_lib::bugreport;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // `--crash-notice <pid>`: we are the tiny helper a dying app spawned, not the
    // app. Show the native error window, relaunch if the user says yes, and
    // leave. This must come FIRST — before any Tauri app is built — so the helper
    // never puts a second webview on screen, and never trips a single-instance
    // guard the day one is added.
    if bugreport::run_crash_notice(&args) {
        return;
    }
    // `--test-crash`: drill the crash loop on the shipped exe. Deliberately a
    // launch flag: there is no button and no IPC command behind it.
    bugreport::arm_test_crash(&args);
    // Opt-in bug reporting: a panic writes a SCRUBBED crash report to a local
    // file so the next launch can offer to report it. The report is never sent
    // anywhere without a click.
    bugreport::install_panic_hook();

    freally_midi_master_lib::run()
}
