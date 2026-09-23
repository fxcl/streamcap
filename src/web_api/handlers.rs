use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tracing::{info, warn};

use crate::models::{Recording, RecordingFormatted, RecordingStatus};
use crate::recording::stream_recorder::RecorderConfig;

use super::models::*;

/// 构造录制响应
fn build_response(rec_id: &str, recording: &Recording, is_recording: bool) -> RecordingResponse {
    RecordingResponse {
        rec_id: rec_id.to_string(),
        url: recording.url.clone(),
        streamer_name: recording.streamer_name.clone(),
        title: recording.title.clone(),
        quality: recording.quality.clone(),
        monitor_status: recording.monitor_status,
        is_recording,
        status_info: if is_recording {
            "Recording".to_string()
        } else if recording.monitor_status {
            "Monitoring".to_string()
        } else {
            "Idle".to_string()
        },
        speed: "0 KB/s".to_string(),
        platform: recording.platform.clone(),
    }
}

/// GET /api/recordings — 列表所有录制任务
pub async fn list_recordings(
    State(state): State<Arc<AppState>>,
) -> Json<RecordingListResponse> {
    let mgr = state.recording_manager.read().await;
    let summary = mgr.get_status_summary().await;

    let recordings: Vec<RecordingResponse> = summary
        .into_iter()
        .map(|(rec_id, url, is_recording)| RecordingResponse {
            rec_id,
            url,
            streamer_name: String::new(),
            title: String::new(),
            quality: "OD".to_string(),
            monitor_status: true,
            is_recording,
            status_info: if is_recording {
                "Recording".to_string()
            } else {
                "Idle".to_string()
            },
            speed: "0 KB/s".to_string(),
            platform: None,
        })
        .collect();

    let total = recordings.len();
    Json(RecordingListResponse { total, recordings })
}

