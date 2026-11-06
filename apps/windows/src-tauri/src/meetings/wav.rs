//! Cutting a recording's WAV back to its first `keep_ms` (trim, spec
//! 2026-11-05-trim-and-still-in-meeting-design.md). The recorder writes a canonical 44-byte
//! header (`RIFF`, `fmt `, `data`), so a trim drops the bytes past the cut and rewrites the two
//! sizes, as `pipeline::repair_wav_header` fills them in.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const HEADER: u64 = 44;

/// Shortens `path` to its first `keep_ms` of audio. Returns whether it cut anything: a file
/// already that short, or missing, is left alone. A header this doesn't recognize is an error,
/// so nothing is cut blind.
pub fn truncate_wav(path: &Path, keep_ms: i64) -> std::io::Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let length = file.metadata()?.len();
    let mut header = [0u8; HEADER as usize];
    if length < HEADER {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a WAV file"));
    }
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" || &header[12..16] != b"fmt " || &header[36..40] != b"data" {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a canonical WAV header"));
    }
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().unwrap()) as u64;
    let block_align = u16::from_le_bytes(header[32..34].try_into().unwrap()).max(1) as u64;
    if byte_rate == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "WAV header has no byte rate"));
    }
    let keep_bytes = (keep_ms.max(0) as u64 * byte_rate / 1000) / block_align * block_align;
    let target = HEADER + keep_bytes;
    if target >= length {
        return Ok(false);
    }
    file.set_len(target)?;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&((target - 8) as u32).to_le_bytes())?;
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&(keep_bytes as u32).to_le_bytes())?;
    file.sync_all()?;
    tracing::info!(path = %path.display(), keep_ms, bytes = target, "trimmed a recording");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(path: &Path, seconds: u32, sample_rate: u32, channels: u16) {
        let spec = hound::WavSpec { channels, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..seconds * sample_rate * channels as u32 {
            writer.write_sample((i % 100) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-wav-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_trimmed_wav_reads_back_at_its_new_length() {
        let d = dir("cut");
        let path = d.join("system-audio-1.wav");
        wav(&path, 10, 16_000, 1);
        assert!(truncate_wav(&path, 4_250).unwrap());
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 68_000, "4.25 s at 16 kHz");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 68_000 * 2);
        assert!(!truncate_wav(&path, 9_000).unwrap(), "already shorter: left alone");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cut_lands_on_a_whole_frame() {
        let d = dir("frame");
        let path = d.join("stereo.wav");
        wav(&path, 2, 48_000, 2);
        assert!(truncate_wav(&path, 1_001).unwrap());
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 48_048, "1.001 s of stereo frames");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_file_is_nothing_and_a_foreign_one_is_refused() {
        let d = dir("bad");
        assert!(!truncate_wav(&d.join("gone.wav"), 1_000).unwrap());
        let junk = d.join("junk.wav");
        std::fs::write(&junk, vec![7u8; 100]).unwrap();
        assert!(truncate_wav(&junk, 1).is_err());
        assert_eq!(std::fs::metadata(&junk).unwrap().len(), 100, "nothing cut blind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
