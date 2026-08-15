//! Decoding a one-shot off the producer's disk (TASK-131B).
//!
//! ⛔ **A different problem from [`super::kit::decode_wav`], which is why it is a
//! different module.** That reader decodes the *shipped* kit: bytes this repo
//! generated, in a format it controls exactly, and its narrowness is the feature
//! — a kit that shipped in the wrong format fails the build rather than sounding
//! like noise. This reads whatever a sample pack happened to use, off a disk
//! nobody here controls, and has to say *why* when it cannot.
//!
//! ## The rules that are not negotiable
//!
//! - **This is a trust boundary.** The bytes are an arbitrary file, chosen in a
//!   file dialog, parsed inside somebody else's DAW process. Symphonia is
//!   `#![forbid(unsafe_code)]` for that reason, and everything below it is
//!   bounded: the file is refused before it is read if it is absurd, and the
//!   decode stops at [`MAX_SECONDS`] rather than trusting a header.
//! - **Never on the audio thread.** Decoding allocates, seeks and parses. It
//!   runs on the loader thread [`crate::oneshot`] owns, and what reaches the
//!   audio thread is a finished buffer.
//! - **Refuse rather than approximate.** A file that decodes to silence, or to a
//!   rate no sampler should believe, is handed back as an error with the reason
//!   in it. A pad that loads "successfully" and plays nothing is the exact
//!   failure TASK-131A existed to fix, arriving through a different door.

use std::io::Cursor;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use super::kit::DecodedAudio;

/// The largest file this will read off disk at all, before any decoding.
///
/// A one-shot is kilobytes to a few megabytes. This is set far above anything
/// real so it never binds on a legitimate sample, and exists so that picking a
/// video file, a disk image or a whole album by mistake is refused in one stat
/// call rather than by reading gigabytes into the host's memory.
pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

/// The longest sample a pad will hold.
///
/// ⛔ **A bound on the decoded output, not on the file.** A compressed format is
/// a compression ratio, so "the file is small" says nothing about what it
/// decodes to — a few hundred kilobytes of FLAC can be minutes of audio, and a
/// crafted one can be far worse. This is what actually stops the decode.
///
/// Thirty seconds is well past any one-shot and long enough for a riser or a
/// long 808 tail, so it does not bind on the thing the feature is for.
pub const MAX_SECONDS: u32 = 30;

/// The longest file the note extractor will read (TASK-058F).
///
/// ⛔ **Its own number, because it bounds a different thing.** [`MAX_SECONDS`] is
/// how long a *pad* holds; extraction is handed loops, stems and sections, and a
/// four-bar loop at 70 BPM is already 13 seconds while a two-bar intro of a whole
/// stem is a minute. Refusing a stem for being longer than a one-shot would refuse
/// the case the feature is for.
///
/// ⚠ **And it is not larger than it has to be.** The extractor filters the buffer
/// into four bands and decimates two more, so peak memory is several times the
/// decoded length — inside somebody else's DAW. A minute of mono `f32` at 48 kHz
/// is 11.5 MB and the whole analysis stays inside a couple of hundred; ten minutes
/// would not.
pub const MAX_EXTRACT_SECONDS: u32 = 60;

/// Rates a sampler should believe. Below the first, resampling a pad up to the
/// device rate is inaudible mush; above the second, a header is lying.
const MIN_RATE: u32 = 4_000;
const MAX_RATE: u32 = 384_000;

/// Anything at or under this peak is silence for our purposes.
///
/// Not zero: a lossy codec renders digital silence as very small non-zero
/// values, so an exact test would let an empty MP3 through and call it a pad.
const SILENCE_PEAK: f32 = 1.0e-4;

/// Read and decode a file into mono f32.
///
/// The extension is passed to the prober as a *hint* only — the format is
/// decided by the bytes, so a `.wav` that is really an MP3 still plays and a
/// renamed text file still fails.
pub fn decode_file(path: &std::path::Path) -> Result<DecodedAudio, String> {
    decode_file_within(path, MAX_SECONDS)
}

