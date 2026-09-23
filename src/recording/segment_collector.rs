#![allow(dead_code)]
//! HLS segment collection and platform-proxy configuration.


use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

/// 分段录制的一个结果片段
#[derive(Debug, Clone)]
pub struct SegmentFile {
    pub path: PathBuf,
    pub index: u32,
    pub duration_secs: u64,
    pub bytes: u64,
    pub created_at: Instant,
}

/// 分段录制结果收集器
///
/// 对应 Python 中的分段处理:
/// - `utils.get_file_paths` 收集目录文件
/// - 根据文件名前缀匹配分段
/// - 返回排序后的分段列表
///
/// Rust 版本用更结构化的方式跟踪分段
pub struct SegmentCollector {
    output_dir: String,
    prefix: String,
    segments: Arc<RwLock<Vec<SegmentFile>>>,
}

impl SegmentCollector {
    pub fn new(output_dir: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            output_dir: output_dir.into(),
            prefix: prefix.into(),
            segments: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 注册一个新的分段文件
    pub async fn register_segment(
        &self,
        path: PathBuf,
        index: u32,
        duration_secs: u64,
        bytes: u64,
    ) {
        let mut segments = self.segments.write().await;
        segments.push(SegmentFile {
            path,
            index,
            duration_secs,
            bytes,
            created_at: Instant::now(),
        });
    }

    /// 扫描磁盘查找已存在的分段 (启动时或恢复时使用)
    pub async fn scan_existing(&self) -> Vec<SegmentFile> {
        let dir = Path::new(&self.output_dir);
        let prefix = &self.prefix;

        let mut results = Vec::new();
        if !dir.exists() {
            return results;
        }

        if let Ok(entries) = std::fs::read_dir(dir) {
            for (idx, entry) in entries.flatten().enumerate() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with(prefix) && name != prefix {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    results.push(SegmentFile {
                        path,
                        index: idx as u32,
                        duration_secs: 0, // 无法从文件名恢复
                        bytes,
                        created_at: Instant::now(),
                    });
                }
            }
        }

        results.sort_by_key(|s| s.index);

        // 缓存结果
        *self.segments.write().await = results.clone();

        info!(
            "Scanned {} existing segments in {}/{}*",
            results.len(),
            self.output_dir,
            self.prefix
        );
        results
    }

    /// 获取所有分段 (内存缓存)
    pub async fn get_segments(&self) -> Vec<SegmentFile> {
        self.segments.read().await.clone()
    }

    /// 获取下一个分段编号
    pub async fn next_index(&self) -> u32 {
        let segments = self.segments.read().await;
        segments.iter().map(|s| s.index).max().map(|m| m + 1).unwrap_or(0)
    }

    /// 获取总字节数
    pub async fn total_bytes(&self) -> u64 {
        self.segments.read().await.iter().map(|s| s.bytes).sum()
    }

    /// 获取总时长
    pub async fn total_duration(&self) -> u64 {
        self.segments.read().await.iter().map(|s| s.duration_secs).sum()
    }

    /// 删除所有分段文件 (对应 Python 分段清理)
    pub async fn cleanup(&self) -> Result<(), std::io::Error> {
        let segments = self.segments.write().await;
        for seg in segments.iter() {
            if seg.path.exists() {
                tokio::fs::remove_file(&seg.path).await?;
                info!("Cleaned up segment: {:?}", seg.path);
            }
        }
        segments.iter().for_each(|_| {}); // 消费 guard
        info!("Cleaned up {} segments", segments.len());
        Ok(())
    }

    /// 合并所有分段到一个文件 (简化实现 - 仅复制字节)
    pub async fn concat_to(&self, output_path: &Path) -> Result<(), std::io::Error> {
        let segments = self.get_segments().await;
        if segments.is_empty() {
            warn!("No segments to concat");
            return Ok(());
        }

        let mut output = tokio::fs::File::create(output_path).await?;
        for seg in &segments {
            let data = tokio::fs::read(&seg.path).await?;
            tokio::io::AsyncWriteExt::write_all(&mut output, &data).await?;
        }
        info!(
            "Concatenated {} segments to {:?}",
            segments.len(),
            output_path
        );
        Ok(())
    }
}

/// 平台代理选择器 - 对应 Python stream_manager.is_use_proxy
///
/// 允许按平台配置是否使用代理, 以及自定义代理地址
pub struct PlatformProxyConfig {
    /// 使用代理的平台列表
    pub platforms_with_proxy: Vec<String>,
    /// 全局代理地址
    pub proxy_address: Option<String>,
    /// 强制所有流量走 HTTPS
    pub force_https: bool,
}

