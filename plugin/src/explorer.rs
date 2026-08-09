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

use serde::Serialize;

/// The audio extensions [`crate::audio::import`] can actually decode.
///
/// ⚠ **The same list the Open dialog filters on**, and it has to stay that way:
/// a file the explorer offers and the loader refuses is a row that does nothing
/// when you drop it, with no way for the producer to tell why.
const AUDIO: &[&str] = &["wav", "aif", "aiff", "flac", "mp3", "m4a", "ogg", "oga"];

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
/// poll on the thread the host draws its window from. Two dozen is past any
/// real library.
const MAX_ROOTS: usize = 24;

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
        let kept: Vec<PathBuf> = paths
            .iter()
            .map(PathBuf::from)
            .filter(|path| crate::oneshot::refuse_remote(path).is_ok())
            .take(MAX_ROOTS)
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
                    if !roots.contains(&dir) {
                        roots.push(dir.clone());
                    }
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

    /// Is `path` inside a folder the producer added?
    ///
    /// ⚠ **The same test `open` applies, exposed so the file-reading commands
    /// can apply it too.** `waveform` and `preview_load` both took a raw path
    /// from the page; the explorer must only describe and audition what it
    /// would list.
    pub fn contains(&self, path: &Path) -> bool {
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
                });
            } else if is_audio(&path) {
                files.push(Entry {
                    name: name.to_owned(),
                    path: text.to_owned(),
                    is_dir: false,
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
    fn lists_audio_and_folders_and_nothing_else() {
        let dir = temp("listing");
        std::fs::write(dir.join("kick.wav"), b"x").unwrap();
        std::fs::write(dir.join("snare.WAV"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("cover.png"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("Kicks")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();

        let state = list(&dir);
        let names: Vec<&str> = state.entries.iter().map(|e| e.name.as_str()).collect();

        // Folders first, then audio, each by name. The extension check is
        // case-insensitive because a sample library is full of `.WAV`.
        assert_eq!(names, vec!["Kicks", "kick.wav", "snare.WAV"]);
        assert!(!state.truncated);
        assert!(state.entries[0].is_dir, "the folder sorts first");
        assert!(!state.entries[1].is_dir, "then the audio files");

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
    fn a_project_file_cannot_plant_more_roots_than_anyone_has() {
        // `MAX_ENTRIES` bounded a listing; nothing bounded the root count, and
        // every root is canonicalised on every poll.
        let many: Vec<String> = (0..MAX_ROOTS * 10).map(|i| format!("/root/{i}")).collect();
        let explorer = Explorer::default();
        explorer.restore(&many);
        assert_eq!(explorer.snapshot().len(), MAX_ROOTS);
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
        let saved = vec!["/one".to_owned(), "/two".to_owned()];
        explorer.restore(&saved);
        assert_eq!(explorer.snapshot(), saved, "order is the producer's");
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
