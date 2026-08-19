//! The drum generator: the kick grammar, snare placement and ghost notes
//! (FR-003, research ch. 1).
//!
//! Everything here writes notes **on the grid**. The feel — swing, jitter,
//! velocity spread — is [`crate::humanize`]'s job and the caller applies it
//! after, so these tests can say "the snare is on beat 3" and mean the tick
//! rather than a tolerance. The one exception is `offGridMs`, a deliberate
//! displacement a genre is *made of* (UK drill's nudged snare) rather than a
//! hand being imprecise; that belongs to the grammar and is applied here.
//!
//! Lanes are generated in a fixed order and each draws from its own seeded
//! stream, so rerolling the snare cannot move the kick.

use std::collections::BTreeMap;

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::read::{
    block, flag, number, optional_number, pair, string_spec, strings, text,
};
use crate::generators::{grid, rolls};
use crate::humanize::VelocityTiers;
use crate::midi::gm_drum_note;
use crate::pattern::{Articulation, Lane, LaneTrack, Note, SectionKind};
use crate::rng;
use crate::theory;

/// How long a drum hit is written for.
///
/// A one-shot's length is decided by its sample and its envelope, not by the
/// note — but a zero-length note is invalid in an SMF and invisible in a piano
/// roll, so drums get a 16th and the sampler ignores it.
const HIT_TICKS: u32 = grid::SIXTEENTH;

/// The tempo at which a half-time genre is heard as fast, for
/// `snare.fullTimeAtHighTempoProb`.
///
/// 140 is the line the roster itself draws: every half-time trap and drill model
/// sits under it and `rage` — the one model that authors the key — states
/// `bpm.mode: 150` over a 130–170 range, so the switch is reachable from inside
/// its own tempo and silent at the bottom of it. `ctx.half_time` deliberately
/// does not enter into it: the producer reads the number on the transport, and
/// that number is the stated BPM.
const FULL_TIME_BPM: f32 = 140.0;

/// How close a snare has to be to an 808 note before the two count as the same
/// musical moment rather than one muting the other.
const MUTE_TOLERANCE: u32 = grid::SIXTEENTH / 2;

/// Lane order: the order a drum pattern is built, exported and drawn in.
///
/// Kick first because the whole grammar hangs off it — the 808 locks to it and
/// the snare gap rule is measured against it.
///
/// ⛔⛔ **A lane missing from here is a lane whose notes are thrown away**, and
/// TASK-043A hit it: eleven lanes were added to [`PERC_LANES`], authored by nine
/// shipped models, and produced *nothing* — `Kit` accumulates per lane in this
/// order and drops what it has no slot for. The two lists are not
/// interchangeable (see [`PERC_LANES`]), but **every** `PERC_LANES` entry must
/// appear here, which is what `every_perc_lane_can_be_built` holds.
/// `pub` for [`crate::smf_read`], which needs the inverse of `gm_drum_note` and
/// must not restate the lane list to get it — two lists naming the same set is
/// how one of them starts being wrong, which is the failure this file's own
/// notes record twice over.
pub const LANE_ORDER: &[Lane] = &[
    Lane::Kick,
    Lane::SubKick,
    Lane::Snare,
    Lane::OffSnare,
    Lane::GhostSnare,
    Lane::Clap,
    Lane::ClosedHat,
    Lane::OpenHat,
    Lane::PedalHat,
    Lane::Ride,
    Lane::RideBell,
    Lane::Crash,
    Lane::Tom,
    Lane::TomHigh,
    Lane::TomLow,
    Lane::Clave,
    Lane::Conga,
    Lane::Bongo,
    Lane::Timbale,
    Lane::Triangle,
    Lane::Perc2,
    Lane::Riser,
    Lane::Impact,
    Lane::Reverse,
    Lane::Rim,
    Lane::Snap,
    Lane::Perc,
    Lane::Shaker,
    Lane::Tambourine,
    Lane::Cowbell,
    Lane::Woodblock,
    Lane::Sub,
];

/// The lanes a model may name in `drums.percs.lanes`.
///
/// Deliberately not every lane in [`LANE_ORDER`]: the kick, the snare and the
/// hats have their own authored blocks and their own placement grammar, so
/// naming one here would put two stages in charge of the same voice. These are
/// the ones whose whole behaviour is "sprinkle hits at this density".
pub const PERC_LANES: &[Lane] = &[
    Lane::Ride,
    Lane::Crash,
    Lane::Tom,
    Lane::Rim,
    Lane::Snap,
    Lane::Perc,
    Lane::Shaker,
    Lane::Tambourine,
    Lane::Cowbell,
    Lane::Woodblock,
    // ── TASK-043A ────────────────────────────────────────────────────────
    //
    // ⛔ **The hand and Latin percussion the genres actually describe.**
    // Research ch. 1 names a vibraslap and a triangle in west coast, claves
    // and cowbell in Memphis, and congas across the afrobeats work Phase 5
    // will author. Until now a model asking for any of them had nowhere to
    // put it — `uk-drill` asked for a `woodblock` before there was a lane and
    // the request vanished twice over, which is what TASK-140 records.
    //
    // ⚠ **The FX lanes are here too**, because a riser and an impact are
    // placed exactly like a perc — a density and a placement — and giving
    // them their own authoring block would be a second scheduler for the same
    // job.
    Lane::TomHigh,
    Lane::TomLow,
    Lane::Perc2,
    Lane::Clave,
    Lane::Conga,
    Lane::Bongo,
    Lane::Timbale,
    Lane::Triangle,
    Lane::Riser,
    Lane::Impact,
    Lane::Reverse,
    // ── TASK-140, second pass ────────────────────────────────────────────
    //
    // ⛔⛔ **Four shipped models were already asking for these two and being
    // silently refused**, which is the `woodblock` failure this list's own
    // history records, happening again while nothing looked: `john-mayer` and
    // `maroon-5` name `pedalHat`, `chris-dave` and `robert-glasper` name
    // `rideBell`. Both are real `Lane` arms with a GM note (`midi.rs`: 44 and
    // 53), a humanize stream, a kit pad and a lane bit — but `percs()` filters
    // `lanes` through **this** list, so the request was dropped between the
    // dataset and the grid with nothing on any gate saying so.
    //
    // ⚠ **They belong here on the rule this list already states**, not as an
    // exception to it: the kick, snare and hats are excluded because each has
    // an authored block and a placement grammar of its own. A pedal hat and a
    // ride bell have neither — their whole behaviour is "sprinkle hits at this
    // density", which is the membership test.
    //
    // ▶ Found by measuring the dataset against this constant rather than by
    // reading either one. That check is now `every_authored_perc_lane_is_one_the_engine_writes`.
    Lane::PedalHat,
    Lane::RideBell,
];

/// Where the snare lands, bar by bar (PRD § 3, research ch. 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum SnarePlacement {
    /// Beat 3 only — the half-time feel of trap and drill.
    #[serde(rename = "halftime_3")]
    Halftime3,
    /// Beats 2 and 4 — the full-time backbeat.
    #[serde(rename = "backbeat_24")]
    Backbeat24,
    /// Beat 3 in the first bar, beat 4 in the second: the NY drill two-bar form.
    #[serde(rename = "drill_3_4")]
    Drill34,
    /// A 16th-note stream with the backbeat accented — the country train beat.
    #[serde(rename = "train_16ths")]
    Train16ths,
    /// Beats 1 and 3 — the Milwaukee `lowend` cell, and the only placement here
    /// that puts a snare on the downbeat.
    ///
    /// ⛔ **Added 2026-08-15 because the archetype could not be spelled without
    /// it.** Volume 3 sources `lowend` as *"8th-note handclaps + snares on 1 and
    /// 3"*, and `jerk` carries Certified Trapper and J.P. as a **declared
    /// exception** precisely because no model could hold them. Two placements
    /// shipped — `halftime_3` and `backbeat_24` — and a snare on 1 and 3 is
    /// neither: it is the backbeat displaced a beat early, which is what makes
    /// the lane sound like it is falling forwards.
    #[serde(rename = "downbeat_1_3")]
    Downbeat13,
}

impl SnarePlacement {
    /// ⛔⛔ **Parsed THROUGH serde, so the names exist once** (TASK-167). This was
    /// a hand-written match beside a hand-written list in
    /// `StyleEditor.tsx::PLACEMENTS`, and the doc there stated the hazard
    /// outright — *"adding a sixth there without adding it here is a placement no
    /// user model can reach"*. TASK-158F then did exactly that for `lowend`, and
    /// it only worked because both edits were remembered, which is not a
    /// mechanism. The variant renames above are now the single spelling: serde
    /// reads them, ts-rs exports them to `ipc-types.ts`, and the editor's radio
    /// list is driven off that union — so a sixth placement is a typecheck
    /// failure rather than an unreachable feature.
    ///
    /// ⚠ Same idiom as `lane_by_name` below, for the same reason.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_value(Value::String(text.to_owned())).ok()
    }

    /// The snare hits in one bar, as `(tick within the bar, articulation)`.
    ///
    /// A placement names beats by number, and not every meter has all of them:
    /// a 2-and-4 backbeat in 3/4 has no beat 4. Hits that fall outside the bar
    /// are dropped rather than written, which is the same rule
    /// [`grid::position_ticks`] applies to authored positions — without it the
    /// "beat 4" of a 3/4 bar landed on the downbeat of the next one, and in the
    /// final bar it escaped the pattern altogether.
    fn hits(self, bar: u32, ctx: &SessionContext) -> Vec<(u32, Option<Articulation>)> {
        let bar_ticks = ctx.ticks_per_bar();
        let mut hits = self.hits_unbounded(bar, ctx);
        hits.retain(|(tick, _)| *tick < bar_ticks);
        hits
    }

    /// The placement's beats, before the meter is taken into account.
    fn hits_unbounded(self, bar: u32, ctx: &SessionContext) -> Vec<(u32, Option<Articulation>)> {
        let beat = grid::ticks_per_beat(ctx);
        match self {
            Self::Halftime3 => vec![(beat * 2, None)],
            Self::Backbeat24 => vec![(beat, None), (beat * 3, None)],
            // ⚠ **Beat 1 is tick 0**, so this is the one placement that puts a
            // snare on the downbeat with the kick. That collision is the sound,
            // not a fault: the research names the pair together — handclaps on
            // the eighths *over* snares on 1 and 3.
            Self::Downbeat13 => vec![(0, None), (beat * 2, None)],
            // Bar 1 of the pair takes beat 3, bar 2 takes beat 4.
            Self::Drill34 => {
                if bar.is_multiple_of(2) {
                    vec![(beat * 2, None)]
                } else {
                    vec![(beat * 3, None)]
                }
            }
            Self::Train16ths => (0..grid::sixteenths_per_bar(ctx))
                .map(|i| {
                    let tick = i * grid::SIXTEENTH;
                    // The backbeat is what a train beat is heard as; everything
                    // between it is the engine underneath.
                    let articulation = if tick == beat || tick == beat * 3 {
                        Some(Articulation::Accent)
                    } else {
                        Some(Articulation::Ghost)
                    };
                    (tick, articulation)
                })
                .collect(),
        }
    }
}

/// Notes accumulating per lane, in the order [`LANE_ORDER`] states.
///
/// Every generator stage pushes into one of these; the empty lanes are dropped
/// at the end so a pattern does not export silent tracks.
#[derive(Debug, Default)]
pub struct DrumKit {
    notes: BTreeMap<Lane, Vec<Note>>,
}

impl DrumKit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Place one hit.
    pub fn hit(&mut self, lane: Lane, tick: u32, vel: u8, articulation: Option<Articulation>) {
        // ⛔⛔ **ONE VOICE CANNOT SOUND TWICE AT ONE INSTANT, so the second note
        // is refused here rather than at each of the writers** (2026-08-12).
        // Several stages target the same lane and none of them can see the
        // others: `percs.lanes` may name the tambourine while
        // `tambourineMirrorsClap` mirrors the clap into it, and a steady
        // tambourine stream is a third writer again. Volume 1 found two
        // different pairs within one gate run — `1500-or-nothin` at 4 bars seed
        // 4, then `beanie-sigel` at seed 0 — which is the shape of a class, not
        // of two cases.
        //
        // ⚠ **The duplicate was never audible and always shipped.** It reaches
        // `to_midi`, the host's track and `stem_files` as a second note-on at the
        // same tick on the same GM note; nothing plays it and everything carries
        // it. Keeping the first writer's note is deterministic and sounds
        // identical.
        //
        // ⚠ `extend` deliberately does **not** do this — the hat stream builds
        // its own stream and knows what it placed — so
        // `no_lane_ever_carries_two_notes_on_the_same_tick` still has the paths
        // that bypass this to guard.
        let notes = self.notes.entry(lane).or_default();
        if notes.iter().any(|note| note.start_tick == tick) {
            return;
        }
        notes.push(note_at(lane, tick, vel, articulation));
    }

    /// Add notes a stage built on its own — the hat stream, which has to know
    /// what it already placed before an open hat can close it.
    pub fn extend(&mut self, lane: Lane, notes: impl IntoIterator<Item = Note>) {
        self.notes.entry(lane).or_default().extend(notes);
    }

    /// Clear the main hits from a stretch of a lane so a fill can take it.
    ///
    /// Ghosts stay. A fill replaces the pattern it interrupts, but drill's
    /// and-of-4 ghost snare is a signature answering the backbeat and lives in
    /// exactly the beat a fill lands on — clearing those too cost the genre
    /// half of them, which no listener would call a fill.
    pub fn clear_for_fill(&mut self, lane: Lane, range: std::ops::Range<u32>) {
        if let Some(notes) = self.notes.get_mut(&lane) {
            notes.retain(|n| {
                !range.contains(&n.start_tick) || n.articulation == Some(Articulation::Ghost)
            });
        }
    }

    pub fn notes(&self, lane: Lane) -> &[Note] {
        self.notes.get(&lane).map(Vec::as_slice).unwrap_or_default()
    }

    /// One lane's notes, to be rewritten in place.
    ///
    /// ⛔ **For the keys that are facts about the finished lane rather than
    /// about a hit.** `lockedBackbeat` marks whichever notes survived the fills
    /// and `detuneSemis` retunes every snare in the part, including the ones
    /// `fills` and `percs` wrote — neither can be spelled at [`Self::hit`],
    /// which sees one note and does not know which pass will take it away.
    pub fn notes_mut(&mut self, lane: Lane) -> &mut Vec<Note> {
        self.notes.entry(lane).or_default()
    }

    /// The finished lanes, ordered, sorted and without the empty ones.
    pub fn into_lanes(mut self) -> Vec<LaneTrack> {
        LANE_ORDER
            .iter()
            .filter_map(|lane| {
                let mut notes = self.notes.remove(lane)?;
                if notes.is_empty() {
                    return None;
                }
                notes.sort_by_key(|n| n.start_tick);
                Some(LaneTrack { lane: *lane, notes })
            })
            .collect()
    }
}

/// One drum note. The pitch is the lane's GM voice, so a pattern reads
/// correctly everywhere before the writer replaces it with the same value.
fn note_at(lane: Lane, tick: u32, vel: u8, articulation: Option<Articulation>) -> Note {
    Note {
        model_vel: None,
        start_tick: tick,
        len_ticks: HIT_TICKS,
        pitch: gm_drum_note(lane),
        vel: vel.max(1),
        slide_to_pitch: None,
        slide_ms: None,
        slide_overlap_ticks: None,
        timing_locked: false,
        articulation,
        reversed: false,
    }
}

/// Pick one entry by weight and take it out of the pool.
fn take_weighted(pool: &mut Vec<(u32, f64)>, rng: &mut impl Rng) -> Option<u32> {
    let total: f64 = pool.iter().map(|(_, w)| *w).sum();
    if pool.is_empty() || total <= 0.0 {
        return None;
    }
    let roll = rng.random_range(0.0..total);
    let mut acc = 0.0;
    for i in 0..pool.len() {
        acc += pool[i].1;
        if roll < acc {
            return Some(pool.remove(i).0);
        }
    }
    // Floating-point accumulation can land a hair under the total.
    pool.pop().map(|(tick, _)| tick)
}

/// The candidate kick positions in a bar, split by how they feel.
struct Pools {
    downbeats: Vec<(u32, f64)>,
    offbeat_eighths: Vec<(u32, f64)>,
    sixteenths: Vec<(u32, f64)>,
}

impl Pools {
    fn build(ctx: &SessionContext, tresillo_bias: f64, taken: &[u32]) -> Self {
        let mut pools = Pools {
            downbeats: Vec::new(),
            offbeat_eighths: Vec::new(),
            sixteenths: Vec::new(),
        };
        for i in 0..grid::sixteenths_per_bar(ctx) {
            let tick = i * grid::SIXTEENTH;
            if taken.contains(&tick) {
                continue;
            }
            // The 3-3-2 positions are weighted up rather than forced: a model
            // with a high tresilloBias leans on them, it does not only use them.
            let weight = if grid::is_tresillo(i) {
                1.0 + tresillo_bias * 3.0
            } else {
                1.0
            };
            if grid::is_downbeat(i, ctx) {
                pools.downbeats.push((tick, weight));
            } else if grid::is_offbeat_eighth(i, ctx) {
                pools.offbeat_eighths.push((tick, weight));
            } else {
                pools.sixteenths.push((tick, weight));
            }
        }
        pools
    }
}

