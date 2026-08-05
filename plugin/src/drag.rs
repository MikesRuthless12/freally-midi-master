//! Dragging a generated part out of the plugin and into the DAW
//! (TASK-063C / FMM-S03).
//!
//! Mike, 2026-08-05: *"you need to be able to drag each generator's midi or
//! audio from the generator to the DAW and ensure it shows a preview of what
//! you are dragging"* … *"same with the song arrangement"*.
//!
//! ## ⛔ An HTML5 drag inside a webview is not an OS file drag
//!
//! The webview hands a `dragstart` to the *page*, not to the window server, so
//! the DAW never sees a file and the drag dies at the edge of the plugin
//! window. What is needed is a **native drag source**, and there is one per
//! platform: `DoDragDrop` on Windows, `NSFilePromiseProvider` on macOS, XDND on
//! Linux. [`platform`] is that seam, and today only Windows is behind it —
//! [`supported`] is what stops the other two offering an affordance that does
//! nothing.
//!
//! ## Two calls, not one, and the split is the whole design
//!
//! ⛔ **Rendering must not happen on the frame the page is waiting on.** The
//! bridge answers over the custom protocol, and that handler runs on the thread
//! a DAW draws its editor from — `export.rs` documents at length what putting
//! slow work there costs. Rendering eight lanes of audio is exactly that kind
//! of work. So:
//!
//! 1. **`prepare`** returns immediately and renders on a detached thread,
//!    dropping the outcome into a one-slot mailbox — the same shape, and for
//!    the same reason, as [`crate::export::Exports`].
//! 2. The page polls **`status`** until the bytes are on disk.
//! 3. **`start`** enters the platform's modal drag loop, which is cheap to
//!    enter once the files exist.
//!
//! ⚠ **`status` does not consume `Ready`, where `export::take_status` consumes
//! everything terminal.** The difference is deliberate: an export's `Done` is
//! the end of the story, but a drag's `Ready` is what `start` then needs. Only
//! `Failed` is taken.
//!
//! ## ⛔ Why the files are written to disk rather than promised
//!
//! Windows can carry a *promised* file — `CFSTR_FILEDESCRIPTOR` plus
//! `CFSTR_FILECONTENTS` — whose bytes are produced only if something accepts
//! the drop. That is the shape Explorer and Outlook use, and it is what this
//! project's own notes assumed would be needed. It is **not** what shipped, for
//! two reasons that are worth not re-litigating:
//!
//! - **DAWs accept `CF_HDROP`.** It is what a file dragged out of Explorer
//!   carries, so it is the format every drop target has had to handle for
//!   thirty years. A promised file is a narrower path with a real chance of
//!   dropping *nothing* into Ableton — the readout-that-lies failure this
//!   project has now recorded five times, and the one Mike caught himself with
//!   the KIT panel.
//! - **The bytes have to exist on disk either way.** Ableton *references*
//!   dropped audio by path until the producer runs Collect All and Save; it
//!   does not copy it into the set. A promise would only move the question of
//!   where the file lives from us to the host.
//!
//! What the promise would have bought is not rendering on a cancelled drag, and
//! `prepare`/`start` already buys that back: nothing is rendered until the
//! producer has actually begun the gesture.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, SystemTime};

use engine::pattern::Pattern;
use serde::Serialize;

#[cfg(windows)]
#[path = "drag/windows.rs"]
mod platform;

#[cfg(not(windows))]
#[path = "drag/unsupported.rs"]
mod platform;

/// Whether this build can start an OS file drag at all.
///
/// ⛔ **The page asks before it renders a drag handle.** macOS and Linux have no
/// native drag source yet, and a handle that looks draggable and drops nothing
/// is worse than no handle: the producer blames their DAW. Those platforms keep
/// the Export path, and the chip keeps saying "Export" — the same rule
/// TASK-131F already followed for the same reason.
///
/// ⛔⛔ **`standalone` is not a nicety — a drag there aborts the process.** The
/// standalone calls `own_message_queue()`, so its RPC handler runs from inside
/// baseview's window procedure, and `DoDragDrop`'s modal loop dispatches
/// straight back into it. `drag/windows.rs`'s header has the full mechanism.
/// The honest condition is "this process pumps its own message queue"; today
/// that is exactly the standalone, and nothing else may call
/// `own_message_queue()` without being refused here as well.
pub fn supported_in(standalone: bool) -> bool {
    platform::SUPPORTED && !standalone
}

/// What a platform with no drag source says, in one place.
///
/// ⚠ Both the refusal above the seam and the one below it said this, word for
/// word — two copies of a sentence a producer reads is two chances for them to
/// stop agreeing about what to do instead.
pub(crate) const NO_DRAG_SOURCE: &str = "this platform has no drag source yet — use Export instead";

