#![allow(dead_code)]
//! Recording task manager (add/remove/start/stop, periodic checking).


use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::error::RecordingError;
use super::stream_recorder::{RecorderConfig, StreamRecorder};
use crate::models::RecordingFormatted;

/// 录制条目 - 维护一条录制任务的全生命周期
pub struct RecordingEntry {
    pub recording: RecordingFormatted,
    pub recorder: StreamRecorder,
    pub last_check_time: Instant,
    pub consecutive_failures: u32,
}

impl RecordingEntry {
    pub fn new(recording: RecordingFormatted) -> Self {
        Self {
            recording,
            recorder: StreamRecorder::new(),
            last_check_time: Instant::now(),
            consecutive_failures: 0,
        }
    }

    pub async fn is_recording(&self) -> bool {
        self.recorder.is_recording().await
    }

    pub fn mark_checked(&mut self) {
        self.last_check_time = Instant::now();
    }
}

/// 录制管理器 - 对应 Python RecordingManager
pub struct RecordingManager {
    entries: Arc<RwLock<HashMap<String, RecordingEntry>>>,
    periodic_task: Arc<RwLock<Option<JoinHandle<()>>>>,
    check_interval_secs: Arc<RwLock<u64>>,
    is_running: Arc<RwLock<bool>>,
    convert_to_mp4: Arc<RwLock<bool>>,
    delete_original: Arc<RwLock<bool>>,
}