impl Default for PlatformProxyConfig {
    fn default() -> Self {
        Self {
            platforms_with_proxy: vec![
                // 部分平台和 force_https 配合, 优先 https 否则 http
                "pandalive".to_string(),
                "winktv".to_string(),
                "popkontv".to_string(),
                "flextv".to_string(),
                "lang".to_string(),
            ],
            proxy_address: None,
            force_https: false,
        }
    }
}

impl PlatformProxyConfig {
    /// 判断一个平台是否应使用代理
    pub fn should_use_proxy(&self, platform_key: &str) -> bool {
        self.proxy_address.is_some() && self.platforms_with_proxy.contains(&platform_key.to_string())
    }

    /// 为指定平台构建代理配置
    pub fn build_proxy(&self, platform_key: &str) -> Option<String> {
        if self.should_use_proxy(platform_key) {
            self.proxy_address.clone()
        } else {
            None
        }
    }

    /// 设置自定义代理地址
    pub fn set_proxy<S: Into<String>>(&mut self, address: Option<S>) {
        self.proxy_address = address.map(|s| s.into());
    }

    /// 添加平台到代理列表
    pub fn add_platform(&mut self, platform: impl Into<String>) {
        let p = platform.into();
        if !self.platforms_with_proxy.contains(&p) {
            self.platforms_with_proxy.push(p);
        }
    }

    /// 移除平台
    pub fn remove_platform(&mut self, platform: &str) {
        self.platforms_with_proxy.retain(|p| p != platform);
    }
}

/// 录制完成钩子 trait - 在录制结束时触发后置处理
///
/// 对应 Python:
/// - `converts_mp4` (TS → MP4 转码)
/// - `delete_original` (删除原文件)
/// - `custom_script_execute` (自定义脚本)
#[async_trait::async_trait]
pub trait RecordingCompleteHook: Send + Sync {
    /// 录制完成后调用
    ///
    /// - `output_path`: 录制产物文件路径
    /// - `rec_id`: 录制条目 ID
    async fn on_recording_complete(
        &self,
        output_path: &str,
        rec_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// 默认录制完成钩子: 可选的 TS→MP4 后置转码
pub struct DefaultPostProcessHook {
    pub convert_to_mp4: bool,
    pub delete_original: bool,
}

#[async_trait::async_trait]
impl RecordingCompleteHook for DefaultPostProcessHook {
    async fn on_recording_complete(
        &self,
        output_path: &str,
        _rec_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.convert_to_mp4 {
            return Ok(());
        }

        let path = Path::new(output_path);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if ext == "ts" {
            // TS -> MP4 转码
            let mp4_path = path.with_extension("mp4");
            super::post_processor::FFmpegPostProcessor::remux_ts_to_mp4(
                output_path,
                mp4_path.to_str().unwrap(),
            ).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            })?;

            if self.delete_original {
                super::post_processor::FFmpegPostProcessor::delete_original(output_path).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                })?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_proxy_config_default() {
        let mut config = PlatformProxyConfig::default();
        config.set_proxy(Some("http://127.0.0.1:7890"));
        assert!(config.should_use_proxy("pandalive"));
        assert!(!config.should_use_proxy("douyin"));
    }

    #[test]
    fn test_platform_proxy_custom_address() {
        let mut config = PlatformProxyConfig::default();
        config.set_proxy(Some("http://127.0.0.1:7890"));
        assert_eq!(
            config.build_proxy("pandalive"),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(config.build_proxy("douyin"), None);
    }

    #[test]
    fn test_platform_proxy_add_remove() {
        let mut config = PlatformProxyConfig::default();
        config.set_proxy(Some("http://127.0.0.1:7890"));
        config.add_platform("my_platform");
        assert!(config.should_use_proxy("my_platform"));
        config.remove_platform("my_platform");
        assert!(!config.should_use_proxy("my_platform"));
    }

    #[test]
    fn test_platform_proxy_no_addr() {
        let config = PlatformProxyConfig::default();
        // 没有地址时, 即使用户在列表中也不使用代理
        assert_eq!(config.build_proxy("pandalive"), None);
    }

    #[test]
    fn test_segment_collector_basic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let collector = SegmentCollector::new("/tmp/test", "stream");
            collector
                .register_segment(PathBuf::from("/tmp/test/stream_001.mp4"), 0, 60, 1024)
                .await;
            collector
                .register_segment(PathBuf::from("/tmp/test/stream_002.mp4"), 1, 60, 2048)
                .await;

            assert_eq!(collector.next_index().await, 2);
            assert_eq!(collector.total_bytes().await, 3072);
            assert_eq!(collector.total_duration().await, 120);
            assert_eq!(collector.get_segments().await.len(), 2);
        });
    }
}
