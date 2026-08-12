//! Getting a song out of the plugin as a file (TASK-073).
//!
//! ## ⛔ Why this is a thread and a mailbox rather than one blocking call
//!
//! A native Save As dialog is **modal and blocking**. The bridge answers an
//! HTTP call from the webview over the custom protocol, and that handler runs on
//! a frame the page is waiting on — `editor.rs` already documents that it sits
//! in an `extern "C"` frame where a panic cannot even unwind. Opening a modal
//! dialog there stalls the webview for as long as the producer is browsing for
//! a folder, and in a host that is the DAW's own editor thread: no crash, no
//! error, and no way out but killing the DAW. That is the exact failure class
//! this project has been bitten by twice and writes down in capitals.
//!
//! So the command **starts** an export and returns immediately. A detached
//! thread owns the dialog and the write, and drops its outcome into a one-slot
//! mailbox the page reads on its next poll. The page shows "Choose a
//! folder…" while that is happening, which is also the honest readout — the
//! dialog really is what it is waiting for.
//!
//! ## One at a time
//!
//! A second export while one is open is refused rather than queued. Two native
//! dialogs from one plugin is a window a producer cannot explain, and "export
//! twice quickly" is not a thing anybody means to do.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use engine::pattern::{Pattern, Song, PART_ORDER};
use serde::Serialize;

/// How an export ended, for the page to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Nothing has been exported since the last time the page looked.
    #[default]
    Idle,
    /// A dialog is open, or the bytes are being written.
    Running,
    /// Written. The path is shown to the producer, because a file they cannot
    /// find is a file they did not get.
    Done {
        path: String,
    },
    /// The producer closed the dialog. **Not an error** — it is the ordinary
    /// way out of a Save As, and reporting it as a failure would train people
    /// to ignore the one message that matters.
    Cancelled,
    Failed {
        reason: String,
    },
}

/// One in-flight export and its outcome, held **per plugin instance**.
///
/// ⛔ **Not a process global, and the difference is a real failure.** A DAW can
/// load this plugin on twenty tracks in one process, and `take_status` is
/// *destructive* — so a shared slot means instance B's 400 ms poll steals
/// instance A's `Done { path }`. A's chip then reads "exporting…" until the
/// five-minute timeout while B announces a file it never wrote, and the
/// one-at-a-time refusal fires across instances with no dialog visible in the
/// one that was refused. Everything else global in this crate — the dataset,
/// the kit, the licence flag — is genuinely process-wide and effectively
/// immutable; this is per-user-action state and belongs beside the session.
#[derive(Debug, Default)]
pub struct Exports {
    status: Arc<Mutex<Status>>,
}

impl Exports {
    /// Start writing `song` to a file the producer picks.
    ///
    /// Returns as soon as the thread is running. `suggested` is the file name
    /// the dialog opens with — the artist and the seed, so a folder full of
    /// exports is readable without opening any of them.
    pub fn start_song_midi(&self, song: Song, suggested: &str) -> Result<(), String> {
        let name = sanitize(suggested);
        let claimed = self.claim()?;
        // ⛔ **Encoded inside the job, after the dialog returns, not before it
        // opens.** Doing it on the caller's thread put a whole-song SMF encode
        // on the frame the page is waiting on — the very thread this module
        // exists to keep free — and threw all of it away on the cancel that the
        // header calls the ordinary way out.
        run_dialog(claimed, move || {
            let Some(path) = rfd::FileDialog::new()
                .set_file_name(&name)
                .add_filter("MIDI file", &["mid"])
                .save_file()
            else {
                return Status::Cancelled;
            };
            write(&with_extension(path), &engine::midi::song_to_smf(&song))
        });
        Ok(())
    }

    /// Write one file per part into a folder the producer picks (TASK-069).
    ///
    /// ## MIDI stems for a whole song; audio is per pattern
    ///
    /// ⚠ **This comment used to say the audio half was impossible, and that
    /// stopped being true in TASK-131A.** The reason it gave was real and
    /// measured — `audio::Kit::pad_for` answered `None` for Melody, Counter,
    /// Bass and Chords, so rendering them would have written four silent files
    /// and called them stems. The shipped kit now covers every generated lane.
    ///
    /// What is still true is that a *song* is minutes long and rendering one to
    /// audio is a different job from rendering a four-bar loop: it needs
    /// progress the producer can watch and a cancel they can press. So a song
    /// still writes one **type-0 `.mid` per part**, and
    /// [`Self::start_pattern_stems`] is where the audio half landed
    /// (TASK-131F).
    ///
    /// ## What "aligned to bar 1, identical lengths" means here
    ///
    /// Every file is the whole song's timeline with one part in it, so they line
    /// up by construction: dropping all five onto a DAW at bar 1 reassembles the
    /// arrangement. That is the property the wav half would have needed too, and
    /// flattening is what gives it — see `Song::flatten_parts`.
    pub fn start_song_stems(&self, song: Song, folder_name: &str) -> Result<(), String> {
        // ⛔ **The emptiness check runs before the dialog, and it is the one
        // thing that must.** A folder of nothing is a successful-looking export
        // the producer then has to work out was always empty — and refusing
        // after they have browsed for a folder is worse than refusing before.
        // It is a note count, not an encode.
        // ⚠ Flattened once, here, and reused below — the first cut walked the
        // whole arrangement twice, once to ask whether anything played and
        // again to write it.
        let parts = song_stem_patterns(&song);
        if parts.is_empty() {
            return Err("this song plays nothing, so there are no stems to write".to_owned());
        }

        let stem_dir = sanitize(folder_name);
        let claimed = self.claim()?;
        run_dialog(claimed, move || {
            let Some(root) = rfd::FileDialog::new().pick_folder() else {
                return Status::Cancelled;
            };
            let files: Vec<(String, Vec<u8>)> = parts
                .iter()
                .map(|flat| {
                    // ⛔ The *engine's* name, so the file on disk and the track
                    // inside it cannot disagree. A second table here put
                    // `FMM Melody.mid` on disk with `trap — Drums` in it.
                    //
                    // ⚠ **Deliberately not [`stem_name`], which is what the
                    // drag-out uses.** These land in a folder already named for
                    // the artist and the seed, so repeating both in every file
                    // name is noise; a dragged file arrives with no folder
                    // around it and has to say what it is on its own. The
                    // *patterns* are shared — see [`song_stem_patterns`] — and
                    // only the label differs.
                    (
                        format!("{}.mid", engine::midi::part_track_name(flat.part)),
                        engine::midi::pattern_to_smf(flat),
                    )
                })
                .collect();
            write_stems(&root.join(&stem_dir), &files)
        });
        Ok(())
    }

