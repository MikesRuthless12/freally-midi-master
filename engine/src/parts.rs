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
pub fn render(model: &StyleModel, ctx: &SessionContext, seed: u64, part: Part) -> Vec<LaneTrack> {
    let mut lanes = generate(model, ctx, seed, part);
    humanize(&mut lanes, ctx, seed);
    lanes
}

/// The notes, before feel is applied.
///
/// Split out from [`render`] because the humanizer must run exactly once over a
/// lane. Private: Song Mode needs the *dependencies* shared across a section
/// rather than one part at a time, so it has its own renderer
/// (`arrange::render_section`) and this has exactly one caller.
fn generate(model: &StyleModel, ctx: &SessionContext, seed: u64, part: Part) -> Vec<LaneTrack> {
    match part {
        Part::Drums => drums::generate(model, ctx, seed),
        Part::Chords => vec![chords::generate(model, ctx, seed).track],
        Part::Melody => {
            let harmony = chords::generate(model, ctx, seed);
            let kit = drums::generate(model, ctx, seed);
            vec![melody::generate(model, ctx, seed, &harmony, &kit)]
        }
        Part::Counter => {
            let harmony = chords::generate(model, ctx, seed);
            let kit = drums::generate(model, ctx, seed);
            let lead = melody::generate(model, ctx, seed, &harmony, &kit);
            vec![counter::generate(model, ctx, seed, &harmony, &lead)]
        }
        Part::Bass => {
            let harmony = chords::generate(model, ctx, seed);
            let kit = drums::generate(model, ctx, seed);
            vec![bass::generate(model, ctx, seed, &harmony, &kit)]
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
