#![allow(dead_code)]
//! Huya (虎牙) platform handler.
//!
//! 虎牙直播流获取: 解析移动端页面 HTML，提取 sFlvUrl + sStreamName + sFlvUrlSuffix 拼装
//! URL 格式: https://www.huya.com/{room_id} 或 https://m.huya.com/{room_id}

use async_trait::async_trait;
use regex::Regex;
use tracing::{debug, info, warn};

use super::base::{build_http_client, PlatformHandler, StreamData};

/// 虎牙流信息 (从 HTML 中解析出)
#[derive(Debug)]
struct HuyaStreamInfo {
    s_flv_url: String,
    s_stream_name: String,
    s_flv_url_suffix: String,
    s_hls_url: Option<String>,
    s_room_name: Option<String>,
    s_nick: Option<String>,
    live_line_url: Option<String>,
}

/// 虎牙直播处理器
pub struct HuyaHandler {
    proxy: Option<String>,
}

impl HuyaHandler {
    pub fn new() -> Self {
        Self { proxy: None }
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        let patterns = [
            "https://m.huya.com/",
            "http://m.huya.com/",
            "https://www.huya.com/",
            "http://www.huya.com/",
            "https://huya.com/",
            "http://huya.com/",
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
        format!("https://m.huya.com/{}", room_id)
    }

    fn extract_string_field(&self, html: &str, field: &str) -> Option<String> {
        // 匹配多种格式: var field = "value"; 或 field : "value"; 或 "field": "value"
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

    fn extract_stream_info(&self, html: &str) -> Option<HuyaStreamInfo> {
        // liveLineUrl
        let re_live_line = Regex::new(r#"liveLineUrl\s*[:=]\s*["']([^"']+)["']"#).ok()?;
        let live_line_url = re_live_line.captures(html).map(|cap| cap[1].to_string());

        let s_stream_name = self.extract_string_field(html, "sStreamName")
            .or_else(|| self.extract_string_field(html, "sRoomName"))
            .unwrap_or_default();

        if s_stream_name.is_empty() && live_line_url.is_none() {
            return None;
        }

        let s_flv_url = self.extract_string_field(html, "sFlvUrl")
            .unwrap_or_else(|| "https://ms.flv.huya.com/src".to_string());

        let s_flv_url_suffix = self.extract_string_field(html, "sFlvUrlSuffix")
            .unwrap_or_else(|| "flv".to_string());

        let s_hls_url = self.extract_string_field(html, "sHlsUrl");
        let s_room_name = self.extract_string_field(html, "sRoomName");
        let s_nick = self.extract_string_field(html, "sNick");

        Some(HuyaStreamInfo {
            s_flv_url,
            s_stream_name,
            s_flv_url_suffix,
            s_hls_url,
            s_room_name,
            s_nick,
            live_line_url,
        })
    }

    fn build_flv_url(&self, info: &HuyaStreamInfo) -> Option<String> {
        if info.s_stream_name.is_empty() {
            return None;
        }
        Some(format!("{}/{}.{}",
            info.s_flv_url.trim_end_matches('/'),
            info.s_stream_name,
            info.s_flv_url_suffix
        ))
    }

    fn build_m3u8_url(&self, info: &HuyaStreamInfo) -> Option<String> {
        if let Some(ref url) = info.live_line_url {
            return Some(url.clone());
        }
        if info.s_stream_name.is_empty() && info.s_hls_url.is_none() {
            return None;
        }
        if let Some(ref hls_url) = info.s_hls_url {
            Some(format!("{}/{}.m3u8",
                hls_url.trim_end_matches('/'),
                info.s_stream_name
            ))
        } else {
            Some(format!("{}/{}.m3u8",
                info.s_flv_url.trim_end_matches('/'),
                info.s_stream_name
            ))
        }
    }
}

#[async_trait]
impl PlatformHandler for HuyaHandler {
    fn platform_id(&self) -> &str {
        "huya"
    }

    fn platform_name(&self) -> &str {
        "虎牙"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse Huya room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching Huya stream info for room: {}", room_id);

        let page_url = self.build_page_url(&room_id);
        let client = build_http_client(self.proxy.as_deref());

        match client.get(&page_url)
            .header("User-Agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1")
            .header("Referer", "https://m.huya.com/")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await
        {
            Ok(resp) => {
                match resp.text().await {
                    Ok(html) => {
                        debug!("Huya page HTML length: {}", html.len());

                        if let Some(info) = self.extract_stream_info(&html) {
                            let flv_url = self.build_flv_url(&info);
                            let m3u8_url = self.build_m3u8_url(&info);

                            let anchor = info.s_nick
                                .or(info.s_room_name)
                                .unwrap_or_default();

                            let best_url = flv_url.clone()
                                .or(m3u8_url.clone())
                                .unwrap_or_default();

                            if !best_url.is_empty() && !info.s_stream_name.is_empty() {
                                info!("Huya stream found for room {}: flv={}, m3u8={}",
                                    room_id, flv_url.is_some(), m3u8_url.is_some());

                                Some(StreamData {
                                    platform_name: self.platform_name().to_string(),
                                    anchor_name: anchor,
                                    title: String::new(),
                                    is_live: true,
                                    record_url: best_url,
                                    flv_url,
                                    m3u8_url,
                                    room_id: Some(room_id),
                                })
                            } else {
                                warn!("Huya room {} is not live", room_id);
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
                            warn!("Could not extract stream info from Huya page for room {}", room_id);
                            None
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read Huya page: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("HTTP request to Huya failed: {}", e);
                None
            }
        }
    }

    fn matches(&self, url: &str) -> bool {
        url.contains("huya.com")
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
        let handler = HuyaHandler::new();
        assert_eq!(handler.platform_id(), "huya");
        assert_eq!(handler.platform_name(), "虎牙");
    }

    #[test]
    fn test_matches_huya_urls() {
        let handler = HuyaHandler::new();
        assert!(handler.matches("https://www.huya.com/12345"));
        assert!(handler.matches("https://m.huya.com/lpl"));
        assert!(!handler.matches("https://live.bilibili.com/12345"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = HuyaHandler::new();
        assert_eq!(handler.parse_room_id("https://www.huya.com/12345"), Some("12345".to_string()));
        assert_eq!(handler.parse_room_id("https://m.huya.com/lpl"), Some("lpl".to_string()));
        assert_eq!(handler.parse_room_id("http://huya.com/test"), Some("test".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/123"), None);
    }

    #[test]
    fn test_extract_stream_info() {
        let handler = HuyaHandler::new();
        let html = r#"<script>
var sStreamName = "test123";
var sFlvUrl = "https://ms.flv.huya.com/src";
var sFlvUrlSuffix = "flv";
var sNick = "主播名字";
var sRoomName = "房间标题";
</script>"#;

        let info = handler.extract_stream_info(html).unwrap();
        assert_eq!(info.s_stream_name, "test123");
        assert_eq!(info.s_flv_url, "https://ms.flv.huya.com/src");
        assert_eq!(info.s_flv_url_suffix, "flv");
        assert_eq!(info.s_nick, Some("主播名字".to_string()));
    }

    #[test]
    fn test_build_flv_url() {
        let handler = HuyaHandler::new();
        let info = HuyaStreamInfo {
            s_flv_url: "https://ms.flv.huya.com/src".to_string(),
            s_stream_name: "test123".to_string(),
            s_flv_url_suffix: "flv".to_string(),
            s_hls_url: None,
            s_room_name: None,
            s_nick: None,
            live_line_url: None,
        };
        let url = handler.build_flv_url(&info).unwrap();
        assert_eq!(url, "https://ms.flv.huya.com/src/test123.flv");
    }

    #[test]
    fn test_build_m3u8_url_with_live_line() {
        let handler = HuyaHandler::new();
        let info = HuyaStreamInfo {
            s_flv_url: "https://ms.flv.huya.com/src".to_string(),
            s_stream_name: "test123".to_string(),
            s_flv_url_suffix: "flv".to_string(),
            s_hls_url: None,
            s_room_name: None,
            s_nick: None,
            live_line_url: Some("https://hw.hls.huya.com/src/test123.m3u8?wsSecret=abc".to_string()),
        };
        let url = handler.build_m3u8_url(&info).unwrap();
        assert_eq!(url, "https://hw.hls.huya.com/src/test123.m3u8?wsSecret=abc");
    }

    #[test]
    fn test_extract_live_line_url() {
        let handler = HuyaHandler::new();
        let html = r#"<script>
var liveLineUrl = "https://hw.hls.huya.com/src/test.flv?wsSecret=abc123";
var sStreamName = "test";
</script>"#;

        let info = handler.extract_stream_info(html).unwrap();
        assert!(info.live_line_url.is_some());
    }
}