/// The multi-bar kick form this pattern will play, resolved once.
///
/// ⛔ **"Exactly" used to mean "only", and that was a defect.** `kick_bar`
/// returned `grammar[bar % len]` and never touched the rng, so **uk-drill,
/// ny-drill and pop-smoke wrote exactly one kick pattern across 200 seeds** —
/// measured 2026-08-05, after Mike reported the roster sounding the same in
/// Ableton. A signature that cannot vary is not a signature; it is a loop.
///
/// ⚠ **The fix is not to make the grammar statistical.** That would throw away
/// the thing it exists to protect — drill's two-bar form is the genre, and an
/// approximation of it is a different genre. A model may instead author
/// `grammarVariants`: several complete multi-bar forms, one chosen per pattern
/// from the seed. Every row still reproduces exactly; there are simply more than
/// one of them. Mike, 2026-08-05: "as many distinct drum patterns as possible
/// per artist/producer **as long as it follows that artist's type of
/// workflow**" — for a grammar, this is what that sentence means.
///
/// ⛔ **Resolved here rather than inside `kick_bar`, and that is load-bearing.**
/// `kick_bar` runs once per bar off one rng stream, so drawing there would pick
/// a different form every bar — cutting between two two-bar shapes mid-phrase
/// and destroying the very thing the grammar encodes.
///
/// ⚠ `fourBarGrammar` still works and still means exactly what it meant. An
/// artist authoring one form gets one form; nothing already in the dataset
/// changes behaviour.
fn kick_grammar(
    kick: Option<&Value>,
    ctx: &SessionContext,
    rng: &mut impl Rng,
) -> Option<Vec<Vec<u32>>> {
    let rows = |form: &Value| -> Vec<Vec<u32>> {
        form.as_array()
            .map(|bars| {
                bars.iter()
                    .map(|row| {
                        let mut ticks: Vec<u32> = row
                            .as_array()
                            .map(|positions| {
                                positions
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .filter_map(|p| grid::position_ticks(p, ctx))
                                    .collect()
                            })
                            .unwrap_or_default();
                        ticks.sort_unstable();
                        ticks.dedup();
                        ticks
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let variants = kick
        .and_then(|k| k.get("grammarVariants"))
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty());

    if let Some(variants) = variants {
        let choice = rng.random_range(0..variants.len());
        let form = rows(&variants[choice]);
        if !form.is_empty() {
            return Some(form);
        }
    }

    let single = kick.and_then(|k| k.get("fourBarGrammar"))?;
    let form = rows(single);
    (!form.is_empty()).then_some(form)
}

/// The positions a model says its kick always plays, in ticks from the bar.
///
/// Shared by the fill logic below and by `beatSkipProb`, which must not be able
/// to take one of them away.
fn anchor_ticks(kick: Option<&Value>, ctx: &SessionContext) -> Vec<u32> {
    let mut ticks: Vec<u32> = strings(kick, "anchors")
        .iter()
        .filter_map(|p| grid::position_ticks(p, ctx))
        .collect();
    if let Some(secondary) = kick
        .and_then(|k| k.get("secondaryAnchor"))
        .and_then(Value::as_str)
        .and_then(|p| grid::position_ticks(p, ctx))
    {
        ticks.push(secondary);
    }
    ticks.sort_unstable();
    ticks.dedup();
    ticks
}

/// One bar of kick, placed from the grammar in the model.
fn kick_bar(
    kick: Option<&Value>,
    grammar: Option<&Vec<Vec<u32>>>,
    ctx: &SessionContext,
    bar: u32,
    snares: &[u32],
    rng: &mut impl Rng,
    // ⛔ `beatSkipProb` and `walkingRunProb`, and nothing else. Drawing them from
    // `rng` would shift every *later* bar's grammar as well as changing this
    // one — so turning a skip on would move 43 models' kicks in bars it never
    // touched. See [`generate`]'s streams for where that lesson came from.
    extras: &mut impl Rng,
) -> Vec<u32> {
    // An explicit multi-bar grammar wins over everything statistical: drill's
    // `[["1","2&","4"], ["1&","3"]]` is the genre's signature two-bar form and
    // must reproduce exactly, not approximately. Which *form* is in play was
    // decided once for the whole pattern — see [`kick_grammar`].
    if let Some(grammar) = grammar.filter(|g| !g.is_empty()) {
        return grammar[(bar as usize) % grammar.len()].clone();
    }

    let syncopation = number(kick, "syncopation", 0.3, rng).clamp(0.0, 1.0);
    let tresillo_bias = number(kick, "tresilloBias", 0.0, rng).clamp(0.0, 1.0);
    let offbeat_share = optional_number(kick, "offbeat8thShare", rng)
        .unwrap_or(syncopation)
        .clamp(0.0, 1.0);
    let density = number(kick, "densityPerBar", 3.0, rng).round().max(1.0) as usize;

    // Anchors first: the positions the genre always plays.
    let mut ticks = anchor_ticks(kick, ctx);
    // ⚠ Everything pushed from here on was filled to density, and `Pools::build`
    // skips every tick already `taken` — so `ticks[..anchors]` stays exactly the
    // anchor set without a second parse of the model to prove it.
    let anchors = ticks.len();

    // Then fill to the sampled density.
    let mut pools = Pools::build(ctx, tresillo_bias, &ticks);
    while ticks.len() < density {
        // Each branch states a preference and then falls through the other two:
        // an empty favourite pool means "take the next best", never "give up".
        // Missing one of these fallbacks stopped a dense bar filling at all.
        let picked = if rng.random_bool(offbeat_share) {
            take_weighted(&mut pools.offbeat_eighths, rng)
                .or_else(|| take_weighted(&mut pools.sixteenths, rng))
                .or_else(|| take_weighted(&mut pools.downbeats, rng))
        } else if rng.random_bool((syncopation * 0.4).clamp(0.0, 1.0)) {
            take_weighted(&mut pools.sixteenths, rng)
                .or_else(|| take_weighted(&mut pools.offbeat_eighths, rng))
                .or_else(|| take_weighted(&mut pools.downbeats, rng))
        } else {
            take_weighted(&mut pools.downbeats, rng)
                .or_else(|| take_weighted(&mut pools.offbeat_eighths, rng))
                .or_else(|| take_weighted(&mut pools.sixteenths, rng))
        };
        match picked {
            Some(tick) => ticks.push(tick),
            // The bar is full. A density wider than the grid is a model error,
            // not a reason to loop forever.
            None => break,
        }
    }

    // ⛔ **The skip comes after the density is met, not out of it.** Taking one
    // off `density` would make the bar shorter *and* re-shape which pools it
    // drew from; dropping a placed note leaves the bar's grammar intact and
    // takes a hit out of it, which is what a skipped beat is.
    //
    // ⚠ **Anchors are never skipped.** They are the positions the genre "always
    // plays" — `kick_bar`'s own words — so a skip that can take beat 1 is not a
    // sparser bar, it is a different genre.
    if let Some(chance) = optional_number(kick, "beatSkipProb", extras) {
        let droppable = ticks.len() - anchors;
        if droppable > 0 && extras.random_bool(chance.clamp(0.0, 1.0)) {
            ticks.remove(anchors + extras.random_range(0..droppable));
        }
    }

    // Jerk's walking kick run: three 16ths climbing into the next downbeat,
    // rather than the single lead-in `andOf4EveryOtherBar` writes.
    if let Some(chance) = optional_number(kick, "walkingRunProb", extras) {
        if extras.random_bool(chance.clamp(0.0, 1.0)) {
            let bar_ticks = ctx.ticks_per_bar();
            // Ends on the last 16th of the bar, so the run arrives *at* the next
            // downbeat instead of landing on it — the downbeat is the next bar's
            // to place, and doubling it is what makes a run sound like a stutter.
            for step in 1..=3u32 {
                if let Some(tick) = bar_ticks.checked_sub(step * grid::SIXTEENTH) {
                    ticks.push(tick);
                }
            }
        }
    }

    // Trap's "and-of-4 kick every other bar", which is a lead-in to the next
    // bar rather than part of this one's density.
    if bar % 2 == 1 {
        if let Some(chance) = optional_number(kick, "andOf4EveryOtherBar", rng) {
            if rng.random_bool(chance.clamp(0.0, 1.0)) {
                if let Some(tick) = grid::position_ticks("4&", ctx) {
                    ticks.push(tick);
                }
            }
        }
    }

    // Leave the snare its air: no kick inside the gap before one. The rule wins
    // over the density target — the gap is what the genre is described by, and
    // one kick fewer is the price (research ch. 1 §1).
    if let Some(gap) = kick
        .and_then(|k| k.get("avoidPreSnareGap"))
        .and_then(Value::as_str)
        .and_then(grid::note_value_ticks)
    {
        ticks.retain(|tick| {
            !snares
                .iter()
                .any(|snare| *tick < *snare && snare.saturating_sub(*tick) <= gap)
        });
    }

    ticks.sort_unstable();
    ticks.dedup();
    ticks
}

/// Generate the drum lanes for a resolved style model.
///
/// The result is on the grid; the caller runs [`crate::humanize::humanize`]
/// over it. Hats, percussion, rolls, fills and the 808 arrive with their own
/// tasks — this is the kick, the snare and what layers onto them.
/// How many *extra* takes are drawn when the hats come out empty.
///
/// Four is well past enough. The worst model on the roster is `plugg`, which
/// empties a four-bar pattern about once in a hundred; five independent draws
/// put that at one in ten billion, which is never for any practical purpose.
const MAX_HAT_REDRAWS: usize = 4;

/// The domain each redraw derives its seed from.
///
/// Named rather than numbered so a redraw's stream is auditable, and spelled out
/// rather than formatted so the path allocates nothing — the same convention as
/// `novelty::RETRY_DOMAINS`, which this mirrors deliberately.
const HAT_REDRAW_DOMAINS: [&str; MAX_HAT_REDRAWS] = [
    "drums/hats-empty:1",
    "drums/hats-empty:2",
    "drums/hats-empty:3",
    "drums/hats-empty:4",
];

pub fn generate(model: &StyleModel, ctx: &SessionContext, seed: u64) -> Vec<LaneTrack> {
    generate_in(model, ctx, seed, None)
}

/// The same kit, told which section of a song it is playing.
///
/// ⛔ **Two keys are about the *form* rather than about the beat**, and neither
/// could be read while the drums only ever saw `(model, ctx, seed)`:
/// `kick.fourOnFloorInChorus` (29 models) and `snare.crossStickVerses` (74).
/// Both name a section, and a four-bar loop on the Drums tab is not in one — so
/// `None` is a real answer rather than a missing argument, and it means what the
/// tab has always done.
///
/// ⚠ **Not on [`SessionContext`].** That struct is the session's own settings,
/// serialized from the page and stored in a project; the section is a fact about
/// where in an arrangement this call sits, and belongs to the call.
pub fn generate_in(
    model: &StyleModel,
    ctx: &SessionContext,
    seed: u64,
    section: Option<SectionKind>,
) -> Vec<LaneTrack> {
    let drums = model.blocks.get("drums");
    let tiers = VelocityTiers::from_json(drums);
    let mut kit = DrumKit::new();

    let snare_block = block(drums, "snare");
    let kick_block = block(drums, "kick");

    let mut snare_rng = rng::stream(seed, "drums/snare");
    let mut kick_rng = rng::stream(seed, "drums/kick");
    // ⛔⛔ **A decoration draws from its OWN stream, and a gate two lanes away is
    // what proved it.** This module's header states the rule — "each draws from
    // its own seeded stream, so rerolling the snare cannot move the kick" — and
    // a per-hit draw taken from `kick_rng` breaks it from the inside: the sub
    // layer's velocity advanced the kick's own stream, so every later bar drew
    // its grammar from a shifted position. The kick moved, `bass.rs`'s
    // `mirror_kick` rhythm follows the kick, and `jeru-the-damaja` fell to
    // 946/1000 distinct basslines against a floor of 950 — a **bassline** gate
    // failing because of a **drum velocity**.
    //
    // ⚠ **Two keys on one stream still move each other**, and `cluster` and
    // `grammar-extras` each carry two: the snare's cluster with
    // `fullTimeAtHighTempoProb`, and `beatSkipProb` with `walkingRunProb`. That
    // is not the property being bought here. The property is that a model which
    // authors *neither* key of a pair draws nothing at all — every reader above
    // is `optional_number`, `flag` or `pair`, and none of the three touches the
    // rng for a key that is absent — so no model on the roster generates a note
    // it did not generate before. Within a pair the two do interact, and no
    // model authors both today; splitting them further would be four more names
    // for a case that does not exist.
    let mut snare_extra_rng = rng::stream(seed, "drums/snare/cluster");
    let mut kick_extra_rng = rng::stream(seed, "drums/kick/grammar-extras");
    let mut sub_rng = rng::stream(seed, "drums/kick/sub");
    let mut sloppy_rng = rng::stream(seed, "drums/kick/sloppy");
    let mut hat_extra_rng = rng::stream(seed, "drums/hats/extras");
    let mut rimshot_rng = rng::stream(seed, "drums/snare/rimshot");
    let mut sub_tone_rng = rng::stream(seed, "drums/bass808/tone");
    let mut snare_tone_rng = rng::stream(seed, "drums/snare/tone");

    // ⛔ Drawn once, before the bar loop, for the reason `kick_grammar` gives:
    // a form chosen per bar cuts between two two-bar shapes mid-phrase.
    let kick_form = kick_grammar(kick_block, ctx, &mut kick_rng);

    // Placement is decided once for the whole pattern, not per bar: a snare
    // that changes its mind halfway through is not a style, it is a glitch.
    let mut placement = snare_block
        .and_then(|s| s.get("placement"))
        .and_then(Value::as_str)
        .and_then(SnarePlacement::parse)
        .unwrap_or(SnarePlacement::Backbeat24);
    if let Some(chance) = optional_number(snare_block, "fullTimeVariantProb", &mut snare_rng) {
        if snare_rng.random_bool(chance.clamp(0.0, 1.0)) {
            // The uptempo crossover variant (research ch. 1 §1).
            placement = SnarePlacement::Backbeat24;
        }
    }
    // The same switch, but asked for by the tempo rather than unconditionally:
    // rage's half-time snare goes full time when the session is fast.
    //
    // ⚠ **The draw happens whatever the tempo is**, and only its *effect* is
    // gated. Rolling inside the `if` would make the snare's rng position depend
    // on the BPM, so moving the tempo slider would change the rest of the
    // pattern — a saved seed at 130 and the same seed at 150 would not share a
    // ghost note.
    if let Some(chance) =
        optional_number(snare_block, "fullTimeAtHighTempoProb", &mut snare_extra_rng)
    {
        if snare_extra_rng.random_bool(chance.clamp(0.0, 1.0)) && ctx.bpm >= FULL_TIME_BPM {
            placement = SnarePlacement::Backbeat24;
        }
    }

    // A deliberate displacement, in milliseconds, that the genre is made of —
    // UK drill's snare sits off the grid on purpose. Negative pulls it early.
    let off_grid_ticks = optional_number(snare_block, "offGridMs", &mut snare_rng)
        .map(|ms| offset_ticks(ms, ctx))
        .unwrap_or(0);

    // ⛔ **Not `offGridMs` under another name.** `offGridMs` is ONE displacement
    // the whole lane shares — drill's snare sits late, every time, and that is
    // the genre. `sloppyOffsetMs` is a range drawn PER HIT: the west-coast drag,
    // where no two kicks are late by the same amount. A single number cannot
    // express it and a range applied once would just be a worse `offGridMs`.
    //
    // ⚠ It stays inside the grammar rather than moving to `humanize` for this
    // module's own stated reason: a hand being imprecise is humanize's, a
    // looseness the genre is *made of* is the grammar's.
    let sloppy_ms = pair(kick_block, "sloppyOffsetMs");

    let ghost = snare_block.and_then(|s| s.get("ghost"));
    let ghost_positions = strings(ghost, "pos");
    let clap_offset_ms = optional_number(snare_block, "layerClapOffsetMs", &mut snare_rng);

    // Boom-bap's rimshot under the snare — 231 models author it and nothing
    // played a note of it. It doubles the backbeat on `Lane::Rim`, which every
    // kit already has a pad for and GM already spells (37, Side Stick).
    //
    // ⛔ **`rimshotBelowBpm` is a second way to ask, not a gate on the first.**
    // `rnb-2000s` is the only model that authors it and it authors it *without*
    // `rimshotLayer` — a slow R&B record gets the rim and the same style at 100
    // does not — so reading it as a condition on a flag that model never wrote
    // would leave it as dead as it was.
    //
    // ⚠ **Read without touching the rng.** `flag` never does, and the tempo is
    // read as a plain number rather than through `optional_number`, so a model
    // that authors either key generates every other lane exactly as before.
    let rimshot = flag(snare_block, "rimshotLayer", false)
        || snare_block
            .and_then(|s| s.get("rimshotBelowBpm"))
            .and_then(Value::as_f64)
            .is_some_and(|below| f64::from(ctx.bpm) < below);

    // Country's cross-stick verse: in a verse the backbeat is played on the rim
    // instead of the drum, and the record opens up when the snare arrives for
    // the chorus. 74 models author it. Applied by [`finish`], at the end.
    let cross_stick =
        section == Some(SectionKind::Verse) && flag(snare_block, "crossStickVerses", false);

    // Pop and dance put the kick on all four in the hook and nowhere else — the
    // lift into the chorus that the section rule cannot express, because it is
    // about *which* beats rather than about how many.
    //
    // ⚠ **The model's own kicks are kept.** The four beats are added to the bar
    // rather than replacing it: a chorus that threw the grammar away would be
    // four-on-the-floor by every artist alike, and what makes a hook still sound
    // like the artist is the syncopation they play over the pulse.
    let four_on_floor = section == Some(SectionKind::Hook)
        && kick_block
            .and_then(|k| k.get("fourOnFloorInChorus"))
            .is_some_and(|value| match value {
                // ⚠ Two authoring forms in the corpus: 27 write `true`, two write
                // a probability. A probability here is a statement about how
                // often the style does it, and reading it as a per-song coin
                // toss would make the hook of a saved song depend on a draw the
                // kick's stream has never had to make. Anything above zero is
                // the style saying it does this.
                Value::Bool(on) => *on,
                Value::Number(n) => n.as_f64().is_some_and(|p| p > 0.0),
                _ => false,
            });

    // Jerk's headline marker, and it was authored by 19 models before anything
    // read it. Drawn per hit inside the bar loop, so a cluster is something that
    // happens to *a* backbeat rather than to the whole pattern.
    // ⛔ **Stays an `Option`, and the per-hit draw below is inside it.** Every
    // lane's stream is a seeded sequence, so an unconditional `random_bool` —
    // even one that can only answer `false` at probability zero — advances the
    // snare's rng for the 601 models that never authored a cluster and moves
    // every saved seed's beat. A new parameter must be free for the models that
    // do not use it.
    let cluster = optional_number(snare_block, "clusterProb", &mut snare_extra_rng).map(|p| {
        (
            p.clamp(0.0, 1.0),
            pair(snare_block, "clusterHits").unwrap_or((2.0, 3.0)),
        )
    });

    let bar_ticks = ctx.ticks_per_bar();
    // Kept per bar because the roll engine's `pre_snare` position needs
    // something to be before.
    let mut snares_by_bar: Vec<Vec<u32>> = Vec::with_capacity(usize::from(ctx.bars));
    // The kick's onsets before `sloppyOffsetMs` moves them — see the `hats` call.
    let mut kick_grid: Vec<u32> = Vec::new();

    for bar in 0..u32::from(ctx.bars) {
        let bar_start = bar * bar_ticks;
        let hits = placement.hits(bar, ctx);

        for (offset, articulation) in &hits {
            let tick = displace(bar_start + offset, off_grid_ticks);
            // A cluster replaces the hit rather than joining it: the middle note
            // still lands on the beat, so the backbeat has not moved — it has
            // been surrounded. Only an unarticulated hit clusters, because a
            // train beat's stream is already every 16th and bursting 32nds over
            // it would be clustering the whole part rather than its backbeat.
            let clustered = cluster
                .filter(|_| articulation.is_none())
                .filter(|(prob, _)| snare_extra_rng.random_bool(*prob));
            if let Some((_, hits)) = clustered {
                for note in snare_cluster(tick, hits, &mut snare_extra_rng) {
                    // ⚠ **The note ON the beat keeps the placement's own
                    // articulation and its tier velocity**, so to every reader —
                    // the off-grid gates, the fill histogram, the humanizer's
                    // tiers — the backbeat is exactly the hit it always was. The
                    // rest are marked [`Articulation::Cluster`]; its doc records
                    // the three gates that needed to tell the two apart.
                    let on_the_beat = note.start_tick == tick;
                    kit.hit(
                        Lane::Snare,
                        note.start_tick,
                        if on_the_beat {
                            tiers.pick(*articulation, &mut snare_rng)
                        } else {
                            note.vel
                        },
                        if on_the_beat {
                            *articulation
                        } else {
                            Some(Articulation::Cluster)
                        },
                    );
                }
            } else {
                kit.hit(
                    Lane::Snare,
                    tick,
                    tiers.pick(*articulation, &mut snare_rng),
                    *articulation,
                );
            }

            // Layered clap a few milliseconds off the snare — the trap sound is
            // the two together, and the offset is what stops them phasing.
            if let Some(ms) = clap_offset_ms {
                let clap = displace(tick, offset_ticks(ms, ctx));
                kit.hit(
                    Lane::Clap,
                    clap,
                    tiers.pick(*articulation, &mut snare_rng),
                    *articulation,
                );
            }
        }

        // The off-snare, on the snare's own stream and inside the bar loop for
        // the same reason the ghosts are: it is a positional rule per bar.
        off_snares(
            &mut kit,
            snare_block,
            ctx,
            &tiers,
            bar_start,
            off_grid_ticks,
            &mut snare_rng,
        );

        // Ghost snares: the drill "and-of-4" that answers the backbeat.
        let ghost_chance = number(ghost, "prob", 0.0, &mut snare_rng).clamp(0.0, 1.0);
        for position in &ghost_positions {
            if !snare_rng.random_bool(ghost_chance) {
                continue;
            }
            let Some(offset) = grid::position_ticks(position, ctx) else {
                continue;
            };
            kit.hit(
                Lane::Snare,
                displace(bar_start + offset, off_grid_ticks),
                ghost_velocity(ghost, &tiers, &mut snare_rng),
                Some(Articulation::Ghost),
            );
        }

        // The kick reads this bar's snares, so it can leave the gap before them.
        let snares: Vec<u32> = hits.iter().map(|(tick, _)| *tick).collect();
        snares_by_bar.push(snares.clone());
        for tick in kick_bar(
            kick_block,
            kick_form.as_ref(),
            ctx,
            bar,
            &snares,
            &mut kick_rng,
            &mut kick_extra_rng,
        ) {
            kick_grid.push(bar_start + tick);
            // ⚠ `displace(t, 0)` is `t`, so the un-authored case stays exact.
            let sloppy = sloppy_ms
                .map(|(lo, hi)| sloppy_rng.random_range(lo.min(hi)..=hi.max(lo)))
                .unwrap_or(0.0);
            let at = displace(bar_start + tick, offset_ticks(sloppy, ctx));
            // ⛔ **`velocityRange` is read on the kick and only on the kick.**
            // `rnb-2000s` is the one model in the corpus that authors it and it
            // authors it there; the same file spells `velocities: {main, ghost}`
            // and the ghost's `vel` correctly, which is what makes this a
            // mistake rather than a feature nobody had built. ⚠ **A second lane
            // wants a different shape, not another call here**: on the snare or
            // the hats a single band would *replace* `tiers.pick(articulation)`
            // and erase the accent/ghost distinction `VelocityTiers` exists for.
            // The shape for that is `VelocityTiers::for_lane`, overriding only
            // the `main` band — worth building when a model asks for it.
            let vel = fractional_velocity(kick_block, "velocityRange", &mut kick_rng)
                .unwrap_or_else(|| tiers.pick(None, &mut kick_rng));
            kit.hit(Lane::Kick, at, vel, None);

            // Boom-bap's sub layer: a second, quieter kick voice underneath the
            // first. `Lane::SubKick` has been in `LANE_ORDER` and had a GM voice
            // the whole time with nothing writing to it.
            if let Some(sub_vel) = fractional_velocity(kick_block, "subLayerVelocity", &mut sub_rng)
            {
                // ⚠ `separateLayerProb` is how often the layer is its *own* hit
                // rather than doubling the kick, so the sub is offset by an 8th
                // when it separates. Read only when the sub exists — a model
                // that authors neither must draw neither.
                let separate = optional_number(kick_block, "separateLayerProb", &mut sub_rng)
                    .is_some_and(|prob| sub_rng.random_bool(prob.clamp(0.0, 1.0)));
                let sub_at = if separate {
                    at.saturating_add(grid::ticks_per_beat(ctx) / 2)
                } else {
                    at
                };
                kit.hit(Lane::SubKick, sub_at, sub_vel, None);
            }
        }
    }

    // Pop and dance put the kick on all four in the hook — the lift into the
    // chorus that a section rule cannot express, because it is about *which*
    // beats rather than about how many.
    //
    // ⛔⛔ **On its own stream, and outside the bar loop, because inside it the
    // key rewrote the model.** A pulse pushed into the bar's tick list drew its
    // velocity from `kick_rng` like any other hit — and `kick_rng` is what
    // `kick_bar` reads for every *later* bar, so a hook gained four kicks in
    // bar 1 and a different grammar from bar 2 onward. The claim this key makes
    // is that the model's own kicks are kept; that is only true if the pulses
    // cost the kick nothing.
    //
    // ⚠ **Checked against `kick_grid`, which holds the grid positions before
    // `sloppyOffsetMs` moved anything.** A dragged kick sits a few ticks off its
    // own beat, so comparing against where the notes *landed* would put a
    // second kick beside every dragged one.
    //
    // ⚠ **A kick on every beat of the bar, whatever the meter.** That is what
    // four-on-the-floor generalises to — the pulse is the beat — so a 7/4
    // session gets seven and a 6/8 session six. Every model that authors the key
    // is a 4/4 style; the meter is the producer's, and this is what it does.
    if four_on_floor {
        let mut floor_rng = rng::stream(seed, "drums/kick/fourOnFloor");
        let beat = grid::ticks_per_beat(ctx);
        let mut pulses: Vec<u32> = Vec::new();
        for bar in 0..u32::from(ctx.bars) {
            for pulse in 0..u32::from(ctx.time_sig_num) {
                let on = bar * bar_ticks + pulse * beat;
                if !kick_grid.contains(&on) {
                    pulses.push(on);
                }
            }
        }
        for on in pulses {
            kit.hit(Lane::Kick, on, tiers.pick(None, &mut floor_rng), None);
            kick_grid.push(on);
        }
        kick_grid.sort_unstable();
    }

    // Hats are built across the whole pattern rather than bar by bar: the
    // subdivision, the pitch-bent layer and the swell are all decisions about
    // the part, not about a bar.
    // Fills before the 808, so the 808 sees the snare picture it will actually
    // have to make room for.
    let mut fill_rng = rng::stream(seed, "drums/fills");
    fills(&mut kit, drums, ctx, &mut fill_rng);

    let hihat = block(drums, "hihat");
    let mut hat_rng = rng::stream(seed, "drums/hats");
    // ⛔ **Collected once, here, and read again by the 808 below.** Nothing
    // between the two writes to `Lane::Kick` — `fills` writes the snare and the
    // clap, and `percs` is forbidden the kick by `PERC_LANES`' own rule — so the
    // "both lanes finished" the 808 needs is already true of this one. A `Vec`
    // rather than a slice because `kit` is borrowed mutably by everything in
    // between.
    let kicks: Vec<u32> = kit.notes(Lane::Kick).iter().map(|n| n.start_tick).collect();
    // ⛔⛔ **The MIRRORED hat takes the grid positions, not these.** An open hat
    // lands on the grid and removes the closed hit underneath it by exact tick —
    // `close_over_open`'s rule, "one hi-hat cannot be open and shut at the same
    // instant". `west-coast-club` authors `mirrorsKick` *and*
    // `kick.sloppyOffsetMs: [3, 9]`, so a hat mirroring the drag would sit 5–14
    // ticks off the open hat it belongs under, miss the exclusion, and export GM
    // 42 and GM 46 together — the exact defect that rule exists to prevent.
    //
    // ⚠ It is also the truer reading: the drag is one hand being imprecise on
    // the kick, and the hat is the other hand playing the same rhythm.
    let (mut closed, mut open) = hats(
        hihat,
        ctx,
        &tiers,
        &kick_grid,
        &mut hat_rng,
        &mut hat_extra_rng,
    );

    // ⛔⛔ **A MODEL THAT DECLARES A HAT BLOCK HAS TO PRODUCE HATS**, and the
    // hat stream alone is redrawn until it does — Mike, 2026-08-12: *"redo just
    // the hihat generation until you get hihats and they are not a blank row for
    // the genres you need, not the entire generation."*
    //
    // ⛔ **The cause is the density doing exactly what it says.** A
    // non-continuous stream spends `fillDensity` on *whether a beat plays at
    // all*, so at `clipse`'s 0.28 a bar is silent 0.72⁴ ≈ 27% of the time and
    // all four bars are silent about once in 190 seeds. **80 models can land on
    // it and one is a shipped genre**: `plugg` authors 0.25, which empties a
    // four-bar pattern about once in a hundred — a hatless beat roughly every
    // hundredth press of Generate, with nothing on screen to explain it. Raising
    // `clipse`'s number would have turned the gate green and left the other 79.
    //
    // ⚠ **Only this stream is redrawn**, which is why the retry sits here rather
    // than around `generate`. Every stage already has its own seeded stream so
    // that "rerolling the snare cannot move the kick"; redrawing the whole take
    // would throw away a kick and snare the producer would otherwise have had,
    // to fix a hat part. Injecting a hat instead was the first attempt and was
    // worse — it invents a hit the model never asked for and makes every sparse
    // style quietly less sparse.
    //
    // ⚠ **Still a pure function of its inputs**: the redraw domains are derived
    // and spelled out, so one seed always walks the same chain and a saved seed
    // rebuilds the pattern the producer heard. This is the convention
    // `novelty::RETRY_DOMAINS` already sets for a rejected melody.
    //
    // ⚠ **The closed hat specifically.** An open hat *replaces* the closed hit
    // underneath it — one hi-hat cannot be open and shut at one instant — so
    // asking whether "some hat exists" would accept a take carrying nothing but
    // the two open hats that ate their own stream.
    if hihat.is_some() {
        for domain in HAT_REDRAW_DOMAINS {
            if !closed.is_empty() {
                break;
            }
            let mut redraw = rng::stream(seed, domain);
            (closed, open) = hats(
                hihat,
                ctx,
                &tiers,
                &kick_grid,
                &mut redraw,
                &mut hat_extra_rng,
            );
        }
    }

    // Hat rolls schedule themselves from the model's own `positions` and
    // `freqPerBar`, so they belong to the hat part rather than to the fill
    // logic. On their own stream, so changing a roll parameter cannot shift
    // the stream around it.
    let mut roll_rng = rng::stream(seed, "drums/hatRolls");
    rolls::hat_rolls(&mut closed, hihat, ctx, &snares_by_bar, &mut roll_rng);

    // The hat's *fill* — the phrase-end figure that hands one bar to the next
    // (TASK-043H). ⛔ **After the rolls, and on its own stream.** After,
    // because a fill's whole job is to break the stream and it has to be able
    // to break a roll too; on its own stream, so re-rolling the fill cannot
    // move the hats it interrupts.
    let mut hat_fill_rng = rng::stream(seed, "drums/hats/fill");
    rolls::hat_fills(&mut closed, hihat, ctx, &mut hat_fill_rng);

    // ⛔⛔ **The open-hat exclusion, re-applied once both decorators are done.**
    // `hats()` deletes the closed hit underneath every open hat it places — one
    // hi-hat cannot be open and shut at the same instant — and then two things
    // write fresh closed notes over the top of that decision: `hat_rolls` puts
    // a roll wherever its positions land, and `hat_fills` clears a whole window
    // and redraws it. Either one could put a closed hat straight back on an
    // open hat's tick, and GM 42 and 46 firing together is not a hat sound, it
    // is two of them ringing over each other.
    //
    // ⛔ **Here rather than inside each of them, and that is the point.** The
    // rule belongs to the hat lane, not to any one thing that decorates it —
    // installed at each door, the *next* decorator would arrive without it, the
    // way the fill did. `no_hat_is_open_and_shut_at_the_same_instant` holds it
    // over every shipped model and every seed for the same reason.
    close_over_open(&mut closed, &open);

    // The 808 last, because it rides the kick and stops for the snare — it
    // needs both lanes finished before it can be placed. The kick's were taken
    // above, where the hats needed them; the snare's have to be taken here,
    // because `fills` writes to that lane in between.
    // The backbeat, not every snare note: a fill is a wall of them, and muting
    // the 808 under each one would shred the line instead of clearing the way
    // for the hit that matters.
    let snares: Vec<u32> = kit
        .notes(Lane::Snare)
        .iter()
        .filter(|n| {
            !matches!(
                n.articulation,
                Some(Articulation::Ghost) | Some(Articulation::Roll)
            )
        })
        .map(|n| n.start_tick)
        .collect();
    // ⛔ **Every snare, fills included, for the length clamp only.** The list
    // above is what the 808 *skips*, and it is the backbeat on purpose. This is
    // what it must not *ring through*, and the two are genuinely different
    // rules: a fill is a wall of snares, so dropping the 808 under each one
    // shreds the line — but letting it sustain across the whole wall is the
    // thing drill is defined against.
    //
    // ⚠ **They only ever agreed by luck, and TASK-131C's data change broke the
    // luck.** `drills_808_stops_under_the_snare` filters ghosts and *not* rolls,
    // so it has always asserted this stricter rule — it passed because no kick
    // grammar had yet put an 808 across a fill. The moment uk-drill gained a
    // variant with a kick on beat 4, seed 0 rang an 808 from 6720 through the
    // roll note at 6960. The test was right and the code was one list short.
    let ringing_snares: Vec<u32> = kit
        .notes(Lane::Snare)
        .iter()
        .filter(|n| n.articulation != Some(Articulation::Ghost))
        .map(|n| n.start_tick)
        .collect();
    let mut bass_rng = rng::stream(seed, "drums/bass808");
    kit.extend(
        Lane::Sub,
        bass808(
            drums,
            ctx,
            Around {
                kicks: &kicks,
                snares: &snares,
                ringing: &ringing_snares,
            },
            &mut bass_rng,
            &mut sub_tone_rng,
            flag(
                model.blocks.get("arrangement"),
                "melodic808SlideAtLoopEnd",
                false,
            ),
        ),
    );
    kit.extend(Lane::ClosedHat, closed);
    kit.extend(Lane::OpenHat, open);

    // Percussion last, on its own stream, so adding a shaker to a model cannot
    // move a single kick, snare, hat or 808 note in any pattern that already
    // exists. ⚠ That is the property that makes TASK-140 safe to land before
    // the roster: 15 models already author `percs`, and if this stage shared a
    // stream with anything above it, reading their block for the first time
    // would silently rewrite every beat they produce.
    //
    // It runs after the clap exists because `tambourineMirrorsClap` reads it.
    let mut perc_rng = rng::stream(seed, "drums/percs");
    percs(&mut kit, drums, ctx, &tiers, &mut perc_rng);

    // ⛔ **The kit had no clip boundary at all**, so a hit on the final 16th — a
    // `4a` ghost nudged late by `offGridMs`, a tambourine on the last
    // subdivision — carried its length past the end of the pattern. Found only
    // by widening `coherence::combinations` from twenty (model, seed) pairs to
    // every model across eight seeds; the model it caught first,
    // `west-coast-club`, had shipped that way the whole time.
    //
    // ▶ [`super::fit_to_clip`] is the seam every generator goes through, rather
    // than a fifth hand-written copy of the same rule. ⚠ The alternative was
    // deleting authored ghost and percussion positions out of four models to
    // work around one missing clamp — fixing the data to suit a defect in the
    // code.
    finish(
        &mut kit,
        Finish {
            rimshot,
            cross_stick,
            locked_backbeat: flag(snare_block, "lockedBackbeat", false),
            detune: optional_number(snare_block, "detuneSemis", &mut snare_tone_rng)
                .map(|semis| semis.clamp(-24.0, 24.0).round() as i16)
                .filter(|semis| *semis != 0),
        },
        &tiers,
        &mut rimshot_rng,
    );

    let mut lanes = kit.into_lanes();
    super::fit_to_clip(&mut lanes, ctx);
    lanes
}

/// What the four keys about the *finished* snare lane ask for.
struct Finish {
    rimshot: bool,
    cross_stick: bool,
    locked_backbeat: bool,
    /// Semitones, already known to be non-zero.
    detune: Option<i16>,
}

/// The passes that rewrite a lane once nothing else will write to it.
///
/// ⛔ **Every one of these is a fact about the finished lane, and none can be
/// spelled at [`DrumKit::hit`]** — `fills::clear_for_fill` takes hits away after
/// they are placed, so a rim written under a backbeat during the bar loop would
/// be doubling a hit the drummer has stopped playing.
///
/// ⛔⛔ **And they run after `percs` and after the 808, because the 808 reads the
/// snare lane.** The cross-stick moves the backbeat off [`Lane::Snare`]; done any
/// earlier, `bass808`'s `snares` and `ringing_snares` come back without it and
/// the 808 rings straight through a backbeat it is supposed to stop for. A pass
/// placed too early is a defect in a lane that never mentions it, which is
/// exactly the class of bug this file keeps a register for.
fn finish(kit: &mut DrumKit, ask: Finish, tiers: &VelocityTiers, rimshot_rng: &mut impl Rng) {
    // The rimshot layer doubles the backbeat on its own voice, on the beat
    // rather than beside it: a rimshot under a snare is one drum struck two
    // ways, and the millisecond offset the clap gets exists to stop two
    // *different* samples phasing.
    //
    // ⛔ **Its own stream, and every draw is inside the `if`.** A velocity taken
    // from the snare's stream would advance its sequence, and this module has
    // already paid for that once: a sub-kick velocity on the kick's stream moved
    // every later bar's grammar and failed a bassline gate two lanes away.
    //
    // ⚠ The main hits only. A ghost is the snare answering itself and a roll is
    // the fill's own vocabulary; doubling either would be a rim part, and what
    // 231 models asked for is a rim *under the backbeat*.
    if ask.rimshot {
        let backbeats: Vec<u32> = main_hits(kit.notes(Lane::Snare))
            .map(|note| note.start_tick)
            .collect();
        for tick in backbeats {
            kit.hit(Lane::Rim, tick, tiers.pick(None, rimshot_rng), None);
        }
    }

    // `lockedBackbeat` — the drum-and-bass rule that 2 and 4 do not move while
    // everything around them does. Five models author it; `jungle` authors it
    // `false`, which is the genre saying the opposite out loud.
    //
    // ⛔ **A mark for `humanize`, not a displacement of its own.** All five
    // authors also write `offGridMs: 0`, so there is nothing in the grammar left
    // to exempt the backbeat from — what moves it is the session's timing
    // jitter, read per lane with no way for one note to opt out.
    // `Note::timing_locked` is that way out, and its doc records why the lane
    // cannot be the unit.
    if ask.locked_backbeat {
        for note in kit.notes_mut(Lane::Snare) {
            note.timing_locked = note.articulation.is_none();
        }
    }

    // `detuneSemis` — the snare tuned off the pitch its sample was recorded at.
    // Seven models, `trap` among them at `[-2, -1]`: the deep, detuned trap
    // snare is a sound, and it was authored long before anything read it.
    //
    // ⛔ **The register files this under the 808 and it is a `drums.snare` key.**
    // Written into the 808's tone the value would have detuned a *bassline*,
    // which is a wrong note rather than a timbre — resolve a key's real path out
    // of `data/` before writing a line of code for it.
    //
    // ⚠ **An audio-domain effect, exactly like `pitchWalk`** (TASK-131D). The
    // sampler and the rendered stems repitch the pad from the lane's own GM note,
    // so a number below 38 reads as "the snare, tuned down"; MIDI — live and
    // exported — still carries GM 38, because in a `.mid` a drum's note number
    // *is* which drum it is. Added to each note's own pitch, so a roll that walks
    // keeps its walk and the whole figure moves with the tuning.
    if let Some(semis) = ask.detune {
        for note in kit.notes_mut(Lane::Snare) {
            note.pitch = (i16::from(note.pitch) + semis).clamp(0, 127) as u8;
        }
    }

    // The cross-stick verse, last of all: the backbeat *moves* to the rim rather
    // than being doubled by it. That is what a cross-stick is — the same stroke
    // played differently — and it is what makes the verse quieter than the
    // chorus it builds into.
    //
    // ⚠ The placement's own hits only. A ghost is a snare stroke and a fill is
    // the fill's vocabulary; a drummer holding the stick sideways still plays
    // both on the head.
    //
    // ⚠ **The note travels rather than being re-struck**, so a locked backbeat
    // stays locked and its velocity is the one the tiers drew. A tick the rim is
    // already playing — the layer above, on a model that authors both — keeps the
    // rim it has, because one voice cannot sound twice at one instant.
    if ask.cross_stick {
        let taken: Vec<u32> = kit
            .notes(Lane::Rim)
            .iter()
            .map(|note| note.start_tick)
            .collect();
        let mut moved: Vec<Note> = Vec::new();
        kit.notes_mut(Lane::Snare).retain(|note| {
            if note.articulation.is_some() {
                return true;
            }
            // ⚠ `moved` is checked as well as `taken`: `extend` is the one
            // door into a lane that does *not* refuse a second note on one
            // tick, and `no_lane_ever_carries_two_notes_on_the_same_tick`
            // holds over every model and seed.
            if !taken.contains(&note.start_tick)
                && !moved.iter().any(|m: &Note| m.start_tick == note.start_tick)
            {
                let mut moving = note.clone();
                moving.pitch = gm_drum_note(Lane::Rim);
                moved.push(moving);
            }
            false
        });
        kit.extend(Lane::Rim, moved);
    }
}

/// The hits a placement wrote — not a ghost, not a roll, not a cluster ornament.
fn main_hits(notes: &[Note]) -> impl Iterator<Item = &Note> {
    notes.iter().filter(|note| note.articulation.is_none())
}

/// A velocity authored as a fraction of full scale, e.g. `[0.8, 1.0]`.
///
/// Lanes state their own scale where the research measured one — drill's ghost
/// snare at 40–50%, hat mains at 80–100% — and a specific number beats the
/// cross-genre tier when a model bothered to write it down.
fn fractional_velocity(block: Option<&Value>, key: &str, rng: &mut impl Rng) -> Option<u8> {
    optional_number(block, key, rng)
        .map(|fraction| ((fraction * 127.0).round()).clamp(1.0, 127.0) as u8)
}

/// A ghost's velocity: the fraction the model states, or the ghost tier.
fn ghost_velocity(ghost: Option<&Value>, tiers: &VelocityTiers, rng: &mut impl Rng) -> u8 {
    fractional_velocity(ghost, "vel", rng)
        .unwrap_or_else(|| tiers.pick(Some(Articulation::Ghost), rng))
}

/// The hat stream's skeleton for one bar, in ticks from the bar's start.
///
/// Either a plain subdivision — `"8th"`, `"16th"` — or `"tresillo"`, where the
/// onsets follow the authored grouping in 16ths and repeat until the bar is
/// full. Drill's `[3, 3, 2]` sums to half a bar, so it lands twice.
fn hat_base_onsets(base: &str, grouping: &[u32], ctx: &SessionContext) -> Vec<u32> {
    if base == "tresillo" {
        let grouping: Vec<u32> = grouping.iter().copied().filter(|g| *g > 0).collect();
        // A grouping of all zeros would never advance. Fall back to the 3-3-2
        // the name means rather than looping forever.
        let grouping = if grouping.is_empty() {
            vec![3, 3, 2]
        } else {
            grouping
        };

        let mut onsets = Vec::new();
        let mut cursor = 0;
        let total = grid::sixteenths_per_bar(ctx);
        for step in grouping.iter().cycle() {
            if cursor >= total {
                break;
            }
            onsets.push(cursor * grid::SIXTEENTH);
            cursor += step;
        }
        return onsets;
    }

    let step = grid::note_value_ticks(base).unwrap_or(grid::SIXTEENTH * 2);
    (0..ctx.ticks_per_bar())
        .step_by(step.max(1) as usize)
        .collect()
}

/// Is this position one the hand accents — on the 8th-note grid?
///
/// The main/ghost split in a hat stream is positional, not random: the beats
/// and the "&"s carry the pulse and the 16ths between them fill it in
/// (research ch. 1 §1, mains 80–100% against ghosts 40–60%).
///
/// ⛔⛔ **Deliberately NOT `is_downbeat || is_offbeat_eighth`, and writing it
/// that way silently deleted every ghost hat outside 4/4.** Those two predicates
/// became meter-aware (TASK-142's grid fix), and their union covers *every* 16th
/// the moment a beat is two 16ths or fewer: in 6/8 `is_downbeat` is `i % 2 == 0`
/// and `is_offbeat_eighth` is `i % 2 == 1`. So every hat answered "main", the
/// `Articulation::Ghost` tier became unreachable, and every model's authored
/// `hihat.velocities.ghost` band went with it — a machine-gun hat at one flat
/// velocity in 6/8, 12/8 and every x/16 meter. 4/4 was unaffected, which is why
/// nothing caught it.
///
/// ⚠ **The 8th-note grid is the right unit here and it does not depend on the
/// meter.** A 16th is a 16th and an 8th is an 8th whatever the time signature
/// says — that is [`grid::SIXTEENTH`]'s own note — and what the research
/// describes is a hand alternating down-up on 8ths with the in-between 16ths
/// filled in quietly. In 6/8 the 8th *is* the beat, so the ghosts are the 16ths
/// between the beats, which is exactly what this now answers.
fn is_main_position(tick: u32) -> bool {
    (tick / grid::SIXTEENTH).is_multiple_of(2)
}

/// Resolve an open-hat position, including the symbolic `"_pre"` form.
///
/// `"1_pre"` is "just before the downbeat" (research ch. 1 §3, rage) — one 16th
/// early, which for beat 1 means the last 16th of the *previous* bar. In the
/// first bar there is no previous bar, so it is dropped rather than wrapped
/// around to the end of the pattern.
fn open_hat_tick(position: &str, bar_start: u32, ctx: &SessionContext) -> Option<u32> {
    match position.strip_suffix("_pre") {
        Some(base) => {
            let offset = grid::position_ticks(base, ctx)?;
            (bar_start + offset).checked_sub(grid::SIXTEENTH)
        }
        None => Some(bar_start + grid::position_ticks(position, ctx)?),
    }
}

/// Fills: the variation events that mark a phrase boundary.
///
/// Consensus formula #20 — a small variation every two bars, a bigger one every
/// eight, and the densest bars are the ones that close a phrase. That is what
/// makes four bars sound like a loop rather than four copies of one bar.
///
/// A fill **takes** the stretch it lands in: the ladder replaces the backbeat
/// in its bar rather than playing over it, which is what a drummer does.
fn fills(kit: &mut DrumKit, drums: Option<&Value>, ctx: &SessionContext, rng: &mut impl Rng) {
    let fills = block(drums, "fills");
    if fills.is_none() {
        return;
    }

    let small_every = number(fills, "smallEveryBars", 2.0, rng).round().max(1.0) as u32;
    let big_every = number(fills, "bigEveryBars", 8.0, rng).round().max(1.0) as u32;
    // The flag exists so a pattern ends *into* whatever comes next rather than
    // stopping dead at the loop point.
    let before_section = flag(fills, "fillBeforeSection", true);
    let use_ladder = flag(fills, "snareRollLadder", false);
    // The lane a fill turns over on. West-coast club uses the clap — a named
    // Mustard-era device — and naming the lane rather than adding a bool per
    // genre means the next one that fills on a tom costs no code.
    let lane = text(fills, "lane")
        .and_then(lane_by_name)
        .unwrap_or(Lane::Snare);

    let snare_roll = block(drums, "snareRoll");
    let bar_ticks = ctx.ticks_per_bar();
    let beat = grid::ticks_per_beat(ctx);
    let bars = u32::from(ctx.bars);

    for bar in 0..bars {
        let position = bar + 1;
        let last_bar = position == bars;
        let big = position.is_multiple_of(big_every);
        let small = position.is_multiple_of(small_every) || (last_bar && before_section);

        if !big && !small {
            continue;
        }

        // Both sit at the *end* of the bar — "a tom/roll fill on the last
        // 16ths" — so the backbeat keeps its identity in every bar and only
        // the run-up to the next one is given away. A fill that swallowed the
        // whole bar would delete the thing it is leading out of.
        let bar_start = bar * bar_ticks;
        let beats = if big { 2 } else { 1 };
        let length = (beat * beats).min(bar_ticks);
        let start = bar_start + bar_ticks - length;

        kit.clear_for_fill(lane, start..(start + length));
        let notes = if big && use_ladder {
            rolls::snare_ladder(snare_roll, ctx, lane, start, length, rng)
        } else {
            // ⛔ **Was a hardcoded `Roll::new(..).ramp(64,120)` written inline
            // here, and that was the defect Mike reported on 2026-08-05: six of
            // the ten flagship trap artists wrote a byte-identical roll.**
            // `rolls::snare_fill` reads the artist's own block and samples
            // inside it; its doc comment carries the measurement.
            rolls::snare_fill(snare_roll, lane, start, length, rng)
        };

        // The ghosts `clear_for_fill` keeps live on the same 16th grid the
        // fill is written on — `"4&"` is 3360, and every non-backbeat 16th of
        // a train beat is one — so the roll landed on the exact tick a ghost
        // already occupied. Two note-ons on one key at one tick is the
        // collision `midi::pattern_to_smf` already calls "the one the note-off
        // pairing cannot survive": the second off is orphaned and the hit
        // doubles. Eleven of the fifteen genres produced these.
        //
        // The fill yields, because the ghost is the thing being played over.
        let taken: Vec<u32> = kit.notes(lane).iter().map(|n| n.start_tick).collect();
        kit.extend(
            lane,
            notes
                .into_iter()
                .filter(|note| !taken.contains(&note.start_tick)),
        );
    }
}

/// What the rest of the kit already plays, which is what the 808 fits around.
///
/// ⚠ **Three lists rather than one**, because the 808 asks three different
/// questions of the beat and the answers genuinely differ. Grouped into a struct
/// because the alternative was an eighth positional argument to `bass808`, and
/// the register still has 808 keys queued behind this one.
struct Around<'a> {
    /// The rhythm the 808 rides.
    kicks: &'a [u32],
    /// The backbeat: where the 808 does not play at all.
    snares: &'a [u32],
    /// Every snare including a fill's, which the 808 may start on but must not
    /// ring past. See the call site for why these are two lists.
    ringing: &'a [u32],
}

/// The 808 line (research ch. 1 §1 trap, §2 drill).
///
/// The 808 is not a bass part that happens to be low — in these genres it *is*
/// the low end and the kick is its transient, which is why its rhythm comes
/// from the kick lane (`kick.lockTo808`) rather than from a rhythm of its own.
///
/// Three rules make it sound like an 808 rather than a synth bass:
///
/// - **Legato**: every note runs to the next one. A gap between 808 notes is
///   audible as a hole in the record.
/// - **Slides are overlapping notes.** `slide_to_pitch` says where the note
///   glides; `midi::pattern_to_smf` writes the overlap the sampler reads as
///   portamento. That is the FL convention the research documents.
/// - **Mono, cut-self**: two 808s at once is a mix problem, so notes never
///   overlap except across a slide.
fn bass808(
    drums: Option<&Value>,
    ctx: &SessionContext,
    around: Around<'_>,
    rng: &mut impl Rng,
    // The 808's *tone* parameters, which are settings on the instrument rather
    // than choices the player makes — so they draw from their own stream and
    // the line itself comes out note for note as it did before.
    tone: &mut impl Rng,
    // `arrangement.melodic808SlideAtLoopEnd` — an arrangement key about this
    // lane, so it arrives as a decision rather than as a block to read.
    slide_at_loop_end: bool,
) -> Vec<Note> {
    let Around {
        kicks,
        snares,
        ringing: ringing_snares,
    } = around;
    // `read::block` treats an explicit `null` as absent, which is how a
    // country kit or a boom-bap break says it has no 808 at all.
    let block = block(drums, "bass808");
    if block.is_none() {
        return Vec::new();
    }
    if kicks.is_empty() {
        // The 808 rides the kick. With no kick there is nothing to ride, and
        // inventing a rhythm here would be a bassline, not an 808.
        return Vec::new();
    }

    // How much of the kick the 808 follows. Authored on the *kick* because it
    // describes how tightly the two are locked (trap 1.0 — "one instrument
    // played twice"; drill 0.6 — the 808 goes its own way more often).
    let kick = crate::generators::read::block(drums, "kick");
    let lock = number(kick, "lockTo808", 1.0, rng).clamp(0.0, 1.0);

    let (low, high) = pair(block, "register")
        .map(|(lo, hi)| (lo as u8, hi as u8))
        .unwrap_or((17, 43));

    let Some(root) = theory::pitch_class_in_register(ctx.key_root, low, high) else {
        return Vec::new();
    };

    // A counter-riff keeps whatever pitch it slid to until the next phrase; a
    // bassline returns to the root. That difference is most of what separates
    // drill's 808 from trap's (research ch. 1 §2: "counter-riff in 5ths, b7s
    // and octaves rather than doubling the roots").
    let counter_riff =
        text(block, "role").and_then(Bass808Role::parse) == Some(Bass808Role::CounterRiff);

    // Authored either as a plain list or as a weighted choice. Sampling the
    // weighted form repeatedly turns it into a list whose *proportions* carry
    // the weights, so one code path picks from either.
    let mut intervals: Vec<String> = strings(block, "slideIntervals");
    if intervals.is_empty() {
        intervals = (0..24)
            .filter_map(|_| string_spec(block, "slideIntervals", rng))
            .collect();
    }

    let slide_chance = number(block, "slideProb", 0.3, rng).clamp(0.0, 1.0);
    let down_glide = number(block, "longDownGlideProb", 0.0, rng).clamp(0.0, 1.0);
    let positions: Vec<RollLikePosition> = strings(block, "slidePositions")
        .iter()
        .filter_map(|p| RollLikePosition::parse(p))
        .collect();

    let bar_ticks = ctx.ticks_per_bar();
    let mute = flag(block, "muteUnderSnare", false);

    // ⛔⛔ **The 808 can only ever SUBTRACT from the kick, and this is the
    // authored way out.** The line below filters the kick's own onsets, so
    // `lockTo808` at 0.6 does not give an 808 that goes its own way four times
    // in ten — it gives one that plays four fewer notes in the same places. A
    // sub cannot syncopate against a kick it is a subset of, and 15 models
    // spell `melodicallyIndependent: true` asking for exactly that.
    //
    // ⛔⛔ **The count is the LOCKED count, not the kick's, and the difference is
    // a genre.** Independence is about *where* the notes are; how many of them
    // there are is what `lockTo808` already said, and 15 models say it. Taking
    // the kick's own count instead made uk-drill's 808 40% busier at a stroke —
    // 5 notes to 7 in the four-bar golden — which is a change to how the genre
    // sounds that no gate in this repo can hear. The lock is drawn either way,
    // so the line has exactly the density it always had, in its own places.
    //
    // ⚠ Drawing the lock on both paths is also what keeps the ordinary path byte
    // for byte what it was: same draws, same order, same notes.
    let independent = flag(block, "melodicallyIndependent", false);
    let locked: Vec<u32> = kicks
        .iter()
        .copied()
        .filter(|_| rng.random_bool(lock))
        .collect();

    let kept: Vec<u32> = if independent {
        independent_onsets(ctx, locked.len(), rng)
    } else {
        locked
    }
    .into_iter()
    // "Mutes at snare hits" means the 808 does not play there at all —
    // not that it plays and is cut to a click. Dropped *here*, before
    // slides are chosen, so a slide is never handed to a note that is
    // about to disappear: doing it the other way round silently cost UK
    // drill a third of the slides its model asks for.
    .filter(|tick| {
        !mute
            || !snares
                .iter()
                .any(|snare| snare.abs_diff(*tick) <= MUTE_TOLERANCE)
    })
    .collect();

    // Which of those may slide, by the model's positions.
    let eligible: Vec<usize> = kept
        .iter()
        .enumerate()
        .filter(|(_, tick)| {
            let bar = *tick / bar_ticks;
            positions.iter().any(|p| p.covers(bar, u32::from(ctx.bars)))
        })
        .map(|(i, _)| i)
        .collect();

    // Drill states a *count* — "2–3 slides per 4 bars" — and trap states a
    // chance per opportunity. A count is the stronger claim, so when a model
    // gives one it is met from the eligible positions rather than approximated
    // by rolling the dice at each of them.
    let mut sliding: Vec<usize> = Vec::new();
    match optional_number(block, "slidesPer4Bars", rng) {
        Some(per_phrase) => {
            let phrase = bar_ticks * 4;
            let phrases = u32::from(ctx.bars).div_ceil(4).max(1);
            for phrase_index in 0..phrases {
                let window = (phrase_index * phrase)..((phrase_index + 1) * phrase);
                let mut candidates: Vec<usize> = eligible
                    .iter()
                    .copied()
                    .filter(|i| window.contains(&kept[*i]))
                    .collect();

                // The positions say where a slide *prefers* to land; the count
                // says how many there are. When the preferred bars cannot
                // supply the count — drill's kick grammar leaves one usable
                // note in each of them, against an authored two to three — the
                // rest come from the other notes in the phrase, latest first,
                // because a slide is an end-of-phrase gesture.
                let wanted = per_phrase.round().max(0.0) as usize;
                if candidates.len() < wanted {
                    let mut rest: Vec<usize> = (0..kept.len())
                        .filter(|i| window.contains(&kept[*i]) && !candidates.contains(i))
                        .collect();
                    rest.sort_by_key(|i| std::cmp::Reverse(kept[*i]));
                    candidates.extend(rest.into_iter().take(wanted - candidates.len()));
                }

                let wanted = wanted.min(candidates.len());
                for _ in 0..wanted {
                    let choice = rng.random_range(0..candidates.len());
                    sliding.push(candidates.remove(choice));
                }
            }
        }
        None => {
            sliding = eligible
                .iter()
                .copied()
                .filter(|_| rng.random_bool(slide_chance))
                .collect();
        }
    }

    let mut notes: Vec<Note> = Vec::new();
    let mut pitch = root;

    for (index, tick) in kept.iter().enumerate() {
        let mut slide_to = None;
        if sliding.contains(&index) && !intervals.is_empty() {
            let name = &intervals[rng.random_range(0..intervals.len())];
            if let Some(semitones) = theory::interval_semitones(name) {
                let direction = if rng.random_bool(down_glide) { -1 } else { 1 };
                let target = i16::from(pitch) + i16::from(semitones) * direction;
                // A slide may reach an octave above the *root* — never above
                // wherever the line has already climbed to.
                //
                // `register` says where the line sits, not how far a gesture
                // may travel: UK drill authors `[24, 28]`, four semitones, and
                // folding a fifth back into that is impossible, so an octave of
                // headroom is real. But measuring it from the *running* pitch
                // let a counter-riff ratchet — each slide raised the note and
                // the ceiling together, so `fold_into_register` could never
                // bring it back down and uk-drill walked 24 → 31 → 38 → 50 →
                // 60 → 70. MIDI 70 is three and a half octaves above the
                // authored ceiling: a lead, not an 808. 28% of its notes ended
                // up there.
                //
                // Anchored to the root, the fold always has somewhere to land.
                // For a bassline this changes nothing — its pitch *is* the root
                // on every note.
                let ceiling = high.max(root.saturating_add(12));
                slide_to = theory::fold_into_register(target, low, ceiling)
                    // A slide onto the pitch it is already on is not a slide;
                    // the writer would collapse it back to one note anyway.
                    .filter(|target| *target != pitch);
            }
        }

        notes.push(Note {
            model_vel: None,
            start_tick: *tick,
            // Filled in by the legato pass below; a length of zero here would
            // survive as a zero-length note if that pass ever stopped running.
            len_ticks: grid::SIXTEENTH,
            pitch,
            vel: 100,
            slide_to_pitch: slide_to,
            slide_ms: None,
            slide_overlap_ticks: None,
            timing_locked: false,
            articulation: Some(Articulation::Legato),
            reversed: false,
        });

        pitch = match slide_to {
            // The riff stays where it landed; the bassline goes home.
            Some(target) if counter_riff => target,
            _ => root,
        };
    }

    // The turnaround — `arrangement.melodic808SlideAtLoopEnd`. The loop's last
    // note glides back into the pitch it opens on, so four bars lead into
    // themselves instead of stopping dead. `uk-drill` is the one author.
    //
    // ⛔ **It moves a slide; it never adds one.** The same model authors
    // `slidesPer4Bars: [2, 3]`, and `drill_slides_two_to_three_times_every_four_bars`
    // calls that ceiling absolute — "a fourth would be a different genre". This
    // key is a statement about *where* the gesture lands, so when the last note
    // is not already sliding it takes the last slide in the pattern with it.
    //
    // ⚠ **Before the two passes below**, so the whoop can still turn it: an
    // octave up out of the loop's last note is drill's own signature and the
    // turnaround must not be the thing that silences it.
    if slide_at_loop_end && notes.len() > 1 {
        let opening = notes[0].pitch;
        let last = notes.len() - 1;
        // A slide onto the pitch the note is already on is not a slide, and
        // moving one there would delete a gesture rather than place it.
        if notes[last].pitch != opening {
            if notes[last].slide_to_pitch.is_some() {
                notes[last].slide_to_pitch = Some(opening);
            } else if let Some(from) = (0..last).rev().find(|i| notes[*i].slide_to_pitch.is_some())
            {
                notes[from].slide_to_pitch = None;
                notes[last].slide_to_pitch = Some(opening);
            }
        }
    }

    // Rage's chromatic approach: the note *before* a target drops a semitone
    // and glides up into it. Written over the finished line rather than during
    // it, because an approach is a fact about a pair of notes and the pitch it
    // approaches is only known once the next note exists.
    //
    // ⚠ Only a note with no slide of its own — a note already sliding somewhere
    // has been given a gesture, and stacking a second on it would overwrite the
    // model's own `slideIntervals` with a semitone.
    if let Some(chance) = optional_number(block, "chromaticApproachProb", rng) {
        let chance = chance.clamp(0.0, 1.0);
        // ⛔⛔ **Backwards, and forwards was a real defect.** Each step reads the
        // pitch of note `i` and rewrites note `i - 1`. Going forwards, note `i`
        // is rewritten one step *later* — so where two adjacent notes both drew
        // an approach, the first was left gliding to a pitch the second no
        // longer played: up a semitone, then straight back down. Going
        // backwards, the note being read is the one just written, so a run of
        // approaches descends chromatically into its target and every glide
        // lands on the note that actually sounds. `rage` is the only author, at
        // 0.25, and with eight notes in four bars an adjacent pair came up on
        // roughly a third of patterns.
        for i in (1..notes.len()).rev() {
            // ⚠ **Never the first note when the loop turns around on it.** The
            // turnaround above read `notes[0].pitch` to aim the last note at the
            // pitch the loop opens on; an approach here would drop that same
            // note a semitone and leave the glide pointing at a pitch nothing
            // plays. No model authors both keys today — `uk-drill` has the
            // turnaround and `rage` the approach — so this costs nothing and
            // closes it before the two ever meet.
            if slide_at_loop_end && i == 1 {
                continue;
            }
            let target = notes[i].pitch;
            if notes[i - 1].slide_to_pitch.is_some() || !rng.random_bool(chance) {
                continue;
            }
            // Below, not above: an approach from underneath is what a leading
            // tone is, and `register`'s floor is what stops it walking off the
            // bottom of the 808's range.
            let Some(from) = target.checked_sub(1).filter(|from| *from >= low) else {
                continue;
            };
            notes[i - 1].pitch = from;
            notes[i - 1].slide_to_pitch = Some(target);
        }
    }

    // Drill's rising whoop: the phrase's last slide goes UP an octave instead of
    // resolving wherever `slideIntervals` sent it.
    //
    // ⛔⛔ **It turns a slide the model already placed; it never adds one.** Both
    // models that author it also author `slidesPer4Bars` — uk-drill `[2, 3]`,
    // uk-underground `[2, 4]` — and a whoop written onto a note that was not
    // sliding is a *fourth* slide in a four-bar phrase that asked for three.
    // `drill_slides_two_to_three_times_every_four_bars` caught exactly that at
    // seed 11, and its comment says why the ceiling is absolute: "three is what
    // the model asks for and a fourth would be a different genre".
    //
    // ⚠ **Never inert, and that is what makes the octave the right target.** Both
    // authors also carry `longDownGlideProb` — 0.35 and 0.3 — so an ordinary
    // phrase-end slide may already be going down, and an octave up is audibly
    // not any of `P5`, `m7`, `P8` or `M2` taken downward.
    if let Some(chance) = optional_number(block, "upwardWhoopProb", rng) {
        let chance = chance.clamp(0.0, 1.0);
        let phrase = bar_ticks * 4;
        let phrases = u32::from(ctx.bars).div_ceil(4).max(1);
        let ceiling = high.max(root.saturating_add(12));
        for phrase_index in 0..phrases {
            let window = (phrase_index * phrase)..((phrase_index + 1) * phrase);
            // ⚠ Drawn before the search, so a phrase with no slide in it costs
            // the same rng position as one that has.
            let whoops = rng.random_bool(chance);
            let Some(note) = notes
                .iter_mut()
                .rfind(|note| window.contains(&note.start_tick) && note.slide_to_pitch.is_some())
            else {
                continue;
            };
            if !whoops {
                continue;
            }
            note.slide_to_pitch =
                theory::fold_into_register(i16::from(note.pitch) + 12, low, ceiling)
                    .filter(|target| *target != note.pitch)
                    // A fold that lands back on the note itself leaves the slide
                    // the model placed rather than deleting it: the whoop failed
                    // to be a gesture, and a note that was sliding still is.
                    .or(note.slide_to_pitch);
        }
    }

    // ── The glide's shape ────────────────────────────────────────────────
    //
    // ⛔ **`portamentoMs` is the largest thing the authored-key register ever
    // held: 105 models asking for a glide time, against a sampler that held one
    // rate per voice.** Drill snaps in 60 ms and afroswing swoops over 220, and
    // until now both came out of the speakers as "the rest of the note".
    //
    // ⚠ **Drawn once for the take, not once per note.** Portamento is a setting
    // on the instrument — a producer sets the glide and plays the line, they do
    // not re-dial it between notes — and a per-note draw would put 105 models'
    // rng position somewhere new for every slide, moving pitches that have
    // nothing to do with the key.
    //
    // ⚠ Stamped in one pass at the end, because two later passes *create*
    // slides: `chromaticApproachProb` gives one to the note before its target
    // and `upwardWhoopProb` turns one that already existed. Stamping at push
    // time would have left both of those with the sampler's own constant.
    let portamento = optional_number(block, "portamentoMs", tone)
        .map(|ms| ms.clamp(0.0, f64::from(u16::MAX)) as u16);
    let overlap = text(block, "slideOverlap")
        .and_then(grid::note_value_ticks)
        .map(|ticks| ticks.min(u32::from(u16::MAX)) as u16);
    for note in &mut notes {
        if note.slide_to_pitch.is_some() {
            note.slide_ms = portamento;
            note.slide_overlap_ticks = overlap;
        }
    }

    // Legato: each note runs to the next, and the last to the end of the
    // pattern. This is the pass that makes it an 808 rather than a bass drum
    // with a pitch.
    //
    // Unless the model asks for the other kind. Plugg's "Light 808" is short
    // and staccato on purpose — a bounce, not a sustain — and running it
    // legato would make the genre sound like trap, which is exactly what it is
    // defined against.
    let legato = text(block, "sustain")
        .and_then(Sustain::parse)
        .unwrap_or(Sustain::Legato)
        == Sustain::Legato;
    let total = ctx.total_ticks();
    for i in 0..notes.len() {
        let next = notes.get(i + 1).map(|n| n.start_tick).unwrap_or(total);
        let room = next.saturating_sub(notes[i].start_tick).max(1);
        notes[i].len_ticks = if legato {
            room
        } else {
            // Short, but never longer than the room it has: a staccato note
            // that overran the next one would break the mono rule.
            room.min(grid::SIXTEENTH)
        };
        if !legato {
            notes[i].articulation = Some(Articulation::Staccato);
        }
    }

    // "Mutes at snare hits" — the drill signature gap. The note stops at the
    // snare instead of ringing through it.
    // And a note that merely *reaches* a snare stops there. Nothing starts
    // within the tolerance of one any more, so the cut always leaves a real
    // note behind rather than a click.
    //
    // ⛔ **`ringing_snares`, not `snares` — a fill's notes count here and do
    // not count above.** Dropping the 808 under every note of a roll shreds the
    // line; sustaining it across the whole roll is what drill is defined
    // against. Stopping at the first one it reaches is both rules at once.
    if mute {
        for note in &mut notes {
            // ⛔ **`min`, not `find`.** The lane's notes are in insertion order —
            // backbeat, then ghosts, then the fill's roll appended last — so
            // `find` returned whichever happened to come first in the *vector*
            // and clamped to it, leaving an earlier snare still rung through.
            // "The first snare it reaches" is a fact about time, and only `min`
            // says that. This was invisible while the list held nothing but a
            // backbeat already in bar order.
            if let Some(snare) = ringing_snares
                .iter()
                .filter(|s| **s > note.start_tick && **s < note.start_tick + note.len_ticks)
                .min()
            {
                note.len_ticks = snare - note.start_tick;
            }
        }
    }

    notes
}

/// How an 808 note is held.
///
/// Spelled as an enum with a `parse`, like every other vocabulary the dataset
/// uses, so an unrecognised value is visibly not understood rather than
/// collapsing silently into the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sustain {
    /// Every note runs to the next — the trap and drill 808.
    Legato,
    /// Short and bouncy — plugg's "Light 808".
    Staccato,
}

impl Sustain {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "legato" => Some(Self::Legato),
            "staccato" => Some(Self::Staccato),
            _ => None,
        }
    }
}