/// GET /api/recordings/:id — 获取单条录制
pub async fn get_recording(
    State(state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
) -> Result<Json<RecordingResponse>, ApiErrorResp> {
    let mgr = state.recording_manager.read().await;
    let summary = mgr.get_status_summary().await;

    if let Some((id, url, is_recording)) = summary.into_iter().find(|(id, _, _)| id == &rec_id) {
        Ok(Json(RecordingResponse {
            rec_id: id,
            url,
            streamer_name: String::new(),
            title: String::new(),
            quality: "OD".to_string(),
            monitor_status: true,
            is_recording,
            status_info: "Found".to_string(),
            speed: "0 KB/s".to_string(),
            platform: None,
        }))
    } else {
        Err(ApiErrorResp::not_found(format!("Recording {} not found", rec_id)))
    }
}

/// POST /api/recordings — 添加新录制
pub async fn add_recording(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddRecordingRequest>,
) -> Result<(StatusCode, Json<RecordingResponse>), ApiErrorResp> {
    if req.url.is_empty() {
        return Err(ApiErrorResp::bad_request("URL is required"));
    }

    let rec_id = new_uuid();
    let recording = Recording {
        rec_id: rec_id.clone(),
        url: req.url.clone(),
        quality: req.quality.unwrap_or_else(|| "OD".to_string()),
        record_format: req.record_format.unwrap_or_else(|| "mp4".to_string()),
        monitor_status: req.monitor_status.unwrap_or(true),
        segment_record: req.segment_record.unwrap_or(false),
        segment_time: req.segment_time.unwrap_or_else(|| "1800".to_string()),
        streamer_name: req.streamer_name.unwrap_or_default(),
        scheduled_recording: false,
        scheduled_start_time: String::new(),
        monitor_hours: String::new(),
        recording_dir: req.recording_dir.unwrap_or_default(),
        enabled_message_push: false,
        only_notify_no_record: false,
        flv_use_direct_download: false,
        video_bitrate: None,
        platform: req.platform,
        platform_key: req.platform_key,
        title: String::new(),
        display_title: String::new(),
        last_duration: String::new(),
    };

    let formatted = RecordingFormatted {
        base: recording.clone(),
        status: RecordingStatus::Monitoring,
        status_info: RecordingStatus::Monitoring.description().to_string(),
        loop_time_seconds: 300,
        ..Default::default()
    };

    {
        let mgr = state.recording_manager.read().await;
        mgr.add_recording(formatted).await;
    }

    info!("Recording added: {} -> {}", rec_id, req.url);

    let resp = build_response(&rec_id, &recording, false);
    Ok((StatusCode::CREATED, Json(resp)))
}

/// PUT /api/recordings/:id — 更新录制
pub async fn update_recording(
    State(_state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
    Json(req): Json<UpdateRecordingRequest>,
) -> Result<Json<ApiMessage>, ApiErrorResp> {
    if req.url.is_none()
        && req.monitor_status.is_none()
        && req.quality.is_none()
        && req.segment_record.is_none()
        && req.streamer_name.is_none()
    {
        return Err(ApiErrorResp::bad_request("No fields to update"));
    }

    warn!(
        "update_recording({}): partial update not yet fully wired",
        rec_id
    );
    Ok(Json(ApiMessage {
        message: format!("Recording {} update queued", rec_id),
    }))
}

/// DELETE /api/recordings/:id — 删除录制
pub async fn delete_recording(
    State(state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
) -> Result<Json<ApiMessage>, ApiErrorResp> {
    {
        let mgr = state.recording_manager.read().await;
        mgr.remove_recording(&rec_id).await;
    }
    info!("Recording deleted: {}", rec_id);
    Ok(Json(ApiMessage {
        message: format!("Recording {} deleted", rec_id),
    }))
}

/// POST /api/recordings/:id/start — 启动录制
pub async fn start_recording(
    State(state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
) -> Result<Json<ApiMessage>, ApiErrorResp> {
    let config = {
        let mgr = state.recording_manager.read().await;
        let summary = mgr.get_status_summary().await;
        let Some((_, url, is_recording)) = summary.into_iter().find(|(id, _, _)| id == &rec_id) else {
            return Err(ApiErrorResp::not_found(format!("Recording {} not found", rec_id)));
        };

        if is_recording {
            return Err(ApiErrorResp::bad_request("Already recording"));
        }

        RecorderConfig {
            record_url: url,
            output_path: format!("./recordings/{}.mp4", &rec_id[..8.min(rec_id.len())]),
            format: crate::format::OutputFormat::Mp4,
            segment_record: false,
            segment_time: 1800,
            is_overseas: false,
            headers: None,
            proxy: None,
            use_direct_download: false,
            input_options: None,
            max_retries: 3,
        }
    };

    {
        let mgr = state.recording_manager.read().await;
        match mgr.start_recording(&rec_id, config).await {
            Ok(()) => {
                info!("Recording started via API: {}", rec_id);
                Ok(Json(ApiMessage {
                    message: format!("Recording started: {}", rec_id),
                }))
            }
            Err(e) => Err(ApiErrorResp::internal(format!("Failed: {}", e))),
        }
    }
}

/// POST /api/recordings/:id/stop — 停止录制
pub async fn stop_recording(
    State(state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
) -> Result<Json<ApiMessage>, ApiErrorResp> {
    {
        let mgr = state.recording_manager.read().await;
        mgr.stop_recording(&rec_id).await;
    }
    info!("Recording stopped via API: {}", rec_id);
    Ok(Json(ApiMessage {
        message: format!("Recording stopped: {}", rec_id),
    }))
}

/// POST /api/recordings/:id/monitor — 切换监控状态
pub async fn toggle_monitor(
    State(state): State<Arc<AppState>>,
    Path(rec_id): Path<String>,
    Json(req): Json<MonitorToggleRequest>,
) -> Result<Json<ApiMessage>, ApiErrorResp> {
    let mgr = state.recording_manager.read().await;

    let result = if req.enabled {
        mgr.start_monitoring(&rec_id).await
    } else {
        mgr.stop_monitoring(&rec_id).await
    };

    match result {
        Ok(()) => Ok(Json(ApiMessage {
            message: format!(
                "Monitor {}: {}",
                rec_id,
                if req.enabled { "enabled" } else { "disabled" }
            ),
        })),
        Err(e) => Err(ApiErrorResp::internal(format!(
            "Recording {}: {}",
            rec_id, e
        ))),
    }
}

/// POST /api/recordings/start_all — 启动全部监控
pub async fn start_all(State(state): State<Arc<AppState>>) -> Json<ApiMessage> {
    {
        let mgr = state.recording_manager.read().await;
        mgr.start_all_monitoring().await;
    }
    Json(ApiMessage {
        message: "All monitoring started".to_string(),
    })
}

/// POST /api/recordings/stop_all — 停止全部监控
pub async fn stop_all(State(state): State<Arc<AppState>>) -> Json<ApiMessage> {
    {
        let mgr = state.recording_manager.read().await;
        mgr.stop_all_monitoring().await;
    }
    Json(ApiMessage {
        message: "All monitoring stopped".to_string(),
    })
}

/// GET /api/stats — 系统状态
pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<StatsResponse> {
    let mgr = state.recording_manager.read().await;

    let active = mgr.get_active_recording_count().await;
    let summary = mgr.get_status_summary().await;
    let total = summary.len();

    let status_summary: Vec<StatusItem> = summary
        .into_iter()
        .map(|(rec_id, url, is_recording)| StatusItem {
            rec_id,
            url,
            is_recording,
        })
        .collect();

    let disk_free_gb = crate::utils::check_disk_free_space(std::path::Path::new(
        state.video_dir.to_str().unwrap_or("."),
    ));

    Json(StatsResponse {
        total_recordings: total,
        active_recordings: active,
        output_dir: state.video_dir.to_string_lossy().to_string(),
        disk_free_gb,
        status_summary,
    })
}

/// GET /api/videos/*path — 视频文件服务
pub async fn serve_video(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    use axum::http::header;

    let file_path = state.video_dir.join(&path);

    // 安全路径检查: 防止 path traversal
    let canonical_video_dir = match std::fs::canonicalize(&state.video_dir) {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Invalid video dir").into_response();
        }
    };

    let canonical_file = match std::fs::canonicalize(&file_path) {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "File not found").into_response();
        }
    };

    if !canonical_file.starts_with(&canonical_video_dir) {
        return (StatusCode::FORBIDDEN, "Access denied").into_response();
    }

    match tokio::fs::read(&canonical_file).await {
        Ok(content) => {
            let ext = canonical_file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let content_type = match ext {
                "mp4" | "m4v" => "video/mp4",
                "webm" => "video/webm",
                "ts" => "video/mp2t",
                "flv" => "video/x-flv",
                "mkv" => "video/x-matroska",
                "m3u8" => "application/vnd.apple.mpegurl",
                "srt" | "vtt" => "text/plain",
                _ => "application/octet-stream",
            };

            ([(header::CONTENT_TYPE, content_type)], content).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}

// ─── 错误响应 ───────────────────────────────────────────────

pub struct ApiErrorResp {
    status: StatusCode,
    message: String,
}

impl ApiErrorResp {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }

    fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiErrorResp {
    fn into_response(self) -> Response {
        let body = Json(ApiError { error: self.message });
        (self.status, body).into_response()
    }
}

// ─── 简单的 UUID 生成 ───────────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};
static UUID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn new_uuid() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let seq = UUID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:016x}{:016x}", ts, seq)
}
