//! Rendering one part, with the four others it depends on.
//!
//! ⛔ **The melodic parts are not independent, and the order below is the
//! dependency rather than a preference.** A melody is written against the
//! harmony and around the drums; a countermelody answers the melody; a bassline
//! follows the harmony and locks to the kick. Generating one in isolation
//! produces notes in the right key that fit nothing — which is the difference
//! between five parts and one part played five times.
//!
//! This lived in `plugin/src/bridge.rs` and had a comment on it saying *"a
//! change here that is not a change there is a change nobody meant"*. Song Mode
//! needs the identical order for every section it builds, so the two copies
//! became one: the pattern path and the song path now render through this
//! function or they do not render at all. A dependency order duplicated across
//! two call sites is the drift class this project has already been bitten by
//! three times.

use crate::context::SessionContext;
use crate::dataset::StyleModel;
use crate::generators::{bass, chords, counter, drums, melody};
use crate::humanize::humanize;
use crate::novelty;
use crate::pattern::{LaneTrack, Part};

/// The parts a take is written **against**, and therefore what a press on it
/// should leave on screen.
///
/// ⛔⛔ **This graph used to live in TypeScript** — `session.ts`'s `UPSTREAM`, a
/// hand-maintained copy of what `upstream` below actually does, with no type or
/// test tying the two together (TASK-166).
///
/// ⛔ **Drums are deliberately not here, and this is the line a future reader
/// will want to "fix".** A melody is phrased around a *reference* kit at the
/// song seed, not the drum take on screen; filling the Drums tab from a melody
/// press would put a kit there that is neither the one the melody was written
/// against nor the one Generate on Drums would produce.
fn dependents(part: Part) -> &'static [Part] {
    match part {
        Part::Drums | Part::Chords => &[],
        Part::Melody | Part::Bass => &[Part::Chords],
        // In this order, because it is the order a producer watches the tabs
        // fill: a countermelody landing before the melody it answers reads as a
        // bug even when the notes are right.
        Part::Counter => &[Part::Chords, Part::Melody],
    }
}

/// Whether the novelty screen should hold this part to the guard.
///
/// ⛔⛔ **TASK-169: the parameter is the DECISION, not the input to it.**
/// `novelty::screen` takes a `bass_follows_the_kick` flag that is meaningless
/// for every part except the bass, and its own doc says callers that cannot
/// answer should pass `true`. So `arrange.rs` passed a literal `true` for Melody
/// and Counter — at a site that **has `model` in scope** and could have answered
/// properly — while `parts.rs` and the Bass arm each spelled the real rule a
/// third and fourth time. The next part added to `render_section` would have
/// been copied off the melody line and arrived with `true` baked in.
///
/// ⚠ **The model deliberately does not go into `novelty.rs`.** That module
/// depends on nothing from `dataset` and is the reason the guard can be tested
/// without a roster; this is the seam where a model is allowed to be known.
///
/// ⚠ `true` for a non-bass part is not a shrug — it is the honest answer. The
/// flag only ever gates the bass arm of [`novelty::screens`], so every other
/// part is screened unconditionally.
pub fn screenable(model: &StyleModel, part: Part) -> bool {
    part != Part::Bass || bass::follows_the_kick(model)
}

/// Generate `part` on the grid, screen it, and apply feel — in the one order
/// that is correct.
///
/// The parts a caller does not ask for are still generated, because they are
/// its inputs. That is not a wasted cache miss *on this path*: every generator
/// is a pure function of `(model, ctx, seed)`, and a single-pattern request
/// gives all five parts one seed — so the harmony computed for a melody is
/// byte-for-byte the harmony a `Chords` request returns for the same seed.
/// `engine/tests/coherence.rs` builds all five in exactly this order and is what
/// proves they belong in the same record.
///
/// ⛔ **That argument holds only while one seed serves the whole request.** Song
/// Mode builds many parts per section and would repeat every dependency here, so
/// it does not call this in a loop — `arrange::render_section` derives the
/// section's inputs once and hands them down. An earlier cut of it did call this
/// per part *with a per-part seed*, which made each melody's internal harmony a
/// different voicing from the chords beside it; both clips were individually
/// correct and the pair had never been written against each other.
///
/// Returns lanes that may be **empty**, and that is a real answer rather than a
/// failure: a style whose 808 *is* the bassline authors no separate bass part on
/// purpose. Deciding what an empty part means belongs to the caller — a pattern
/// request says so to the producer, and Song Mode simply leaves the part out of
/// the section.
///
/// ⛔ **The novelty guard sits between the notes and the feel, and both sides
/// of that are deliberate** (FR-011, TASK-039).
///
/// - **After generation**, because there is nothing to screen until there are
///   notes.
/// - **Before humanising**, because humanising is the expensive-to-throw-away
///   half: a take the guard rejects would have had its feel computed for
///   nothing, three times over.
///
/// A rejected take is redrawn at a *derived* seed, so [`render`] stays a pure
/// function of its inputs and a saved seed still rebuilds the pattern the
/// producer heard. `novelty::screen` hands back the seed that survived, and the
/// feel is taken from **that** — humanising the fourth take with the first
/// take's seed would give two different performances one shared set of jitter.
pub fn render(
    model: &StyleModel,
    ctx: &SessionContext,
    seeds: Seeds,
    part: Part,
) -> Vec<LaneTrack> {
    render_against(model, ctx, seeds, part).0
}

