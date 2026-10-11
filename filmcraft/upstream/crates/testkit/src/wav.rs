//! Minimal WAV writer (IEEE float, interleaved) for handing generated signals to ffmpeg.

use std::path::Path;

/// Encode planar `f32` channels (equal lengths) as a 32-bit float WAV file.
pub fn encode_f32(channels: &[&[f32]], sample_rate: u32) -> Vec<u8> {
    let nch = channels.len().max(1) as u16;
    let frames = channels.first().map_or(0, |c| c.len());
    assert!(channels.iter().all(|c| c.len() == frames), "channels must have equal lengths");
    let data_len = (frames * nch as usize * 4) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes()); // WAVE_FORMAT_IEEE_FLOAT
    v.extend_from_slice(&nch.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * nch as u32 * 4).to_le_bytes());
    v.extend_from_slice(&(nch * 4).to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        for c in channels {
            v.extend_from_slice(&c[i].to_le_bytes());
        }
    }
    v
}

/// Write planar `f32` channels to `path` as a float WAV.
pub fn write_f32(path: &Path, channels: &[&[f32]], sample_rate: u32) -> std::io::Result<()> {
    std::fs::write(path, encode_f32(channels, sample_rate))
}

#[cfg(test)]
mod tests {
    #[test]
    fn header_sizes() {
        let a = [0.5f32, -0.5];
        let w = super::encode_f32(&[&a, &a], 48000);
        assert_eq!(w.len(), 44 + 16);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 16);
    }
}
