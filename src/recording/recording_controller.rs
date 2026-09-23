#![allow(dead_code)]
//! High-level recording orchestrator (disk policy, speed tracking, live checking).


use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::input_options::{InputOptions, StreamSelection, select_stream_source};
use super::recording_manager::RecordingManager;
use super::stream_recorder::RecorderConfig;
use crate::models::RecordingFormatted;

/// 直播状态回调 trait - 用于解耦平台解析层
///
/// 对应 Python stream_manager 中的:
/// `platform_handlers.get_platform_handler(...)` → `fetch_stream()` → `StreamData`
///
/// Rust 版本通过 trait 抽象, 允许测试时不依赖平台 handlers
#[async_trait::async_trait]
pub trait LiveStatusChecker: Send + Sync {
    /// 检查一条录制源的直播状态
    ///
    /// 返回 Some(StreamData) 表示已开播, None 表示未开播
    async fn check_live_status(
        &self,
        url: &str,
        platform_key: &str,
        quality: &str,
        proxy: Option<&str>,
    ) -> Option<StreamInfo>;
}

/// 简化的直播信息 - 对应 Python StreamData (部分字段)
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub anchor_name: String,
    pub platform_name: String,
    pub title: String,
    pub flv_url: Option<String>,
    pub record_url: String,
    pub m3u8_url: Option<String>,
    pub is_live: bool,
}

impl Default for StreamInfo {
    fn default() -> Self {
        Self {
            anchor_name: String::new(),
            platform_name: String::new(),
            title: String::new(),
            flv_url: None,
            record_url: String::new(),
            m3u8_url: None,
            is_live: false,
        }
    }
}

/// 磁盘空间阈值配置
#[derive(Debug, Clone)]
pub struct DiskSpacePolicy {
    /// 低于此值 (GB) 时停止所有录制
    pub min_free_gb: f64,
    /// 录制期间检查间隔 (秒)
    pub check_interval_secs: u64,
}

impl Default for DiskSpacePolicy {
    fn default() -> Self {
        Self {
            min_free_gb: 1.0,
            check_interval_secs: 60,
        }
    }
}

/// 录制速度跟踪
#[derive(Debug, Clone, Default)]
pub struct SpeedTracker {
    pub bytes_per_sec: u64,
    pub cumulative_bytes: u64,
    pub cumulative_duration: Duration,
    pub last_update: Option<Instant>,
}

impl SpeedTracker {
    /// 更新速度和累计值
    pub fn update(&mut self, bytes_delta: u64, duration_delta: Duration) {
        self.cumulative_bytes += bytes_delta;
        self.cumulative_duration += duration_delta;

        if duration_delta.as_secs() > 0 {
            self.bytes_per_sec = bytes_delta / duration_delta.as_secs().max(1);
        }
        self.last_update = Some(Instant::now());
    }

    /// 人类可读速度字符串 (如 "1.2 MB/s")
    pub fn speed_string(&self) -> String {
        let bps = self.bytes_per_sec;
        if bps >= 1024 * 1024 {
            format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
        } else if bps >= 1024 {
            format!("{:.1} KB/s", bps as f64 / 1024.0)
        } else {
            format!("{} B/s", bps)
        }
    }

    /// 人类可读累计时长
    pub fn cumulative_duration_string(&self) -> String {
        let secs = self.cumulative_duration.as_secs();
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    }
}

/// 录制控制器 - 全局最高层组件
///
/// 对应 Python 中最上层交互:
/// - services.recording_manager
/// - stream_manager._record_thread (调度录制启停)
///
/// 协调:
/// ├─ RecordingManager (多源管理)
/// ├─ DiskSpacePolicy (低磁盘空间 → 自动停止)
/// ├─ SpeedTracker (录制速度跟踪)
/// ├─ LiveStatusChecker trait (平台状态检查, 可替换)
/// └─ PeriodicMonitor (定时任务)
pub struct RecordingController {
    manager: Arc<RecordingManager>,
    disk_policy: Arc<RwLock<DiskSpacePolicy>>,
    speed_trackers: Arc<RwLock<HashMap<String, SpeedTracker>>>,
    monitor_task: Arc<RwLock<Option<JoinHandle<()>>>>,
    is_monitoring: Arc<RwLock<bool>>,
    output_dir: Arc<RwLock<String>>,
    /// 平台 status checker (由外部注入)
    live_checker: Arc<RwLock<Option<Arc<dyn LiveStatusChecker>>>>,
    /// 磁盘监视 task
    disk_monitor_task: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl Default for RecordingController {
    fn default() -> Self {
        Self::new("./recordings")
    }
}

impl RecordingController {
    pub fn new(output_dir: impl Into<String>) -> Self {
        Self {
            manager: Arc::new(RecordingManager::new()),
            disk_policy: Arc::new(RwLock::new(DiskSpacePolicy::default())),
            speed_trackers: Arc::new(RwLock::new(HashMap::new())),
            monitor_task: Arc::new(RwLock::new(None)),
            is_monitoring: Arc::new(RwLock::new(false)),
            output_dir: Arc::new(RwLock::new(output_dir.into())),
            live_checker: Arc::new(RwLock::new(None)),
            disk_monitor_task: Arc::new(RwLock::new(None)),
        }
    }

