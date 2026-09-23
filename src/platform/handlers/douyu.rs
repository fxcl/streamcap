#![allow(dead_code)]
//! Douyu (斗鱼) platform handler.
//!
//! 斗鱼直播流获取:
//!   1. POST /api/room/ratestream 带签名
//!   2. 回退移动端页面解析
//!
//! URL 格式: https://www.douyu.com/{room_id}

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

use super::base::{build_http_client, PlatformHandler, StreamData};

/// ratestream API 响应
#[derive(Debug, Deserialize)]
struct DouyuRateStreamResp {
    error: Option<i32>,
    msg: Option<String>,
    data: Option<DouyuStreamData>,
}

#[derive(Debug, Deserialize)]
struct DouyuStreamData {
    url: Option<String>,
    rate: Option<i64>,
}

/// 斗鱼直播处理器
pub struct DouyuHandler {
    proxy: Option<String>,
    did: String,
    version: String,
}

impl DouyuHandler {
    pub fn new() -> Self {
        Self {
            proxy: None,
            did: "1000000000000000000000000001501".to_string(),
            version: "220120240101".to_string(),
        }
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        let patterns = [
            "https://m.douyu.com/topic/",
            "https://www.douyu.com/topic/",
            "https://www.douyu.com/",
            "http://www.douyu.com/",
            "https://m.douyu.com/",
            "http://m.douyu.com/",
        ];
        for prefix in &patterns {
            if let Some(r) = url.strip_prefix(prefix) {
                let id = r.split('/').next().unwrap_or(r);
                let id = id.split('?').next().unwrap_or(id);
                let id = id.split('#').next().unwrap_or(id);
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    fn build_api_url(&self) -> String {
        "https://m.douyu.com/api/room/ratestream".to_string()
    }

    fn build_page_url(&self, room_id: &str) -> String {
        format!("https://m.douyu.com/{}", room_id)
    }

    /// 斗鱼签名算法: md5(rid + did + tt + salt)
    fn compute_sign(&self, room_id: &str, timestamp: u64) -> String {
        let raw = format!("{}{}{}{}", room_id, self.did, timestamp, "r5*^5}B4_BE");
        format!("{:x}", md5::compute(raw.as_bytes()))
    }

    fn now_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn extract_from_html(&self, html: &str) -> Option<String> {
        // window._$room
        let re_room = Regex::new(r#"_\$room\s*=\s*(\{.+?\});"#).ok()?;
        if let Some(cap) = re_room.captures(html) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&cap[1]) {
                if let Some(url) = json.get("room_src").and_then(|v| v.as_str()) {
                    return Some(url.to_string());
                }
            }
        }

        // streamUrl / var streamUrl
        let re_stream = Regex::new(r#"(?:var\s+)?streamUrl\s*[:=]\s*["']([^"']+)["']"#).ok()?;
        if let Some(cap) = re_stream.captures(html) {
            return Some(cap[1].to_string());
        }

        // hlsUrl / hls_url / var hlsUrl
        let re_hls = Regex::new(r#"(?:var\s+)?hls[_]?[Uu]rl\s*[:=]\s*["']([^"']+\.m3u8[^"']*)["']"#).ok()?;
        if let Some(cap) = re_hls.captures(html) {
            return Some(cap[1].to_string());
        }

        // live_room / var live_room
        let re_live = Regex::new(r#"(?:var\s+)?live_room\s*[:=]\s*["']([^"']+)["']"#).ok()?;
        if let Some(cap) = re_live.captures(html) {
            return Some(cap[1].to_string());
        }

        None
    }
}

#[async_trait]
impl PlatformHandler for DouyuHandler {
    fn platform_id(&self) -> &str {
        "douyu"
    }

    fn platform_name(&self) -> &str {
        "斗鱼"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse Douyu room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching Douyu stream info for room: {}", room_id);
        let client = build_http_client(self.proxy.as_deref());

        // 策略1: API + 签名
        let api_url = self.build_api_url();
        let timestamp = self.now_timestamp();
        let sign = self.compute_sign(&room_id, timestamp);

        let form_params = [
            ("rid", room_id.clone()),
            ("did", self.did.clone()),
            ("tt", timestamp.to_string()),
            ("sign", sign),
            ("v", self.version.clone()),
        ];

        match client.post(&api_url)
            .header("Referer", &format!("https://m.douyu.com/{}", room_id))
            .header("Accept", "application/json")
            .form(&form_params)
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(api_resp) = resp.json::<DouyuRateStreamResp>().await {
                    if api_resp.error == Some(0) {
                        if let Some(data) = api_resp.data {
                            if let Some(url) = data.url {
                                if !url.is_empty() {
                                    info!("Douyu API success for room {}, rate={:?}", room_id, data.rate);
                                    let is_hls = url.contains(".m3u8");
                                    return Some(StreamData {
                                        platform_name: self.platform_name().to_string(),
                                        anchor_name: String::new(),
                                        title: String::new(),
                                        is_live: true,
                                        record_url: url.clone(),
                                        flv_url: if !is_hls { Some(url.clone()) } else { None },
                                        m3u8_url: if is_hls { Some(url) } else { None },
                                        room_id: Some(room_id),
                                    });
                                }
                            }
                        }
                    }
                    warn!("Douyu API error: {:?} - {:?}", api_resp.error, api_resp.msg);
                }
            }
            Err(e) => warn!("Douyu API request failed: {}", e),
        }

        // 策略2: 回退到移动端页面
        warn!("Douyu API failed, trying mobile page");
        let page_url = self.build_page_url(&room_id);
        match client.get(&page_url)
            .header("Referer", "https://m.douyu.com/")
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(html) = resp.text().await {
                    debug!("Douyu page HTML length: {}", html.len());
                    if let Some(url) = self.extract_from_html(&html) {
                        if !url.is_empty() {
                            let is_hls = url.contains(".m3u8");
                            return Some(StreamData {
                                platform_name: self.platform_name().to_string(),
                                anchor_name: String::new(),
                                title: String::new(),
                                is_live: true,
                                record_url: url.clone(),
                                flv_url: if !is_hls { Some(url.clone()) } else { None },
                                m3u8_url: if is_hls { Some(url) } else { None },
                                room_id: Some(room_id),
                            });
                        }
                    }
                }
            }
            Err(e) => warn!("Douyu page request failed: {}", e),
        }

        None
    }

    fn matches(&self, url: &str) -> bool {
        url.contains("douyu.com")
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
        let handler = DouyuHandler::new();
        assert_eq!(handler.platform_id(), "douyu");
        assert_eq!(handler.platform_name(), "斗鱼");
    }

    #[test]
    fn test_matches_douyu_urls() {
        let handler = DouyuHandler::new();
        assert!(handler.matches("https://www.douyu.com/12345"));
        assert!(handler.matches("https://m.douyu.com/12345"));
        assert!(!handler.matches("https://live.bilibili.com/12345"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = DouyuHandler::new();
        assert_eq!(handler.parse_room_id("https://www.douyu.com/12345"), Some("12345".to_string()));
        assert_eq!(handler.parse_room_id("https://m.douyu.com/lol"), Some("lol".to_string()));
        assert_eq!(handler.parse_room_id("https://m.douyu.com/topic/wzry"), Some("wzry".to_string()));
    }

    #[test]
    fn test_compute_sign() {
        let handler = DouyuHandler::new();
        let sign1 = handler.compute_sign("123", 1700000000);
        let sign2 = handler.compute_sign("123", 1700000000);
        assert_eq!(sign1, sign2);
        assert_eq!(sign1.len(), 32);
        let sign3 = handler.compute_sign("456", 1700000000);
        assert_ne!(sign1, sign3);
    }

    #[test]
    fn test_extract_from_html_stream_url() {
        let handler = DouyuHandler::new();
        let html = r#"<script>var live_room = "https://flv.douyucdn.cn/live/test123.flv?wsSecret=abc";</script>"#;
        let result = handler.extract_from_html(html);
        assert_eq!(result.unwrap(), "https://flv.douyucdn.cn/live/test123.flv?wsSecret=abc");
    }

    #[test]
    fn test_extract_from_html_hls() {
        let handler = DouyuHandler::new();
        let html = r#"<script>window.hlsUrl = "https://hls.douyucdn.cn/live/test123.m3u8?token=xyz";</script>"#;
        let result = handler.extract_from_html(html);
        assert!(result.unwrap().contains(".m3u8"));
    }

    #[test]
    fn test_extract_from_html_room_json() {
        let handler = DouyuHandler::new();
        let html = r#"<script>window._$room = {"room_id": 12345, "room_src": "https://cdn.douyu.cn/live/123.flv"};</script>"#;
        let result = handler.extract_from_html(html);
        assert_eq!(result.unwrap(), "https://cdn.douyu.cn/live/123.flv");
    }
}