/// The same, for a caller whose bound is not a pad's.
///
/// ⚠ **A parameter rather than a second reader.** Everything else about reading
/// an arbitrary file off a producer's disk — the size stat before the read, the
/// format probe, the mono fold, the NaN guard, the silence check — is identical,
/// and a second copy of it is the one that would drift.
pub fn decode_file_within(
    path: &std::path::Path,
    max_seconds: u32,
) -> Result<DecodedAudio, String> {
    // ⛔ Length first, then read. Reading and then checking is the check the
    // allocation already happened for.
    let size = std::fs::metadata(path)
        .map_err(|e| format!("could not open that file: {e}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "that file is {} MB, and a one-shot may be up to {} MB",
            size / (1024 * 1024),
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }

    let bytes = std::fs::read(path).map_err(|e| format!("could not read that file: {e}"))?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    decode_within(bytes, extension.as_deref(), max_seconds)
}

/// Decode bytes already in memory. The seam the tests drive.
pub fn decode(bytes: Vec<u8>, extension: Option<&str>) -> Result<DecodedAudio, String> {
    decode_within(bytes, extension, MAX_SECONDS)
}

/// The same, bounded by the caller. See [`decode_file_within`].
pub fn decode_within(
    bytes: Vec<u8>,
    extension: Option<&str>,
    max_seconds: u32,
) -> Result<DecodedAudio, String> {
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }

    let stream = MediaSourceStream::new(
        Box::new(Cursor::new(bytes)),
        MediaSourceStreamOptions::default(),
    );

    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("that file is not audio this can read ({e})"))?;

    // ⛔ `first_track_known_codec`, not `tracks()[0]`. An MP4 opens with a video
    // track and an M4A written by some encoders carries a cover-art track ahead
    // of the audio — taking the first would fail on a file that plays perfectly
    // everywhere else.
    let track = reader
        .first_track_known_codec(TrackType::Audio)
        .ok_or("that file has no audio track this can decode")?;
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err("that file has no audio track this can decode".to_owned());
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("that file uses a codec this cannot decode ({e})"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_rate = 0u32;
    // Held so the mono fold below cannot silently change meaning mid-file: a
    // stream that switches channel count has to be refused, not averaged
    // across two different layouts.
    let mut channels = 0usize;
    let mut budget = 0usize;

    while let Some(packet) = reader
        .next_packet()
        .map_err(|e| format!("that file stops part way through ({e})"))?
    {
        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // ⛔ One bad packet is not a bad file. A trailing partial frame is
            // ordinary in MP3 and OGG; refusing the whole sample over it would
            // reject files every DAW plays.
            Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => continue,
            Err(e) => return Err(format!("that file could not be decoded ({e})")),
        };

        let (rate, planes) = spec_of(&decoded);
        if sample_rate == 0 {
            if !(MIN_RATE..=MAX_RATE).contains(&rate) {
                return Err(format!("{rate} Hz is not a sample rate this can play"));
            }
            if planes == 0 {
                return Err("that file declares no channels".to_owned());
            }
            sample_rate = rate;
            channels = planes;
            // Computed from the rate the file actually decoded at, so the bound
            // is a number of *seconds* rather than a frame count that means
            // something different at every rate.
            budget = max_seconds as usize * rate as usize;
        } else if rate != sample_rate || planes != channels {
            return Err("that file changes format part way through".to_owned());
        }

        decoded.copy_to_vec_interleaved(&mut interleaved);
        // Downmix by averaging, the same way the shipped kit's reader does.
        // Summing would clip a correlated stereo pair.
        samples.extend(
            interleaved
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );

        if samples.len() > budget {
            return Err(format!(
                "that sample is longer than {max_seconds} seconds, which is as long as this reads"
            ));
        }
    }

    if samples.is_empty() {
        return Err("that file decoded to no audio at all".to_owned());
    }

    // ⛔ **Checked explicitly, because `f32::max` CANNOT see a NaN.** It is
    // documented to return the other operand when one side is NaN, so folding
    // with `max` skips every NaN and yields a finite peak — the guard below used
    // to read that peak and could never fire. A crafted float-PCM WAV (the `pcm`
    // feature decodes PCM_F32LE) mixing NaN with ordinary audio therefore loaded
    // "successfully", and the sampler mixed NaN into the host's output bus,
    // which propagates through the DAW's entire signal path.
    if samples.iter().any(|s| !s.is_finite()) {
        return Err("that file decoded to values that are not audio".to_owned());
    }
    let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    if peak <= SILENCE_PEAK {
        // ⚠ Reported rather than loaded. A silent pad is indistinguishable from
        // a broken assignment, and the producer is the only one who can tell
        // "I picked the wrong file" from "the plugin is broken".
        return Err("that file is silent".to_owned());
    }

    Ok(DecodedAudio {
        samples,
        sample_rate,
    })
}

