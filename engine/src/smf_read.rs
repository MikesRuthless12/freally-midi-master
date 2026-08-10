//! Reading a Standard MIDI File back into a [`Pattern`] (TASK-040T).
//!
//! ⛔ **This exists so a producer's own MIDI can train a workflow.** Mike:
//! *"you should be able to drag in MIDI from the file explorer to train your
//! original artist/workflow."* The requirement that shapes the whole module is
//! the one beside it: **one measurement path, not two.** The fit reads a
//! `Pattern` and nothing else, so a file has to *become* a `Pattern` — a second
//! feature extractor for files would fit differently from the one for
//! generations, and a model trained from a folder would not match one trained
//! from the same music generated.
//!
//! ## What is recovered, and what is honestly not
//!
//! Notes, their timing rebased onto this engine's PPQ, velocities, the tempo and
//! the meter. **Not** the key or the scale: an SMF may carry a key signature and
//! most exported loops do not, and guessing one from the pitches would be a
//! measurement presented where there was none. The fit does not read either, so
//! the default costs nothing — but a caller that starts reading `scale` off an
//! imported pattern is reading an assumption.
//!
//! ⚠ **Drum lanes are recovered by General MIDI note**, the inverse of
//! [`crate::midi::gm_drum_note`]. A file whose drums were written against a
//! different map arrives on the wrong lanes, which is a real limitation and not
//! a bug we can fix from inside the file: GM is the only shared vocabulary there
//! is. A note on no known lane is dropped rather than guessed onto one.

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::collections::BTreeMap;

use crate::generators::drums::LANE_ORDER;
use crate::midi::gm_drum_note;
use crate::pattern::{
    ticks_per_bar_of, Lane, LaneTrack, Note, Part, Pattern, PatternRef, Scale, Section,
    SectionKind, Song, PPQ,
};

/// The longest file this will read, in notes.
///
/// ⛔ A bound rather than trust: the bytes come from a file a producer picked,
/// and a malformed or enormous one must not turn a training run into an
/// allocation the host cannot survive. Ten thousand notes is far past any loop
/// and far short of a problem.
const MAX_NOTES: usize = 10_000;

/// The GM percussion channel, counted from zero.
///
/// ⛔ **Channel 10 is drums by convention and the convention is near-universal.**
/// It is the one part assignment in a MIDI file that is *stated* rather than
/// inferred, which is why [`split`] uses it before it measures anything.
const DRUM_CHANNEL: u8 = 9;

/// What one read of a file yielded, before any part is decided.
///
/// ⚠ Its own type so [`smf_to_pattern`] and [`split`] share **one** parser. Two
/// readers over the same bytes is how a file imported one way comes to disagree
/// with the same file imported the other, and this module's own header already
/// argues that case for the fit.
struct Parsed {
    /// Every note, paired with the channel it arrived on.
    notes: Vec<(u8, Note)>,
    bpm: Option<f32>,
    time_sig_num: u8,
    time_sig_den: u8,
}

/// Read a type-0 or type-1 SMF as one pattern for `part`.
///
/// Every track is merged: a file exported per instrument still describes one
/// clip, and which track a note sat on says nothing about which lane it is.
pub fn smf_to_pattern(bytes: &[u8], part: Part, id: &str) -> Result<Pattern, String> {
    let mut parsed = read(bytes)?;
    // ⚠ Taken rather than borrowed: `assemble` needs the tempo and meter, and
    // `into_lanes` consumes the notes. The channel is dropped here, where that is
    // visible, rather than carried into a function that never read it.
    let notes: Vec<Note> = std::mem::take(&mut parsed.notes)
        .into_iter()
        .map(|(_, note)| note)
        .collect();
    let lanes = into_lanes(part, notes);
    if lanes.is_empty() {
        return Err("none of the notes in this file land on a lane this part can use".to_owned());
    }
    Ok(assemble(id, part, lanes, &parsed))
}

/// Everything one file yielded, read once.
fn read(bytes: &[u8]) -> Result<Parsed, String> {
    let smf = Smf::parse(bytes).map_err(|error| format!("not a MIDI file: {error}"))?;

    let file_ppq = match smf.header.timing {
        Timing::Metrical(ticks) => u32::from(ticks.as_int()).max(1),
        // ⛔ Refused rather than approximated. SMPTE timing is in frames per
        // second, so its ticks are wall-clock and this engine's are musical:
        // "rebasing" one onto the other needs a tempo the file may change
        // half-way through, and a silently wrong result is worse than a refusal
        // a producer can act on.
        Timing::Timecode(..) => {
            return Err("this file is timed in SMPTE frames rather than musical ticks".to_owned())
        }
    };

    // ⚠ **An `Option`, not a 120.0 sentinel.** The first cut compared against
    // 120.0 to mean "not set yet" — and 120 BPM is exactly 500,000 µs per
    // quarter, the single most common tempo any file declares. So a file whose
    // map went 120 → 140 read back as 140: the *last* tempo, which is the
    // outcome the rule below exists to avoid.
    let mut bpm: Option<f32> = None;
    let mut time_sig_num = 4_u8;
    let mut time_sig_den = 4_u8;
    let mut open: Vec<(u8, u32, u8, u8)> = Vec::new();
    let mut notes: Vec<(u8, Note)> = Vec::new();

    for track in &smf.tracks {
        let mut at: u64 = 0;
        for event in track {
            at += u64::from(event.delta.as_int());
            // Rebased as we go, so a long file cannot drift: integer maths on
            // the absolute tick rather than on each delta.
            let tick = rebase(at, file_ppq);

            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) => {
                    let us = us.as_int().max(1);
                    // ⚠ The **first** tempo wins. A file with a tempo map is
                    // describing something this pattern cannot hold — one clip,
                    // one tempo — and taking the last would silently describe
                    // the end of the file.
                    bpm.get_or_insert(60_000_000.0 / us as f32);
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(num, den_pow, ..)) => {
                    time_sig_num = num.max(1);
                    time_sig_den = 1u8.checked_shl(u32::from(den_pow)).unwrap_or(4).max(1);
                }
                TrackEventKind::Midi { channel, message } => match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        open.push((channel.as_int(), tick, key.as_int(), vel.as_int()));
                    }
                    // ⚠ A note-on with velocity zero **is** a note-off, and a
                    // great many exporters write them that way. Treating them as
                    // onsets would double every note in the file.
                    MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                        close(&mut open, &mut notes, channel.as_int(), key.as_int(), tick);
                    }
                    _ => {}
                },
                _ => {}
            }

            if notes.len() > MAX_NOTES {
                return Err(format!("this file has more than {MAX_NOTES} notes"));
            }
        }

        // A note left hanging at the end of a track is given the length it had
        // when the track stopped, rather than being dropped: an exporter that
        // omits the final note-off is common, and dropping the last note of
        // every phrase would bias every measurement taken from the file.
        // ⚠ `at` is already the sum of every delta in this track — the loop
        // above accumulated it. Re-walking the events to add them up again
        // doubled the traversal of every imported file for a number in hand.
        let end = rebase(at, file_ppq);
        while let Some((channel, start, key, vel)) = open.pop() {
            notes.push((
                channel,
                Note {
                    start_tick: start,
                    len_ticks: end.saturating_sub(start).max(1),
                    pitch: key,
                    vel: vel.max(1),
                    model_vel: None,
                    slide_to_pitch: None,
                    articulation: None,
                },
            ));
        }
    }

    if notes.is_empty() {
        return Err("this file carries no notes".to_owned());
    }

    notes.sort_by_key(|(_, note)| (note.start_tick, note.pitch));
    Ok(Parsed {
        notes,
        bpm,
        time_sig_num,
        time_sig_den,
    })
}

/// Wrap a set of lanes as a pattern, with the file's own tempo and meter.
fn assemble(id: &str, part: Part, lanes: Vec<LaneTrack>, parsed: &Parsed) -> Pattern {
    let last = lanes
        .iter()
        .flat_map(|track| track.notes.iter())
        // ⚠ Saturating: a file declaring `ppq: 1` with a huge delta can rebase
        // to a tick near `u32::MAX`, and a plain `+` would wrap to a small
        // number — a wrong bar count rather than a crash, but wrong silently.
        .map(|note| note.start_tick.saturating_add(note.len_ticks))
        .max()
        .unwrap_or(0);
    let per_bar = PPQ * u32::from(parsed.time_sig_num) * 4 / u32::from(parsed.time_sig_den).max(1);
    let bars = last
        .div_ceil(per_bar.max(1))
        .max(1)
        .min(u32::from(u16::MAX)) as u16;

    Pattern {
        id: id.to_owned(),
        part,
        artist_id: String::new(),
        seed: 0,
        song_seed: 0,
        bars,
        bpm: parsed.bpm.unwrap_or(120.0),
        time_sig_num: parsed.time_sig_num,
        time_sig_den: parsed.time_sig_den,
        // ⚠ Not recovered — see the module note. The fit reads neither.
        key_root: 0,
        scale: Scale::NaturalMinor,
        lanes,
        ppq: PPQ,
        mood: None,
        loop_region: None,
        clip_region: None,
    }
}

/// Why [`split`] put a voice on the part it did.
///
/// ⛔⛔ **The reason travels with the result, and that is a requirement rather
/// than a nicety.** TASK-058D states it for the audio path and it applies here:
/// *"the UI names what it detected … and never presents a guess as a
/// transcription"*, so *"a wrong guess is one click to redirect rather than a
/// silent mis-file."* Two of these reasons are facts about the file and three are
/// measurements — a producer is owed the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub enum SplitReason {
    /// It arrived on GM channel 10. **Stated by the file, not inferred.**
    DrumChannel,
    /// It plays like a kit — a handful of GM drum pitches, struck often and
    /// short — whatever channel it came in on.
    ///
    /// ⛔ **Distinct from [`Self::DrumChannel`] on purpose.** That variant's own
    /// doc promises the file *stated* it, and this one is a measurement; emitting
    /// `DrumChannel` for an inferred kit made the one reason this module presents
    /// as a fact into a guess. Real sample packs export on channel 0, so this is
    /// the common case and `DrumChannel` is the rare one.
    KitShape,
    /// Notes overlap enough that the voice is chordal rather than a line.
    Polyphonic,
    /// The lowest line in the file.
    LowestVoice,
    /// The highest line in the file.
    HighestVoice,
    /// A line that is neither the lowest nor the highest.
    InnerVoice,
    /// ⚠ **The file had one voice and it was cut by pitch.** The weakest of
    /// these by a distance, and the page must say so.
    SplitByPitch,
    /// ⛔ **The producer's own filename said so** — `…-Bass.mid`, `…Drums.mid`.
    ///
    /// Stated intent rather than inference, and the strongest signal available
    /// for a single-voice file. See [`part_from_name`] for why it outranks every
    /// measurement except the drum-kit test.
    FromName,
}