impl Default for RecordingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingManager {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            periodic_task: Arc::new(RwLock::new(None)),
            check_interval_secs: Arc::new(RwLock::new(180)),
            is_running: Arc::new(RwLock::new(false)),
            convert_to_mp4: Arc::new(RwLock::new(false)),
            delete_original: Arc::new(RwLock::new(true)),
        }
    }

    /// 注册录制任务 (对应 Python: manager.add_recording via UI)
    pub async fn add_recording(&self, recording: RecordingFormatted) {
        let mut entries = self.entries.write().await;
        let rec_id = recording.base.rec_id.clone();
        info!("Registering recording: {} ({})", recording.base.url, rec_id);
        entries.insert(rec_id, RecordingEntry::new(recording));
    }

    /// 批量注册录制任务
    pub async fn add_recordings(&self, recordings: Vec<RecordingFormatted>) {
        let mut entries = self.entries.write().await;
        for rec in recordings {
            let rec_id = rec.base.rec_id.clone();
            info!("Registering recording: {} ({})", rec.base.url, rec_id);
            entries.insert(rec_id, RecordingEntry::new(rec));
        }
    }

    /// 取消注册录制任务
    pub async fn remove_recording(&self, rec_id: &str) {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.remove(rec_id) {
            if entry.is_recording().await {
                entry.recorder.stop().await;
            }
            info!("Removed recording: {}", rec_id);
        }
    }

    /// 启动单个录制的监控 (对应 Python start_monitor_recording)
    pub async fn start_monitoring(&self, rec_id: &str) -> Result<(), RecordingError> {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(rec_id) {
            entry.recording.base.monitor_status = true;
            info!("Started monitoring: {}", rec_id);
            Ok(())
        } else {
            Err(RecordingError::Unknown(format!(
                "Recording not found: {}",
                rec_id
            )))
        }
    }

    /// 停止单个录制监控 (对应 Python stop_monitor_recording)
    pub async fn stop_monitoring(&self, rec_id: &str) -> Result<(), RecordingError> {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.get_mut(rec_id) {
            entry.recording.base.monitor_status = false;
            entry.recorder.stop().await;
            info!("Stopped monitoring: {}", rec_id);
            Ok(())
        } else {
            Err(RecordingError::Unknown(format!(
                "Recording not found: {}",
                rec_id
            )))
        }
    }

    /// 启动所有监控 (对应 Python start_monitor_recordings)
    pub async fn start_all_monitoring(&self) {
        let rec_ids: Vec<String> = {
            let entries = self.entries.read().await;
            entries.keys().cloned().collect()
        };
        for rec_id in &rec_ids {
            let _ = self.start_monitoring(rec_id).await;
        }
    }

    /// 停止所有监控 (对应 Python stop_monitor_recordings)
    pub async fn stop_all_monitoring(&self) {
        let rec_ids: Vec<String> = {
            let entries = self.entries.read().await;
            entries.keys().cloned().collect()
        };
        for rec_id in &rec_ids {
            let _ = self.stop_monitoring(rec_id).await;
        }
    }

    /// 启动周期性直播检测 (对应 Python setup_periodic_live_check)
    pub async fn start_periodic_check(&self, interval_secs: u64) {
        {
            let mut running = self.is_running.write().await;
            if *running {
                warn!("Periodic check already running");
                return;
            }
            *running = true;
        }

        {
            let mut interval = self.check_interval_secs.write().await;
            *interval = interval_secs;
        }

        let entries = Arc::clone(&self.entries);
        let is_running = Arc::clone(&self.is_running);
        let check_interval = Arc::clone(&self.check_interval_secs);

        let handle = tokio::spawn(async move {
            info!("Periodic live check started");
            loop {
                let interval = *check_interval.read().await;
                tokio::time::sleep(Duration::from_secs(interval)).await;

                if !*is_running.read().await {
                    info!("Periodic check stopped");
                    break;
                }

                Self::check_all_live_status(&entries).await;
            }
        });

        let mut task = self.periodic_task.write().await;
        if let Some(old) = task.take() {
            old.abort();
        }
        *task = Some(handle);
        info!("Periodic live check started with interval: {}s", interval_secs);
    }

    /// 停止周期性直播检测
    pub async fn stop_periodic_check(&self) {
        {
            let mut running = self.is_running.write().await;
            *running = false;
        }
        let mut task = self.periodic_task.write().await;
        if let Some(handle) = task.take() {
            handle.abort();
            info!("Periodic live check stopped");
        }
    }

    /// 检查所有监控中源的直播状态
    async fn check_all_live_status(_entries: &Arc<RwLock<HashMap<String, RecordingEntry>>>) {
        // TODO: 调用平台 handler 检查直播状态
        // 需要 async handler → 待平台解析模块完成后接入
        info!("Checking live status for all monitored recordings...");
    }

    /// 手动启动一条录制
    pub async fn start_recording(
        &self,
        rec_id: &str,
        config: RecorderConfig,
    ) -> Result<(), RecordingError> {
        let entries = self.entries.read().await;
        if let Some(entry) = entries.get(rec_id) {
            let recorder = &entry.recorder;
            recorder.start(config).await?;
            info!("Started recording: {}", rec_id);
            Ok(())
        } else {
            Err(RecordingError::Unknown(format!(
                "Recording not found: {}",
                rec_id
            )))
        }
    }

    /// 手动停止一条录制
    pub async fn stop_recording(&self, rec_id: &str) {
        let entries = self.entries.read().await;
        if let Some(entry) = entries.get(rec_id) {
            entry.recorder.stop().await;
            info!("Stopped recording: {}", rec_id);
        }
    }

    /// 配置转码参数
    pub async fn set_post_processing(&self, convert_to_mp4: bool, delete_original: bool) {
        *self.convert_to_mp4.write().await = convert_to_mp4;
        *self.delete_original.write().await = delete_original;
    }

    /// 获取正在录制的任务数
    pub async fn get_active_recording_count(&self) -> usize {
        let entries = self.entries.read().await;
        let mut count = 0;
        for entry in entries.values() {
            if entry.is_recording().await {
                count += 1;
            }
        }
        count
    }

    /// 获取所有录制条目的状态快照
    pub async fn get_status_summary(&self) -> Vec<(String, String, bool)> {
        let entries = self.entries.read().await;
        let mut result = Vec::new();
        for (id, entry) in entries.iter() {
            let recording = entry.is_recording().await;
            result.push((id.clone(), entry.recording.base.url.clone(), recording));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Recording, RecordingFormatted, RecordingStatus};

    fn mock_recording_formatted(rec_id: &str, url: &str) -> RecordingFormatted {
        RecordingFormatted {
            base: Recording {
                rec_id: rec_id.to_string(),
                url: url.to_string(),
                quality: "OD".to_string(),
                record_format: "mp4".to_string(),
                monitor_status: true,
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
            cumulative_duration: std::time::Duration::ZERO,
            last_duration: std::time::Duration::ZERO,
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

    #[test]
    fn test_new_manager() {
        let mgr = RecordingManager::new();
        assert!(!*mgr.is_running.blocking_read());
    }

    #[test]
    fn test_add_and_query_recording() {
        let mgr = RecordingManager::new();
        let rec = mock_recording_formatted("1", "https://example.com/live");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            mgr.add_recording(rec).await;
            let summary = mgr.get_status_summary().await;
            assert_eq!(summary.len(), 1);
            assert_eq!(summary[0].0, "1");
        });
    }
}
