//! The per-generation session: everything that is true of a generation but not
//! carried by the style model itself (PRD § 3, § 4 `SessionOverrides`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::pattern::{Lane, Scale, ScaleCharacter, PPQ};

/// The grid swing is applied against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum SwingGrid {
    Eighth,
    Sixteenth,
}

/// MPC-style swing. `0.50` is straight and `0.667` is fully triplet; the
/// research constants cluster at 0.54–0.66 (PRD § 3, research ch. 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Swing {
    pub grid: SwingGrid,
    pub amount: f32,
}

impl Default for Swing {
    fn default() -> Self {
        Self {
            grid: SwingGrid::Sixteenth,
            amount: 0.5,
        }
    }
}

/// How far generated notes are pulled off the grid, and how much velocities
/// vary. Jitter is per lane because a hat and a kick do not breathe alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct Humanize {
    /// `1.0` snaps hard to the grid; `0.0` leaves the raw performance offset.
    pub quantize_strength: f32,
    /// Fractional velocity spread, e.g. `0.12` = ±12%.
    pub velocity_var: f32,
    /// Per-lane timing jitter in milliseconds.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub timing_jitter_ms: BTreeMap<Lane, f32>,
}

impl Default for Humanize {
    fn default() -> Self {
        Self {
            quantize_strength: 0.92,
            velocity_var: 0.12,
            timing_jitter_ms: BTreeMap::new(),
        }
    }
}

/// Everything a generator needs beyond the style model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct SessionContext {
    pub bpm: f32,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    /// Pitch class of the key root, 0 = C.
    pub key_root: u8,
    pub scale: Scale,
    pub swing: Swing,
    pub bars: u16,
    /// Halves the perceived tempo — the drums sit at half speed against the
    /// stated BPM, which is how most trap and drill models are notated.
    pub half_time: bool,
    pub humanize: Humanize,
}

impl Default for SessionContext {
    fn default() -> Self {
        Self {
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            swing: Swing::default(),
            bars: 4,
            half_time: false,
            humanize: Humanize::default(),
        }
    }
}

impl SessionContext {
    /// Ticks in one bar at this time signature.
    ///
    /// A tick is a fraction of a *quarter note*, so a bar of 6/8 is three
    /// quarter notes long, not six.
    pub fn ticks_per_bar(&self) -> u32 {
        // ⚠ **The reasoning that used to be written out here now lives in
        // `pattern::normalise_meter`, and only there.** A zero denominator is
        // malformed, and `.max(1)` is the wrong guard for it: 1 is a legal
        // denominator meaning whole notes, so it would turn the bar into four
        // 4/4 bars rather than rejecting the value. Falling back to 4 is what
        // `pattern_to_smf` already writes for an unrecognised denominator — and
        // the two must agree, or the file says one thing and the tick
        // arithmetic another. That agreement is now structural.
        crate::pattern::ticks_per_bar_of(self.time_sig_num, self.time_sig_den)
    }

    /// Total ticks for the whole generation.
    pub fn total_ticks(&self) -> u32 {
        self.ticks_per_bar() * u32::from(self.bars)
    }

    /// Milliseconds per tick at this tempo — the bridge between the lane jitter
    /// values, which are in milliseconds, and note positions, which are ticks.
    pub fn ms_per_tick(&self) -> f32 {
        60_000.0 / (self.bpm * PPQ as f32)
    }