/// The part a producer's own filename names, if it names one.
///
/// ⛔⛔ **This exists because measuring a single-voice file is a coin flip, and
/// no amount of threshold-tuning fixes that.** Mike, 2026-08-10, on the rule
/// fitted to his static 808: *"in trap beats the 808 bass notes can go up and
/// down though."* He is right — a trap 808 follows the chord roots, so "few
/// pitches, held long" describes one 808 and not the next. Widening the
/// threshold until it covers moving 808s would swallow every slow lead instead.
///
/// ▶ **The way out is not a better measurement.** Producers name their files.
/// `Starlight-Bass.mid`, `Starlight-Drums.mid`, `Starlight-Hook.mid` — that is
/// the producer saying what the file is, and it beats anything inferable from
/// five notes. It is the same *class* of evidence as GM channel 10: stated, not
/// guessed. Unlike channel 10, real files actually carry it.
///
/// ⚠ **Whole words only.** A substring test would read `Starlight` as containing
/// "star" and, worse, match `Bassoon` or a track called `Embassy`. Split on the
/// separators producers actually use and compare whole tokens.
///
/// ⚠ **A hint, not an override.** [`split`] still runs the drum-kit measurement
/// first: a file named `Bass` that is unmistakably 192 short hits on GM drum
/// pitches is a drum loop that was named badly, and the measurement there is
/// near-certain. Everywhere else the name wins.
pub fn part_from_name(name: &str) -> Option<Part> {
    for token in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        let token = token.to_ascii_lowercase();
        let part = match token.as_str() {
            "bass" | "808" | "sub" | "bassline" => Part::Bass,
            "drum" | "drums" | "perc" | "percussion" | "beat" => Part::Drums,
            "melody" | "lead" | "hook" | "topline" | "arp" | "pluck" => Part::Melody,
            "chord" | "chords" | "keys" | "pad" | "pads" | "harmony" => Part::Chords,
            "counter" | "countermelody" | "harm" => Part::Counter,
            _ => continue,
        };
        return Some(part);
    }
    None
}

/// One part a file was separated into.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-types.ts")]
pub struct SplitPart {
    pub part: Part,
    pub pattern: Pattern,
    pub reason: SplitReason,
    /// How many notes landed here, so the page can show it without walking lanes.
    pub notes: u32,
}

/// Where a single-voice file is cut when there is nothing else to go on.
///
/// ⚠ **C3.** Below it is where a bassline lives in almost every file a producer
/// will drop in; above it is the line they hear as the melody. It is a heuristic
/// and [`SplitReason::SplitByPitch`] is what makes it one the producer can see.
const PITCH_SPLIT: u8 = 60;

/// How far apart the two halves must sit before a one-voice file is cut at all.
///
/// ⛔⛔ **Found on Mike's own fixture, and the cut was wrong without it.**
/// `TrapQuake.mid` is a single line running from pitch 54 to 62 — it crosses C3
/// because melodies do, not because there is a bassline under it. The bare
/// [`PITCH_SPLIT`] rule cut it into a ten-note "bass" and a two-note "melody",
/// which is exactly the silent mis-file this module's own doc says never to
/// produce.
///
/// ▶ **A real bass-and-melody file has a gap**, because the two instruments are
/// playing in different registers: a bass around 28–45 under a lead at 60–80
/// leaves fifteen semitones of daylight. A single line wandering across middle C
/// leaves three. An octave is the smallest gap that means "two instruments"
/// rather than "one line with a wide range".
const SPLIT_GAP: u8 = 12;

/// ...and how much of the file each half must hold to count as a part.
///
/// ⚠ Two notes out of twelve is an ornament at the top of a phrase, not a
/// second instrument. Guards the case where the gap is wide but one side is a
/// single grace note.
const SPLIT_SHARE: f32 = 0.15;

/// How many notes must start *together* before a voice is chordal.
///
/// ⛔⛔ **Overlap was the wrong measurement, and a real file proved it.** This
/// used to ask what fraction of notes *sounded against* the note before them —
/// and `Starlight-Hook.mid`, a sung hook line, read as chords, because held
/// melodic notes overlap constantly. Legato is not harmony.
///
/// ▶ **Notes struck on the same tick is the signal.** Measured on Mike's
/// fixtures: the hook shares an onset 4 times in 20 notes (21%); the Anthem
/// keys/pad file — genuinely chordal — shares one 88 times in 144 (61%). A third
/// separates them cleanly and is nowhere near either.
const CHORD_ONSETS: f32 = 0.34;

/// How many distinct pitches a voice may use and still be a drum kit.
///
/// ⚠ A kit is a small vocabulary struck many times. Measured on Mike's fixtures:
/// `Starlight-Drums` uses **3**, `Starlight-Melody` 9, `Starlight-Hook` 12.
/// Eight sits clear of all three.
const KIT_MAX_PITCHES: usize = 8;

/// ...and at least this many, because one repeated note is not a kit.
///
/// ⛔⛔ **A trap 808 hammering one root would otherwise read as a drum machine.**
/// C1 and D1 are ordinary bass notes *and* the GM kick and snare, so a two-note
/// 808 line matches the GM test perfectly. A real kit is at least a kick and
/// something else; a single pitch struck repeatedly is a bass part or one
/// percussion element, and neither is a kit.
const KIT_MIN_PITCHES: usize = 2;

/// The longest a kit's notes may typically be, as a fraction of a quarter.
///
/// ⛔ **The strongest signal of the four, and it is what separates drums from a
/// sustained 808 that shares their pitches.** Measured: `Starlight-Drums` has a
/// median length of 240 ticks — a 16th at PPQ 960 — and a *maximum* of 250.
/// `Starlight-Melody`'s median is 920 and `Starlight-Hook`'s 1250. An eighth note
/// (480) sits between the drum maximum and the melodic median with room either
/// side. Percussion is short because it is struck; a bass note is held.
const KIT_MAX_MEDIAN_LEN: u32 = PPQ / 2;

/// ...and how many times, on average, each of them must be struck.
///
/// ⚠ 192 notes over 3 pitches is 64. A melody in the same register runs closer
/// to 1.5. Four is far above one and far below the other.
const KIT_MIN_REPEATS: f32 = 4.0;

/// What share of a voice's notes must land on a GM drum note to be a kit.
///
/// ⛔ **Not "any of them".** GM drum notes are just MIDI pitches 35–81, so any
/// melody in that register hits some by coincidence — `Starlight-Melody.mid`
/// "matches" Clap, Snare, OpenHat and GhostSnare while being a melody. Nearly
/// all of them, or it is not a kit.
const KIT_MIN_MATCH: f32 = 0.9;

/// Below this a lone pitched line may be read as a bassline rather than a melody.
///
/// ⛔ **C2, not C3.** Defaulting a lone voice to Bass because its median dips
/// below middle C mislabels most melodies that have any low notes in them.
const LONE_BASS_CEILING: u8 = 48;

/// How few pitches a held line may use before it is a root part rather than a
/// melody, **whatever register it is written in**.
///
/// ⛔⛔ **808 MIDI is written at TRIGGER pitch, not SOUNDING pitch, and that
/// breaks register-based bass detection outright.** Mike's `Starlight-Bass.mid`
/// is five notes, all on pitch **61** — C♯4, squarely melody register — each a
/// full quarter long. The 808 *sample* is already low; the MIDI note only says
/// which key fires it, and the sampler transposes it down octaves. Every rule
/// above that reasons from register is blind to this, and filed it as a melody.
///
/// ▶ **So the rule is musical role, not pitch.** A bass part holds the root: a
/// handful of pitches, held long. A melody moves — `Starlight-Melody` uses nine
/// pitches, `Starlight-Hook` twelve. Two is comfortably below either.
///
/// ⚠ **Fitted against a single example**, and worth saying so: this is one 808
/// file. The *argument* is general — it describes what a bass part does rather
/// than what one file measured — but the threshold has not been tested against a
/// walking bassline or a melodic 808, both of which move more.
const ROOT_PART_MAX_PITCHES: usize = 2;

/// ...and how long its notes must be held to count as one.
///
/// ⚠ A quarter note. Measured: the 808's median is exactly `PPQ`, its minimum
/// 710. A stab or a plucked one-note ostinato is far shorter and stays a melody.
const ROOT_PART_MIN_LEN: u32 = PPQ * 3 / 4;

/// ...and how far it may roam and still be one.
///
/// ⛔⛔ **Register alone was not enough, and a real file settled it.**
/// `Starlight-Melody.mid` has a median pitch of **39** with fourteen of its
/// twenty-one notes below C3 — by register it reads as a bassline, and the first
/// cut filed it as one. But it spans 33..74: **three and a half octaves.**
///
/// ▶ **A bassline does not roam.** It sits in its register and works rhythmically
/// — two octaves is already a wide one. A line that crosses three is a melody
/// that happens to start low, which is most dark trap leads. So a lone voice is
/// a bass only when it is *both* low and narrow.
const LONE_BASS_RANGE: u8 = 24;

/// Separate a layered file into the generators its voices belong to.
///
/// ⛔ Mike, 2026-08-10: *"split it into the proper generators if it is a layered
/// melody file with the bass and countermelody included."*
///
/// ## The order is the honesty
///
/// 1. **Channel 10 → Drums.** The one assignment the file *states*.
/// 2. **Chordal voices → Chords**, measured by how much they overlap themselves.
/// 3. **The rest, by register**: lowest → Bass, highest → Melody, anything
///    between → Counter.
/// 4. **Only if there is one voice left and nothing above it applied**, cut it at
///    [`PITCH_SPLIT`] into Bass and Melody — and say that is what happened.
///
/// ⚠ **This is not transcription and does not claim to be.** It separates voices
/// a file already keeps apart, and falls back to one measurable rule when it does
/// not. Build Philosophy § 7 bans the ML that would do better, which is a
/// constraint on the result rather than an excuse for it — so every part carries
/// its [`SplitReason`].
pub fn split(bytes: &[u8], id: &str) -> Result<Vec<SplitPart>, String> {
    split_parsed(&read(bytes)?, id)
}