/// As [`render`], plus **the parts the take was written against**.
///
/// ⛔ **A sibling rather than a wider `render`** (TASK-166). Twelve callers —
/// every one of them a test, plus `arrange.rs` which shares its upstream through
/// `Carry` instead — want the notes and nothing else, and making them all
/// destructure a tuple to ignore its second half would be churn that hides which
/// call site actually cares. The bridge is the one caller that does.
pub fn render_against(
    model: &StyleModel,
    ctx: &SessionContext,
    seeds: Seeds,
    part: Part,
) -> (Vec<LaneTrack>, Vec<(Part, Vec<LaneTrack>)>) {
    // ⛔ **The bass is screened unless it is locked to the kick** — see
    // `novelty::screens`. Asked here because this is where the model is.
    //
    // ⚠ **Asked only for the bass.** `novelty::screens` reads the flag in its
    // `Part::Bass` arm and nowhere else, and `true` is already its documented
    // "leave it alone" answer for a caller that cannot know — so four presses in
    // five were paying for a value that was thrown away.
    let kick_locked = screenable(model, part);
    // ⛔ **Built once, outside the screen** (TASK-168). `novelty::screen` calls
    // this closure up to `MAX_RETRIES + 1` times and only `take` differs between
    // them, so building the harmony and the kit inside it generated each of them
    // four times over on a screened press — see [`Upstream`].
    let against = upstream(model, ctx, seeds, part);
    let (mut lanes, take, report) =
        novelty::screen(novelty::bundled(), part, kick_locked, seeds.part, |take| {
            against.take(model, ctx, take, part)
        });
    novelty::log(part, &report);
    // The *take's* feel, so two takes of one part breathe differently.
    humanize(&mut lanes, ctx, take);
    // ⛔⛔ **And the parts it was written against, rendered the way a request for
    // them would be** (TASK-166). Handing back `Upstream`'s own copies was
    // wrong twice, and a review caught both: that lead is a bare
    // `melody::generate`, so it has been through **neither** `novelty::screen`
    // — a melody matching a bundled known hook would have landed in the
    // producer's tab and been exported — **nor** `humanize`, so a filled clip
    // came back dead flat while pressing Generate on the same tab gave jittered
    // timing and scaled velocities.
    //
    // ▶ **The saving that matters is still taken.** The point was never the
    // arithmetic: it was three *synchronous* round trips, each served on the
    // webview thread by `with_custom_protocol`, each blocking the hosted DAW's
    // window. Those are gone. Generating a part properly here costs the engine
    // one pass and the producer nothing.
    //
    // ⚠ At the **record**, which is what `songSeed` means — a fill at the
    // caller's own take would be a different melody from the one it answers.
    let record = Seeds {
        part: seeds.song,
        ..seeds
    };
    let handed = dependents(part)
        .iter()
        .map(|dep| (*dep, render(model, ctx, record, *dep)))
        .collect();
    (lanes, handed)
}

/// The two seeds a part is generated from (TASK-141).
///
/// ⛔ **This exists because the Defect 2 fix had to give something up.** Defect 2
/// was *"Generate returns the same beat every press"*, caused by the seed box
/// echoing the engine's seed and the next press re-sending it. The fix was to
/// send `null` unless the seed is pinned — but the five parts are only
/// guaranteed to agree **when they share a seed**, so the ordinary workflow
/// (Generate on Drums, switch tab, Generate on Melody) drew two unrelated seeds
/// and wrote the melody against a *different harmonic plan* from the drums.
///
/// Two seeds buy both properties at once: a different take on every press, and
/// every take written against the same record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seeds {
    /// **The record.** Key, tempo and the harmonic plan — everything the five
    /// parts have to agree on. Changes when the producer asks for a new record.
    pub song: u64,
    /// **The take.** This part's own variation within that record, and its
    /// feel. Rerolled on every press of Generate.
    pub part: u64,
    /// **The drums' take**, when a drum pattern is already on screen.
    ///
    /// ⛔ **This exists because a bass that mirrors the kick has to mirror the
    /// kick the producer can hear.** `Rhythm::MirrorKick` — the trap and drill
    /// default — copies the kick's ticks verbatim, and the reference kit it read
    /// was generated at the *song* seed while the drums on screen came from
    /// their own take. Measured on the shipped roster: with one seed 13 of 13
    /// boom-bap bass notes landed on a real kick; with split seeds, 9 of 13, and
    /// `uk-drill` fell to 1 of 14. Two parts that are supposed to read as one
    /// instrument played twice were landing in different places.
    ///
    /// ⚠ **`None` is a real answer, not a missing value**: no drums have been
    /// generated yet, so there is no take to mirror and the song seed's
    /// canonical kit is the right kit. That is also what every pre-existing
    /// caller and every project saved before this field means.
    ///
    /// ⛔ **A seed rather than the lanes themselves, deliberately.** Handing the
    /// session's real notes in would stop [`render`] being a pure function of
    /// its seeds, which is the property that lets a saved seed rebuild a
    /// pattern at all. `drums::generate` is pure in `(model, ctx, seed)`, so the
    /// take seed reconstructs the same kick byte for byte.
    pub drums: Option<u64>,
}

