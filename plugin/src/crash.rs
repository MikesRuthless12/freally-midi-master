//! Getting a crash onto disk, from either half of the app (TASK-093).
//!
//! ⛔⛔ **Two things can take the window down and neither left a trace.** A Rust
//! panic aborts the process — `release` is built `panic = "abort"`, so it takes
//! the host DAW with it — and a React render throw unmounts the page, which in a
//! hosted plugin is a dead rectangle inside the producer's project. Before this,
//! the only report either produced was whatever the producer could describe from
//! memory.
//!
//! ⚠ **One folder, two writers.** The panic hook and the page's `report_crash`
//! both land in `%APPDATA%\Freally MIDI Master\crashes\`, beside `takes.json` and
//! `recent.json`. A producer asked for "the crash folder" should not have to be
//! told which of two to look in, and a page throw that follows a Rust panic
//! should sit next to it in time order.
//!
//! ⛔ **Never panics, and that is not a style preference.** The panic hook calls
//! [`write`]; a panic inside a panic hook is an immediate abort with the
//! original message lost, which is precisely the failure this module exists to
//! prevent. Every path here returns rather than unwrapping, and a crash that
//! cannot be written is silently not written — the producer already has a bigger
//! problem than a missing log.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::presets::data_dir;

/// How many crash logs to keep before the oldest is dropped.
///
/// ⚠ **A cap rather than none, because this folder is never read by the app.**
/// Nothing prunes it on the producer's behalf and nothing shows them it is
/// growing, so an uncapped folder is one that quietly accumulates for the life
/// of the install. Twenty is far more than anyone needs to diagnose a repeat and
/// small enough to attach to a bug report whole.
const KEEP: usize = 20;

/// `%APPDATA%\Freally MIDI Master\crashes`, or `None` where there is no data
/// directory to speak of.
fn crash_dir() -> Option<PathBuf> {
    data_dir().map(|base| base.join("crashes"))
}

/// Seconds since the epoch, for the filename.
///
/// ⚠ **Not a formatted date, deliberately.** Formatting one needs a calendar
/// crate — a dependency, a licence to clear in `deny.toml` and something to
/// audit — to produce a name nothing parses. Sorting is what the name is for,
/// and an integer sorts.
fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Write one crash report. Returns the path when it landed.
///
/// `kind` becomes part of the filename so a panic and a page throw are told
/// apart without opening either.
pub fn write(kind: &str, detail: &str) -> Option<PathBuf> {
    write_in(&crash_dir()?, kind, detail)
}

/// The same, into a directory the caller names.
///
/// ⛔⛔ **This split exists BECAUSE OF A TEST THAT LIED, and the failure is worth
/// keeping.** The first cut of the tests pointed `crash_dir` at a temp folder by
/// setting `%APPDATA%` — a **process-global** — and cargo runs tests in parallel
/// threads, so four cases raced on one variable and each pruned inside another's
/// folder. It presented as `prune` leaving 25 files under a cap of 20, which
/// reads exactly like a bug in the pruning arithmetic. `set_var` is `unsafe` for
/// this reason. Taking the directory as an argument removes the shared mutable
/// state rather than serialising the tests around it.
fn write_in(dir: &std::path::Path, kind: &str, detail: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;

    // ⛔ **One reading of the clock for the whole report.** The name and the
    // `at:` line are the same moment, and taking the stamp twice made them two
    // independent readings separated by a `create_dir_all`, up to sixteen
    // `open` calls and a `prune` — so a report could be named `…-07-panic.log`
    // and say `at: …08` inside. Whoever is matching a file against a producer's
    // "it fell over at about half past" should not have to know which of the
    // two to believe.
    let at = stamp();

    // ⚠ The stamp can repeat within a second — a panic that cascades writes
    // more than once — so the file is opened `create_new` and stepped rather
    // than truncating a report written moments earlier.
    let mut file = None;
    let mut path = dir.join(format!("crash-{at}-{kind}.log"));
    for attempt in 0..16 {
        if attempt > 0 {
            path = dir.join(format!("crash-{at}-{kind}-{attempt}.log"));
        }
        match std::fs::File::create_new(&path) {
            Ok(handle) => {
                file = Some(handle);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }

    let mut file = file?;
    writeln!(file, "Freally MIDI Master {}", env!("CARGO_PKG_VERSION")).ok()?;
    writeln!(file, "kind: {kind}").ok()?;
    writeln!(file, "at:   {at}").ok()?;
    writeln!(file).ok()?;
    file.write_all(detail.as_bytes()).ok()?;

    prune(dir);
    Some(path)
}

/// Drop the oldest reports past [`KEEP`].
///
/// ⚠ **By filename, not by mtime.** The name carries the stamp this module
/// wrote, and a folder copied between machines or restored from a backup has
/// mtimes that say when it was copied rather than when the app fell over.
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "log")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("crash-"))
        })
        .collect();
    if logs.len() <= KEEP {
        return;
    }
    logs.sort();
    for path in &logs[..logs.len() - KEEP] {
        let _ = std::fs::remove_file(path);
    }
}

