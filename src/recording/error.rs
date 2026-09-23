#![allow(dead_code)]
//! Error types for recording operations.


use thiserror::Error;

#[derive(Error, Debug)]
pub enum RecordingError {
    #[error("FFmpeg error: {0}")]
    FFmpegError(String),

    #[error("Format error: {0}")]
    FormatError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Recording stopped by user")]
    StoppedByUser,

    #[error("Recording already in progress")]
    AlreadyRecording,

    #[error("Recording was interrupted by callback")]
    StreamInterrupted,

    #[error("Open input error: {0}")]
    OpenInputError(String),

    #[error("Find stream info error: {0}")]
    FindStreamInfoError(String),

    #[error("Write header error: {0}")]
    WriteHeaderError(String),

    #[error("Write frame error: {0}")]
    WriteFrameError(String),

    #[error("Write trailer error: {0}")]
    WriteTrailerError(String),

    #[error("No stream found")]
    NoStreamFound,

    #[error("Timeout error: {0}")]
    TimeoutError(String),

    #[error("Disk space error: {0}")]
    DiskSpaceError(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<rsmpeg::error::RsmpegError> for RecordingError {
    fn from(e: rsmpeg::error::RsmpegError) -> Self {
        use rsmpeg::error::RsmpegError::*;
        match e {
            OpenInputError(_) => RecordingError::OpenInputError(e.to_string()),
            FindStreamInfoError(_) => RecordingError::FindStreamInfoError(e.to_string()),
            // 检查是否是 AVERROR_EXIT (interrupt_callback 触发的退出)
            AVError(code) if code == rsmpeg::shared::AVERROR_EXIT => {
                RecordingError::StreamInterrupted
            }
            AVError(_) => RecordingError::FFmpegError(e.to_string()),
            _ => RecordingError::FFmpegError(e.to_string()),
        }
    }
}