/// How a drag is going, for the page to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// No drag has been prepared since the last one finished.
    Idle,
    /// The bytes are being rendered and written.
    Preparing,
    /// The files are on disk and `start` can hand them to the OS.
    ///
    /// ⚠ **Carries nothing.** It used to carry the file names "for the preview
    /// to label", and nothing ever read them: the picture is drawn page-side
    /// from the row's own label before this is ever asked, so the names could
    /// not reach it even in principle. A field that is plumbed across the
    /// bridge and rendered nowhere reads as a contract the page depends on.
    Ready,
    Failed {
        reason: String,
    },
}

/// What the drop target did with it.
///
/// ⚠ **None of these is a failure** — letting go over nothing is the ordinary
/// way out of a drag, and reporting it as an error would train people to ignore
/// the one message that matters. Same rule as a closed Save As dialog.
///
/// ⛔ **`Refused` and `Cancelled` are separate because one of them may have
/// taken the files and the other cannot have.** Collapsing them is what made a
/// successful drop delete the clip it had just handed over; [`Drags::start`]
/// carries the mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Dropped {
    /// A target took it and said so.
    Copied,
    /// A target had it and reported no effect. ⚠ It may still have taken the
    /// data — an asynchronous target extracts after the call returns.
    Refused,
    /// The gesture ended without a drop. Nothing ever saw the files.
    Cancelled,
}

/// One prepared drag, held **per plugin instance**.
///
/// ⛔ **Not a process global, for the reason [`crate::export::Exports`] spells
/// out**: a DAW can load this plugin on twenty tracks in one process, and a
/// shared slot means instance B's poll reads instance A's files. Here that is
/// worse than a stolen toast — `start` would hand the OS a path belonging to a
/// pattern on another track, and the producer would get the wrong loop with no
/// indication anything went wrong.
#[derive(Debug, Default)]
pub struct Drags {
    slot: Arc<Mutex<Stage>>,
    /// Hands out the id each prepare stamps on its `Preparing`.
    next_id: AtomicU64,
}

#[derive(Debug, Default)]
enum Stage {
    #[default]
    Idle,
    /// A render is in flight, and this is *which* one.
    ///
    /// ⛔ **The id is what lets a render be disowned.** `cancel` cannot reach
    /// into a running thread, so it moves the slot out of `Preparing` instead;
    /// the thread then finds an id that is not its own and takes its own folder
    /// back rather than publishing files nobody asked for. Without it an
    /// abandoned press left its render on disk forever — and for audio the
    /// sweep is forbidden to remove it at any age.
    Preparing(u64),
    Ready(Prepared),
    Failed(String),
}

#[derive(Debug, Clone)]
struct Prepared {
    /// Absolute paths, in the order they should be handed to the drop target.
    paths: Vec<PathBuf>,
    /// The folder they were spooled into, so an abandoned drag can take it back.
    dir: PathBuf,
}

/// What is being picked up.
///
/// ⛔ **The whole subject travels, un-rendered, so the work happens on the
/// thread that is allowed to do it.** A song used to be flattened in the
/// command handler — five walks of the entire arrangement, each allocating a
/// note per emitted note and sorting every lane, on the thread a DAW draws its
/// editor from, for every press of the chip. That is exactly the stall the
/// `prepare`/`start` split exists to avoid, and it was the one render-shaped
/// piece still sitting on the wrong side of it.
#[derive(Debug, Clone)]
pub(crate) enum Subject {
    /// The parts on screen, cut into files one way or another.
    Patterns {
        patterns: Vec<Pattern>,
        cut: crate::export::Cut,
    },
    /// The whole arrangement, one file per part.
    ///
    /// ⚠ Boxed because a `Song` is far larger than a `Vec<Pattern>`, and an
    /// enum is as big as its largest variant wherever it is held.
    Song(Box<engine::pattern::Song>),
}

/// The picture the producer sees under the cursor while they drag.
///
/// Mike, 2026-08-05: *"ensure it shows a preview of what you are dragging"*.
///
/// ⛔ **Drawn by the page and carried down here, rather than drawn natively.**
/// A drag image has to be an OS-level bitmap — an HTML5 `setDragImage` lives
/// inside the webview and vanishes the moment the cursor crosses into the DAW,
/// which is precisely where the producer needs to see it. What the page is
/// better at is *drawing*: it already knows how a clip's notes look in this
/// app, and it can put the stem's name beside them in the right font. So the
/// page renders a canvas and hands over the pixels.
/// ⛔⛔ **The fields are private, and that is a memory-safety boundary, not
/// style.** `windows.rs` builds a 32-bpp DIB from `width`x`height` and then
/// writes `rgba.len()` bytes into it through a raw slice. Those two agree only
/// because [`Preview::new`] refuses a buffer whose length is not exactly
/// `width * height * 4` — a 32-bpp DIB's stride is always `width * 4`, so the
/// allocation is exactly that many bytes and not one more. A struct literal
/// built anywhere else in this crate, with a large `width` and a short `rgba`,
/// would be an out-of-bounds write **inside somebody's DAW**. The constructor
/// is the only way in, so the invariant cannot be sidestepped by accident.
///
/// ⚠ **The `allow` is what keeps the non-Windows legs of CI green**, and it is
/// narrow on purpose. [`Preview::parts`] is the only reader and it is called
/// from `drag/windows.rs`, so on Linux and macOS these fields are written and
/// never read — and CI builds with `-D warnings`. The type is still constructed
/// and *validated* on every platform, deliberately: a malformed drag image must
/// be refused identically everywhere, not only where one could be drawn.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct Preview {
    width: u32,
    height: u32,
    /// Straight (not premultiplied) RGBA, top row first — exactly what
    /// `CanvasRenderingContext2D.getImageData` produces.
    rgba: Vec<u8>,
}

