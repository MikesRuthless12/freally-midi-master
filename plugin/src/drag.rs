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
//! platform: `DoDragDrop` on Windows, `NSDraggingSession` on macOS, and GTK
//! carrying `text/uri-list` on Linux. [`platform`] is that seam.
//!
//! ⚠ **All three are switched on, and they are not equally proven.** Each
//! platform module owns a `SUPPORTED` flag and [`supported_in`] is what the page
//! asks before it draws a handle. Windows has been dropped into Ableton by a
//! human; Linux compiles and has never been dropped; macOS has never even been
//! compiled here — CI's macOS runner is the first thing that type-checks it.
//! `drag/macos.rs` explains why it is on regardless: testers cannot report on a
//! handle they cannot see.
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
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, SystemTime};

use engine::pattern::Pattern;
use serde::Serialize;

#[cfg(windows)]
#[path = "drag/windows.rs"]
mod platform;

#[cfg(target_os = "linux")]
#[path = "drag/linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "drag/macos.rs"]
mod platform;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
#[path = "drag/unsupported.rs"]
mod platform;

/// Whether this build can start an OS file drag at all.
///
/// ⛔ **The page asks before it renders a drag handle.** A handle that looks
/// draggable and drops nothing is worse than no handle: the producer blames
/// their DAW. A platform that answers no keeps the Export path, and the chip
/// keeps saying "Export" — the same rule TASK-131F already followed.
///
/// ⚠ **All three platforms answer yes now.** macOS does so having never been
/// run — see `drag/macos.rs` for why that exception was made deliberately.
///
/// ✅ **No platform refuses the standalone any more, as of TASK-063D** — and the
/// history is worth keeping, because the refusal was wrong twice over.
///
/// It began as a blanket rule for all three. That was one platform's problem
/// standing in for a rule: `own_message_queue()` is a no-op off Windows —
/// `standalone.rs` says so at the call site — and neither a Cocoa dragging
/// session nor a GTK drag re-enters anybody's window procedure. Lifting it for
/// macOS and Linux gave them back a feature they could always have had.
///
/// Windows was genuinely blocked, and the mechanism was exact: the standalone
/// calls `own_message_queue()`, `windows_pump` drained from `on_frame`, which is
/// inside baseview's window procedure with a `RefCell` borrow live, and
/// `DoDragDrop`'s modal loop dispatched a `WM_TIMER` straight back into it —
/// panicking inside an `extern "system"` frame, where it cannot unwind, and
/// aborting the process. ▶ **That was fixed at the root rather than gated
/// around:** the pump now posts to a child window and `baseview`'s own
/// `open_blocking` loop dispatches it, so nothing is borrowed by the time a drag
/// runs. `drag/windows.rs` and `windows_pump::request` carry it in full.
///
/// The honest condition remains **"this process pumps its own message queue
/// from inside somebody's window procedure"**, and
/// [`platform::STANDALONE_SAFE`] is where each platform answers it. ⛔ Anything
/// that reintroduces a drain on a borrowed stack has to answer `false` again.
pub fn supported_in(standalone: bool) -> bool {
    platform::SUPPORTED && (!standalone || platform::STANDALONE_SAFE)
}

/// What a platform with no drag source says, in one place.
///
/// ⚠ Both the refusal above the seam and the one below it said this, word for
/// word — two copies of a sentence a producer reads is two chances for them to
/// stop agreeing about what to do instead.
pub(crate) const NO_DRAG_SOURCE: &str = "this platform has no drag source yet — use Export instead";

/// How a drag is going, for the page to show.
///
/// ⚠ `PartialEq` and not `Eq` since [`Self::Preparing`] began carrying a float.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// No drag has been prepared since the last one finished.
    Idle,
    /// The bytes are being rendered and written.
    ///
    /// ⛔⛔ **`done` is what makes a whole-arrangement audio drag possible at
    /// all.** Rendering a three-minute record is seconds of work, not the
    /// milliseconds a four-bar loop takes — and the producer is holding the
    /// mouse button down for every one of them. Without a number on screen that
    /// is indistinguishable from the plugin having hung, which is exactly why
    /// `editor.rs` used to refuse the request outright and say "render audio
    /// stems from Export instead".
    ///
    /// ⚠ **0.0 for the short renders, and that is honest rather than lazy.** A
    /// single loop finishes inside one poll interval, so a bar that appeared
    /// and vanished would be noise; the page shows a plain "getting it ready"
    /// until this passes zero.
    Preparing {
        /// How much of the work is done, 0.0 to 1.0.
        done: f32,
    },
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
    /// How far the render in flight has got, in thousandths.
    ///
    /// ⛔ **Beside the slot rather than inside `Stage::Preparing`, and the reason
    /// is lock traffic.** The page polls this every 40 ms while the producer
    /// holds the button, and the render publishes on every part it finishes;
    /// putting it in the slot would make both take the mutex the *drag start*
    /// also needs. An integer atomic costs neither of them anything.
    ///
    /// ⚠ **The fraction's bits, the way `SharedState::set_playhead` already
    /// publishes a 0..1 `f32` through an `AtomicU32`.** There is no `AtomicF32`,
    /// and this crate had already answered that for this exact quantity —
    /// `to_bits`/`from_bits` is exact and needs no clamping arithmetic at the
    /// write and no divide at the read. A thousandths integer was a second,
    /// lossy answer to a solved problem.
    progress: Arc<AtomicU32>,
}