impl Seeds {
    /// Both seeds the same — what a single-seed caller has always meant.
    ///
    /// ⚠ **This is the compatibility shape, and it is exactly correct.** Every
    /// existing caller, every saved project written before TASK-141, and
    /// `arrange::render_section` all mean "one seed for everything", which is
    /// the case where the two-seed design collapses back to the old behaviour.
    /// `drums` is `None` for the same reason: one seed means the reference kit
    /// *is* the kit on screen, so there is nothing separate to point at.
    pub fn shared(seed: u64) -> Self {
        Self {
            song: seed,
            part: seed,
            drums: None,
        }
    }

    /// The kit a part written against the drums should read.
    ///
    /// The drums' own take when there is one, and the record's canonical kit
    /// when there is not.
    fn drum_seed(self) -> u64 {
        self.drums.unwrap_or(self.song)
    }
}

/// What a take is written *against*, built **once** for a whole novelty screen.
///
/// ⛔⛔ **TASK-168: this was rebuilt on every retry, and none of it varies.**
/// `novelty::screen` calls its closure up to `MAX_RETRIES + 1` times and only
/// `take` changes between them — `song` and `drum_seed()` are fixed. So a
/// screened Bass press ran 4 chord generations and 4 full kit generations where
/// 1 of each was needed, and a Counter also re-derived the lead each time, all
/// synchronously on the DAW's UI thread.
///
/// ▶ `arrange::render_section` already had this right and its own doc says why:
/// *"calling it five times generates the harmony four times over… Building it
/// once and handing it down is the same notes and a third less work."*
/// `parts::render` was the outlier.
///
/// ⚠ **Same notes, not merely similar.** Every field here is a pure function of
/// inputs that do not change across retries, so hoisting them cannot alter what
/// is generated — only how many times it is generated. `golden.rs` is the check.
enum Upstream {
    /// Drums answer to nothing upstream: the kit IS the take.
    None,
    /// ⛔ The harmonic plan belongs to the RECORD, so it is generated at the song
    /// seed and a new take must not move it — `arrange.rs`'s
    /// `a_new_take_changes_the_part_and_leaves_the_record_alone` is the check,
    /// and it caught this being transcribed as the take seed.
    Chords { song: u64 },
    /// The harmony and the reference kit a melody is phrased around.
    Melodic {
        harmony: chords::Chords,
        kit: Vec<LaneTrack>,
    },
    /// The harmony and the *song's* lead a countermelody answers.
    Counter {
        harmony: chords::Chords,
        lead: LaneTrack,
    },
    /// The harmony, and the drums' own take — see [`Seeds::drums`].
    Bass {
        harmony: chords::Chords,
        kit: Vec<LaneTrack>,
    },
}

