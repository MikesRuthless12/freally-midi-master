//! One-shot sampler voices and the master limiter.
//!
//! Everything here runs on the audio callback, which means the rules are
//! absolute: **no allocation, no locking, no I/O, no panicking path**. Voices
//! live in a fixed array and reference kit pads by index, so triggering a note
//! moves no memory and touches no reference count.
//!
//! `engine/tests` cannot reach this and neither can a device — so the whole
//! thing is a pure function of its inputs, and the tests drive it by hand.

use super::kit::Kit;

/// Enough for the densest thing the engine generates — a 64th-note hat roll
/// under a fill — with room to spare. Voices are ~48 bytes, so the array is
/// cheap; running out is what sounds broken.
pub const MAX_VOICES: usize = 48;

/// The knee above which the limiter starts bending. Below it the mix is
/// untouched, so ordinary playback is not coloured at all.
const LIMIT_KNEE: f32 = 0.6;

/// A voice that is never gated: it plays until its sample runs out.
///
/// ⚠ Decremented like any other hold, which is safe because 4.29 billion frames
/// is a full day of audio and no sample is that long. One counter, no branch on
/// the audio thread to ask whether this voice has an end.
pub const RINGS_OUT: u32 = u32::MAX;

/// Frames the gate takes to close — 5 ms at 48 kHz.
///
/// ⛔ **Not zero.** Cutting a sub off mid-cycle is a step to silence, and a step
/// is a click; on an 808 under a melody it is a click on *every note*. Short
/// enough that the note still ends where the producer drew it.
const RELEASE_FRAMES: f32 = 240.0;

/// An 808 slide: where the pitch is going, and how long it takes to get there.
///
/// ⛔⛔ **The audio half of `Note::slide_to_pitch`, which nothing rendered until
/// TASK-138's follow-up.** The engine has written slides since the bassline
/// generator gained `glideProb` and the drum 808 lane gained `slideProb`, and
/// MIDI export has encoded them as overlapping notes for as long — but the
/// sampler held **one constant rate per voice**, so every exported WAV, every
/// clip dragged into a DAW and every preview played the slide as a flat note.
/// Mike, 2026-08-06: *"ensure i have 808 slides that can be activated as well
/// for my audio being dragged into the DAW or audio being exported and for my
/// generator playback."*
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    /// Semitones to travel from the note's starting pitch. Signed: 808s slide
    /// down at least as often as up.
    pub semis: f32,
    /// How many output frames the travel takes.
    pub frames: u32,
    /// Frames to hold the starting pitch before the travel begins.
    ///
    /// ⛔ **So the audio matches the MIDI rather than merely being a glide.**
    /// `midi.rs` puts the destination note-on at `start + len/2` — the origin is
    /// held for half the note and then moves — and this codebase's own rule is
    /// that a rendered stem sounds like what the DAW receives. A glide that
    /// began at the note-on would be a different performance from the `.mid`
    /// beside it in the same folder.
    pub delay: u32,
}