    /// Write the *current pattern's* parts into a folder, as MIDI or as audio
    /// (TASK-131F).
    ///
    /// Mike, 2026-08-05: *"i also need to be able to export the drums by
    /// themselves as midi or audio stems."* [`Self::start_song_stems`] does this
    /// for a whole arrangement; this is the four- or eight-bar loop on screen,
    /// which is what a producer actually has in front of them most of the time.
    ///
    /// ⛔ **The audio half was blocked until TASK-131A/131B and the block was
    /// measured, not assumed.** `Kit::pad_for` answered `None` for the four
    /// melodic parts, so rendering them would have written silent files and
    /// called them stems. [`crate::audio::render`] carries the full note.
    ///
    /// ⚠ **A part that renders nothing gets no file.** An empty stem is one a
    /// producer imports, hears nothing from, and has to work out was always
    /// empty — the same rule `start_song_stems` already follows for notes.
    /// ## ⛔ Per lane, not only per part — Mike, 2026-08-05
    ///
    /// *"i want to be able to drag just one drum lane out just like drum monkey,
    /// where i can just drag the hihats out to the daw or just the snares, etc.
    /// with either audio or midi."*
    ///
    /// `split_lanes` turns one drum pattern into one file per **lane** — kick,
    /// snare, closed hat — rather than a single `FMM Drums` file holding all of
    /// them. That is what makes a hat pattern something a producer can drop onto
    /// its own track, and it is the whole reason to have this over the existing
    /// per-part export.
    ///
    /// ⚠ **Export, not drag.** An HTML5 drag inside a webview is not an OS file
    /// drag; it needs a native drag source per platform, which is TASK-063C
    /// blocked on FMM-S03. Writing the files is the half that can ship now, and
    /// a producer can drag them from the folder.
    pub fn start_pattern_stems(
        &self,
        patterns: Vec<Pattern>,
        folder_name: &str,
        audio: bool,
        split_lanes: bool,
        kit: Option<Arc<crate::audio::kit::Kit>>,
    ) -> Result<(), String> {
        // ⚠ **The lane split is [`stem_files`]'s, not this function's.** It used
        // to happen here, which meant "split into lanes" and "name the file
        // after the lane" lived in two places a caller had to get right
        // together — and the drag-out (TASK-063C) is a second caller that would
        // have had to get them right again. Splitting cannot change the total
        // note count, so the emptiness refusal below is unaffected by the move.
        if patterns.iter().all(|p| p.note_count() == 0) {
            return Err("these parts play nothing, so there are no stems to write".to_owned());
        }
        // ⛔ **The kit that is PLAYING, handed in by the caller — not
        // `preview_kit()`.** This resolved the shipped base itself, so a
        // producer who had assigned their own snare heard it in the preview and
        // got the stock one in the exported wav: the plugin telling them one
        // thing and writing another. The caller has `Shared` and therefore the
        // one-shots; this does not, which is exactly why it must be passed.
        //
        // ⚠ Still resolved before the dialog opens — building it decodes and
        // allocates, and doing that inside the job would put the work behind a
        // modal window for no reason.
        let kit = if audio {
            match kit {
                Some(kit) => Some(kit),
                None => {
                    return Err(
                        "the preview kit did not load, so there is no audio to render".to_owned(),
                    )
                }
            }
        } else {
            None
        };

        // ⛔ **Before the folder dialog, not after it.** A refusal a producer
        // meets *after* choosing where to put the files reads as the export
        // having failed; met here it is an answer to what they just asked for.
        crate::audio::render::refuse_if_too_long(&patterns, kit.is_some())?;

        let stem_dir = sanitize(folder_name);
        let claimed = self.claim()?;
        run_dialog(claimed, move || {
            let Some(root) = rfd::FileDialog::new().pick_folder() else {
                return Status::Cancelled;
            };
            let files = stem_files(
                &patterns,
                if split_lanes {
                    Cut::EveryLane
                } else {
                    Cut::Parts
                },
                kit.as_deref(),
            );

            if files.is_empty() {
                return Status::Failed {
                    reason: "none of these parts has a sound to render".to_owned(),
                };
            }
            write_stems(&root.join(&stem_dir), &files)
        });
        Ok(())
    }

    /// Read the outcome, and clear it if there is one.
    ///
    /// ⛔ **Taken rather than merely read.** The page polls this, so a terminal
    /// status left in place would re-announce the same successful export on
    /// every tick — a toast that will not go away. `Running` stays, because it
    /// is not an outcome.
    pub fn take_status(&self) -> Status {
        let Ok(mut slot) = self.status.lock() else {
            return Status::Failed {
                reason: "the export state is unusable".to_owned(),
            };
        };
        match &*slot {
            Status::Running => Status::Running,
            other => {
                let taken = other.clone();
                *slot = Status::Idle;
                taken
            }
        }
    }

