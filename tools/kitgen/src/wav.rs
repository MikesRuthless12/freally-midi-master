//! A minimal 24-bit PCM WAV writer.
//!
//! Hand-rolled rather than pulled from a crate: the format needed here is one
//! fixed shape (44.1 kHz, mono, 24-bit), the writer is forty lines, and a
//! dependency in a tool that generates shipped assets is a dependency in the
//! supply chain of those assets.

use std::io::{self, Write};

pub const SAMPLE_RATE: u32 = 44_100;
const BITS_PER_SAMPLE: u16 = 24;
const CHANNELS: u16 = 1;

/// Write mono f32 samples in [-1.0, 1.0] as a 24-bit PCM WAV.
pub fn write_wav(mut out: impl Write, samples: &[f32]) -> io::Result<()> {
    let bytes_per_sample = u32::from(BITS_PER_SAMPLE / 8);
    let data_len = samples.len() as u32 * bytes_per_sample * u32::from(CHANNELS);
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * bytes_per_sample;
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVE")?;

    out.write_all(b"fmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // PCM chunk size
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&CHANNELS.to_le_bytes())?;
    out.write_all(&SAMPLE_RATE.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;

    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;

    for &s in samples {
        // Clamp before converting: a sample past full scale wraps rather than
        // clipping once it is an integer, which sounds like a click, not like
        // distortion.
        let clamped = s.clamp(-1.0, 1.0);
        let scaled = (clamped * 8_388_607.0) as i32;
        out.write_all(&scaled.to_le_bytes()[..3])?;
    }

    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_describes_the_data_that_follows() {
        let mut buf = Vec::new();
        write_wav(&mut buf, &[0.0; 10]).unwrap();

        assert_eq!(&buf[0..4], b"RIFF");
        assert_eq!(&buf[8..12], b"WAVE");
        assert_eq!(&buf[12..16], b"fmt ");
        assert_eq!(&buf[36..40], b"data");

        // 44-byte header + 10 samples * 3 bytes.
        assert_eq!(buf.len(), 44 + 30);

        let data_len = u32::from_le_bytes(buf[40..44].try_into().unwrap());
        assert_eq!(data_len, 30);
        let riff_len = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        assert_eq!(riff_len as usize, buf.len() - 8);
    }

    #[test]
    fn full_scale_does_not_wrap() {
        let mut buf = Vec::new();
        // 2.0 is past full scale; it must clip, not wrap to a large negative.
        write_wav(&mut buf, &[2.0, -2.0]).unwrap();
        let first = i32::from_le_bytes([buf[44], buf[45], buf[46], 0]);
        assert_eq!(
            first, 8_388_607,
            "positive overload must clip to full scale"
        );
    }

    #[test]
    fn silence_is_actually_silent() {
        let mut buf = Vec::new();
        write_wav(&mut buf, &[0.0; 4]).unwrap();
        assert!(buf[44..].iter().all(|b| *b == 0));
    }
}
