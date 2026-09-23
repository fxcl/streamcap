#![allow(dead_code)]
//! YY 直播平台 handler.
//!
//! YY 直播流获取: 解析页面 HTML 中嵌入的 liveUrl / hlsUrl / streamUrl
//! URL 格式: https://www.yy.com/{room_id}

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::base::{build_http_client, PlatformHandler, StreamData};

/// YY 页面嵌入的直播 JSON (简化)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YyLiveData {
    #[serde(default)]
    live_url: Option<String>,
    #[serde(default)]
    hls_url: Option<String>,
    #[serde(default)]
    stream_url: Option<String>,
    #[serde(default)]
    status: Option<i32>,
    #[serde(default)]
    room_name: Option<String>,
    #[serde(default)]
    anchor_name: Option<String>,
}

/// YY 直播处理器
pub struct YYHandler {
    proxy: Option<String>,
}

impl YYHandler {
    pub fn new() -> Self {
        Self { proxy: None }
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        let patterns = [
            "https://www.yy.com/",
            "http://www.yy.com/",
            "https://yy.com/",
            "http://yy.com/",
        ];
        for prefix in &patterns {
            if let Some(r) = url.strip_prefix(prefix) {
                let id = r.split('/').next().unwrap_or(r);
                let id = id.split('?').next().unwrap_or(id);
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    fn build_page_url(&self, room_id: &str) -> String {
        format!("https://www.yy.com/{}", room_id)
    }

    fn extract_string_field(&self, html: &str, field: &str) -> Option<String> {
        let patterns = [
            format!(r#"var\s+{}\s*=\s*["']([^"']+)""#, regex::escape(field)),
            format!(r#"{}\s*[:=]\s*["']([^"']+)""#, regex::escape(field)),
            format!(r#""{}"\s*:\s*"([^"]+)""#, regex::escape(field)),
            format!(r"'{}'\s*:\s*'([^']+)'", regex::escape(field)),
        ];
        for pat in &patterns {
            if let Ok(re) = Regex::new(pat) {
                if let Some(cap) = re.captures(html) {
                    return Some(cap[1].to_string());
                }
            }
        }
        None
    }

    fn extract_stream_data(&self, html: &str) -> Option<(String, String, bool)> {
        // 方法1: JSON 解析
        let re_json = Regex::new(r#"window\.(?:roomInfo|roomData|__INITIAL_STATE__)\s*=\s*(\{.+?\});?</script>"#).ok()?;
        if let Some(cap) = re_json.captures(html) {
            if let Ok(data) = serde_json::from_str::<YyLiveData>(&cap[1]) {
                let url = data.live_url
                    .or(data.hls_url)
                    .or(data.stream_url)
                    .filter(|u| !u.is_empty());

                let anchor = data.anchor_name
                    .or(data.room_name)
                    .unwrap_or_default();

                let is_live = data.status == Some(1) || url.is_some();
                if let Some(u) = url {
                    return Some((u, anchor, is_live));
                }
            }
        }

        // 方法2: 正则提取字段
        let url = self.extract_string_field(html, "liveUrl")
            .or_else(|| self.extract_string_field(html, "hlsUrl"))
            .or_else(|| self.extract_string_field(html, "streamUrl"))
            .filter(|u| !u.is_empty());

        if let Some(u) = url {
            let anchor = self.extract_string_field(html, "anchorName")
                .or_else(|| self.extract_string_field(html, "roomName"))
                .unwrap_or_default();
            return Some((u, anchor, true));
        }

        // 方法3: 直接找 .m3u8 / .flv URL
        let re_m3u8 = Regex::new(r#"(https?://[^"'\s]+\.m3u8[^\s"'<>]*)"#).ok()?;
        if let Some(cap) = re_m3u8.captures(html) {
            return Some((cap[1].to_string(), String::new(), true));
        }

        let re_flv = Regex::new(r#"(https?://[^"'\s]+\.flv[^\s"'<>]*)"#).ok()?;
        if let Some(cap) = re_flv.captures(html) {
            return Some((cap[1].to_string(), String::new(), true));
        }

        None
    }
}

#[async_trait]
impl PlatformHandler for YYHandler {
    fn platform_id(&self) -> &str {
        "yy"
    }

    fn platform_name(&self) -> &str {
        "YY直播"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse YY room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching YY stream info for room: {}", room_id);

        let page_url = self.build_page_url(&room_id);
        let client = build_http_client(self.proxy.as_deref());

        match client.get(&page_url)
            .header("Referer", "https://www.yy.com/")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await
        {
            Ok(resp) => {
                match resp.text().await {
                    Ok(html) => {
                        debug!("YY page HTML length: {}", html.len());

                        if let Some((stream_url, anchor, is_live)) = self.extract_stream_data(&html) {
                            if is_live && !stream_url.is_empty() {
                                info!("YY stream found for room {}: anchor={}, url_len={}",
                                    room_id, anchor.len(), stream_url.len());

                                let is_hls = stream_url.contains(".m3u8");

                                Some(StreamData {
                                    platform_name: self.platform_name().to_string(),
                                    anchor_name: anchor,
                                    title: String::new(),
                                    is_live: true,
                                    record_url: stream_url.clone(),
                                    flv_url: if !is_hls { Some(stream_url.clone()) } else { None },
                                    m3u8_url: if is_hls { Some(stream_url) } else { None },
                                    room_id: Some(room_id),
                                })
                            } else {
                                warn!("YY room {} is not live", room_id);
                                Some(StreamData {
                                    platform_name: self.platform_name().to_string(),
                                    anchor_name: anchor,
                                    title: String::new(),
                                    is_live: false,
                                    record_url: String::new(),
                                    flv_url: None,
                                    m3u8_url: None,
                                    room_id: Some(room_id),
                                })
                            }
                        } else {
                            warn!("Could not extract stream data from YY page");
                            None
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read YY page: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("HTTP request to YY failed: {}", e);
                None
            }
        }
    }

    fn matches(&self, url: &str) -> bool {
        // 避免误匹配 youtube.com
        url.contains("yy.com") && !url.contains("youtube")
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
        let handler = YYHandler::new();
        assert_eq!(handler.platform_id(), "yy");
        assert_eq!(handler.platform_name(), "YY直播");
    }

    #[test]
    fn test_matches_yy_urls() {
        let handler = YYHandler::new();
        assert!(handler.matches("https://www.yy.com/12345"));
        assert!(handler.matches("https://yy.com/live/test"));
        assert!(!handler.matches("https://www.youtube.com/watch?v=123"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = YYHandler::new();
        assert_eq!(handler.parse_room_id("https://www.yy.com/12345"), Some("12345".to_string()));
        assert_eq!(handler.parse_room_id("http://www.yy.com/test_room"), Some("test_room".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/123"), None);
    }

    #[test]
    fn test_extract_stream_data_json() {
        let handler = YYHandler::new();
        let html = r#"<script>
window.roomInfo = {"liveUrl": "https://yy-live.example.com/stream.m3u8", "anchorName": "主播A", "status": 1};
</script>"#;

        let result = handler.extract_stream_data(html);
        assert!(result.is_some());
        let (url, anchor, is_live) = result.unwrap();
        assert_eq!(url, "https://yy-live.example.com/stream.m3u8");
        assert_eq!(anchor, "主播A");
        assert!(is_live);
    }

    #[test]
    fn test_extract_stream_data_live_url_field() {
        let handler = YYHandler::new();
        let html = r#"<script>var liveUrl = "https://cdn.yy.com/live/12345.m3u8?token=abc";</script>"#;
        let result = handler.extract_stream_data(html);
        let (url, _, _) = result.unwrap();
        assert_eq!(url, "https://cdn.yy.com/live/12345.m3u8?token=abc");
    }

    #[test]
    fn test_extract_stream_data_direct_m3u8() {
        let handler = YYHandler::new();
        let html = r#"<html><video src="https://cdn.yy.com/live/test123.m3u8?token=xyz"></html>"#;
        let result = handler.extract_stream_data(html);
        let (url, _, _) = result.unwrap();
        assert!(url.contains("test123.m3u8"));
    }

    #[test]
    fn test_extract_stream_data_not_live() {
        let handler = YYHandler::new();
        let html = r#"<html><body>No live content</body></html>"#;
        let result = handler.extract_stream_data(html);
        assert!(result.is_none());
    }
}
