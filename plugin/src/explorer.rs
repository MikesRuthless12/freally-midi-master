//! The sample File Explorer (TASK-132).
//!
//! Mike, 2026-08-06: *"when we do the 'File Explorer' then we will be able to
//! drop samples on the generators and drum lanes."*
//!
//! ## What this is, and what it deliberately is not
//!
//! It is a **library of saved folders**, and a listing of one of them. Mike
//! chose that over a single picked folder on 2026-08-07: a library is set up
//! once, not once per project, and re-picking it every session is what makes a
//! browser not worth opening. Adding one opens a native dialog on its own
//! thread for the reason [`crate::oneshot`] spells out at length — a modal
//! dialog on the editor thread stalls the host — and listing is a cheap
//! synchronous read the page can ask for whenever it likes.
//!
//! ⛔⛔ **Browsing cannot leave the folders the producer added.** The path
//! arrives from the webview, which is untrusted in exactly the sense
//! `oneshot::refuse_remote` exists for: a page free to pass any string could
//! enumerate the whole disk and hand the listing back through a channel that
//! leaves the process. `open` refuses anything outside a saved root, compared
//! **canonically** so `..` cannot spell its way out.
//!
//! ⛔ **It does not load anything.** Dropping a file onto a lane goes through
//! [`crate::oneshot::OneShots::restore`], which already decodes, refuses a
//! remote path, reports a missing file and rebuilds the kit. A second loader
//! here would be a second set of those rules to keep in agreement, and this
//! project's own record is that the second copy is the one that drifts.
//!
//! ## Why the listing is not recursive
//!
//! A producer's sample library is tens of thousands of files across hundreds of
//! folders. Walking it on the editor thread would stall the host exactly the way
//! the dialog would, and streaming it would need a protocol the page does not
//! have. One folder at a time is also how every DAW browser behaves.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// The audio extensions [`crate::audio::import`] can actually decode.
///
/// ⛔ **Re-exported from `engine::formats`, not restated.** This was its own
/// literal and the Open dialog's filter in `oneshot.rs` was another — two copies
/// that happened to agree, with nothing making them and nothing that would have
/// noticed when they stopped. TASK-058 calls that out by name: a file the
/// explorer offers and the loader refuses is a row that does nothing when you
/// drop it, with no way for the producer to tell why.
use engine::formats::{kind_of, Kind, AUDIO};

/// How many entries one folder may list.
///
/// ⛔ **A bound, because a folder is attacker-controlled in the same sense a
/// project file is** — and because a producer really does have folders with
/// 50,000 samples in them. The page renders every row it is given, so an
/// unbounded list is a multi-second layout inside somebody's DAW.
const MAX_ENTRIES: usize = 2_000;

/// How many library folders a project may carry.
///
/// ⛔ **`MAX_ENTRIES` bounds a listing; nothing bounded the root count.** The
/// list arrives from a project file, and `open` and `state()` both walk every
/// root canonicalising as they go — so 100,000 roots is 100,000 syscalls per
/// poll on the thread the host draws its window from.
///
/// ⛔⛔ **Eight, and it is the producer's number rather than a safety margin.**
/// Mike, 2026-08-10: *"i should be able to have up to 8 folders in the view to be
/// able to be tabbed to be used and sifted through at any given time, and if you
/// want to add more, then you have to exit out of one of them."* It was 24 — a
/// bound chosen to stop a hostile project file, which it still does; what it did
/// not do was give the page a rule it could show, so the ninth folder simply
/// vanished. The page disables **Add folder** at eight; this refuses a ninth
/// however it arrives, because a disabled button is a UI state and this is the
/// real bound.
pub const MAX_ROOTS: usize = 8;

/// How many roots may be *carried* — as opposed to added.
///
/// ⛔⛔ **These are two different numbers and conflating them destroyed data.**
/// [`MAX_ROOTS`] is what a producer may *add up to*; this is what a project or a
/// `library.json` written by an older build may legitimately still contain. It was
/// 24 before the tab strip, and lowering the stored bound to 8 truncated existing
/// projects — which `explorer_state` then wrote back, deleting the rest for good.
///
/// ⚠ It is still a bound rather than trust: `state()` canonicalises every root on
/// the editor thread, so an unbounded list from a hostile file is a syscall storm
/// per poll. Twenty-four is what shipped, and is past any real library.
const MAX_STORED_ROOTS: usize = 24;

/// What is written to `library.json`.
///
/// A struct rather than a bare array so a later field — a favourite, a per-folder
/// note — does not need a new file and a migration. Same reasoning `eula::Record`
/// records for the same reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Library {
    folders: Vec<String>,
}

/// Where the sample library lives between sessions.
///
/// ⛔⛔ **Per user, not per project, and that is Mike's own rule arriving where
/// it was never actually applied.** He chose a saved library over a folder picker
/// on 2026-08-07 because *"a library is set up once, not once per project"* — and
/// then the only place it was written was `PluginSession::sample_folders`, which
/// the **host** serialises into the project file. So it came back with an `.als`
/// and never with the standalone. Mike, 2026-08-10: *"the folders should persist
/// between app openings."*
///
/// ⚠ **`eula.rs` had already written down the distinction this needed**: a
/// property of *this person on this machine* does not belong in a blob that
/// travels inside a song you send to a label.
///
/// ⚠ The project blob is still read and still written — see
/// `Shared::restore_sample_folders`. Dropping it would lose the library of every
/// project saved before this, and a producer who set one up per project is not
/// wrong to expect it back.
fn library_path() -> Option<PathBuf> {
    crate::presets::data_dir().map(|dir| dir.join("library.json"))
}

/// The folders this machine had last time, or empty if there is no record.
pub fn saved_folders() -> Vec<String> {
    library_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Library>(&text).ok())
        .map(|library| library.folders)
        .unwrap_or_default()
}

/// Write the library for next time.
///
/// ⚠ **Failure is swallowed deliberately.** This runs when a producer adds or
/// removes a folder, and the folder *is* added either way — the panel already
/// shows it. Surfacing "could not write your library" over a working action would
/// be an error about the next session, raised during this one, with nothing the
/// producer can do about it. It is logged where the plugin's other disk problems
/// are.
fn save_folders(folders: &[String]) {
    let Some(path) = library_path() else { return };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            nih_plug::nih_log!("could not create {parent:?} for the sample library: {error}");
            return;
        }
    }
    let library = Library {
        folders: folders.to_vec(),
    };
    // ⛔ **Temp-and-rename**, for the reason `favourites::write` gives: this is a
    // whole-list rewrite of state the producer cannot reconstruct, and a
    // truncated file reads back as an empty library.
    match serde_json::to_string_pretty(&library) {
        Ok(text) => {
            if let Err(error) = crate::patterns::write_atomic(&path, &text) {
                nih_plug::nih_log!("could not write the sample library: {error}");
            }
        }
        Err(error) => nih_plug::nih_log!("could not serialise the sample library: {error}"),
    }
}

