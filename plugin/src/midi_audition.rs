//! Hearing a `.mid` from the browser (TASK-160).
//!
//! Mike, 2026-08-10: *".mid files … have its own sound like Ableton does that
//! can play the .mid file"*.
//!
//! ## ⛔⛔ Rendered to a buffer, NOT scheduled on the audio thread
//!
//! The roadmap said the cost of this feature was *"its own note scheduler"*, and
//! that is the expensive answer rather than the necessary one. A scheduler on the
//! audio thread means new lock-free state, a second idea of "now" running beside
//! the transport's, and every real-time rule re-derived for a feature nobody
//! plays a set with.
//!
//! ▶ **A `.mid` from the browser is a handful of seconds and is not live.** So it
//! is rendered here, on the editor thread, into exactly the shape
//! [`crate::preview::Preview::load`] already takes — the same `Vec<f32>` a decoded
//! `.wav` arrives as. Play, pause, stop, seek, click-to-play-from-there, loop,
//! reverse, the position poll and the whole transport strip then work on a MIDI
//! file because they are the *same code*, tested once.
//!
//! ⚠ **And it cannot touch the transport**, which is TASK-058B's rule for this
//! panel, because `Preview` is a separate voice from `Schedule` and always has
//! been. Auditioning a file is not playing the project.
//!
//! ## ⛔ A neutral instrument, deliberately
//!
//! The roadmap left the choice open — *"the current kit, or a neutral built-in
//! preview instrument the way Ableton uses a default piano"* — and named the
//! reason the second wins: **it sounds the same whatever artist happens to be
//! selected**. A producer auditioning a bass loop is asking "what are the notes",
//! and answering through whichever kit is loaded makes the same file sound
//! different on Tuesday.
//!
//! So: a plucked tone with three harmonics and a percussive decay for pitched
//! lanes, and a filtered noise burst for drum lanes. It is not a piano and does
//! not pretend to be; it is legible, which is what an audition is for.
//!
//! ⚠ **`roles::is_melodic` decides which is which** — the one list, the same one
//! the sampler's note gate and the dice both read. A second answer here is how
//! the two come to disagree about what a drum lane is.

use engine::pattern::Pattern;

/// What the buffer is rendered at.
///
/// ⚠ **A constant rather than the device's rate**, because `Preview` resamples on
/// the way out: it already holds a `rate` per buffer for exactly this reason — a
/// 44.1k `.wav` on a 48k device is the ordinary case. Rendering at the device
/// rate would mean reading it from the audio thread's side, which is a
/// synchronisation problem this does not need to have.
const RENDER_RATE: u32 = 48_000;

/// The longest audition, in seconds.
///
/// ⛔ **A bound on the allocation, and it has to exist.** A `.mid` may declare a
/// tempo of 1 BPM and a note at bar 300, and `MAX_MIDI_BYTES` does not bound the
/// *duration* a file describes — a few kilobytes of MIDI can ask for hours of
/// audio. At this length the buffer is 23 MB, which is already the largest thing
/// this plugin allocates for a preview; unbounded it is a page-supplied
/// out-of-memory.
///
/// ⚠ Two minutes is past any loop or any song section somebody auditions from a
/// browser, and the reply says when it clipped rather than clipping in silence.
const MAX_SECONDS: f32 = 120.0;