    /// Convert a lane's jitter in milliseconds to ticks.
    pub fn ms_to_ticks(&self, ms: f32) -> f32 {
        ms / self.ms_per_tick()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_four_is_four_quarter_notes() {
        let ctx = SessionContext::default();
        assert_eq!(ctx.ticks_per_bar(), PPQ * 4);
        assert_eq!(ctx.total_ticks(), PPQ * 4 * 4);
    }

    #[test]
    fn three_four_is_three_quarter_notes() {
        let ctx = SessionContext {
            time_sig_num: 3,
            ..Default::default()
        };
        assert_eq!(ctx.ticks_per_bar(), PPQ * 3);
    }

    #[test]
    fn six_eight_is_three_quarter_notes_not_six() {
        let ctx = SessionContext {
            time_sig_num: 6,
            time_sig_den: 8,
            ..Default::default()
        };
        assert_eq!(ctx.ticks_per_bar(), PPQ * 3);
    }

    #[test]
    fn a_zero_denominator_falls_back_to_four_four() {
        // Guards against a malformed override reaching the engine.
        //
        // Asserted against the 4/4 bar rather than a literal, because the
        // literal is what hid the bug: this test used to assert `PPQ * 4 * 4`,
        // which reads like "four beats of four" but is really four bars' worth
        // of ticks — the value a `.max(1)` guard produced by reinterpreting the
        // denominator as whole notes.
        let ctx = SessionContext {
            time_sig_den: 0,
            ..Default::default()
        };
        assert_eq!(
            ctx.ticks_per_bar(),
            SessionContext::default().ticks_per_bar()
        );
        assert_eq!(ctx.ticks_per_bar(), PPQ * 4, "one bar, not four");
    }

    #[test]
    fn a_denominator_of_one_is_still_honoured() {
        // 1 is a legal denominator (whole notes) and must not be confused with
        // the malformed 0 above — that conflation is exactly what `.max(1)` did.
        let ctx = SessionContext {
            time_sig_num: 1,
            time_sig_den: 1,
            ..Default::default()
        };
        assert_eq!(ctx.ticks_per_bar(), PPQ * 4);
    }

    #[test]
    fn tick_duration_tracks_tempo() {
        let slow = SessionContext {
            bpm: 60.0,
            ..Default::default()
        };
        // At 60 BPM a quarter note is exactly one second.
        assert!((slow.ms_per_tick() * PPQ as f32 - 1000.0).abs() < 0.001);

        let fast = SessionContext {
            bpm: 120.0,
            ..Default::default()
        };
        assert!(fast.ms_per_tick() < slow.ms_per_tick());
    }

    #[test]
    fn milliseconds_convert_to_ticks_against_the_tempo() {
        let ctx = SessionContext {
            bpm: 60.0,
            ..Default::default()
        };
        // 1000 ms == one quarter note == PPQ ticks at 60 BPM.
        assert!((ctx.ms_to_ticks(1000.0) - PPQ as f32).abs() < 0.001);
    }

    #[test]
    fn session_context_roundtrips_through_json() {
        let mut ctx = SessionContext::default();
        ctx.humanize.timing_jitter_ms.insert(Lane::ClosedHat, 3.0);
        ctx.humanize.timing_jitter_ms.insert(Lane::Kick, 1.0);
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("timeSigNum"), "got {json}");
        assert!(json.contains("closedHat"), "got {json}");
        let back: SessionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, back);
    }

    #[test]
    fn an_empty_jitter_map_stays_out_of_the_payload() {
        let json = serde_json::to_string(&SessionContext::default()).unwrap();
        assert!(!json.contains("timingJitterMs"), "got {json}");
    }
}

/// What a caller may pin instead of letting the model choose it.
///
/// Everything is optional: an override the user has not touched must stay
/// absent rather than arrive as a default, or the artist's own value is
/// silently replaced by whatever the UI happened to initialise (PRD § 4
/// `SessionOverrides`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct SessionOverrides {
    pub bpm: Option<f32>,
    /// Pitch class, 0 = C.
    pub key_root: Option<u8>,
    pub scale: Option<Scale>,
    pub swing: Option<f32>,
    pub bars: Option<u16>,
    pub half_time: Option<bool>,
    /// The clip's own meter, when the producer set one (TASK-041E).
    ///
    /// ⛔ **Absent means "whatever the host is in", not 4/4.** Inside a DAW the
    /// meter comes from the project, and a default here would silently drag a
    /// 6/8 session back to common time on the next Generate. Set, it wins — a
    /// producer writing a 3/4 clip in a 4/4 project has been more specific than
    /// the project.
    pub time_sig_num: Option<u8>,
    pub time_sig_den: Option<u8>,
}