/// The library to restore, from the project's copy and this machine's.
///
/// ⛔ **Both, rather than one winning.** Dropping the project's copy loses the
/// library of every project saved before `library.json` existed; dropping the
/// machine's puts back the bug where a standalone launch has no folders at all.
///
/// ⚠ The project's come first: a producer who opened *this song* most likely
/// wants its folders at the front, and [`Explorer::restore`] keeps the first
/// [`MAX_ROOTS`].
pub fn merge_folders(from_project: Vec<String>, from_machine: Vec<String>) -> Vec<String> {
    let mut folders = from_project;
    for folder in from_machine {
        // ⚠ Compared as written. Two spellings of one folder is the bug
        // `same_or_inside` records twice over — but these are both raw paths
        // saved the way the producer added them, and canonicalising here would
        // touch the filesystem for every root on every load, including the
        // unplugged-drive case this list deliberately keeps.
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

/// One row in the explorer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// What the producer reads.
    pub name: String,
    /// What a drop hands to `OneShots::restore`.
    pub path: String,
    /// Folders sort first and open on click; files are draggable.
    pub is_dir: bool,
    /// `"dir"`, `"audio"` or `"midi"`.
    ///
    /// ⛔⛔ **Two kinds, two sets of affordances, and the panel may not confuse
    /// them** — TASK-058's rule, stated as a field so the page cannot get it
    /// wrong per call site. An audio file gets the waveform, the audition and a
    /// drum pad. A `.mid` gets a piano-roll thumbnail, playback and
    /// drag-to-generator; **a waveform for a `.mid`, or a pad assignment for
    /// one, is a control that can only fail.**
    pub kind: &'static str,
}

/// What the page draws.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The producer's saved library folders, in the order they were added.
    ///
    /// ⛔ **These persist; the browse location does not.** Mike, 2026-08-07,
    /// choosing this model over a single picked folder: a library is set up
    /// once, not once per project. Re-picking it every session is the thing
    /// that makes a browser not worth opening.
    pub roots: Vec<Entry>,
    /// The folder being shown, or `None` before one is chosen.
    pub folder: Option<String>,
    /// Its parent, so the page can offer "up" without parsing paths itself.
    ///
    /// ⚠ `None` at a root, so "up" cannot walk out of the library the producer
    /// added and into their home directory.
    pub parent: Option<String>,
    pub entries: Vec<Entry>,
    /// Whether a folder dialog is open right now.
    ///
    /// ⛔ **The page has no other way to know, and without it the poll is a
    /// guess.** Adding a folder opens a modal on its own thread — see
    /// [`Explorer::pick`] — so the only way the page learns it finished is by
    /// asking again. It used to ask 301 times at 400 ms whatever happened, so a
    /// *cancelled* dialog cost two minutes of `read_dir` over a folder that may
    /// hold [`MAX_ENTRIES`] entries, serialized and re-rendered each time. This
    /// makes the poll last exactly as long as the dialog does.
    pub picking: bool,
    /// Set when the folder held more than [`MAX_ENTRIES`].
    ///
    /// ⚠ **Reported rather than silently truncated.** A list that stops at 2000
    /// with nothing saying so is the readout-that-lies failure this project
    /// keeps recording — the producer scrolls, does not find their sample, and
    /// concludes the explorer is broken.
    pub truncated: bool,
}

/// The chosen folder, per plugin instance.
///
/// ⚠ **Per instance for the reason [`crate::oneshot::OneShots`] gives**: the
/// folder a producer is working from on one track is not the one they want on
/// the next, and a process global would let one instance move another's view.
#[derive(Debug, Default)]
pub struct Explorer {
    /// The saved library folders. Persisted with the project.
    roots: Arc<Mutex<Vec<PathBuf>>>,
    /// Where the producer is browsing right now. **Not** persisted — a
    /// position in a tree is not a preference, and restoring somebody three
    /// levels into a folder they were passing through is noise.
    folder: Arc<Mutex<Option<PathBuf>>>,
    /// Whether a dialog is already up — the same one-at-a-time claim `oneshot`
    /// and `export` both make, and for the same reason.
    picking: Arc<Mutex<bool>>,
}