    /// Take the one slot, or say why it cannot be taken.
    ///
    /// ⛔ Written once because the "refuse a second export" rule and the
    /// "always publish a terminal status" rule are a pair: a copy that claims
    /// and forgets to publish leaves the chip reading "exporting…" for the rest
    /// of the session, which is the failure `EXPORT_TIMEOUT_MS` on the page only
    /// papers over. A third exporter — the audio stems this module's own note
    /// promises — gets both for free.
    fn claim(&self) -> Result<Claim, String> {
        let mut slot = self
            .status
            .lock()
            .map_err(|_| "the export state is unusable".to_owned())?;
        if *slot == Status::Running {
            return Err("an export is already open — finish that one first".to_owned());
        }
        *slot = Status::Running;
        Ok(Claim(Arc::clone(&self.status)))
    }
}

/// Run `job` on its own thread and publish whatever it returns.
///
/// See the module header: the dialog is modal, and the thread the bridge answers
/// on is the one a DAW draws its editor from.
fn run_dialog(claim: Claim, job: impl FnOnce() -> Status + Send + 'static) {
    std::thread::spawn(move || {
        let status = job();
        if let Ok(mut slot) = claim.0.lock() {
            *slot = status;
        }
    });
}

/// Proof that the slot was taken, and the handle for putting an outcome back.
///
/// A value rather than a bare `Arc` so `run_dialog` cannot be called without
/// having claimed first — the type is what makes the pair inseparable.
struct Claim(Arc<Mutex<Status>>);

/// The files a set of patterns becomes: a name and its bytes, one per part or
/// one per lane.
///
/// ⛔ **The one place that turns patterns into named bytes**, and it is shared
/// rather than copied because the drag-out (TASK-063C) needs exactly the same
/// answer as the export does. A producer who drags a hi-hat loop into Ableton
/// and then exports the same loop to a folder must get the same file with the
/// same name in it; two implementations of that is how they stop agreeing.
///
/// `kit` is `Some` for audio and `None` for MIDI. ⚠ It is the kit that is
/// **playing**, resolved by the caller — see [`Exports::start_pattern_stems`]
/// for why passing it is not optional.
pub(crate) fn stem_files(
    patterns: &[Pattern],
    cut: Cut,
    kit: Option<&crate::audio::kit::Kit>,
) -> Vec<(String, Vec<u8>)> {
    // Nothing to report to and nothing that can call it off — the dialog-driven
    // exports already have a thread of their own and a producer watching a
    // folder picker.
    stem_files_with(patterns, cut, kit, &mut |_, _| true)
}

/// [`stem_files`], reporting as it goes and stopping when asked.
///
/// ⛔⛔ **This is what makes a whole arrangement renderable to audio at all.**
/// `editor.rs` used to refuse that request outright — *"a whole arrangement
/// drags out as MIDI — render audio stems from Export instead"* — and the reason
/// it gave was true: a record is minutes long, so the render is seconds of work
/// with the producer holding a mouse button, and there was no way to say how far
/// along it was or to stop it. `on_step` is both of those.
///
/// `on_step(done, total)` is called after each file is encoded and returns
/// `false` to abandon the rest. ⚠ **What has already been encoded is returned
/// rather than thrown away** — the caller is what decides whether a partial set
/// is worth keeping, and for the drag it never is; it discards the folder. A
/// function that returned nothing would make "stopped early" and "produced
/// nothing" the same answer.
pub(crate) fn stem_files_with(
    patterns: &[Pattern],
    cut: Cut,
    kit: Option<&crate::audio::kit::Kit>,
    on_step: &mut dyn FnMut(usize, usize) -> bool,
) -> Vec<(String, Vec<u8>)> {
    let split;
    let patterns = match cut {
        Cut::Parts => patterns,
        Cut::EveryLane => {
            split = patterns.iter().flat_map(per_lane).collect::<Vec<_>>();
            &split[..]
        }
        // ⛔⛔ **MIDI only. Audio falls back to `EveryLane`, and that is a
        // correctness-shaped performance decision rather than a shortcut.**
        // Offsetting lane *i* means its clip has to be `i + 1` times as long or
        // `within_clip` drops the notes — which for MIDI is a few more bytes,
        // and for audio is a rendered buffer that grows with the *square* of
        // the lane count. Eight lanes of a four-bar loop is 8+16+…+64 bars of
        // stereo f32, seven eighths of it silence, allocated and soft-limited
        // and peak-scanned and encoded, inside somebody's DAW, while they hold
        // the mouse button down.
        //
        // ⚠ Mike's request was about MIDI in his own words — *"it has to be
        // separate midi clips one after the other"* — and a producer dragging
        // audio stems wants them stacked so the kit plays as a kit. So the
        // sequential layout is offered where it was asked for and where it is
        // cheap, and audio gets the layout it wants anyway.
        Cut::EveryLaneInSequence if kit.is_none() => {
            split = patterns.iter().flat_map(in_sequence).collect::<Vec<_>>();
            &split[..]
        }
        Cut::EveryLaneInSequence => {
            split = patterns.iter().flat_map(per_lane).collect::<Vec<_>>();
            &split[..]
        }
        Cut::OneLane(lane) => {
            split = patterns
                .iter()
                .flat_map(per_lane)
                .filter(|one| one.lanes.first().is_some_and(|track| track.lane == lane))
                .collect::<Vec<_>>();
            &split[..]
        }
    };
    let by_lane = !matches!(cut, Cut::Parts);
    let mut taken = std::collections::BTreeSet::new();
    // ⚠ **Counted over what will actually be encoded**, not over `patterns` —
    // the silent ones are filtered out below, and a total that included them
    // would leave the bar short of 100% on every render that had any.
    let total = patterns.iter().filter(|p| p.note_count() > 0).count();
    let mut done = 0usize;
    patterns
        .iter()
        // A part nothing plays gets no file — the rule `start_song_stems`
        // already follows, for the same reason: an empty stem is one a producer
        // imports, hears nothing from, and has to work out was always empty.
        .filter(|pattern| pattern.note_count() > 0)
        // ⛔ **`map_while`, so a refusal stops the iterator rather than skipping
        // one file.** With `filter_map` a cancelled render would go on to encode
        // every remaining part and simply discard them — which is the CPU this
        // whole mechanism exists to stop spending.
        .map_while(|pattern| {
            // ⛔ **The name is reserved only once a file really exists.** The
            // reservation used to run ahead of the render, so a pattern whose
            // lane the kit cannot play — `to_stereo` answers `None` and nothing
            // is written — still consumed its name, and the next identical
            // pattern came out as `… (2).wav` with no `(1)` anywhere. That reads
            // as an overwrite that never happened.
            let stem = stem_name(pattern, by_lane);
            let file = match kit {
                Some(kit) => crate::audio::render::to_stereo(pattern, kit).map(|samples| {
                    (
                        format!("{}.wav", distinct(&mut taken, stem)),
                        // ⛔ The same pattern the samples were rendered from, so
                        // the tempo in the file describes the audio in it.
                        crate::audio::render::to_wav(&samples, pattern),
                    )
                }),
                None => Some((
                    format!("{}.mid", distinct(&mut taken, stem)),
                    engine::midi::pattern_to_smf(pattern),
                )),
            };
            // ⚠ **Reported even for a lane the kit could not play.** That part
            // is finished as far as the producer is concerned — the work of
            // deciding was done — and a bar that stalled on the silent ones
            // would read as the render having hung.
            done += 1;
            if !on_step(done, total) {
                return None;
            }
            // ⚠ `Some(None)` keeps the iterator alive past a pattern that
            // produced no file; `None` above is the stop. `map_while` reads the
            // outer layer, and `flatten` drops the inner.
            Some(file)
        })
        .flatten()
        .collect()
}

