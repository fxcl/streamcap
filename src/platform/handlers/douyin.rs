#![allow(dead_code)]
//! Douyin (TikTok China) platform handler.


use async_trait::async_trait;
use reqwest;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::base::{PlatformHandler, StreamData};

/// 抖音流数据 API 响应 (简化)
#[derive(Debug, Deserialize)]
struct DouyinStreamResponse {
    data: Option<DouyinStreamData>,
}

#[derive(Debug, Deserialize)]
struct DouyinStreamData {
    status: Option<i32>,
    stream_url: Option<DouyinStreamUrl>,
}

#[derive(Debug, Deserialize)]
struct DouyinStreamUrl {
    flv_pull_url: Option<DouyinFlvUrl>,
}

#[derive(Debug, Deserialize)]
struct DouyinFlvUrl {
    #[serde(rename = "FULL_HD1")]
    full_hd1: Option<String>,
    #[serde(rename = "HD1")]
    hd1: Option<String>,
    #[serde(rename = "SD1")]
    sd1: Option<String>,
    #[serde(rename = "SD2")]
    sd2: Option<String>,
}

/// 抖音直播处理器 (对应 Python DouyinHandler)
pub struct DouyinHandler {
    client: reqwest::Client,
    proxy: Option<String>,
}

impl DouyinHandler {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .build()
                .unwrap_or_default(),
            proxy: None,
        }
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        if let Some(r) = url.strip_prefix("https://live.douyin.com/") {
            if !r.is_empty() && !r.contains('/') && !r.contains('?') {
                return Some(r.to_string());
            }
        }
        if let Some(r) = url.strip_prefix("http://live.douyin.com/") {
            if !r.is_empty() && !r.contains('/') && !r.contains('?') {
                return Some(r.to_string());
            }
        }
        if url.contains("v.douyin.com") {
            debug!("Douyin short link detected, needs HTTP redirect to resolve");
        }
        None
    }

    fn build_api_url(&self, room_id: &str) -> String {
        format!(
            "https://live.douyin.com/webcast/room/web/enter/?aid=6383&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&room_id_str={}&web_rid={}",
            room_id, room_id
        )
    }

    fn pick_best_flv(&self, url: Option<&String>) -> Option<String> {
        url.filter(|s| !s.is_empty()).cloned()
    }
}

#[async_trait]
impl PlatformHandler for DouyinHandler {
    fn platform_id(&self) -> &str {
        "douyin"
    }

    fn platform_name(&self) -> &str {
        "抖音"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse Douyin room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching Douyin stream info for room: {}", room_id);

        let api_url = self.build_api_url(&room_id);
        let result = self.client
            .get(&api_url)
            .header("Referer", "https://live.douyin.com/")
            .header("Origin", "https://live.douyin.com")
            .send()
            .await;

        match result {
            Ok(resp) => {
                match resp.json::<DouyinStreamResponse>().await {
                    Ok(stream_resp) => {
                        if let Some(data) = stream_resp.data {
                            let is_live = data.status == Some(2);

                            if !is_live {
                                return Some(StreamData {
                                    platform_name: self.platform_name().to_string(),
                                    anchor_name: String::new(),
                                    title: String::new(),
                                    is_live: false,
                                    record_url: String::new(),
                                    flv_url: None,
                                    m3u8_url: None,
                                    room_id: Some(room_id),
                                });
                            }

                            let flv_url = data.stream_url.as_ref()
                                .and_then(|u| u.flv_pull_url.as_ref())
                                .and_then(|f| self.pick_best_flv(f.full_hd1.as_ref()))
                                .or_else(|| {
                                    data.stream_url.as_ref()
                                        .and_then(|u| u.flv_pull_url.as_ref())
                                        .and_then(|f| self.pick_best_flv(f.hd1.as_ref()))
                                })
                                .or_else(|| {
                                    data.stream_url.as_ref()
                                        .and_then(|u| u.flv_pull_url.as_ref())
                                        .and_then(|f| self.pick_best_flv(f.sd1.as_ref()))
                                });

                            Some(StreamData {
                                platform_name: self.platform_name().to_string(),
                                anchor_name: String::new(),
                                title: String::new(),
                                is_live,
                                record_url: flv_url.clone().unwrap_or_default(),
                                flv_url,
                                m3u8_url: None,
                                room_id: Some(room_id),
                            })
                        } else {
                            warn!("Douyin API response has no data field");
                            None
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse Douyin API response: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("HTTP request to Douyin API failed: {}", e);
                None
            }
        }
    }

    fn matches(&self, url: &str) -> bool {
        url.contains("douyin.com")
    }

    fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    fn set_proxy(&mut self, proxy: Option<String>) {
        self.proxy = proxy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_id() {
        let handler = DouyinHandler::new();
        assert_eq!(handler.platform_id(), "douyin");
    }

    #[test]
    fn test_matches_douyin_urls() {
        let handler = DouyinHandler::new();
        assert!(handler.matches("https://live.douyin.com/12345"));
        assert!(handler.matches("https://v.douyin.com/AbCdEf"));
        assert!(!handler.matches("https://live.bilibili.com/12345"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = DouyinHandler::new();
        assert_eq!(handler.parse_room_id("https://live.douyin.com/12345"), Some("12345".to_string()));
        assert_eq!(handler.parse_room_id("https://live.douyin.com/abcDEF123"), Some("abcDEF123".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/12345"), None);
    }

    #[test]
    fn test_build_api_url() {
        let handler = DouyinHandler::new();
        let url = handler.build_api_url("12345");
        assert!(url.contains("12345"));
        assert!(url.contains("webcast"));
    }
}