impl Explorer {
    /// The saved roots, for writing into the project.
    pub fn snapshot(&self) -> Vec<String> {
        self.roots
            .lock()
            .map(|roots| {
                roots
                    .iter()
                    .filter_map(|p| p.to_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Put the saved roots back when a project reopens.
    ///
    /// ⚠ **A folder that no longer exists is kept, not dropped.** An external
    /// drive that happens to be unplugged is the common case, and silently
    /// removing the producer's library because they booted without it would be
    /// unrecoverable — they would have to remember what was there. The page
    /// draws it as unavailable instead.
    pub fn restore(&self, paths: &[String]) {
        // ⛔⛔ **A PROJECT FILE IS UNTRUSTED, and this re-opened the hole
        // `one_shots` already closed.** Both ride the same `#[persist]` blob;
        // `oneshot.rs:145` guards its paths and this guarded nothing. A shared
        // project or preset carrying
        // `"sampleFolders": ["\\\\evil.example.com\\share"]` would be restored
        // on load, and then **every** `open` and **every** `state()` poll runs
        // `canonicalize` over all roots — so it is not one outbound SMB
        // authentication, it is one per poll, indefinitely.
        //
        // ⚠ The page cannot add a root: the commands are pick (native dialog),
        // remove, open, state, waveform and drop. The project file is the only
        // injection point, which is precisely the vector `refuse_remote`
        // describes.
        //
        // ⚠ Refused entries are **dropped**, not kept-and-marked. The
        // unplugged-drive kindness above deliberately keeps a root that cannot
        // be resolved — but a remote path is not an absent drive, and keeping
        // it would leave the payload armed in the project for the next load.
        // ⛔⛔ **NOT `.take(MAX_ROOTS)` — that was a data-loss bug.** Found by a
        // code review, 2026-08-10, and introduced the same night by lowering the
        // bound from 24 to 8 for the tab strip.
        //
        // A project saved under the old bound can legally carry twelve folders.
        // Truncating on load would be survivable on its own; what made it
        // destructive is `explorer_state` — it calls `store_sample_folders()` on
        // **every poll**, writing the surviving list straight back over
        // `session.sample_folders`. So the four dropped folders were gone from
        // the producer's project the next time the host saved, and gone from
        // `library.json` the next time they added or removed one. Silently.
        //
        // ▶ **A bound on what the page may ADD is not a licence to delete what a
        // producer already had.** `pick` refuses a ninth; the tab strip shows
        // however many are really there. Over the bound the panel is crowded,
        // which the producer can see and fix — the opposite of a loss they
        // cannot.
        let kept: Vec<PathBuf> = paths
            .iter()
            .map(PathBuf::from)
            .filter(|path| crate::oneshot::refuse_remote(path).is_ok())
            .filter(|path| is_plausible_library(path))
            // ⚠ A bound is still needed against a hostile file — `state()` walks
            // every root canonicalising on the editor thread — but it is far
            // above any real library rather than at the UI's tab count.
            .take(MAX_STORED_ROOTS)
            .collect();
        if let Ok(mut roots) = self.roots.lock() {
            *roots = kept;
        }
    }

    /// Forget a saved root. Browsing moves out of it if that is where we were.
    pub fn remove(&self, path: &str) {
        let target = PathBuf::from(path);
        if let Ok(mut roots) = self.roots.lock() {
            roots.retain(|root| root != &target);
        }
        // ⚠ Written here rather than on the `explorer_state` poll: the poll runs
        // every 500ms for the life of the editor, and a JSON write at that rate
        // inside somebody's DAW is a cost for nothing. Adding and removing are
        // the only two things that change the library.
        save_folders(&self.snapshot());
        // ⛔ Otherwise the list loses the folder and the pane goes on showing
        // its contents, with no root highlighted and no way back — a view of
        // something that is no longer in the library.
        if let Ok(mut folder) = self.folder.lock() {
            if folder
                .as_ref()
                .is_some_and(|at| same_or_inside(&target, at))
            {
                *folder = None;
            }
        }
    }

    /// Open a folder dialog, and add what was picked to the library.
    ///
    /// Returns as soon as the thread is running. The page polls [`Self::state`],
    /// which is how it already learns about everything else that happens off
    /// the editor thread.
    pub fn pick(&self) -> Result<(), String> {
        // ⛔⛔ **Refused at [`MAX_ROOTS`], before the dialog opens.** Mike,
        // 2026-08-10: *"if you want to add more, then you have to exit out of one
        // of them."* The page disables **Add folder** at eight, so this is the
        // path nobody should reach — and it is here anyway because a disabled
        // button is a UI state and the bound is a real one. ⚠ Refused *before*
        // the dialog rather than after: letting a producer browse to a folder and
        // then telling them it cannot be added is the worse half of both.
        {
            let roots = self
                .roots
                .lock()
                .map_err(|_| "the explorer is unavailable")?;
            if roots.len() >= MAX_ROOTS {
                return Err(format!(
                    "you can browse {MAX_ROOTS} folders at a time — close one to add another"
                ));
            }
        }

        {
            let mut picking = self
                .picking
                .lock()
                .map_err(|_| "the explorer is unavailable")?;
            if *picking {
                return Err("a folder dialog is already open".into());
            }
            *picking = true;
        }

        let roots = Arc::clone(&self.roots);
        let folder = Arc::clone(&self.folder);
        let picking = Arc::clone(&self.picking);
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new().pick_folder();
            if let Some(dir) = picked {
                if let Ok(mut roots) = roots.lock() {
                    // Adding a folder twice is a no-op rather than a second
                    // identical row, which is what a producer means when they
                    // add one they already have.
                    // ⚠ The bound is re-checked here as well as before the
                    // dialog: this runs minutes later on another thread, and the
                    // invariant is "never more than `MAX_ROOTS`" rather than
                    // "the count was fine when we asked".
                    if !roots.contains(&dir) && roots.len() < MAX_ROOTS {
                        roots.push(dir.clone());
                    }
                    // ⚠ Inside the lock's scope, from the snapshot we already
                    // hold, so the file cannot disagree with the list in memory.
                    let folders: Vec<String> = roots
                        .iter()
                        .filter_map(|root| root.to_str().map(str::to_owned))
                        .collect();
                    drop(roots);
                    save_folders(&folders);
                }
                // Show it straight away — adding a folder in order to then
                // click it is a step nobody wants.
                if let Ok(mut slot) = folder.lock() {
                    *slot = Some(dir);
                }
            }
            // ⛔ Always cleared, on every path including a cancel. A flag left
            // set refuses every dialog after it with no way to recover short of
            // reloading the plugin.
            if let Ok(mut flag) = picking.lock() {
                *flag = false;
            }
        });
        Ok(())
    }

    /// Show `dir` instead — how the page walks into a subfolder or back up.
    ///
    /// ⛔⛔ **Refused unless it is inside a folder the producer added, and this
    /// is a boundary rather than a nicety.** The path arrives from the webview,
    /// which is the same untrusted door `oneshot::refuse_remote` exists for: a
    /// page that could pass any string would let the plugin enumerate the whole
    /// disk — every folder name, every sample name — and hand the listing back
    /// through a channel that leaves the process. Adding a folder grants access
    /// to *that* folder; it is not a licence to walk out of it.
    ///
    /// ⚠ **Canonicalised before comparing**, or `library/../../..` passes a
    /// `starts_with` on the literal string and defeats the whole check.
    pub fn open(&self, dir: &str) -> Result<(), String> {
        let path = PathBuf::from(dir);
        // ⛔⛔ **FIRST, before anything touches the filesystem.** The
        // containment check below refused the *listing* but the side effect had
        // already happened: `is_dir()` on `\\evil.example.com\share` makes the
        // SMB redirector resolve the host, connect out and **authenticate**,
        // handing over NetNTLMv2. `oneshot::refuse_remote` exists for exactly
        // that and its doc calls it a real vulnerability rather than hardening.
        // Refusing after the syscall is cold comfort.
        crate::oneshot::refuse_remote(&path)?;

        // ⚠ **One error for all three outcomes**, deliberately. Distinct
        // messages for "does not exist" / "cannot be read" / "not in your
        // library" let a page with no other filesystem reach map the whole disk
        // one probe at a time.
        // ⚠ **The same words whatever went wrong, and the path is not echoed
        // back.** Naming it would re-confirm to the caller what it already
        // sent, which still separates "this exists" from "this does not".
        let refused = || String::from("that is not a folder in your sample library");
        if !path.is_dir() {
            return Err(refused());
        }
        let real = path.canonicalize().map_err(|_| refused())?;

        let roots = self
            .roots
            .lock()
            .map_err(|_| "the explorer is unavailable")?;
        let inside = roots.iter().any(|root| same_or_inside(root, &real));
        if !inside {
            return Err(refused());
        }
        drop(roots);

        let mut slot = self
            .folder
            .lock()
            .map_err(|_| "the explorer is unavailable")?;
        *slot = Some(real);
        Ok(())
    }

    /// Read one folder's rows **without moving where the producer is browsing**.
    ///
    /// ⛔⛔ **This is what makes a tree possible at all.** Mike, 2026-08-10:
    /// *"you need to show it like a real file explorer does, where the main
    /// folder is at the top, and then indents and shows the subfolders underneath
    /// and then the files beneath that."* A tree has **several** folders open at
    /// once; [`Explorer::state`] answers for exactly one, because `folder` is a
    /// single slot. Expanding a node through `open` would therefore collapse the
    /// idea of "where I am" into "the last thing I clicked", and the *selected*
    /// folder is not decoration — [`crate::oneshot::OneShots::randomize`] rolls a
    /// pad from it.
    ///
    /// So: `open` still means "this is the folder I am working from", and this
    /// means "also show me what is in that one". Two questions, two commands.
    ///
    /// ⚠ **Still one folder per call.** The module header's argument stands —
    /// walking a producer's whole library on the editor thread would stall the
    /// host. The page asks for a folder when the producer expands it, which is
    /// the same amount of work `open` already did, at the same moment.
    ///
    /// ⚠ Guarded exactly like `open`: remote first (before any syscall touches
    /// the path), then containment.
    pub fn list_one(&self, dir: &str) -> Result<State, String> {
        let path = PathBuf::from(dir);
        // ⛔ FIRST, before `is_dir` — the reason `open` gives: on Windows a
        // syscall against a UNC path *is* the outbound authentication.
        crate::oneshot::refuse_remote(&path)?;

        // The one message all three failures share, for the reason `open`
        // records: distinct wording is a filesystem oracle.
        let refused = || String::from("that is not a folder in your sample library");
        if !path.is_dir() {
            return Err(refused());
        }
        let real = path.canonicalize().map_err(|_| refused())?;

        let roots = self
            .roots
            .lock()
            .map_err(|_| "the explorer is unavailable")?;
        if !roots.iter().any(|root| same_or_inside(root, &real)) {
            return Err(refused());
        }
        drop(roots);

        Ok(list(&real))
    }

    /// Is `path` inside a folder the producer added?
    ///
    /// ⚠ **The same test `open` applies, exposed so the file-reading commands
    /// can apply it too.** `waveform` and `preview_load` both took a raw path
    /// from the page; the explorer must only describe and audition what it
    /// would list.
    pub fn contains(&self, path: &Path) -> bool {
        // ⛔⛔ **REMOTE FIRST, BEFORE THE CANONICALISE BELOW.** A security review
        // found this inverted on 2026-08-10 and it was a live credential leak.
        //
        // `canonicalize` on Windows is `CreateFileW` + `GetFinalPathNameByHandleW`
        // — on a UNC string that is **not a read**. It is the SMB redirector
        // resolving the host, connecting out and performing a session setup that
        // sends the logged-in user's NetNTLMv2 credentials. `\\host@SSL@443\s\a`
        // does the same over WebDAV on 443, so blocking 445 does not save you.
        //
        // ⛔ **The guard belongs HERE rather than at each call site**, which is
        // the whole finding: `favourites_add` applied containment *before* the
        // remote check and `explorer_drop` did the same, so one bridge message
        // with a `\\attacker\share\x.wav` path authenticated outbound and was
        // then politely told the file "is not in your sample library" — the
        // credentials already gone. Every other command had the order right.
        // Putting it in the one function that syscalls on a raw page string
        // makes the invariant unforgettable for the next command that grows a
        // `contains` call, which is exactly how both of these arrived.
        if crate::oneshot::refuse_remote(path).is_err() {
            return false;
        }

        // ⛔⛔ **Canonicalised HERE, because `same_or_inside`'s fallback is only
        // conservative for `open`.** `open` resolves the incoming path before
        // comparing, so its fallback pits a resolved path against a raw root
        // and fails closed. `contains` is handed the **raw** page-supplied
        // string, and the fallback then compared raw against raw — so with a
        // root that cannot be resolved (the unplugged-drive state `restore`
        // deliberately keeps) `library\..\secret.wav` matched `library` by
        // prefix and a file outside the library was decoded. A hostile project
        // file can plant a non-existent root and arm that permanently.
        let Ok(real) = path.canonicalize() else {
            return false;
        };
        let Ok(roots) = self.roots.lock() else {
            return false;
        };
        // ⚠ And the root must genuinely resolve: a root that does not exist
        // contains nothing, rather than everything that happens to share its
        // spelling.
        roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|root| real.starts_with(&root))
        })
    }

    /// The library, and the folder being browsed inside it.
    pub fn state(&self) -> State {
        let roots: Vec<PathBuf> = self
            .roots
            .lock()
            .map(|roots| roots.clone())
            .unwrap_or_default();

        let named: Vec<Entry> = roots
            .iter()
            .filter_map(|root| {
                let path = root.to_str()?;
                Some(Entry {
                    // The folder's own name, not the whole path: a producer
                    // recognises "Splice" far faster than
                    // "D:\Audio\Libraries\Splice".
                    name: root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path)
                        .to_owned(),
                    path: path.to_owned(),
                    is_dir: true,
                    kind: "dir",
                })
            })
            .collect();

        let at = self
            .folder
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());

        // ⚠ `try_lock` is not needed — this is the editor thread and the only
        // other holder is the dialog thread, which takes it twice for a couple
        // of instructions each. A poisoned lock reads as "no dialog", which is
        // the answer that lets the page stop polling rather than spin forever.
        let picking = self.picking.lock().map(|open| *open).unwrap_or(false);

        let Some(dir) = at else {
            return State {
                roots: named,
                picking,
                ..State::default()
            };
        };

        let mut state = list(&dir);
        state.picking = picking;
        // ⛔ **"Up" stops at a root.** Without this the producer can walk out of
        // the folder they added, up through their home directory, and browse
        // the whole disk from inside a plugin — which is both a surprise and a
        // wider reach than they granted by picking one folder.
        if roots.iter().any(|root| same_or_inside(&dir, root)) {
            state.parent = None;
        }
        state.roots = named;
        state
    }
}