/// `name`, or `name (2)` if an earlier file in this batch already took it.
///
/// ⚠ The extension is added by the caller, so this numbers the *stem* — which
/// is also the readable place for it: `trap - Drums - 140 BPM - C Minor (2).mid`
/// rather than `… .mid (2)`.
///
/// ⛔ **The search is unbounded, and the bound it used to have was reasoning
/// about the wrong quantity.** `check_patterns` caps how many *patterns* may
/// arrive, but `Cut::EveryLane` multiplies that by a lane count nothing bounds
/// — and `lanes` is a `Vec` that may repeat the same `Lane`. A project file
/// with five patterns of twenty identical lanes yields a hundred stems all
/// naming themselves the same thing; a cap of 64 uniquified the first 64 and
/// then handed the bare name back for the rest, so they overwrote each other
/// and the drop target received the same file thirty-six times. That is the
/// exact failure the suffix exists to prevent, one step past the cap.
fn distinct(taken: &mut std::collections::BTreeSet<String>, name: String) -> String {
    if taken.insert(name.clone()) {
        return name;
    }
    // Terminates because `taken` grows by one on every successful insert, so
    // some `nth` is always free.
    for nth in 2.. {
        let candidate = format!("{name} ({nth})");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("a free suffix always exists")
}

/// How a set of patterns is carved into files.
///
/// ⛔ **A cut is both halves at once — which patterns become files *and* what
/// those files are called.** They were separable when only the export called
/// [`stem_files`], and the drag-out is what made the pair worth naming: the
/// page used to slice a single lane out for itself and pass "split by lane",
/// which meant "what a lane stem is" was implemented on both sides of the
/// bridge. It is one thing here now, and the page names a lane instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cut {
    /// One file per pattern, named for its part.
    Parts,
    /// One file per lane, each named for its lane.
    EveryLane,
    /// One file per lane, each starting where the last one ended (2026-08-06).
    ///
    /// ⛔⛔ **Mike:** *"it has to be separate midi clips one after the other, but
    /// on the same line unless you hold ctrl or press and hold ctrl during the
    /// dragging then it stacks them."* So this is [`Self::EveryLane`] with the
    /// notes of lane *i* pushed `i` clip-lengths later, and the clip grown to
    /// cover them.
    ///
    /// ⚠ **What this can and cannot promise.** The *time* relationship is baked
    /// into the files, so it holds in every host: drop them anywhere and the
    /// kick plays, then the snare, then the hats. Which **track** each one lands
    /// on is the host's answer to a multi-file drop and cannot be set from the
    /// drag source — see `drag/windows.rs`, where the modifier is read.
    EveryLaneInSequence,
    /// Just this lane — *"i can just drag the hihats out"*.
    OneLane(engine::pattern::Lane),
}

/// The parts of an arrangement that become files, one flattened pattern each.
///
/// ⛔ **Shared by the song export and the song drag-out, because they had this
/// loop twice and it is the loop that decides what a producer gets.** Every
/// pattern is the whole timeline with one part in it, so dropping them all at
/// bar 1 reassembles the arrangement — that property is what makes the split
/// safe to offer, and it belongs in one place.
///
/// ⚠ **A part nothing plays gets no pattern**, so neither caller writes a file
/// a producer imports and hears nothing from.
pub(crate) fn song_stem_patterns(song: &Song) -> Vec<Pattern> {
    PART_ORDER
        .into_iter()
        .map(|part| song.flatten_parts(Some(&[part])))
        .filter(|flat| flat.note_count() > 0)
        .collect()
}

/// What a stem is called on disk (TASK-131F).
///
/// Mike, 2026-08-05: *"it needs to be labeled when you drag it out like this:
/// `Artist/Genre - Snares - 140 BPM - C# Minor`"*. A folder of those is readable
/// without opening any of them, and dropped onto a DAW track the clip carries
/// its own tempo and key — which is exactly what a producer needs to know before
/// deciding whether it fits.
///
/// ⛔ **The lane or part name comes from the engine, never a table here.** A
/// second naming table once put `FMM Melody.mid` on disk with `trap — Drums`
/// inside it.
///
/// ⚠ Sanitized by the caller, not here: `sanitize` also has to run on the folder
/// name, and doing it in one place is what stops the two drifting.
fn stem_name(pattern: &Pattern, split_lanes: bool) -> String {
    let what = match (split_lanes, pattern.lanes.first()) {
        // ⚠ A lane split names the file after the *lane* — `FMM Drums` five
        // times over is five files a producer cannot tell apart, which defeats
        // the point of splitting.
        (true, Some(track)) => format!("{:?}", track.lane),
        _ => engine::midi::part_track_name(pattern.part)
            .trim_start_matches("FMM ")
            .to_owned(),
    };
    // Rounded, because a tempo the host reported as 139.9999 is 140 to a human
    // and a file called `139.9999 BPM` reads as broken.
    let bpm = pattern.bpm.round() as i64;
    let key = engine::theory::key_label(pattern.key_root, pattern.scale);
    safe_file_name(&format!(
        "{} - {what} - {bpm} BPM - {key}",
        pattern.artist_id
    ))
}

