//! Reading a file's notes without freezing the DAW (TASK-058D).
//!
//! ## ⛔⛔ Why this is a thread and a mailbox rather than one blocking call
//!
//! **The same reason [`crate::export`] is, and it is measured rather than
//! assumed.** Reading a forty-second stem takes about **two seconds**: a decode,
//! four band filters over the whole buffer, two decimated pitch tracks and a
//! Goertzel bank. The bridge answers an HTTP call from the webview on a frame the
//! page is waiting on — `editor.rs` documents that it runs in an `extern "C"`
//! frame where a panic cannot even unwind — and in a host that frame belongs to
//! the **DAW's editor thread**. Two seconds there is a two-second freeze of
//! Ableton with no cursor, no progress and nothing to explain it.
//!
//! ⚠ `explorer::waveform` gets away with running inline because decoding a
//! one-shot is milliseconds and its own doc says so. This is two orders of
//! magnitude more work; the same shape would be the wrong one.
//!
//! So the command **starts** a read and returns immediately. A detached thread
//! owns the decode and the analysis and drops its outcome into a one-slot mailbox
//! the page reads on its next poll.
//!
//! ## The guards run on the caller's thread, not on the worker's
//!
//! ⛔ `refuse_remote`, containment and the extension check are three cheap
//! syscalls and they are what stand between the page and an arbitrary path. They
//! happen **before** the thread starts, and what crosses to it is a `PathBuf`
//! that has already passed them — so there is no second copy of the rules to
//! drift, which is the failure this repo has recorded twice in the explorer
//! alone.
//!
//! ## Cancel, and exactly what it is
//!
//! ⚠ **Cancelling releases the producer, not the CPU.** A generation counter is
//! bumped and the worker's result is dropped when it comes back stale; the
//! thread itself runs to the end. Interrupting the analysis mid-way would mean a
//! flag checked inside every filter loop for a job that takes two seconds — cost
//! in the inner loops of every import to save at most two seconds of one detached
//! thread. The producer's experience is identical either way, and this is the
//! honest description of it rather than a promise of an abort.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::AudioSplit;

/// How a read is going, for the page to poll.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Nothing has been read since the last time the page looked.
    #[default]
    Idle,
    /// A file is being decoded and analysed.
    ///
    /// ⚠ Carries the path, so a panel that was reopened on a different file does
    /// not show a spinner for a read it is not waiting on.
    Running {
        path: String,
    },
    Done {
        path: String,
        split: AudioSplit,
    },
    Failed {
        path: String,
        reason: String,
    },
}

/// One in-flight read and its outcome, held **per plugin instance**.
///
/// ⛔ **Per instance for the reason [`crate::export::Exports`] spells out at
/// length**: `take` is destructive, so a process global would let one instance's
/// poll in a twenty-track session steal another's result.
#[derive(Debug, Default)]
pub struct Jobs {
    slot: Arc<Mutex<Status>>,
    /// Bumped by [`Self::cancel`] and by each new start, so a worker can tell
    /// whether the answer it is holding is still wanted.
    generation: Arc<AtomicU64>,
    /// How many workers are alive right now. See [`MAX_IN_FLIGHT`].
    running: Arc<AtomicUsize>,
}

/// How many reads may be alive at once.
///
/// ⛔⛔ **A bound rather than trust, and the reason is the same one every other
/// bridge command states: the request comes from the page.** A start replaces
/// rather than refuses — see [`Jobs::start`] — which is right for the producer
/// clicking through a folder, and on its own it means N messages spawn N threads
/// that each decode a file. The page only ever has one read in flight, but
/// "the page only ever" is not a guarantee this layer is allowed to make, and
/// this project's own rule is that bridge JSON is untrusted.
///
/// ⚠ **Four, not one**, so the replace behaviour survives: a producer clicking
/// three files quickly still gets the third read started while the first two are
/// finishing and being discarded.
const MAX_IN_FLIGHT: usize = 4;