#[derive(Clone, Copy)]
struct Voice {
    /// Index into `Kit::pads`, and `usize::MAX` when the voice is free.
    pad: usize,
    /// Read position in the pad's samples. Fractional: the step is rarely 1.0.
    pos: f64,
    /// Samples advanced per output frame — device rate against pad rate, times
    /// the pitch ratio.
    step: f64,
    /// What `step` is multiplied by each frame while a slide is running, and
    /// `1.0` when it is not.
    ///
    /// ⛔ **Multiplied, not added, and that is the whole reason a slide sounds
    /// like a slide.** Pitch is logarithmic in playback rate: moving `step`
    /// linearly from one rate to another sweeps *fast* at the start and crawls
    /// at the end, which reads as a pitch envelope rather than a glide. One
    /// precomputed factor per frame is exponential in rate, so it is linear in
    /// semitones — and it costs a single multiply on the audio thread instead of
    /// a `powf` per sample.
    glide_mul: f64,
    /// Frames of slide left to run.
    glide_left: u32,
    /// Frames still to hold the starting pitch before the slide begins.
    glide_wait: u32,
    gain_l: f32,
    gain_r: f32,
    /// What a mono bus gets: the pad gain *before* the pan split.
    ///
    /// ⛔ Equal-power panning means `gain_l` and `gain_r` are `gain·cos θ` and
    /// `gain·sin θ`, so the power-preserving fold-down is `sqrt(l² + r²)` —
    /// which is exactly `gain`. Storing it costs a float and saves a `sqrt` per
    /// sample on the audio thread. Summing the two amplitudes instead would be
    /// 3 dB hot on a centred pad; taking only the left is 3 dB down and silences
    /// a hard-right one.
    gain_mono: f32,
    /// Read this voice back to front — see the note in `render`.
    ///
    /// ⚠ A property of the NOTE, not of the pad: the same pad may sound forwards
    /// on one hit and backwards on the next inside one pattern.
    reversed: bool,
    choke_group: Option<u8>,
    /// When this voice started, for stealing the oldest.
    started: u64,
    /// Frames left before the gate closes, or [`RINGS_OUT`] for a drum.
    ///
    /// ⛔⛔ **THE NOTE'S LENGTH BOUNDS A MELODIC ONE-SHOT** — Mike, 2026-08-12,
    /// after dropping a sub 808 on the Melody generator: *"it had a lot of
    /// static and kept playing it throughout the actual melody over and over
    /// again."* A three-second 808 retriggered every eighth note stacked a dozen
    /// copies of itself on top of each other; that pile is the static, and the
    /// tail running under the next six notes is the "over and over".
    ///
    /// ▶ **The rule he asked for, in his own two cases:** *"if i drop a hihat on
    /// the melody generator, then it shouldn't extend the length of the hihat,
    /// but if i drop a oneshot (like a violin) then it should play to the end of
    /// the note."* So the sample plays **at its own speed** and stops at
    /// whichever comes first — the end of the sample, or the end of the note.
    /// The hat is shorter than the note and finishes on its own, untouched; the
    /// violin and the 808 are longer and are cut at the note. Nothing is
    /// stretched: he asked *"or does it stretch it to meet the length of the
    /// note"* and it must not — stretching a hat to a half note would pitch it
    /// down two octaves and turn a hi-hat into a wash.
    ///
    /// ⚠ **Drums are unchanged.** A kick under a sixteenth has always rung out,
    /// and gating it would rewrite how every kit in the product sounds.
    /// `roles::is_melodic` is the one list that decides which lanes this is for.
    hold: u32,
    /// Gate level, 1.0 until `hold` runs out and then ramping to 0.
    fade: f32,
}

impl Voice {
    const FREE: usize = usize::MAX;

    fn free() -> Self {
        Voice {
            pad: Self::FREE,
            pos: 0.0,
            step: 1.0,
            glide_mul: 1.0,
            glide_left: 0,
            glide_wait: 0,
            gain_l: 0.0,
            gain_r: 0.0,
            gain_mono: 0.0,
            reversed: false,
            choke_group: None,
            started: 0,
            hold: RINGS_OUT,
            fade: 1.0,
        }
    }

    fn active(&self) -> bool {
        self.pad != Self::FREE
    }
}

pub struct Sampler {
    voices: [Voice; MAX_VOICES],
    /// Monotonic counter deciding which voice is oldest.
    clock: u64,
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler {
            voices: [Voice::free(); MAX_VOICES],
            clock: 0,
        }
    }
}

/// Everything about one hit that belongs to the **note** rather than the pad.
///
/// ⚠ **A struct because the list kept growing** — velocity and pitch, then the
/// slide, then the direction, then the gate — and five positional arguments of
/// which two are `bool`/`u32` is a call site nobody can read and clippy is right
/// to refuse. Each field's own rule is on the `Voice` field it feeds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// 0–1, multiplied into the pad's own gain.
    pub velocity: f32,
    /// Added to the pad's own pitch offset.
    pub semis: f32,
    pub glide: Option<Glide>,
    /// Read the pad back to front for this hit alone — see `Voice::reversed`.
    pub reversed: bool,
    /// Frames before the gate closes, or [`RINGS_OUT`] — see `Voice::hold`.
    pub hold: u32,
}

impl Default for Hit {
    fn default() -> Self {
        Hit {
            velocity: 1.0,
            semis: 0.0,
            glide: None,
            reversed: false,
            hold: RINGS_OUT,
        }
    }
}