/// What the 808 is doing musically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum Bass808Role {
    /// Doubles the roots under the kick.
    #[serde(rename = "bassline")]
    Bassline,
    /// Carries its own line — the UK drill marker.
    #[serde(rename = "counter_riff")]
    CounterRiff,
}

impl Bass808Role {
    /// Parsed through serde so the names exist once — see
    /// [`SnarePlacement::parse`] for what that closes.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_value(Value::String(text.to_owned())).ok()
    }
}

/// A lane by the name the dataset uses for it, so a model can name the lane a
/// fill turns over on without the engine growing a flag per genre.
fn lane_by_name(name: &str) -> Option<Lane> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

/// Where in the bar a perc layer is allowed to land.
///
/// ⛔ **This was a bare string compared against `"offbeat"`, and the fallback
/// was silent.** Any other word — including a typo, and including `"downbeat"`,
/// which reads like it ought to work — meant "anywhere", so a model could ask
/// for an accent layer and get a sprinkle with nothing saying so. That is the
/// same class of hole `snare.placement` has a shipped-roster gate for, and
/// [`can_read_perc_placement`] is what gives this one the same gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PercPlacement {
    /// Anywhere on the 16th grid — what a sprinkle means, and the default.
    #[default]
    Anywhere,
    /// Between the beats. The layer that answers the pulse rather than doubling
    /// it — drill's rim, and `uk-drill`'s woodblock.
    Offbeat,
    /// On the beats only.
    ///
    /// ⛔ **What makes a crash authorable at all.** A cymbal accent is a
    /// *position*, not a density: sprinkled across the 16ths it lands between
    /// the beats and reads as a mistake rather than as an accent. Without this
    /// the only honest way to ship `Lane::Crash` was not to ship it.
    Downbeat,
}