    /// 注入平台直播状态检查器
    pub async fn set_live_checker(&self, checker: Arc<dyn LiveStatusChecker>) {
        let mut c = self.live_checker.write().await;
        *c = Some(checker);
        info!("Live status checker injected");
    }

    /// 设置磁盘空间策略
    pub async fn set_disk_policy(&self, policy: DiskSpacePolicy) {
        let min = policy.min_free_gb;
        *self.disk_policy.write().await = policy;
        info!("Disk policy set: min_free_gb={}", min);
    }

    /// 注册录制任务
    pub async fn add_recording(&self, recording: RecordingFormatted) {
        self.manager.add_recording(recording).await;
    }

    /// 批量注册录制任务
    pub async fn add_recordings(&self, recordings: Vec<RecordingFormatted>) {
        self.manager.add_recordings(recordings).await;
    }

    /// 启动全局监控 + 磁盘监控 + 自动录制调度
    ///
    /// 对应 Python services.start_service:
    /// - 启动 periodic_live_check
    /// - 启动录制调度循环
    /// - 启动磁盘检查
    pub async fn start_service(&self, check_interval_secs: u64) {
        // 启动周期性直播检测
        self.manager.start_periodic_check(check_interval_secs).await;

        // 启动磁盘空间监控
        self.start_disk_monitor().await;

        // 启动监控调度循环
        self.start_auto_record_scheduler(check_interval_secs).await;

        info!("Recording service started");
    }

    /// 停止全局服务
    pub async fn stop_service(&self) {
        // 停止监控调度
        {
            let mut running = self.is_monitoring.write().await;
            *running = false;
        }
        {
            let mut task = self.monitor_task.write().await;
            if let Some(h) = task.take() {
                h.abort();
            }
        }

        // 停止磁盘监控
        {
            let mut task = self.disk_monitor_task.write().await;
            if let Some(h) = task.take() {
                h.abort();
            }
        }

        // 停止周期性检查 + 所有录制
        self.manager.stop_periodic_check().await;
        self.manager.stop_all_monitoring().await;

        info!("Recording service stopped");
    }

    /// 启动磁盘空间监控 - 低空间时停止录制
    async fn start_disk_monitor(&self) {
        let policy = Arc::clone(&self.disk_policy);
        let output_dir = Arc::clone(&self.output_dir);
        let manager = Arc::clone(&self.manager);
        let is_monitoring = Arc::clone(&self.is_monitoring);

        let handle = tokio::spawn(async move {
            loop {
                let policy_val = policy.read().await.clone();
                tokio::time::sleep(Duration::from_secs(policy_val.check_interval_secs)).await;

                if !*is_monitoring.read().await {
                    break;
                }

                let output = output_dir.read().await.clone();
                let free_gb = crate::utils::check_disk_free_space(Path::new(&output));

                if free_gb < policy_val.min_free_gb {
                    warn!(
                        "LOW DISK SPACE: {:.2} GB free (threshold: {:.2} GB), stopping all recordings",
                        free_gb, policy_val.min_free_gb
                    );
                    manager.stop_all_monitoring().await;
                    // TODO: 发送通知
                }
            }
        });

        let mut task = self.disk_monitor_task.write().await;
        *task = Some(handle);
    }

    /// 自动录制调度器 - 定期扫描, 开播的启动录制, 停播的停止录制
    ///
    /// 对应 Python setup_periodic_live_check + _record_when_live 逻辑:
    /// 1. 遍历所有监控中录制条目
    /// 2. 调用 live_checker.check_live_status() 检测直播状态
    /// 3. 开播 + 未录制 → 构建 RecorderConfig → 启动录制
    /// 4. 下播 + 正在录制 → 停止录制 + 后置转码
    async fn start_auto_record_scheduler(&self, interval_secs: u64) {
        {
            let mut running = self.is_monitoring.write().await;
            if *running {
                warn!("Auto record scheduler already running");
                return;
            }
            *running = true;
        }

        let manager = Arc::clone(&self.manager);
        let live_checker = Arc::clone(&self.live_checker);
        let speed_trackers = Arc::clone(&self.speed_trackers);
        let output_dir = Arc::clone(&self.output_dir);
        let is_monitoring = Arc::clone(&self.is_monitoring);

        let handle = tokio::spawn(async move {
            info!("Auto record scheduler started");
            loop {
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;

                if !*is_monitoring.read().await {
                    info!("Auto record scheduler stopped");
                    break;
                }

                Self::tick_auto_record(
                    &manager,
                    &live_checker,
                    &speed_trackers,
                    &output_dir,
                )
                .await;
            }
        });

        let mut task = self.monitor_task.write().await;
        *task = Some(handle);
    }

