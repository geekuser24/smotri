//! Bounded size-targeted H.264 exports. Only accepted files reach the destination.
use crate::{AudioCodec, BitrateMode, ExportError, ExportSettings, Format, HardwareEncoding, Result};
use filmcraft_time::Tick;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

impl ExportSettings {
    pub(crate) fn validate_size_limit(&self) -> Result<()> {
        if let Some(limit) = self.target_size_bytes {
            if !(1_000_000..=100_000_000_000).contains(&limit) {
                return Err(ExportError::Unsupported("Size limit must be between 1 and 100000 MB (1 MB = 1000000 bytes)".into()));
            }
            if self.format != Format::H264 || self.output_file.is_some() {
                return Err(ExportError::Unsupported("Size-limited export requires H.264 and a regular output path".into()));
            }
        }
        Ok(())
    }

    /// Fit video and AAC audio into a decimal-byte budget. Called again when the range changes.
    pub fn fit_size_limit(&mut self, duration: Tick) -> Result<()> {
        self.validate_size_limit()?;
        let Some(limit) = self.target_size_bytes else { return Ok(()) };
        let seconds = duration.seconds();
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(ExportError::Unsupported("Size-limited export requires a positive duration".into()));
        }
        let total = (((limit as f64 * 0.96 - 32768.0) * 8.0 / seconds / 1000.0).floor()).min(1_000_000.0) as u32;
        let audio = if self.has_audio() {
            if total >= 800 {
                128
            } else if total >= 500 {
                96
            } else if total >= 200 {
                64
            } else {
                32
            }
        } else {
            0
        };
        let video = total.saturating_sub(audio);
        if video < 100 {
            return Err(ExportError::Unsupported("Size limit is too small for this duration; increase it or shorten the export".into()));
        }
        self.bitrate_kbps = video;
        self.max_bitrate_kbps = Some(video.saturating_mul(2));
        self.adaptive_bitrate = None;
        self.bitrate_mode = BitrateMode::Vbr2Pass;
        self.hardware_encoding = HardwareEncoding::Off;
        if audio > 0 {
            self.audio.codec = AudioCodec::Aac;
            self.audio.bitrate_kbps = audio;
            self.audio.channels = self.audio.channels.clamp(1, 2);
        }
        Ok(())
    }

    pub(crate) fn check_output_size(&self, actual: u64) -> Result<()> {
        if let Some(limit) = self.target_size_bytes
            && actual > limit
        {
            return Err(ExportError::SizeLimit { actual, limit });
        }
        Ok(())
    }

    pub(crate) fn lower_size_bitrate(&mut self, actual: u64, limit: u64) -> bool {
        if actual <= limit || actual == 0 {
            return false;
        }
        let next = ((self.bitrate_kbps as f64 * (limit as f64 * 0.94 / actual as f64)).floor()) as u32;
        if next < 100 || next >= self.bitrate_kbps {
            return false;
        }
        self.bitrate_kbps = next;
        self.max_bitrate_kbps = Some(next.saturating_mul(2));
        true
    }
}

/// Unique staging file in the destination directory; errors/cancellation remove it.
/// The writer is owned before this guard so it closes first on Windows too.
pub(crate) struct StagePath(PathBuf);
impl StagePath {
    pub(crate) fn create(destination: &str) -> Result<(std::fs::File, Self)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = Path::new(destination).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        for _ in 0..100 {
            let path = parent.join(format!(".smotri-export-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((file, Self(path))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(ExportError::Io(e.to_string())),
            }
        }
        Err(ExportError::Io("Cannot reserve an export staging file".into()))
    }
    pub(crate) fn publish(self, destination: &str) -> Result<()> {
        // rename is atomic on supported filesystems; if replacement is unsupported, leave the old file intact.
        std::fs::rename(&self.0, destination).map_err(|e| ExportError::Io(e.to_string()))
    }
}
impl Drop for StagePath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