/// Read one folder: subfolders first, then the audio files, both by name.
fn list(dir: &Path) -> State {
    let mut folders: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();
    let mut truncated = false;

    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            if folders.len() + files.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // A path that is not valid UTF-8 cannot cross the JSON bridge, so
            // it is skipped rather than mangled into something that will not
            // reopen.
            let Some(text) = path.to_str() else { continue };

            // entry.file_type() rather than path.is_dir(): the latter is a
            // second stat syscall per entry, and read_dir already returned the
            // type — free on Windows and on Linux alike.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                // ⚠ Hidden folders are skipped: on macOS and Linux a sample
                // library is full of `.git`, `.DS_Store` and friends, and a
                // browser that shows them is a browser nobody scrolls.
                if name.starts_with('.') {
                    continue;
                }
                folders.push(Entry {
                    name: name.to_owned(),
                    path: text.to_owned(),
                    is_dir: true,
                    kind: "dir",
                });
            } else if let Some(kind) = kind_of(&path) {
                // ⛔ **MIDI is listed alongside audio now** — Mike, 2026-08-10:
                // *"i want to be able to view .mid files in the File
                // Explorer."* It was filtered out entirely, which is why
                // `explorer_midi` shipped with nothing able to reach it.
                files.push(Entry {
                    name: name.to_owned(),
                    path: text.to_owned(),
                    is_dir: false,
                    kind: match kind {
                        Kind::Audio => "audio",
                        Kind::Midi => "midi",
                    },
                });
            }
        }
    }

    // sort_by_cached_key, not sort_by_key: the latter evaluates the key inside
    // the comparator, so a 2000-entry folder allocated ~40,000 lowercased
    // Strings instead of 2000.
    folders.sort_by_cached_key(|e| e.name.to_lowercase());
    files.sort_by_cached_key(|e| e.name.to_lowercase());
    folders.extend(files);

    State {
        // Filled in by `Explorer::state`, which is the only thing that knows
        // the library. `list` reads one folder and nothing else.
        roots: Vec::new(),
        folder: dir.to_str().map(str::to_owned),
        parent: dir.parent().and_then(|p| p.to_str()).map(str::to_owned),
        entries: folders,
        truncated,
        // Filled in by `Explorer::state`, which is the only thing that knows —
        // `list` reads one folder and nothing else.
        picking: false,
    }
}

/// How many peak pairs a waveform is reduced to.
///
/// ⛔ **The audio itself must not cross the bridge.** A four-second stereo
/// sample is ~700 KB of f32; as JSON it is several megabytes, serialized on the
/// editor thread, inside somebody's DAW. 800 columns is more than any panel is
/// wide, so the page has a pixel per column and nothing is lost that could be
/// drawn.
const WAVE_COLUMNS: usize = 800;

/// The drawable shape of a sample: min/max per column, plus its length.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    /// The file this describes, so a late reply cannot draw over a newer one.
    pub path: String,
    pub name: String,
    /// `[min, max]` per column, each in -1.0..=1.0.
    ///
    /// ⚠ **Both, not one amplitude.** A single peak per column draws a
    /// half-waveform that reads as a DC-offset recording; min and max is what
    /// makes it look like every other editor's.
    pub peaks: Vec<[f32; 2]>,
    /// Total length in seconds, for the time readout and for mapping a click
    /// to a position.
    pub seconds: f32,
}

/// Decode `path` and reduce it to something the page can draw.
///
/// ⚠ **Runs where it is called from — the editor thread.** Decoding a one-shot
/// is milliseconds, which is why this is not a mailbox like the dialog is; a
/// sample long enough to be worth worrying about is refused by `import`'s own
/// size bound before it reaches here.
pub fn waveform(explorer: &Explorer, path: &str) -> Result<Waveform, String> {
    let file = Path::new(path);
    // ⛔⛔ **The same guard `open` needs, and for the same reason.** This took a
    // raw path from the page with nothing but an extension check — which
    // `\\evil.example.com\share\a.wav` passes — and then ran `fs::metadata` and
    // `fs::read` on it. Three impacts in one: an outbound SMB authentication,
    // a filesystem oracle (the raw `io::Error` distinguishes missing from
    // unreadable for any path on disk), and symphonia handed arbitrary bytes
    // without the producer ever picking a file.
    crate::oneshot::refuse_remote(file)?;

    // ⛔ **And containment: the explorer may only describe what it would
    // list.** The module header claims browsing cannot leave the folders the
    // producer added; that claim did not hold for this function.
    if !explorer.contains(file) {
        return Err("that sample is not in your sample library".into());
    }
    if !is_audio(file) {
        return Err("that is not a sample this plugin can read".into());
    }
    let audio = crate::audio::import::decode_file(file)?;
    if audio.samples.is_empty() || audio.sample_rate == 0 {
        return Err("that file decoded to no audio".into());
    }

    // Non-empty is proven above, so no guard is needed on either side.
    let per = audio.samples.len().div_ceil(WAVE_COLUMNS);
    let peaks: Vec<[f32; 2]> = audio
        .samples
        .chunks(per)
        .map(|chunk| {
            let mut low = f32::MAX;
            let mut high = f32::MIN;
            for sample in chunk {
                low = low.min(*sample);
                high = high.max(*sample);
            }
            [low, high]
        })
        .collect();

    Ok(Waveform {
        path: path.to_owned(),
        name: file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_owned(),
        peaks,
        seconds: audio.samples.len() as f32 / audio.sample_rate as f32,
    })
}