/// Send every Rust panic to a file before the process goes.
///
/// ⛔ **Chained, never replaced.** `plugin/src/bin/standalone.rs` records that
/// nih-plug installs its own hook and would replace one set here; the inverse is
/// equally true, and a hook that dropped the previous one would take the
/// framework's logging with it. This calls through to whatever was there.
///
/// ⚠ **Idempotent HERE, rather than at the caller.** Installing twice would
/// chain the hook to itself and write every report twice, and a host may
/// instantiate the plugin several times in one process. The guard lives with
/// the hazard so a second entry point — the standalone bin already calls
/// [`write`] — cannot acquire the bug by forgetting a `Once` of its own.
pub(crate) fn install_panic_hook() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // `Backtrace::force_capture` needs no env var, which matters: a
            // producer hitting this once is never going to reproduce it with
            // `RUST_BACKTRACE` set, so the one capture we get has to be the
            // useful one.
            let detail = format!("{info}\n\n{}", std::backtrace::Backtrace::force_capture());
            let _ = write("panic", &detail);
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own.
    ///
    /// ⛔ **No environment variable is touched, and that is the point** — see
    /// [`write_in`]. The name is the test's, so parallel cases cannot collide.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-crash-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_report_lands_with_the_detail_in_it() {
        let dir = temp("write");
        let path = write_in(&dir, "page", "TypeError: the piano roll fell over").expect("a path");

        let body = std::fs::read_to_string(&path).expect("readable");
        assert!(
            body.contains("the piano roll fell over"),
            "the report does not carry what it was given: {body}"
        );
        // ⛔ The kind is in the name, so a panic and a page throw are told apart
        // without opening either.
        assert!(path.to_string_lossy().contains("page"));
    }

    #[test]
    fn two_reports_in_one_second_do_not_overwrite_each_other() {
        // ⛔ A cascading panic writes more than once inside one stamp.
        // Truncating would lose the first report, which is the one that says
        // what started it.
        let dir = temp("collide");
        let first = write_in(&dir, "panic", "the first").expect("first");
        let second = write_in(&dir, "panic", "the second").expect("second");

        assert_ne!(first, second);
        assert!(std::fs::read_to_string(&first)
            .unwrap()
            .contains("the first"));
        assert!(std::fs::read_to_string(&second)
            .unwrap()
            .contains("the second"));
    }

    #[test]
    fn the_folder_is_capped_and_the_newest_survives() {
        let dir = temp("prune");
        // Written by hand so the names are ordered and the count is exact —
        // `write_in` would collide them all into one second.
        for at in 0..(KEEP + 5) {
            std::fs::write(dir.join(format!("crash-{at:04}-panic.log")), "x").expect("write");
        }

        // One more through the real path, which is what prunes.
        let newest = write_in(&dir, "page", "the newest").expect("newest");

        let left = std::fs::read_dir(&dir).unwrap().count();
        assert!(
            left <= KEEP,
            "the folder grew to {left} past a cap of {KEEP}"
        );
        assert!(
            std::fs::read_to_string(&newest)
                .unwrap()
                .contains("the newest"),
            "pruning dropped the report it had just written"
        );
    }
}