/// A name that is safe as a file name and still readable as one.
///
/// ⛔ **Not [`sanitize`], and the difference is the whole point.** That one
/// collapses everything but `[A-Za-z0-9-_.]` into dashes, which is right for a
/// *suggested* name in a Save As box and would turn
/// `trap - Snare - 140 BPM - C# Minor` into `trap-Snare-140-BPM-C-Minor`. Mike
/// asked for the readable form, so spaces and `#` survive here.
///
/// ⚠ **`artist_id` still arrives as JSON from the webview**, so this is a trust
/// boundary exactly as `sanitize` is: path separators go, and `..` goes with
/// them, or a crafted id writes outside the folder the producer picked. The
/// Windows-reserved set (`:*?"<>|`) goes too — on Linux those are legal in a
/// name and would produce a file that cannot be copied to a Windows machine.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            // Control characters are legal in a POSIX name and are how a file
            // name hides what it really is.
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.replace("..", "-");
    let trimmed = cleaned.trim().trim_matches(['-', '.']).trim();
    if trimmed.is_empty() {
        "stem".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// One pattern per lane, each carrying only that lane's notes.
///
/// ⛔ **Everything else about the pattern is copied unchanged** — meter, tempo,
/// bars, ppq — because the files have to line up when a producer drops them all
/// onto a DAW at bar 1. That is the same property `start_song_stems` documents
/// for the song case, and it is the whole reason the split is safe to offer.
///
/// ⚠ The clone keeps `part`, so a drum lane's file still says it came from the
/// drum generator; only the *name* is the lane's. Rewriting `part` would make
/// `pattern_to_smf` write a track name that disagrees with the file name, which
/// is the exact bug the naming comment above records.
///
/// ⚠ **The empty base is cloned, not the whole pattern.** `..pattern.clone()`
/// per lane deep-copies every lane's notes and then throws all but one away —
/// eight lanes' worth allocated and dropped to produce one, eight times over.
/// It never mattered while this ran once behind a Save As dialog; the drag-out
/// runs it on a gesture.
fn per_lane(pattern: &Pattern) -> Vec<Pattern> {
    let base = Pattern {
        lanes: Vec::new(),
        ..pattern.clone()
    };
    pattern
        .lanes
        .iter()
        .filter(|track| !track.notes.is_empty())
        .map(|track| Pattern {
            lanes: vec![track.clone()],
            ..base.clone()
        })
        .collect()
}

/// [`per_lane`], with each lane pushed a clip-length later than the one before.
///
/// ⛔ **The offset is inside the file, which is the only place it can survive
/// the trip.** A drag source hands over paths; where the drop target puts them
/// is its own decision. Baking the time in means "kick, then snare, then hats"
/// holds whether the host stacks them on one track or spreads them over eight.
///
/// ⚠ **`bars` grows with the offset or the notes fall outside the clip.**
/// `within_clip` is honoured by the MIDI writer and by the audio renderer, and
/// this file's own history has the matching failure written down: a boundary
/// that only moves marks on screen silently drops what is past it. The eighth
/// lane of a four-bar loop starts at bar 29, so its clip has to be 32 bars long.
///
/// ⚠ Lanes are ordered as the pattern holds them, which is the order
/// [`per_lane`] already produces — so the two cuts disagree about placement and
/// about nothing else.
fn in_sequence(pattern: &Pattern) -> Vec<Pattern> {
    let span = pattern
        .ticks_per_bar()
        .saturating_mul(u32::from(pattern.bars));
    per_lane(pattern)
        .into_iter()
        .enumerate()
        .map(|(index, mut one)| {
            let shift = span.saturating_mul(index as u32);
            for track in &mut one.lanes {
                for note in &mut track.notes {
                    note.start_tick = note.start_tick.saturating_add(shift);
                }
            }
            // ⛔⛔ **The clip's own boundary moves with its notes, and forgetting
            // it emptied seven files out of eight.** `pattern_to_smf` filters
            // every note through `Pattern::within_clip`, which is bounded by
            // `clip_region` and not by `bars` — so a producer who had trimmed
            // the clip with the piano roll's markers and then dragged "All
            // Tracks" got lane 0 intact and every later lane written, named for
            // its lane, and containing nothing at all. Growing `bars` below is
            // not enough on its own; these are two different boundaries and both
            // are read.
            //
            // ⚠ `loop_region` is deliberately left alone: it is what the
            // transport repeats, and nothing that writes a file consults it.
            if let Some(region) = &mut one.clip_region {
                region.from_tick = region.from_tick.saturating_add(shift);
                region.to_tick = region.to_tick.saturating_add(shift);
            }
            // ⚠ Saturating rather than wrapping: `bars` is a `u16`, and a
            // pathological pattern must give a clip that is merely long rather
            // than one that wrapped around to nothing.
            one.bars = pattern
                .bars
                .saturating_mul(u16::try_from(index + 1).unwrap_or(u16::MAX));
            one
        })
        .collect()
}

fn write(path: &Path, bytes: &[u8]) -> Status {
    match std::fs::write(path, bytes) {
        Ok(()) => Status::Done {
            path: path.display().to_string(),
        },
        Err(error) => Status::Failed {
            reason: format!("could not write {}: {error}", path.display()),
        },
    }
}

/// ⚠ **Into a sub-folder of the one that was picked, not into it directly.**
/// Five files dropped into somebody's Desktop is a mess they have to clean up
/// by hand, and it makes "which of these go together" unanswerable once a
/// second song is exported.
fn write_stems(dir: &Path, files: &[(String, Vec<u8>)]) -> Status {
    match spill(dir, files) {
        Ok(_) => Status::Done {
            path: dir.display().to_string(),
        },
        Err(reason) => Status::Failed { reason },
    }
}

/// Write every file into `dir`, and answer with where each one landed.
///
/// ⛔ **Shared with the drag-out (TASK-063C), which needs the paths rather than
/// a `Status`.** The two had the same body and the same two error strings; the
/// only difference was what they returned, so the split is at the return rather
/// than at the loop.
pub(crate) fn spill(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    files
        .iter()
        .map(|(name, bytes)| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).map_err(|e| format!("could not write {name}: {e}"))?;
            Ok(path)
        })
        .collect()
}

