use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::recording::RecordingManager;

/// HTTP API 共享状态
#[derive(Clone)]
pub struct AppState {
    pub recording_manager: Arc<RwLock<RecordingManager>>,
    pub video_dir: PathBuf,
}

/// 录制任务响应
#[derive(Debug, Serialize)]
pub struct RecordingResponse {
    pub rec_id: String,
    pub url: String,
    pub streamer_name: String,
    pub title: String,
    pub quality: String,
    pub monitor_status: bool,
    pub is_recording: bool,
    pub status_info: String,
    pub speed: String,
    pub platform: Option<String>,
}

/// 录制任务列表响应
#[derive(Debug, Serialize)]
pub struct RecordingListResponse {
    pub total: usize,
    pub recordings: Vec<RecordingResponse>,
}

/// 添加录制请求 (对齐 Python Recording 模型)
#[derive(Debug, Deserialize)]
pub struct AddRecordingRequest {
    pub url: String,
    pub streamer_name: Option<String>,
    pub quality: Option<String>,
    pub record_format: Option<String>,
    pub monitor_status: Option<bool>,
    pub segment_record: Option<bool>,
    pub segment_time: Option<String>,
    pub recording_dir: Option<String>,
    pub platform: Option<String>,
    pub platform_key: Option<String>,
}

/// 更新录制请求
#[derive(Debug, Deserialize, Default)]
pub struct UpdateRecordingRequest {
    pub url: Option<String>,
    pub streamer_name: Option<String>,
    pub quality: Option<String>,
    pub monitor_status: Option<bool>,
    pub segment_record: Option<bool>,
}

/// 系统状态响应 (对齐 ControllerStats)
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_recordings: usize,
    pub active_recordings: usize,
    pub output_dir: String,
    pub disk_free_gb: f64,
    pub status_summary: Vec<StatusItem>,
}

#[derive(Debug, Serialize)]
pub struct StatusItem {
    pub rec_id: String,
    pub url: String,
    pub is_recording: bool,
}

/// 批量监控切换请求
#[derive(Debug, Deserialize)]
pub struct MonitorToggleRequest {
    pub enabled: bool,
}

/// 通用 API 消息
#[derive(Debug, Serialize)]
pub struct ApiMessage {
    pub message: String,
}

/// 错误响应
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}