    /// 一次调度 tick
    async fn tick_auto_record(
        manager: &Arc<RecordingManager>,
        live_checker: &Arc<RwLock<Option<Arc<dyn LiveStatusChecker>>>>,
        _speed_trackers: &Arc<RwLock<HashMap<String, SpeedTracker>>>,
        output_dir: &Arc<RwLock<String>>,
    ) {
        let entries = manager.get_status_summary().await;

        for (rec_id, url, is_currently_recording) in entries {
            // 注入的 checker 是否存在
            let checker_opt = {
                let guard = live_checker.read().await;
                guard.clone()
            };

            let Some(checker) = checker_opt else {
                warn!("No live status checker, skipping auto record tick");
                break;
            };

            // TODO: 解析 quality / platform_key / proxy 从 recording 的完整配置
            let quality = "OD";
            let platform_key = "";
            let proxy: Option<&str> = None;

            match checker.check_live_status(&url, platform_key, quality, proxy).await {
                Some(stream_info) if stream_info.is_live && !is_currently_recording => {
                    info!("Detected live: {} (url: {})", stream_info.anchor_name, url);

                    // 根据源信息构建 RecorderConfig
                    let (selection_url, use_direct_download) =
                        match select_stream_source(&stream_info.record_url, platform_key) {
                            StreamSelection::FlvDirectDownload { url, .. } => {
                                (url, true)
                            }
                            StreamSelection::HlsRemux { url }
                            | StreamSelection::UnknownRemux { url } => (url, false),
                        };

                    let save_format = if use_direct_download { "ts" } else { "mp4" };
                    let output_dir_str = output_dir.read().await.clone();
                    let filename = format!("{}_{}", stream_info.anchor_name, platform_key);
                    let save_path = format!("{}/{}.{}", output_dir_str.trim_end_matches('/'), filename, save_format);

                    let input_opts = if InputOptions::is_overseas_domain(&url) {
                        InputOptions::default_overseas()
                    } else {
                        InputOptions::default_cn()
                    };

                    let config = RecorderConfig {
                        record_url: selection_url,
                        output_path: save_path,
                        format: match save_format {
                            "ts" => crate::format::OutputFormat::Ts,
                            "mp4" => crate::format::OutputFormat::Mp4,
                            "flv" => crate::format::OutputFormat::Flv,
                            _ => crate::format::OutputFormat::Mp4,
                        },
                        segment_record: false,
                        segment_time: 1800,
                        is_overseas: false,
                        headers: None,
                        proxy: None,
                        use_direct_download,
                        input_options: Some(input_opts),
                        max_retries: 3,
                    };

                    let _ = manager.start_recording(&rec_id, config).await;
                }

                Some(_) if !is_currently_recording => { /* do nothing, not live */ }

                Some(_stream_info) if is_currently_recording => {
                    info!("Stream ended ({}), stopping recording", rec_id);
                    manager.stop_recording(&rec_id).await;
                    // TODO: 触发后置转码
                }

                _ => {}
            }
        }
    }

    /// 获取全局统计信息
    pub async fn get_stats(&self) -> ControllerStats {
        let active_count = self.manager.get_active_recording_count().await;
        let summary = self.manager.get_status_summary().await;
        ControllerStats {
            active_recordings: active_count,
            total_recordings: summary.len(),
            recording_statuses: summary,
            output_dir: self.output_dir.read().await.clone(),
            disk_free_gb: crate::utils::check_disk_free_space(Path::new(
                &*self.output_dir.read().await,
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControllerStats {
    pub active_recordings: usize,
    pub total_recordings: usize,
    pub recording_statuses: Vec<(String, String, bool)>,
    pub output_dir: String,
    pub disk_free_gb: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_creation() {
        let c = RecordingController::new("./test_recordings");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let stats = c.get_stats().await;
            assert_eq!(stats.total_recordings, 0);
            assert_eq!(stats.active_recordings, 0);
        });
    }

    #[test]
    fn test_speed_tracker() {
        let mut st = SpeedTracker::default();
        st.update(1024 * 1024, Duration::from_secs(1));
        assert_eq!(st.bytes_per_sec, 1024 * 1024);
        assert!(st.speed_string().contains("MB/s"));
    }

    #[test]
    fn test_cumulative_duration() {
        let mut st = SpeedTracker::default();
        st.update(0, Duration::from_secs(3661));
        let dur_str = st.cumulative_duration_string();
        assert_eq!(dur_str, "01:01:01");
    }
}