/// The tempo a pinned session may ask for, wider than any authored model.
///
/// `midi::pattern_to_smf` and the audio transport both already refuse a
/// non-positive tempo and substitute one, so an unclamped `0` does not crash —
/// it plays and exports at *their* fallback while every readout in the app says
/// `0`. And `ms_per_tick` divides by the tempo, so at `0` it is infinite and
/// every humanize jitter collapses to zero ticks: the feel disappears with
/// nothing reporting it. Clamping here keeps the tempo shown, the tempo heard
/// and the tempo written the same number. Authored tempos come from a linted
/// dataset; these arrive in an IPC payload, so this is the edge for the check.
/// Ableton Live's tempo range, which is also what the plugin trusts from a
/// host and what the BPM chip accepts. One range, three places, so a project
/// the DAW is running at is never a tempo this engine refuses.
const BPM_MIN: f32 = 20.0;
const BPM_MAX: f32 = 999.0;

/// The denominators a MIDI file can express.
///
/// ⛔ The meta event stores the denominator as a **power of two**, so these are
/// the only values that survive an export intact. A `3` used to divide the tick
/// arithmetic by three while `pattern_to_smf` fell back to writing `/4` — the
/// clip and the file disagreeing about how long a bar is.
const TIME_SIG_DENOMINATORS: [u8; 6] = [1, 2, 4, 8, 16, 32];

/// The longest bar a meter may ask for, in quarter notes.
///
/// ⛔ **A size limit, not a taste one, and it has to bound the *pair*.**
/// `ticks_per_bar` is `num × 4 / den`, and the generators write on a fixed
/// 240-tick grid — so the meter multiplies the note count exactly as `bars`
/// does. `MAX_BARS` exists to stop a value from a file, a preset or devtools
/// asking for a pattern that takes minutes to build on the thread the host
/// draws its window from; checking the numerator and denominator separately
/// does not close it, because `32/1` is a 128-quarter bar. Sixteen quarters is
/// four times a common-time bar, which is past anything anyone writes and
/// keeps the worst case inside the ceiling `MAX_BARS` was chosen for.
const MAX_QUARTERS_PER_BAR: u32 = 16;

/// A meter from untrusted input, or `4/4` if it is not one this can honour.
///
/// Refused as a pair rather than clamped field by field: a `255` nobody asked
/// for is corrupt input, and honouring half of it — the denominator, say —
/// would produce a bar the producer never chose and cannot see they have.
fn meter_or_common_time(num: Option<u8>, den: Option<u8>) -> (u8, u8) {
    let (Some(num), Some(den)) = (num.or(Some(4)), den.or(Some(4))) else {
        return (4, 4);
    };
    if num == 0 || !TIME_SIG_DENOMINATORS.contains(&den) {
        return (4, 4);
    }
    // ⛔ Measured in *ticks*, not in whole quarters. `4/32` is an eighth of a
    // quarter note — a real, if tiny, bar — and integer-dividing to quarters
    // rounded it to zero, so a legal meter was refused as degenerate.
    let ticks = PPQ * 4 / u32::from(den) * u32::from(num);
    if ticks == 0 || ticks > MAX_QUARTERS_PER_BAR * PPQ {
        return (4, 4);
    }
    (num, den)
}

/// Straight to fully triplet, with a little past each end for feel.
/// `humanize` reads the amount as a ratio of the swung subdivision; outside
/// this range the off-beat lands on — or past — the note after it.
const SWING_MIN: f32 = 0.5;
const SWING_MAX: f32 = 0.75;

/// Clamp a caller-supplied value, falling back for a `NaN` that `clamp` would
/// otherwise pass straight through.
fn sane(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_nan() {
        fallback
    } else {
        value.clamp(min, max)
    }
}