impl PercPlacement {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "any" => Some(Self::Anywhere),
            "offbeat" => Some(Self::Offbeat),
            "downbeat" => Some(Self::Downbeat),
            _ => None,
        }
    }

    /// May a hit land on this 16th?
    ///
    /// ⛔⛔ **A meter with no in-between positions honours neither filter, and
    /// the alternative was a silent lane.** `sixteenths_per_beat` floors at 1,
    /// so in x/16 and x/32 *every* 16th is a beat — which made `Offbeat` refuse
    /// all of them and collect an empty pool. `uk-drill`, `ny-drill` and
    /// `pop-smoke` all author `placement: "offbeat"`, so a producer whose host
    /// project is in 5/16 got a rim lane that emitted nothing at all: absent
    /// from the grid, from playback and from the exported stem, with nothing on
    /// screen saying why. The old `index % 4 == 0` left 12 of 16 open in any
    /// meter and never had to face this.
    ///
    /// ⚠ **A layer the model asked for sounds.** Where the meter cannot express
    /// the placement, the placement is what gives way — the readout-that-lies
    /// rule this project keeps writing down cuts the other way round.
    fn allows(self, index: u32, ctx: &SessionContext) -> bool {
        if grid::sixteenths_per_beat(ctx) < 2 {
            return true;
        }
        match self {
            Self::Anywhere => true,
            Self::Offbeat => !grid::is_downbeat(index, ctx),
            Self::Downbeat => grid::is_downbeat(index, ctx),
        }
    }
}

