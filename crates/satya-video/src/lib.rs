//! Wrap recovered H.264 NAL units into an MP4 container.
//!
//! We shell out to the `ffmpeg` binary rather than linking libav*.
//! This keeps the build dependency-free and works on air-gapped
//! forensic workstations where a packaged ffmpeg is available.

use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VideoError {
    #[error("ffmpeg not found in PATH")]
    FfmpegMissing,
    #[error("ffmpeg failed: {0}")]
    FfmpegFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convert a raw H.264 (Annex B) file to MP4.
pub fn h264_to_mp4(raw_h264_path: &Path, out_mp4: &Path) -> Result<(), VideoError> {
    let status = Command::new("ffmpeg")
    .args(["-y", "-fflags", "+genpts", "-r", "25"])
    .arg("-i")
    .arg(raw_h264_path)
    .args(["-c:v", "copy", "-bsf:v", "h264_mp4toannexb"])
    .args(["-movflags", "+faststart"])
    .arg(out_mp4)
    .output()
    .map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            VideoError::FfmpegMissing
        } else {
            VideoError::Io(e)
        }
    })?;

    if !status.status.success() {
        return Err(VideoError::FfmpegFailed(
            String::from_utf8_lossy(&status.stderr).into_owned(),
        ));
    }
    Ok(())
}