/// Could this plausibly be a folder somebody keeps samples in?
///
/// ⛔⛔ **Because the containment boundary is defined by an UNTRUSTED blob.**
/// Found by a security review, 2026-08-10. `sample_folders` rides the
/// `#[persist]` state, so a shared `.als`, preset or type-beat starter can carry
/// `"sampleFolders": ["C:\\Users"]` — an ordinary local path that
/// `refuse_remote` passes. `restore` then installs it, and because `contains` is
/// `real.starts_with(root)`, **every containment check in the plugin is
/// satisfied for that whole tree**. The page could walk it with `explorer_list`,
/// read `.mid` content back through `explorer_song`, decode any audio through
/// `explorer_waveform`, and star-then-`reveal` any file in it — the one command
/// that launches a process.
///
/// ▶ **This does not make the blob trusted; it removes the useful targets.**
/// A drive root and a user-profile root are the two shapes worth planting and
/// neither is a sample library — nobody adds `C:\` or `C:\Users\them` as one.
/// A folder the producer genuinely picked is always deeper than that.
///
/// ⚠ **Depth, not a name list.** Blocking `C:\Users` by spelling would miss
/// `C:\Users\..\Users`, every non-English profile root and every other OS. Two
/// components under the prefix is the rule, and it is the same rule on all
/// three platforms.
///
/// ⚠ **Only on the way in from a FILE.** `Explorer::pick` does not go through
/// this: a producer who genuinely wants to add a shallow folder through the
/// native dialog is choosing it, which is the distinction the whole guard turns
/// on.
fn is_plausible_library(path: &Path) -> bool {
    use std::path::Component;
    // Prefix and root separator are not folders anybody chose.
    let depth = path
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .count();
    depth >= 2
}

/// Is `path` the same folder as `root`, or inside it?
///
/// ⛔ **Both sides canonicalised, and mixing the two spellings is a bug this
/// already caused.** `open` stores the canonical form — on Windows that is the
/// `\\?\C:\…` verbatim prefix, which is not textually equal to the `C:\…` a
/// root was added as — so comparing a canonical browse location against a raw
/// root matched nothing: "up" was offered at a root, and removing a root left
/// its contents on screen.
///
/// ⚠ Falls back to a literal comparison when a path cannot be canonicalised,
/// which is the unplugged-drive case. That is deliberately the *conservative*
/// direction for `contains`: a root that cannot be resolved matches only
/// itself, so it can never widen what `open` allows.
fn same_or_inside(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(&root),
        _ => path.starts_with(root),
    }
}

/// Does this name end in something the decoder can read?
fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO.iter().any(|a| e.eq_ignore_ascii_case(a)))
        .unwrap_or(false)
}

/// The longest MIDI file this will read, in bytes.
///
/// A four-bar loop is a couple of kilobytes; a megabyte is a type-1 orchestral
/// score, and reading one into a training set is not what anybody meant. Bounded
/// before the read rather than after, so an enormous file costs a `metadata`
/// call rather than the memory.
const MAX_MIDI_BYTES: u64 = 1 << 20;

/// Read a `.mid` from the producer's own library as a trainable pattern
/// (TASK-040T).
///
/// ⛔ **The same two guards `waveform` needs, and for the same reasons.** The
/// path arrives from the page: `refuse_remote` first, because a UNC path makes
/// the SMB redirector authenticate outward before any containment check could
/// refuse it; then containment, because the module's claim is that browsing
/// cannot leave the folders the producer added, and a reader that ignored it
/// would be a second door out of the library.
pub fn midi_pattern(
    explorer: &Explorer,
    path: &str,
    part: engine::pattern::Part,
) -> Result<engine::pattern::Pattern, String> {
    let (bytes, name) = read_midi_bytes(explorer, path)?;
    engine::smf_read::smf_to_pattern(&bytes, part, &name)
}

/// The bytes of a `.mid` in the library, and the name to give what comes out.
///
/// ⛔ **Every guard the two MIDI commands share, in one place.** They were about
/// to be two copies of the same four checks — remote, containment, extension,
/// size — and this codebase's own record is that the second copy is the one that
/// drifts. `explorer::AUDIO` and the Open dialog's filter had exactly that shape
/// until `engine::formats` took it away.
fn read_midi_bytes(explorer: &Explorer, path: &str) -> Result<(Vec<u8>, String), String> {
    let file = Path::new(path);
    // ⛔ First, before any syscall touches it — the reason `open` gives.
    crate::oneshot::refuse_remote(file)?;

    // ⚠ **One error for every outcome**, as `open` does: distinct messages
    // would let a page map the disk one probe at a time.
    let refused = || String::from("that is not a MIDI file in your sample library");
    if !explorer.contains(file) {
        return Err(refused());
    }
    // ⚠ `engine::formats`, not a third spelling of the extension list.
    if kind_of(file) != Some(Kind::Midi) {
        return Err(refused());
    }

    let size = std::fs::metadata(file)
        .map(|meta| meta.len())
        .map_err(|_| refused())?;
    if size > MAX_MIDI_BYTES {
        return Err("that MIDI file is too large to train from".into());
    }

    let bytes = std::fs::read(file).map_err(|_| refused())?;
    let name = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".to_owned());
    Ok((bytes, name))
}

/// Separate a layered `.mid` into the generators its voices belong to.
///
/// ⛔ Mike, 2026-08-10: *"split it into the proper generators if it is a layered
/// melody file with the bass and countermelody included."*
///
/// ⚠ **The same two guards, in the same order, as every other path command.**
/// The only difference from [`midi_pattern`] is which engine function reads the
/// bytes — so the checks are lifted into [`read_midi_bytes`] rather than written
/// twice, because a second copy is the one that would drift.
pub fn midi_split(
    explorer: &Explorer,
    path: &str,
) -> Result<Vec<engine::smf_read::SplitPart>, String> {
    let (bytes, name) = read_midi_bytes(explorer, path)?;
    engine::smf_read::split(&bytes, &name)
}