/// The `Lane` a `percs.lanes` entry names, if the generator will write it.
///
/// ⛔⛔ **THE ONE PLACE THE MEMBERSHIP RULE IS SPELLED, and it has to be the one
/// [`percs`] itself uses.** The first cut wrote the rule twice — `percs` filtered
/// inline and this answered separately — which reads as harmless because the two
/// spellings agree today. They agree *by coincidence*: a lane later gated on
/// something else (a kit pad, a tier) would be added to the generator's filter
/// and not to this one, and the lint and the test that both derive their promise
/// from this function would go on passing while the layer went silent. That is
/// the exact failure this whole check exists to catch, one level up.
///
/// ⚠ `dataset::validate` refuses a lane that would be dropped in silence — see
/// [`can_read_perc_placement`], which exists for the same reason one field over.
fn perc_lane(name: &str) -> Option<Lane> {
    lane_by_name(name).filter(|lane| PERC_LANES.contains(lane))
}

/// Is this `percs.lanes` entry one the generator will write?
pub fn is_perc_lane(name: &str) -> bool {
    perc_lane(name).is_some()
}

/// Is this `percs.placement` one the generator can act on?
pub fn can_read_perc_placement(name: &str) -> bool {
    PercPlacement::parse(name).is_some()
}

/// The percussion lanes a model names in `drums.percs` (TASK-140).
///
/// ⛔ **15 of the 30 shipped models authored this block before anything read
/// it.** `lanes`, `densityPerBar`, `placement` and `gainOffsetDb` were being
/// written by hand and thrown away, and `dataset:validate` could not say so
/// because `drums` resolves to `$defs/partBlock`, which declares no properties
/// at all. ▶ **The shape here is the one the dataset already uses, not a new
/// one** — `uk-drill` asked for `["rim", "woodblock"]` at `[1, 3]`, offbeat,
/// -12 dB, and this is what finally plays it. `woodblock` was not even a
/// `Lane`, so `lane_by_name` answered `None` and the request vanished twice
/// over.
///
/// Density is the whole grammar: these are the lanes whose behaviour is
/// "sprinkle hits at this rate". That is why the kick, snare and hats are kept
/// out of [`PERC_LANES`] — each has an authored block and a placement grammar
/// of its own, and two stages writing one voice is how a lane doubles up.
fn percs(
    kit: &mut DrumKit,
    drums: Option<&Value>,
    ctx: &SessionContext,
    tiers: &VelocityTiers,
    rng: &mut impl Rng,
) {
    let percs = block(drums, "percs");

    // ⛔ **Through `perc_lane`, not an inline filter.** See its own note: the
    // lint and the shipped-corpus test both promise that what the dataset names
    // is what this writes, and that promise is only true while there is one
    // spelling of the rule.
    let lanes: Vec<Lane> = strings(percs, "lanes")
        .iter()
        .filter_map(|name| perc_lane(name))
        .collect();

    // ⚠ The tambourine is authored as its own flag rather than through `lanes`
    // because neither of its two shipped forms is density-driven:
    // `country-train` wants a steady stream and `west-coast-club` wants it
    // doubling the clap. Folding them into `lanes` would have meant inventing a
    // placement word for each and rewriting two models to say the same thing.
    let steady_tambourine = flag(percs, "tambourine", false);
    let tambourine_mirrors_clap = flag(percs, "tambourineMirrorsClap", false);

    if lanes.is_empty() && !steady_tambourine && !tambourine_mirrors_clap {
        return;
    }

    // Authored in dB because that is how a producer thinks about a perc layer
    // sitting under the kit rather than beside it.
    let gain = optional_number(percs, "gainOffsetDb", rng)
        .map(|db| 10f64.powf(db / 20.0))
        .unwrap_or(1.0);
    let scaled = |vel: u8| ((f64::from(vel) * gain).round() as u8).clamp(1, 127);

    let placement = text(percs, "placement")
        .and_then(PercPlacement::parse)
        .unwrap_or_default();
    let (low, high) = pair(percs, "densityPerBar").unwrap_or((0.0, 2.0));
    let (low, high) = (low.max(0.0), high.max(0.0));

    let per_bar = grid::sixteenths_per_bar(ctx);
    let bar_ticks = ctx.ticks_per_bar();

    for bar in 0..u32::from(ctx.bars) {
        let bar_start = bar * bar_ticks;

        if steady_tambourine {
            // Straight 8ths — the country backbeat's shaker-hand, which is a
            // pulse rather than a sprinkle.
            for index in (0..per_bar).step_by(2) {
                kit.hit(
                    Lane::Tambourine,
                    bar_start + index * grid::SIXTEENTH,
                    scaled(tiers.pick(None, rng)),
                    None,
                );
            }
        }

        for lane in &lanes {
            // Redrawn per bar so a two-bar phrase is not the same bar twice —
            // the collision gate measures the skeleton, and a perc layer that
            // repeats exactly adds nothing to it.
            let count = if high <= low {
                low.round() as u32
            } else {
                rng.random_range(low..=high).round() as u32
            };

            // The candidate positions, drawn without replacement so two hits of
            // one voice never land on the same 16th and read as one.
            let mut pool: Vec<(u32, f64)> = (0..per_bar)
                .filter(|index| placement.allows(*index, ctx))
                .map(|index| (index, 1.0))
                .collect();

            for _ in 0..count.min(pool.len() as u32) {
                let Some(index) = take_weighted(&mut pool, rng) else {
                    break;
                };
                kit.hit(
                    *lane,
                    bar_start + index * grid::SIXTEENTH,
                    scaled(tiers.pick(None, rng)),
                    None,
                );
            }
        }
    }

    // Last, because it reads the clap lane the snare stage already wrote.
    if tambourine_mirrors_clap {
        let claps: Vec<u32> = kit.notes(Lane::Clap).iter().map(|n| n.start_tick).collect();
        for tick in claps {
            kit.hit(Lane::Tambourine, tick, scaled(tiers.pick(None, rng)), None);
        }
    }
}

/// The off-snare: a second snare voice on the off-beats (TASK-140).
///
/// Mike named it alongside claps, and a clap is a lane, so this is a lane — see
/// [`Lane::OffSnare`]. Authored the same way the ghost snare is, with `pos` and
/// `prob`, because it is the same kind of rule and a second vocabulary for
/// "where does a snare-ish hit go" is one more thing to keep in agreement.
/// A burst of snares bunched around the beat, from `clusterProb` /
/// `clusterHits`.
///
/// ⛔⛔ **The cluster is CENTRED on the beat, not started on it.** The research
/// calls jerk's marker *"snare CLUSTERS, 2–4 hits bunched **around** the
/// expected backbeat"*, and a burst that begins on the beat is not that: it is
/// the backbeat plus a tail, and every note of it arrives late. Centring keeps
/// one note exactly where the un-clustered snare would have been — whatever the
/// count — so the beat is still heard in the same place and the cluster reads
/// as a smear around it rather than as a drag.
///
/// ⚠ **It calls [`rolls::stutter_cluster`]** rather than writing a third burst
/// loop. Its 112 → 84 ramp is what makes a cluster one gesture instead of four
/// snares: the hits after the first are softer, the way a hand bouncing off a
/// drum is.
fn snare_cluster(tick: u32, hits: (f64, f64), rng: &mut impl Rng) -> Vec<Note> {
    // 2..=4 is what the dataset authors and what a snare can physically be
    // played as. Below two there is no cluster; above four it is a roll, which
    // is [`rolls`]' own vocabulary and a different device.
    let lo = (hits.0.round() as i64).clamp(2, 4);
    let hi = (hits.1.round() as i64).clamp(lo, 4);
    let count = rng.random_range(lo..=hi) as usize;

    // A 32nd: tight enough that the burst is one gesture and not four snares.
    let subdivision = grid::SIXTEENTH / 2;
    // The note that lands on the beat is index `count / 2`, so the window opens
    // that many subdivisions early. `displace` keeps bar 0 off the negative.
    let start = displace(tick, -((count as i64 / 2) * i64::from(subdivision)));
    rolls::stutter_cluster(Lane::Snare, start, subdivision, count, rng)
}