impl SessionContext {
    /// The session a style model asks for, with anything the user pinned
    /// taking precedence.
    ///
    /// Sampled from a seeded stream of its own, so the same seed always yields
    /// the same tempo and key — the seed chip's promise covers the session, not
    /// only the notes — and so that adding a session parameter later cannot
    /// shift the note streams.
    pub fn from_model(model: &crate::StyleModel, overrides: &SessionOverrides, seed: u64) -> Self {
        let mut rng = crate::rng::stream(seed, "session");
        let session = model.session.as_ref();
        let defaults = Self::default();

        let bpm = match overrides.bpm {
            Some(pinned) => sane(pinned, BPM_MIN, BPM_MAX, defaults.bpm),
            None => session
                .and_then(|s| s.bpm.as_ref())
                .map(|spec| spec.nominal() as f32)
                .unwrap_or(defaults.bpm),
        };

        let key_root = overrides.key_root.unwrap_or_else(|| {
            session
                .and_then(|s| s.keys.as_ref())
                .and_then(|spec| spec.sample(&mut rng).ok())
                .and_then(|name| crate::theory::key_pitch_class(&name))
                .unwrap_or(defaults.key_root)
        });

        let scale = overrides.scale.unwrap_or_else(|| {
            session
                .and_then(|s| s.scales.as_ref())
                .and_then(|spec| spec.sample(&mut rng).ok())
                .and_then(|name| serde_json::from_value(serde_json::Value::String(name)).ok())
                .unwrap_or(defaults.scale)
        });

        let authored_swing = session.and_then(|s| s.swing.as_ref());
        let swing = Swing {
            grid: match authored_swing.map(|s| s.grid.as_str()) {
                Some("8th") => SwingGrid::Eighth,
                _ => SwingGrid::Sixteenth,
            },
            amount: match overrides.swing {
                Some(pinned) => sane(pinned, SWING_MIN, SWING_MAX, 0.5),
                None => authored_swing.map(|s| s.amount as f32).unwrap_or(0.5),
            },
        };

        let humanize = session
            .and_then(|s| s.humanize.as_ref())
            .map(|spec| Humanize {
                quantize_strength: spec.quantize_strength.unwrap_or(0.92) as f32,
                velocity_var: spec.velocity_var.unwrap_or(0.12) as f32,
                // A lane name the engine does not know is dropped here, which
                // is why `engine/tests/humanize.rs` asserts every authored key
                // parses: a typo would silently cost that lane its feel.
                timing_jitter_ms: spec
                    .timing_jitter_ms
                    .iter()
                    .filter_map(|(lane, ms)| {
                        serde_json::from_value::<Lane>(serde_json::Value::String(lane.clone()))
                            .ok()
                            .map(|lane| (lane, *ms as f32))
                    })
                    .collect(),
            })
            .unwrap_or_default();

        let meter = meter_or_common_time(overrides.time_sig_num, overrides.time_sig_den);

        SessionContext {
            bpm,
            // ⛔ **Clamped, and this is a size limit rather than a taste one.**
            // `ticks_per_bar` is `num × 4 / den`, and the generators write a
            // note every fixed 240 ticks — so the meter multiplies the note
            // count as surely as `bars` does. `MAX_BARS` exists to stop a value
            // from a file, a preset or devtools asking for a pattern that takes
            // minutes to build on the thread the host draws its window from;
            // unclamped, `255/1` multiplies past that ceiling by 63×, and
            // `255/1 × 128 bars` measured at 526,000 notes and ~875 ms of
            // synchronous stall inside the DAW.
            //
            // ⛔ The denominator is restricted to the powers of two
            // `midi::pattern_to_smf` can actually write. A `3` used to divide
            // the tick arithmetic by three while the exported meta event fell
            // back to `/4` — the file and the clip disagreeing about the bar.
            time_sig_num: meter.0,
            time_sig_den: meter.1,
            key_root,
            scale,
            swing,
            bars: overrides.bars.unwrap_or(4),
            half_time: overrides
                .half_time
                .or_else(|| session.and_then(|s| s.half_time))
                .unwrap_or(false),
            humanize,
        }
    }
}