/// The most sample-writes one render may do, across every note.
///
/// ⛔⛔ **[`MAX_SECONDS`] bounds the memory and NOT the work, and the difference
/// is a hang.** The buffer is capped; the number of notes mixed *into* it is not.
/// `MAX_MIDI_BYTES` is 1 MB, a note-on/note-off pair is a handful of bytes, so a
/// legal file the containment checks all wave through can carry on the order of
/// **130,000 notes** — and each one may sound for the whole two minutes, which is
/// 5.7 million writes. That product is 10^11 iterations, on the **host's editor
/// thread**, from a producer clicking Play on a file somebody sent them. Ableton
/// stops drawing.
///
/// ⚠ **Frames, not a note count**, because that is the quantity the cost is
/// actually in: ten thousand sixteenth notes are cheap and a hundred two-minute
/// drones are not. At roughly a nanosecond a frame this is a few hundred
/// milliseconds worst case, and any real loop or song section is orders of
/// magnitude below it.
///
/// ⚠ Running out sets `clipped`, which already means "you are not hearing all of
/// it" — the same word for the same promise, rather than a second flag saying a
/// thing the producer cannot act on differently.
///
/// ⚠ **Twenty million, which is generous and still fast.** A four-bar loop with
/// sixty notes a second long is under three million; the whole two-minute buffer
/// sounding once end to end is five point eight. Each frame costs three `sin`
/// calls on a melodic note — the decay is stepped rather than evaluated, see
/// `mix` — so this is a couple of hundred milliseconds at the very worst.
const MAX_MIXED_FRAMES: u64 = 20_000_000;

/// How long a note takes to fade once it ends, in seconds.
///
/// ⛔ **Not optional, and `sampler::RELEASE_FRAMES` records why in the same
/// words**: cutting a tone mid-cycle is a click on **every note**. Longer here
/// than the sampler's 5 ms because these are synthesised tones rather than
/// recorded one-shots — a plucked note that stops dead at its `lenTicks` sounds
/// like a fault rather than like a short note.
const RELEASE_SECONDS: f32 = 0.06;

/// The audition of one MIDI file: mono, [`RENDER_RATE`], and whether it clipped.
pub struct Audition {
    pub samples: Vec<f32>,
    pub rate: u32,
    /// True when the file was longer than [`MAX_SECONDS`] and was cut.
    ///
    /// ⚠ **Reported rather than trimmed in silence** — the rule
    /// `explorer::State::truncated` states one module over: a producer who does
    /// not hear the end of their file and is told nothing concludes the audition
    /// is broken.
    pub clipped: bool,
}

/// Render every note of every part into one buffer.
///
/// ⛔ **All the parts together, which is what "play the file" means.** The split
/// exists so a producer can send parts to different generators; hearing the file
/// is hearing the file. Playing only the part the panel happened to list first
/// would be an audition of something that is not on disk.
pub fn render(parts: &[engine::smf_read::SplitPart]) -> Audition {
    let mut end = 0.0f32;
    // ⚠ Counted in the pass that is already walking every note, rather than in a
    // second one: whether the file is empty is knowable here for free.
    let mut notes = 0usize;
    for part in parts {
        for lane in &part.pattern.lanes {
            for note in &lane.notes {
                notes += 1;
                // ⚠ **Saturating, because these two are a `.mid`'s numbers and
                // not ours.** A file may declare a start near `u32::MAX` and a
                // length beside it; the plain add panics in a debug build and
                // wraps to a *small* number in release — which would silently
                // size the buffer to nothing and audition a long file as
                // silence. Saturating gives the honest answer, and `MAX_SECONDS`
                // clamps it a few lines down.
                let stop = seconds_at(
                    &part.pattern,
                    note.start_tick.saturating_add(note.len_ticks),
                );
                if stop > end {
                    end = stop;
                }
            }
        }
    }

    let clipped = end > MAX_SECONDS;
    // ⚠ **An empty file is an empty buffer, not a short silent one.**
    // `Preview::load` reads zero frames as "nothing to play" and the transport
    // shows `0:00 / 0:00`; adding the release tail to nothing would give it a
    // playable 0.06 seconds of silence, which is a readout claiming there is
    // something there. A legal `.mid` with no notes is still legal, so this is
    // not an error either — the panel already says what a file contains.
    //
    // ⚠ Asked of the **note count**, not of `end`: a file whose notes are all
    // zero-length at tick 0 has `end == 0` and is not empty.
    let length = if notes == 0 {
        0.0
    } else {
        end.min(MAX_SECONDS) + RELEASE_SECONDS
    };
    let frames = (length * RENDER_RATE as f32).ceil() as usize;
    let mut samples = vec![0.0f32; frames];

    let mut voices = 0usize;
    // ⛔ See [`MAX_MIXED_FRAMES`]: the buffer is bounded and the work poured into
    // it is not, and a legal 1 MB file can ask for 10^11 writes on the host's
    // editor thread.
    let mut budget = MAX_MIXED_FRAMES;
    let mut exhausted = false;
    'notes: for part in parts {
        for lane in &part.pattern.lanes {
            let melodic = crate::roles::is_melodic(lane.lane);
            for note in &lane.notes {
                let at = seconds_at(&part.pattern, note.start_tick);
                if at > MAX_SECONDS {
                    continue;
                }
                let held = seconds_at(&part.pattern, note.len_ticks).max(0.01);
                let written = mix(&mut samples, at, held, note.pitch, note.vel, melodic);
                voices += 1;
                budget = budget.saturating_sub(written);
                if budget == 0 {
                    // ⚠ **Stops rather than skipping ahead.** The notes are in
                    // time order, so what is dropped is the tail — the producer
                    // hears the start of the file and is told it was cut, which
                    // is the same promise `clipped` already makes about a long
                    // one. Skipping notes to "fit" more of it would render an
                    // arrangement that is not in the file.
                    exhausted = true;
                    break 'notes;
                }
            }
        }
    }

    // ⛔⛔ **Normalised by what was actually mixed, not by the peak.** Peak
    // normalisation makes a one-note file as loud as a full arrangement, so
    // stepping down a folder of loops would swing the monitor level on every
    // click — the opposite of what an audition is for. A gentle divisor by voice
    // density keeps a dense file from clipping while leaving a sparse one quiet,
    // which is how the two actually differ.
    let headroom = 1.0 / (1.0 + (voices as f32 / 64.0)).sqrt();
    for sample in &mut samples {
        *sample = (*sample * headroom).clamp(-1.0, 1.0);
    }

    Audition {
        samples,
        rate: RENDER_RATE,
        // ⚠ Either bound reads the same way to a producer: what you are hearing
        // is not all of it.
        clipped: clipped || exhausted,
    }
}