fn off_snares(
    kit: &mut DrumKit,
    snare_block: Option<&Value>,
    ctx: &SessionContext,
    tiers: &VelocityTiers,
    bar_start: u32,
    off_grid_ticks: i64,
    rng: &mut impl Rng,
) {
    let off = block(snare_block, "offSnare");
    let positions = strings(off, "pos");
    if positions.is_empty() {
        return;
    }
    let chance = number(off, "prob", 1.0, rng).clamp(0.0, 1.0);

    for position in &positions {
        if !rng.random_bool(chance) {
            continue;
        }
        let Some(offset) = grid::position_ticks(position, ctx) else {
            continue;
        };
        kit.hit(
            Lane::OffSnare,
            displace(bar_start + offset, off_grid_ticks),
            tiers.pick(None, rng),
            None,
        );
    }
}

/// The 808's own onsets, when it is not riding the kick.
///
/// The 8th-note grid rather than the 16th: a sub moving on 16ths is a bassline,
/// and a bassline is [`super::bass`]'s job. `count` is the number of notes the
/// model's `lockTo808` already gave the line — see the call site. Positions are
/// taken without replacement so the line never doubles a note on itself, and the
/// result is sorted because everything downstream — the slide windows, the
/// legato pass, the mute — reads it as a timeline.
fn independent_onsets(ctx: &SessionContext, count: usize, rng: &mut impl Rng) -> Vec<u32> {
    let eighth = (grid::ticks_per_beat(ctx) / 2).max(1);
    let mut pool: Vec<u32> = (0..ctx.total_ticks().max(1))
        .step_by(eighth as usize)
        .collect();

    let mut out: Vec<u32> = Vec::with_capacity(count.min(pool.len()));
    for _ in 0..count.min(pool.len()) {
        out.push(pool.remove(rng.random_range(0..pool.len())));
    }
    out.sort_unstable();
    out
}

/// Where an 808 slide may go.
///
/// Named separately from the roll positions because the vocabularies genuinely
/// differ — an 808 slides at the end of a two- or four-bar phrase, never
/// "before the snare".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RollLikePosition {
    PhraseEnd,
    Bar2,
    Bar4,
}

impl RollLikePosition {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "phrase_end" => Some(Self::PhraseEnd),
            "bar_2" => Some(Self::Bar2),
            "bar_4" => Some(Self::Bar4),
            _ => None,
        }
    }

    /// Does this position cover the given bar?
    ///
    /// The whole bar, not its final beat. The research says slides land "at the
    /// ends of 2/4-bar phrases", and drill asks for two to three of them every
    /// four bars — a single-beat window cannot hold that many, so the unit is
    /// the bar that *closes* the phrase rather than the beat that ends it.
    fn covers(self, bar: u32, bars: u32) -> bool {
        match self {
            // A phrase is two bars — the shorter of the two the research names —
            // and the pattern's last bar always closes one.
            Self::PhraseEnd => (bar + 1).is_multiple_of(2) || bar + 1 == bars,
            Self::Bar2 => (bar + 1).is_multiple_of(2),
            Self::Bar4 => (bar + 1).is_multiple_of(4),
        }
    }
}

/// The hi-hat lanes: the base stream, its fill, and the open hats over it.
/// Drop every closed hat that lands on a tick an open hat already occupies.
///
/// The rule `hats()` states when it places one: "one hi-hat cannot be open and
/// shut at the same instant, so the closed hit underneath goes." Dropped rather
/// than nudged, which is what `hats()` does — the stream keeps its grid and
/// simply has a hole where the open hat is sounding.
fn close_over_open(closed: &mut Vec<Note>, open: &[Note]) {
    if open.is_empty() {
        return;
    }
    closed.retain(|hit| !open.iter().any(|hat| hat.start_tick == hit.start_tick));
}

fn hats(
    hihat: Option<&Value>,
    ctx: &SessionContext,
    tiers: &VelocityTiers,
    // Where the kick plays, for `mirrorsKick`. Empty for every model that does
    // not author it, and read nowhere else.
    kicks: &[u32],
    rng: &mut impl Rng,
    // ⛔ **Everything below that the stream did not used to ask for.** A new key
    // drawing from `rng` shifts the whole hat part for the model that authors
    // it: two draws for `tripletBarAlternationProb` moved which position
    // `openHat` landed on, that position ate a closed hit underneath it, and
    // `drill_hats_sit_on_the_tresillo_the_model_authors` lost a 3-3-2 hit — for
    // a key that, on a `tresillo` base, had no effect at all. See the kick's
    // streams in [`generate`] for the same lesson learned the same way.
    extras: &mut impl Rng,
) -> (Vec<Note>, Vec<Note>) {
    if hihat.is_none() {
        return (Vec::new(), Vec::new());
    }

    // The subdivision is chosen once for the whole pattern. Trap authors it as
    // a weighted choice between 8ths and 16ths; re-rolling it every bar would
    // be a different hat part each bar rather than one hat part.
    let base = string_spec(hihat, "base", rng).unwrap_or_else(|| "8th".to_owned());

    let grouping: Vec<u32> = hihat
        .and_then(|h| h.get("tresilloGrouping"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|n| n as u32)
                .collect()
        })
        .unwrap_or_else(|| vec![3, 3, 2]);

    let fill_density = number(hihat, "fillDensity", 0.4, rng).clamp(0.0, 1.0);
    // Rage's hats are "fast but SPARSE — bursts, not continuous streams", so a
    // non-continuous stream spends its density on *whether a beat plays at all*
    // rather than on filling the gaps between hits.
    let continuous = flag(hihat, "continuous", true);

    // ⛔ **The hat stops running its own subdivision and plays where the kick
    // plays.** west-coast-club's hat is the kick's rhythm doubled an octave up
    // in the kit, which is a different instruction from any density: no
    // subdivision, however sparse, lands a hat on exactly the kick's onsets.
    let mirrors_kick = flag(hihat, "mirrorsKick", false);

    // Phonk's gated hats: the stream is CUT, not shortened. A hat "cut short
    // rather than ringing" is a note length, and note length does not reach the
    // sampler yet (TASK-053A) — so a gate that shortened notes would be one more
    // authored key doing nothing audible. Silencing a beat of the stream is the
    // same gesture and it is heard.
    let gating = optional_number(hihat, "gatingProb", extras).map(|p| p.clamp(0.0, 1.0));

    // Plugg's open-hat chains: an open hat that is a run rather than a single
    // stab. The chain rides the 8th after the one it opened on.
    let open_chains = flag(hihat, "openHatChains", false);

    // uk-drill's alternating triplet bars. Drawn ONCE for the pattern, like the
    // base itself: a stream that changes its mind every bar is a glitch, and
    // what the key names is an *alternation* — the odd bars go triplet and the
    // even ones do not.
    let triplet_bars = optional_number(hihat, "tripletBarAlternationProb", extras)
        .is_some_and(|prob| extras.random_bool(prob.clamp(0.0, 1.0)));

    let velocities = hihat.and_then(|h| h.get("velocities"));
    let mut closed: Vec<Note> = Vec::new();
    let mut open: Vec<Note> = Vec::new();

    let bar_ticks = ctx.ticks_per_bar();
    let beat = grid::ticks_per_beat(ctx);
    let onsets = hat_base_onsets(&base, &grouping, ctx);
    // The subdivision an alternating bar runs at, and the stream it plays.
    //
    // ⚠ **A base that is not a note value — `"tresillo"` — still has a triplet.**
    // The tresillo grouping is counted in 16ths, so its alternating bar is the
    // 16th triplet, which is the drill hat everybody knows. Returning nothing
    // there would leave the key authored, read, and doing nothing, which is the
    // exact condition this whole pass exists to end.
    let triplet_step = rolls::triplet_of(grid::note_value_ticks(&base).unwrap_or(grid::SIXTEENTH));
    let triplet_onsets: Vec<u32> = match (triplet_bars, triplet_step) {
        (true, Some(step)) => (0..bar_ticks).step_by(step as usize).collect(),
        _ => Vec::new(),
    };
    let open_hat = block(hihat, "openHat");
    let positions = strings(open_hat, "pos");

    for bar in 0..u32::from(ctx.bars) {
        let bar_start = bar * bar_ticks;

        // Which beats play at all.
        let beats_played: Vec<u32> = (0..u32::from(ctx.time_sig_num.max(1)))
            .filter(|_| continuous || rng.random_bool(fill_density))
            .collect();

        // ⚠ Odd bars only, and only when the alternation was drawn — that is
        // what makes it an alternation rather than a triplet hat part.
        // ⛔ **The gap-filling below moves with it.** A triplet bar back-filled
        // at 16ths is triplets *and* straight 16ths at once, which is not an
        // alternating bar — it is a mess with the marker buried in it.
        let triplet_now =
            triplet_step.filter(|_| !bar.is_multiple_of(2) && !triplet_onsets.is_empty());
        let bar_onsets: &[u32] = if triplet_now.is_some() {
            &triplet_onsets
        } else {
            &onsets
        };
        let fill_step = triplet_now.unwrap_or(grid::SIXTEENTH);

        let mut ticks: Vec<u32> = if mirrors_kick {
            // The kick's onsets in this bar, brought back to bar-relative ticks
            // so everything below — the fill pass, the main/ghost split, the
            // open hats — keeps working on the one coordinate it expects.
            kicks
                .iter()
                .filter(|tick| (bar_start..bar_start + bar_ticks).contains(*tick))
                .map(|tick| tick - bar_start)
                .collect()
        } else {
            bar_onsets
                .iter()
                .copied()
                .filter(|tick| beats_played.contains(&(tick / beat)))
                .collect()
        };

        // A continuous stream fills the gaps between its onsets; the extras are
        // the quiet 16ths that make it breathe.
        //
        // ⚠ Not when the stream is the kick's: filling the gaps of a mirrored
        // hat is exactly the subdivision `mirrorsKick` says not to play.
        if continuous && !mirrors_kick {
            for index in 0..bar_ticks / fill_step {
                let tick = index * fill_step;
                if !ticks.contains(&tick) && rng.random_bool(fill_density) {
                    ticks.push(tick);
                }
            }
        }
        ticks.sort_unstable();
        ticks.dedup();

        // The gate: one beat of this bar goes silent. Drawn per bar, because a
        // gate that fired once for the pattern would be a hole rather than a
        // stutter.
        if let Some(prob) = gating {
            if extras.random_bool(prob) {
                let gated = extras.random_range(0..u32::from(ctx.time_sig_num.max(1)));
                ticks.retain(|tick| tick / beat != gated);
            }
        }

        for tick in ticks {
            let main = is_main_position(tick);
            let key = if main { "main" } else { "ghost" };
            let articulation = if main {
                None
            } else {
                Some(Articulation::Ghost)
            };
            let vel = fractional_velocity(velocities, key, rng)
                .unwrap_or_else(|| tiers.pick(articulation, rng));
            closed.push(note_at(
                Lane::ClosedHat,
                bar_start + tick,
                vel,
                articulation,
            ));
        }

        // Open hats sit over the stream — and close it: one hi-hat cannot be
        // open and shut at the same instant, so the closed hit underneath goes.
        // (`prob` and `perBar` are sampled per bar on purpose — those are real
        // rerolls. Only the position list, which never changes, is hoisted.)
        let chance = number(open_hat, "prob", 0.0, rng).clamp(0.0, 1.0);
        if !positions.is_empty() && rng.random_bool(chance) {
            let wanted = number(open_hat, "perBar", 1.0, rng).round().max(1.0) as usize;
            let mut available: Vec<&String> = positions.iter().collect();
            for _ in 0..wanted.min(available.len()) {
                let choice = rng.random_range(0..available.len());
                let position = available.remove(choice);
                let Some(tick) = open_hat_tick(position, bar_start, ctx) else {
                    continue;
                };
                // A chain is the open hat plus the 8th after it — plugg's
                // "open hats in runs" rather than one stab. It stops at the
                // bar's edge: a chain that ran into the next bar would open a
                // hat the next bar never asked for.
                let chain = if open_chains { 2 } else { 1 };
                let eighth = beat / 2;
                for step in 0..chain {
                    let at = tick + step * eighth;
                    if at >= bar_start + bar_ticks {
                        break;
                    }
                    closed.retain(|n| n.start_tick != at);
                    // ⚠ Drawn per note of the chain, so the run breathes
                    // instead of being one velocity stamped twice.
                    let vel = fractional_velocity(velocities, "main", rng)
                        .unwrap_or_else(|| tiers.pick(Some(Articulation::Accent), rng));
                    open.push(note_at(Lane::OpenHat, at, vel, Some(Articulation::Accent)));
                }
            }
        }
    }

    // The second hat layer, repitched a few semitones (research ch. 1 §1). It
    // rides on `Note.pitch`, which the sampler reads and the SMF writer
    // replaces with the lane's GM voice — GM has exactly one closed hat, so
    // this is a detail of *our* playback rather than of the exported file.
    let bend_chance = number(hihat, "pitchBendProb", 0.0, rng).clamp(0.0, 1.0);
    if !closed.is_empty() && rng.random_bool(bend_chance) {
        let semitones = rng.random_range(1..=3);
        let up = rng.random_bool(0.5);
        let bar = rng.random_range(0..u32::from(ctx.bars));
        let range = (bar * bar_ticks)..((bar + 1) * bar_ticks);
        for note in closed.iter_mut().filter(|n| range.contains(&n.start_tick)) {
            note.pitch = if up {
                note.pitch.saturating_add(semitones)
            } else {
                note.pitch.saturating_sub(semitones)
            };
        }
    }

    // The hat swell: a gradual rise across the loop (research ch. 1 §1). It
    // scales what is there rather than overwriting it, so the main/ghost
    // contour survives the gesture.
    let swell_chance = number(hihat, "swellProb", 0.0, rng).clamp(0.0, 1.0);
    if rng.random_bool(swell_chance) {
        let total = ctx.total_ticks().max(1) as f32;
        for note in &mut closed {
            let progress = note.start_tick as f32 / total;
            let scale = 0.7 + 0.3 * progress;
            note.vel = ((f32::from(note.vel) * scale).round()).clamp(1.0, 127.0) as u8;
        }
    }

    (closed, open)
}

/// Shift a tick by a signed displacement without falling off the start.
fn displace(tick: u32, ticks: i64) -> u32 {
    (i64::from(tick) + ticks).max(0) as u32
}

/// The widest deliberate displacement a grammar may ask for, in milliseconds.
///
/// ⛔⛔ **A bound, because the alternative is the host process.** `read::pair`
/// validates nothing and `sloppyOffsetMs` matches no probability suffix, so a
/// hand-edited or imported model may state `[-1e308, 1e308]` and reach
/// `Rng::random_range` — whose `UniformFloat` computes `high - low`, gets
/// `inf`, and `unwrap()`s a `NonFinite` error. `release` is `panic = "abort"`.
/// The same value at `[0, 1e30]` instead saturates the `as i64` cast and
/// overflows `displace`, which panics under the workspace's dev/test
/// `overflow-checks`.
///
/// ⚠ 250 ms is an eighth note at 120 BPM — past anything a producer would call
/// a displacement rather than a different beat. The dataset's widest is 14.
const MAX_OFFSET_MS: f64 = 250.0;

