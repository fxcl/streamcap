//! Data models (Recording, RecordingFormatted, QualityInput).
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::time::Duration;
use time::OffsetDateTime;

use super::recording_status::RecordingStatus;

/// 画质输入
#[derive(Debug, Clone)]
pub enum QualityInput {
    Quality(super::video_quality::VideoQuality),
    Custom(String),
}

impl ToString for QualityInput {
    fn to_string(&self) -> String {
        match self {
            QualityInput::Quality(q) => q.description().to_string(),
            QualityInput::Custom(s) => s.clone(),
        }
    }
}

impl From<super::video_quality::VideoQuality> for QualityInput {
    fn from(q: super::video_quality::VideoQuality) -> Self {
        QualityInput::Quality(q)
    }
}

impl From<String> for QualityInput {
    fn from(s: String) -> Self {
        QualityInput::Custom(s)
    }
}

impl From<&str> for QualityInput {
    fn from(s: &str) -> Self {
        QualityInput::Custom(s.to_string())
    }
}

/// 录制任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub rec_id: String,
    pub url: String,
    pub quality: String,
    pub record_format: String,
    pub monitor_status: bool,
    pub segment_record: bool,
    pub segment_time: String,
    pub streamer_name: String,
    pub scheduled_recording: bool,
    pub scheduled_start_time: String,
    pub monitor_hours: String,
    pub recording_dir: String,
    pub enabled_message_push: bool,
    pub only_notify_no_record: bool,
    pub flv_use_direct_download: bool,
    pub video_bitrate: Option<u32>,
    pub platform: Option<String>,
    pub platform_key: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub last_duration: String,
}

/// 格式化后的录制信息 (运行时状态)
#[derive(Debug, Clone)]
pub struct RecordingFormatted {
    pub base: Recording,
    pub is_live: bool,
    pub is_recording: bool,
    pub is_checking: bool,
    pub status: RecordingStatus,
    pub status_info: String,
    pub cumulative_duration: Duration,
    pub last_duration: Duration,
    pub start_time: Option<OffsetDateTime>,
    pub live_title: Option<String>,
    pub speed: String,
    pub selected: bool,
    pub showed_checking_status: bool,
    pub force_stop: bool,
    pub stopping_in_progress: bool,
    pub manually_stopped: bool,
    pub notified_live_start: bool,
    pub notified_live_end: bool,
    pub detection_time: Option<time::Time>,
    pub loop_time_seconds: u64,
    pub use_proxy: bool,
    pub record_url: Option<String>,
    pub preview_url: Option<String>,
    pub scheduled_time_range: Option<Vec<String>>,
}

impl Default for RecordingFormatted {
    fn default() -> Self {
        Self {
            base: Recording {
                rec_id: String::new(),
                url: String::new(),
                quality: String::new(),
                record_format: "mp4".to_string(),
                monitor_status: false,
                segment_record: false,
                segment_time: "1800".to_string(),
                streamer_name: String::new(),
                scheduled_recording: false,
                scheduled_start_time: String::new(),
                monitor_hours: String::new(),
                recording_dir: String::new(),
                enabled_message_push: false,
                only_notify_no_record: false,
                flv_use_direct_download: false,
                video_bitrate: None,
                platform: None,
                platform_key: None,
                title: String::new(),
                display_title: String::new(),
                last_duration: String::new(),
            },
            is_live: false,
            is_recording: false,
            is_checking: false,
            status: RecordingStatus::Monitoring,
            status_info: RecordingStatus::Monitoring.description().to_string(),
            cumulative_duration: Duration::ZERO,
            last_duration: Duration::ZERO,
            start_time: None,
            live_title: None,
            speed: "0 KB/s".to_string(),
            selected: false,
            showed_checking_status: false,
            force_stop: false,
            stopping_in_progress: false,
            manually_stopped: false,
            notified_live_start: false,
            notified_live_end: false,
            detection_time: None,
            loop_time_seconds: 300,
            use_proxy: false,
            record_url: None,
            preview_url: None,
            scheduled_time_range: None,
        }
    }
}
