//! Rendering a pattern to audio, offline (TASK-131F).
//!
//! Mike, 2026-08-05: *"i also need to be able to export the drums by themselves
//! as midi or audio stems or drag them into a daw as midi or audio stems."*
//!
//! ⛔ **This was blocked until TASK-131A/131B, and the block was measured rather
//! than assumed.** `audio::Kit::pad_for` answered `None` for Melody, Counter,
//! Bass and Chords, so rendering those parts would have written four silent
//! files and called them stems — which is worse than not offering them, because
//! a producer imports them, hears nothing, and blames their DAW. That is
//! verbatim why TASK-069 shipped MIDI stems instead. The shipped kit now covers
//! every generated lane, and a producer's own one-shots cover the rest, so the
//! honest answer changed.
//!
//! ## Offline, not real time
//!
//! Nothing here runs on the audio thread. It allocates a whole buffer, walks the
//! pattern once, and hands back bytes — the same [`sampler::Sampler`] the
//! callback uses, driven by a loop instead of by the host. That is the point of
//! the sampler being a pure function of its inputs.

use engine::pattern::{Pattern, PPQ};

use super::kit::Kit;
use super::sampler::{self, Glide, Sampler};

/// The rate stems are written at. 44.1 kHz because that is what the kit is, so
/// the common case resamples nothing.
pub const RATE: u32 = 44_100;

/// How long a stem may be, in seconds.
///
/// ⛔ **A bound, because a `Pattern` can arrive from a project file.** Bars and
/// tempo are both attacker-controlled through the same door `check_song`
/// already guards, and `bars * ticks_per_bar` at 60 BPM is minutes of stereo
/// f32 per part.
///
/// ⚠ **Raised from five minutes when a whole arrangement became renderable.**
/// Five was "far past any loop the plugin generates", and that was true while
/// only a four-bar clip could reach here. Once a *song* could, the same number
/// became a silent truncation of any record longer than it — the clamp below
/// shortens the buffer and nothing says so, which is the readout-that-lies
/// failure this project keeps recording. Fifteen minutes is past any record
/// anybody arranges, and [`too_long_to_render`] is what refuses rather than
/// quietly cutting.
pub(crate) const MAX_SECONDS: u32 = 900;

/// How long this clip would render for, in seconds.
///
/// ⛔ **So a caller can refuse before it renders, rather than after.** The
/// buffer is clamped to [`MAX_SECONDS`] and a clamp cannot report — it just
/// hands back a shorter file. Anything that could exceed the bound has to ask
/// first, and say so in words the producer can act on.
pub(crate) fn seconds_of(pattern: &Pattern) -> f64 {
    let ticks = pattern
        .ticks_per_bar()
        .saturating_mul(u32::from(pattern.bars));
    f64::from(ticks) * (60.0 / f64::from(tempo(pattern)) / f64::from(PPQ)) + TAIL_SECONDS
}

/// Would rendering this clip run past what [`MAX_SECONDS`] allows?
pub(crate) fn too_long_to_render(pattern: &Pattern) -> bool {
    seconds_of(pattern) > f64::from(MAX_SECONDS)
}

/// Refuse, in words a producer can act on, if any of these would run past it.
///
/// ⛔⛔ **Every path that renders audio has to call this, and for a while only
/// one did.** [`MAX_SECONDS`] was tripled when a whole arrangement became
/// draggable, and the refusal that was supposed to compensate was wired into the
/// song drag alone — so the pattern drag, `start_pattern_stems` and
/// `start_song_stems` all still met the *clamp*, now at three times the
/// allocation. A clamp cannot report: it hands back a shorter buffer and says
/// nothing, and the producer finds out in their arrangement rather than here.
///
/// ⚠ `audio` rather than an `Option<&Kit>`, because the bound is about rendered
/// samples. MIDI has no length limit worth enforcing — a long `.mid` is a few
/// more bytes.
pub(crate) fn refuse_if_too_long(patterns: &[Pattern], audio: bool) -> Result<(), String> {
    if audio && patterns.iter().any(too_long_to_render) {
        return Err(format!(
            "this is longer than {} minutes, which is as much audio as the plugin \
             will render at once — use MIDI instead, or work in shorter sections",
            MAX_SECONDS / 60
        ));
    }
    Ok(())
}

/// Let every voice finish rather than cutting the last hit dead.
const TAIL_SECONDS: f64 = 2.0;