/// Where a tick falls, in seconds.
///
/// ⚠ **The file's own tempo and its own ppq.** `smf_read` fills both from the
/// file rather than from the session — auditioning somebody else's loop at the
/// project's tempo would play it at a speed it was never written at, which is the
/// readout-that-lies failure in the one place the producer is listening rather
/// than reading.
fn seconds_at(pattern: &Pattern, tick: u32) -> f32 {
    let ppq = pattern.ppq.max(1) as f32;
    // ⚠ **`render::tempo`, not a second fallback of its own.** That one guards
    // `is_finite` as well as `> 1.0`, and a copy carrying only the second would
    // divide by an infinity a corrupt file declared — an audition of NaN.
    (tick as f32 / ppq) * (60.0 / crate::audio::render::tempo(pattern))
}

/// Add one note to the buffer, and answer how many frames it wrote.
///
/// ⚠ The count is what `render` spends against [`MAX_MIXED_FRAMES`] — it is the
/// real cost of this note rather than an estimate of it, so a file cannot be
/// arranged to be cheap to predict and expensive to run.
fn mix(into: &mut [f32], at: f32, held: f32, pitch: u8, vel: u8, melodic: bool) -> u64 {
    let start = (at * RENDER_RATE as f32) as usize;
    if start >= into.len() {
        return 0;
    }
    // The tail is the note plus its release, so nothing is cut mid-cycle.
    let sounding = ((held + RELEASE_SECONDS) * RENDER_RATE as f32) as usize;
    let last = (start + sounding).min(into.len());
    let gain = f32::from(vel.max(1)) / 127.0;

    // ⚠ **A `u32` seed advanced by hand rather than a random source.** The noise
    // for a drum lane has to be the *same* noise every time the file is
    // auditioned: a preview that sounds different on each press is one a producer
    // cannot compare two files with. Same reasoning `ipc-mock` gives about a
    // fixture that moves.
    let mut noise: u32 = 0x9E37_79B9 ^ u32::from(pitch);

    let step = std::f32::consts::TAU * midi_hz(pitch) / RENDER_RATE as f32;

    // ⛔ **The decay is stepped, not evaluated.** `(-3 * t).exp()` per frame is
    // one transcendental per sample per note, and the frame budget above allows
    // enough frames that it dominates the whole render — on the host's editor
    // thread. `e^(-3t)` is a geometric sequence at a fixed sample rate, so the
    // ratio is computed once here and multiplied in below.
    //
    // ⚠ **Not a numerical shortcut with a different answer**: this *is* the same
    // curve, sampled the same way, to f32 precision over at most two minutes.
    let decay_per_frame = (-3.0 / RENDER_RATE as f32).exp();
    let released_at = (-3.0 * held).exp();
    let held_frames = (held * RENDER_RATE as f32) as usize;
    let fade_frames = (RELEASE_SECONDS * RENDER_RATE as f32).max(1.0);
    let mut decay = 1.0f32;

    // ⚠ **Iterated rather than indexed.** The slice is bounded by `last` above,
    // so `into[frame]` was always in range — but clippy denies the pattern on
    // three platforms and `windows-only-reads-fail-ci-elsewhere` is this repo's
    // note about finding that out from CI rather than locally. `elapsed` is the
    // frame's offset from the note's own start, which is what the envelope and
    // the phase are both measured in.
    for (elapsed, sample) in into[start..last].iter_mut().enumerate() {
        // A percussive decay, which is what makes this legible rather than a
        // drone: the shape says "a note happened here" without a full ADSR.
        let envelope = if elapsed > held_frames {
            // Released — fade what is left rather than stopping.
            let fade = 1.0 - ((elapsed - held_frames) as f32 / fade_frames).min(1.0);
            released_at * fade
        } else {
            let now = decay;
            decay *= decay_per_frame;
            now
        };

        let value = if melodic {
            let phase = step * elapsed as f32;
            // Three harmonics, falling away — enough for a pitch to be
            // unambiguous, few enough that a chord does not turn to mud.
            (phase.sin() + 0.5 * (2.0 * phase).sin() + 0.25 * (3.0 * phase).sin()) / 1.75
        } else {
            // ⛔ **A drum lane's "pitch" is a GM drum number, not a pitch.**
            // Rendering 36 as a low C would make every kit sound like a bassline.
            // A noise burst says "a hit happened here", which is the whole
            // information a drum lane carries.
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (noise >> 8) as f32 / 8_388_608.0 - 1.0
        };

        *sample += value * envelope * gain * 0.35;
    }
    (last - start) as u64
}