/// The same rules, over notes already read.
///
/// ⚠ **Its own function so [`smf_to_song`] can classify each section with the
/// identical decisions.** A second classifier for sections is exactly the drift
/// this module's header warns about — one measurement path, not two.
fn split_parsed(parsed: &Parsed, id: &str) -> Result<Vec<SplitPart>, String> {
    // ⚠ Grouped by channel, which is what "a voice" means in a MIDI file. A
    // type-1 file exported per instrument puts each on its own channel; a type-0
    // merged one may put everything on channel 1, which is the case step 4 is
    // for.
    let mut voices: Vec<(u8, Vec<Note>)> = Vec::new();
    for (channel, note) in &parsed.notes {
        match voices.iter_mut().find(|(held, _)| held == channel) {
            Some((_, notes)) => notes.push(note.clone()),
            None => voices.push((*channel, vec![note.clone()])),
        }
    }

    let mut out: Vec<SplitPart> = Vec::new();

    // 1. Drums, by the channel the file names.
    if let Some(at) = voices
        .iter()
        .position(|(channel, _)| *channel == DRUM_CHANNEL)
    {
        let (_, notes) = voices.remove(at);
        // ⚠ `None` when nothing on the channel matched a GM lane — dropped rather
        // than emitted as a drum part with no drums in it.
        if let Some(part) = drums(id, &notes, parsed, SplitReason::DrumChannel) {
            out.push(part);
        }
    }

    // 2. Drums by their *shape*, on whatever channel they arrived on.
    //
    // ⛔⛔ **This is what makes real sample-pack MIDI work.** See
    // [`looks_like_a_kit`]: Mike's `Starlight-Drums.mid` is 192 notes of kick,
    // closed hat and snare **on channel 0**, and the channel rule above never
    // sees it. Sample-pack MIDI comes out of a DAW's piano roll, which does not
    // care which channel General MIDI reserved.
    let mut rest: Vec<(u8, Vec<Note>)> = Vec::new();
    for (channel, notes) in voices {
        if looks_like_a_kit(&notes) {
            // ⚠ **`KitShape`, not `DrumChannel`** — this is a measurement and
            // that variant's doc promises a stated fact. See `SplitReason`.
            if let Some(part) = drums(id, &notes, parsed, SplitReason::KitShape) {
                out.push(part);
                continue;
            }
            // ⚠ Nothing mapped after all — fall through and treat it as pitched
            // rather than dropping the voice on the floor.
        }
        rest.push((channel, notes));
    }

    // 3. Chordal voices, measured on notes struck *together*.
    let mut lines: Vec<(u8, Vec<Note>)> = Vec::new();
    for (channel, notes) in rest {
        if shared_onset_ratio(&notes) >= CHORD_ONSETS {
            out.push(pitched(
                id,
                Part::Chords,
                notes,
                parsed,
                SplitReason::Polyphonic,
            ));
        } else {
            lines.push((channel, notes));
        }
    }

    // 4. One line, and the producer's filename says what it is.
    //
    // ⛔⛔ **Before any register rule**, because measuring a single voice is a
    // coin flip and the name is not — see [`part_from_name`]. This is what makes
    // `Starlight-Bass.mid` land on Bass whether its 808 sits still or walks the
    // chords, which is the case Mike raised and which no threshold covers.
    if lines.len() == 1 {
        if let Some(part) = part_from_name(id) {
            let (_, notes) = lines.remove(0);
            out.push(match part {
                // ⚠ A file *named* drums whose notes are not on GM drum pitches
                // cannot become a drum pattern — `into_lanes` would drop every
                // note. Named or not, the pitches have to mean something.
                Part::Drums => match drums(id, &notes, parsed, SplitReason::FromName) {
                    Some(part) => part,
                    // A file *named* drums whose notes are on no GM pitch cannot
                    // become a drum pattern — named or not, the pitches have to
                    // mean something.
                    None => pitched(id, Part::Melody, notes, parsed, SplitReason::SplitByPitch),
                },
                other => pitched(id, other, notes, parsed, SplitReason::FromName),
            });
            return Ok(merge_by_part(out, id, parsed));
        }
    }

    // 5. One line and nothing else decided: cut it by register.
    //
    // ⚠ Checked before step 6 because it *replaces* it — with a single line
    // there is no "lowest and highest" to compare, and calling that one voice
    // the melody would silently drop a bassline written underneath it.
    //
    // ⛔ **Not conditional on `out` being empty.** It was, and a file of drums
    // plus one bassline therefore skipped every lone-line rule and fell through
    // to "the highest of one voice is the melody" — so a drums-and-808 file
    // imported its 808 as a lead. Whether drums were also found says nothing
    // about what the one pitched line is.
    if lines.len() == 1 {
        let (_, notes) = lines.remove(0);
        let total = notes.len();
        let (low, high): (Vec<Note>, Vec<Note>) =
            notes.into_iter().partition(|note| note.pitch < PITCH_SPLIT);

        // ⛔⛔ **Only when the two halves are genuinely two instruments.** See
        // [`SPLIT_GAP`] — without this a melody that crosses middle C is filed
        // as a bassline plus a two-note lead, which is what Mike's `TrapQuake`
        // fixture did on the first cut.
        let gap = match (
            low.iter().map(|note| note.pitch).max(),
            high.iter().map(|note| note.pitch).min(),
        ) {
            (Some(top), Some(bottom)) => bottom.saturating_sub(top),
            _ => 0,
        };
        let share = |half: &[Note]| half.len() as f32 / total.max(1) as f32;
        let two_instruments =
            gap >= SPLIT_GAP && share(&low) >= SPLIT_SHARE && share(&high) >= SPLIT_SHARE;

        if two_instruments {
            out.push(pitched(
                id,
                Part::Bass,
                low,
                parsed,
                SplitReason::SplitByPitch,
            ));
            out.push(pitched(
                id,
                Part::Melody,
                high,
                parsed,
                SplitReason::SplitByPitch,
            ));
            return Ok(merge_by_part(out, id, parsed));
        }

        // ⚠ **One line after all**, so it stays one part and the register only
        // decides *which* — a melody is not improved by being cut in half.
        //
        // ⛔ **And the default is Melody, not Bass.** This compared the median
        // against [`PITCH_SPLIT`] (C3), which files a great many melodies as
        // basslines — `Starlight-Melody.mid` runs 33..74 with a median under
        // middle C and is plainly the melody. A lone line is a bassline only
        // when it is genuinely low; [`LONE_BASS_CEILING`] is where that starts.
        let mut notes = low;
        notes.extend(high);
        notes.sort_by_key(|note| (note.start_tick, note.pitch));

        // ⛔ **Low AND narrow** — see [`LONE_BASS_RANGE`]. Low alone filed
        // `Starlight-Melody.mid` (median 39, but spanning three and a half
        // octaves) as a bassline.
        let span = match (
            notes.iter().map(|note| note.pitch).min(),
            notes.iter().map(|note| note.pitch).max(),
        ) {
            (Some(low), Some(high)) => high.saturating_sub(low),
            _ => 0,
        };
        let part = if holds_a_root(&notes) {
            // ⛔ Checked BEFORE register — see `ROOT_PART_MAX_PITCHES`. An 808
            // written at trigger pitch sits in melody register and is still the
            // bassline, so register must not get to answer first.
            Part::Bass
        } else if median_pitch(&notes) < LONE_BASS_CEILING && span <= LONE_BASS_RANGE {
            Part::Bass
        } else {
            Part::Melody
        };
        // ⚠ `SplitByPitch` still, because register is still what decided it —
        // and the page has to keep saying so.
        out.push(pitched(id, part, notes, parsed, SplitReason::SplitByPitch));
        return Ok(merge_by_part(out, id, parsed));
    }

    // 3. By register: lowest is the bass, highest is the melody, the rest are
    // counter-melodies.
    // ⚠ `sort_by_cached_key`, not `sort_by_key` — the rule `explorer::list` already
    // records: the latter evaluates the key inside the comparator, and
    // `median_pitch` allocates and sorts the whole voice each time.
    lines.sort_by_cached_key(|(_, notes)| median_pitch(notes));
    let last = lines.len().saturating_sub(1);
    for (at, (_, notes)) in lines.into_iter().enumerate() {
        let (part, reason) = if at == 0 && last > 0 {
            (Part::Bass, SplitReason::LowestVoice)
        } else if at == last {
            (Part::Melody, SplitReason::HighestVoice)
        } else {
            (Part::Counter, SplitReason::InnerVoice)
        };
        out.push(pitched(id, part, notes, parsed, reason));
    }

    if out.is_empty() {
        return Err("none of the notes in this file land on a part".to_owned());
    }
    Ok(merge_by_part(out, id, parsed))
}

/// Fold voices that landed on the same part into one clip each.
///
/// ⛔⛔ **Called on EVERY exit from [`split_parsed`], not just the last one.**
/// It was applied only on the final fall-through, so a file that also contained
/// exactly one pitched line returned early and skipped it — and a post-condition
/// that holds on one of four paths is not a post-condition. Found by a code
/// review, 2026-08-10; the test that was meant to pin it used two chord voices
/// and no line, so it took the fall-through and never exercised the gap.
///
/// ⛔⛔ **Because the result is one clip PER PART, and nothing enforced it.**
/// Steps above decide per *voice*: a file with piano and pad on two channels
/// yields two Chords entries, and a kick track plus a hat track yields two Drums.
/// Every consumer then dropped one, differently and silently —
/// `smf_to_song` and `session.importSplit` both write into a map keyed by part
/// (last wins), and `MidiPreview` keys its rows on the part, so React saw
/// duplicate keys. The panel would report "Chords — 88 notes" and the import
/// would land the *other* chord voice.
///
/// ⚠ **Merged, not discarded.** Two chord voices are both the chords; keeping
/// only one loses half the harmony. The notes are concatenated and re-laned once.
///
/// ⚠ **The first voice's reason wins**, because the steps run in confidence
/// order — a stated fact (`DrumChannel`, `FromName`) is decided before any
/// measurement, so the earliest reason is the strongest one that applied.
fn merge_by_part(parts: Vec<SplitPart>, id: &str, parsed: &Parsed) -> Vec<SplitPart> {
    let mut out: Vec<SplitPart> = Vec::new();
    for part in parts {
        let Some(at) = out.iter().position(|held| held.part == part.part) else {
            out.push(part);
            continue;
        };
        let mut notes: Vec<Note> = out[at]
            .pattern
            .lanes
            .iter()
            .chain(part.pattern.lanes.iter())
            .flat_map(|lane| lane.notes.iter().cloned())
            .collect();
        notes.sort_by_key(|note| (note.start_tick, note.pitch));
        let reason = out[at].reason;
        let merged = pitched_or_drums(id, out[at].part, notes, parsed, reason);
        out[at] = merged;
    }
    out
}

/// One part from notes, taking the drum road when the part is drums.
fn pitched_or_drums(
    id: &str,
    part: Part,
    notes: Vec<Note>,
    parsed: &Parsed,
    reason: SplitReason,
) -> SplitPart {
    if part == Part::Drums {
        if let Some(built) = drums(id, &notes, parsed, reason) {
            return built;
        }
    }
    pitched(id, part, notes, parsed, reason)
}

/// The block lengths a file's own loop might be, in bars, longest first.
///
/// ⚠ **Longest first, and that is the point.** Mike, 2026-08-10: *"some trap
/// drums vary every 4 bars for a verse."* If a file runs A-A-A-B, four bars
/// repeat three times out of four and eight repeat exactly — taking the shortest
/// period that "mostly" matches would return the four-bar loop and silently drop
/// the fill. Checking long to short returns the block that contains the
/// variation.
const SECTION_BARS: &[u16] = &[16, 8, 4, 2, 1];

/// How alike two blocks must be to count as the same section.
///
/// ⚠ Not identity: a producer's verse repeats with a different last hat or a
/// dropped note, and demanding byte-equality would make every bar its own
/// section. Nine tenths of the onsets shared is a repeat.
const SECTION_MATCH: f32 = 0.9;

