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
use crate::pattern::{Articulation, Lane, LaneTrack, Note};
use crate::rng;
use crate::theory;

/// How long a drum hit is written for.
///
/// A one-shot's length is decided by its sample and its envelope, not by the
/// note — but a zero-length note is invalid in an SMF and invisible in a piano
/// roll, so drums get a 16th and the sampler ignores it.
const HIT_TICKS: u32 = grid::SIXTEENTH;

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

/// One bar of kick, placed from the grammar in the model.
fn kick_bar(
    kick: Option<&Value>,
    grammar: Option<&Vec<Vec<u32>>>,
    ctx: &SessionContext,
    bar: u32,
    snares: &[u32],
    rng: &mut impl Rng,
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
    let drums = model.blocks.get("drums");
    let tiers = VelocityTiers::from_json(drums);
    let mut kit = DrumKit::new();

    let snare_block = block(drums, "snare");
    let kick_block = block(drums, "kick");

    let mut snare_rng = rng::stream(seed, "drums/snare");
    let mut kick_rng = rng::stream(seed, "drums/kick");

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

    // A deliberate displacement, in milliseconds, that the genre is made of —
    // UK drill's snare sits off the grid on purpose. Negative pulls it early.
    let off_grid_ticks = optional_number(snare_block, "offGridMs", &mut snare_rng)
        .map(|ms| ctx.ms_to_ticks(ms as f32).round() as i64)
        .unwrap_or(0);

    let ghost = snare_block.and_then(|s| s.get("ghost"));
    let ghost_positions = strings(ghost, "pos");
    let clap_offset_ms = optional_number(snare_block, "layerClapOffsetMs", &mut snare_rng);

    let bar_ticks = ctx.ticks_per_bar();
    // Kept per bar because the roll engine's `pre_snare` position needs
    // something to be before.
    let mut snares_by_bar: Vec<Vec<u32>> = Vec::with_capacity(usize::from(ctx.bars));

    for bar in 0..u32::from(ctx.bars) {
        let bar_start = bar * bar_ticks;
        let hits = placement.hits(bar, ctx);

        for (offset, articulation) in &hits {
            let tick = displace(bar_start + offset, off_grid_ticks);
            kit.hit(
                Lane::Snare,
                tick,
                tiers.pick(*articulation, &mut snare_rng),
                *articulation,
            );

            // Layered clap a few milliseconds off the snare — the trap sound is
            // the two together, and the offset is what stops them phasing.
            if let Some(ms) = clap_offset_ms {
                let clap = displace(tick, ctx.ms_to_ticks(ms as f32).round() as i64);
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
        ) {
            kit.hit(
                Lane::Kick,
                bar_start + tick,
                tiers.pick(None, &mut kick_rng),
                None,
            );
        }
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
    let (mut closed, mut open) = hats(hihat, ctx, &tiers, &mut hat_rng);

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
            (closed, open) = hats(hihat, ctx, &tiers, &mut redraw);
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
    // needs both lanes finished before it can be placed.
    let kicks: Vec<u32> = kit.notes(Lane::Kick).iter().map(|n| n.start_tick).collect();
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
        bass808(drums, ctx, &kicks, &snares, &ringing_snares, &mut bass_rng),
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
    let mut lanes = kit.into_lanes();
    super::fit_to_clip(&mut lanes, ctx);
    lanes
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
    kicks: &[u32],
    // The backbeat: where the 808 does not play at all.
    snares: &[u32],
    // Every snare including a fill's, which the 808 may start on but must not
    // ring past. See the call site for why these are two lists.
    ringing_snares: &[u32],
    rng: &mut impl Rng,
) -> Vec<Note> {
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

    let kept: Vec<u32> = kicks
        .iter()
        .copied()
        .filter(|_| rng.random_bool(lock))
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
            articulation: Some(Articulation::Legato),
            reversed: false,
        });

        pitch = match slide_to {
            // The riff stays where it landed; the bassline goes home.
            Some(target) if counter_riff => target,
            _ => root,
        };
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

    let lanes: Vec<Lane> = strings(percs, "lanes")
        .iter()
        .filter_map(|name| lane_by_name(name))
        .filter(|lane| PERC_LANES.contains(lane))
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
    rng: &mut impl Rng,
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

    let velocities = hihat.and_then(|h| h.get("velocities"));
    let mut closed: Vec<Note> = Vec::new();
    let mut open: Vec<Note> = Vec::new();

    let bar_ticks = ctx.ticks_per_bar();
    let beat = grid::ticks_per_beat(ctx);
    let onsets = hat_base_onsets(&base, &grouping, ctx);
    let open_hat = block(hihat, "openHat");
    let positions = strings(open_hat, "pos");

    for bar in 0..u32::from(ctx.bars) {
        let bar_start = bar * bar_ticks;

        // Which beats play at all.
        let beats_played: Vec<u32> = (0..u32::from(ctx.time_sig_num.max(1)))
            .filter(|_| continuous || rng.random_bool(fill_density))
            .collect();

        let mut ticks: Vec<u32> = onsets
            .iter()
            .copied()
            .filter(|tick| beats_played.contains(&(tick / beat)))
            .collect();

        // A continuous stream fills the gaps between its onsets; the extras are
        // the quiet 16ths that make it breathe.
        if continuous {
            for index in 0..grid::sixteenths_per_bar(ctx) {
                let tick = index * grid::SIXTEENTH;
                if !ticks.contains(&tick) && rng.random_bool(fill_density) {
                    ticks.push(tick);
                }
            }
        }
        ticks.sort_unstable();
        ticks.dedup();

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
                closed.retain(|n| n.start_tick != tick);
                let vel = fractional_velocity(velocities, "main", rng)
                    .unwrap_or_else(|| tiers.pick(Some(Articulation::Accent), rng));
                open.push(note_at(
                    Lane::OpenHat,
                    tick,
                    vel,
                    Some(Articulation::Accent),
                ));
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
