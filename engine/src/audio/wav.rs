//! Minimal 16-bit PCM WAV writer (used to export the generated audio for inspection).

use std::io::Write;
use std::path::Path;

/// Encodes interleaved `samples` (`channels` per frame, nominal range `-1..=1`, clamped) as a
/// 16-bit PCM RIFF/WAVE file.
pub fn encode_wav(sample_rate: u32, channels: u16, samples: &[f32]) -> Vec<u8> {
    let channels = channels.max(1);
    let data_len = (samples.len() * 2) as u32;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        out.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes());
    }
    out
}

pub fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[f32]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(&encode_wav(sample_rate, channels, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_consistent() {
        let bytes = encode_wav(44_100, 2, &[0.0, 1.0, -1.0, 2.0]);
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), 36 + 8);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]), 8);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 32767);
        assert_eq!(i16::from_le_bytes([bytes[50], bytes[51]]), 32767); // clamped
    }
}