/// What a long render reports to, and asks whether anybody still wants it.
///
/// ⛔⛔ **The "cancel" half of progress-with-cancel, and it is not a new
/// mechanism — it is the existing disown made *promptly observable*.** `cancel`
/// already replaced the slot so a finished render would find an id that was not
/// its own and throw its own work away; what it could not do was stop the work.
/// For a four-bar loop that cost nothing. For a whole arrangement it is seconds
/// of CPU inside somebody's DAW, burned after they let go — so the render now
/// asks between parts whether it is still the one in the slot, and stops if it
/// is not.
struct Progress {
    slot: Arc<Mutex<Stage>>,
    published: Arc<AtomicU32>,
    /// The id this render was stamped with, as `Stage::Preparing` holds it.
    id: u64,
}

impl Progress {
    /// Note that `done` of `total` units are finished, and say whether to go on.
    ///
    /// ⛔⛔ **Asks whether it is still wanted BEFORE it publishes, and the order
    /// is the fix rather than a preference.** [`Self::published`] is one atomic
    /// shared by every render this instance ever starts, so a render that has
    /// been disowned and reports anyway is writing into its *successor's* bar.
    /// It reads as a drag that opens at 85%, and the page's stall clock — which
    /// only re-arms on a figure it has not already seen — then watched the new
    /// render's honest 0.05, 0.10, 0.15 go by underneath it, waited ten seconds,
    /// and killed a perfectly healthy whole-arrangement render. On every retry.
    ///
    /// ⚠ The other order had a reason and it does not survive: "a render that
    /// stopped without publishing its last step leaves the bar short of where it
    /// got". True, but only the *owning* render has a bar anybody is watching —
    /// a disowned one is reporting to a slot that has moved on.
    fn step(&self, done: usize, total: usize) -> bool {
        if !self.wanted() {
            return false;
        }
        let fraction = if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).clamp(0.0, 1.0)
        };
        self.published.store(fraction.to_bits(), Ordering::Relaxed);
        true
    }

    /// Is this render still the one the slot is holding?
    ///
    /// ⚠ **The generation is what disowns a render, not the lock.** An earlier
    /// cut answered "no" whenever the mutex was poisoned, on the reasoning that
    /// something had already panicked and the DAW's CPU should stop being spent.
    /// A panic elsewhere says nothing about whether *this* render is still
    /// wanted, and the id below already answers that exactly — see `crate::held`
    /// for why the slot is trustworthy either way.
    fn wanted(&self) -> bool {
        matches!(&*crate::held(&self.slot), Stage::Preparing(id) if *id == self.id)
    }

    /// A handle with nobody watching, for tests that only care about the files.
    ///
    /// ⚠ Its slot is genuinely `Preparing`, so [`Self::wanted`] answers yes —
    /// a stub that always said no would make every test render nothing.
    #[cfg(test)]
    fn detached() -> Self {
        Self {
            slot: Arc::new(Mutex::new(Stage::Preparing(0))),
            published: Arc::new(AtomicU32::new(0)),
            id: 0,
        }
    }
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
    /// The other layout of the same material, handed over **while Ctrl is
    /// held**. Empty when Ctrl has nothing to choose between here.
    ///
    /// ⚠ **Renamed from `stacked` on 2026-08-11, because it stopped being
    /// stacked.** It held one thing when it was written — the drum lanes all
    /// starting at bar 1 instead of end to end — and it now holds two, so a name
    /// describing one of them was about to be a lie about the other:
    ///
    /// - **A multi-part drag** ([`crate::export::Cut::Parts`]) sends **one file**
    ///   by default and this is the **one-file-per-part** set. Mike,
    ///   2026-08-11: *"all parts of a song should be able to press 'Ctrl+Drag
    ///   in' to put them on separate lanes, but together if you don't press
    ///   ctrl."* Here Ctrl **separates**.
    /// - **The sequential lane cut** ([`crate::export::Cut::EveryLaneInSequence`])
    ///   sends the lanes end to end and this is the all-at-bar-1 set. There Ctrl
    ///   **stacks**. ⚠ That path is currently unreachable from the page — nothing
    ///   sends `lanes: true` — so it is the older meaning kept alive by tests.
    ///
    /// ⛔ **Spooled up front, alongside the other set, because the choice is
    /// made mid-drag.** `drag/windows.rs` watches the modifier from inside
    /// `QueryContinueDrag` and swaps the payload when it changes — which it can
    /// only do if the files already exist. Rendering at the moment of the drop
    /// would put a multi-lane audio render inside the drag loop, and the whole
    /// `prepare`/`start` split exists to keep it out of there.
    ///
    /// ⚠ The expensive half is shared: both sets come from the same render —
    /// [`crate::export::multi_part_files`] is written that way on purpose — and
    /// only the summing and the file writing differ.
    alternative: Vec<PathBuf>,
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
            let mut slot = crate::held(&self.slot);
            // Anything already prepared is dropped here rather than reused. A
            // producer who starts a second gesture has changed their mind about
            // what they are dragging, and handing them the previous selection is
            // the wrong loop with no way to tell. ⚠ Its folder goes with it, for
            // the reason [`Self::cancel`] gives: nothing was ever dropped, so
            // nothing can be referencing it.
            discard(std::mem::replace(&mut *slot, Stage::Preparing(mine)));
            Arc::clone(&self.slot)
        };
        // ⚠ Zeroed with the claim, not when the render starts: the page can poll
        // in between, and a stale figure from the *previous* drag would show a
        // bar that starts at 70% and goes backwards.
        self.progress.store(0, Ordering::Relaxed);

        let progress = Progress {
            slot: Arc::clone(&slot),
            published: Arc::clone(&self.progress),
            id: mine,
        };

        std::thread::spawn(move || {
            let outcome = match render_and_spool(subject, kit.as_deref(), &progress) {
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
            let mut slot = crate::held(&slot);
            if matches!(*slot, Stage::Preparing(waiting) if waiting == mine) {
                *slot = outcome;
            } else {
                discard(outcome);
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
        let mut slot = crate::held(&self.slot);
        match &*slot {
            Stage::Idle => Status::Idle,
            Stage::Preparing(_) => Status::Preparing {
                // ⚠ `from_bits`, matching `Progress::step`'s `to_bits` — the
                // same pairing `SharedState::playhead` uses. Exact, and no
                // divide to keep in step with a multiply at the other end.
                done: f32::from_bits(self.progress.load(Ordering::Relaxed)),
            },
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
            let mut slot = crate::held(&self.slot);
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
        let dropped = match platform::drag(&prepared.paths, &prepared.alternative, preview.as_ref())
        {
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
            // ⛔⛔ **NOTHING IS DELETED HERE ANY MORE, AND THAT IS THE FL STUDIO
            // MIDI BUG** (2026-08-11).
            //
            // This used to keep audio and delete MIDI, on the reasoning that *"a
            // `.mid` is copied into the project by every DAW and its spool is
            // nobody's either way."* ▶ **That is a claim about every drop target
            // on the system, and FL Studio is the counter-example** — the same
            // shape of over-confident claim the paragraph above already records
            // being wrong once.
            //
            // ▶ **Mike's report is the proof, and it is exact.** Dragging from
            // the standalone into FL: every `.wav` lands and every `.mid` fails —
            // Melody, Chords, Counter, All Parts MIDI and Song MIDI, while All
            // Parts audio and Song audio all work. Ableton takes both. The only
            // thing this code does differently between those two sets is delete
            // one of them: FL reports `DROPEFFECT_NONE` and reads the file
            // **after** `DoDragDrop` returns, so the `.mid` was being removed out
            // from under it while the `.wav` beside it survived to be read.
            //
            // ⚠ **Nothing leaks.** [`sweep`] still reclaims MIDI-only folders
            // once they are old enough — it is only forbidden to touch the ones
            // holding audio, which a live session may still be pointing at. The
            // cost of keeping them is a few kilobytes of temp for an hour; the
            // cost of deleting them is the clip the producer just dragged.
            Dropped::Refused => {}
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
        discard(std::mem::replace(
            &mut *crate::held(&self.slot),
            Stage::Idle,
        ));
    }
}

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
    progress: &Progress,
) -> Result<Prepared, String> {
    // ⛔ **The export's own functions, not second copies.** A hi-hat loop
    // dragged into Ableton and the same loop exported to a folder must be the
    // same file; two implementations of that is how they stop agreeing.
    // ⛔ **The Ctrl alternative, built here because it cannot be built later.**
    // Only the sequential cut has one: every other gesture is a single file or
    // a single lane, and Ctrl has nothing to choose between.
    let mut alternative = Vec::new();
    let files = match subject {
        Subject::Patterns { patterns, cut } => {
            // ⛔ **Refused before rendering here too.** This branch renders audio
            // for every lane, and it used to meet only the silent clamp — see
            // [`crate::audio::render::refuse_if_too_long`], which is now the one
            // rule every audio path asks.
            crate::audio::render::refuse_if_too_long(&patterns, kit.is_some())?;

            // ⛔⛔ **NO OFFSET, NO PADDING: every part is its own clip, at bar 1,
            // exactly as long as the pattern is.** Mike, 2026-08-11, looking at
            // what the offset actually produced: *"the samples are 8 bars and the
            // clips are only supposed to be 4 bars and the sounds play at the end
            // of the 8 bars on the last 4 bars, when they should all only be 4
            // bars long."*
            //
            // ▶ **That was a bug of mine, not a difference of opinion.** The
            // sequencing pushed part *n* by *n* clip-lengths and grew its `bars`
            // to cover the offset, so the second file really was eight bars with
            // four of silence in front. Read as "one clip after the next", baked
            // into the files, it is also the wrong reading: a host gives every
            // dropped file its own row, so offsets produce a **staircase across
            // rows** rather than a run along one — which is what he screenshotted.
            //
            // ⚠ So this is plain `Cut::Parts` again, and there is no Ctrl
            // alternative for a multi-part drag: with nothing offset there are no
            // two layouts to switch between. `Cut::EveryLaneInSequence` below is
            // untouched — that is the *drum lane* cut, and unreachable today.
            // ⛔⛔ **An alternative only when the two layouts actually differ,
            // which is MIDI only.** `stem_files_with` renders the sequential cut
            // as the stacked one for audio — see the note on
            // `Cut::EveryLaneInSequence` — so building one there would render
            // every lane a second time to produce byte-identical files. That is a
            // full extra sampler pass per lane, on a gesture, inside a DAW.
            if matches!(cut, crate::export::Cut::EveryLaneInSequence) && kit.is_none() {
                alternative = crate::export::stem_files_with(
                    &patterns,
                    crate::export::Cut::EveryLane,
                    kit,
                    &mut |done, total| progress.step(done, total * 2),
                );
            }
            // ⛔ **Progress is reported here too, not only for songs.** This is
            // the branch that renders audio for every lane, so it is the one a
            // producer is most likely to abandon mid-render — and the page's
            // clock is a *stall* timer now, re-armed only when `done` advances.
            // Left unreported, the longest render in the system was the one path
            // that could neither say how far it had got nor be told to stop.
            let offset = alternative.len();
            crate::export::stem_files_with(&patterns, cut, kit, &mut |done, total| {
                progress.step(offset + done, if offset > 0 { total * 2 } else { total })
            })
        }
        // ⛔⛔ **Audio is allowed here now, and the comment this replaces said it
        // never could be.** It read: *"MIDI, and the caller has already refused
        // the audio half — a song is minutes long and rendering one needs
        // progress a producer can watch."* That was the right objection and it
        // is the one [`Progress`] answers, so `kit` is passed through rather
        // than hard-coded to `None`. A song still drags as MIDI when no kit was
        // asked for; the difference is that asking is no longer refused.
        Subject::Song(song) => {
            let parts = crate::export::song_stem_patterns(&song);
            // ⛔ **Refused before rendering, not truncated during it.** The
            // render buffer is clamped to `MAX_SECONDS`, and a clamp cannot
            // report — a record longer than the bound would drag out silently
            // short, which is worse than not dragging at all because the
            // producer would find out in their arrangement rather than here.
            crate::audio::render::refuse_if_too_long(&parts, kit.is_some())?;
            // ⛔⛔ **A SONG IS SEPARATE CLIPS THAT LAND AS AN ARRANGEMENT — one
            // file per part, every one starting at bar 1.** Mike, 2026-08-11:
            // *"song needs to be all separate midi clips and all separate audio
            // clips that land as an arrangement"*, and before that, when the
            // All Parts rule was being applied here too: *"no, not the song."*
            //
            // ▶ **Which is exactly what this always did, and the reason is worth
            // keeping.** `song_stem_patterns` flattens each part onto the
            // **whole timeline**, so the five files are five simultaneous views
            // of one record: dropped together at bar 1 they reassemble it, on a
            // row each. ⛔ Neither of the other two layouts is available to a
            // song — laying them end to end would play the drums for the length
            // of the record and then the chords after it, and merging them into
            // one clip would throw away the separation a stem drag exists for.
            //
            // ⚠ **So there is no Ctrl alternative here**, deliberately: both
            // alternatives are wrong for an arrangement, and offering a modifier
            // that produced one would be worse than offering none.
            crate::export::stem_files_with(
                &parts,
                crate::export::Cut::Parts,
                kit,
                &mut |done, total| progress.step(done, total),
            )
        }
    };
    if files.is_empty() {
        return Err("there is nothing here to drag".to_owned());
    }
    // ⛔ **Checked after the render, before anything is written.** A cancelled
    // render returns whatever it had already encoded — `stem_files_with` hands
    // it back rather than deciding — and spilling that would leave a partial
    // arrangement on disk that the producer could then drag out as if it were
    // the whole record.
    if !progress.wanted() {
        return Err("that drag was cancelled".to_owned());
    }

    SWEEP.call_once(sweep);
    let dir = fresh_spool_dir()?;
    let paths = crate::export::spill(&dir, &files)?;
    // ⚠ **A sub-folder, because the two sets can carry the same names.** A lane
    // stem is named for its lane in either layout — that is the point of the
    // naming rule — so writing them side by side would have the second set
    // overwrite the first and both drags hand over the same files. The folder
    // goes with `dir` when the drag is abandoned, so nothing new has to be swept.
    let alternative = if alternative.is_empty() {
        Vec::new()
    } else {
        crate::export::spill(&dir.join("ctrl"), &alternative)?
    };
    Ok(Prepared {
        paths,
        alternative,
        dir,
    })
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
            song_seed: 7,
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
                    slide_ms: None,
                    slide_overlap_ticks: None,
                    timing_locked: false,
                    articulation: None,
                    reversed: false,
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
    /// Exclusive use of the spool root, for the tests that count what is in it.
    ///
    /// ⛔⛔ **[`spool_root`] is ONE directory shared by every test in this
    /// module, and `cargo test` runs them on parallel threads in a single
    /// process — so they also share the `{pid}-` prefix the folder names are
    /// built from.** `a_render_that_finishes_after_being_cancelled_takes_its_own_folder_back`
    /// asserts that no folder with that prefix survives, which is only a
    /// statement about *its own* render if nothing else is spooling at the time.
    /// `two_drags_do_not_write_over_each_other` claims two folders outright, and
    /// half a dozen more prepare real renders.
    ///
    /// ⚠ **This was always a race and it had simply never been lost.** It became
    /// systematic the moment `Progress::step` started asking `wanted()` *before*
    /// publishing: a cancelled render now returns on its first step instead of
    /// rendering every file first, so the leak scan happens milliseconds after
    /// `prepare` rather than tens of milliseconds later — reliably inside a
    /// sibling test's window. It went red on all three runners at once.
    ///
    /// ⚠ **Holding the guard is not enough on its own — a test must also let its
    /// render finish before releasing it**, because `prepare` spawns a thread
    /// that claims its folder later. That is what [`settle`] is for, and every
    /// holder of this guard calls it.
    ///
    /// ⛔⛔ **And `settle` is not enough for a *cancelled* render**, which is
    /// what made this whole arrangement look sound while still being flaky. A
    /// disowned render publishes no terminal status by design, so `settle` has
    /// nothing to wait for — it returns, the guard is released, and the render
    /// goes on to claim and then drop its folder while the next test is counting
    /// folders. [`settle_spool`] is the wait that covers that case, and any test
    /// that ends with a render still disowned has to call it.
    ///
    /// ⚠ The poison is deliberately ignored: a sibling test failing while
    /// holding this must not turn every other test in the module red as well,
    /// which would bury the one real failure. ⛔ That is the same answer
    /// [`crate::held`] gives for the four job mailboxes and the same argument,
    /// so it is that function rather than a sixth hand-rolled copy of its body.
    fn spool_guard() -> std::sync::MutexGuard<'static, ()> {
        static SPOOL: Mutex<()> = Mutex::new(());
        crate::held(&SPOOL)
    }

    /// Every spool folder this process owns right now.
    fn spool_folders() -> Vec<std::path::PathBuf> {
        let prefix = format!("{}-", std::process::id());
        std::fs::read_dir(spool_root())
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            })
            .collect()
    }

    /// Wait until no disowned render is still holding a folder.
    ///
    /// ⛔⛔ **The other half of [`spool_guard`]'s rule, and its absence made
    /// `a_render_that_finishes_after_being_cancelled_takes_its_own_folder_back`
    /// fail once in a full-lib run and pass every time in isolation.** The guard
    /// serialises the *tests*; it cannot serialise the threads they leave
    /// running. A test that cancels a render releases the guard with that render
    /// still alive — it claims its folder a moment later and removes it a moment
    /// after that — and the next holder counting folders sees two that are not
    /// its own and calls them a leak.
    ///
    /// ⚠ `settle` is not enough for this: it waits for a *terminal status*, and
    /// a disowned render publishes none by design. The folder going away is the
    /// only observable that says it finished.
    fn settle_spool() {
        for _ in 0..600 {
            if spool_folders().is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("a disowned render never gave its spool folder back");
    }

    fn settle(drags: &Drags) -> Status {
        for _ in 0..600 {
            let status = drags.status();
            // ⚠ Matched on the variant rather than compared, since `Preparing`
            // began carrying how far it has got — any `done` is still preparing.
            if !matches!(status, Status::Preparing { .. }) {
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
        let _spool = spool_guard();
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
        let _spool = spool_guard();
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
            &Progress::detached(),
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

    /// ⛔⛔ **The AUDIO half of the test above, which had none at all.** Mike,
    /// 2026-08-06: *"i want the separate audio drum lanes just like the midi
    /// lanes."*
    ///
    /// ⚠ **Every single-lane test passed `None` for the kit**, so the per-lane
    /// *audio* path was entirely unasserted — and his own spool folders say it
    /// had never once run: across all 20 audio drags in `%TEMP%\Freally MIDI
    /// Master\drag\`, every `.wav` is named for a part and not one for a lane,
    /// while the MIDI spools beside them hold `Snare.mid` and `Kick.mid`. A
    /// capability nothing exercises and nobody has used is one nobody can say
    /// works, which is the shape of failure this file keeps recording.
    #[test]
    fn one_drum_lane_spools_one_wav_of_that_lane_alone() {
        let _spool = spool_guard();
        if !supported_in(false) {
            return;
        }
        // ⛔ `expect`, never a silent skip — `the_preview_kit_is_in_the_binary_
        // and_decodes` already pins that this is always `Some`, and a test that
        // quietly does nothing is the "124 passed with 32 never executed"
        // readout this project has been burned by.
        let kit = crate::audio::preview_kit().expect("the preview kit is compiled in");

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
            Some(kit.as_ref()),
            &Progress::detached(),
        )
        .expect("a hat lane must spool as audio");

        assert_eq!(
            prepared
                .paths
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["trap - ClosedHat - 140 BPM - C# Minor.wav"],
            "one audio lane must be one .wav named for that lane, not the part"
        );
        remove(&prepared.dir);
    }

    #[test]
    fn two_drags_do_not_write_over_each_other() {
        let _spool = spool_guard();
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
        let _spool = spool_guard();
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

        let leaked = spool_folders();
        assert!(
            leaked.is_empty(),
            "an abandoned render left {} folder(s) behind",
            leaked.len()
        );
    }

    #[test]
    fn a_second_press_during_a_cancelled_render_is_allowed() {
        let _spool = spool_guard();
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

        // ⛔ **Two disowned renders are still running here**, and releasing the
        // guard with them alive is what made a sibling test flaky: it counted
        // their folders as a leak of its own. See [`settle_spool`].
        settle_spool();
    }

    /// ⛔⛔ **The Windows standalone is allowed, and it is the one that had to be
    /// earned.** TASK-063D.
    ///
    /// This assertion is the inverse of the one it replaces, which read
    /// `assert!(!supported_in(true))` and existed because a drag there aborted
    /// the process. It is flipped rather than deleted so the reason survives:
    /// the abort was real, and it was fixed by moving the message pump off
    /// baseview's window procedure — **not** by deciding the risk was
    /// acceptable. ⚠ If this ever fails, read `windows_pump::request` before
    /// changing it; the likely cause is a `drain` call put back on a borrowed
    /// stack, and the correct response is to fix that rather than this.
    #[test]
    fn the_windows_standalone_may_drag_now_that_the_pump_is_off_the_borrowed_stack() {
        if !cfg!(windows) {
            return;
        }
        assert!(
            supported_in(true),
            "the Windows standalone is refused again — was the pump moved back into `on_frame`?"
        );
    }

    /// ⛔⛔ **Which platforms may drag out of the standalone, pinned.**
    ///
    /// TASK-063D. Mike, 2026-08-06: *"the Windows/macOS/Linux OS coverage of
    /// drag-and-drop to a DAW from the standalone, and this HAS to be done and
    /// done right."* All three now answer yes, and this is what stops one of
    /// them drifting back to no quietly.
    #[test]
    fn every_platform_allows_the_standalone_now() {
        // ⚠ Was `!cfg!(windows)` — Windows was the exception, for a documented
        // reason, until the pump stopped draining from inside baseview's window
        // procedure. There is no exception left.
        //
        // ⛔ **A `const` block, so drift fails the BUILD rather than the test
        // run.** It is a compile-time constant on every platform, and the old
        // form only escaped `clippy::assertions_on_constants` because it
        // compared against `!cfg!(windows)`. Being unable to compile a build
        // whose standalone quietly stopped offering the drag is the stronger
        // guarantee, and it costs nothing.
        const {
            assert!(
                platform::STANDALONE_SAFE,
                "a platform changed its mind about the standalone without saying why"
            )
        };

        // ⚠ And the gate composes: a platform with no drag source at all must
        // stay refused in the standalone regardless of this flag.
        if !platform::SUPPORTED {
            assert!(!supported_in(true));
            assert!(!supported_in(false));
        }
    }

    #[test]
    fn a_standalone_that_can_drag_is_not_told_it_cannot() {
        let _spool = spool_guard();
        // ⚠ The other direction of the same rule, driven through `prepare`
        // rather than the flag — a gate that answers correctly while the code
        // path still refuses would be worse than no gate.
        if !supported_in(true) {
            return;
        }
        let drags = Drags::default();
        assert!(
            drags.prepare(one_kick(), None, true).is_ok(),
            "the standalone was refused on a platform where it is safe"
        );
        // ⛔ Waited for and taken back. This is the one that only started
        // leaking when macOS and Linux turned `STANDALONE_SAFE` on: before
        // that it returned above on every platform where the render could
        // actually run, so the folder it now spools had never existed.
        settle(&drags);
        drags.cancel();
    }

    /// ⛔⛔ **Which platforms offer a handle, pinned so it cannot drift quietly.**
    ///
    /// `SUPPORTED` is what `drag_supported` answers and therefore what decides
    /// whether the page draws a handle at all — and this project has recorded
    /// five times that a handle which drops nothing is worse than no handle,
    /// because the producer blames their DAW. So the matrix is asserted rather
    /// than left to whatever each platform module happens to say.
    ///
    /// ⚠ **macOS is ON without anybody having watched a file land, and that is
    /// Mike's call, made on 2026-08-06**: *"i thought you were going to build the
    /// macOS drag-and-drop and let my Discord users test it for me totally?"* He
    /// has testers on real Macs, and a flag that hides the handle leaves them
    /// nothing to try — the caution would prevent the very report that resolves
    /// it. Releases are suspended until v1.0.0 (TASK-075), so no unsuspecting
    /// end user meets it first.
    ///
    /// ⚠ **This test previously asserted the opposite**, and the two halves of
    /// one change disagreed: `macos.rs` shipped `SUPPORTED = true` while this
    /// insisted macOS answer no, so `cargo test --workspace` on `macos-latest`
    /// went red on every run. Whichever way it is set, both have to say so.
    #[test]
    fn only_the_platforms_somebody_has_decided_to_offer_are_offered() {
        let offered = supported_in(false);

        if cfg!(windows) || cfg!(target_os = "linux") || cfg!(target_os = "macos") {
            assert!(offered, "a platform with a drag source stopped offering it");
        } else {
            assert!(!offered, "a platform with no drag source is offering one");
        }
    }

    /// The Ctrl alternative is spooled up front, or it cannot exist at all.
    ///
    /// ⛔ `drag/windows.rs` swaps the payload from inside `QueryContinueDrag`,
    /// which it can only do if both sets are already on disk — the modifier may
    /// be pressed *during* the drag, long after there is any chance to render.
    #[test]
    fn a_sequential_cut_spools_the_stacked_layout_alongside_it() {
        let _spool = spool_guard();
        let drags = Drags::default();
        let subject = Subject::Patterns {
            patterns: vec![pattern(Lane::Kick)],
            cut: crate::export::Cut::EveryLaneInSequence,
        };
        if drags.prepare(subject, None, false).is_err() {
            return; // No drag source on this platform; covered above.
        }
        assert_eq!(settle(&drags), Status::Ready);

        {
            let slot = drags.slot.lock().unwrap();
            let Stage::Ready(prepared) = &*slot else {
                panic!("the render did not publish");
            };
            assert!(
                !prepared.alternative.is_empty(),
                "Ctrl would have nothing to switch to"
            );
            // ⚠ Sibling folders, because both sets carry the SAME file names — a
            // lane stem is named for its lane in either layout. Side by side the
            // second would overwrite the first and both drags would hand over the
            // same files.
            assert_ne!(
                prepared.paths[0].parent(),
                prepared.alternative[0].parent(),
                "the two layouts were spooled into one folder"
            );
        }
        // ⛔ **Takes its own render back, and NOT tidiness.** A `Drags` left
        // holding `Ready` has no `Drop` that cleans up, so this used to leave a
        // spooled folder in temp for the rest of the process — which
        // `a_render_that_finishes_after_being_cancelled_takes_its_own_folder_back`
        // then counted as the leak *it* was looking for. That is what turned CI
        // red on all three runners. ⚠ The lock above is scoped, because `cancel`
        // takes the same mutex.
        drags.cancel();
    }

    /// ⛔⛔ **EVERY PART IS ITS OWN CLIP, AT BAR 1, THE LENGTH OF THE PATTERN —
    /// NOTHING IS OFFSET AND NOTHING IS PADDED.**
    ///
    /// ▶ **Three wrong answers preceded this one, and the last is why the test
    /// asserts what it does.** Shipped behaviour was one file per part at bar 1;
    /// the first fix merged them into one clip, which Mike rejected as
    /// *"altogether"*; the second pushed part *n* by *n* clip-lengths and grew
    /// its bar count to cover the offset. He screenshotted the result — *"the
    /// samples are 8 bars and the clips are only supposed to be 4 bars and the
    /// sounds play at the end of the 8 bars on the last 4 bars, when they should
    /// all only be 4 bars long."*
    ///
    /// ⚠ **Baking an offset into the files was the wrong reading of "one clip
    /// after the next" anyway**: a host gives every dropped file its own row, so
    /// offsets arrive as a staircase ACROSS rows rather than a run along one.
    ///
    /// ⚠ **So this asserts the spooled bytes are EXACTLY what encoding each
    /// pattern on its own produces**, which is the sharpest statement available.
    /// File size is a poor proxy: a MIDI offset moves a delta by one or two bytes
    /// of variable-length quantity, and the first attempt at this test failed on
    /// a **one-byte** difference that turned out to be only the track name
    /// (`trap — Drums` against `trap — Melody`). Byte equality against a fresh
    /// encode says no transformation happened at all.
    #[test]
    fn every_part_drags_as_its_own_clip_with_no_padding() {
        let _spool = spool_guard();
        let drags = Drags::default();
        let drums = pattern(Lane::Kick);
        let mut melody = pattern(Lane::Kick);
        melody.part = engine::pattern::Part::Melody;
        let subject = Subject::Patterns {
            patterns: vec![drums.clone(), melody.clone()],
            cut: crate::export::Cut::Parts,
        };
        if drags.prepare(subject, None, false).is_err() {
            return; // No drag source on this platform; covered above.
        }
        assert_eq!(settle(&drags), Status::Ready);

        // ⛔ Read out, then cancelled, and only then asserted — a failing
        // assertion inside the lock would skip the cancel and leave a spool
        // folder for the leak scan in a sibling test to blame itself for.
        let (paths, alternative) = {
            let slot = drags.slot.lock().unwrap();
            let Stage::Ready(prepared) = &*slot else {
                panic!("the render did not publish");
            };
            (prepared.paths.clone(), prepared.alternative.clone())
        };
        let spooled: Vec<Vec<u8>> = paths
            .iter()
            .map(|path| std::fs::read(path).unwrap_or_default())
            .collect();
        drags.cancel();

        assert_eq!(spooled.len(), 2, "one file per part");
        for (at, source) in [drums, melody].iter().enumerate() {
            assert_eq!(
                spooled[at],
                engine::midi::pattern_to_smf(source),
                "part {at} was transformed on the way out"
            );
        }
        // ⚠ And no Ctrl payload, because there is no second layout to reach.
        assert!(
            alternative.is_empty(),
            "nothing is offset, so Ctrl has no set"
        );
    }

    #[test]
    fn a_gesture_with_only_one_meaning_spools_no_alternative() {
        let _spool = spool_guard();
        // ⚠ A single lane has one arrangement, so Ctrl has nothing to choose
        // between — and rendering a second copy of it would be work and disk
        // spent on a switch that can never fire.
        let drags = Drags::default();
        if drags.prepare(one_kick(), None, false).is_err() {
            return;
        }
        assert_eq!(settle(&drags), Status::Ready);

        {
            let slot = drags.slot.lock().unwrap();
            let Stage::Ready(prepared) = &*slot else {
                panic!("the render did not publish");
            };
            assert!(prepared.alternative.is_empty());
        }
        // Takes its own render back — see the sibling test above for what a
        // `Ready` left standing does to the leak scan.
        drags.cancel();
    }

    #[test]
    fn a_folder_an_earlier_run_left_behind_is_stepped_over_rather_than_reused() {
        let _spool = spool_guard();
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

    /// Progress-with-cancel (2026-08-06).
    ///
    /// ⛔⛔ Mike: *"we need to do progress-with-cancel so that way we can drag
    /// the entire song arrangement to the DAW all at once."* Before this the
    /// bridge refused a song's audio outright, because a record is minutes of
    /// rendering with no way to say how far along it was and no way to stop it.
    mod progress {
        use super::*;

        #[test]
        fn a_render_reports_how_far_it_has_got() {
            let progress = Progress::detached();
            // ⚠ Read back the way  reads it — as the bits of an f32,
            // which is how  has always published a
            // 0..1 fraction through an atomic.
            let published = || f32::from_bits(progress.published.load(Ordering::Relaxed));
            assert!(progress.step(1, 4));
            assert_eq!(published(), 0.25);
            assert!(progress.step(4, 4));
            assert_eq!(published(), 1.0);
        }

        #[test]
        fn nothing_to_do_is_reported_as_no_progress_rather_than_dividing_by_zero() {
            let progress = Progress::detached();
            assert!(progress.step(0, 0));
            assert_eq!(
                f32::from_bits(progress.published.load(Ordering::Relaxed)),
                0.0
            );
        }

        /// ⛔ **The cancel half, and it is the existing disown made promptly
        /// observable.** `cancel` always replaced the slot so a finished render
        /// would throw its own work away; what it could not do was *stop* the
        /// work. For a four-bar loop that cost nothing. For a whole arrangement
        /// it is seconds of CPU inside somebody's DAW, spent after they let go.
        #[test]
        fn a_disowned_render_is_told_to_stop_rather_than_finishing_unwanted() {
            let progress = Progress::detached();
            assert!(progress.step(1, 4), "it should still be wanted");

            // Exactly what `cancel` does: move the slot out from under it.
            *progress.slot.lock().unwrap() = Stage::Idle;

            assert!(!progress.wanted());
            assert!(
                !progress.step(2, 4),
                "a render nobody is waiting for was told to carry on"
            );
        }

        #[test]
        fn a_render_superseded_by_a_newer_one_is_also_told_to_stop() {
            // ⚠ Not the same as a cancel: the slot is still `Preparing`, but for
            // a *different* gesture. The id is what tells them apart, and
            // without checking it the older render would go on burning CPU for
            // a drag the producer has already replaced.
            let progress = Progress::detached();
            *progress.slot.lock().unwrap() = Stage::Preparing(progress.id + 1);

            assert!(!progress.wanted());
        }

        #[test]
        fn the_page_is_told_the_figure_the_render_published() {
            // ⚠ End to end through `status`, because the number crossing the
            // bridge is the one the producer reads — an atomic that advanced
            // while `status` reported nothing would be worse than no bar.
            let drags = Drags::default();
            drags.progress.store(0.375f32.to_bits(), Ordering::Relaxed);
            *drags.slot.lock().unwrap() = Stage::Preparing(0);

            match drags.status() {
                Status::Preparing { done } => assert!((done - 0.375).abs() < 0.001),
                other => panic!("expected a preparing status, got {other:?}"),
            }
        }
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
            None,
            &Progress::detached()
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
            alternative: Vec::new(),
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
        let _spool = spool_guard();
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