/// How long a note on `lane` may sound: its own length, or forever for a drum.
///
/// ⛔⛔ **ONE POLICY, BECAUSE TWO CALLERS MUST AGREE.** The live preview
/// (`lib.rs`) and the offline renderer (`audio/render.rs`) both build a [`Hit`]
/// per note, and this codebase's standing rule is that **a rendered stem sounds
/// like what was heard** — a producer drags the `.wav` of the thing they just
/// auditioned. The *list* of melodic lanes was already shared (`roles::is_melodic`);
/// the branch over it was written out twice, so widening the gate — gating `Sub`,
/// adding a release tail, exempting a lane — could land in one path and not the
/// other, and the export would silently stop matching playback.
///
/// ⚠ Only the *frames* derivation legitimately differs between the two: the
/// preview has `Fired::frames` already, the renderer computes it from ticks.
///
/// ⛔⛔ **A SLIDING NOTE IS NEVER GATED, AND THAT IS NOT AN EXCEPTION — IT IS THE
/// ARITHMETIC.** `Glide` holds the origin pitch for `len / 2` and travels for the
/// remaining `len / 2`, so the note **arrives at its destination on exactly the
/// frame `len_frames` names**. Gating there closes the envelope at the instant
/// the pitch lands, and the destination — the whole point of an 808 slide — is
/// never heard at all. Before the gate existed the voice rang out there, which is
/// what a slide has always sounded like.
///
/// ▶ **Mike's report is untouched by this.** The pile-up he heard was a sub 808
/// dropped on **Melody**, and `slide_to_pitch` is written by exactly one melodic
/// generator — `engine::generators::bass`. So the notes that ring out here are
/// the ones on a monophonic 808 line, which is where you want the tail, and the
/// ones that are cut are every melody, counter and chord note, which is where
/// the stacking was.
pub fn hold_for(lane: engine::pattern::Lane, len_frames: u32, sliding: bool) -> u32 {
    if crate::roles::is_melodic(lane) && !sliding {
        len_frames
    } else {
        RINGS_OUT
    }
}

impl Sampler {
    /// Start a pad. `velocity` is 0–1; `semis` is added to the pad's own offset.
    pub fn trigger(&mut self, kit: &Kit, pad_index: usize, velocity: f32, semis: f32, rate: f64) {
        self.trigger_with(
            kit,
            pad_index,
            rate,
            Hit {
                velocity,
                semis,
                ..Hit::default()
            },
        );
    }

    /// Start a pad with everything the note has to say about it.
    ///
    /// ⚠ **A sibling rather than a wider `trigger`**: the plain form has a dozen
    /// call sites, most of them tests that say nothing about pitch, and widening
    /// it would have put a default in all of them to serve two real callers —
    /// the offline renderer and the live preview.
    pub fn trigger_with(&mut self, kit: &Kit, pad_index: usize, rate: f64, hit: Hit) {
        let Hit {
            velocity,
            semis,
            glide,
            reversed,
            hold,
        } = hit;
        let Some(pad) = kit.pads.get(pad_index) else {
            return;
        };

        // A choke group is a physical claim, not a mix decision: the same hat
        // cannot be open and closed at once, so the new hit silences the old.
        if let Some(group) = pad.choke_group {
            for voice in &mut self.voices {
                if voice.active() && voice.choke_group == Some(group) {
                    *voice = Voice::free();
                }
            }
        }

        let slot = self.claim();
        self.clock += 1;

        // Constant power, so a centred pad is not louder than a panned one.
        let angle = (pad.pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::FRAC_PI_4);
        let gain = pad.gain * velocity.clamp(0.0, 1.0);

        // ⛔ **A slide of no frames is not a slide**, and dividing by it would
        // be an infinity in `step` on the audio thread. A slide of no semitones
        // is a no-op that would cost a multiply per frame for nothing.
        let glide = glide.filter(|g| g.frames > 0 && g.semis != 0.0);

        self.voices[slot] = Voice {
            pad: pad_index,
            pos: 0.0,
            step: f64::from(pad.sample_rate) / rate
                * 2f64.powf(f64::from(pad.pitch_semis as f32 + semis) / 12.0),
            // The whole travel spread evenly across the window, in semitones —
            // see `Voice::glide_mul` for why that is a constant factor.
            glide_mul: glide.map_or(1.0, |g| {
                2f64.powf(f64::from(g.semis) / 12.0 / f64::from(g.frames))
            }),
            glide_left: glide.map_or(0, |g| g.frames),
            glide_wait: glide.map_or(0, |g| g.delay),
            gain_l: gain * angle.cos(),
            gain_r: gain * angle.sin(),
            gain_mono: gain,
            reversed,
            choke_group: pad.choke_group,
            started: self.clock,
            // ⚠ **At least one frame of hold**, or a zero-length note would open
            // the release on the frame it started and the voice would be a
            // 5 ms blip of fade with no body. The grid cannot draw a zero-length
            // note, but an import can carry one.
            hold: hold.max(1),
            fade: 1.0,
        };
    }