/// Read a whole song into an arrangement (TASK-058D).
///
/// ⛔⛔ **Mike's design, 2026-08-10:** *"could you put the midi for the entire
/// song into the 'Song' tab and allow them to pick which parts they want for the
/// generators, just like you would for a generation, but using someone elses song
/// as the starting point?"*
///
/// It is a better shape than importing straight into a generator, for three
/// reasons that are all properties of the data rather than of the UI:
///
/// 1. **A song is not a clip.** It has sections; a generator holds one loop.
///    Dropping thirty bars into a four-bar editor loses everything after bar
///    four, or keeps thirty and is unusable.
/// 2. **It answers "which bars do I take?" by not asking.** The producer sees the
///    arrangement and picks, and drums that change every four bars are two
///    sections they can both reach.
/// 3. **It is not destructive.** Nothing overwrites a generator until they drill
///    into a cell — which [`crate::pattern::Song`] and the page's existing
///    `drillInto` already do.
///
/// ⚠ **This is the drag-in path only.** Generating a song is untouched — Mike
/// was explicit that the generated route *"is pretty much self-explanatory"* and
/// must not change.
///
/// ⚠ **An imported song has no artist and no seed**, and says so: `artist_id` is
/// empty and `seed` is 0, exactly as [`smf_to_pattern`] already leaves them. A
/// number invented here would be a seed that reproduces nothing.
pub fn smf_to_song(bytes: &[u8], id: &str) -> Result<Song, String> {
    let parsed = read(bytes)?;
    let per_bar = ticks_per_bar_of(parsed.time_sig_num, parsed.time_sig_den);

    let end = parsed
        .notes
        .iter()
        .map(|(_, note)| note.start_tick.saturating_add(note.len_ticks))
        .max()
        .unwrap_or(0);
    // ⛔⛔ **Clamped, like `assemble` already does.** Without the `min` this cast
    // truncates: a file at one tick per quarter with a 255/1 time signature
    // rebases to ticks near `u32::MAX`, and the wrapped bar count then drove the
    // block arithmetic below past the end of the tick space. Found by a security
    // review, reproduced in both profiles — and `panic = "abort"` is release-only,
    // so what merely panics in debug **kills the host process** in a shipped
    // build, taking the producer's unsaved session with it.
    let total_bars = end.div_ceil(per_bar.max(1)).max(1).min(u32::from(u16::MAX)) as u16;

    let block = section_length(&parsed.notes, per_bar, total_bars);

    let mut sections: Vec<Section> = Vec::new();
    let mut patterns: BTreeMap<String, Pattern> = BTreeMap::new();

    let mut bar = 0_u16;
    while bar < total_bars {
        let bars = block.min(total_bars - bar);
        // ⚠ Saturating throughout — see the clamp on `total_bars`. A window that
        // runs off the end of the tick space is an empty window, not an overflow.
        let from = u32::from(bar).saturating_mul(per_bar);
        let to = from.saturating_add(u32::from(bars).saturating_mul(per_bar));

        // ⚠ **Rebased to the section's own start.** A clip is played from tick
        // zero wherever its section sits, so a pattern carrying absolute ticks
        // would put the whole verse after the end of its own four bars.
        let notes: Vec<(u8, Note)> = parsed
            .notes
            .iter()
            .filter(|(_, note)| note.start_tick >= from && note.start_tick < to)
            .map(|(channel, note)| {
                let mut moved = note.clone();
                moved.start_tick -= from;
                (*channel, moved)
            })
            .collect();

        if !notes.is_empty() {
            let slice = Parsed {
                notes,
                bpm: parsed.bpm,
                time_sig_num: parsed.time_sig_num,
                time_sig_den: parsed.time_sig_den,
            };
            let mut refs: BTreeMap<Part, PatternRef> = BTreeMap::new();
            // ⚠ A section that classifies to nothing is skipped rather than
            // fatal: one odd bar of a long file must not cost the whole import.
            for part in split_parsed(&slice, id).unwrap_or_default() {
                // ⛔ **Deduped by content**, which is what `Song::patterns` is
                // shaped for: *"verse 1 and verse 2 are the same beat, and in
                // these genres that is the rule rather than an optimisation."*
                let key = pattern_key(&part.pattern);
                patterns.entry(key.clone()).or_insert_with(|| {
                    let mut stored = part.pattern.clone();
                    stored.id = key.clone();
                    stored
                });
                refs.insert(
                    part.part,
                    PatternRef {
                        pattern_id: key,
                        bars: None,
                    },
                );
            }
            if !refs.is_empty() {
                sections.push(Section {
                    // ⚠ **Every section is a Verse**, and that is honesty rather
                    // than laziness: nothing in an SMF says which eight bars were
                    // the hook. Labelling them by guesswork would put a name on
                    // the timeline the file never carried.
                    kind: SectionKind::Verse,
                    start_bar: u32::from(bar),
                    bars,
                    patterns: refs,
                    drop_out_beats: 0,
                    decay: false,
                    markers: Vec::new(),
                });
            }
        }
        bar += bars;
    }

    if sections.is_empty() {
        return Err("this file carries no notes".to_owned());
    }

    Ok(Song {
        id: id.to_owned(),
        artist_id: String::new(),
        seed: 0,
        bpm: parsed.bpm.unwrap_or(120.0),
        key_root: 0,
        scale: Scale::NaturalMinor,
        sections,
        time_sig_num: parsed.time_sig_num,
        time_sig_den: parsed.time_sig_den,
        patterns,
        ppq: PPQ,
    })
}

/// Identity for a pattern, so two identical sections share one entry.
///
/// ⚠ Built from the notes rather than from a counter: two verses are the same
/// beat *because their notes match*, and a counter would store the same clip
/// twice and lose the sharing the type exists for.
fn pattern_key(pattern: &Pattern) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |value: u64| {
        hash ^= value;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    };
    eat(pattern.part as u64);
    for lane in &pattern.lanes {
        eat(lane.lane as u64);
        for note in &lane.notes {
            eat(u64::from(note.start_tick));
            eat(u64::from(note.len_ticks));
            eat(u64::from(note.pitch));
            eat(u64::from(note.vel));
        }
    }
    format!("{:?}-{hash:016x}", pattern.part).to_lowercase()
}

/// How many bars one section of this file is.
///
/// ⛔ **Measured, not assumed.** Compares the onset pattern against itself at
/// each candidate length in [`SECTION_BARS`] and takes the longest that repeats —
/// see that constant for why longest rather than shortest.
fn section_length(notes: &[(u8, Note)], per_bar: u32, total_bars: u16) -> u16 {
    for candidate in SECTION_BARS.iter().copied() {
        if candidate >= total_bars {
            continue;
        }
        let span = u32::from(candidate) * per_bar;
        // ⚠ `candidate < total_bars` above, so this is always ≥ 2.
        let blocks = total_bars.div_ceil(candidate);

        // Onsets of one block, as (tick within block, pitch).
        //
        // ⛔ **Binary-searched, not filtered.** `read` returns the notes sorted
        // by `(start_tick, pitch)`, so a block is a contiguous slice — and this
        // is called once per block per candidate period, 233 times on a 120-bar
        // file. Scanning all ten thousand notes each time is 2.3 million visits
        // where two `partition_point`s are about three thousand.
        //
        // ⚠ **And no re-sort.** Subtracting a constant from an ascending
        // `start_tick` preserves order, so the `sort_unstable` that used to be
        // here was 233 sorts of an already-sorted slice.
        let signature = |index: u16| -> Vec<(u32, u8)> {
            let from = u32::from(index) * span;
            // ⛔ **Saturating, and `max(start)`.** `from + span` overflowed `u32`
            // on a file whose ticks reach the top of the space — in release it
            // wrapped to a small number, so `end` came back *below* `start` and
            // the slice below aborted the process. Both halves are needed: the
            // saturate stops the wrap, the `max` stops an inverted range if the
            // two partition points ever disagree.
            let until = from.saturating_add(span);
            let start = notes.partition_point(|(_, note)| note.start_tick < from);
            let end = notes
                .partition_point(|(_, note)| note.start_tick < until)
                .max(start);
            notes[start..end]
                .iter()
                .map(|(_, note)| (note.start_tick - from, note.pitch))
                .collect()
        };

        let first = signature(0);
        if first.is_empty() {
            continue;
        }
        let matched = (1..blocks)
            .map(signature)
            .filter(|other| similarity(&first, other) >= SECTION_MATCH)
            .count();
        // ⚠ Half the blocks, not all: a song's hook differs from its verse and
        // the verse still sets the section length.
        if matched * 2 >= usize::from(blocks - 1) {
            return candidate;
        }
    }
    // ⚠ Nothing repeated — the whole file is one section, which is the honest
    // reading of a file with no structure to find.
    total_bars
}

/// What share of `a`'s onsets `b` also has.
///
/// ⛔ **A merge, not a `contains` per element.** Both sides come out of
/// [`signature`] already sorted, and the naive form is O(|a|·|b|) — on a
/// 120-bar file at the note cap that is roughly **23 million tuple comparisons**
/// across the five candidate periods. Walking two sorted slices is O(|a|+|b|),
/// about thirty thousand.
fn similarity(a: &[(u32, u8)], b: &[(u32, u8)]) -> f32 {
    if a.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0_usize, 0_usize, 0_usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    shared as f32 / a.len() as f32
}

/// A drum part from a set of notes, or `None` if none of them name a GM lane.
///
/// ⚠ **One place, three callers**, and the reason is a parameter rather than a
/// constant: the same four steps were written out at each of the three sites that
/// build drums, and they had already diverged — the shape-detected one reported
/// `DrumChannel`, whose own doc promises the file *stated* it.
fn drums(id: &str, notes: &[Note], parsed: &Parsed, reason: SplitReason) -> Option<SplitPart> {
    let lanes = into_lanes(Part::Drums, notes.to_vec());
    if lanes.is_empty() {
        return None;
    }
    Some(finish(id, Part::Drums, lanes, parsed, reason))
}

/// One pitched part from a set of notes.
fn pitched(
    id: &str,
    part: Part,
    notes: Vec<Note>,
    parsed: &Parsed,
    reason: SplitReason,
) -> SplitPart {
    finish(id, part, into_lanes(part, notes), parsed, reason)
}

fn finish(
    id: &str,
    part: Part,
    lanes: Vec<LaneTrack>,
    parsed: &Parsed,
    reason: SplitReason,
) -> SplitPart {
    let notes = lanes.iter().map(|lane| lane.notes.len() as u32).sum();
    SplitPart {
        part,
        pattern: assemble(id, part, lanes, parsed),
        reason,
        notes,
    }
}

/// The middle pitch of a voice, which is what puts it above or below another.
///
/// ⚠ **Median rather than mean.** One octave-jump ornament at the top of a
/// bassline drags a mean far enough to make it read as the melody; the median
/// describes where the voice actually sits.
fn median_pitch(notes: &[Note]) -> u8 {
    if notes.is_empty() {
        return 0;
    }
    let mut pitches: Vec<u8> = notes.iter().map(|note| note.pitch).collect();
    pitches.sort_unstable();
    pitches[pitches.len() / 2]
}

