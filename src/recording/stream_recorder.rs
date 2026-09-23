//! Core stream recorder (remux engine + reconnect + interrupt callback).
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};
use serde::{Deserialize, Serialize};

use super::error::RecordingError;
use super::input_options::InputOptions;

/// 录制配置
#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub record_url: String,
    pub output_path: String,
    pub format: crate::format::OutputFormat,
    pub segment_record: bool,
    pub segment_time: u32,
    pub is_overseas: bool,
    /// 平台特定 HTTP headers (自动根据平台填充, 可覆盖)
    pub headers: Option<String>,
    pub proxy: Option<String>,
    pub use_direct_download: bool,
    /// 自定义 Input Options (可选, 默认根据 is_overseas 自动选择)
    pub input_options: Option<InputOptions>,
    /// 最大重连次数
    pub max_retries: u32,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            record_url: String::new(),
            output_path: String::new(),
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
    }
}

/// 录制统计 - 对应 Python RecordingStatus 中的运行时字段
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecorderStats {
    pub total_packets: u64,
    pub duration_secs: u64,
    pub bytes_written: u64,
    /// 当前录制速度 (bytes/sec)
    pub bytes_per_sec: u64,
    /// 累计录制字节
    pub cumulative_bytes: u64,
    /// 录制速度的人类可读字符串 (如 "1.2 MB/s")
    pub speed_string: String,
}

/// 直播流录制器 - 异步封装
pub struct StreamRecorder {
    engine: Arc<Mutex<super::RemuxEngine>>,
    config: Arc<RwLock<RecorderConfig>>,
    is_recording: Arc<RwLock<bool>>,
    start_time: Arc<RwLock<Option<Instant>>>,
}

impl StreamRecorder {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(Mutex::new(super::RemuxEngine::new())),
            config: Arc::new(RwLock::new(RecorderConfig::default())),
            is_recording: Arc::new(RwLock::new(false)),
            start_time: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self, config: RecorderConfig) -> Result<(), RecordingError> {
        {
            let mut recording = self.is_recording.write().await;
            if *recording {
                return Err(RecordingError::AlreadyRecording);
            }
            *recording = true;
        }

        {
            let mut cfg = self.config.write().await;
            *cfg = config.clone();
        }

        {
            let mut start = self.start_time.write().await;
            *start = Some(Instant::now());
        }

        if config.use_direct_download {
            self.start_direct_download(config).await
        } else {
            self.start_remux(config).await
        }
    }

    pub async fn stop(&self) {
        info!("Stop requested for recorder");
        let engine = self.engine.lock().await;
        engine.request_stop();
        let mut recording = self.is_recording.write().await;
        *recording = false;
    }

    pub async fn is_recording(&self) -> bool {
        *self.is_recording.read().await
    }

    pub async fn elapsed_secs(&self) -> Option<u64> {
        self.start_time.read().await.map(|t| t.elapsed().as_secs())
    }

    async fn start_remux(&self, config: RecorderConfig) -> Result<(), RecordingError> {
        let engine = Arc::clone(&self.engine);
        let is_recording = Arc::clone(&self.is_recording);

        info!("Starting remux recording: {}", config.record_url);
        info!("Output: {}", config.output_path);
        info!("Format: {:?}", config.format);

        if let Some(parent) = Path::new(&config.output_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 解析 input_options: 如果未在 config 中设置, 按 is_overseas 选默认
        let mut input_options = config
            .input_options
            .clone()
            .unwrap_or_else(|| {
                if config.is_overseas {
                    InputOptions::default_overseas()
                } else {
                    InputOptions::default_cn()
                }
            });

        // 合并代理到 input_options
        if let Some(ref proxy) = config.proxy {
            if input_options.http_proxy.is_none() {
                input_options.http_proxy = Some(proxy.clone());
            }
        }

        // 合并用户传入的 headers 到 input_options (附加到已有 headers 后面)
        if let Some(ref user_headers) = config.headers {
            match input_options.headers {
                Some(ref existing) => {
                    input_options.headers = Some(format!("{}\r\n{}", existing, user_headers));
                }
                None => {
                    input_options.headers = Some(user_headers.clone());
                }
            }
        }

        // 自动推导平台 headers: 如果 headers 为空且无用户给定, 尝试用 record_url 推断
        if input_options.headers.is_none() && config.headers.is_none() {
            // 从 URL 域名匹配平台 (bilibili/douyin 需要 Referer)
            let url_lower = config.record_url.to_lowercase();
            if url_lower.contains("bilibili") {
                input_options.merge_platform_headers("bilibili");
            }
        }

        let record_url = config.record_url.clone();
        let output_path = config.output_path.clone();
        let format_config = crate::format::FormatConfig {
            format: config.format,
            segment_record: config.segment_record,
            segment_time: config.segment_time,
            video_bitrate: None,
            movflags: None,
            mpegts_flags: None,
        };
        let max_retries = config.max_retries;

        let result = tokio::task::spawn_blocking(move || {
            let eng = engine.blocking_lock();
            eng.remux(&record_url, &output_path, &format_config, &input_options, max_retries)
        }).await;

        *is_recording.write().await = false;

        match result {
            Ok(Ok(stats)) => {
                info!("Recording completed: {} packets, {} bytes in {}s",
                    stats.total_packets, stats.bytes_written, stats.duration_secs);
                Ok(())
            }
            Ok(Err(e)) => {
                error!("Recording failed: {}", e);
                Err(e)
            }
            Err(e) => {
                error!("Recording task panicked: {}", e);
                Err(RecordingError::Unknown(format!("Task join error: {}", e)))
            }
        }
    }

    async fn start_direct_download(&self, config: RecorderConfig) -> Result<(), RecordingError> {
        info!("Starting direct download: {}", config.record_url);

        if let Some(parent) = Path::new(&config.output_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let url = config.record_url.clone();
        let output_path = config.output_path.clone();
        let headers = config.headers.clone();
        let is_recording = Arc::clone(&self.is_recording);
        let engine = Arc::clone(&self.engine);

        tokio::spawn(async move {
            match super::direct_downloader::download_stream(
                &url,
                &output_path,
                headers.as_deref(),
                engine.clone(),
            ).await {
                Ok(bytes) => {
                    info!("Direct download completed: {} bytes", bytes);
                }
                Err(e) => {
                    error!("Direct download failed: {}", e);
                }
            }
            *is_recording.write().await = false;
        });

        Ok(())
    }
}

impl Default for StreamRecorder {
    fn default() -> Self {
        Self::new()
    }
}