    /// A free slot, or the oldest sounding one.
    ///
    /// Stealing the oldest is the right trade for one-shots: the newest hit is
    /// the one the user is listening for, and the oldest is furthest into its
    /// decay, so it is the least audible thing to lose.
    fn claim(&mut self) -> usize {
        let mut oldest = 0usize;
        for (i, voice) in self.voices.iter().enumerate() {
            if !voice.active() {
                return i;
            }
            if voice.started < self.voices[oldest].started {
                oldest = i;
            }
        }
        oldest
    }

    pub fn stop_all(&mut self) {
        self.voices = [Voice::free(); MAX_VOICES];
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// Is anything sounding? Short-circuits, unlike [`Sampler::active_voices`],
    /// which counts all 48 — this runs once per block to decide whether the
    /// block needs rendering at all.
    pub fn is_silent(&self) -> bool {
        !self.voices.iter().any(Voice::active)
    }

    /// Mix every sounding voice into `out`, which is interleaved by `channels`.
    ///
    /// Adds rather than overwrites: the caller renders a block in segments, so
    /// a segment boundary must not erase what came before it.
    /// ⛔ **A mono bus gets the power-preserving fold-down, not the left
    /// channel.** See `Voice::gain_mono`: taking `gain_l` alone put a mono
    /// insert 3 dB below the stereo track beside it and would have silenced the
    /// first hard-panned pad anyone authored.
    pub fn render(&mut self, kit: &Kit, out: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }

        for voice in &mut self.voices {
            if !voice.active() {
                continue;
            }
            let Some(pad) = kit.pads.get(voice.pad) else {
                *voice = Voice::free();
                continue;
            };
            let samples: &[f32] = &pad.samples;

            for frame in out.chunks_mut(channels) {
                let index = voice.pos as usize;
                if index + 1 >= samples.len() {
                    *voice = Voice::free();
                    break;
                }

                // ⛔⛔ **A reversed note reads the same buffer from the far end**
                // — Mike, 2026-08-11: *"you should be able to select the note and
                // press 'Ctrl+R' … to reverse the note just for that single note
                // being played."*
                //
                // ⛔ **Mirrored here rather than by flipping the buffer**, and
                // the distinction is the whole reason this is on the voice.
                // Assigning a *reversed one-shot* flips the samples once at load
                // (`oneshot::load`), because that never changes. This is the
                // other case: the same pad sounding forwards on one hit and
                // backwards on the next inside one pattern, so it can only be
                // decided per voice — and a second copy of every reversed pad's
                // audio, per note, is not a thing to allocate.
                //
                // ⚠ **The position still walks forwards**, so `active()`, the
                // end test above and the slide below are untouched; only the
                // *index* is mirrored. A backwards `pos` would need every one of
                // those to grow a second case.
                let (a, b) = if voice.reversed {
                    let last = samples.len() - 1;
                    (last - index, last - index - 1)
                } else {
                    (index, index + 1)
                };

                // Linear interpolation. The step is almost never 1.0 — the kit
                // is 44.1 kHz and the device usually is not — so reading the
                // nearest sample instead would alias audibly on the hats.
                let frac = (voice.pos - index as f64) as f32;
                let value = (samples[a] + (samples[b] - samples[a]) * frac) * voice.fade;

                if channels > 1 {
                    frame[0] += value * voice.gain_l;
                    frame[1] += value * voice.gain_r;
                } else {
                    frame[0] += value * voice.gain_mono;
                }
                // Anything past stereo stays silent rather than being fed a
                // copy: a surround device would otherwise put the kit in the
                // centre and the rears at once.

                voice.pos += voice.step;

                // ⛔ **The gate.** A melodic voice was given the note's length in
                // frames; a drum was given `RINGS_OUT` and this branch is a
                // decrement it will never finish. See `Voice::hold`.
                //
                // ⚠ **Freed at the bottom of the ramp, not at the top**, so the
                // slot is released only once the voice is actually silent —
                // stealing it a moment earlier is the click the ramp exists to
                // avoid, arriving through the back door.
                if voice.hold > 0 {
                    voice.hold -= 1;
                } else {
                    voice.fade -= 1.0 / RELEASE_FRAMES;
                    if voice.fade <= 0.0 {
                        *voice = Voice::free();
                        break;
                    }
                }

                // The slide (TASK-138 follow-up). The hold comes first, so the
                // note sits at its own pitch and *then* moves — matching where
                // `midi.rs` puts the destination note-on. A voice that is not
                // sliding pays one predictable branch per frame.
                if voice.glide_wait > 0 {
                    voice.glide_wait -= 1;
                } else if voice.glide_left > 0 {
                    voice.step *= voice.glide_mul;
                    voice.glide_left -= 1;
                }
            }
        }
    }
}