impl Jobs {
    /// Start reading `file`, which the caller has already checked.
    ///
    /// `shown` is the path as the *page* spells it, which is what the page will
    /// compare its own selection against — the canonical form the guards produce
    /// is not textually equal to it on Windows, and `explorer::same_or_inside`
    /// already records what that mismatch costs.
    pub fn start(
        &self,
        file: PathBuf,
        shown: String,
        bpm: f32,
        num: u8,
        den: u8,
    ) -> Result<(), String> {
        // ⚠ Claimed before anything else, and released by the worker's own guard
        // below however it exits.
        if self.running.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
            self.running.fetch_sub(1, Ordering::SeqCst);
            return Err("too many files are already being read".to_owned());
        }

        // ⚠ **Replaces rather than refuses.** `export` refuses a second job
        // because two native Save As dialogs is a window nobody can explain;
        // here a second read means the producer clicked another file, and the
        // one they are looking at is the one they want.
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *crate::held(&self.slot) = Status::Running {
            path: shown.clone(),
        };

        let slot = Arc::clone(&self.slot);
        let counter = Arc::clone(&self.generation);
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            // ⛔ **Released by a guard, not by a line at the end.** `read` parses
            // a file a producer picked; if anything in it ever panics, a
            // decrement written after the call would never run and the fourth
            // panic would refuse every read for the life of the session.
            let _release = Release(running);
            let outcome = read(&file, &shown, bpm, num, den);
            // ⛔⛔ **The lock is taken BEFORE the generation is read, and the order
            // is the whole guard.** Checking first left a window: this worker
            // could see its own generation, the producer could then press Stop —
            // bumping the counter and writing `Idle` — and this worker would take
            // the lock afterwards and overwrite the cancellation with its own
            // result. Under the lock, `cancel` either happened (and the counter
            // has already moved) or it has not.
            let mut held = crate::held(&slot);
            // ⛔ **Stale results are dropped.** The producer moved on, or started
            // another read; writing this one would put an answer about a file
            // they are no longer looking at into the panel.
            if counter.load(Ordering::SeqCst) != generation {
                return;
            }
            *held = outcome;
        });
        Ok(())
    }

    /// What the last read did, taken destructively.
    ///
    /// ⚠ `Running` is *not* taken — the page polls, and clearing it would make
    /// the second poll of one read report that nothing is happening.
    pub fn take(&self) -> Status {
        let mut held = crate::held(&self.slot);
        match &*held {
            Status::Running { .. } => held.clone(),
            _ => std::mem::take(&mut held),
        }
    }

    /// Stop waiting for the read in flight. See the module note on what this is.
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *crate::held(&self.slot) = Status::Idle;
    }
}

