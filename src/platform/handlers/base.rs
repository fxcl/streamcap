#![allow(dead_code)]
//! Platform handler registry (URL routing + trait abstraction).


use std::collections::HashMap;
use async_trait::async_trait;
use tracing::{info, warn};

use super::douyin::DouyinHandler;
use super::bilibili::BilibiliHandler;
use super::generic::GenericHandler;
use super::kuaishou::KuaishouHandler;
use super::huya::HuyaHandler;
use super::douyu::DouyuHandler;
use super::yy::YYHandler;

/// 直播流信息 - 对应 Python StreamData
#[derive(Debug, Clone, Default)]
pub struct StreamData {
    /// 平台名 (如 "抖音", "哔哩哔哩")
    pub platform_name: String,
    /// 主播名
    pub anchor_name: String,
    /// 直播标题
    pub title: String,
    /// 是否正在直播
    pub is_live: bool,
    /// 主要录制 URL (可能是 FLV 或 HLS)
    pub record_url: String,
    /// FLV 流 URL (如果有)
    pub flv_url: Option<String>,
    /// M3U8/HLS 流 URL (如果有)
    pub m3u8_url: Option<String>,
    /// 房间/直播 ID
    pub room_id: Option<String>,
}

impl StreamData {
    /// 判断流数据是否可用于录制
    pub fn is_recordable(&self) -> bool {
        self.is_live && !self.record_url.is_empty()
    }

    /// 获取最佳录制 URL (优先 FLV 后 HLS)
    pub fn best_record_url(&self) -> Option<&str> {
        if let Some(ref flv) = self.flv_url {
            if !flv.is_empty() {
                return Some(flv);
            }
        }
        if let Some(ref m3u8) = self.m3u8_url {
            if !m3u8.is_empty() {
                return Some(m3u8);
            }
        }
        if !self.record_url.is_empty() {
            Some(&self.record_url)
        } else {
            None
        }
    }
}

/// 平台处理器 trait - 对应 Python PlatformHandler (ABC)
#[async_trait]
pub trait PlatformHandler: Send + Sync {
    /// 获取平台唯一标识 (如 "douyin", "bilibili")
    fn platform_id(&self) -> &str;

    /// 获取平台显示名称
    fn platform_name(&self) -> &str;

    /// 核心方法: 获取直播流信息
    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData>;

    /// 检查 URL 是否匹配此平台
    fn matches(&self, url: &str) -> bool;

    /// 获取代理地址
    fn proxy(&self) -> Option<&str>;

    /// 设置代理地址
    fn set_proxy(&mut self, proxy: Option<String>);
}

/// 构建带有可选代理的 reqwest::Client
pub fn build_http_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");

    if let Some(proxy_url) = proxy {
        if !proxy_url.is_empty() {
            if let Ok(p) = reqwest::Proxy::http(proxy_url) {
                builder = builder.proxy(p);
            }
        }
    }

    builder.build().unwrap_or_default()
}