/// ⛔ **Which seed each line takes is the whole design, so read this before
/// changing one.**
///
/// - **Dependencies take the song seed.** The harmony every part is written
///   against, and the reference kit a melodic line phrases around, belong to
///   the *record*. That is what makes five parts generated at five different
///   moments still belong to one another.
/// - **The part's own generator takes the part seed.** That is the take, and it
///   is what rerolls on every press.
///
/// ⚠ **`Part::Chords` takes the SONG seed, and that is not an oversight.** The
/// harmonic plan *is* the record — if it rolled per take, every other part
/// would be written against a progression that is no longer on screen, which is
/// the exact defect this task exists to fix. A new progression comes from
/// asking for a new record, not from rerolling the chords tab.
///
/// ⛔ **The `kit` a melody phrases around is `drums::generate` at the SONG
/// seed** — a canonical reference kit, deliberately *not* the drum pattern
/// currently on screen (which was generated at its own part seed). Two other
/// answers were considered and rejected: handing the session's real parts in
/// would stop `render` being a pure function of its seeds, which is what makes
/// it reproducible from a saved seed at all; and anchoring drums to the song
/// seed would stop the drums varying per press, which is the defect Mike
/// reported in the first place. Melodies key off the kick's *placement
/// grammar*, which is a property of the model and the song seed rather than of
/// one particular take.
///
/// ⛔⛔ **The BASS is the exception, and the difference is literal versus
/// grammatical.** `bassline.rhythm = "mirror_kick"` — the default, and what
/// most of the trap and drill roster authors — copies the kick's ticks one for
/// one. A reference kit is the right input for "phrase around the kick" and the
/// wrong one for "play exactly where the kick plays": it put bass notes on
/// kicks that are not in the pattern. So `Part::Bass` reads
/// [`Seeds::drum_seed`] instead, which is still a *seed* — purity is kept, and
/// only the number changes.
///
/// ⚠ **`arrange::render_section` solves the same problem differently and must
/// stay that way.** Song Mode renders all five parts together, so it can share
/// one already-generated kit through `Carry` — the real one the section plays.
/// Its own doc records what happened when an earlier cut called `parts::render`
/// per part with a per-part seed: *"both clips were individually correct and
/// the pair had never been written against each other."* That is the failure
/// this must not reintroduce, and sharing the song seed is what prevents it.
fn upstream(model: &StyleModel, ctx: &SessionContext, seeds: Seeds, part: Part) -> Upstream {
    let song = seeds.song;
    match part {
        Part::Drums => Upstream::None,
        Part::Chords => Upstream::Chords { song },
        Part::Melody => Upstream::Melodic {
            harmony: chords::generate(model, ctx, song),
            kit: drums::generate(model, ctx, song),
        },
        Part::Counter => {
            let harmony = chords::generate(model, ctx, song);
            let kit = drums::generate(model, ctx, song);
            // ⚠ The lead the counter answers is the **song's** lead, not this
            // take's. A countermelody written against a melody nobody has
            // generated yet still has to sit against the one they will.
            let lead = melody::generate(model, ctx, song, &harmony, &kit);
            Upstream::Counter { harmony, lead }
        }
        Part::Bass => Upstream::Bass {
            harmony: chords::generate(model, ctx, song),
            // ⛔ **The drums' own take, not the record's reference kit** — see
            // [`Seeds::drums`]. The bass is the one part that reads the kick
            // *literally*: `mirror_kick` copies its ticks, so a kit generated
            // at a different seed puts the bass on kicks nobody is playing.
            // Melody and Counter deliberately do not do this; they phrase around
            // the kick's grammar rather than copy it.
            kit: drums::generate(model, ctx, seeds.drum_seed()),
        },
    }
}

impl Upstream {
    /// One take, against parts that were built before the screen started.
    fn take(
        &self,
        model: &StyleModel,
        ctx: &SessionContext,
        take: u64,
        part: Part,
    ) -> Vec<LaneTrack> {
        match (self, part) {
            (_, Part::Drums) => drums::generate(model, ctx, take),
            (Upstream::Chords { song }, _) => vec![chords::generate(model, ctx, *song).track],
            (Upstream::Melodic { harmony, kit }, _) => {
                vec![melody::generate(model, ctx, take, harmony, kit)]
            }
            (Upstream::Counter { harmony, lead }, _) => {
                vec![counter::generate(model, ctx, take, harmony, lead)]
            }
            (Upstream::Bass { harmony, kit }, _) => {
                vec![bass::generate(model, ctx, take, harmony, kit)]
            }
            // ⚠ Unreachable by construction: `upstream` builds exactly one
            // variant per part.
            //
            // ⚠ **The compile error for a new `Part` comes from `upstream` and
            // `dependents`, not from here** — every arm above matches `part`
            // with `_`, so this one would happily absorb a new variant at
            // runtime. An earlier comment claimed otherwise; a review checked
            // it. Those two exhaustive matches are what actually force the
            // decision, and this is the shout if one of them is ever loosened.
            (Upstream::None, other) => panic!("{other:?} has no upstream variant"),
        }
    }
}

/// Did this part come out silent?
///
/// Every lane empty rather than no lanes at all is the shape the generators
/// return, so `lanes.is_empty()` is the wrong check and would report a silent
/// part as a sounding one.
pub fn is_silent(lanes: &[LaneTrack]) -> bool {
    lanes.iter().all(|lane| lane.notes.is_empty())
}

// Tested from `engine/tests/arrange.rs`, which is where the shipped dataset is
// reachable — every generator this dispatches to needs a resolved model, and
// `data/` is loaded by the integration suites rather than from inside the lib.