/// What share of a voice's notes are struck at the same instant as another.
///
/// ⛔ **Onsets, not overlap** — see [`CHORD_ONSETS`]. Two notes sounding at once
/// because the first is held is legato; two notes *starting* together is a
/// chord. The first cut measured the former and filed a sung hook as chords.
///
/// ⚠ Sorted first so one pass is enough; a full pairwise sweep is quadratic and
/// a comped part with a few thousand notes is an ordinary file.
fn shared_onset_ratio(notes: &[Note]) -> f32 {
    if notes.len() < 2 {
        return 0.0;
    }
    let mut ticks: Vec<u32> = notes.iter().map(|note| note.start_tick).collect();
    ticks.sort_unstable();
    let shared = ticks.windows(2).filter(|pair| pair[0] == pair[1]).count();
    shared as f32 / (ticks.len() - 1) as f32
}

/// Does this voice hold a root rather than play a line?
///
/// ⛔ **Register-free, and that is the whole point.** See
/// [`ROOT_PART_MAX_PITCHES`]: an 808 is written wherever the producer's sampler
/// wants it, so the only reliable signal is what the part *does* — sit on one or
/// two pitches and hold them.
fn holds_a_root(notes: &[Note]) -> bool {
    if notes.is_empty() {
        return false;
    }
    let mut distinct: Vec<u8> = notes.iter().map(|note| note.pitch).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() > ROOT_PART_MAX_PITCHES {
        return false;
    }

    let mut lengths: Vec<u32> = notes.iter().map(|note| note.len_ticks).collect();
    lengths.sort_unstable();
    lengths[lengths.len() / 2] >= ROOT_PART_MIN_LEN
}

/// Is this voice a drum kit, whatever channel it arrived on?
///
/// ⛔⛔ **Channel 10 is the convention and real files ignore it.** Mike's own
/// `Starlight-Drums.mid` — a Cymatics loop, 192 notes of kick, closed hat and
/// snare — is on **channel 0**, and the first cut of this module filed it as a
/// bassline. Sample-pack MIDI is exported from a DAW's piano roll, and the piano
/// roll does not care which channel General MIDI reserved.
///
/// ▶ **Four things together, because no one of them is enough.** Measured on
/// Mike's fixtures, which is where every threshold below comes from:
///
/// | | drums | melody | hook |
/// |---|---|---|---|
/// | on a GM drum pitch | 100% | 62% | 50% |
/// | distinct pitches | 3 | 9 | 12 |
/// | strikes per pitch | 64.0 | 2.3 | 1.7 |
/// | median length | 240 | 920 | 1250 |
///
/// 1. Nearly every note lands on a GM drum pitch ([`KIT_MIN_MATCH`]) — on its
///    own this is weak, since those are ordinary MIDI pitches and any melody in
///    the register hits a few by coincidence.
/// 2. It uses **few** distinct pitches, and more than one ([`KIT_MAX_PITCHES`],
///    [`KIT_MIN_PITCHES`]).
/// 3. It strikes each of them **many** times ([`KIT_MIN_REPEATS`]).
/// 4. Its notes are **short** ([`KIT_MAX_MEDIAN_LEN`]) — the one that separates a
///    kit from a sustained 808 sitting on the same pitches.
///
/// A kit is a small vocabulary, struck hard and struck short. A melody is a large
/// vocabulary, used once or twice each and held.
fn looks_like_a_kit(notes: &[Note]) -> bool {
    if notes.is_empty() {
        return false;
    }

    let matching = notes
        .iter()
        .filter(|note| {
            LANE_ORDER
                .iter()
                .any(|lane| gm_drum_note(*lane) == note.pitch)
        })
        .count();
    if (matching as f32 / notes.len() as f32) < KIT_MIN_MATCH {
        return false;
    }

    let mut distinct: Vec<u8> = notes.iter().map(|note| note.pitch).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < KIT_MIN_PITCHES || distinct.len() > KIT_MAX_PITCHES {
        return false;
    }

    if (notes.len() as f32 / distinct.len() as f32) < KIT_MIN_REPEATS {
        return false;
    }

    let mut lengths: Vec<u32> = notes.iter().map(|note| note.len_ticks).collect();
    lengths.sort_unstable();
    lengths[lengths.len() / 2] <= KIT_MAX_MEDIAN_LEN
}

/// This engine's tick for a tick counted at the file's own resolution.
fn rebase(tick: u64, file_ppq: u32) -> u32 {
    let scaled = tick * u64::from(PPQ) / u64::from(file_ppq);
    scaled.min(u64::from(u32::MAX)) as u32
}

/// Close the most recent open note on this channel and pitch.
///
/// Most recent rather than first: two note-ons on one pitch before either off is
/// legal and is what a held-then-restruck note looks like, and pairing the off
/// with the *older* one would leave the newer hanging to the end of the track.
fn close(
    open: &mut Vec<(u8, u32, u8, u8)>,
    notes: &mut Vec<(u8, Note)>,
    channel: u8,
    key: u8,
    tick: u32,
) {
    let Some(at) = open
        .iter()
        .rposition(|(ch, _, k, _)| *ch == channel && *k == key)
    else {
        // A note-off with no note-on is an exporter's mistake, not ours.
        return;
    };
    let (_, start, pitch, vel) = open.remove(at);
    notes.push((
        channel,
        Note {
            start_tick: start,
            len_ticks: tick.saturating_sub(start).max(1),
            pitch,
            vel: vel.max(1),
            model_vel: None,
            slide_to_pitch: None,
            articulation: None,
        },
    ));
}