/// The largest drag image that will be accepted.
///
/// ⛔ **A bound, because the pixels arrive as JSON from the webview.** Width and
/// height are whatever that JSON said, and they are multiplied together to size
/// an allocation. Well past any drag image worth looking at.
const MAX_PREVIEW_EDGE: u32 = 512;

impl Preview {
    /// Build one from what the page sent, or say why it is not usable.
    ///
    /// ⚠ The length check is the one that matters: a `width`/`height` that does
    /// not match the buffer is how a decode walks off the end of it.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 || width > MAX_PREVIEW_EDGE || height > MAX_PREVIEW_EDGE {
            return Err(format!("{width}x{height} is not a usable drag image"));
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(format!(
                "the drag image is {} bytes and {width}x{height} needs {expected}",
                rgba.len()
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// The picture's size, and the bytes that are guaranteed to fill it exactly.
    ///
    /// ⚠ Handed out together, so a caller cannot pair one drag image's
    /// dimensions with another's pixels.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn parts(&self) -> (u32, u32, &[u8]) {
        (self.width, self.height, &self.rgba)
    }
}

impl Drags {
    /// Render `patterns` and spool them, ready to be dragged.
    ///
    /// Returns as soon as the thread is running. `kit` is `Some` for audio and
    /// `None` for MIDI, and it must be the kit that is **playing** — see
    /// [`crate::export::Exports::start_pattern_stems`] for why resolving it
    /// here instead would hand the producer a stem that does not match what
    /// they heard.
    ///
    /// ⛔ **`standalone` is taken here rather than checked by the caller**, so
    /// the refusal cannot be skipped by forgetting it at a call site. A drag in
    /// the standalone aborts the process — see [`supported_in`].
    pub(crate) fn prepare(
        &self,
        subject: Subject,
        kit: Option<Arc<crate::audio::kit::Kit>>,
        standalone: bool,
    ) -> Result<(), String> {
        if !supported_in(standalone) {
            return Err(NO_DRAG_SOURCE.to_owned());
        }
        // ⛔ **Every prepare disowns the one before it, and that is what makes a
        // second press legal.** The first cut refused while a render was in
        // flight — but `cancel` deliberately leaves a running render alone, so
        // a producer who tapped a chip and pressed it again a moment later was
        // told "a drag is already being prepared" for the drag they were
        // currently making. Worse, the disowned render then published its files
        // into the slot and **nothing ever deleted them**: for audio the sweep
        // refuses to, at any age, so every abandoned press leaked its render.
        //
        // A generation solves both. The thread carries the number it started
        // with; if it no longer matches when it finishes, it is nobody's — so
        // it takes its own folder back and publishes nothing.
        let mine = self.next_id.fetch_add(1, Ordering::Relaxed);
        let slot = {
            let mut slot = self.lock()?;
            // Anything already prepared is dropped here rather than reused. A
            // producer who starts a second gesture has changed their mind about
            // what they are dragging, and handing them the previous selection is
            // the wrong loop with no way to tell. ⚠ Its folder goes with it, for
            // the reason [`Self::cancel`] gives: nothing was ever dropped, so
            // nothing can be referencing it.
            discard(std::mem::replace(&mut *slot, Stage::Preparing(mine)));
            Arc::clone(&self.slot)
        };

        std::thread::spawn(move || {
            let outcome = match render_and_spool(subject, kit.as_deref()) {
                Ok(prepared) => Stage::Ready(prepared),
                Err(reason) => Stage::Failed(reason),
            };
            // ⛔ **Published only into the slot that is still waiting for
            // *this* render.** The claim above and this are a pair: a return
            // that forgets to publish leaves the page polling `preparing` for
            // the rest of the session, which is the failure `export.rs` records
            // the same pairing for. But publishing into a slot that has moved
            // on is the other half — those files are nobody's, and nothing
            // would ever delete them.
            if let Ok(mut slot) = slot.lock() {
                if matches!(*slot, Stage::Preparing(waiting) if waiting == mine) {
                    *slot = outcome;
                } else {
                    discard(outcome);
                }
            }
        });
        Ok(())
    }