/// A deliberate displacement in milliseconds, as whole ticks.
///
/// ⚠ Shared by every grammar-level nudge — `snare.offGridMs`,
/// `snare.layerClapOffsetMs` and `kick.sloppyOffsetMs` — so the bound above is
/// stated once rather than at each of them.
fn offset_ticks(ms: f64, ctx: &SessionContext) -> i64 {
    ctx.ms_to_ticks(ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS) as f32)
        .round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_perc_lane_can_be_built() {
        // ⛔⛔ **The gate for the defect TASK-043A walked straight into.** Eleven
        // lanes went into `PERC_LANES`, nine shipped models authored them, and
        // every one produced *nothing* — `Kit` accumulates per lane in
        // `LANE_ORDER` and silently drops whatever it has no slot for. The
        // dataset said one thing, the output said another, and the only test
        // that noticed was a roster-wide one two files away.
        //
        // ⚠ **One direction only.** `LANE_ORDER` is deliberately longer: the
        // kick, the snare and the hats have their own grammar and must never be
        // nameable as percs. What must hold is that nothing a model can *ask*
        // for is unbuildable.
        let missing: Vec<&Lane> = PERC_LANES
            .iter()
            .filter(|lane| !LANE_ORDER.contains(lane))
            .collect();
        assert!(
            missing.is_empty(),
            "a model can author these percs and the kit has nowhere to put them: {missing:?}"
        );
    }

    fn model(drums: Value) -> StyleModel {
        serde_json::from_value(json!({
            "id": "test", "type": "genre", "name": "Test",
            "drums": drums,
        }))
        .expect("the test model must parse")
    }

    fn ctx(bars: u16) -> SessionContext {
        SessionContext {
            bars,
            ..Default::default()
        }
    }

    fn lane(lanes: &[LaneTrack], lane: Lane) -> Option<&LaneTrack> {
        lanes.iter().find(|l| l.lane == lane)
    }

    fn starts(lanes: &[LaneTrack], want: Lane) -> Vec<u32> {
        lane(lanes, want)
            .map(|l| l.notes.iter().map(|n| n.start_tick).collect())
            .unwrap_or_default()
    }

    // ── The authored-but-unread keys, wired 2026-08-18 ──────────────────────
    //
    // Each of these was a parameter real models carry that no code read. The
    // gate that found them is `engine/tests/authored_keys.rs`; these are what
    // says the reading is the one the research describes, rather than merely a
    // mention of the key somewhere in the source — which is all that gate can
    // see, and it says so itself.

    /// Every start tick of one lane, sorted.
    fn sorted(lanes: &[LaneTrack], want: Lane) -> Vec<u32> {
        let mut out = starts(lanes, want);
        out.sort_unstable();
        out
    }

    #[test]
    fn a_snare_cluster_surrounds_the_beat_instead_of_starting_on_it() {
        // The research calls jerk's marker hits "bunched around the expected
        // backbeat". A burst that begins on the beat would put every note of
        // the cluster late and take the backbeat with it.
        let m = model(json!({
            "snare": { "placement": "backbeat_24", "clusterProb": 1.0, "clusterHits": [3, 3] },
            "kick": { "anchors": ["1"], "densityPerBar": 1 },
        }));
        let c = ctx(1);
        let beat = grid::ticks_per_beat(&c);
        let snares = sorted(&generate(&m, &c, 3), Lane::Snare);

        let thirty_second = grid::SIXTEENTH / 2;
        for beat_tick in [beat, beat * 3] {
            assert!(
                snares.contains(&beat_tick),
                "the backbeat itself is still played: {snares:?}"
            );
            assert!(snares.contains(&(beat_tick - thirty_second)), "{snares:?}");
            assert!(snares.contains(&(beat_tick + thirty_second)), "{snares:?}");
        }
        assert_eq!(snares.len(), 6, "two clusters of three: {snares:?}");
    }

    #[test]
    fn a_lane_that_states_its_own_velocity_range_does_not_use_the_tier() {
        // `rnb-2000s` writes `kick.velocityRange: [0.7, 0.85]` — 89 to 108 — and
        // the generic main tier is 76 to 89. Only one of those bands can be
        // right, and the model's own is the specific claim.
        let m = model(json!({
            "kick": {
                "anchors": ["1", "2", "3", "4"],
                "densityPerBar": 4,
                "velocityRange": [0.7, 0.85],
            },
        }));
        let lanes = generate(&m, &ctx(2), 11);
        let kicks = lane(&lanes, Lane::Kick).expect("the kick plays");
        assert!(
            kicks.notes.iter().all(|n| (89..=108).contains(&n.vel)),
            "every kick sits in the authored band: {:?}",
            kicks.notes.iter().map(|n| n.vel).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_sub_layer_doubles_the_kick_unless_it_is_told_to_separate() {
        // Boom-bap's sub sits under the kick. `Lane::SubKick` had a GM voice and
        // a slot in `LANE_ORDER` the whole time with nothing writing to it.
        let m = model(json!({
            "kick": { "anchors": ["1", "3"], "densityPerBar": 2, "subLayerVelocity": [0.5, 0.6] },
        }));
        let lanes = generate(&m, &ctx(1), 5);
        assert_eq!(
            sorted(&lanes, Lane::SubKick),
            sorted(&lanes, Lane::Kick),
            "with no `separateLayerProb` the layer is the kick's own rhythm"
        );
        let sub = lane(&lanes, Lane::SubKick).expect("the sub plays");
        assert!(
            sub.notes.iter().all(|n| (63..=77).contains(&n.vel)),
            "and quieter than the kick over it: {:?}",
            sub.notes.iter().map(|n| n.vel).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_high_tempo_full_time_switch_is_silent_at_a_slow_tempo() {
        // `rage` authors `fullTimeAtHighTempoProb`. At 1.0 the draw is certain,
        // so the only thing left to vary is the tempo — which is the whole
        // claim the key makes.
        let m = model(json!({
            "snare": { "placement": "halftime_3", "fullTimeAtHighTempoProb": 1.0 },
            "kick": { "anchors": ["1"], "densityPerBar": 1 },
        }));
        let beat = grid::ticks_per_beat(&ctx(1));

        let slow = SessionContext {
            bars: 1,
            bpm: 120.0,
            ..Default::default()
        };
        assert_eq!(
            sorted(&generate(&m, &slow, 2), Lane::Snare),
            vec![beat * 2],
            "half-time: beat 3 alone"
        );

        let fast = SessionContext {
            bars: 1,
            bpm: 160.0,
            ..Default::default()
        };
        assert_eq!(
            sorted(&generate(&m, &fast, 2), Lane::Snare),
            vec![beat, beat * 3],
            "full time: the 2 and 4 backbeat"
        );
    }

    #[test]
    fn a_skipped_beat_never_takes_an_anchor() {
        // The anchors are the positions the genre "always plays". A skip that
        // can take beat 1 is not a sparser bar, it is a different genre.
        let m = model(json!({
            "kick": {
                "anchors": ["1"],
                "secondaryAnchor": "3",
                "densityPerBar": 4,
                "beatSkipProb": 1.0,
            },
        }));
        let c = ctx(8);
        let beat = grid::ticks_per_beat(&c);
        let bar_ticks = c.ticks_per_bar();
        let kicks = sorted(&generate(&m, &c, 7), Lane::Kick);
        for bar in 0..8u32 {
            let start = bar * bar_ticks;
            assert!(kicks.contains(&start), "beat 1 of bar {bar}: {kicks:?}");
            assert!(
                kicks.contains(&(start + beat * 2)),
                "beat 3 of bar {bar}: {kicks:?}"
            );
        }
    }

    #[test]
    fn a_walking_run_arrives_at_the_next_downbeat_without_landing_on_it() {
        // Jerk's walking kick. The downbeat belongs to the next bar; doubling it
        // is what makes a run sound like a stutter.
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 1, "walkingRunProb": 1.0 },
        }));
        let c = ctx(2);
        let bar_ticks = c.ticks_per_bar();
        let kicks = sorted(&generate(&m, &c, 1), Lane::Kick);
        for step in 1..=3u32 {
            assert!(
                kicks.contains(&(bar_ticks - step * grid::SIXTEENTH)),
                "the run climbs into bar 2: {kicks:?}"
            );
        }
    }

    #[test]
    fn a_sloppy_kick_is_late_by_a_different_amount_each_time() {
        // Not `offGridMs` under another name: that is ONE displacement the whole
        // lane shares. This is the west-coast drag, drawn per hit.
        let m = model(json!({
            "kick": {
                "anchors": ["1", "2", "3", "4"],
                "densityPerBar": 4,
                "sloppyOffsetMs": [5, 40],
            },
        }));
        let c = ctx(4);
        let beat = grid::ticks_per_beat(&c);
        let kicks = sorted(&generate(&m, &c, 9), Lane::Kick);

        let lateness: Vec<u32> = kicks.iter().map(|tick| tick % beat).collect();
        assert!(
            lateness.iter().all(|late| *late > 0),
            "every hit drags: {kicks:?}"
        );
        assert!(
            lateness
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 1,
            "and not all by the same amount, which is what `offGridMs` already does: {lateness:?}"
        );
    }

    #[test]
    fn a_mirrored_hat_plays_where_the_kick_plays() {
        // west-coast-club's hat is the kick's rhythm, not a subdivision. No
        // density, however sparse, lands a hat on exactly the kick's onsets.
        let m = model(json!({
            "kick": { "anchors": ["1", "2&", "4"], "densityPerBar": 3 },
            "hihat": { "base": "16th", "mirrorsKick": true, "fillDensity": 0.9 },
        }));
        let lanes = generate(&m, &ctx(2), 4);
        assert_eq!(
            sorted(&lanes, Lane::ClosedHat),
            sorted(&lanes, Lane::Kick),
            "the hat stream is the kick's"
        );
    }

    #[test]
    fn a_gated_hat_loses_a_whole_beat_of_the_bar() {
        // Phonk's gate cuts the stream. A hat "cut short rather than ringing" is
        // a note length, and note length does not reach the sampler yet — so a
        // gate that shortened notes would be one more key doing nothing audible.
        let hihat = json!({ "base": "16th", "continuous": true, "fillDensity": 1.0 });
        let mut gated = hihat.clone();
        gated["gatingProb"] = json!(1.0);

        let c = ctx(1);
        let open = generate(&model(json!({ "hihat": hihat })), &c, 6);
        let shut = generate(&model(json!({ "hihat": gated })), &c, 6);

        let beat = grid::ticks_per_beat(&c);
        let played = sorted(&shut, Lane::ClosedHat);
        let silent: Vec<u32> = (0..u32::from(c.time_sig_num))
            .filter(|b| !played.iter().any(|t| t / beat == *b))
            .collect();
        assert_eq!(silent.len(), 1, "exactly one beat goes: {silent:?}");
        assert!(
            sorted(&open, Lane::ClosedHat).len() > played.len(),
            "and the ungated stream keeps it"
        );
    }

    #[test]
    fn an_open_hat_chain_is_a_run_rather_than_a_stab() {
        let block = |chains: bool| {
            json!({
                "base": "8th",
                "openHat": { "prob": 1.0, "perBar": 1, "pos": ["3"] },
                "openHatChains": chains,
            })
        };
        let c = ctx(1);
        let eighth = grid::ticks_per_beat(&c) / 2;

        let single = sorted(
            &generate(&model(json!({ "hihat": block(false) })), &c, 8),
            Lane::OpenHat,
        );
        let chained = sorted(
            &generate(&model(json!({ "hihat": block(true) })), &c, 8),
            Lane::OpenHat,
        );
        assert_eq!(single.len(), 1, "one stab: {single:?}");
        assert_eq!(chained.len(), 2, "a run of two: {chained:?}");
        assert_eq!(chained[1] - chained[0], eighth, "on the 8th after it");
    }

    #[test]
    fn alternating_triplet_bars_leave_the_even_bars_alone() {
        // uk-drill's alternation. The even bars keep their own 16ths; the odd
        // ones run at the triplet of that, which lands on ticks the straight
        // stream cannot reach.
        let m = model(json!({
            "hihat": {
                "base": "16th",
                "continuous": false,
                "fillDensity": 1.0,
                "tripletBarAlternationProb": 1.0,
            },
        }));
        let c = ctx(2);
        let bar_ticks = c.ticks_per_bar();
        let hats = sorted(&generate(&m, &c, 12), Lane::ClosedHat);

        assert!(
            hats.iter()
                .filter(|t| **t < bar_ticks)
                .all(|t| t.is_multiple_of(grid::SIXTEENTH)),
            "bar 1 is straight: {hats:?}"
        );
        let triplet = grid::SIXTEENTH * 2 / 3;
        assert!(
            hats.iter()
                .any(|t| *t >= bar_ticks && !(t - bar_ticks).is_multiple_of(grid::SIXTEENTH)),
            "bar 2 is not: {hats:?}"
        );
        assert!(
            hats.iter()
                .filter(|t| **t >= bar_ticks)
                .all(|t| (t - bar_ticks).is_multiple_of(triplet)),
            "and every one of its hits is on the triplet grid: {hats:?}"
        );
    }

    #[test]
    fn an_independent_808_is_not_a_subset_of_the_kick() {
        // ⛔ The whole point. The 808 filters the kick's own onsets, so
        // `lockTo808` can only ever make the sub SPARSER — it can never move it,
        // and 15 models spell `melodicallyIndependent` asking for exactly that.
        let drums = |independent: bool| {
            json!({
                "kick": {
                    "anchors": ["1", "2", "3", "4"],
                    "densityPerBar": 4,
                    "lockTo808": 1.0,
                },
                "bass808": { "register": [24, 36], "melodicallyIndependent": independent },
            })
        };
        let c = ctx(2);
        let riding = generate(&model(drums(false)), &c, 21);
        let free = generate(&model(drums(true)), &c, 21);

        let kicks = sorted(&riding, Lane::Kick);
        assert!(
            sorted(&riding, Lane::Sub).iter().all(|t| kicks.contains(t)),
            "riding the kick, it is a subset of it"
        );
        let independent = sorted(&free, Lane::Sub);
        assert_eq!(independent.len(), kicks.len(), "same busyness");
        assert!(
            independent.iter().any(|t| !kicks.contains(t)),
            "different placement: {independent:?} against {kicks:?}"
        );
    }

    #[test]
    fn a_chromatic_approach_arrives_from_a_semitone_below() {
        let m = model(json!({
            "kick": { "anchors": ["1", "2", "3", "4"], "densityPerBar": 4, "lockTo808": 1.0 },
            "bass808": { "register": [30, 42], "slideProb": 0.0, "chromaticApproachProb": 1.0 },
        }));
        let lanes = generate(&m, &ctx(2), 33);
        let sub = lane(&lanes, Lane::Sub).expect("the 808 plays");
        let approaches: Vec<&Note> = sub
            .notes
            .iter()
            .filter(|n| n.slide_to_pitch.is_some())
            .collect();
        assert!(!approaches.is_empty(), "something approaches");
        for note in approaches {
            let target = note.slide_to_pitch.expect("filtered on");
            assert_eq!(target, note.pitch + 1, "a semitone below its target");
        }
    }

    #[test]
    fn the_whoop_turns_the_last_slide_upward_and_does_not_add_one() {
        // ⛔ Both models that author the whoop also author `slidesPer4Bars`, so a
        // whoop written onto a note that was not sliding is one slide more than
        // the model asked for. Every slide here glides DOWN a semitone; the
        // whoop is the one that does not.
        let with = |whoop: f64| {
            model(json!({
                "kick": { "anchors": ["1", "3"], "densityPerBar": 2, "lockTo808": 1.0 },
                "bass808": {
                    "register": [24, 48],
                    "role": "bassline",
                    "slideProb": 1.0,
                    "slidePositions": ["phrase_end", "bar_2", "bar_4"],
                    "slideIntervals": ["m2"],
                    "upwardWhoopProb": whoop,
                },
            }))
        };
        let c = ctx(4);
        let sliding = |lanes: &[LaneTrack]| -> Vec<(u8, u8)> {
            lane(lanes, Lane::Sub)
                .map(|l| {
                    l.notes
                        .iter()
                        .filter_map(|n| n.slide_to_pitch.map(|target| (n.pitch, target)))
                        .collect()
                })
                .unwrap_or_default()
        };

        let plain = sliding(&generate(&with(0.0), &c, 44));
        let whooped = sliding(&generate(&with(1.0), &c, 44));
        assert!(!plain.is_empty(), "the model slides at all");
        assert_eq!(plain.len(), whooped.len(), "the whoop adds no slide");
        assert_eq!(
            plain[..plain.len() - 1],
            whooped[..whooped.len() - 1],
            "and moves nothing but the phrase's last one"
        );

        let (from, to) = *whooped.last().expect("checked non-empty");
        assert_eq!(to, from + 12, "which rises an octave");
        assert_ne!(
            *plain.last().expect("checked non-empty"),
            (from, to),
            "instead of going where `slideIntervals` was sending it"
        );
    }

    // ── TASK-140: the percussion lanes ──────────────────────────────────────

    #[test]
    fn a_model_gets_the_perc_lanes_it_asked_for() {
        // uk-drill's own authored block, verbatim. Before TASK-140 this wrote
        // nothing at all: `rim` resolved to a lane no generator touched, and
        // `woodblock` was not a `Lane` variant, so `lane_by_name` said `None`.
        let m = model(json!({
            "percs": {
                "lanes": ["rim", "woodblock"],
                "densityPerBar": [1, 3],
                "placement": "offbeat",
                "gainOffsetDb": -12,
            }
        }));
        let lanes = generate(&m, &ctx(4), 7);

        assert!(!starts(&lanes, Lane::Rim).is_empty(), "rim was asked for");
        assert!(
            !starts(&lanes, Lane::Woodblock).is_empty(),
            "woodblock was asked for, and used not to be a lane at all"
        );
    }

    #[test]
    fn a_lane_the_model_did_not_name_stays_silent() {
        let m = model(json!({
            "percs": { "lanes": ["rim"], "densityPerBar": [2, 2] }
        }));
        let lanes = generate(&m, &ctx(2), 3);

        assert!(!starts(&lanes, Lane::Rim).is_empty());
        for quiet in [Lane::Shaker, Lane::Tambourine, Lane::Cowbell, Lane::Tom] {
            assert!(
                lane(&lanes, quiet).is_none(),
                "{quiet:?} was never named and must not appear"
            );
        }
    }

    #[test]
    fn only_percussion_lanes_can_be_named_as_percs() {
        // Naming the kick here would put two stages in charge of one voice —
        // the kick grammar writes it, and this would sprinkle over the top.
        let m = model(json!({
            "kick": { "grammar": ["1"] },
            "percs": { "lanes": ["kick", "snare", "closedHat"], "densityPerBar": [4, 4] }
        }));
        let with_percs = generate(&m, &ctx(2), 5);

        let without = model(json!({ "kick": { "grammar": ["1"] } }));
        let plain = generate(&without, &ctx(2), 5);

        assert_eq!(
            starts(&with_percs, Lane::Kick),
            starts(&plain, Lane::Kick),
            "naming the kick as a perc must be ignored, not doubled"
        );
    }

    #[test]
    fn an_offbeat_placement_never_lands_on_a_beat_in_any_meter() {
        // ⛔ **The meters are the test.** In 4/4 this passed while
        // `grid::is_downbeat` was `index % 4 == 0`, because there four 16ths
        // *are* a beat. In 6/8 a beat is two 16ths, so 2, 6 and 10 are beats
        // that the `% 4` waved through — and `uk-drill`, `ny-drill` and
        // `pop-smoke` all author `placement: "offbeat"`, so the layer meant to
        // sit between the pulse played on top of it. Asserted against
        // `ticks_per_beat` arithmetic spelled out here rather than against the
        // predicate under test, so a predicate that goes wrong again cannot
        // agree with the assertion about it.
        let m = model(json!({
            "percs": {
                "lanes": ["perc"],
                "densityPerBar": [3, 3],
                "placement": "offbeat",
            }
        }));

        // ⚠ **x/16 and x/32 are in here now**, and they are the two the first
        // cut of this list left out — which is exactly where the placement
        // filter emptied the pool and silenced the lane.
        for (num, den) in [(4, 4), (3, 4), (6, 8), (12, 8), (5, 4), (5, 16), (8, 32)] {
            let c = SessionContext {
                bars: 4,
                time_sig_num: num,
                time_sig_den: den,
                ..Default::default()
            };
            let beat = grid::ticks_per_beat(&c);
            let hits = starts(&generate(&m, &c, 11), Lane::Perc);
            // ⛔ **The lane sounds in every meter, and this half is the one that
            // was missing.** In x/16 and x/32 a beat is a 16th or shorter, so
            // *every* position on this grid is a beat and "offbeat" cannot be
            // honoured — the filter collected nothing and the lane went silent.
            assert!(!hits.is_empty(), "{num}/{den}: three a bar were asked for");

            // ...and where the meter *can* express an offbeat, none of them
            // lands on a beat.
            if grid::sixteenths_per_beat(&c) < 2 {
                continue;
            }
            for tick in hits {
                assert!(
                    !(tick % c.ticks_per_bar()).is_multiple_of(beat),
                    "{num}/{den}: an offbeat perc landed on beat {} (tick {tick})",
                    (tick % c.ticks_per_bar()) / beat + 1
                );
            }
        }
    }

    #[test]
    fn a_hat_stream_keeps_its_ghost_notes_in_every_meter() {
        // ⛔⛔ **`is_main_position` was `is_downbeat || is_offbeat_eighth`, and
        // once those became meter-aware their union covered every 16th.** In
        // 6/8 that made every hat a "main", so the `Articulation::Ghost` tier
        // and every model's authored `hihat.velocities.ghost` band became
        // unreachable — a machine-gun hat at one flat velocity. 4/4 was
        // unaffected, which is why only a meter sweep can see it.
        let m = model(json!({
            "hihat": {
                "base": "16th",
                "continuous": true,
                "fillDensity": 1.0,
                "velocities": { "main": [0.9, 1.0], "ghost": [0.2, 0.3] }
            }
        }));

        for (num, den) in [(4, 4), (6, 8), (12, 8), (3, 4)] {
            let c = SessionContext {
                bars: 2,
                time_sig_num: num,
                time_sig_den: den,
                ..Default::default()
            };
            let lanes = generate(&m, &c, 5);
            let hats: Vec<&Note> = lanes
                .iter()
                .filter(|track| track.lane == Lane::ClosedHat)
                .flat_map(|track| track.notes.iter())
                .collect();
            assert!(!hats.is_empty(), "{num}/{den}: no hats at all");

            let ghosts = hats
                .iter()
                .filter(|note| note.articulation == Some(Articulation::Ghost))
                .count();
            assert!(
                ghosts > 0,
                "{num}/{den}: every hat came out a main — the ghost tier is unreachable"
            );
            assert!(
                ghosts < hats.len(),
                "{num}/{den}: every hat came out a ghost"
            );
        }
    }

    #[test]
    fn one_voice_never_lands_twice_on_the_same_sixteenth() {
        // Drawn without replacement: two hits of one voice on one 16th read as
        // a single hit, so the authored density would quietly under-deliver.
        let m = model(json!({
            "percs": { "lanes": ["shaker"], "densityPerBar": [6, 6] }
        }));
        let lanes = generate(&m, &ctx(4), 13);
        let hits = starts(&lanes, Lane::Shaker);

        let mut unique = hits.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(hits.len(), unique.len(), "a 16th was used twice");
    }

    #[test]
    fn a_gain_offset_pulls_the_layer_under_the_kit() {
        let loud = model(json!({
            "percs": { "lanes": ["perc"], "densityPerBar": [4, 4] }
        }));
        let quiet = model(json!({
            "percs": { "lanes": ["perc"], "densityPerBar": [4, 4], "gainOffsetDb": -12 }
        }));

        let peak = |m: &StyleModel| {
            lane(&generate(m, &ctx(2), 4), Lane::Perc)
                .map(|l| l.notes.iter().map(|n| n.vel).max().unwrap_or(0))
                .unwrap_or(0)
        };
        assert!(
            peak(&quiet) < peak(&loud),
            "-12 dB must be audibly under the unattenuated layer"
        );
    }

    #[test]
    fn a_steady_tambourine_is_a_pulse_and_a_mirrored_one_doubles_the_clap() {
        // country-train's form: straight 8ths.
        let steady = model(json!({ "percs": { "tambourine": true } }));
        let lanes = generate(&steady, &ctx(1), 2);
        assert_eq!(
            starts(&lanes, Lane::Tambourine),
            vec![0, 480, 960, 1440, 1920, 2400, 2880, 3360],
            "eight 8ths in a 4/4 bar"
        );

        // west-coast-club's form: wherever the clap went.
        let mirrored = model(json!({
            "snare": { "placement": "backbeat_24", "layerClapOffsetMs": 8 },
            "percs": { "tambourineMirrorsClap": true }
        }));
        let lanes = generate(&mirrored, &ctx(2), 2);
        assert_eq!(
            starts(&lanes, Lane::Tambourine),
            starts(&lanes, Lane::Clap),
            "the tambourine must land exactly where the clap did"
        );
        assert!(!starts(&lanes, Lane::Clap).is_empty(), "there was a clap");
    }

    #[test]
    fn the_off_snare_is_its_own_lane_and_not_more_main_snare() {
        let m = model(json!({
            "snare": {
                "placement": "halftime_3",
                "offSnare": { "pos": ["4&"], "prob": 1.0 }
            }
        }));
        let lanes = generate(&m, &ctx(2), 9);

        assert_eq!(
            starts(&lanes, Lane::Snare),
            vec![1920, 5760],
            "the main snare is untouched"
        );
        assert_eq!(
            starts(&lanes, Lane::OffSnare),
            vec![3360, 7200],
            "the off-snare is on 4& of each bar, in its own lane"
        );
    }

    #[test]
    fn percs_moves_no_note_in_any_other_lane() {
        // ⛔ THE PROPERTY THAT MAKES TASK-140 SAFE TO LAND BEFORE THE ROSTER.
        // 15 of the 30 shipped models already author a `percs` block. If this
        // stage shared an RNG stream with anything above it, reading their
        // block for the first time would silently rewrite every beat they
        // produce — and the collision gate would then be measuring a roster
        // that had changed underneath it for reasons nobody intended.
        let base = json!({
            "kick": { "grammar": ["1", "2&", "4"] },
            "snare": { "placement": "halftime_3", "layerClapOffsetMs": 10 },
            "hihat": { "base": "16th" },
            "bassline": { "glideProb": 0.3 },
        });

        let mut with_percs = base.clone();
        with_percs["percs"] = json!({
            "lanes": ["rim", "shaker", "cowbell"],
            "densityPerBar": [2, 4],
        });

        for seed in [0, 1, 7, 42, 1337] {
            let plain = generate(&model(base.clone()), &ctx(4), seed);
            let perced = generate(&model(with_percs.clone()), &ctx(4), seed);

            for untouched in [
                Lane::Kick,
                Lane::Snare,
                Lane::Clap,
                Lane::ClosedHat,
                Lane::OpenHat,
                Lane::Sub,
            ] {
                assert_eq!(
                    starts(&plain, untouched),
                    starts(&perced, untouched),
                    "seed {seed}: adding percs moved {untouched:?}"
                );
            }
        }
    }

    #[test]
    fn placements_parse_from_the_names_the_dataset_uses() {
        assert_eq!(
            SnarePlacement::parse("halftime_3"),
            Some(SnarePlacement::Halftime3)
        );
        assert_eq!(
            SnarePlacement::parse("backbeat_24"),
            Some(SnarePlacement::Backbeat24)
        );
        assert_eq!(
            SnarePlacement::parse("drill_3_4"),
            Some(SnarePlacement::Drill34)
        );
        assert_eq!(
            SnarePlacement::parse("train_16ths"),
            Some(SnarePlacement::Train16ths)
        );
        assert_eq!(SnarePlacement::parse("halftime"), None);
    }

    #[test]
    fn a_halftime_snare_plays_beat_three_and_nothing_else() {
        let m = model(json!({ "snare": { "placement": "halftime_3" } }));
        let lanes = generate(&m, &ctx(4), 1);
        assert_eq!(starts(&lanes, Lane::Snare), vec![1920, 5760, 9600, 13440]);
    }

    #[test]
    fn a_backbeat_snare_plays_two_and_four() {
        let m = model(json!({ "snare": { "placement": "backbeat_24" } }));
        let lanes = generate(&m, &ctx(1), 1);
        assert_eq!(starts(&lanes, Lane::Snare), vec![960, 2880]);
    }

    #[test]
    fn the_drill_two_bar_snare_moves_from_three_to_four() {
        let m = model(json!({ "snare": { "placement": "drill_3_4" } }));
        let lanes = generate(&m, &ctx(4), 1);
        assert_eq!(
            starts(&lanes, Lane::Snare),
            vec![1920, 3840 + 2880, 9600, 11520 + 2880]
        );
    }

    #[test]
    fn a_train_beat_is_a_sixteenth_stream_with_an_accented_backbeat() {
        let m = model(json!({ "snare": { "placement": "train_16ths" } }));
        let lanes = generate(&m, &ctx(1), 1);
        let snare = lane(&lanes, Lane::Snare).unwrap();
        assert_eq!(snare.notes.len(), 16);

        let accents: Vec<u32> = snare
            .notes
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Accent))
            .map(|n| n.start_tick)
            .collect();
        assert_eq!(accents, vec![960, 2880], "the backbeat carries the accents");
        // And the accents are actually louder, not merely labelled.
        let quietest_accent = snare
            .notes
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Accent))
            .map(|n| n.vel)
            .min()
            .unwrap();
        let loudest_ghost = snare
            .notes
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Ghost))
            .map(|n| n.vel)
            .max()
            .unwrap();
        assert!(quietest_accent > loudest_ghost);
    }

    #[test]
    fn an_unknown_placement_falls_back_to_the_backbeat() {
        // It must not vanish: a pattern with no snare is silent in a way that
        // reads as "the generator is broken" rather than "the model is".
        let m = model(json!({ "snare": { "placement": "sideways" } }));
        let lanes = generate(&m, &ctx(1), 1);
        assert_eq!(starts(&lanes, Lane::Snare), vec![960, 2880]);
    }

    #[test]
    fn ghost_snares_answer_the_backbeat_at_the_stated_position() {
        let m = model(json!({
            "snare": {
                "placement": "halftime_3",
                "ghost": { "prob": 1.0, "pos": ["4&"], "vel": [0.45, 0.45] }
            }
        }));
        let lanes = generate(&m, &ctx(1), 1);
        let snare = lane(&lanes, Lane::Snare).unwrap();

        let ghosts: Vec<&Note> = snare
            .notes
            .iter()
            .filter(|n| n.articulation == Some(Articulation::Ghost))
            .collect();
        assert_eq!(ghosts.len(), 1);
        assert_eq!(ghosts[0].start_tick, 3360, "and-of-4");
        // 45% of full velocity, as the model states — not the generic ghost
        // tier, which is quieter.
        assert_eq!(ghosts[0].vel, 57);
    }

    #[test]
    fn a_ghost_probability_of_zero_produces_none() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "ghost": { "prob": 0.0, "pos": ["4&"] } }
        }));
        let lanes = generate(&m, &ctx(8), 1);
        let snare = lane(&lanes, Lane::Snare).unwrap();
        assert!(snare
            .notes
            .iter()
            .all(|n| n.articulation != Some(Articulation::Ghost)));
    }

    #[test]
    fn a_clap_layers_a_few_milliseconds_off_the_snare() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "layerClapOffsetMs": [5, 5] }
        }));
        let lanes = generate(&m, &ctx(1), 1);
        let snare = starts(&lanes, Lane::Snare);
        let clap = starts(&lanes, Lane::Clap);

        assert_eq!(snare.len(), clap.len());
        // 5 ms at 140 BPM is 11 ticks — audible as thickness, not as a flam.
        assert_eq!(clap[0] - snare[0], 11);
    }

    #[test]
    fn a_model_with_no_clap_offset_grows_no_clap_lane() {
        let m = model(json!({ "snare": { "placement": "halftime_3" } }));
        let lanes = generate(&m, &ctx(1), 1);
        assert!(lane(&lanes, Lane::Clap).is_none(), "no empty lanes");
    }

    #[test]
    fn an_off_grid_snare_is_displaced_by_the_stated_milliseconds() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "offGridMs": [6, 6] }
        }));
        let lanes = generate(&m, &ctx(1), 1);
        // 6 ms at 140 BPM is 13 ticks, late.
        assert_eq!(starts(&lanes, Lane::Snare), vec![1920 + 13]);
    }

    #[test]
    fn a_negative_off_grid_snare_pulls_it_early() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "offGridMs": -6.0 }
        }));
        let lanes = generate(&m, &ctx(1), 1);
        assert_eq!(starts(&lanes, Lane::Snare), vec![1920 - 13]);
    }

    #[test]
    fn an_explicit_kick_grammar_reproduces_exactly_and_cycles() {
        // Drill's authored two-bar form. This is the genre's signature and must
        // come out identical every time, not approximately.
        let m = model(json!({
            "snare": { "placement": "halftime_3" },
            "kick": { "fourBarGrammar": [["1", "2&", "4"], ["1&", "3"]] }
        }));
        let lanes = generate(&m, &ctx(4), 7);
        assert_eq!(
            starts(&lanes, Lane::Kick),
            vec![
                0,
                1440,
                2880, // bar 1: 1, 2&, 4
                3840 + 480,
                3840 + 1920, // bar 2: 1&, 3
                7680,
                7680 + 1440,
                7680 + 2880, // bar 3 repeats bar 1
                11520 + 480,
                11520 + 1920,
            ]
        );
    }

    #[test]
    fn the_explicit_grammar_does_not_drift_with_the_seed() {
        let m = model(json!({
            "kick": { "fourBarGrammar": [["1", "2&", "4"]] }
        }));
        let first = starts(&generate(&m, &ctx(2), 1), Lane::Kick);
        let second = starts(&generate(&m, &ctx(2), 9_999), Lane::Kick);
        assert_eq!(first, second);
    }

    #[test]
    fn anchors_are_always_played() {
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 3, "syncopation": 0.9 }
        }));
        for seed in 0..40 {
            let lanes = generate(&m, &ctx(2), seed);
            let kicks = starts(&lanes, Lane::Kick);
            assert!(kicks.contains(&0), "seed {seed}: bar 1 lost its anchor");
            assert!(kicks.contains(&3840), "seed {seed}: bar 2 lost its anchor");
        }
    }

    #[test]
    fn density_decides_how_many_kicks_a_bar_gets() {
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 5, "syncopation": 0.5 }
        }));
        for seed in 0..25 {
            let lanes = generate(&m, &ctx(1), seed);
            assert_eq!(starts(&lanes, Lane::Kick).len(), 5, "seed {seed}");
        }
    }

    #[test]
    fn a_density_wider_than_the_bar_stops_rather_than_spinning() {
        // A model error, but it must fail as "the bar is full", not as a hang.
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 99 }
        }));
        let lanes = generate(&m, &ctx(1), 1);
        assert_eq!(starts(&lanes, Lane::Kick).len(), 16);
    }

    #[test]
    fn the_offbeat_share_is_the_share_that_lands_offbeat() {
        // Drill's "roughly 40% of kicks land on offbeat 8ths" is a statistic
        // about the output, so it is checked as one.
        let m = model(json!({
            "kick": { "anchors": [], "densityPerBar": 4, "offbeat8thShare": 0.4,
                      "syncopation": 0.4 }
        }));
        let (mut offbeat, mut total) = (0, 0);
        for seed in 0..200 {
            for tick in starts(&generate(&m, &ctx(2), seed), Lane::Kick) {
                let index = (tick % 3840) / grid::SIXTEENTH;
                if grid::is_offbeat_eighth(index, &ctx(2)) {
                    offbeat += 1;
                }
                total += 1;
            }
        }
        let share = offbeat as f64 / total as f64;
        assert!(
            (0.33..=0.47).contains(&share),
            "asked for 40% offbeat, got {share:.3}"
        );
    }

    #[test]
    fn a_zero_offbeat_share_keeps_every_kick_on_the_beat() {
        // Authored zero and absent must not mean the same thing.
        let m = model(json!({
            "kick": { "anchors": ["1"], "densityPerBar": 4, "offbeat8thShare": 0.0,
                      "syncopation": 0.0 }
        }));
        for seed in 0..30 {
            for tick in starts(&generate(&m, &ctx(1), seed), Lane::Kick) {
                assert!(
                    grid::is_downbeat(tick / grid::SIXTEENTH, &ctx(1)),
                    "seed {seed}: {tick} is off the beat"
                );
            }
        }
    }

    #[test]
    fn tresillo_bias_leans_the_kick_onto_the_three_three_two() {
        let count_tresillo = |bias: f64| {
            let m = model(json!({
                "kick": { "anchors": [], "densityPerBar": 3, "syncopation": 0.5,
                          "tresilloBias": bias }
            }));
            let mut hits = 0;
            for seed in 0..150 {
                for tick in starts(&generate(&m, &ctx(1), seed), Lane::Kick) {
                    if grid::is_tresillo(tick / grid::SIXTEENTH) {
                        hits += 1;
                    }
                }
            }
            hits
        };

        let flat = count_tresillo(0.0);
        let leaning = count_tresillo(1.0);
        assert!(
            leaning > flat + 40,
            "a full tresillo bias should be obvious: {flat} vs {leaning}"
        );
    }

    #[test]
    fn no_kick_sits_in_the_gap_before_the_snare() {
        // Research ch. 1 §1: leave an 8th before the beat-3 snare. This is the
        // rule that makes a trap kick pattern breathe.
        let m = model(json!({
            "snare": { "placement": "halftime_3" },
            "kick": { "anchors": ["1"], "densityPerBar": 6, "syncopation": 0.8,
                      "avoidPreSnareGap": "8th" }
        }));
        for seed in 0..60 {
            for tick in starts(&generate(&m, &ctx(2), seed), Lane::Kick) {
                let within_bar = tick % 3840;
                assert!(
                    !(1440..1920).contains(&within_bar),
                    "seed {seed}: kick at {within_bar} is inside the pre-snare 8th"
                );
            }
        }
    }

    #[test]
    fn without_the_gap_rule_kicks_do_land_there() {
        // The control for the test above: if nothing ever landed in that window
        // anyway, the rule would be untested and the assertion meaningless.
        let m = model(json!({
            "snare": { "placement": "halftime_3" },
            "kick": { "anchors": ["1"], "densityPerBar": 6, "syncopation": 0.8 }
        }));
        let landed = (0..60).any(|seed| {
            starts(&generate(&m, &ctx(2), seed), Lane::Kick)
                .iter()
                .any(|tick| (1440..1920).contains(&(tick % 3840)))
        });
        assert!(landed, "the gap window is reachable without the rule");
    }

    #[test]
    fn the_and_of_four_lead_in_only_happens_every_other_bar() {
        let m = model(json!({
            "snare": { "placement": "halftime_3" },
            "kick": { "anchors": ["1"], "densityPerBar": 1, "syncopation": 0.0,
                      "andOf4EveryOtherBar": 1.0 }
        }));
        let lanes = generate(&m, &ctx(4), 3);
        let kicks = starts(&lanes, Lane::Kick);
        assert_eq!(kicks, vec![0, 3840, 3840 + 3360, 7680, 11520, 11520 + 3360]);
    }

    #[test]
    fn generation_is_reproducible_and_seed_dependent() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "ghost": { "prob": 0.5, "pos": ["4&"] } },
            "kick": { "anchors": ["1"], "densityPerBar": [2, 5], "syncopation": 0.5 }
        }));
        let a = generate(&m, &ctx(4), 4242);
        let b = generate(&m, &ctx(4), 4242);
        let c = generate(&m, &ctx(4), 4243);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn rerolling_the_snare_does_not_move_the_kick() {
        // Different snare grammar, same seed: the kick must be untouched. This
        // is what lane locking will rest on (US-003).
        let kick = json!({ "anchors": ["1"], "densityPerBar": 4, "syncopation": 0.5 });
        let a = model(json!({ "kick": kick, "snare": { "placement": "backbeat_24" } }));
        let b = model(json!({
            "kick": kick,
            "snare": { "placement": "backbeat_24", "ghost": { "prob": 1.0, "pos": ["4&"] } }
        }));
        assert_eq!(
            starts(&generate(&a, &ctx(4), 88), Lane::Kick),
            starts(&generate(&b, &ctx(4), 88), Lane::Kick)
        );
    }

    #[test]
    fn lanes_come_out_in_order_and_never_empty() {
        let m = model(json!({
            "snare": { "placement": "halftime_3", "layerClapOffsetMs": 5 },
            "kick": { "anchors": ["1"], "densityPerBar": 3 }
        }));
        let lanes = generate(&m, &ctx(2), 5);

        let order: Vec<Lane> = lanes.iter().map(|l| l.lane).collect();
        assert_eq!(order, vec![Lane::Kick, Lane::Snare, Lane::Clap]);
        for track in &lanes {
            assert!(!track.notes.is_empty());
            let mut sorted: Vec<u32> = track.notes.iter().map(|n| n.start_tick).collect();
            let original = sorted.clone();
            sorted.sort_unstable();
            assert_eq!(original, sorted, "{:?} is out of order", track.lane);
        }
    }

    #[test]
    fn every_note_carries_its_lanes_gm_voice_and_a_playable_length() {
        let m = model(json!({
            "snare": { "placement": "backbeat_24", "layerClapOffsetMs": 4 },
            "kick": { "anchors": ["1"], "densityPerBar": 3 }
        }));
        for track in generate(&m, &ctx(2), 6) {
            for n in &track.notes {
                assert_eq!(n.pitch, gm_drum_note(track.lane));
                assert!(n.len_ticks > 0);
                assert!(n.vel >= 1 && n.vel <= 127);
            }
        }
    }

    #[test]
    fn a_model_with_no_drums_block_produces_a_backbeat_rather_than_silence() {
        let m: StyleModel =
            serde_json::from_value(json!({ "id": "bare", "type": "genre", "name": "Bare" }))
                .unwrap();
        let lanes = generate(&m, &ctx(1), 1);
        assert_eq!(starts(&lanes, Lane::Snare), vec![960, 2880]);
        assert!(!starts(&lanes, Lane::Kick).is_empty());
    }

    #[test]
    fn the_pattern_stays_inside_its_own_bars() {
        let m = model(json!({
            "snare": { "placement": "backbeat_24", "ghost": { "prob": 1.0, "pos": ["4&"] } },
            "kick": { "anchors": ["1"], "densityPerBar": 5, "syncopation": 0.7,
                      "andOf4EveryOtherBar": 1.0 }
        }));
        let context = ctx(4);
        let total = context.total_ticks();
        for seed in 0..50 {
            for track in generate(&m, &context, seed) {
                for n in &track.notes {
                    assert!(
                        n.start_tick < total,
                        "seed {seed}: {:?} at {} is past the pattern",
                        track.lane,
                        n.start_tick
                    );
                }
            }
        }
    }
}