/// The frequency of a MIDI note, equal-tempered from A4 = 440.
fn midi_hz(pitch: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(pitch) - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::Lane;
    use engine::smf_read::SplitPart;
    use serde_json::json;

    /// One note, as the wire shape.
    ///
    /// ⚠ **Built through serde rather than as a struct literal**, the way
    /// `drums_hats.rs` builds its models: `Pattern` and `Note` both carry
    /// optional fields that exist for the editor, and a literal here would have
    /// to be revisited every time one is added — for a renderer that reads four
    /// of them.
    fn note(start_tick: u32, len_ticks: u32, pitch: u8) -> serde_json::Value {
        json!({ "startTick": start_tick, "lenTicks": len_ticks, "pitch": pitch, "vel": 100 })
    }

    fn part(lane: Lane, notes: Vec<serde_json::Value>, bpm: f32) -> SplitPart {
        let lane = serde_json::to_value(lane).expect("a lane names itself");
        serde_json::from_value(json!({
            "part": "melody",
            "pattern": {
                "id": "t",
                "part": "melody",
                "artistId": "",
                "seed": 0,
                "songSeed": 0,
                "bars": 4,
                "bpm": bpm,
                "timeSigNum": 4,
                "timeSigDen": 4,
                "keyRoot": 0,
                "scale": "natural_minor",
                "ppq": 960,
                "lanes": [{ "lane": lane, "notes": notes }],
            },
            "reason": "highestVoice",
            "notes": 1,
        }))
        .expect("the fixture matches the wire shape")
    }

    #[test]
    fn the_buffer_is_as_long_as_the_file_says_it_is() {
        // ⚠ One note of one beat at 120 BPM is half a second, plus the release
        // that keeps it from clicking. Measured against the file's own tempo —
        // auditioning somebody else's loop at the project's tempo would play it
        // at a speed it was never written at.
        let rendered = render(&[part(Lane::Melody, vec![note(0, 960, 60)], 120.0)]);
        let seconds = rendered.samples.len() as f32 / rendered.rate as f32;
        assert!(
            (seconds - (0.5 + RELEASE_SECONDS)).abs() < 0.01,
            "half a second plus the release, got {seconds}"
        );
        assert!(!rendered.clipped);
    }

    #[test]
    fn a_file_that_asks_for_hours_is_cut_and_says_so() {
        // ⛔⛔ **A page-supplied out-of-memory, and `MAX_MIDI_BYTES` does not stop
        // it.** A few kilobytes of MIDI can declare 1 BPM and a note at bar 300 —
        // the *bytes* are bounded and the duration they describe is not.
        let rendered = render(&[part(Lane::Melody, vec![note(0, 960 * 4_000, 60)], 30.0)]);
        let seconds = rendered.samples.len() as f32 / rendered.rate as f32;
        assert!(
            seconds <= MAX_SECONDS + 1.0,
            "cut to the bound, got {seconds}"
        );
        assert!(
            rendered.clipped,
            "and it says so rather than cutting quietly"
        );
    }

    #[test]
    fn a_drum_lane_is_not_rendered_as_a_pitch() {
        // ⛔ A drum lane's "pitch" is a GM drum number. Rendered as a tone, 36
        // would be a low C and every kit in the browser would sound like a
        // bassline. The two paths must produce different audio for the same
        // number — that is the whole claim.
        let tone = render(&[part(Lane::Melody, vec![note(0, 960, 36)], 120.0)]);
        let hit = render(&[part(Lane::Kick, vec![note(0, 960, 36)], 120.0)]);
        assert_eq!(tone.samples.len(), hit.samples.len());
        assert!(
            tone.samples != hit.samples,
            "a drum lane and a melodic lane cannot render alike"
        );
    }

    #[test]
    fn the_same_file_auditions_identically_every_time() {
        // ⚠ The noise is seeded by hand rather than drawn from a random source:
        // a preview that sounds different on each press is one a producer cannot
        // compare two files with.
        let once = render(&[part(Lane::Kick, vec![note(0, 480, 36)], 120.0)]);
        let twice = render(&[part(Lane::Kick, vec![note(0, 480, 36)], 120.0)]);
        assert_eq!(once.samples, twice.samples);
    }

    #[test]
    fn nothing_leaves_the_buffer_clipping() {
        // ⚠ A dense file must not come back as a square wave. Density is handled
        // by the headroom divisor rather than by peak normalisation, which would
        // make a one-note file as loud as a full arrangement — so stepping down a
        // folder would swing the monitor level on every click.
        let notes: Vec<serde_json::Value> = (0..64).map(|i| note(0, 960, 40 + i)).collect();
        let rendered = render(&[part(Lane::Melody, notes, 120.0)]);
        assert!(rendered.samples.iter().all(|s| s.abs() <= 1.0));
        assert!(
            rendered.samples.iter().any(|s| s.abs() > 0.05),
            "and it is audible rather than silent"
        );
    }

    #[test]
    fn an_empty_file_is_silence_rather_than_a_refusal() {
        // ⚠ A legal `.mid` with no notes is a legal `.mid`. `Preview::load` reads
        // zero frames as "nothing to play", and the panel already says what a file
        // contains — an error here would report a fine file as broken.
        let rendered = render(&[]);
        assert!(rendered.samples.is_empty());
        assert!(!rendered.clipped);
    }
}