/// 平台注册表 - 自动路由 URL 到对应的 PlatformHandler
pub struct PlatformRegistry {
    handlers: HashMap<String, Box<dyn PlatformHandler>>,
    use_proxy: bool,
    proxy: Option<String>,
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            use_proxy: false,
            proxy: None,
        }
    }

    /// 注册一个平台处理器
    pub fn register(&mut self, handler: Box<dyn PlatformHandler>) {
        let id = handler.platform_id().to_string();
        info!("Registering platform handler: {}", id);
        self.handlers.insert(id, handler);
    }

    /// 自动匹配 URL 并返回对应的处理器
    ///
    /// 优先匹配非 generic 的专用处理器，最后再 fallback 到 generic。
    pub fn resolve(&self, url: &str) -> Option<&dyn PlatformHandler> {
        let mut fallback = None;
        for handler in self.handlers.values() {
            if handler.matches(url) {
                if handler.platform_id() != "generic" {
                    return Some(handler.as_ref());
                }
                fallback = Some(handler.as_ref());
            }
        }
        fallback
    }

    /// 获取指定 ID 的处理器
    pub fn get(&self, platform_id: &str) -> Option<&dyn PlatformHandler> {
        self.handlers.get(platform_id).map(|h| h.as_ref())
    }

    /// 获取指定 ID 的可变处理器
    pub fn get_mut(&mut self, platform_id: &str) -> Option<&mut Box<dyn PlatformHandler>> {
        self.handlers.get_mut(platform_id)
    }

    /// 获取所有已注册的处理器
    pub fn all_handlers(&self) -> Vec<&dyn PlatformHandler> {
        self.handlers.values().map(|h| h.as_ref()).collect()
    }

    /// 根据 URL 自动获取流信息
    ///
    /// 对应 Python:
    /// ```python
    /// handler = self.get_platform_handler(url, platform_key)
    /// stream_data = await handler.get_stream_info(url)
    /// ```
    pub async fn fetch_stream_data(&mut self, live_url: &str) -> Option<StreamData> {
        // 优先匹配非 generic 专用处理器 (HashMap 迭代顺序不确定)
        let mut specific_id = None;
        let mut generic_id = None;
        for (id, handler) in &self.handlers {
            if handler.matches(live_url) {
                if id == "generic" {
                    generic_id = Some(id.clone());
                } else {
                    specific_id = Some(id.clone());
                    break;
                }
            }
        }

        // 优先用专用处理器
        if let Some(id) = specific_id {
            info!("Resolved platform '{}' for URL: {}", id, live_url);
            if let Some(handler) = self.handlers.get_mut(&id) {
                let result = handler.get_stream_info(live_url).await;
                if result.is_some() {
                    return result;
                }
                // 如果专用处理器获取失败，继续 fallback
                warn!("Handler '{}' failed to get stream info, trying fallback", id);
            }
        }

        // fallback: generic
        if let Some(id) = generic_id {
            warn!("No specific handler for URL: {}, using generic fallback", live_url);
            if let Some(handler) = self.handlers.get_mut(&id) {
                return handler.get_stream_info(live_url).await;
            }
        }

        None
    }

    /// 设置代理
    pub fn set_proxy(&mut self, proxy: Option<String>) {
        self.proxy = proxy.clone();
        self.use_proxy = proxy.is_some();
        for handler in self.handlers.values_mut() {
            handler.set_proxy(proxy.clone());
        }
    }

    /// 是否启用代理
    pub fn is_using_proxy(&self) -> bool {
        self.use_proxy
    }

    /// 构建默认注册表 + 常用平台
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        registry.register(Box::new(DouyinHandler::new()));
        registry.register(Box::new(BilibiliHandler::new()));
        registry.register(Box::new(KuaishouHandler::new()));
        registry.register(Box::new(HuyaHandler::new()));
        registry.register(Box::new(DouyuHandler::new()));
        registry.register(Box::new(YYHandler::new()));
        registry.register(Box::new(GenericHandler::new(None)));

        info!(
            "Built default platform registry with {} handlers",
            registry.handlers.len()
        );
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_data_default() {
        let data = StreamData::default();
        assert!(!data.is_recordable());
    }

    #[test]
    fn test_stream_data_recordable() {
        let mut data = StreamData::default();
        data.is_live = true;
        data.record_url = "https://example.com/live.flv".to_string();
        assert!(data.is_recordable());
    }

    #[test]
    fn test_stream_data_best_url() {
        let mut data = StreamData::default();
        data.flv_url = Some("https://example.com/live.flv".to_string());
        data.m3u8_url = Some("https://example.com/live.m3u8".to_string());
        assert_eq!(data.best_record_url(), data.flv_url.as_deref());
    }

    #[test]
    fn test_registry_register_and_resolve() {
        let mut registry = PlatformRegistry::new();
        registry.register(Box::new(DouyinHandler::new()));

        let handler = registry.get("douyin");
        assert!(handler.is_some());
        assert_eq!(handler.unwrap().platform_name(), "抖音");
    }

    #[test]
    fn test_registry_with_defaults() {
        let registry = PlatformRegistry::with_defaults();
        assert_eq!(registry.handlers.len(), 7);
    }

    #[test]
    fn test_resolve_douyin_url() {
        let registry = PlatformRegistry::with_defaults();
        let url = "https://live.douyin.com/12345";
        let handler = registry.resolve(url);
        assert!(handler.is_some());
        assert_eq!(handler.unwrap().platform_id(), "douyin");
    }

    #[test]
    fn test_resolve_bilibili_url() {
        let registry = PlatformRegistry::with_defaults();
        let url = "https://live.bilibili.com/67890";
        let handler = registry.resolve(url);
        assert!(handler.is_some());
        assert_eq!(handler.unwrap().platform_id(), "bilibili");
    }
}