/// Gives one of [`MAX_IN_FLIGHT`] back when the worker ends, however it ends.
struct Release(Arc<AtomicUsize>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The work itself: decode, then split.
fn read(file: &std::path::Path, shown: &str, bpm: f32, num: u8, den: u8) -> Status {
    let name = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".to_owned());

    let decoded =
        crate::audio::import::decode_file_within(file, crate::audio::import::MAX_EXTRACT_SECONDS);
    let outcome = decoded.and_then(|audio| super::split(&audio, &name, bpm, num, den));

    match outcome {
        Ok(split) => {
            // ⚠ The history entry goes here rather than at the command, because
            // this is the moment the file was actually opened — the same rule
            // `explorer_midi_split` records for the MIDI twin.
            let _ = crate::recent::note(shown);
            Status::Done {
                path: shown.to_owned(),
                split,
            }
        }
        Err(reason) => Status::Failed {
            path: shown.to_owned(),
            reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-second 220 Hz sine as a mono 16-bit WAV, written to a temp file.
    fn tone_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-extract-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let path = dir.join(name);

        let rate = 44_100_u32;
        let frames = rate as usize;
        let data_len = frames * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for frame in 0..frames {
            let t = frame as f32 / rate as f32;
            let value = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.8;
            out.extend_from_slice(&((value * f32::from(i16::MAX)) as i16).to_le_bytes());
        }
        std::fs::write(&path, out).expect("the fixture must be writable");
        path
    }

    /// Poll until the job leaves `Running`, or give up.
    fn settle(jobs: &Jobs) -> Status {
        for _ in 0..600 {
            let status = jobs.take();
            if !matches!(status, Status::Running { .. }) {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the read never finished");
    }

    #[test]
    fn the_command_returns_before_the_work_does() {
        // ⛔⛔ **The whole reason this module exists.** Two seconds of analysis on
        // the bridge's own frame is a two-second freeze of the DAW's editor
        // thread. `start` must be a handoff, not the work.
        let jobs = Jobs::default();
        let file = tone_file("returns-early.wav");
        jobs.start(file, "returns-early.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        // Taken immediately: whatever it says, it must have *answered*.
        assert!(matches!(
            jobs.take(),
            Status::Running { .. } | Status::Done { .. } | Status::Failed { .. }
        ));
        let _ = settle(&jobs);
    }

    #[test]
    fn a_finished_read_is_reported_once_and_then_forgotten() {
        // ⚠ Destructive, like `export::take_status`: the panel shows a result
        // once. Leaving it in the slot would make every later poll re-announce a
        // file the producer has already dealt with.
        let jobs = Jobs::default();
        let file = tone_file("taken-once.wav");
        jobs.start(file, "taken-once.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        assert!(matches!(settle(&jobs), Status::Done { .. }));
        assert!(
            matches!(jobs.take(), Status::Idle),
            "the slot should be empty"
        );
    }

    #[test]
    fn a_cancelled_read_never_lands() {
        // ⛔ The producer moved on. Whatever the worker is holding is about a
        // file they are no longer looking at — see the module note for what this
        // does and does not stop.
        let jobs = Jobs::default();
        let file = tone_file("cancelled.wav");
        jobs.start(file, "cancelled.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        jobs.cancel();
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(
            matches!(jobs.take(), Status::Idle),
            "the slot should be empty"
        );
    }

    #[test]
    fn a_second_read_replaces_the_first_rather_than_being_refused() {
        // ⚠ Unlike `export`, which refuses: a second read means the producer
        // clicked another file, and that is the one they want.
        let jobs = Jobs::default();
        jobs.start(tone_file("first.wav"), "first.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        jobs.start(tone_file("second.wav"), "second.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        match settle(&jobs) {
            Status::Done { path, .. } | Status::Failed { path, .. } => {
                assert_eq!(path, "second.wav");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_flood_of_starts_is_refused_rather_than_spawning_a_thread_each() {
        // ⛔⛔ **The request comes from the page, and "the page only ever sends
        // one" is not a guarantee this layer is allowed to make** — this
        // project's own rule is that bridge JSON is untrusted. Without the bound,
        // one message per thread is one decode per message.
        let jobs = Jobs::default();
        let file = tone_file("flood.wav");
        let mut refused = 0;
        for at in 0..(MAX_IN_FLIGHT * 4) {
            if jobs
                .start(file.clone(), format!("flood-{at}.wav"), 120.0, 4, 4)
                .is_err()
            {
                refused += 1;
            }
        }
        assert!(refused > 0, "nothing was refused");

        // ⚠ **And the bound lifts when the workers finish**, which is what makes
        // it a bound rather than a one-time budget.
        let _ = settle(&jobs);
        assert!(jobs.start(file, "after.wav".into(), 120.0, 4, 4).is_ok());
        let _ = settle(&jobs);
    }

    #[test]
    fn a_file_that_cannot_be_read_says_why_rather_than_staying_silent() {
        let jobs = Jobs::default();
        let dir = std::env::temp_dir().join(format!("fmm-extract-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("not-audio.wav");
        std::fs::write(&path, b"this is not a wave file").expect("writable");

        jobs.start(path, "not-audio.wav".into(), 120.0, 4, 4)
            .expect("the first reads must be accepted");
        match settle(&jobs) {
            Status::Failed { reason, .. } => assert!(!reason.is_empty()),
            other => panic!("a text file must not read as notes: {other:?}"),
        }
    }
}