    /// What the page polls.
    ///
    /// ⚠ **`Ready` survives being read; `Failed` does not.** `start` still needs
    /// the paths, so consuming `Ready` the way `export::take_status` consumes
    /// `Done` would make the poll that reports success also destroy it. A
    /// failure is taken, because a message left in place re-announces itself on
    /// every tick.
    pub fn status(&self) -> Status {
        let Ok(mut slot) = self.lock() else {
            return Status::Failed {
                reason: LOCK_LOST.to_owned(),
            };
        };
        match &*slot {
            Stage::Idle => Status::Idle,
            Stage::Preparing(_) => Status::Preparing,
            Stage::Ready(_) => Status::Ready,
            Stage::Failed(reason) => {
                let taken = Status::Failed {
                    reason: reason.clone(),
                };
                *slot = Stage::Idle;
                taken
            }
        }
    }

    /// Hand the spooled files to the OS and block until the drag ends.
    ///
    /// ⛔ **This blocks for as long as the producer holds the mouse down**, and
    /// that is not avoidable: `DoDragDrop` owns a modal loop, and it has to run
    /// on the thread the drag started from. The page's `fetch` stays outstanding
    /// for the whole gesture, which is correct — there is nothing for it to do
    /// until the drop lands.
    /// `preview` is the picture to put under the cursor, if the page drew one.
    ///
    /// ⛔ **Given here rather than to `prepare`, and the move is worth 96 KB a
    /// click.** It used to arrive with the prepare — which runs on every press,
    /// including the presses that are ordinary clicks — so a whole RGBA image
    /// was read back from the canvas, base64'd, JSON-encoded, posted, parsed and
    /// decoded every time anybody touched a chip. The page draws it once the
    /// gesture is real instead, and the bytes ride in on the call that actually
    /// needs them.
    pub fn start(&self, preview: Option<Preview>) -> Result<Dropped, String> {
        let prepared = {
            let mut slot = self.lock()?;
            match std::mem::replace(&mut *slot, Stage::Idle) {
                Stage::Ready(prepared) => prepared,
                Stage::Preparing(waiting) => {
                    // Put it back, id and all: the thread is still running and
                    // is entitled to publish into this slot.
                    *slot = Stage::Preparing(waiting);
                    return Err("the files are not written yet".to_owned());
                }
                Stage::Failed(reason) => return Err(reason),
                Stage::Idle => return Err("there is nothing prepared to drag".to_owned()),
            }
        };
        let dropped = match platform::drag(&prepared.paths, preview.as_ref()) {
            Ok(dropped) => dropped,
            Err(error) => {
                // ⛔ **The slot is already `Idle`, so nothing else will ever
                // reclaim this.** The first cut let `?` drop `prepared` here,
                // which leaked the whole render — and if the failure is
                // systematic (OLE refusing this thread, say) every attempt
                // leaked another folder of multi-megabyte WAVs that the sweep
                // is forbidden to delete.
                remove(&prepared.dir);
                return Err(error);
            }
        };
        match dropped {
            // ⛔ **Deleted only when nothing can possibly hold it.**
            // `DRAGDROP_S_CANCEL` means the gesture ended without a drop at
            // all, so these files were never handed to anyone.
            Dropped::Cancelled => remove(&prepared.dir),
            // ⛔⛔ **A refusal is NOT a cancel, and treating it as one deleted
            // the producer's clip.** A target that reports `DROPEFFECT_NONE`
            // may still have taken the data — `IDataObjectAsyncCapability`
            // extracts *after* `DoDragDrop` returns, and some targets simply
            // never set an effect on a successful drop. The old code asserted
            // "there is no case where one accepted them and the call still
            // reports a cancel", which was a claim about every drop target on
            // the system rather than anything the API guarantees.
            //
            // ⚠ So audio is kept and MIDI is not: a dropped `.wav` is
            // referenced by path and losing it empties a clip, while a `.mid`
            // is copied into the project by every DAW and its spool is nobody's
            // either way. Keeping a file nobody wanted costs temp space;
            // deleting one somebody has costs their record.
            Dropped::Refused => {
                if !holds_audio(&prepared.dir) {
                    remove(&prepared.dir);
                }
            }
            Dropped::Copied => {}
        }
        Ok(dropped)
    }

    /// Forget anything prepared, because the gesture ended without a drag.
    ///
    /// A `mousedown` that never crosses the threshold is an ordinary click, and
    /// leaving its payload in the slot would make the *next* drag start from a
    /// stale selection.
    ///
    /// ⛔ **The spooled folder goes too.** Preparing renders and writes, so a
    /// mis-click on an Audio chip with the drum lanes split writes eight WAVs —
    /// and [`sweep`] deliberately never deletes a folder containing audio,
    /// because it cannot tell an abandoned render from a loop somebody's
    /// session is pointing at. Here it can: nothing was ever handed to a drop
    /// target, so nothing can be referencing it.
    /// ⛔⛔ **`Preparing` is cleared too, and skipping it was a leak.** The old
    /// version left a running render alone on the reasoning that its thread
    /// would overwrite the slot anyway — which is exactly what went wrong:
    /// the render landed, the files sat in a slot the page had stopped
    /// watching, and nothing ever deleted them. Moving out of `Preparing`
    /// **is** how a render is disowned: the thread finds an id that is no
    /// longer its own and takes its own folder back. Releasing before the bytes
    /// exist is the single most likely abandonment there is.
    pub fn cancel(&self) {
        if let Ok(mut slot) = self.lock() {
            discard(std::mem::replace(&mut *slot, Stage::Idle));
        }
    }