/// Bend a signal into ±1 without a corner.
///
/// Below the knee this is the identity, so nothing that was already in range is
/// coloured. Above it the curve is continuous in value *and* slope, which is
/// what stops a limiter from sounding like distortion, and it approaches full
/// scale asymptotically — so stacking previews cannot crack the output. Far
/// enough out it lands on exactly 1.0, where `f32` no longer has the resolution
/// to hold it apart; that is the ceiling, not a breach of it.
pub fn soft_clip(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= LIMIT_KNEE {
        return x;
    }
    let headroom = 1.0 - LIMIT_KNEE;
    let shaped = LIMIT_KNEE + headroom * ((magnitude - LIMIT_KNEE) / headroom).tanh();
    shaped.copysign(x)
}

/// Apply the limiter across a rendered block.
pub fn limit(out: &mut [f32]) {
    for sample in out {
        *sample = soft_clip(*sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::kit::Pad;
    use engine::pattern::Lane;
    use std::sync::Arc;

    /// A kit of test tones: every pad is a run of 1.0s, so a rendered sample
    /// says exactly which pad and gain produced it.
    fn test_kit() -> Kit {
        let pad = |id: &str, lane: Lane, choke: Option<u8>, pan: f32| Pad {
            id: id.into(),
            lane,
            samples: Arc::from(vec![1.0f32; 64].into_boxed_slice()),
            sample_rate: 48_000,
            gain: 1.0,
            pan,
            pitch_semis: 0,
            choke_group: choke,
            root_note: None,
        };
        Kit {
            id: "test".into(),
            pads: vec![
                pad("kick", Lane::Kick, None, 0.0),
                pad("closed", Lane::ClosedHat, Some(1), 0.0),
                pad("open", Lane::OpenHat, Some(1), 0.0),
                pad("left", Lane::Perc, None, -1.0),
            ],
        }
    }

    fn render_block(sampler: &mut Sampler, kit: &Kit, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        sampler.render(kit, &mut out, 2);
        out
    }

    /// The playback rate of the one sounding voice.
    ///
    /// ⚠ The slide *is* the rate moving, so this is the property under test.
    /// Asserting on rendered samples instead would be asserting on a test kit
    /// of constant 1.0s, which sounds identical at every pitch.
    fn step(sampler: &Sampler) -> f64 {
        sampler
            .voices
            .iter()
            .find(|v| v.active())
            .expect("a voice should still be sounding")
            .step
    }

    #[test]
    fn a_slide_arrives_at_exactly_the_pitch_it_was_asked_for() {
        // ⛔⛔ The audio half of `Note::slide_to_pitch`, which rendered as a flat
        // note until this existed. An octave down over 32 frames must land on
        // half the rate — no more, no less, or the slide ends out of tune.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                glide: Some(Glide {
                    semis: -12.0,
                    frames: 32,
                    delay: 0,
                }),
                ..Hit::default()
            },
        );

        assert!(
            (step(&sampler) - 1.0).abs() < 1e-9,
            "a slide starts at the note's own pitch"
        );
        render_block(&mut sampler, &kit, 32);
        assert!(
            (step(&sampler) - 0.5).abs() < 1e-6,
            "an octave down must be half the rate, got {}",
            step(&sampler)
        );
    }

    #[test]
    fn a_slide_is_even_in_semitones_rather_than_in_rate() {
        // ⛔ The reason `glide_mul` multiplies. Halfway through an octave slide
        // the pitch must be a tritone down — rate 2^(-6/12) ≈ 0.7071 — not the
        // arithmetic midpoint 0.75, which would sweep fast then crawl and read
        // as a pitch envelope rather than a glide.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                glide: Some(Glide {
                    semis: -12.0,
                    frames: 32,
                    delay: 0,
                }),
                ..Hit::default()
            },
        );

        render_block(&mut sampler, &kit, 16);
        let half = step(&sampler);
        assert!(
            (half - 0.5f64.sqrt()).abs() < 1e-6,
            "halfway must be a tritone (~0.7071), got {half}"
        );
    }

    #[test]
    fn a_slide_stops_when_it_arrives_rather_than_running_away() {
        // ⛔ Without the `glide_left` countdown the factor would keep compounding
        // and the note would slide off the bottom of hearing.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                glide: Some(Glide {
                    semis: -12.0,
                    frames: 16,
                    delay: 0,
                }),
                ..Hit::default()
            },
        );

        render_block(&mut sampler, &kit, 16);
        let arrived = step(&sampler);
        render_block(&mut sampler, &kit, 16);
        assert!(
            (step(&sampler) - arrived).abs() < 1e-12,
            "the rate must hold once the slide has arrived"
        );
    }

    #[test]
    fn a_slide_holds_its_own_pitch_before_it_moves() {
        // ⛔ So the WAV matches the `.mid` written beside it: `midi.rs` puts the
        // destination note-on at the note's midpoint, so the audio must sit at
        // the origin pitch until then rather than gliding from the note-on.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                glide: Some(Glide {
                    semis: -12.0,
                    frames: 16,
                    delay: 16,
                }),
                ..Hit::default()
            },
        );

        render_block(&mut sampler, &kit, 16);
        assert!(
            (step(&sampler) - 1.0).abs() < 1e-12,
            "the hold must leave the pitch alone, got {}",
            step(&sampler)
        );

        render_block(&mut sampler, &kit, 16);
        assert!(
            (step(&sampler) - 0.5).abs() < 1e-6,
            "and then it must still arrive, got {}",
            step(&sampler)
        );
    }

    #[test]
    fn a_note_with_no_slide_holds_one_rate() {
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        render_block(&mut sampler, &kit, 16);
        assert!((step(&sampler) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_slide_of_nothing_is_refused_rather_than_dividing_by_zero() {
        // ⛔ `frames: 0` divides in the factor. This is reachable from real data:
        // a slide whose window rounds to nothing at a fast tempo.
        let kit = test_kit();
        for glide in [
            Glide {
                semis: -12.0,
                frames: 0,
                delay: 0,
            },
            Glide {
                semis: 0.0,
                frames: 32,
                delay: 0,
            },
        ] {
            let mut sampler = Sampler::default();
            sampler.trigger_with(
                &kit,
                0,
                48_000.0,
                Hit {
                    glide: Some(glide),
                    ..Hit::default()
                },
            );
            render_block(&mut sampler, &kit, 8);
            let rate = step(&sampler);
            assert!(
                rate.is_finite() && (rate - 1.0).abs() < 1e-12,
                "{glide:?} must leave the rate alone, got {rate}"
            );
        }
    }

    #[test]
    fn a_triggered_pad_is_heard_immediately() {
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        let out = render_block(&mut sampler, &kit, 4);
        assert!(out[0] > 0.5, "the first frame must already carry the hit");
    }

    #[test]
    fn a_hat_choke_silences_the_hat_under_it() {
        // The open hat and the closed hat are the same piece of metal.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 1, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 1);

        sampler.trigger(&kit, 2, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 1, "the closed hat should be gone");

        // ...and a kick, which is in no group, survives both.
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        assert_eq!(sampler.active_voices(), 2);
    }

    #[test]
    fn velocity_scales_the_output_and_pan_moves_it() {
        let kit = test_kit();
        let mut sampler = Sampler::default();

        sampler.trigger(&kit, 0, 0.5, 0.0, 48_000.0);
        let half = render_block(&mut sampler, &kit, 1);
        sampler.stop_all();

        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        let full = render_block(&mut sampler, &kit, 1);
        assert!(half[0] < full[0], "velocity must change the level");

        // Hard left: the right channel stays silent.
        sampler.stop_all();
        sampler.trigger(&kit, 3, 1.0, 0.0, 48_000.0);
        let left = render_block(&mut sampler, &kit, 1);
        assert!(left[0] > 0.5, "left should carry it");
        assert!(left[1].abs() < 0.001, "right should not, got {}", left[1]);
    }

    #[test]
    fn a_pad_plays_at_its_own_rate_whatever_the_device_runs_at() {
        // The kit is 44.1 kHz and a device is usually 48. Ignoring that plays
        // every drum ~9% sharp, which is subtle enough to ship by accident.
        let kit = test_kit();
        let mut sampler = Sampler::default();

        sampler.trigger(&kit, 0, 1.0, 0.0, 96_000.0);
        // Pad at 48 kHz played at 96 kHz: half a sample per frame, so 64
        // samples last ~128 frames rather than 64.
        let out = render_block(&mut sampler, &kit, 100);
        assert!(
            out.chunks(2).all(|f| f[0] > 0.5),
            "a half-speed voice must still be sounding after 100 frames"
        );
    }

    #[test]
    fn a_voice_ends_at_the_end_of_its_sample_and_frees_its_slot() {
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        let _ = render_block(&mut sampler, &kit, 128);
        assert_eq!(
            sampler.active_voices(),
            0,
            "a finished one-shot must free up"
        );
    }

    /// A kit whose one pad is far longer than any note — a sub 808, a violin, a
    /// pad. `test_kit`'s pads are 64 frames, which is shorter than the release.
    fn long_kit() -> Kit {
        Kit {
            id: "long".into(),
            pads: vec![Pad {
                id: "sub".into(),
                lane: Lane::Melody,
                samples: Arc::from(vec![1.0f32; 48_000].into_boxed_slice()),
                sample_rate: 48_000,
                gain: 1.0,
                pan: 0.0,
                pitch_semis: 0,
                choke_group: None,
                root_note: None,
            }],
        }
    }

    #[test]
    fn a_gated_voice_stops_at_the_end_of_its_note_instead_of_stacking_up() {
        // ⛔⛔ Mike, 2026-08-12, on a sub 808 dropped on the Melody generator:
        // *"it had a lot of static and kept playing it throughout the actual
        // melody over and over again."* A one-second sample under a 500-frame
        // note ran for the whole second, so eight notes had eight copies of a
        // sub sounding at once — that stack is the static.
        let kit = long_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                hold: 500,
                ..Hit::default()
            },
        );

        let out = render_block(&mut sampler, &kit, 500);
        assert!(
            // ⚠ 0.6, not 1.0: a centred pad is `gain·cos(π/4)` ≈ 0.707 per side.
            out.chunks(2).all(|f| f[0] > 0.6),
            "the note itself must sound at full level"
        );

        // The release runs after the hold, so the voice is gone a few hundred
        // frames later and nothing of it is under the next note.
        let _ = render_block(&mut sampler, &kit, RELEASE_FRAMES as usize + 8);
        assert_eq!(
            sampler.active_voices(),
            0,
            "a gated voice must free itself once its note has ended"
        );
    }

    #[test]
    fn a_gate_closes_without_a_step_to_silence() {
        // ⚠ The ramp is the whole reason the gate does not click. Sampling it at
        // the halfway point is enough to say it is a ramp and not a cut.
        let kit = long_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                hold: 1,
                ..Hit::default()
            },
        );

        let out = render_block(&mut sampler, &kit, RELEASE_FRAMES as usize / 2);
        let last = out[out.len() - 2];
        assert!(
            (0.05..0.95).contains(&last),
            "halfway through the release the gate should be part open, was {last}"
        );
    }

    #[test]
    fn a_sample_shorter_than_its_note_is_not_stretched_to_fill_it() {
        // ⛔ Mike, 2026-08-12: *"if i drop a hihat on the melody generator, then
        // it shouldn't extend the length of the hihat."* `test_kit`'s pads are
        // 64 frames; the note is 4000. The voice ends with the sample.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        sampler.trigger_with(
            &kit,
            0,
            48_000.0,
            Hit {
                hold: 4_000,
                ..Hit::default()
            },
        );

        let _ = render_block(&mut sampler, &kit, 128);
        assert_eq!(
            sampler.active_voices(),
            0,
            "a short sample must finish on its own rather than being held open"
        );
    }

    #[test]
    fn a_drum_still_rings_out_past_the_length_of_its_note() {
        // ⚠ The other half of the rule: only melodic lanes are gated, or every
        // kit in the product would change how it sounds. `trigger` is the drum
        // path and passes `RINGS_OUT`.
        let kit = long_kit();
        let mut sampler = Sampler::default();
        sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);

        let out = render_block(&mut sampler, &kit, 4_000);
        assert!(
            out[out.len() - 2] > 0.6,
            "an ungated voice must still be at full level long after a note would have ended"
        );
    }

    #[test]
    fn the_gate_is_for_melodic_notes_that_are_not_sliding() {
        // ⛔⛔ **A SLIDE ARRIVES ON EXACTLY THE FRAME THE GATE WOULD CLOSE.**
        // `Glide` holds for `len / 2` and travels the rest, so gating a sliding
        // note silences it at the instant it reaches its destination pitch — the
        // one thing an 808 slide is *for*. Only `generators::bass` writes
        // `slide_to_pitch` on a melodic lane, so this exempts basslines and
        // leaves every melody, counter and chord note gated, which is where the
        // stacking Mike reported was.
        assert_eq!(
            hold_for(Lane::Melody, 500, false),
            500,
            "a melodic note is gated"
        );
        assert_eq!(
            hold_for(Lane::Bass, 500, true),
            RINGS_OUT,
            "a sliding note must be heard where it lands"
        );
        assert_eq!(
            hold_for(Lane::Kick, 500, false),
            RINGS_OUT,
            "a drum has always rung out and must go on doing so"
        );
        // ⚠ `Sub` is pitched but it is a *kit* lane, authored inside the drums
        // part — see `roles::is_melodic`, which excludes it deliberately.
        assert_eq!(hold_for(Lane::Sub, 500, false), RINGS_OUT);
    }

    #[test]
    fn running_out_of_voices_steals_the_oldest_rather_than_dropping_the_newest() {
        // The newest hit is the one being listened for.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        for _ in 0..MAX_VOICES + 8 {
            sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        }
        assert_eq!(sampler.active_voices(), MAX_VOICES);

        let out = render_block(&mut sampler, &kit, 1);
        assert!(out[0] > 0.0, "the pool must still be producing audio");
    }

    #[test]
    fn the_limiter_leaves_an_ordinary_mix_alone() {
        for x in [0.0, 0.1, -0.25, 0.6, -0.6] {
            assert_eq!(soft_clip(x), x, "{x} is inside the knee");
        }
    }

    #[test]
    fn the_limiter_bounds_anything_without_a_corner() {
        // Bounded everywhere: full scale is the ceiling and nothing may pass
        // it, however much is stacked up. The curve approaches 1.0 and reaches
        // it only where f32 runs out of room to tell the difference.
        let mut x = -64.0f32;
        while x <= 64.0 {
            assert!(soft_clip(x).abs() <= 1.0, "{x} left the range");
            x += 0.05;
        }

        // Strictly monotonic across the range a mix actually occupies. A hard
        // clamp is bounded too — this is what it fails, because everything past
        // the threshold collapses onto one value and the differences between
        // loud things stop existing.
        let mut previous = f32::NEG_INFINITY;
        let mut x = -2.0f32;
        while x <= 2.0 {
            let y = soft_clip(x);
            assert!(y > previous, "not monotonic at {x}: {y} after {previous}");
            previous = y;
            x += 0.01;
        }

        // The slope either side of the knee matches: below it the gain is 1,
        // and immediately above it must still be ~1, not a step.
        let below = (soft_clip(LIMIT_KNEE) - soft_clip(LIMIT_KNEE - 0.001)) / 0.001;
        let above = (soft_clip(LIMIT_KNEE + 0.001) - soft_clip(LIMIT_KNEE)) / 0.001;
        assert!(
            (below - above).abs() < 0.01,
            "kink at the knee: {below} vs {above}"
        );
    }

    #[test]
    fn a_stack_of_voices_cannot_crack_the_output() {
        // Every voice in the pool at full velocity on the same pad — far more
        // than playback produces, and the point is that it still cannot clip.
        let kit = test_kit();
        let mut sampler = Sampler::default();
        for _ in 0..MAX_VOICES {
            sampler.trigger(&kit, 0, 1.0, 0.0, 48_000.0);
        }

        let mut out = render_block(&mut sampler, &kit, 8);
        assert!(
            out.iter().any(|s| s.abs() > 1.0),
            "the raw mix should be over full scale, or this proves nothing"
        );
        limit(&mut out);
        assert!(
            out.iter().all(|s| s.abs() <= 1.0),
            "the limiter let something past full scale"
        );
    }
}
