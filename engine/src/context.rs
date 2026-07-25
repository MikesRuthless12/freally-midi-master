//! The per-generation session: everything that is true of a generation but not
//! carried by the style model itself (PRD § 3, § 4 `SessionOverrides`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::pattern::{Lane, Scale, PPQ};

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
        // A zero denominator is malformed, and `.max(1)` is the wrong guard for
        // it: 1 is a legal denominator meaning whole notes, so it turns the bar
        // into four 4/4 bars rather than rejecting the value. Fall back to 4,
        // which is what `pattern_to_smf` already writes for an unrecognised
        // denominator — the two must agree or the file says one thing and the
        // tick arithmetic another.
        let den = if self.time_sig_den == 0 {
            4
        } else {
            u32::from(self.time_sig_den)
        };
        let per_beat = PPQ * 4 / den;
        per_beat * u32::from(self.time_sig_num.max(1))
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

        let bpm = overrides.bpm.unwrap_or_else(|| {
            session
                .and_then(|s| s.bpm.as_ref())
                .map(|spec| spec.nominal() as f32)
                .unwrap_or(defaults.bpm)
        });

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
            amount: overrides
                .swing
                .unwrap_or_else(|| authored_swing.map(|s| s.amount as f32).unwrap_or(0.5)),
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

        SessionContext {
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
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