/// The rate and channel count of a decoded packet.
fn spec_of(decoded: &GenericAudioBufferRef<'_>) -> (u32, usize) {
    (decoded.spec().rate(), decoded.num_planes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PCM WAV of `frames` frames at `rate`, `channels` wide, whose samples
    /// are a full-scale square so a decode can be checked for level as well as
    /// for length.
    fn wav(rate: u32, channels: u16, frames: usize) -> Vec<u8> {
        let data_len = frames * usize::from(channels) * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        let block = u32::from(channels) * 2;
        out.extend_from_slice(&(rate * block).to_le_bytes());
        out.extend_from_slice(&(block as u16).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for frame in 0..frames {
            let value = if frame % 2 == 0 { i16::MAX } else { i16::MIN };
            for _ in 0..channels {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn a_wav_decodes_to_mono_at_its_own_rate() {
        let decoded = decode(wav(44_100, 1, 128), Some("wav")).expect("a plain WAV must load");
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 128);
        let peak = decoded
            .samples
            .iter()
            .fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(
            peak > 0.9,
            "full scale should survive the decode, got {peak}"
        );
    }

    #[test]
    fn stereo_is_downmixed_rather_than_played_at_double_speed() {
        // Interleaved frames read as mono play twice as fast and an octave up,
        // which sounds like a broken sample rather than a bug — the same
        // failure `kit::decode_wav` already guards, arriving through the other
        // reader.
        let decoded = decode(wav(48_000, 2, 64), Some("wav")).expect("stereo must load");
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(
            decoded.samples.len(),
            64,
            "two channels are still one frame"
        );
    }

    #[test]
    fn the_extension_is_a_hint_and_the_bytes_decide() {
        // ⛔ A producer renames files. A WAV called `.mp3` must still load, or
        // the feature fails on a file every DAW opens.
        let decoded = decode(wav(44_100, 1, 64), Some("mp3")).expect("the bytes are a WAV");
        assert_eq!(decoded.samples.len(), 64);

        // And with no hint at all, which is what a file with no extension gives.
        assert!(decode(wav(44_100, 1, 64), None).is_ok());
    }

    #[test]
    fn something_that_is_not_audio_is_refused_with_a_reason() {
        let err = decode(b"this is a text file, not a sample".to_vec(), Some("wav")).unwrap_err();
        assert!(err.contains("not audio"), "{err}");

        // Empty, which is what a zero-byte file gives.
        assert!(decode(Vec::new(), Some("wav")).is_err());
    }

    #[test]
    fn a_silent_file_is_reported_rather_than_loaded() {
        // ⛔ **The TASK-131A failure through a different door.** A pad that
        // loads "successfully" and plays nothing is indistinguishable from a
        // broken generator, and the producer is the only one who can tell
        // "I picked the wrong file" from "the plugin is broken".
        let mut silent = wav(44_100, 1, 64);
        let data_start = silent.len() - 128;
        silent[data_start..].fill(0);

        let err = decode(silent, Some("wav")).unwrap_err();
        assert!(err.contains("silent"), "{err}");
    }

    #[test]
    fn a_sample_longer_than_a_pad_holds_is_refused_rather_than_truncated() {
        // ⛔ **The bound is on the decoded audio, not on the file** — a
        // compressed format is a compression ratio, so file size says nothing
        // about how much audio comes out. Truncating instead would hand back a
        // pad that is quietly not the sample the producer picked.
        let frames = (MAX_SECONDS as usize + 1) * 8_000;
        let err = decode(wav(8_000, 1, frames), Some("wav")).unwrap_err();
        assert!(err.contains("longer than"), "{err}");

        // ...and one just inside the bound still loads, so the limit does not
        // bind on the thing the feature is for.
        let ok = decode(wav(8_000, 1, 8_000), Some("wav"));
        assert!(ok.is_ok(), "a one-second sample must load: {ok:?}");
    }

    #[test]
    fn a_rate_no_sampler_should_believe_is_refused() {
        let err = decode(wav(1_000, 1, 64), Some("wav")).unwrap_err();
        assert!(err.contains("sample rate"), "{err}");
    }

    #[test]
    fn a_missing_file_says_so_rather_than_panicking() {
        let err = decode_file(std::path::Path::new("./no-such-one-shot.wav")).unwrap_err();
        assert!(err.contains("could not open"), "{err}");
    }

    #[test]
    fn a_file_past_the_size_bound_is_refused_before_it_is_read() {
        // ⛔ The stat, not the read. Checking after reading is a check the
        // allocation has already happened for — which on a large enough file is
        // the host running out of memory rather than an error message.
        let dir = std::env::temp_dir().join("fmm-oneshot-size-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("huge.wav");
        let file = std::fs::File::create(&path).expect("the test file must be creatable");
        // Sparse where the filesystem allows it, so this costs no real disk.
        file.set_len(MAX_FILE_BYTES + 1)
            .expect("length must be set");
        drop(file);

        let err = decode_file(&path).unwrap_err();
        assert!(err.contains("MB"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod nan_tests {
    use super::*;

    /// A float-PCM WAV whose data chunk mixes NaN with ordinary audio.
    fn float_wav_with_nan() -> Vec<u8> {
        let samples: [f32; 4] = [0.5, f32::NAN, -0.5, 0.25];
        let data_len = samples.len() * 4;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&44_100u32.to_le_bytes());
        out.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_file_carrying_nan_is_refused_rather_than_mixed_into_the_host() {
        // The guard here used to read a peak folded with `f32::max`, which is
        // documented to return the OTHER operand when one is NaN — so the fold
        // skipped every NaN, the peak came out finite, and the check could never
        // fire. The pad loaded and the sampler wrote NaN into the host's output
        // bus, which propagates through the DAW's whole signal path.
        let error = decode(float_wav_with_nan(), Some("wav"))
            .expect_err("a file carrying NaN must be refused");
        assert!(error.contains("not audio"), "{error}");
    }
}
