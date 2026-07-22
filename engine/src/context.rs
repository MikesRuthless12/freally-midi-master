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
        let per_beat = PPQ * 4 / u32::from(self.time_sig_den.max(1));
        per_beat * u32::from(self.time_sig_num)
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
    fn a_zero_denominator_cannot_divide_by_zero() {
        // Guards against a malformed override reaching the engine.
        let ctx = SessionContext {
            time_sig_den: 0,
            ..Default::default()
        };
        assert_eq!(ctx.ticks_per_bar(), PPQ * 4 * 4);
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