    /// ⚠ **One lock idiom, so the poisoning policy is one thing to read.** The
    /// first cut had three — this helper, a `let Ok(..) else`, and an `if let`
    /// that silently did nothing — which meant working out what a poisoned
    /// mutex does here took reading all three.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Stage>, String> {
        self.slot.lock().map_err(|_| LOCK_LOST.to_owned())
    }
}

/// What a poisoned slot reads as. One string, because it reaches the page.
const LOCK_LOST: &str = "the drag state is unusable";

/// Give back the folder a stage was holding, if it was holding one.
fn discard(stage: Stage) {
    if let Stage::Ready(prepared) = stage {
        remove(&prepared.dir);
    }
}

/// ⚠ Failure is ignored on purpose: a folder that will not delete is a week of
/// temp space, and there is nothing useful to say to the producer about it.
fn remove(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Render the subject and write it where the drop target can read it.
///
/// ⛔ **Everything expensive is here, on the spawned thread**, including
/// flattening a song — see [`Subject`] for what that cost when it was not.
fn render_and_spool(
    subject: Subject,
    kit: Option<&crate::audio::kit::Kit>,
) -> Result<Prepared, String> {
    // ⛔ **The export's own functions, not second copies.** A hi-hat loop
    // dragged into Ableton and the same loop exported to a folder must be the
    // same file; two implementations of that is how they stop agreeing.
    let files = match subject {
        Subject::Patterns { patterns, cut } => crate::export::stem_files(&patterns, cut, kit),
        Subject::Song(song) => crate::export::stem_files(
            &crate::export::song_stem_patterns(&song),
            crate::export::Cut::Parts,
            // ⛔ MIDI, and the caller has already refused the audio half — a
            // song is minutes long and rendering one needs progress a producer
            // can watch. This is belt and braces: `None` here cannot render.
            None,
        ),
    };
    if files.is_empty() {
        return Err("there is nothing here to drag".to_owned());
    }

    SWEEP.call_once(sweep);
    let dir = fresh_spool_dir()?;
    let paths = crate::export::spill(&dir, &files)?;
    Ok(Prepared { paths, dir })
}

/// Decode standard base64 — the drag image, and nothing else.
///
/// ⛔ **Written here rather than pulled in, and the reason is the dependency
/// graph, not pride.** `base64` is in this workspace's lockfile and **not** in
/// the plugin's own tree, so taking it would add a crate to the shared library
/// that loads into somebody's DAW — for one call site decoding pixels the page
/// next door produced. Twenty lines with a test is the cheaper trade.
///
/// ⚠ Rejects anything outside the standard alphabet rather than skipping it.
/// The input is JSON from the webview; quietly ignoring stray bytes is how a
/// corrupted image decodes to a *shifted* one instead of an error.
pub(crate) fn from_base64(text: &str) -> Result<Vec<u8>, String> {
    fn sextet(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let body = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in body.bytes() {
        let value = sextet(byte).ok_or("the drag image is not base64")?;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // ⚠ Whatever is left is the padding's own bits, and they must be zero. A
    // non-zero remainder means the encoder and this disagree about the length,
    // which is worth an error rather than a truncated image.
    if bits >= 6 || (acc & ((1 << bits) - 1)) != 0 {
        return Err("the drag image is truncated".to_owned());
    }
    Ok(out)
}

/// Where a drag's files live until the producer's DAW is done with them.
fn spool_root() -> PathBuf {
    std::env::temp_dir()
        .join("Freally MIDI Master")
        .join("drag")
}

/// A folder no other drag has ever written into, created empty.
///
/// ⛔ **A counter, not the artist and the seed.** Those name the *generation*,
/// and the piano roll can edit a pattern without changing either — so two drags
/// of the same seed can carry different notes, and reusing the folder would
/// overwrite a file the producer's project is still pointing at.
///
/// ⛔⛔ **And "no other drag" has to mean earlier *runs* too, which a counter
/// alone does not give.** The counter restarts at 0 in every process and Windows
/// recycles process ids, so `1234-0` can name a folder a previous session left
/// behind — and that folder may hold a `.wav` an Ableton set is still
/// referencing by path, because [`sweep`] deliberately never deletes audio.
/// Writing into it would overwrite the producer's clip, and abandoning that drag
/// would `remove_dir_all` it: the exact failure the never-delete-audio rule
/// exists to prevent, reached from the other side. So the folder is **claimed by
/// creating it**, and a name that already exists is skipped rather than reused.
fn fresh_spool_dir() -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = spool_root();
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("could not create {}: {e}", root.display()))?;
    claim_spool_dir(&root, &NEXT)
}

/// How many taken names to step over before giving up.
///
/// Far past any real session: it only binds if a thousand folders from earlier
/// runs share this process id, which needs the id to have been recycled a
/// thousand times with the counter landing in the same place.
const SPOOL_ATTEMPTS: u64 = 1_024;

/// The claim itself, with the root and the counter passed in so it is testable.
fn claim_spool_dir(root: &Path, next: &AtomicU64) -> Result<PathBuf, String> {
    for _ in 0..SPOOL_ATTEMPTS {
        let n = next.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("{}-{n}", std::process::id()));
        // ⛔ `create_dir`, **not** `create_dir_all` — the whole point is that
        // this fails when the folder is already there.
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("could not create {}: {error}", dir.display())),
        }
    }
    Err("there is nowhere left to spool a drag".to_owned())
}