/// The lanes a set of notes becomes, for this part.
///
/// ⚠ **Takes notes, not `(channel, note)`.** It never read the channel — both
/// arms discarded it — and taking one meant every caller rebuilt a `Vec` of pairs
/// purely as ceremony, with `pitched` inventing a channel `0` that meant nothing.
/// The one caller that *has* channels drops them on the way in, where that is
/// visible.
fn into_lanes(part: Part, notes: Vec<Note>) -> Vec<LaneTrack> {
    if part != Part::Drums {
        // One line. The pitched parts each write a single lane, so a file read
        // as a melody is a melody however many channels it arrived on.
        let lane = match part {
            Part::Melody => Lane::Melody,
            Part::Counter => Lane::Counter,
            Part::Bass => Lane::Bass,
            _ => Lane::Chords,
        };
        return vec![LaneTrack { lane, notes }];
    }

    // ⚠ The inverse of `gm_drum_note`, built once from the lane list rather than
    // restated as a second table — two tables mapping the same thing is how one
    // of them starts being wrong.
    let mut tracks: Vec<LaneTrack> = Vec::new();
    for note in notes {
        let Some(lane) = LANE_ORDER
            .iter()
            .copied()
            .find(|lane| gm_drum_note(*lane) == note.pitch)
        else {
            // Dropped rather than guessed onto a lane. A note on no known GM
            // drum is something this map cannot name, and putting it on the
            // nearest one would invent percussion the producer never played.
            continue;
        };

        match tracks.iter_mut().find(|track| track.lane == lane) {
            Some(track) => track.notes.push(note),
            None => tracks.push(LaneTrack {
                lane,
                notes: vec![note],
            }),
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::pattern_to_smf;

    /// A pattern, written and read back.
    fn round_trip(pattern: &Pattern) -> Pattern {
        let bytes = pattern_to_smf(pattern);
        smf_to_pattern(&bytes, pattern.part, "read").expect("what we wrote must read")
    }

    fn clip(part: Part, lane: Lane, pitches: &[(u32, u8)]) -> Pattern {
        Pattern {
            id: "written".into(),
            part,
            artist_id: "trap".into(),
            seed: 7,
            song_seed: 7,
            bars: 4,
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane,
                notes: pitches
                    .iter()
                    .map(|(tick, pitch)| Note {
                        start_tick: *tick,
                        len_ticks: PPQ / 4,
                        pitch: *pitch,
                        vel: 100,
                        model_vel: None,
                        slide_to_pitch: None,
                        articulation: None,
                    })
                    .collect(),
            }],
            ppq: PPQ,
            mood: None,
            loop_region: None,
            clip_region: None,
        }
    }

    #[test]
    fn what_the_writer_wrote_is_what_the_reader_reads() {
        // ⛔ The strongest available check on both halves: the exporter is
        // already gated against a real SMF parser, so agreeing with it is
        // agreeing with the format.
        let written = clip(Part::Melody, Lane::Melody, &[(0, 60), (480, 63), (960, 67)]);
        let read = round_trip(&written);

        assert_eq!(read.lanes.len(), 1);
        let got: Vec<(u32, u8)> = read.lanes[0]
            .notes
            .iter()
            .map(|n| (n.start_tick, n.pitch))
            .collect();
        assert_eq!(got, vec![(0, 60), (480, 63), (960, 67)]);
        assert_eq!(read.bpm.round(), 140.0);
        assert_eq!(read.ppq, PPQ);
    }

    #[test]
    fn a_drum_file_comes_back_on_the_lanes_it_was_written_from() {
        // ⛔ The writer replaces each note's pitch with its lane's GM note, so a
        // two-lane pattern is the only way to test the inverse — one lane
        // carrying two "pitches" would be written as two hits of that one lane,
        // which is what the exporter is *for*.
        let mut written = clip(Part::Drums, Lane::Kick, &[(0, 0), (960, 0)]);
        written.lanes.push(LaneTrack {
            lane: Lane::Snare,
            notes: vec![Note {
                start_tick: 480,
                len_ticks: PPQ / 4,
                pitch: 0,
                vel: 100,
                model_vel: None,
                slide_to_pitch: None,
                articulation: None,
            }],
        });

        let read = round_trip(&written);
        let lanes: Vec<Lane> = read.lanes.iter().map(|track| track.lane).collect();
        assert!(lanes.contains(&Lane::Kick), "{lanes:?}");
        assert!(lanes.contains(&Lane::Snare), "{lanes:?}");
    }

    #[test]
    fn a_note_on_no_known_drum_is_dropped_rather_than_guessed_onto_a_lane() {
        // Putting it on the nearest lane would invent percussion the producer
        // never played, and the fit would then measure it. Pitch 3 is below
        // every GM drum note, and read as drums it maps to nothing.
        let stray = clip(Part::Melody, Lane::Melody, &[(0, 3)]);
        let bytes = pattern_to_smf(&stray);

        let error = smf_to_pattern(&bytes, Part::Drums, "read").unwrap_err();
        assert!(error.contains("lane"), "{error}");
    }

    #[test]
    fn an_empty_file_is_refused_rather_than_counted_as_a_kept_generation() {
        // ⛔ Otherwise the thirty-generation floor could be cleared with thirty
        // empty files, which is the failure the floor exists to prevent.
        let empty = clip(Part::Melody, Lane::Melody, &[]);
        let bytes = pattern_to_smf(&empty);
        let error = smf_to_pattern(&bytes, Part::Melody, "read").unwrap_err();
        assert!(error.contains("no notes"), "{error}");
    }

    #[test]
    fn rubbish_is_refused_by_name_rather_than_panicking() {
        let error = smf_to_pattern(b"this is not a midi file", Part::Melody, "read").unwrap_err();
        assert!(error.contains("MIDI"), "{error}");
    }

    #[test]
    fn a_file_at_another_resolution_is_rebased_rather_than_played_at_the_wrong_speed() {
        // A 96-PPQ file — the old hardware default, and still what a great many
        // exports use — has a quarter note at tick 96, not 960.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"MThd");
        bytes.extend(6u32.to_be_bytes());
        bytes.extend(0u16.to_be_bytes()); // format 0
        bytes.extend(1u16.to_be_bytes()); // one track
        bytes.extend(96u16.to_be_bytes()); // 96 ticks per quarter

        let mut track: Vec<u8> = Vec::new();
        track.extend([0x00, 0x90, 60, 100]); // note on at 0
        track.extend([96, 0x80, 60, 0]); // note off one quarter later
        track.extend([0x00, 0xFF, 0x2F, 0x00]); // end of track

        bytes.extend(b"MTrk");
        bytes.extend((track.len() as u32).to_be_bytes());
        bytes.extend(track);

        let read = smf_to_pattern(&bytes, Part::Melody, "read").expect("a 96-PPQ file must read");
        assert_eq!(read.lanes[0].notes[0].start_tick, 0);
        assert_eq!(
            read.lanes[0].notes[0].len_ticks, PPQ,
            "a quarter note at 96 PPQ is a quarter note at ours"
        );
    }

    #[test]
    fn a_note_on_with_zero_velocity_closes_a_note_rather_than_opening_one() {
        // ⛔ A great many exporters write note-offs this way. Treating them as
        // onsets would double every note in the file, and every density the fit
        // measured from it.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"MThd");
        bytes.extend(6u32.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend(1u16.to_be_bytes());
        bytes.extend(960u16.to_be_bytes());

        let mut track: Vec<u8> = Vec::new();
        track.extend([0x00, 0x90, 60, 100]);
        track.extend([0x60, 0x90, 60, 0]); // note-on, velocity 0
        track.extend([0x00, 0xFF, 0x2F, 0x00]);

        bytes.extend(b"MTrk");
        bytes.extend((track.len() as u32).to_be_bytes());
        bytes.extend(track);

        let read = smf_to_pattern(&bytes, Part::Melody, "read").expect("it must read");
        assert_eq!(read.lanes[0].notes.len(), 1, "the file has one note in it");
    }

    /// Build a type-1 SMF with one track per voice.
    ///
    /// ⚠ **Hand-built rather than round-tripped through `pattern_to_smf`.** The
    /// writer emits one `Pattern`, and a `Pattern` is one part — so the very
    /// thing `split` exists for, *several* voices in one file, cannot be
    /// produced by it. This is the fixture the feature is actually about.
    /// One voice for the fixture builder: (start tick, pitch, length).
    type Hit = (u32, u8, u32);

    fn smf_with(voices: &[(u8, &[Hit])]) -> Vec<u8> {
        fn vlq(mut value: u32, out: &mut Vec<u8>) {
            let mut buffer = vec![(value & 0x7F) as u8];
            value >>= 7;
            while value > 0 {
                buffer.push(((value & 0x7F) as u8) | 0x80);
                value >>= 7;
            }
            out.extend(buffer.iter().rev());
        }

        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend(b"MThd");
        bytes.extend(6u32.to_be_bytes());
        bytes.extend(1u16.to_be_bytes()); // format 1
        bytes.extend((voices.len() as u16).to_be_bytes());
        bytes.extend(960u16.to_be_bytes());

        for (channel, notes) in voices {
            // (tick, status, pitch, velocity), sorted — a track's deltas are
            // relative, so the events have to be in time order before encoding.
            let mut events: Vec<(u32, u8, u8, u8)> = Vec::new();
            for (start, pitch, len) in *notes {
                events.push((*start, 0x90 | channel, *pitch, 100));
                events.push((start + len, 0x80 | channel, *pitch, 0));
            }
            events.sort_by_key(|(tick, ..)| *tick);

            let mut track: Vec<u8> = Vec::new();
            let mut at = 0_u32;
            for (tick, status, pitch, vel) in events {
                vlq(tick - at, &mut track);
                at = tick;
                track.extend([status, pitch, vel]);
            }
            track.extend([0x00, 0xFF, 0x2F, 0x00]);

            bytes.extend(b"MTrk");
            bytes.extend((track.len() as u32).to_be_bytes());
            bytes.extend(track);
        }
        bytes
    }

    /// ⛔⛔ **The feature Mike asked for**, 2026-08-10: *"split it into the proper
    /// generators if it is a layered melody file with the bass and countermelody
    /// included."*
    #[test]
    fn a_layered_file_separates_into_bass_counter_and_melody() {
        // Three lines, three registers, none of them overlapping itself.
        let low: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 36, 240)).collect();
        let middle: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 60, 240)).collect();
        let high: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 79, 240)).collect();
        let bytes = smf_with(&[(0, &low), (1, &middle), (2, &high)]);

        let parts = split(&bytes, "layered").expect("a layered file must separate");
        let by_part: Vec<(Part, SplitReason)> = parts.iter().map(|p| (p.part, p.reason)).collect();

        assert!(
            by_part.contains(&(Part::Bass, SplitReason::LowestVoice)),
            "{by_part:?}"
        );
        assert!(
            by_part.contains(&(Part::Counter, SplitReason::InnerVoice)),
            "{by_part:?}"
        );
        assert!(
            by_part.contains(&(Part::Melody, SplitReason::HighestVoice)),
            "{by_part:?}"
        );
        // ⚠ Every note is accounted for. A split that quietly dropped a voice
        // would still pass the three assertions above.
        assert_eq!(parts.iter().map(|p| p.notes).sum::<u32>(), 12);
    }

    /// ⛔ Channel 10 is the one part assignment a file *states* rather than
    /// implies, so it is taken before anything is measured.
    #[test]
    fn the_gm_percussion_channel_becomes_the_drums() {
        let kick = gm_drum_note(Lane::Kick);
        let drums: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, kick, 120)).collect();
        let lead: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 72, 240)).collect();
        let bytes = smf_with(&[(DRUM_CHANNEL, &drums), (0, &lead)]);

        let parts = split(&bytes, "with-drums").expect("it must separate");
        let drums_part = parts
            .iter()
            .find(|p| p.part == Part::Drums)
            .expect("channel 10 is the drums");
        assert_eq!(drums_part.reason, SplitReason::DrumChannel);
        assert!(
            drums_part
                .pattern
                .lanes
                .iter()
                .any(|l| l.lane == Lane::Kick),
            "the kick must come back on its own lane"
        );
    }

    /// A voice that sounds against itself is a comp, not a line.
    #[test]
    fn a_chordal_voice_becomes_the_chords() {
        // Four triads, each note held over the next — dense with overlap.
        let mut chords: Vec<(u32, u8, u32)> = Vec::new();
        for bar in 0..4_u32 {
            for offset in [0_u8, 4, 7] {
                chords.push((bar * 960, 60 + offset, 960));
            }
        }
        let lead: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 84, 240)).collect();
        let bytes = smf_with(&[(0, &chords), (1, &lead)]);

        let parts = split(&bytes, "comped").expect("it must separate");
        let chords_part = parts
            .iter()
            .find(|p| p.part == Part::Chords)
            .expect("the held triads are the chords");
        assert_eq!(chords_part.reason, SplitReason::Polyphonic);
    }

    /// ⚠ **The weakest rule, and the one the page has to label.** A merged file
    /// has no voices to separate, so the only thing left is register — and
    /// `SplitByPitch` is what stops that being presented as a detection.
    #[test]
    fn one_merged_voice_is_cut_by_register_and_says_so() {
        let mut merged: Vec<(u32, u8, u32)> = Vec::new();
        for i in 0..4_u32 {
            merged.push((i * 960, 40, 240)); // a bassline...
            merged.push((i * 960 + 480, 72, 240)); // ...under a lead
        }
        let bytes = smf_with(&[(0, &merged)]);

        let parts = split(&bytes, "merged").expect("it must separate");
        assert_eq!(parts.len(), 2, "a low half and a high half");
        assert!(parts.iter().all(|p| p.reason == SplitReason::SplitByPitch));
        assert!(parts.iter().any(|p| p.part == Part::Bass));
        assert!(parts.iter().any(|p| p.part == Part::Melody));
    }

    /// ⛔⛔ **Found on a real file rather than on a fixture, and the fixtures had
    /// all passed.**
    ///
    /// Mike put `audioTest/TrapQuake.mid` in the repo on 2026-08-10 — one melodic
    /// line running pitch 54 to 62. It crosses middle C because melodies do, and
    /// the first cut of this rule filed it as a ten-note *bassline* under a
    /// two-note *lead*. Both parts were wrong, and the panel would have said so
    /// with a straight face.
    ///
    /// ▶ The rule now asks whether the halves are far enough apart to be two
    /// instruments at all — see [`SPLIT_GAP`].
    #[test]
    fn a_melody_that_merely_crosses_middle_c_is_not_cut_in_half() {
        // The shape of the real file: a line either side of C3 with no gap.
        let line: Vec<(u32, u8, u32)> = [54_u8, 57, 59, 62, 59, 57, 54, 59]
            .iter()
            .enumerate()
            .map(|(i, pitch)| (i as u32 * 480, *pitch, 240))
            .collect();
        let bytes = smf_with(&[(0, &line)]);

        let parts = split(&bytes, "one-line").expect("it must read");
        assert_eq!(parts.len(), 1, "one line is one part: {parts:?}");
        assert_eq!(parts[0].notes, 8, "and it keeps every note");
    }

    #[test]
    fn a_lead_over_a_real_bassline_is_still_cut() {
        // ⚠ The other side of the rule. Two instruments leave daylight between
        // them — this is fifteen semitones, where the melody above was three.
        let mut merged: Vec<(u32, u8, u32)> = Vec::new();
        for i in 0..4_u32 {
            merged.push((i * 960, 33, 240));
            merged.push((i * 960 + 480, 74, 240));
        }
        let bytes = smf_with(&[(0, &merged)]);

        let parts = split(&bytes, "two-instruments").expect("it must separate");
        assert_eq!(parts.len(), 2, "{parts:?}");
    }

    #[test]
    fn a_single_line_in_one_register_is_one_part_rather_than_an_empty_split() {
        // ⛔ Cutting this would emit a part with no notes in it, which reads as a
        // detection that found something and did not.
        let lead: Vec<(u32, u8, u32)> = (0..4).map(|i| (i * 960, 72, 240)).collect();
        let bytes = smf_with(&[(0, &lead)]);

        let parts = split(&bytes, "one-line").expect("it must read");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].part, Part::Melody);
        assert!(parts[0].notes > 0, "an empty part is not a result");
    }

    #[test]
    fn rubbish_is_refused_by_split_too() {
        assert!(split(b"this is not a MIDI file", "nope").is_err());
    }

    /// ⛔⛔ **One clip per part is a post-condition, and nothing enforced it.**
    ///
    /// The steps decide per *voice*: two chordal channels — piano and pad, which
    /// is an ordinary export — produced two `Chords` entries. Every consumer then
    /// dropped one silently: `smf_to_song` and `importSplit` write into a map
    /// keyed by part, and `MidiPreview` keys its rows on it, so React saw
    /// duplicate keys. The panel would report one voice's note count and the
    /// import would land the other's.
    #[test]
    fn two_voices_of_one_part_merge_rather_than_racing() {
        let mut piano: Vec<(u32, u8, u32)> = Vec::new();
        let mut pad: Vec<(u32, u8, u32)> = Vec::new();
        for bar in 0..4_u32 {
            for offset in [0_u8, 4, 7] {
                piano.push((bar * 960, 60 + offset, 960));
                pad.push((bar * 960, 72 + offset, 960));
            }
        }
        let bytes = smf_with(&[(0, &piano), (1, &pad)]);

        let parts = split(&bytes, "untitled-5").expect("it must read");
        let chords: Vec<&SplitPart> = parts.iter().filter(|p| p.part == Part::Chords).collect();
        assert_eq!(chords.len(), 1, "one clip per part: {parts:?}");
        // ⚠ **Merged, not discarded** — two chord voices are both the chords, and
        // keeping one loses half the harmony.
        assert_eq!(chords[0].notes, 24, "every note survives: {parts:?}");
    }

    /// ⛔⛔ **...and on the early-return paths too.**
    ///
    /// The fixture above has two chordal voices and *no* line, so it reaches the
    /// end of `split_parsed` and the merge runs. Add one bass line and
    /// `lines.len() == 1` returns three steps earlier — which used to skip the
    /// merge entirely and hand back two `Chords` entries. Found by a code review,
    /// 2026-08-10.
    #[test]
    fn two_chord_voices_beside_a_single_line_still_merge() {
        let mut piano: Vec<Hit> = Vec::new();
        let mut pad: Vec<Hit> = Vec::new();
        for bar in 0..4_u32 {
            for offset in [0_u8, 4, 7] {
                piano.push((bar * 960, 60 + offset, 960));
                pad.push((bar * 960, 72 + offset, 960));
            }
        }
        // The one pitched line that sends it down the early return.
        let line: Vec<Hit> = (0..4_u32).map(|i| (i * 960, 40, 400)).collect();
        let bytes = smf_with(&[(0, &piano), (1, &pad), (2, &line)]);

        let parts = split(&bytes, "untitled-6").expect("it must read");
        let chords = parts.iter().filter(|p| p.part == Part::Chords).count();
        assert_eq!(chords, 1, "one clip per part on every exit: {parts:?}");
    }

    /// Importing a whole song as an arrangement (TASK-058D).
    ///
    /// ⛔⛔ **Mike's design, 2026-08-10** — the file lands in the Song tab and the
    /// producer drills the parts they want out of it, rather than one drop
    /// overwriting a generator.
    mod as_a_song {
        use super::*;

        /// A bar of kick-and-hat, at `bar`, optionally with an extra snare.
        fn drum_bar(bar: u32, fill: bool) -> Vec<(u32, u8, u32)> {
            let at = bar * 3840;
            let (kick, hat, snare) = (
                gm_drum_note(Lane::Kick),
                gm_drum_note(Lane::ClosedHat),
                gm_drum_note(Lane::Snare),
            );
            let mut out: Vec<(u32, u8, u32)> = Vec::new();
            for step in 0..16_u32 {
                out.push((at + step * 240, hat, 240));
                if step % 4 == 0 {
                    out.push((at + step * 240, kick, 240));
                }
            }
            if fill {
                out.push((at + 3600, snare, 240));
            }
            out
        }

        /// ⛔⛔ **The case Mike asked about**, 2026-08-10: *"will it get an 8 bar
        /// generation of the full drum loop if it splits into 2 parts because
        /// some trap drums vary every 4 bars for a verse"*
        ///
        /// A file that runs four identical bars then four with a fill has a
        /// **four**-bar period that mostly matches and an **eight**-bar one that
        /// matches exactly. Taking the shortest would return four bars and
        /// silently drop the fill; [`SECTION_BARS`] is walked longest-first for
        /// this reason.
        #[test]
        fn a_variation_every_four_bars_is_kept_by_sectioning_at_eight() {
            let mut notes: Vec<(u32, u8, u32)> = Vec::new();
            for block in 0..2_u32 {
                for bar in 0..8_u32 {
                    // Bars 0-3 plain, 4-7 with a fill — repeated twice.
                    notes.extend(drum_bar(block * 8 + bar, bar >= 4));
                }
            }
            let bytes = smf_with(&[(0, &notes)]);

            let song = smf_to_song(&bytes, "verse").expect("it must read");
            assert_eq!(song.sections.len(), 2, "two eight-bar blocks");
            assert!(
                song.sections.iter().all(|section| section.bars == 8),
                "the fill must not be cut away: {:?}",
                song.sections.iter().map(|s| s.bars).collect::<Vec<_>>()
            );
            // ⛔ **And the two identical blocks share one pattern**, which is
            // what `Song::patterns` is shaped for.
            assert_eq!(song.patterns.len(), 1, "{:?}", song.patterns.keys());
        }

        #[test]
        fn each_section_names_the_parts_it_actually_contains() {
            // Drums throughout; a bass only in the second half.
            let mut notes: Vec<(u32, u8, u32)> = Vec::new();
            for bar in 0..8_u32 {
                notes.extend(drum_bar(bar, false));
            }
            let mut bass: Vec<(u32, u8, u32)> = Vec::new();
            for beat in 0..16_u32 {
                bass.push((4 * 3840 + beat * 960, 61, 900));
            }
            let bytes = smf_with(&[(0, &notes), (1, &bass)]);

            let song = smf_to_song(&bytes, "half-and-half").expect("it must read");
            let with_bass = song
                .sections
                .iter()
                .filter(|s| s.patterns.contains_key(&Part::Bass))
                .count();
            assert!(with_bass >= 1, "the bass must appear: {:?}", song.sections);
            assert!(
                song.sections
                    .iter()
                    .any(|s| s.patterns.contains_key(&Part::Drums)),
                "and so must the drums"
            );
        }

        /// ⛔ **A section's clip starts at tick zero.** A pattern carrying
        /// absolute ticks would place the whole of verse two after the end of its
        /// own four bars, and the section would sound empty.
        #[test]
        fn a_sections_clip_is_rebased_to_its_own_start() {
            let mut notes: Vec<(u32, u8, u32)> = Vec::new();
            for bar in 0..4_u32 {
                notes.extend(drum_bar(bar, false));
            }
            let bytes = smf_with(&[(0, &notes)]);

            let song = smf_to_song(&bytes, "rebased").expect("it must read");
            for pattern in song.patterns.values() {
                let earliest = pattern
                    .lanes
                    .iter()
                    .flat_map(|lane| lane.notes.iter())
                    .map(|note| note.start_tick)
                    .min()
                    .unwrap_or(u32::MAX);
                assert!(
                    earliest < ticks_per_bar_of(4, 4),
                    "every clip must begin in its first bar, not at its song position"
                );
            }
        }

        /// ⚠ **An import has no artist and no seed, and says so.** A number
        /// invented here would be a seed that reproduces nothing.
        #[test]
        fn an_imported_song_claims_no_artist_and_no_seed() {
            let mut notes: Vec<(u32, u8, u32)> = Vec::new();
            for bar in 0..4_u32 {
                notes.extend(drum_bar(bar, false));
            }
            let bytes = smf_with(&[(0, &notes)]);

            let song = smf_to_song(&bytes, "borrowed").expect("it must read");
            assert!(song.artist_id.is_empty(), "{:?}", song.artist_id);
            assert_eq!(song.seed, 0);
        }

        #[test]
        fn rubbish_is_refused_rather_than_arranged() {
            assert!(smf_to_song(b"not a MIDI file at all", "nope").is_err());
        }

        /// ⛔⛔ **A hostile `.mid` could kill the host process, and in a shipped
        /// build that is not a panic — it is an abort.**
        ///
        /// Found by a security review on 2026-08-10 and reproduced in both
        /// profiles. A file declaring **one tick per quarter** and a **255/1**
        /// time signature rebases its ticks to near `u32::MAX` and makes a bar
        /// 979,200 ticks wide. `total_bars` was cast to `u16` without the clamp
        /// `assemble` applies, and `section_length`'s window then computed
        /// `from + span` past the end of the tick space:
        ///
        /// - debug: `attempt to add with overflow`
        /// - release: wrapped, so `end` came back *below* `start` and the slice
        ///   panicked — and `panic = "abort"` is release-only, so the DAW dies
        ///   with the producer's unsaved session in it.
        ///
        /// ⚠ Delivery is ordinary for this product: MIDI packs and type-beat
        /// starters are traded exactly like the project files this codebase
        /// already treats as untrusted. The producer only has to drop the pack in
        /// a library folder and click the file.
        #[test]
        fn a_file_whose_ticks_reach_the_top_of_the_space_is_read_rather_than_aborting() {
            let mut bytes: Vec<u8> = Vec::new();
            bytes.extend(b"MThd");
            bytes.extend(6u32.to_be_bytes());
            bytes.extend(0u16.to_be_bytes());
            bytes.extend(1u16.to_be_bytes());
            // ⚠ One tick per quarter, so `rebase` multiplies by PPQ and saturates.
            bytes.extend(1u16.to_be_bytes());

            let mut track: Vec<u8> = Vec::new();
            // 255/1 — the widest bar the format can declare.
            track.extend([0x00, 0xFF, 0x58, 0x04, 0xFF, 0x00, 0x18, 0x08]);
            track.extend([0x00, 0x90, 60, 100]);
            track.extend([0x01, 0x80, 60, 0]);
            // A delta large enough to push the last tick to the top of `u32`.
            track.extend([0x00, 0x90, 62, 100]);
            track.extend([0x9E, 0xB1, 0xD9, 0x05, 0x80, 62, 0]);
            track.extend([0x00, 0xFF, 0x2F, 0x00]);

            bytes.extend(b"MTrk");
            bytes.extend((track.len() as u32).to_be_bytes());
            bytes.extend(track);

            // ⛔ The assertion is that it *returns* — either an arrangement or a
            // refusal. What it must never do is take the process with it.
            let _ = smf_to_song(&bytes, "hostile");
            let _ = split(&bytes, "hostile");
            let _ = smf_to_pattern(&bytes, Part::Melody, "hostile");
        }
    }

    /// ⛔⛔ **Three real files, three wrong answers — and every threshold in this
    /// module now comes from measuring them.**
    ///
    /// Mike put `audioTest/Starlight-{Drums,Melody,Hook}.mid` in the tree on
    /// 2026-08-10. They are Cymatics sample-pack exports, and the rules — all of
    /// which passed the synthetic fixtures above — got all three wrong:
    ///
    /// | file | was | is |
    /// |---|---|---|
    /// | Drums | Bass | Drums |
    /// | Melody | Bass | Melody |
    /// | Hook | Chords | Melody |
    ///
    /// The fixtures here reproduce each file's *measured shape* rather than
    /// depending on the files, which are gitignored and are Mike's own. The
    /// numbers in the table below are the real ones.
    mod shapes_measured_from_real_sample_pack_midi {
        use super::*;

        /// `Starlight-Drums.mid`: 192 notes, pitches 36/38/42, median length 240,
        /// **on channel 0**. Sample packs export from a piano roll, and a piano
        /// roll does not use GM's reserved channel 10.
        #[test]
        fn a_drum_loop_on_channel_zero_is_still_drums() {
            let (kick, snare, hat) = (
                gm_drum_note(Lane::Kick),
                gm_drum_note(Lane::Snare),
                gm_drum_note(Lane::ClosedHat),
            );
            let mut hits: Vec<(u32, u8, u32)> = Vec::new();
            for step in 0..64_u32 {
                // A 16th hat throughout, kick on the beat, snare on the backbeat.
                hits.push((step * 240, hat, 240));
                if step % 4 == 0 {
                    hits.push((step * 240, kick, 240));
                }
                if step % 8 == 4 {
                    hits.push((step * 240, snare, 240));
                }
            }
            let bytes = smf_with(&[(0, &hits)]);

            let parts = split(&bytes, "pack-drums").expect("it must read");
            assert_eq!(parts.len(), 1, "{parts:?}");
            assert_eq!(parts[0].part, Part::Drums, "{parts:?}");
            // ⚠ **KitShape, not DrumChannel** — this arrived on channel 0 and was
            // recognised by measurement. `DrumChannel` promises the file said so.
            assert_eq!(parts[0].reason, SplitReason::KitShape);
        }

        /// ⛔ **The adversarial case the length and pitch-count rules exist for.**
        /// C1 and D1 are ordinary 808 notes *and* the GM kick and snare, so an
        /// 808 line passes the GM test outright. What it does not do is strike a
        /// handful of pitches *short* — a bass note is held.
        #[test]
        fn a_sustained_808_on_drum_pitches_is_not_a_kit() {
            let kick = gm_drum_note(Lane::Kick);
            let snare = gm_drum_note(Lane::Snare);
            // Long notes, the shape of an 808 line rather than a kit.
            let line: Vec<(u32, u8, u32)> = (0..16_u32)
                .map(|i| (i * 960, if i % 3 == 0 { snare } else { kick }, 900))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "eight-o-eight").expect("it must read");
            assert!(
                parts.iter().all(|p| p.part != Part::Drums),
                "a held 808 is not a drum kit: {parts:?}"
            );
        }

        /// `Starlight-Hook.mid`: 20 notes, 4 sharing an onset (21%), median
        /// length 1250. Held melodic notes overlap constantly — legato is not
        /// harmony, and measuring overlap rather than shared onsets filed this as
        /// chords.
        #[test]
        fn a_legato_line_is_not_chords() {
            // Every note overlaps the next by half its length; none start together.
            let line: Vec<(u32, u8, u32)> = [67_u8, 70, 72, 75, 72, 70, 67, 65]
                .iter()
                .enumerate()
                .map(|(i, pitch)| (i as u32 * 480, *pitch, 960))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "untitled-1").expect("it must read");
            assert_eq!(parts.len(), 1, "{parts:?}");
            assert_eq!(parts[0].part, Part::Melody, "{parts:?}");
            // ⛔ **The reason, not just the part.** A neutral id is what forces
            // this through the register rule — with a descriptive one,
            // `part_from_name` answers first and the rule under test never runs.
            assert_eq!(parts[0].reason, SplitReason::SplitByPitch, "{parts:?}");
        }

        /// `Starlight-Melody.mid`: median pitch **39**, spanning 33..74 — three
        /// and a half octaves. Low by median, but a bassline does not roam three
        /// octaves.
        ///
        /// ⚠ **The octave histogram is copied from the real file**, not
        /// caricatured: `C1=3 C2=11 C3=5 C4=1 C5=1`. The first version of this
        /// fixture put nothing in C3 — which left an octave of daylight in the
        /// middle, so the file split into two instruments and never reached the
        /// rule under test. A fixture that skips the middle is testing a
        /// different file.
        #[test]
        fn a_low_melody_that_roams_is_not_a_bassline() {
            let pitches: [u8; 21] = [
                33, 34, 35, // C1 = 3
                36, 38, 39, 40, 41, 43, 45, 46, 40, 38, 36, // C2 = 11
                48, 50, 53, 55, 59, // C3 = 5 — the ones that close the gap
                62, // C4 = 1
                74, // C5 = 1
            ];
            let line: Vec<(u32, u8, u32)> = pitches
                .iter()
                .enumerate()
                .map(|(i, pitch)| (i as u32 * 480, *pitch, 400))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "untitled-2").expect("it must read");
            assert_eq!(parts.len(), 1, "one line, not two instruments: {parts:?}");
            assert_eq!(parts[0].part, Part::Melody, "{parts:?}");
            assert_eq!(parts[0].reason, SplitReason::SplitByPitch, "{parts:?}");
        }

        /// ⛔⛔ **808 MIDI is written at TRIGGER pitch, not SOUNDING pitch.**
        ///
        /// `Starlight-Bass.mid` is five notes, every one on pitch **61** — C♯4,
        /// squarely melody register — each a full quarter long. The 808 sample is
        /// already low; the MIDI note only says which key fires it. Every
        /// register-based rule in this module is blind to that, and filed the
        /// bassline as a melody until `holds_a_root` was checked first.
        ///
        /// ⚠ This is the single most common bass instrument in the genre this
        /// product is for, so the case is not exotic.
        #[test]
        fn an_808_written_in_melody_register_is_still_the_bass() {
            // The real file's shape: one pitch, held a quarter each, sparse.
            let line: Vec<(u32, u8, u32)> = (0..5_u32).map(|i| (i * 1920, 61_u8, 960)).collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "eight-o-eight").expect("it must read");
            assert_eq!(parts.len(), 1, "{parts:?}");
            assert_eq!(
                parts[0].part,
                Part::Bass,
                "an 808 at trigger pitch is still the bass: {parts:?}"
            );
        }

        /// ⚠ The other side of `holds_a_root`: a *moving* line in the same
        /// register is a melody, however long its notes are. Without this the
        /// root rule would swallow every slow lead.
        #[test]
        fn a_held_but_moving_line_is_still_a_melody() {
            let line: Vec<(u32, u8, u32)> = [61_u8, 64, 66, 68, 66, 64]
                .iter()
                .enumerate()
                .map(|(i, pitch)| (i as u32 * 960, *pitch, 960))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "untitled-3").expect("it must read");
            assert_eq!(parts[0].part, Part::Melody, "{parts:?}");
            assert_eq!(parts[0].reason, SplitReason::SplitByPitch, "{parts:?}");
        }

        /// ⛔⛔ **The case Mike raised, and the reason the filename is used at
        /// all.**
        ///
        /// 2026-08-10: *"in trap beats the 808 bass notes can go up and down
        /// though."* Correct — a trap 808 follows the chord roots. So "few
        /// pitches, held long" describes his static fixture and not the common
        /// case, and widening that threshold until it covered moving 808s would
        /// swallow every slow lead instead.
        ///
        /// ▶ A measurement cannot separate a moving 808 from a slow melody when
        /// both are one voice in an arbitrary register. **The filename can**, and
        /// it is the producer saying so rather than the app guessing.
        #[test]
        fn a_moving_808_is_still_the_bass_when_the_file_says_bass() {
            // Walking the roots of a progression, in melody register, moving.
            let line: Vec<(u32, u8, u32)> = [61_u8, 68, 63, 66, 61, 70, 63, 68]
                .iter()
                .enumerate()
                .map(|(i, pitch)| (i as u32 * 960, *pitch, 900))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            // Measured alone this is a melody — eight pitches, roaming, high.
            let guessed = split(&bytes, "untitled-loop").expect("it must read");
            assert_eq!(guessed[0].part, Part::Melody, "{guessed:?}");

            // Named, it is the bass, and the reason says where that came from.
            let named = split(&bytes, "Starlight-Bass").expect("it must read");
            assert_eq!(named[0].part, Part::Bass, "{named:?}");
            assert_eq!(named[0].reason, SplitReason::FromName);
        }

        /// ⚠ **Whole words.** A substring test would match `Bassoon`, and
        /// `Starlight` contains neither "star" nor anything else that matters —
        /// but a careless matcher finds parts in ordinary words constantly.
        #[test]
        fn a_name_is_matched_by_whole_words_only() {
            assert_eq!(part_from_name("Starlight-Bass"), Some(Part::Bass));
            assert_eq!(part_from_name("Starlight Drums 164"), Some(Part::Drums));
            assert_eq!(part_from_name("my_808_loop"), Some(Part::Bass));
            assert_eq!(part_from_name("Hook Midnight"), Some(Part::Melody));

            // ...and these name nothing.
            assert_eq!(part_from_name("Bassoon Solo"), None);
            assert_eq!(part_from_name("Embassy"), None);
            assert_eq!(part_from_name("Starlight"), None);
            assert_eq!(part_from_name("untitled-1"), None);
        }

        /// ⛔ **A near-certain measurement outranks a careless name.** A file
        /// called `Bass` that is 192 short hits on GM drum pitches is a drum loop
        /// somebody named badly.
        #[test]
        fn an_unmistakable_drum_loop_beats_a_wrong_filename() {
            let (kick, hat) = (gm_drum_note(Lane::Kick), gm_drum_note(Lane::ClosedHat));
            let mut hits: Vec<(u32, u8, u32)> = Vec::new();
            for step in 0..64_u32 {
                hits.push((step * 240, hat, 240));
                if step % 4 == 0 {
                    hits.push((step * 240, kick, 240));
                }
            }
            let bytes = smf_with(&[(0, &hits)]);

            let parts = split(&bytes, "Starlight-Bass").expect("it must read");
            assert_eq!(parts[0].part, Part::Drums, "{parts:?}");
            assert_eq!(parts[0].reason, SplitReason::KitShape);
        }

        /// ...and the other side of that rule: low **and** narrow really is bass.
        #[test]
        fn a_low_narrow_line_is_a_bassline() {
            let line: Vec<(u32, u8, u32)> = [33_u8, 33, 36, 33, 40, 33, 36, 33]
                .iter()
                .enumerate()
                .map(|(i, pitch)| (i as u32 * 480, *pitch, 400))
                .collect();
            let bytes = smf_with(&[(0, &line)]);

            let parts = split(&bytes, "untitled-4").expect("it must read");
            assert_eq!(parts[0].part, Part::Bass, "{parts:?}");
            assert_eq!(parts[0].reason, SplitReason::SplitByPitch, "{parts:?}");
        }
    }
}
