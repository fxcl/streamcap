#![allow(dead_code)]
//! Bilibili live platform handler.


use async_trait::async_trait;
use reqwest;
use serde::Deserialize;
use tracing::{info, warn};

use super::base::{PlatformHandler, StreamData};

/// B站直播 API 响应 (新接口 playurl_info)
#[derive(Debug, Deserialize)]
struct BiliPlayUrlResponse {
    code: i32,
    data: Option<BiliPlayUrlData>,
}

#[derive(Debug, Deserialize)]
struct BiliPlayUrlData {
    #[serde(rename = "playurl_info")]
    playurl_info: Option<BiliPlayUrlInfo>,
}

#[derive(Debug, Deserialize)]
struct BiliPlayUrlInfo {
    playurl: Option<BiliPlayUrlNew>,
}

#[derive(Debug, Deserialize)]
struct BiliPlayUrlNew {
    stream: Vec<BiliStream>,
}

#[derive(Debug, Deserialize)]
struct BiliStream {
    format: Vec<BiliFormat>,
}

#[derive(Debug, Deserialize)]
struct BiliFormat {
    codec: Vec<BiliCodec>,
}

#[derive(Debug, Deserialize)]
struct BiliCodec {
    #[serde(rename = "base_url")]
    base_url: String,
    #[serde(rename = "url_info")]
    url_info: Vec<BiliUrlInfo>,
}

#[derive(Debug, Deserialize)]
struct BiliUrlInfo {
    host: String,
    extra: String,
}

impl BiliPlayUrlNew {
    /// 收集所有 URL (FLV + HLS)
    fn collect_all_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for stream in &self.stream {
            for format in &stream.format {
                for codec in &format.codec {
                    for info in &codec.url_info {
                        urls.push(format!("{}{}{}", info.host, codec.base_url, info.extra.clone()));
                    }
                }
            }
        }
        urls
    }

    /// 选取可用的 FLV URL
    ///
    /// 优先选 sign= (旧签名) 而不是 upsig=, 因为 upsig= 需要 B站 web
    /// player 的额外鉴权, 外部客户端直接拿到 403。如果有多个候选, 先用 sign=
    /// 的; 如果只有 upsig= 的, 那就用它 (fallback)。
    fn first_flv_url(&self) -> Option<String> {
        let all: Vec<String> = self
            .collect_all_urls()
            .into_iter()
            .filter(|u| u.contains(".flv"))
            .collect();
        if all.is_empty() {
            return None;
        }
        // 优先 sign=, 其次 upsig=
        let with_sign = all.iter().find(|u| u.contains("sign="));
        let with_upsig = all.iter().find(|u| u.contains("upsig="));
        with_sign.or(with_upsig).cloned()
    }

    fn first_hls_url(&self) -> Option<String> {
        let all: Vec<String> = self
            .collect_all_urls()
            .into_iter()
            .filter(|u| u.contains(".m3u8") || u.contains("index.m3u8"))
            .collect();
        if all.is_empty() {
            return None;
        }
        let with_sign = all.iter().find(|u| u.contains("sign="));
        let with_upsig = all.iter().find(|u| u.contains("upsig="));
        with_sign.or(with_upsig).cloned()
    }
}

/// 哔哩哔哩直播处理器 (对应 Python BilibiliHandler)
pub struct BilibiliHandler {
    client: reqwest::Client,
    proxy: Option<String>,
}

impl BilibiliHandler {
    pub fn new() -> Self {
        Self {
            client: build_client(None),
            proxy: None,
        }
    }

    fn rebuild_client(&mut self) {
        self.client = build_client(self.proxy.clone());
    }

    fn parse_room_id(&self, url: &str) -> Option<String> {
        // https://live.bilibili.com/123456 或 https://live.bilibili.com/h5/123456
        let url = url.strip_prefix("https://live.bilibili.com/h5/")
            .or_else(|| url.strip_prefix("https://live.bilibili.com/blanc/"))
            .or_else(|| url.strip_prefix("https://live.bilibili.com/"))?;

        let room_id = url.split(['/', '?']).next().unwrap_or("");
        if room_id.chars().all(|c| c.is_ascii_digit()) && !room_id.is_empty() {
            Some(room_id.to_string())
        } else {
            None
        }
    }
}

#[async_trait]
impl PlatformHandler for BilibiliHandler {
    fn platform_id(&self) -> &str {
        "bilibili"
    }

    fn platform_name(&self) -> &str {
        "哔哩哔哩"
    }

    async fn get_stream_info(&mut self, live_url: &str) -> Option<StreamData> {
        let room_id = match self.parse_room_id(live_url) {
            Some(id) => id,
            None => {
                warn!("Failed to parse Bilibili room ID from URL: {}", live_url);
                return None;
            }
        };

        info!("Fetching Bilibili stream info for room: {}", room_id);

        let api_url = format!(
            "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={}&protocol=0,1&format=0,1,2&codec=0,1&qn=10000&platform=web&ptype=8&dolby=5&panorama=1",
            room_id
        );

        match self.client.get(&api_url).send().await {
            Ok(resp) => {
                match resp.json::<BiliPlayUrlResponse>().await {
                    Ok(api_resp) if api_resp.code == 0 => {
                        let playurl = api_resp.data
                            .and_then(|d| d.playurl_info)
                            .and_then(|info| info.playurl);

                        let flv_url = playurl.as_ref().and_then(|p| p.first_flv_url());
                        let hls_url = playurl.as_ref().and_then(|p| p.first_hls_url());
                        let is_live = flv_url.is_some() || hls_url.is_some();

                        info!("Bilibili stream available: is_live={}, flv={}", is_live, flv_url.as_deref().unwrap_or("(none)"));

                        Some(StreamData {
                            platform_name: self.platform_name().to_string(),
                            anchor_name: String::new(),
                            title: String::new(),
                            is_live,
                            record_url: flv_url.clone().or_else(|| hls_url.clone()).unwrap_or_default(),
                            flv_url,
                            m3u8_url: hls_url,
                            room_id: Some(room_id),
                        })
                    }
                    Ok(api_resp) => {
                        warn!("Bilibili API returned code: {}", api_resp.code);
                        None
                    }
                    Err(e) => {
                        warn!("Failed to parse Bilibili API response: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("HTTP request to Bilibili API failed: {}", e);
                None
            }
        }
    }

    fn matches(&self, url: &str) -> bool {
        url.contains("bilibili.com") && url.contains("live")
    }

    fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    fn set_proxy(&mut self, proxy: Option<String>) {
        self.proxy = proxy;
        self.rebuild_client();
    }
}

/// Build a reqwest Client with optional proxy
fn build_client(proxy: Option<String>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
    if let Some(proxy_url) = proxy {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_id() {
        let handler = BilibiliHandler::new();
        assert_eq!(handler.platform_id(), "bilibili");
    }

    #[test]
    fn test_matches_bilibili_urls() {
        let handler = BilibiliHandler::new();
        assert!(handler.matches("https://live.bilibili.com/12345"));
        assert!(handler.matches("https://live.bilibili.com/h5/12345"));
        assert!(!handler.matches("https://live.douyin.com/12345"));
    }

    #[test]
    fn test_parse_room_id() {
        let handler = BilibiliHandler::new();
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/12345"), Some("12345".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/99999?broadcast_type=0"), Some("99999".to_string()));
        assert_eq!(handler.parse_room_id("https://live.bilibili.com/h5/54321"), Some("54321".to_string()));
        assert_eq!(handler.parse_room_id("https://www.bilibili.com/video/BV123"), None);
    }
}