/// Make sure the file really is a `.mid`.
///
/// ⚠ Every platform's dialog *offers* the extension from the filter and none of
/// them guarantees it: a producer who types `verse-idea` gets a file with no
/// extension, which a DAW's import browser then hides. Added only when it is
/// missing, so `beat.mid` does not become `beat.mid.mid`.
fn with_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mid"))
    {
        return path;
    }
    path.with_extension("mid")
}

/// A suggested name that cannot be a path.
///
/// ⛔ The name comes from the page — the artist id and the seed — and a `Song`
/// arrives at the bridge as JSON from the webview, so `artist_id` is whatever
/// that JSON said. Without this, `../../..` in an artist id would move the
/// dialog's starting directory, and a name carrying a separator is refused
/// outright by some platform dialogs and silently truncated by others.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // `..` survives the filter above because `.` is legal in a file name — it
    // is the one sequence that means something to a path rather than to a name.
    let cleaned = cleaned.replace("..", "-");

    // ⚠ Runs are collapsed, and it is not only tidiness: each replaced
    // character leaves its own dash, so `a/../../b` came out as `a-----b`. A
    // suggested name is the first thing a producer reads in the dialog, and one
    // that looks corrupted reads as the export having gone wrong already.
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }

    let trimmed = out.trim_matches(['-', '.']).to_owned();
    if trimmed.is_empty() {
        "song.mid".to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_muted_or_soloed_lane_still_exports_every_note() {
        // ⛔ **TASK-043's rule, stated as bytes.** Mute and solo are *view and
        // playback* state: the notes have already gone to the host's track by
        // the time the sampler runs, so silencing a lane must never change what
        // is written to a file. `Shared::lane_muted` has exactly one reader —
        // `render_preview` — and this is what keeps it that way: if the mask
        // ever reached the writer, the two byte strings below would differ.
        use engine::pattern::{Lane, LaneTrack, Note, Part, Pattern, Scale, PPQ};

        let note = |start_tick, pitch| Note {
            start_tick,
            len_ticks: 120,
            pitch,
            vel: 100,
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
            reversed: false,
        };
        let pattern = Pattern {
            id: "muted-export".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 1,
            song_seed: 1,
            bars: 1,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            ppq: PPQ,
            lanes: vec![
                LaneTrack {
                    lane: Lane::Kick,
                    notes: vec![note(0, 36), note(PPQ, 36)],
                },
                LaneTrack {
                    lane: Lane::Snare,
                    notes: vec![note(PPQ, 38)],
                },
            ],
            mood: None,
            loop_region: None,
            clip_region: None,
        };

        let shared = crate::shared::Shared::default();
        let clean = stem_files(std::slice::from_ref(&pattern), Cut::EveryLane, None);

        shared.set_lane_audio(&[Lane::Kick], &[Lane::Snare]);
        assert!(shared.lane_muted(Lane::Kick), "the fixture must be muting");
        let silenced = stem_files(std::slice::from_ref(&pattern), Cut::EveryLane, None);

        assert_eq!(
            clean, silenced,
            "muting and soloing changed what was exported"
        );
    }

    #[test]
    fn a_name_that_could_be_a_path_is_reduced_to_a_name() {
        // ⛔ The artist id reaches here from the webview, so this is a trust
        // boundary in the same sense `check_song` is. A separator in a dialog's
        // suggested name is refused outright by some platforms and silently
        // truncated by others; a traversal moves the starting directory.
        assert_eq!(sanitize("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize("trap/../../x"), "trap-x");
        assert_eq!(sanitize("osamason-2291.mid"), "osamason-2291.mid");
        assert_eq!(sanitize("uk drill 7"), "uk-drill-7");
    }

    #[test]
    fn an_unusable_name_still_produces_a_file_name() {
        // A dialog opened with an empty name shows the folder and no file, which
        // reads as the export having failed before it started.
        assert_eq!(sanitize(""), "song.mid");
        assert_eq!(sanitize("///"), "song.mid");
        assert_eq!(sanitize(".."), "song.mid");
    }

    #[test]
    fn the_extension_is_added_once_and_only_when_missing() {
        assert_eq!(
            with_extension(PathBuf::from("beat")),
            PathBuf::from("beat.mid")
        );
        assert_eq!(
            with_extension(PathBuf::from("beat.mid")),
            PathBuf::from("beat.mid")
        );
        // Case-insensitively, because Save As on Windows and macOS will hand
        // back whatever the producer typed.
        assert_eq!(
            with_extension(PathBuf::from("beat.MID")),
            PathBuf::from("beat.MID")
        );
        // A name with an unrelated extension keeps its stem and becomes a .mid,
        // rather than becoming `beat.wav.mid`.
        assert_eq!(
            with_extension(PathBuf::from("beat.wav")),
            PathBuf::from("beat.mid")
        );
    }

    #[test]
    fn a_terminal_status_is_taken_once_and_running_is_not() {
        let exports = Exports::default();
        // ⛔ The page polls, so a `Done` left in the slot would re-announce the
        // same export on every tick — a toast that never goes away.
        *exports.status.lock().unwrap() = Status::Done {
            path: "C:/x/beat.mid".into(),
        };
        assert!(matches!(exports.take_status(), Status::Done { .. }));
        assert_eq!(exports.take_status(), Status::Idle);

        *exports.status.lock().unwrap() = Status::Running;
        assert_eq!(exports.take_status(), Status::Running);
        assert_eq!(
            exports.take_status(),
            Status::Running,
            "running is not an outcome"
        );
        *exports.status.lock().unwrap() = Status::Idle;
    }

    #[test]
    fn stems_land_in_their_own_folder_under_the_one_that_was_picked() {
        // ⚠ Five files dropped straight into somebody's Desktop is a mess they
        // clean up by hand, and it makes "which of these go together"
        // unanswerable the moment a second song is exported.
        let root = std::env::temp_dir().join("fmm-stem-test");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("trap-7-stems");

        let files = vec![
            ("FMM Drums.mid".to_owned(), vec![0x4d, 0x54, 0x68, 0x64]),
            ("FMM Melody.mid".to_owned(), vec![0x4d, 0x54, 0x68, 0x64]),
        ];
        let status = write_stems(&dir, &files);

        assert!(matches!(status, Status::Done { .. }), "{status:?}");
        assert!(dir.join("FMM Drums.mid").is_file());
        assert!(dir.join("FMM Melody.mid").is_file());
        // Nothing was written beside the folder.
        let loose = std::fs::read_dir(&root)
            .expect("the root exists")
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(loose, 0, "a stem landed outside its folder");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_song_that_plays_nothing_is_refused_rather_than_writing_an_empty_folder() {
        let exports = Exports::default();
        // A folder of nothing is a successful-looking export the producer then
        // has to work out was always empty.
        let empty = engine::pattern::Song {
            id: "s".into(),
            artist_id: "trap".into(),
            seed: 1,
            bpm: 140.0,
            key_root: 0,
            scale: engine::pattern::Scale::NaturalMinor,
            sections: vec![],
            time_sig_num: 4,
            time_sig_den: 4,
            patterns: Default::default(),
            ppq: engine::pattern::PPQ,
        };
        let err = exports.start_song_stems(empty, "trap-1-stems").unwrap_err();
        assert!(err.contains("plays nothing"), "{err}");
        // And it did not leave the mailbox claiming an export is running.
        assert_eq!(exports.take_status(), Status::Idle);
    }

    #[test]
    fn cancelling_is_not_a_failure() {
        let exports = Exports::default();
        // Closing a Save As is the ordinary way out of it. Reporting it as an
        // error trains people to ignore the one message that matters.
        *exports.status.lock().unwrap() = Status::Cancelled;
        let taken = exports.take_status();
        assert_eq!(taken, Status::Cancelled);
        assert!(!matches!(taken, Status::Failed { .. }));
        *exports.status.lock().unwrap() = Status::Idle;
    }
}

#[cfg(test)]
mod stem_name_tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale};

    fn pattern(lane: Lane, artist: &str) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: artist.into(),
            seed: 7,
            song_seed: 7,
            bars: 4,
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
                    reversed: false,
                }],
            }],
            ppq: engine::pattern::PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    /// ⛔⛔ Mike, 2026-08-06: *"it has to be separate midi clips one after the
    /// other, but on the same line unless you hold ctrl … then it stacks them."*
    #[test]
    fn a_sequential_cut_starts_each_lane_where_the_last_one_ended() {
        let kit = three_lanes();
        let span = kit.ticks_per_bar() * u32::from(kit.bars);
        let cut = in_sequence(&kit);

        // ⚠ Two, not three: the fixture's snare is empty and `per_lane` drops
        // it — so the hat becomes the *second* clip and starts one span in,
        // not the third starting two spans in. A silent lane must not leave a
        // gap in the sequence.
        assert_eq!(
            cut.len(),
            2,
            "one clip per playing lane, still separate files"
        );
        for (index, one) in cut.iter().enumerate() {
            assert_eq!(
                one.lanes[0].notes[0].start_tick,
                span * index as u32,
                "lane {index} does not begin where lane {} ended",
                index.saturating_sub(1)
            );
        }
    }

    #[test]
    fn a_sequential_clip_is_long_enough_to_hold_the_notes_it_was_given() {
        // ⛔ **`within_clip` is honoured by the MIDI writer and the renderer**,
        // so a clip left at its original length would silently drop every lane
        // but the first — the file would exist, be named for its lane, and be
        // empty. That is the readout-that-lies failure in its worst form,
        // because the drag would look like it worked.
        let kit = three_lanes();
        for (index, one) in in_sequence(&kit).iter().enumerate() {
            let last = one.lanes[0]
                .notes
                .iter()
                .map(|n| n.start_tick)
                .max()
                .unwrap();
            assert!(
                last < one.ticks_per_bar() * u32::from(one.bars),
                "lane {index}'s notes fall outside its own clip"
            );
        }
    }

    #[test]
    fn the_stacked_cut_leaves_every_lane_at_the_start() {
        // The Ctrl half, and the thing the sequential cut must differ from:
        // `EveryLane` is unchanged and every lane still begins at bar 1.
        let kit = three_lanes();
        for one in per_lane(&kit) {
            assert_eq!(one.lanes[0].notes[0].start_tick, 0);
            assert_eq!(one.bars, kit.bars, "the clip grew when it should not have");
        }
    }

    #[test]
    fn both_layouts_name_their_files_the_same_way() {
        // ⚠ The two sets are spooled into sibling folders precisely because
        // they collide by name — which is correct, since a lane stem is named
        // for its lane in either layout. If this ever stops being true,
        // `render_and_spool`'s sub-folder stops being necessary and the comment
        // there becomes a lie.
        let kit = three_lanes();
        let sequential: Vec<String> =
            stem_files(std::slice::from_ref(&kit), Cut::EveryLaneInSequence, None)
                .into_iter()
                .map(|(name, _)| name)
                .collect();
        let stacked: Vec<String> = stem_files(&[kit], Cut::EveryLane, None)
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        assert_eq!(sequential, stacked);
        assert_eq!(sequential.len(), 2);
    }

    #[test]
    fn a_stem_is_labelled_the_way_a_producer_would_label_it() {
        // ⛔ Mike, 2026-08-05, verbatim: "it needs to be labeled when you drag
        // it out like this: Artist/Genre - Snares - 140 BPM - C# Minor".
        assert_eq!(
            stem_name(&pattern(Lane::Snare, "trap"), true),
            "trap - Snare - 140 BPM - C# Minor"
        );
        // Without the lane split it is named for the part instead — and the
        // engine's `FMM ` prefix is dropped, because the name already says
        // which artist it came from.
        assert_eq!(
            stem_name(&pattern(Lane::Snare, "uk-drill"), false),
            "uk-drill - Drums - 140 BPM - C# Minor"
        );
    }

    #[test]
    fn a_crafted_artist_id_cannot_write_outside_the_chosen_folder() {
        // ⛔ `artist_id` arrives as JSON from the webview, so this is the same
        // trust boundary `sanitize` guards for the Save As name.
        let name = stem_name(&pattern(Lane::Kick, "../../etc/passwd"), true);
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert!(!name.contains(".."), "{name}");
    }

    #[test]
    fn the_readable_form_survives_where_sanitize_would_destroy_it() {
        // ⚠ The reason `safe_file_name` exists at all: `sanitize` is right for a
        // Save As suggestion and would turn this into `trap-Snare-140-BPM-C-Minor`.
        let name = stem_name(&pattern(Lane::Snare, "trap"), true);
        assert!(name.contains(" - "), "spaces must survive: {name}");
        assert!(name.contains("C#"), "the sharp must survive: {name}");
        assert_ne!(sanitize(&name), name, "sanitize is the stricter one");
    }

    #[test]
    fn splitting_by_lane_gives_one_pattern_per_lane_and_drops_the_empty_ones() {
        let mut source = pattern(Lane::Kick, "trap");
        source.lanes.push(LaneTrack {
            lane: Lane::Snare,
            notes: vec![],
        });
        source.lanes.push(LaneTrack {
            lane: Lane::ClosedHat,
            notes: source.lanes[0].notes.clone(),
        });

        let split = per_lane(&source);
        assert_eq!(split.len(), 2, "the empty snare lane gets no file");
        // Everything else is copied, so the files line up when they are all
        // dropped at bar 1.
        for one in &split {
            assert_eq!(one.bars, source.bars);
            assert_eq!(one.bpm, source.bpm);
            assert_eq!(one.time_sig_num, source.time_sig_num);
            assert_eq!(one.ppq, source.ppq);
        }
    }

    /// A drum pattern with a kick, an empty snare and a hat.
    fn three_lanes() -> Pattern {
        let mut source = pattern(Lane::Kick, "trap");
        source.lanes.push(LaneTrack {
            lane: Lane::Snare,
            notes: vec![],
        });
        source.lanes.push(LaneTrack {
            lane: Lane::ClosedHat,
            notes: source.lanes[0].notes.clone(),
        });
        source
    }

    #[test]
    fn splitting_by_lane_names_every_file_after_its_own_lane() {
        // ⛔ This is the "drag just the hihats out" case, and the names are the
        // whole point of it: `FMM Drums` three times over is three files a
        // producer cannot tell apart.
        let files = stem_files(&[three_lanes()], Cut::EveryLane, None);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "trap - Kick - 140 BPM - C# Minor.mid",
                "trap - ClosedHat - 140 BPM - C# Minor.mid",
            ],
            "the empty snare lane must get no file at all"
        );
    }

    #[test]
    fn without_the_split_the_whole_part_is_one_file_named_for_the_part() {
        let files = stem_files(&[three_lanes()], Cut::Parts, None);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["trap - Drums - 140 BPM - C# Minor.mid"]);
    }

    #[test]
    fn two_patterns_that_would_share_a_name_get_two_files() {
        // ⛔ Same artist, same part, same tempo, same key — so `stem_name`
        // answers the same thing twice. Without the de-collision the second
        // overwrites the first *and both paths are handed to the drop target*,
        // so a producer who asked for two clips gets one of them twice. The UI
        // does not produce this; a project file can.
        let one = pattern(Lane::Kick, "trap");
        let two = pattern(Lane::Kick, "trap");
        let files = stem_files(&[one, two], Cut::Parts, None);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "trap - Drums - 140 BPM - C# Minor.mid",
                "trap - Drums - 140 BPM - C# Minor (2).mid",
            ]
        );
    }

    #[test]
    fn a_part_that_plays_nothing_gets_no_file_rather_than_a_silent_one() {
        // An empty stem is one a producer imports, hears nothing from, and has
        // to work out was always empty.
        let mut silent = pattern(Lane::Kick, "trap");
        silent.lanes[0].notes.clear();
        assert!(stem_files(&[silent], Cut::Parts, None).is_empty());
    }

    #[test]
    fn the_audio_half_writes_a_wav_our_own_reader_accepts() {
        // ⛔ The claim TASK-069 could not make, and the one the drag-out rests
        // on: the melodic parts render audibly rather than silently. If this
        // ever regresses, a drag would hand the DAW a file of zeros.
        let kit = crate::audio::preview_kit().expect("the shipped kit must load");
        let files = stem_files(&[pattern(Lane::Snare, "trap")], Cut::EveryLane, Some(kit));
        let (name, bytes) = files.first().expect("a snare must render");
        assert_eq!(name, "trap - Snare - 140 BPM - C# Minor.wav");
        let decoded = crate::audio::kit::decode_wav(bytes).expect("our own WAV must decode");
        assert!(
            decoded.samples.iter().any(|s| s.abs() > 0.01),
            "the rendered stem is silent"
        );
    }
}