/// What a model asks for, before a seed picks among the options it offers.
///
/// The session chips need this the moment an artist is selected, which is
/// *before* there is a seed — and two of these fields are chosen by one.
/// `bpm`, `swing` and `half_time` are deterministic, so they arrive as values;
/// `keys` and `scales` are sampled, so they arrive as the authored lists. A
/// sampled key here would be a readout that silently changed under the user
/// the moment they pressed Generate (FR-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct SessionDefaults {
    /// The tempo a generation will use: `BpmSpec::nominal`, not a sample.
    pub bpm: f32,
    /// The tempo range the model authored, or `bpm` twice when it authored none
    /// (TASK-158D).
    ///
    /// ⛔ **A range, because a single number is not what a producer is choosing
    /// between.** *"tempo range"* is Mike's own word in TASK-158D: an artist who
    /// works at 68–96 and one who works at 138–142 are different propositions at
    /// the same nominal 82, and the pane exists so that is visible before
    /// Generate rather than after.
    ///
    /// ⚠ **Not overwritten by the host's tempo the way `bpm` is.** That
    /// substitution is TASK-033's, and it is right for "what will this come out
    /// at" — but the range is a fact about the *model*, and replacing it with the
    /// DAW's one tempo would say the artist only ever works there.
    pub bpm_min: f32,
    pub bpm_max: f32,
    /// Which parts this model will actually write notes for (TASK-158D).
    ///
    /// See [`parts_of`] — this is the "and does **not** cover" half of what the
    /// detail pane owes a producer.
    pub parts: Vec<crate::pattern::Part>,
    /// Key names the model draws from, in authored order. Empty when it
    /// authors none, in which case the engine's own default key applies.
    pub keys: Vec<String>,
    /// Scales the model draws from, in authored order.
    pub scales: Vec<Scale>,
    pub swing: Swing,
    pub half_time: bool,
    /// The moods this model offers, in authored order (TASK-040V).
    ///
    /// Empty for a model with no `modes` block, which is most of them — the
    /// chip is hidden rather than shown offering only "Any". Names only: the
    /// overrides behind them are the engine's business, and the UI picks by
    /// name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
}

/// The model's scales narrowed to a mood's character (TASK-041C).
///
/// ⛔ **An intersection, and both terms hold at once.** A dark trap mood offers
/// *trap's* dark scales — never Locrian just because Locrian is dark, and never
/// Lydian just because trap is trap. Applying either constraint alone is what
/// makes "artist-accurate" untrue on one of the two axes.
///
/// ⛔ **An empty intersection falls back to the model's own list rather than
/// emptying the picker.** A bright mood on a genre that authors only minor
/// scales is an authoring mistake, and the right response to one is to generate
/// the genre's own music, not to generate nothing. The mistake is visible in the
/// dataset rather than as a producer staring at a picker with no options.
///
/// A model that authors no scales at all keeps its empty list: it has said
/// nothing, which is not the same as having said "only these", and the caller
/// falls back to the engine's full set.
fn narrow_by_character(scales: Vec<Scale>, character: Option<ScaleCharacter>) -> Vec<Scale> {
    let Some(wanted) = character else {
        return scales;
    };
    if scales.is_empty() {
        return scales;
    }
    let narrowed: Vec<Scale> = scales
        .iter()
        .copied()
        .filter(|scale| crate::theory::scale_character(*scale) == wanted)
        .collect();
    if narrowed.is_empty() {
        scales
    } else {
        narrowed
    }
}