/// Render one pattern through `kit` into interleaved stereo f32.
///
/// ⚠ **Silence is returned as `None`, not as a buffer of zeros.** A lane the kit
/// has no pad for renders nothing, and writing that to disk is the "four silent
/// files called stems" failure this module's header exists to record. The caller
/// skips the file instead.
pub fn to_stereo(pattern: &Pattern, kit: &Kit) -> Option<Vec<f32>> {
    let bpm = f64::from(tempo(pattern));
    let seconds_per_tick = 60.0 / bpm / f64::from(PPQ);

    // ⚠ The clip's own, not a copy of the formula. This used to restate
    // `SessionContext::ticks_per_bar` and `Song::ticks_per_bar` inline with a
    // comment insisting the three agreed — a claim no compiler was checking.
    // All of them delegate to `pattern::ticks_per_bar_of` now.
    let ticks = pattern
        .ticks_per_bar()
        .saturating_mul(u32::from(pattern.bars));
    let frames = (f64::from(ticks) * seconds_per_tick * f64::from(RATE)) as usize
        + (TAIL_SECONDS * f64::from(RATE)) as usize;
    let frames = frames.min(MAX_SECONDS as usize * RATE as usize);
    if frames == 0 {
        return None;
    }

    // Every note in the pattern, in time order, with the frame it starts on.
    // Collected first so the render is one pass over a sorted list rather than
    // a search per frame.
    let mut hits: Vec<(usize, usize, f32, f32, Option<Glide>)> = Vec::new();
    for track in &pattern.lanes {
        let Some(pad_index) = kit.pad_for(track.lane) else {
            // ⛔ Skipped, never defaulted to a nearby pad — `pad_for` refuses to
            // guess for the reason its own doc gives, and this must not undo
            // that by picking one here.
            continue;
        };
        let pad = &kit.pads[pad_index];
        for note in &track.notes {
            let at = (f64::from(note.start_tick) * seconds_per_tick * f64::from(RATE)) as usize;
            if at >= frames {
                continue;
            }
            // Percussion transposes from the lane's own GM note and a pitched
            // pad from its root — the same rule `render_preview` follows, so a
            // stem sounds like the preview did (TASK-131D).
            let semis = Kit::semitones_for(pad, track.lane, note.pitch);
            // ⛔ **The 808 slide, which this rendered as a flat note until
            // 2026-08-06.** The engine has written `slide_to_pitch` for as long
            // as `glideProb` and `slideProb` have existed and `midi.rs` has
            // encoded it for as long — so a producer got a gliding `.mid` and a
            // dead-flat `.wav` out of the same drag. Mike: *"ensure i have 808
            // slides … for my audio being dragged into the DAW or audio being
            // exported."*
            let glide = note.slide_to_pitch.map(|target| {
                let frames =
                    (f64::from(note.len_ticks) * seconds_per_tick * f64::from(RATE)) as u32;
                Glide {
                    // Travel measured the same way the pitch was, so a pad with
                    // its own root or offset cannot make the two disagree.
                    semis: Kit::semitones_for(pad, track.lane, target) - semis,
                    // ⚠ Half the note each, matching where `midi.rs` puts the
                    // destination note-on — the origin is held, then it moves.
                    delay: frames / 2,
                    frames: frames - frames / 2,
                }
            });
            hits.push((at, pad_index, f32::from(note.vel) / 127.0, semis, glide));
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|(at, _, _, _, _)| *at);

    let mut out = vec![0.0f32; frames * 2];
    let mut sampler = Sampler::default();
    let mut next = 0usize;
    let mut at = 0usize;
    while at < frames {
        while next < hits.len() && hits[next].0 <= at {
            let (_, pad, velocity, semis, glide) = hits[next];
            sampler.trigger_with(kit, pad, velocity, semis, f64::from(RATE), glide);
            next += 1;
        }
        // Render up to the next trigger, so a hit lands on its own frame rather
        // than being quantised to a block — the same reason `render_preview`
        // splits its block.
        let until = hits
            .get(next)
            .map(|(frame, _, _, _, _)| (*frame).min(frames))
            .unwrap_or(frames)
            .max(at + 1);
        sampler.render(kit, &mut out[at * 2..until * 2], 2);
        at = until;
    }

    sampler::limit(&mut out);
    let peak = out.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    // A rendered part that is entirely silent is the failure this module exists
    // to avoid writing to disk.
    (peak > 1.0e-4).then_some(out)
}

/// The `acid` chunk's body, in bytes. The layout below is fixed at 24.
const ACID_BODY: usize = 24;

/// The tempo to render and to declare at, with a corrupt one made safe.
///
/// ⛔ **One place, because two would let the file disagree with its own audio.**
/// [`to_stereo`] divides by this to place every hit, and [`write_acid`] writes it
/// into a chunk a DAW believes — so if the two guards ever drifted, the WAV
/// would declare a tempo the samples inside it were not rendered at. That is the
/// exact defect the `acid` chunk was added to fix, arriving from the other side.
fn tempo(pattern: &Pattern) -> f32 {
    if pattern.bpm.is_finite() && pattern.bpm > 1.0 {
        pattern.bpm
    } else {
        // A pattern with no usable tempo still has to render *something* rather
        // than divide by zero; 120 is the neutral guess and only a corrupt
        // project reaches it.
        120.0
    }
}

/// Interleaved stereo f32 as a 16-bit PCM WAV, tempo included.
///
/// ⚠ **A writer, where `kit::decode_wav` is the reader.** They are deliberately
/// separate: that one accepts a little more than this writes, because it has to
/// survive a file somebody replaced. This one emits exactly one format.
///
/// ⛔⛔ **The `acid` chunk is not decoration — it is the fix for a defect Mike
/// hit in Ableton on 2026-08-06:** *"the bpm was set to 120 in my DAW … as soon
/// as I drag the audio out into the DAW, then it showed that each sample of
/// audio output was only playing at 96 bpm"*. This wrote `RIFF`, `WAVEfmt ` and
/// `data` and nothing else, so the file said how fast to play its *samples* and
/// never how fast to play its *music*. Ableton had nothing to read, fell back to
/// warping by guess, and guessed 96.
///
/// ⚠ `pattern` is taken for its tempo and meter alone — no audio comes from it.
/// The samples were rendered from the same pattern by [`to_stereo`], and the two
/// must describe one clip or the file contradicts itself.
pub fn to_wav(samples: &[f32], pattern: &Pattern) -> Vec<u8> {
    const CHANNELS: u16 = 2;
    const BITS: u16 = 16;

    let data_len = samples.len() * 2;
    let acid_len = 8 + ACID_BODY;
    let mut out = Vec::with_capacity(44 + acid_len + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + acid_len + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    let block = u32::from(CHANNELS) * u32::from(BITS) / 8;
    out.extend_from_slice(&(RATE * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    write_acid(&mut out, pattern);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());

    for sample in samples {
        // ⛔ Clamped before the cast. `as i16` on an out-of-range float
        // saturates in Rust, but the limiter has already bounded this to ±1 and
        // relying on the cast's behaviour instead of saying so is how a
        // rounding change becomes a click.
        let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

/// Say how fast this is meant to play, in the chunk loop libraries use.
///
/// ⛔ **`acid` rather than anything tidier, because the requirement is that
/// *Ableton and FL already read it*.** It is the de-facto tempo chunk — every
/// commercial loop pack ships it — which is the only property that matters here:
/// a format both hosts ignore leaves the guess in place, and the guess is the
/// defect. `bext` carries no tempo and `smpl` carries loop points but no tempo,
/// so neither is a substitute.
///
/// The layout is fixed and undocumented by any standard; these are the fields as
/// every reader expects them, in order:
///
/// | bytes | field |
/// |---|---|
/// | 4 | flags |
/// | 2 | root note |
/// | 2 | always `0x8000` |
/// | 4 | always `0.0` |
/// | 4 | beats |
/// | 2 | meter denominator |
/// | 2 | meter numerator |
/// | 4 | tempo |
fn write_acid(out: &mut Vec<u8>, pattern: &Pattern) {
    // ⛔ **`0x04` — "stretch" — and NOT `0x01`.** Bit 0 is *one shot*, and a one
    // shot is exactly the thing a host must never warp: setting it would tell
    // Ableton to ignore the tempo written four fields further down, which is the
    // guess this chunk exists to replace. Bit 1 (root note is set) stays clear
    // because we do not write one; the field below is the conventional filler.
    out.extend_from_slice(b"acid");
    out.extend_from_slice(&(ACID_BODY as u32).to_le_bytes());
    out.extend_from_slice(&0x0000_0004u32.to_le_bytes());
    out.extend_from_slice(&60u16.to_le_bytes());
    out.extend_from_slice(&0x8000u16.to_le_bytes());
    out.extend_from_slice(&0.0f32.to_le_bytes());

    // ⚠ **The musical loop, not the length of the file.** `to_stereo` renders
    // `TAIL_SECONDS` past the end so the last hit rings out rather than being
    // cut dead, so the file is deliberately longer than the loop it contains.
    // Declaring the file's length here would tell the host to squeeze a decaying
    // cymbal into the bar count and stretch everything by however long the tail
    // happened to be.
    //
    // ⚠ The clip's effective meter, from the one place that decides it —
    // `pattern::normalise_meter`, which `ticks_per_bar` is also built on. This
    // used to restate the zero-denominator fallback inline, which made it the
    // fourth copy of a rule whose own comments insisted the copies agreed.
    let (num, den) = pattern.time_sig();

    // ⛔⛔ **QUARTER notes, not bars × numerator — and the difference is the
    // whole defect for any meter whose denominator is not 4.** A reader takes
    // this field with the tempo below it, and that tempo is *beats per minute*
    // in the only sense `to_stereo` renders: `60 / bpm / PPQ` seconds a tick,
    // so one beat is one `PPQ`, so one beat is a quarter note.
    //
    // `bars * num` is quarter notes only when `den == 4`. Four bars of 6/8 is 12
    // quarter notes of audio and this used to declare 24 — so Ableton read twice
    // the music out of the file, warped the stem to half speed, and the loop
    // played at the wrong tempo. That is the exact defect this chunk was added
    // to fix, reintroduced through the meter picker, which offers 6/8, 9/8, 12/8
    // and 7/8. Deriving it from `ticks_per_bar` instead means it cannot disagree
    // with what was rendered, because it is the same expression.
    //
    // ⚠ **Rounded, because the field is an integer and some meters are not.**
    // One bar of 7/8 is three and a half quarter notes and `acid` has nowhere to
    // put the half. Rounding is at worst half a beat out over the whole clip;
    // truncating would be up to a whole one, always short.
    let ticks = pattern
        .ticks_per_bar()
        .saturating_mul(u32::from(pattern.bars));
    let beats = ((f64::from(ticks) / f64::from(PPQ)).round() as u32).max(1);
    out.extend_from_slice(&beats.to_le_bytes());
    out.extend_from_slice(&u16::from(den).to_le_bytes());
    out.extend_from_slice(&u16::from(num).to_le_bytes());

    // ⛔ The same tempo `to_stereo` rendered at, from the same function — a
    // corrupt project must not write `inf` or `0` into a field a DAW believes,
    // and the file must not declare a tempo its samples were not made at.
    out.extend_from_slice(&tempo(pattern).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale};

    fn pattern_with(lane: Lane, pitch: u8) -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "trap".into(),
            seed: 1,
            song_seed: 1,
            bars: 1,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane,
                notes: vec![Note {
                    start_tick: 0,
                    len_ticks: 240,
                    pitch,
                    vel: 110,
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                }],
            }],
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    #[test]
    fn a_pattern_renders_to_audible_stereo() {
        // ⛔ **The claim TASK-069 could not make.** It shipped MIDI stems
        // because the melodic parts rendered pure silence; this is what changed.
        let kit = crate::audio::preview_kit().expect("the shipped kit must load");
        let rendered =
            to_stereo(&pattern_with(Lane::Kick, 36), kit).expect("a kick must render audio");

        let peak = rendered.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(peak > 0.01, "the stem is silent (peak {peak})");
        assert!(peak <= 1.0, "the limiter let something past full scale");
        // One bar at 120 BPM is two seconds, plus the tail.
        assert!(rendered.len() >= 2 * RATE as usize * 2, "too short");
    }

    #[test]
    fn every_generated_part_renders_rather_than_writing_a_silent_file() {
        // ⛔ **The reason this module could not exist before TASK-131A.** Four
        // parts out of five had no pad, so a stem export would have written
        // four silent files and called them stems.
        let kit = crate::audio::preview_kit().unwrap();
        for (lane, pitch) in [
            (Lane::Kick, 36),
            (Lane::Snare, 38),
            (Lane::Melody, 84),
            (Lane::Counter, 72),
            (Lane::Bass, 36),
            (Lane::Chords, 60),
            (Lane::Sub, 28),
        ] {
            let rendered = to_stereo(&pattern_with(lane, pitch), kit)
                .unwrap_or_else(|| panic!("{lane:?} rendered nothing"));
            let peak = rendered.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.01, "{lane:?} rendered silence");
        }
    }

    #[test]
    fn a_lane_the_kit_cannot_play_is_none_rather_than_a_file_of_zeros() {
        // Writing out a lane the kit cannot play would be the exact failure
        // this module's header records: a `.wav` of silence that looks like a
        // successful export.
        //
        // ⚠ The kit has one pad removed rather than being trusted to lack a
        // lane. This used to lean on `Snap` shipping with no pad; TASK-140 gave
        // every lane a default, and a rule about `to_stereo` should never have
        // depended on what the kit happened to omit.
        let mut kit = crate::audio::preview_kit().unwrap().as_ref().clone();
        kit.pads.retain(|pad| pad.lane != Lane::Snap);
        assert!(to_stereo(&pattern_with(Lane::Snap, 39), &kit).is_none());

        // The shipped kit does play it, so the export is real rather than empty.
        let shipped = crate::audio::preview_kit().unwrap();
        assert!(to_stereo(&pattern_with(Lane::Snap, 39), shipped).is_some());
    }

    #[test]
    fn the_wav_round_trips_through_our_own_reader() {
        // ⛔ The writer and the reader are separate on purpose, so this is what
        // stops them drifting apart. A header this reader cannot parse is a file
        // a DAW may not parse either.
        // ⚠ **And that it still round-trips with the `acid` chunk in the middle
        // of the header**, which is the half this nearly lost: a reader that
        // assumed `fmt ` was followed by `data` at a fixed offset would read the
        // tempo chunk as audio. `decode_wav` walks the chunk list, and this is
        // what proves it still does.
        let samples = vec![0.5f32, -0.5, 0.25, -0.25];
        let bytes = to_wav(&samples, &pattern_with(Lane::Kick, 36));
        let decoded = crate::audio::kit::decode_wav(&bytes).expect("our own WAV must decode");

        assert_eq!(decoded.sample_rate, RATE);
        // Two stereo frames, downmixed to mono by the reader.
        assert_eq!(decoded.samples.len(), 2);
        assert!(decoded.samples.iter().all(|s| s.abs() < 0.01), "L+R cancel");
    }

    /// The body of a named chunk, found by walking the list the way a DAW does.
    fn chunk<'a>(bytes: &'a [u8], want: &[u8; 4]) -> Option<&'a [u8]> {
        let mut cursor = 12usize;
        while cursor + 8 <= bytes.len() {
            let size =
                u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
            let body = &bytes[cursor + 8..(cursor + 8 + size).min(bytes.len())];
            if &bytes[cursor..cursor + 4] == want {
                return Some(body);
            }
            cursor += 8 + size + (size & 1);
        }
        None
    }

    /// A pattern at a tempo and length that no fallback in this module shares.
    fn at_140_over_four_bars() -> Pattern {
        Pattern {
            bpm: 140.0,
            bars: 4,
            ..pattern_with(Lane::Kick, 36)
        }
    }

    /// ⛔⛔ The gate for the defect Mike found in Ableton on 2026-08-06.
    ///
    /// ⚠ **140 and 4 bars deliberately, because every fallback in this file is
    /// 120 and 1.** Asserted at the module's own defaults, a writer that had
    /// stopped reading the pattern entirely would pass this and the file would
    /// still be wrong for every artist who is not at 120.
    #[test]
    fn the_wav_says_what_tempo_it_was_rendered_at() {
        let pattern = at_140_over_four_bars();
        let bytes = to_wav(&[0.0f32; 8], &pattern);
        let acid = chunk(&bytes, b"acid").expect("no tempo chunk, so the host has to guess");

        assert_eq!(acid.len(), ACID_BODY);
        let tempo = f32::from_le_bytes(acid[20..24].try_into().unwrap());
        assert!(
            (tempo - 140.0).abs() < 0.001,
            "the file claims {tempo} BPM, so a 120 project warps it by guess"
        );
    }

    #[test]
    fn a_stem_is_never_marked_as_a_one_shot() {
        // ⛔ Bit 0 tells the host *not* to warp, which would make the tempo four
        // fields along dead weight — the guess would stay exactly where it was.
        let bytes = to_wav(&[0.0f32; 8], &at_140_over_four_bars());
        let acid = chunk(&bytes, b"acid").unwrap();
        let flags = u32::from_le_bytes(acid[0..4].try_into().unwrap());
        assert_eq!(flags & 0x01, 0, "a one-shot is never warped to the project");
    }

    #[test]
    fn the_declared_length_is_the_loop_rather_than_the_rendered_file() {
        // ⚠ `to_stereo` renders `TAIL_SECONDS` past the end so the last hit
        // rings out. Declaring the *file* here would tell the host to squeeze
        // that decay into the bar count and stretch the whole loop with it.
        let pattern = at_140_over_four_bars();
        let bytes = to_wav(&[0.0f32; 8], &pattern);
        let acid = chunk(&bytes, b"acid").unwrap();

        let beats = u32::from_le_bytes(acid[12..16].try_into().unwrap());
        assert_eq!(beats, 16, "four bars of 4/4 is sixteen beats");
        assert_eq!(u16::from_le_bytes(acid[16..18].try_into().unwrap()), 4);
        assert_eq!(u16::from_le_bytes(acid[18..20].try_into().unwrap()), 4);

        // And the rendered audio really is longer than that, which is what makes
        // the distinction above worth asserting rather than theoretical.
        let kit = crate::audio::preview_kit().unwrap();
        let rendered = to_stereo(&pattern, kit).unwrap();
        let loop_frames = f64::from(beats) * 60.0 / 140.0 * f64::from(RATE);
        assert!((rendered.len() / 2) as f64 > loop_frames);
    }

    /// ⛔⛔ **The declared length is in QUARTER notes, so an x/8 meter is not
    /// `bars × numerator`.** This is the case all three tests above miss: every
    /// one of them uses the 4/4 fixture, where the two happen to agree.
    ///
    /// `RollBar.tsx` offers 6/8, 9/8, 12/8 and 7/8. Four bars of 6/8 is twelve
    /// quarter notes of rendered audio, and declaring 24 told Ableton to read
    /// twice as much music out of the file as it holds — so it warped the stem
    /// to half speed. That is the *same* defect the chunk was added to fix,
    /// arriving through the meter picker instead of through the tempo field.
    #[test]
    fn a_compound_meter_declares_the_beats_it_actually_rendered() {
        let pattern = Pattern {
            time_sig_num: 6,
            time_sig_den: 8,
            bars: 4,
            ..at_140_over_four_bars()
        };
        let bytes = to_wav(&[0.0f32; 8], &pattern);
        let acid = chunk(&bytes, b"acid").unwrap();

        let beats = u32::from_le_bytes(acid[12..16].try_into().unwrap());
        assert_eq!(
            beats, 12,
            "four bars of 6/8 is twelve quarter notes; {beats} would warp the stem"
        );
        // The meter itself is still reported as written — only the beat count is
        // converted, because that is the field the tempo is read against.
        assert_eq!(u16::from_le_bytes(acid[16..18].try_into().unwrap()), 8);
        assert_eq!(u16::from_le_bytes(acid[18..20].try_into().unwrap()), 6);

        // ⛔ And the declaration matches the audio, which is the property that
        // actually matters: the file must not claim a length it does not hold.
        let kit = crate::audio::preview_kit().unwrap();
        let rendered = to_stereo(&pattern, kit).unwrap();
        let declared = f64::from(beats) * 60.0 / 140.0 * f64::from(RATE);
        let actual = (rendered.len() / 2) as f64 - TAIL_SECONDS * f64::from(RATE);
        assert!(
            (declared - actual).abs() < f64::from(RATE) * 0.01,
            "declared {declared} frames of loop, rendered {actual}"
        );
    }

    #[test]
    fn the_riff_size_counts_every_chunk_including_the_new_one() {
        // ⛔ The failure this catches is silent and total: a RIFF size that does
        // not cover the file makes strict readers stop early, and the stem
        // arrives truncated or refused with nothing saying why.
        let bytes = to_wav(&[0.25f32; 64], &at_140_over_four_bars());
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared, bytes.len() - 8);
    }

    #[test]
    fn full_scale_survives_the_write_without_wrapping() {
        // A float past ±1 cast straight to i16 is the classic way a loud stem
        // comes back as a click.
        let bytes = to_wav(&[1.0, -1.0, 2.0, -2.0], &pattern_with(Lane::Kick, 36));
        let decoded = crate::audio::kit::decode_wav(&bytes).unwrap();
        assert!(
            decoded.samples.iter().all(|s| s.abs() <= 1.0),
            "a sample wrapped: {:?}",
            decoded.samples
        );
    }
}