static SWEEP: Once = Once::new();

/// How long an old MIDI drag is kept before it is swept.
const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Delete old spool folders — but only the ones that cannot break a project.
///
/// ⛔⛔ **A folder holding audio is never swept, at any age, and this is the
/// rule to not "tidy up".** Ableton references a dropped `.wav` by path until
/// the producer runs Collect All and Save, so deleting one silently empties a
/// clip in a session that opened fine last week. There is no age at which that
/// becomes safe.
///
/// MIDI is different in kind, not in degree: every DAW copies a dropped `.mid`
/// into the project as clip data and never looks at the file again, so a
/// week-old MIDI spool is genuinely nobody's. That asymmetry is the whole rule.
///
/// ⚠ **This is the fallback, not the main cleanup.** A drag that was abandoned
/// or refused is deleted at the moment it is known to be nobody's — see
/// [`Drags::cancel`] and [`Drags::start`] — because there the answer is a fact
/// rather than a guess from a file extension. What reaches this sweep is a
/// folder whose process died mid-gesture.
///
/// ⚠ Runs on the render thread, once per process — never on the editor-open
/// path, where it would be disk I/O on the thread the host draws from.
fn sweep() {
    let Ok(entries) = std::fs::read_dir(spool_root()) else {
        // No spool root yet is the ordinary case on a first run.
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !older_than(&path, now, KEEP_FOR) {
            continue;
        }
        if holds_audio(&path) {
            continue;
        }
        // A folder somebody else's process is mid-write into fails here rather
        // than being forced; the next sweep gets it.
        let _ = std::fs::remove_dir_all(&path);
    }
}

fn older_than(path: &Path, now: SystemTime, age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        // ⚠ A clock that has moved backwards makes `duration_since` fail. That
        // reads as "not old enough", which is the safe direction.
        .and_then(|at| now.duration_since(at).ok())
        .is_some_and(|since| since > age)
}