/// Which parts a model will actually write notes for (TASK-158D).
///
/// ⛔⛔ **This is the readout-that-lies failure in the place it costs most.** A
/// generator whose block is absent returns an **empty** track — `bass.rs`,
/// `chords.rs`, `melody.rs` and `counter.rs` each open with a `let … else {
/// return empty }` on exactly that. So an artist who authored no `melody` block
/// answers Generate on the Melody tab with silence, and before this there was no
/// way to know that except by pressing it. Mike's TASK-158D asks for *"what the
/// model does and does **not** cover"*, and this is the exact answer rather than
/// an approximation of one.
///
/// ⛔ **Drums are always covered**, and that is not an oversight: `drums.rs`
/// reads its block as an `Option` and falls back to a real kit — backbeat snare,
/// a kick grammar, hats — so a model with no `drums` block still writes drums.
/// Listing it conditionally would be the same lie in the other direction.
///
/// ⛔ **An 808 that plays the bassline counts as bass.** `bass.rs` returns empty
/// for such a model *deliberately* — FR-007's "unify with the 808 lane", because
/// doubling it is a production mistake — but the producer does get a bassline;
/// it comes out of the drum kit's `bass808` lane. Reporting "no bass" for most of
/// trap would be worse than saying nothing.
///
/// ⚠ **Asked of a RESOLVED model.** `extends` is merged before this sees it, so
/// an artist inherits their genre's blocks — which is what they actually
/// generate from.
pub fn parts_of(model: &crate::StyleModel) -> Vec<crate::pattern::Part> {
    use crate::pattern::Part;
    let has = |name: &str| model.blocks.contains_key(name);
    crate::pattern::PART_ORDER
        .into_iter()
        .filter(|part| match part {
            Part::Drums => true,
            Part::Chords => has("chords"),
            Part::Melody => has("melody"),
            // A counter answers a melody, and `counter.rs` returns empty when
            // the melody did — so a countermelody block with no melody beside it
            // writes nothing.
            Part::Counter => has("countermelody") && has("melody"),
            Part::Bass => {
                has("bassline") || crate::generators::bass::eight_o_eight_is_the_bass(model)
            }
        })
        .collect()
}

