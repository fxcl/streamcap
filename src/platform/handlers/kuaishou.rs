#![allow(dead_code)]
//! Kuaishou (快手) platform handler.
//!
//! 快手直播流获取: 解析页面 HTML 中嵌入的 JSON 数据 (window.__data__)
//! URL 格式: https://live.kuaishou.com/u/{room_id}

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::base::{build_http_client, PlatformHandler, StreamData};

/// 快手页面嵌入的直播状态 JSON (简化)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KuaishouLiveStream {
    #[serde(default)]
    live_stream: Option<KuaishouStreamDetail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KuaishouStreamDetail {
    #[serde(default)]
    play_url_infos: Option<Vec<KuaishouPlayUrl>>,
    #[serde(default)]
    hls_play_url: Option<String>,
    #[serde(default)]
    flv_play_url: Option<String>,
    #[serde(default)]
    play_url: Option<String>,
    #[serde(default)]
    user: Option<KuaishouUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KuaishouPlayUrl {
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KuaishouUser {
    #[serde(default)]
    name: Option<String>,
}

/// 快手直播处理器
pub struct KuaishouHandler {
    proxy: Option<String>,
}

impl KuaishouHandler {
    pub fn new() -> Self {
        Self { proxy: None }
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        let patterns = [
            "https://live.kuaishou.com/u/",
            "http://live.kuaishou.com/u/",
            "https://live.kuailv.com/u/",
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
        format!("https://live.kuaishou.com/u/{}", room_id)
    }

    fn extract_stream_data(&self, html: &str) -> Option<(String, String, bool)> {
        // 方法1: 查找 window.__data__ = {...}
        let re_data = Regex::new(r#"window\.__data__\s*=\s*(\{.+?\});?</script>"#).ok()?;
        if let Some(cap) = re_data.captures(html) {
            let json_str = &cap[1];
            if let Ok(stream) = serde_json::from_str::<KuaishouLiveStream>(json_str) {
                if let Some(detail) = stream.live_stream {
                    let url = detail.play_url
                        .or_else(|| detail.flv_play_url)
                        .or_else(|| detail.hls_play_url)
                        .or_else(|| {
                            detail.play_url_infos.as_ref()
                                .and_then(|infos| {
                                    infos.iter()
                                        .find(|i| i.quality.as_deref() == Some("FULL_HD"))
                                        .or_else(|| infos.iter().find(|i| i.quality.as_deref() == Some("HD")))
                                        .or_else(|| infos.first())
                                })
                                .and_then(|i| i.url.clone())
                                .filter(|u| !u.is_empty())
                        });

                    let anchor = detail.user.as_ref()
                        .and_then(|u| u.name.clone())
                        .unwrap_or_default();

                    let is_live = url.is_some() && !url.as_ref().unwrap().is_empty();
                    return Some((url.unwrap_or_default(), anchor, is_live));
                }
            }
        }

        // 方法2: 查找 liveStream: {...}
        let re_stream = Regex::new(r#"liveStream\s*:\s*(\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\})"#).ok()?;
        if let Some(cap) = re_stream.captures(html) {
            let json_str = &cap[1];
            if let Ok(stream) = serde_json::from_str::<KuaishouStreamDetail>(json_str) {
                let url = stream.play_url
                    .or_else(|| stream.flv_play_url)
                    .or_else(|| stream.hls_play_url)
                    .or_else(|| {
                        stream.play_url_infos.as_ref()
                            .and_then(|infos| infos.first())
                            .and_then(|i| i.url.clone())
                    });

                let anchor = stream.user.as_ref()
                    .and_then(|u| u.name.clone())
                    .unwrap_or_default();

                let is_live = url.as_ref().map(|u| !u.is_empty()).unwrap_or(false);
                return Some((url.unwrap_or_default(), anchor, is_live));
            }
        }

        // 方法3: 直接找 flvPlayUrl / hlsPlayUrl 字符串
        let re_flv = Regex::new(r#"flvPlayUrl["']?\s*[:=]\s*["']([^"']+\.flv[^"']*)"#).ok()?;
        if let Some(cap) = re_flv.captures(html) {
            return Some((cap[1].to_string(), String::new(), true));
        }

        let re_m3u8 = Regex::new(r#"hlsPlayUrl["']?\s*[:=]\s*["']([^"']+\.m3u8[^"']*)"#).ok()?;
        if let Some(cap) = re_m3u8.captures(html) {
            return Some((cap[1].to_string(), String::new(), true));
        }

        None
    }
}

#[async_trait]
impl PlatformHandler for KuaishouHandler {
    fn platform_id(&self) -> &str {
        "kuaishou"
    }

    fn platform_name(&self) -> &str {
        "快手"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse Kuaishou room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching Kuaishou stream info for room: {}", room_id);

        let page_url = self.build_page_url(&room_id);
        let client = build_http_client(self.proxy.as_deref());

        match client.get(&page_url)
            .header("Referer", "https://live.kuaishou.com/")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .send()
            .await
        {
            Ok(resp) => {
                match resp.text().await {
                    Ok(html) => {
                        debug!("Kuaishou page HTML length: {}", html.len());

                        if let Some((stream_url, anchor, is_live)) = self.extract_stream_data(&html) {
                            if is_live && !stream_url.is_empty() {
                                info!("Kuaishou stream found for room {}: anchor={}, url_len={}",
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
                                warn!("Kuaishou room {} is not live", room_id);
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
                            warn!("Could not extract stream data from Kuaishou page");
                            None
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read Kuaishou page: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("HTTP request to Kuaishou failed: {}", e);
                None
            }
        }
    }

    fn matches(&self, url: &str) -> bool {
        url.contains("kuaishou.com") || url.contains("kuailv.com")
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
        let handler = KuaishouHandler::new();
        assert_eq!(handler.platform_id(), "kuaishou");
        assert_eq!(handler.platform_name(), "快手");
    }

    #[test]
    fn test_matches_kuaishou_urls() {
        let handler = KuaishouHandler::new();
        assert!(handler.matches("https://live.kuaishou.com/u/abc123"));
        assert!(handler.matches("https://live.kuailv.com/u/xyz"));
        assert!(!handler.matches("https://live.bilibili.com/12345"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = KuaishouHandler::new();
        assert_eq!(handler.parse_room_id("https://live.kuaishou.com/u/abc123"), Some("abc123".to_string()));
        assert_eq!(handler.parse_room_id("https://live.kuaishou.com/u/ABC_xyz"), Some("ABC_xyz".to_string()));
        assert_eq!(handler.parse_room_id("http://live.kuaishou.com/u/test"), Some("test".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/12345"), None);
    }

    #[test]
    fn test_extract_stream_data_flv() {
        let handler = KuaishouHandler::new();
        let html = r#"<html><script>window.__data__ = {"liveStream":{"playUrl":"https://flv.example.com/live/test.flv?token=abc"}};</script></html>"#;
        let result = handler.extract_stream_data(html);
        assert!(result.is_some());
        let (url, _, is_live) = result.unwrap();
        assert_eq!(url, "https://flv.example.com/live/test.flv?token=abc");
        assert!(is_live);
    }

    #[test]
    fn test_extract_stream_data_hls() {
        let handler = KuaishouHandler::new();
        let html = r#"<script>window.__data__ = {"liveStream":{"hlsPlayUrl":"https://hls.example.com/live/test.m3u8"}};</script>"#;
        let result = handler.extract_stream_data(html);
        assert!(result.is_some());
        let (url, _, is_live) = result.unwrap();
        assert!(url.contains(".m3u8"));
        assert!(is_live);
    }

    #[test]
    fn test_extract_stream_data_not_live() {
        let handler = KuaishouHandler::new();
        let html = r#"<html><body>No live stream here</body></html>"#;
        let result = handler.extract_stream_data(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_stream_play_url_infos() {
        let handler = KuaishouHandler::new();
        let html = r#"<script>window.__data__ = {"liveStream":{"playUrlInfos":[{"quality":"FULL_HD","url":"https://example.com/hd.flv"},{"quality":"SD","url":"https://example.com/sd.flv"}]}};</script>"#;
        let result = handler.extract_stream_data(html);
        assert!(result.is_some());
        let (url, _, _) = result.unwrap();
        assert_eq!(url, "https://example.com/hd.flv");
    }
}
