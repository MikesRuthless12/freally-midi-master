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
use crate::pattern::{LaneTrack, Part};

/// Generate `part` on the grid and apply feel, in the one order that is correct.
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
pub fn render(
    model: &StyleModel,
    ctx: &SessionContext,
    seeds: Seeds,
    part: Part,
) -> Vec<LaneTrack> {
    let mut lanes = generate(model, ctx, seeds, part);
    // The *take's* feel, so two takes of one part breathe differently.
    humanize(&mut lanes, ctx, seeds.part);
    lanes
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

/// The notes, before feel is applied.
///
/// Split out from [`render`] because the humanizer must run exactly once over a
/// lane. Private: Song Mode needs the *dependencies* shared across a section
/// rather than one part at a time, so it has its own renderer
/// (`arrange::render_section`) and this has exactly one caller.
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
fn generate(model: &StyleModel, ctx: &SessionContext, seeds: Seeds, part: Part) -> Vec<LaneTrack> {
    // `drums` is deliberately not destructured: only one arm reads it, and it
    // reads it through `drum_seed` so the "or the record's kit" fallback is
    // spelled once rather than at the call site.
    let Seeds {
        song, part: take, ..
    } = seeds;
    match part {
        Part::Drums => drums::generate(model, ctx, take),
        Part::Chords => vec![chords::generate(model, ctx, song).track],
        Part::Melody => {
            let harmony = chords::generate(model, ctx, song);
            let kit = drums::generate(model, ctx, song);
            vec![melody::generate(model, ctx, take, &harmony, &kit)]
        }
        Part::Counter => {
            let harmony = chords::generate(model, ctx, song);
            let kit = drums::generate(model, ctx, song);
            // ⚠ The lead the counter answers is the **song's** lead, not this
            // take's. A countermelody written against a melody nobody has
            // generated yet still has to sit against the one they will.
            let lead = melody::generate(model, ctx, song, &harmony, &kit);
            vec![counter::generate(model, ctx, take, &harmony, &lead)]
        }
        Part::Bass => {
            let harmony = chords::generate(model, ctx, song);
            // ⛔ **The drums' own take, not the record's reference kit** — see
            // [`Seeds::drums`]. The bass is the one part that reads the kick
            // *literally*: `mirror_kick` copies its ticks, so a kit generated
            // at a different seed puts the bass on kicks nobody is playing.
            // Melody and Counter are directly below and deliberately do not do
            // this; they phrase around the kick's grammar rather than copy it.
            let kit = drums::generate(model, ctx, seeds.drum_seed());
            vec![bass::generate(model, ctx, take, &harmony, &kit)]
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