impl SessionDefaults {
    /// Read a resolved model's session block. No seed, and no sampling.
    pub fn of(model: &crate::StyleModel) -> Self {
        let session = model.session.as_ref();
        let fallback = SessionContext::default();

        let authored_swing = session.and_then(|s| s.swing.as_ref());
        let bpm = session.and_then(|s| s.bpm.as_ref());

        Self {
            bpm: bpm
                .map(|spec| spec.nominal() as f32)
                .unwrap_or(fallback.bpm),
            // ⚠ The nominal twice when nothing was authored, rather than the
            // engine's whole legal range: a model that said nothing about tempo
            // has not said it works from 40 to 300, and drawing that would be an
            // invention rather than a summary.
            bpm_min: bpm.map(|spec| spec.min as f32).unwrap_or(fallback.bpm),
            bpm_max: bpm.map(|spec| spec.max as f32).unwrap_or(fallback.bpm),
            parts: parts_of(model),
            keys: session
                .and_then(|s| s.keys.as_ref())
                .map(|spec| spec.options())
                .unwrap_or_default(),
            // A scale name the engine does not know is dropped, exactly as
            // `from_model` drops it. `engine/tests/session_strings.rs` asserts
            // every authored name parses, so a typo costs a failing test
            // rather than a key the chip cannot offer.
            scales: narrow_by_character(
                session
                    .and_then(|s| s.scales.as_ref())
                    .map(|spec| {
                        spec.options()
                            .into_iter()
                            .filter_map(|name| {
                                serde_json::from_value(serde_json::Value::String(name)).ok()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                session.and_then(|s| s.character),
            ),
            swing: Swing {
                grid: match authored_swing.map(|s| s.grid.as_str()) {
                    Some("8th") => SwingGrid::Eighth,
                    _ => SwingGrid::Sixteenth,
                },
                amount: authored_swing.map(|s| s.amount as f32).unwrap_or(0.5),
            },
            half_time: session.and_then(|s| s.half_time).unwrap_or(false),
            moods: crate::dataset::modes::modes_of(model)
                .into_iter()
                .map(|mode| mode.name)
                .collect(),
        }
    }
}

#[cfg(test)]
mod character_tests {
    use super::*;

    const TRAP: [Scale; 4] = [
        Scale::NaturalMinor,
        Scale::HarmonicMinor,
        Scale::Phrygian,
        Scale::MinorPentatonic,
    ];

    #[test]
    fn a_mode_with_no_character_leaves_the_list_alone() {
        // ⛔ Absent is not "neutral". Most modes say nothing about colour, and
        // collapsing the two would make every one of them offer only the
        // symmetric scales.
        assert_eq!(narrow_by_character(TRAP.to_vec(), None), TRAP.to_vec());
    }

    #[test]
    fn a_dark_mood_offers_the_models_own_dark_scales_and_no_others() {
        // Both constraints at once: trap's list narrowed to the dark ones.
        // Nothing dark that trap does not author may appear.
        let dark = narrow_by_character(TRAP.to_vec(), Some(ScaleCharacter::Dark));
        assert_eq!(dark, TRAP.to_vec(), "every scale trap authors is dark");
        assert!(
            !dark.contains(&Scale::Locrian),
            "Locrian is dark but not trap's"
        );
    }

    #[test]
    fn a_mood_narrows_a_mixed_list_rather_than_reordering_it() {
        let mixed = vec![
            Scale::Major,
            Scale::NaturalMinor,
            Scale::Lydian,
            Scale::Phrygian,
        ];
        assert_eq!(
            narrow_by_character(mixed.clone(), Some(ScaleCharacter::Bright)),
            vec![Scale::Major, Scale::Lydian]
        );
        assert_eq!(
            narrow_by_character(mixed, Some(ScaleCharacter::Dark)),
            vec![Scale::NaturalMinor, Scale::Phrygian]
        );
    }

    #[test]
    fn an_empty_intersection_falls_back_to_the_models_own_list() {
        // ⛔ A bright mood on a genre that authors only minor scales is an
        // authoring mistake. Generating the genre's own music is a better
        // answer than generating nothing, and a picker with no options is what
        // "nothing" looks like to a producer.
        let bright = narrow_by_character(TRAP.to_vec(), Some(ScaleCharacter::Bright));
        assert_eq!(bright, TRAP.to_vec());
    }

    #[test]
    fn a_model_that_authors_no_scales_still_authors_none() {
        // Saying nothing is not saying "only these" — the caller falls back to
        // the engine's full set, and narrowing an empty list must not change
        // that into "only the dark ones of nothing".
        assert!(narrow_by_character(vec![], Some(ScaleCharacter::Dark)).is_empty());
    }
}

#[cfg(test)]
mod override_tests {
    use super::*;

    #[test]
    fn a_pinned_tempo_outside_the_musical_range_is_clamped() {
        assert_eq!(sane(0.0, BPM_MIN, BPM_MAX, 140.0), BPM_MIN);
        assert_eq!(sane(10_000.0, BPM_MIN, BPM_MAX, 140.0), BPM_MAX);
        assert_eq!(sane(f32::INFINITY, BPM_MIN, BPM_MAX, 140.0), BPM_MAX);
        // NaN passes straight through `f32::clamp`, so it needs its own arm.
        assert_eq!(sane(f32::NAN, BPM_MIN, BPM_MAX, 140.0), 140.0);
        // ...and an ordinary value is untouched.
        assert_eq!(sane(88.0, BPM_MIN, BPM_MAX, 140.0), 88.0);
    }

    #[test]
    fn a_pinned_swing_stays_inside_the_subdivision() {
        // Past 0.75 the swung note lands on the one after it.
        assert_eq!(sane(0.9, SWING_MIN, SWING_MAX, 0.5), SWING_MAX);
        assert_eq!(sane(0.0, SWING_MIN, SWING_MAX, 0.5), SWING_MIN);
        assert_eq!(sane(0.62, SWING_MIN, SWING_MAX, 0.5), 0.62);
    }
}