fn holds_audio(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // Unreadable is treated as "leave it alone", for the same reason the
        // doc comment gives: the cost of a wrong delete is a broken project.
        return true;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|ext| !ext.eq_ignore_ascii_case("mid"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale, PPQ};

    fn pattern(lane: Lane) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 7,
            bars: 1,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 1,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane,
                notes: vec![Note {
                    start_tick: 0,
                    len_ticks: 240,
                    pitch: 38,
                    vel: 100,
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                }],
            }],
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    /// Wait for the prepare thread, without a fixed sleep that would either be
    /// flaky or slow.
    fn settle(drags: &Drags) -> Status {
        for _ in 0..600 {
            let status = drags.status();
            if status != Status::Preparing {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the prepare thread never published a terminal status");
    }

    fn one_kick() -> Subject {
        Subject::Patterns {
            patterns: vec![pattern(Lane::Kick)],
            cut: crate::export::Cut::EveryLane,
        }
    }

    #[test]
    fn preparing_writes_the_files_where_the_drop_target_can_read_them() {
        let drags = Drags::default();
        if !supported_in(false) {
            // Off Windows this is a refusal, and that is the tested behaviour.
            assert!(drags.prepare(one_kick(), None, false).is_err());
            return;
        }
        drags
            .prepare(one_kick(), None, false)
            .expect("a kick must be preparable");
        assert_eq!(settle(&drags), Status::Ready);

        // ⛔ The bytes really are on disk under the name a producer would
        // recognise — `Ready` is a claim about the filesystem, and a status that
        // said so without it being true is the readout-that-lies failure.
        let dir = {
            let slot = drags.slot.lock().unwrap();
            let Stage::Ready(prepared) = &*slot else {
                panic!("ready without a payload");
            };
            assert_eq!(
                prepared
                    .paths
                    .iter()
                    .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                ["trap - Kick - 140 BPM - C# Minor.mid"]
            );
            assert!(prepared.paths[0].is_file(), "the file was not written");
            prepared.dir.clone()
        };

        // And abandoning the gesture takes the folder back with it.
        drags.cancel();
        assert!(!dir.exists(), "an abandoned drag left its render on disk");
    }

    #[test]
    fn only_the_named_lane_is_spooled_when_one_is_asked_for() {
        // ⛔ "i can just drag the hihats out to the daw or just the snares".
        // The page used to slice this itself, which put the rule on both sides.
        if !supported_in(false) {
            return;
        }
        let mut both = pattern(Lane::Kick);
        both.lanes.push(engine::pattern::LaneTrack {
            lane: Lane::ClosedHat,
            notes: both.lanes[0].notes.clone(),
        });

        let prepared = render_and_spool(
            Subject::Patterns {
                patterns: vec![both],
                cut: crate::export::Cut::OneLane(Lane::ClosedHat),
            },
            None,
        )
        .expect("a hat lane must spool");
        assert_eq!(
            prepared
                .paths
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["trap - ClosedHat - 140 BPM - C# Minor.mid"],
            "asking for one lane must not spool the others"
        );
        remove(&prepared.dir);
    }

    #[test]
    fn two_drags_do_not_write_over_each_other() {
        // ⛔ The reason the spool folder is a counter and not the seed: the
        // piano roll can edit a pattern without changing the seed, so the same
        // name can carry different notes.
        let first = fresh_spool_dir().expect("a spool folder must be claimable");
        let second = fresh_spool_dir().expect("and a second one");
        assert_ne!(first, second);
        assert!(
            first.is_dir() && second.is_dir(),
            "both are claimed on disk"
        );
        remove(&first);
        remove(&second);
    }

    #[test]
    fn a_render_that_finishes_after_being_cancelled_takes_its_own_folder_back() {
        // ⛔⛔ **The leak the review found, and the one most likely to happen:**
        // releasing before the bytes exist is the ordinary abandonment.
        // `cancel` cannot reach into a running render, so without the
        // generation the files landed in the slot and nothing ever removed them
        // — and for audio the sweep is forbidden to, at any age.
        if !supported_in(false) {
            return;
        }
        let drags = Drags::default();
        drags
            .prepare(one_kick(), None, false)
            .expect("a kick must be preparable");
        // The producer lets go while the render is still running.
        drags.cancel();

        // Whatever the thread publishes, the slot must come to rest empty and
        // the folder must be gone.
        for _ in 0..600 {
            if matches!(drags.status(), Status::Idle) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(drags.status(), Status::Idle, "a disowned render published");

        let leaked: Vec<_> = std::fs::read_dir(spool_root())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}-", std::process::id()))
            })
            .collect();
        assert!(
            leaked.is_empty(),
            "an abandoned render left {} folder(s) behind",
            leaked.len()
        );
    }

    #[test]
    fn a_second_press_during_a_cancelled_render_is_allowed() {
        // ⛔ The refusal used to fire here: `cancel` leaves a running render
        // alone, so the producer was told "a drag is already being prepared"
        // for the drag they were currently making.
        if !supported_in(false) {
            return;
        }
        let drags = Drags::default();
        drags.prepare(one_kick(), None, false).unwrap();
        drags.cancel();
        drags
            .prepare(one_kick(), None, false)
            .expect("a second press must not be refused");
        settle(&drags);
        drags.cancel();
    }

    #[test]
    fn the_standalone_is_refused_because_a_drag_there_aborts_the_process() {
        // ⛔⛔ Not a preference. The standalone pumps its own message queue, so
        // the RPC handler runs inside baseview's window procedure and
        // `DoDragDrop` dispatches straight back into it.
        assert!(!supported_in(true));
        let drags = Drags::default();
        assert_eq!(
            drags.prepare(one_kick(), None, true).unwrap_err(),
            NO_DRAG_SOURCE
        );
    }

    #[test]
    fn a_folder_an_earlier_run_left_behind_is_stepped_over_rather_than_reused() {
        // ⛔⛔ **The bug the security review found, and it is the
        // never-delete-audio rule reached from the other side.** The counter
        // restarts at 0 in every process and Windows recycles process ids, so
        // `1234-0` can name a folder a previous session left behind — holding a
        // `.wav` an Ableton set still references by path, because the sweep
        // deliberately never deletes audio. Writing into it would overwrite the
        // producer's clip; abandoning that drag would delete it outright.
        let root = std::env::temp_dir().join(format!("fmm-claim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // Whatever an earlier run left at the first name this run will try.
        let stale = root.join(format!("{}-0", std::process::id()));
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("keep.wav"), b"RIFF").unwrap();

        let counter = AtomicU64::new(0);
        let claimed = claim_spool_dir(&root, &counter).expect("a free name must be found");

        assert_ne!(claimed, stale, "the occupied folder must not be reused");
        assert!(
            stale.join("keep.wav").is_file(),
            "an earlier run's audio must be left exactly where it was"
        );
        assert!(claimed.is_dir(), "and the claim really creates the folder");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pattern_that_plays_nothing_is_refused_rather_than_spooling_silence() {
        let mut silent = pattern(Lane::Kick);
        silent.lanes[0].notes.clear();
        assert!(render_and_spool(
            Subject::Patterns {
                patterns: vec![silent],
                cut: crate::export::Cut::Parts,
            },
            None
        )
        .is_err());
    }

    #[test]
    fn starting_before_the_files_exist_says_so_rather_than_dragging_nothing() {
        let drags = Drags::default();
        assert_eq!(
            drags.start(None).unwrap_err(),
            "there is nothing prepared to drag"
        );
    }

    #[test]
    fn a_failure_is_taken_once_but_ready_survives_the_poll() {
        // ⚠ The one place this deliberately differs from `export::take_status`,
        // and getting it backwards would make the poll that reports success
        // also destroy what `start` needs.
        let drags = Drags::default();
        *drags.slot.lock().unwrap() = Stage::Failed("nope".to_owned());
        assert!(matches!(drags.status(), Status::Failed { .. }));
        assert_eq!(drags.status(), Status::Idle, "a failure is taken");

        // ⚠ A directory that does not exist, so `discard` has nothing to delete
        // — this test is about the slot, not the filesystem.
        *drags.slot.lock().unwrap() = Stage::Ready(Prepared {
            paths: vec![PathBuf::from("a.mid")],
            dir: spool_root().join("does-not-exist"),
        });
        assert_eq!(drags.status(), Status::Ready);
        assert_eq!(
            drags.status(),
            Status::Ready,
            "reading must not consume what start still needs"
        );
    }

    #[test]
    fn the_base64_decoder_agrees_with_what_a_browser_produces() {
        // ⛔ The exact strings `btoa` gives for these, checked against a browser
        // rather than against this decoder's own output — a decoder tested only
        // by its own encoder proves the two are consistent, not that either is
        // right.
        assert_eq!(from_base64("").unwrap(), b"");
        assert_eq!(from_base64("Zg==").unwrap(), b"f");
        assert_eq!(from_base64("Zm8=").unwrap(), b"fo");
        assert_eq!(from_base64("Zm9v").unwrap(), b"foo");
        assert_eq!(from_base64("Zm9vYmFy").unwrap(), b"foobar");
        // The two characters that separate standard base64 from the URL-safe
        // alphabet, and a byte that needs all eight bits.
        assert_eq!(from_base64("//79").unwrap(), [255, 254, 253]);
    }

    #[test]
    fn a_drag_image_that_is_not_base64_is_refused_rather_than_skipped() {
        // ⚠ Quietly dropping a stray byte would decode a corrupted image into a
        // *shifted* one, which renders as garbage nobody can attribute.
        assert!(from_base64("Zm9v!").is_err());
        assert!(from_base64("Zm9 v").is_err());
        // Trailing bits that are not zero mean the encoder and this disagree
        // about the length.
        assert!(from_base64("Zh==").is_err());
    }

    #[test]
    fn a_drag_image_whose_size_does_not_match_its_pixels_is_refused() {
        // ⛔⛔ **This is the invariant an `unsafe` block depends on.**
        // `windows.rs` builds a 32-bpp DIB from the width and height — stride
        // `width * 4`, no padding — and then writes `rgba.len()` bytes into it
        // through a raw slice. A buffer longer than the picture is an
        // out-of-bounds write inside somebody's DAW, and both numbers arrive as
        // JSON from the webview.
        assert!(Preview::new(2, 2, vec![0; 15]).is_err(), "too short");
        assert!(Preview::new(2, 2, vec![0; 17]).is_err(), "too long");
        assert!(Preview::new(0, 2, vec![]).is_err());
        assert!(Preview::new(2, 0, vec![]).is_err());
        assert!(
            Preview::new(4096, 4096, vec![0; 16]).is_err(),
            "past the cap"
        );

        // And what survives really does fill its own bitmap exactly.
        let preview = Preview::new(2, 2, vec![0; 16]).expect("2x2 must be usable");
        let (width, height, rgba) = preview.parts();
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
    }

    #[test]
    fn a_folder_holding_audio_is_never_swept_however_old_it_is() {
        // ⛔⛔ The rule that must not be "tidied up": Ableton references a
        // dropped wav by path, so deleting one empties a clip in a session that
        // opened fine last week.
        let dir = std::env::temp_dir().join(format!("fmm-sweep-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("loop.wav"), b"RIFF").unwrap();
        assert!(holds_audio(&dir));

        std::fs::remove_file(dir.join("loop.wav")).unwrap();
        std::fs::write(dir.join("loop.mid"), b"MThd").unwrap();
        assert!(!holds_audio(&dir), "a MIDI-only folder is sweepable");

        // An unreadable folder is left alone rather than guessed at.
        assert!(holds_audio(&dir.join("does-not-exist")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
