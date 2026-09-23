#![allow(dead_code)]
//! Fallback platform handler (any URL → direct stream link).


use async_trait::async_trait;
use tracing::debug;

use super::base::{PlatformHandler, StreamData};

/// 通用平台处理器 - 当没有专门的处理器时直接透传直播 URL
///
/// 对应 Python CustomHandler
pub struct GenericHandler {
    proxy: Option<String>,
}

impl GenericHandler {
    pub fn new(proxy: Option<String>) -> Self {
        Self { proxy }
    }
}

#[async_trait]
impl PlatformHandler for GenericHandler {
    fn platform_id(&self) -> &str {
        "generic"
    }

    fn platform_name(&self) -> &str {
        "通用/未知平台"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        debug!("GenericHandler: treating URL as direct stream link: {}", live_url);

        let (is_flv, is_m3u8) = if live_url.contains(".flv") {
            (true, false)
        } else if live_url.contains(".m3u8") || live_url.contains(".ts") {
            (false, true)
        } else {
            (true, false) // default to flv-like handling
        };

        Some(StreamData {
            platform_name: self.platform_name().to_string(),
            anchor_name: String::new(),
            title: String::new(),
            is_live: true,
            record_url: live_url.to_string(),
            flv_url: if is_flv { Some(live_url.to_string()) } else { None },
            m3u8_url: if is_m3u8 { Some(live_url.to_string()) } else { None },
            room_id: None,
        })
    }

    fn matches(&self, _url: &str) -> bool {
        true // GenericHandler 匹配所有 URL, 作为 fallback
    }

    fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    fn set_proxy(&mut self, proxy: Option<String>) {
        self.proxy = proxy;
    }
}