/// Read a whole `.mid` as an arrangement, for the Song tab.
///
/// ⛔ Mike, 2026-08-10: *"could you put the midi for the entire song into the
/// 'Song' tab and allow them to pick which parts they want for the generators,
/// just like you would for a generation, but using someone elses song as the
/// starting point?"*
pub fn midi_song(explorer: &Explorer, path: &str) -> Result<engine::pattern::Song, String> {
    let (bytes, name) = read_midi_bytes(explorer, path)?;
    engine::smf_read::smf_to_song(&bytes, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own.
    ///
    /// ⛔ **Named per test, not per process.** Both callers used one path keyed
    /// on the pid, and `cargo test` runs them on separate threads — so whichever
    /// started second deleted the other's fixture mid-read. A genuine race that
    /// failed differently depending on scheduling.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fmm-explorer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    #[test]
    fn lists_audio_and_midi_and_folders_and_nothing_else() {
        let dir = temp("listing");
        std::fs::write(dir.join("kick.wav"), b"x").unwrap();
        std::fs::write(dir.join("snare.WAV"), b"x").unwrap();
        std::fs::write(dir.join("riff.mid"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("cover.png"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("Kicks")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();

        let state = list(&dir);
        let names: Vec<&str> = state.entries.iter().map(|e| e.name.as_str()).collect();

        // Folders first, then the files, each by name. The extension check is
        // case-insensitive because a sample library is full of `.WAV`.
        assert_eq!(names, vec!["Kicks", "kick.wav", "riff.mid", "snare.WAV"]);
        assert!(!state.truncated);
        assert!(state.entries[0].is_dir, "the folder sorts first");
        assert!(!state.entries[1].is_dir, "then the files");

        // ⛔⛔ **Two kinds, and the page is told which.** Mike, 2026-08-10:
        // *"i want to be able to view .mid files in the File Explorer."* They
        // were filtered out entirely before, which is why `explorer_midi`
        // shipped with nothing able to reach it. ⚠ The kind is what stops the
        // panel offering a waveform or a drum pad for a `.mid` — controls that
        // can only fail on one.
        let kinds: Vec<&str> = state.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec!["dir", "audio", "midi", "audio"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_past_the_bound_says_so_rather_than_stopping_quietly() {
        // ⛔ The failure this reports instead of: a producer with a 50,000
        // sample folder scrolls, does not find their kick, and concludes the
        // explorer is broken. Truncating is fine; truncating in silence is not.
        let dir = temp("bound");
        for i in 0..(MAX_ENTRIES + 10) {
            std::fs::write(dir.join(format!("s{i:05}.wav")), b"x").unwrap();
        }

        let state = list(&dir);
        assert!(state.truncated, "the page has to be able to say so");
        assert!(state.entries.len() <= MAX_ENTRIES);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_that_does_not_exist_is_empty_rather_than_a_panic() {
        let state = list(Path::new("this-folder-does-not-exist-anywhere"));
        assert!(state.entries.is_empty());
        assert!(!state.truncated);
    }

    /// ⛔⛔ **Browsing has to go more than one folder deep.**
    ///
    /// Mike, 2026-08-10: *"you can get to the subfolders list, but you cannot go
    /// into those subfolders."* And he was right — for a reason no test here
    /// could see, because every one of them opened a folder by a path it had
    /// built itself rather than by the path the **page** would have been handed.
    ///
    /// `open` stores `canonicalize`'s answer, which on Windows is `\\?\C:\…`.
    /// `list` then builds every row from `entry.path()` under that, so the rows
    /// wear the prefix too — and `refuse_remote` classified anything starting
    /// `\\` as a network path. Opening a root worked (roots are stored as they
    /// were added); opening a child of one was refused, every time.
    ///
    /// ⚠ **The assertion walks the entry, deliberately.** Re-deriving the
    /// subfolder path with `dir.join("Kicks")` is what the old tests did and it
    /// is exactly what hid this: the bug lives in the round trip through
    /// `state()`, not in either end of it.
    #[test]
    fn a_subfolder_can_be_opened_from_the_row_the_page_was_given() {
        let dir = temp("descend");
        std::fs::create_dir_all(dir.join("Kicks").join("Vinyl")).unwrap();
        std::fs::write(dir.join("Kicks").join("kick-01.wav"), b"x").unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);
        explorer
            .open(dir.to_str().unwrap())
            .expect("the root opens");

        // The row as the page received it — canonical prefix and all.
        let listed = explorer.state();
        let kicks = listed
            .entries
            .iter()
            .find(|entry| entry.name == "Kicks")
            .expect("the subfolder is listed");

        explorer
            .open(&kicks.path)
            .expect("a listed subfolder must open");

        // ...and it is genuinely inside it, two levels down from the root.
        let inside = explorer.state();
        let names: Vec<&str> = inside.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Vinyl", "kick-01.wav"]);

        // The third level too, which is where a real library actually lives.
        let vinyl = inside
            .entries
            .iter()
            .find(|entry| entry.name == "Vinyl")
            .expect("the sub-subfolder is listed");
        explorer
            .open(&vinyl.path)
            .expect("a sub-subfolder must open as well");

        // ⛔ And the file rows are usable too — `contains` is what `explorer_drop`,
        // `preview_load` and `explorer_waveform` all gate on, so a row that lists
        // but does not pass this is a sample that cannot be previewed or dropped.
        let file = listed
            .entries
            .iter()
            .find(|entry| !entry.is_dir)
            .or_else(|| inside.entries.iter().find(|entry| !entry.is_dir))
            .expect("a sample is listed");
        assert!(
            explorer.contains(Path::new(&file.path)),
            "a listed sample must be reachable: {}",
            file.path
        );
        assert!(
            crate::oneshot::refuse_remote(Path::new(&file.path)).is_ok(),
            "a listed sample must not read as a network path: {}",
            file.path
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⛔ **Expanding a node must not move the folder the producer is working
    /// from.** `randomize` rolls a pad from the *selected* folder, so if opening
    /// a twisty re-pointed that, expanding a branch to look at it would silently
    /// change what the dice draws from.
    #[test]
    fn listing_a_folder_leaves_the_browse_location_where_it_was() {
        let dir = temp("list-one");
        std::fs::create_dir_all(dir.join("Kicks")).unwrap();
        std::fs::write(dir.join("Kicks").join("kick-01.wav"), b"x").unwrap();
        std::fs::write(dir.join("clap.wav"), b"x").unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);
        explorer
            .open(dir.to_str().unwrap())
            .expect("the root opens");
        let before = explorer.state().folder;

        let kicks = explorer
            .list_one(dir.join("Kicks").to_str().unwrap())
            .expect("a contained folder lists");
        let names: Vec<&str> = kicks.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["kick-01.wav"]);

        assert_eq!(
            explorer.state().folder,
            before,
            "expanding a node is not navigating to it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same two guards every other path command applies. A new command that
    /// reads a folder is a new door, and it gets the same lock.
    #[test]
    fn listing_applies_containment_and_refuses_a_remote_path() {
        let dir = temp("list-guards");
        let outside = temp("list-outside");
        std::fs::create_dir_all(outside.join("secret")).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);

        let error = explorer
            .list_one(outside.join("secret").to_str().unwrap())
            .expect_err("a folder outside the library must be refused");
        assert!(error.contains("sample library"), "{error}");

        let error = explorer
            .list_one(r"\\evil.example.com\share")
            .expect_err("a remote path must be refused");
        assert!(error.contains("network path"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// ⛔⛔ **The library has to survive the app closing** — Mike, 2026-08-10:
    /// *"the folders should persist between app openings."*
    ///
    /// It only ever lived in `PluginSession::sample_folders`, which the host
    /// writes into the **project file** — so it came back with an `.als` and
    /// never with a standalone launch, which has no project to come back from.
    /// `library.json` is the per-user half; this is the rule that joins them.
    #[test]
    fn the_project_and_the_machine_libraries_are_merged_without_duplicates() {
        let merged = merge_folders(
            vec!["C:/from-project".into(), "C:/shared".into()],
            vec!["C:/shared".into(), "C:/from-machine".into()],
        );

        // The project's first, the machine's appended, and the folder both knew
        // about listed once rather than twice.
        assert_eq!(
            merged,
            vec![
                "C:/from-project".to_owned(),
                "C:/shared".to_owned(),
                "C:/from-machine".to_owned(),
            ]
        );
    }

    #[test]
    fn a_standalone_with_no_project_still_gets_its_library_back() {
        // The case the bug was actually about: nothing from the project, and the
        // machine's list is the whole answer rather than an empty browser.
        let merged = merge_folders(Vec::new(), vec!["C:/samples".into()]);
        assert_eq!(merged, vec!["C:/samples".to_owned()]);
    }

    #[test]
    fn the_saved_library_survives_a_json_round_trip() {
        // ⚠ A struct rather than a bare array, so a later field does not need a
        // new file and a migration — and `default` so a file written by an older
        // build still reads.
        let text = serde_json::to_string(&Library {
            folders: vec!["C:/samples".into()],
        })
        .expect("the library serialises");
        let back: Library = serde_json::from_str(&text).expect("and reads back");
        assert_eq!(back.folders, vec!["C:/samples".to_owned()]);

        let empty: Library = serde_json::from_str("{}").expect("an older file still reads");
        assert!(empty.folders.is_empty());
    }

    #[test]
    fn opening_something_that_is_not_a_folder_is_refused_without_echoing_it() {
        // ⚠ **Inverted deliberately.** This asserted the refusal named the path
        // back, which reads as helpful and is an oracle: echoing it confirms to
        // a caller that already knows the string nothing, while the *distinct*
        // wording it came with told them whether the path existed. One message
        // for every failure, and it repeats nothing.
        let explorer = Explorer::default();
        let error = explorer.open("not-a-real-folder").unwrap_err();
        assert!(!error.contains("not-a-real-folder"), "{error}");
        assert!(error.contains("sample library"), "{error}");
    }

    #[test]
    fn browsing_cannot_leave_the_folders_the_producer_added() {
        // ⛔⛔ **The boundary, and it is a real one rather than tidiness.** The
        // path arrives from the webview. A page that could pass any string
        // would let the plugin enumerate the whole disk — every folder name and
        // every sample name — and hand it back through a channel that leaves
        // the process. Adding a folder grants access to *that* folder.
        let dir = temp("containment");
        let library = dir.join("library");
        let inside = library.join("kicks");
        let outside = dir.join("private");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[library.to_str().unwrap().to_owned()]);

        // Into the library: allowed.
        explorer
            .open(inside.to_str().unwrap())
            .expect("a subfolder of a saved root is browsable");

        // A sibling the producer never added: refused.
        let error = explorer.open(outside.to_str().unwrap()).unwrap_err();
        assert!(error.contains("sample library"), "{error}");

        // ⛔ And the traversal spelling, which a literal `starts_with` on the
        // string would have let straight through.
        let escape = inside.join("..").join("..").join("private");
        assert!(
            explorer.open(escape.to_str().unwrap()).is_err(),
            "`..` must not walk out of the library"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn up_stops_at_a_root_rather_than_walking_into_the_home_directory() {
        let dir = temp("up");
        let library = dir.join("library");
        std::fs::create_dir_all(library.join("kicks")).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[library.to_str().unwrap().to_owned()]);
        explorer.open(library.to_str().unwrap()).unwrap();

        let state = explorer.state();
        assert!(state.parent.is_none(), "a root has no 'up'");
        assert_eq!(state.roots.len(), 1);
        assert_eq!(state.roots[0].name, "library", "named, not a full path");

        // One level in, "up" comes back.
        explorer
            .open(library.join("kicks").to_str().unwrap())
            .unwrap();
        assert!(explorer.state().parent.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_removed_root_takes_the_view_with_it() {
        // Otherwise the library loses the folder and the pane goes on showing
        // its contents, with no root selected and no way back.
        let dir = temp("remove");
        let library = dir.join("library");
        std::fs::create_dir_all(library.join("kicks")).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[library.to_str().unwrap().to_owned()]);
        explorer
            .open(library.join("kicks").to_str().unwrap())
            .unwrap();
        assert!(explorer.state().folder.is_some());

        explorer.remove(library.to_str().unwrap());
        let state = explorer.state();
        assert!(state.roots.is_empty());
        assert!(state.folder.is_none(), "the view must not outlive the root");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_library_folder_that_is_gone_is_kept_rather_than_dropped() {
        // ⚠ An unplugged external drive is the common case. Silently removing
        // the producer's library because they booted without it would be
        // unrecoverable — they would have to remember what was in the list.
        let explorer = Explorer::default();
        explorer.restore(&["/a/drive/that/is/not/plugged/in".into()]);
        assert_eq!(explorer.snapshot().len(), 1);
        assert_eq!(explorer.state().roots.len(), 1);
    }

    #[test]
    fn a_waveform_reduces_to_something_a_panel_can_draw() {
        // ⛔ The audio must not cross the bridge — a four-second sample is
        // megabytes as JSON, serialized on the editor thread inside a DAW.
        // 800 columns is wider than any panel, so nothing drawable is lost.
        let dir = temp("waveform");
        let path = dir.join("tone.wav");
        // One second of a full-scale square at 44.1k, written as 16-bit PCM.
        let frames: Vec<i16> = (0..44_100)
            .map(|i| if (i / 50) % 2 == 0 { 20_000 } else { -20_000 })
            .collect();
        std::fs::write(&path, wav_bytes(&frames)).unwrap();

        // ⚠ The library has to contain it now — the guard is the point.
        let explorer = Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);
        let wave = waveform(&explorer, path.to_str().unwrap()).expect("a real wav decodes");
        assert_eq!(wave.name, "tone.wav");
        assert!(
            wave.peaks.len() <= WAVE_COLUMNS,
            "{} columns is more than the page can use",
            wave.peaks.len()
        );
        assert!(!wave.peaks.is_empty());
        assert!(
            (wave.seconds - 1.0).abs() < 0.05,
            "a second of audio is a second: {}",
            wave.seconds
        );

        // ⚠ Both bounds, not one amplitude. A single peak per column draws a
        // half-waveform that reads as a DC-offset recording.
        let (low, high) = wave
            .peaks
            .iter()
            .fold((0.0f32, 0.0f32), |(l, h), [a, b]| (l.min(*a), h.max(*b)));
        assert!(low < -0.1, "the negative half is drawn too: {low}");
        assert!(high > 0.1, "and the positive: {high}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn something_that_is_not_a_sample_is_refused_before_it_is_decoded() {
        let dir = temp("notaudio");
        let path = dir.join("notes.txt");
        std::fs::write(&path, b"this is not audio").unwrap();

        // ⚠ Refused on the extension, before symphonia is handed an untrusted
        // file at all — the cheapest possible no.
        let explorer = Explorer::default();
        explorer.restore(&[dir.to_str().unwrap().to_owned()]);
        let error = waveform(&explorer, path.to_str().unwrap()).unwrap_err();
        assert!(error.contains("not a sample"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A minimal 16-bit mono PCM WAV, so the test does not need a fixture file.
    fn wav_bytes(frames: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = frames.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend(((36 + data.len()) as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes()); // PCM
        out.extend(1u16.to_le_bytes()); // mono
        out.extend(44_100u32.to_le_bytes());
        out.extend((44_100u32 * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn a_project_file_cannot_plant_a_remote_folder_in_the_library() {
        // ⛔⛔ **The vector `refuse_remote` was written for, arriving through a
        // door it did not cover.** `one_shots` and `sample_folders` ride the
        // same persisted blob; one was guarded and one was not. A shared
        // project carrying a UNC root would be restored on load, and then every
        // `open` and every `state()` poll canonicalises it — an outbound SMB
        // authentication per poll, indefinitely, handing over NetNTLMv2.
        let explorer = Explorer::default();
        explorer.restore(&[
            "\\\\evil.example.com\\share".to_owned(),
            "//evil.example.com/share".to_owned(),
        ]);
        assert!(
            explorer.snapshot().is_empty(),
            "a remote root must not survive a restore: {:?}",
            explorer.snapshot()
        );
    }

    #[test]
    fn reading_midi_applies_the_same_two_guards_as_every_other_path_command() {
        // ⛔⛔ **Asserted together, because the last time this file grew a
        // command it grew with only one of them.** `explorer_drop` applied
        // `refuse_remote` and not containment, and a security review had to
        // find it. A third door — this one — is exactly where that repeats, so
        // both halves are pinned here rather than trusted.
        use engine::pattern::Part;

        let explorer = Explorer::default();

        // The remote guard, and it must fire *before* containment — a UNC
        // string makes the SMB redirector authenticate outward on the first
        // syscall, so refusing afterwards is cold comfort.
        let remote = midi_pattern(
            &explorer,
            "\\\\evil.example.com\\share\\a.mid",
            Part::Melody,
        )
        .unwrap_err();
        assert!(
            !remote.contains("sample library"),
            "a UNC path must be refused by the remote guard first: {remote}"
        );

        // And containment: a real, readable file outside the library is refused
        // in the same words as one that does not exist, so the page cannot use
        // this to map the disk.
        let dir = temp("midi-outside");
        let outside = dir.join("loop.mid");
        std::fs::write(&outside, b"MThd").unwrap();

        let uncontained =
            midi_pattern(&explorer, outside.to_str().unwrap(), Part::Melody).unwrap_err();
        let missing = midi_pattern(
            &explorer,
            dir.join("nope.mid").to_str().unwrap(),
            Part::Melody,
        )
        .unwrap_err();
        assert_eq!(
            uncontained, missing,
            "existing and missing must be indistinguishable outside the library"
        );
    }

    #[test]
    fn a_project_file_cannot_plant_more_roots_than_anyone_has() {
        // `MAX_ENTRIES` bounded a listing; nothing bounded the root count, and
        // every root is canonicalised on every poll.
        let many: Vec<String> = (0..MAX_STORED_ROOTS * 10)
            .map(|i| format!("/root/{i}"))
            .collect();
        let explorer = Explorer::default();
        explorer.restore(&many);
        assert_eq!(explorer.snapshot().len(), MAX_STORED_ROOTS);
    }

    /// ⛔⛔ **Loading a project must never delete folders out of it.**
    ///
    /// Found by a code review, 2026-08-10. The tab strip lowered the *add* bound
    /// to eight and `restore` truncated to the same number — so a project saved
    /// under the old bound of 24 lost the rest on load, and `explorer_state`
    /// wrote the survivors straight back over `session.sample_folders`, making
    /// the loss permanent the next time the host saved. Silently.
    #[test]
    fn a_project_saved_with_more_folders_than_the_tab_strip_keeps_all_of_them() {
        // Twelve, which was legal before the strip and is more than `MAX_ROOTS`.
        let saved: Vec<String> = (0..12).map(|i| format!("/audio/library-{i}")).collect();
        let explorer = Explorer::default();
        explorer.restore(&saved);

        assert_eq!(
            explorer.snapshot(),
            saved,
            "a bound on what may be ADDED is not a licence to delete what is there"
        );
    }

    /// ⛔ **The ninth folder is refused with the rule, not silently dropped.**
    ///
    /// Mike, 2026-08-10: *"if you want to add more, then you have to exit out of
    /// one of them."* The page disables **Add folder** at eight; this is the same
    /// rule where it is actually enforced, so a page that got it wrong cannot
    /// exceed the bound.
    #[test]
    fn a_ninth_folder_is_refused_before_the_dialog_opens() {
        let full: Vec<String> = (0..MAX_ROOTS).map(|i| format!("/root/{i}")).collect();
        let explorer = Explorer::default();
        explorer.restore(&full);

        let error = explorer.pick().expect_err("a ninth folder must be refused");
        assert!(error.contains("close one"), "{error}");
        // ⚠ And no dialog was opened, so the page is not left polling `picking`
        // for something that will never finish.
        assert!(!explorer.state().picking, "no dialog should have started");
    }

    #[test]
    fn opening_a_remote_path_is_refused_before_anything_touches_it() {
        // ⛔ The order is the finding. The containment check refused the
        // *listing*, but `is_dir()` had already run — and on a UNC string that
        // is the SMB redirector resolving the host and authenticating. Refusing
        // afterwards is cold comfort.
        let explorer = Explorer::default();
        let error = explorer.open("\\\\evil.example.com\\share").unwrap_err();
        assert!(
            !error.contains("sample library"),
            "it must be refused by the remote guard, before containment: {error}"
        );
    }

    #[test]
    fn the_three_open_failures_are_indistinguishable() {
        // ⚠ Distinct messages for "does not exist" / "unreadable" / "outside
        // the library" let a page with no other filesystem reach map the whole
        // disk one probe at a time.
        let dir = temp("oracle");
        let outside = dir.join("private");
        std::fs::create_dir_all(&outside).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[dir.join("library").to_str().unwrap().to_owned()]);

        let missing = explorer
            .open(dir.join("nope").to_str().unwrap())
            .unwrap_err();
        let real = explorer.open(outside.to_str().unwrap()).unwrap_err();
        assert_eq!(
            missing.replace("nope", "X"),
            real.replace("private", "X"),
            "a folder that exists and one that does not must read the same"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_waveform_outside_the_library_is_refused() {
        // The module header claims browsing cannot leave the folders the
        // producer added. That claim did not hold for this function: it took a
        // raw path and handed symphonia whatever was there.
        let dir = temp("wave-guard");
        let library = dir.join("library");
        std::fs::create_dir_all(&library).unwrap();
        let outside = dir.join("secret.wav");
        std::fs::write(&outside, wav_bytes(&[0i16; 100])).unwrap();

        let explorer = Explorer::default();
        explorer.restore(&[library.to_str().unwrap().to_owned()]);

        let error = waveform(&explorer, outside.to_str().unwrap()).unwrap_err();
        assert!(error.contains("sample library"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_saved_roots_round_trip() {
        let explorer = Explorer::default();
        // ⚠ Two components deep, which is what a real library folder is — see
        // `is_plausible_library`.
        let saved = vec!["/audio/one".to_owned(), "/audio/two".to_owned()];
        explorer.restore(&saved);
        assert_eq!(explorer.snapshot(), saved, "order is the producer's");
    }

    /// ⛔⛔ **A project file must not be able to point the library at a whole
    /// drive or a user profile.**
    ///
    /// Found by a security review, 2026-08-10. `sample_folders` rides the
    /// `#[persist]` blob, so a shared `.als` or type-beat starter can carry
    /// `"sampleFolders": ["C:\\Users"]` — a local path `refuse_remote` passes.
    /// Because `contains` is a prefix test, installing it satisfies **every**
    /// containment check in the plugin for that whole tree: the page could
    /// enumerate it, read `.mid` content back, decode any audio in it, and
    /// star-then-reveal any file — the one command that launches a process.
    /// ⚠ **Posix spellings, and the Windows ones are a separate test below.** A
    /// backslash is an ordinary filename character on Linux and macOS, so
    /// `C:\Users\mike\Samples` is **one** component there — `refuse_remote`'s own
    /// header records the same trap ("a project file is portable and `Path` is
    /// not"). CI caught this on two runners.
    #[test]
    fn a_project_file_cannot_point_the_library_at_a_drive_or_a_profile() {
        let explorer = Explorer::default();
        explorer.restore(&[
            "/".to_owned(),
            "/home".to_owned(),
            // ...and something that genuinely is a library, to prove the guard
            // is a filter rather than a refusal of everything.
            "/home/mike/Samples".to_owned(),
        ]);

        assert_eq!(
            explorer.snapshot(),
            vec!["/home/mike/Samples".to_owned()],
            "only a folder deep enough to be somebody's library survives"
        );
    }

    /// The same rule on the platform whose paths this actually protects.
    #[test]
    #[cfg(windows)]
    fn a_windows_drive_or_profile_root_is_refused_too() {
        let explorer = Explorer::default();
        explorer.restore(&[
            "C:\\".to_owned(),
            "C:\\Users".to_owned(),
            "C:\\Users\\mike\\Samples".to_owned(),
        ]);

        assert_eq!(
            explorer.snapshot(),
            vec!["C:\\Users\\mike\\Samples".to_owned()],
        );
    }

    #[test]
    fn the_extension_list_matches_what_the_loader_accepts() {
        // ⛔ A file the explorer offers and the loader refuses is a row that
        // does nothing when dropped, with nothing saying why. These two lists
        // are the same list in two places, so this is what holds them together.
        for extension in AUDIO {
            assert!(
                is_audio(Path::new(&format!("a.{extension}"))),
                "{extension} must be listed"
            );
        }
        assert!(!is_audio(Path::new("a.txt")));
        assert!(!is_audio(Path::new("a")));
    }
}
