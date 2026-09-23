#![allow(dead_code)]
//! Input.options / AVDictionary builder with per-platform overrides.

use std::ffi::CString;
use rsmpeg::avutil::AVDictionary;

/// 平台 headers 表 - 对应 Python stream_manager.get_headers_params
///
/// 每个平台的 origin/referer header 用于反爬验证; 没有的留空表示不需要额外 header
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct PlatformHeaders {
    pub origin: Option<String>,
    pub referer: Option<String>,
}

impl PlatformHeaders {
    /// 获取指定平台的 headers
    pub fn for_platform(platform_key: &str) -> Option<PlatformHeaders> {
        let mut result = PlatformHeaders::default();
        let mut found = false;

        match platform_key {
            "pandalive" => {
                result.origin = Some("https://www.pandalive.co.kr".to_string());
                found = true;
            }
            "winktv" => {
                result.origin = Some("https://www.winktv.co.kr".to_string());
                found = true;
            }
            "popkontv" => {
                result.origin = Some("https://www.popkontv.com".to_string());
                found = true;
            }
            "flextv" => {
                result.origin = Some("https://www.flextv.co.kr".to_string());
                found = true;
            }
            "qiandurebo" => {
                result.referer = Some("https://qiandurebo.com".to_string());
                found = true;
            }
            "17live" => {
                result.referer = Some("https://17.live/en/live/6302408".to_string());
                found = true;
            }
            "lang" => {
                result.referer = Some("https://www.lang.live".to_string());
                found = true;
            }
            "shopee" => {
                // Shopee 需要动态构造 origin (在同一域名下)
                // 调用者需手动通过 live_url 域名构造
                result.origin = None;
                found = true; // 标记找到, 但值需外部补
            }
            "blued" => {
                result.referer = Some("https://app.blued.cn".to_string());
                found = true;
            }
            "xindongrebo" => {
                result.referer = Some("https://xcqrkj.com".to_string());
                found = true;
            }
            "bilibili" => {
                result.referer = Some("https://live.bilibili.com/".to_string());
                found = true;
            }
            _ => {}
        }

        if found { Some(result) } else { None }
    }

    /// 根据 shopee 的直播 URL 构造 origin header (特殊处理)
    pub fn for_shopee(live_url: &str) -> Option<Self> {
        // 提取 URL 根域名 origin (如 https://live.shopee.com)
        let parts: Vec<&str> = live_url.split('/').collect();
        if parts.len() >= 3 {
            let origin = format!("{}//{}", parts[0], parts[2]);
            Some(Self {
                origin: Some(origin),
                ..Default::default()
            })
        } else {
            None
        }
    }

    /// 合并为 FFmpeg 期望的 "key:value\n" 格式
    pub fn to_header_string(&self) -> Option<String> {
        let mut lines = Vec::new();
        if let Some(ref o) = self.origin {
            lines.push(format!("origin: {}", o));
        }
        if let Some(ref r) = self.referer {
            lines.push(format!("referer: {}", r));
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\r\n"))
        }
    }
}

/// 输入选项 builder - 替代 Python ffmpeg_builders 中字典拼参数
///
/// 对齐 Python DEFAULT_CONFIG/OVERSEAS_CONFIG 中的 ffmpeg 参数:
/// - rw_timeout, probesize, analyzeduration, bufsize
/// - fflags (discardcorrupt + igndts)
/// - thread_queue_size
/// - correct_ts_overflow, avoid_negative_ts, flush_packets
/// - user-agent, headers, proxy, reconnect
#[derive(Debug, Clone, Default)]
pub struct InputOptions {
    /// 读超时 (微秒) - 默认海外 50s, 非海外 15s
    pub rw_timeout: Option<u64>,
    /// 分析流信息时长 (微秒) - 默认 10s
    pub analyzeduration: Option<u64>,
    /// 探测大小 (字节) - 默认 10MB (海外 20MB)
    pub probesize: Option<u64>,
    /// 缓冲区大小 (字节) - 默认 8MB (海外 15MB)
    pub bufsize: Option<u64>,
    /// 输入 fflags - 默认 "+discardcorrupt+igndts"
    pub fflags: Option<String>,
    /// 线程队列深度 - 默认 1024
    pub thread_queue_size: Option<u32>,
    /// 修正 ts 溢出 - 默认 1
    pub correct_ts_overflow: Option<i32>,
    /// 修正负时间戳 - 默认 1
    pub avoid_negative_ts: Option<i32>,
    /// 输出时刷包 - 默认 1
    pub flush_packets: Option<i32>,
    /// 自定义 User-Agent
    pub user_agent: Option<String>,
    /// 自定义 HTTP headers (包含换行的多行字串)
    pub headers: Option<String>,
    /// HTTP 代理地址 (如 "http://127.0.0.1:7890")
    pub http_proxy: Option<String>,
    /// ffmpeg 重试次数 (设为 Some 时启用 reconnect 选项)
    pub reconnect: Option<i32>,
    /// 最大解复用队列深度
    pub max_muxing_queue_size: Option<u64>,
    /// 额外 ffmpeg 自定义选项 (key, value)
    pub extra_options: Vec<(String, String)>,
}

/// 平台 + 域名信息, 用于自动推导国外/非国外配置
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StreamSourceInfo {
    pub platform_key: String,
    pub live_url: String,
}

/// FLV / HLS 源选择结果
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum StreamSelection {
    /// 优先使用 FLV 直接下载 (切换 SaveFormat 为 ts)
    FlvDirectDownload {
        url: String,
        platform_headers: PlatformHeaders,
    },
    /// 使用 HLS/HTTPS 流 + mp4 录制
    HlsRemux { url: String },
    /// 平台未识别 (FLV 优先)
    UnknownRemux { url: String },
}

/// 智能流选择器 - 对应 Python _select_source_url
///
/// 策略:
/// 1. 抖音 (douyin): 优先 FLV 直链 + direct download
/// 2. 哔哩哔哩 (bilibili): HLS/HTTPS 流
/// 3. 其他: 根据 URL 协议自动判断
pub fn select_stream_source(
    stream_url: &str,
    platform_key: &str,
) -> StreamSelection {
    match platform_key {
        "douyin" => {
            // 抖音: 优先 FLV -> ts 格式
            let headers = PlatformHeaders::for_platform(platform_key)
                .unwrap_or_default();
            StreamSelection::FlvDirectDownload {
                url: stream_url.to_string(),
                platform_headers: headers,
            }
        }
        "bilibili" => {
            // B站: HLS/HTTPS 流, mp4 格式
            StreamSelection::HlsRemux {
                url: stream_url.to_string(),
            }
        }
        _ => {
            // 其他平台: 根据 URL 协议判断
            if stream_url.starts_with("rtmp://") || stream_url.starts_with("http://") {
                StreamSelection::HlsRemux {
                    url: stream_url.to_string(),
                }
            } else {
                StreamSelection::UnknownRemux {
                    url: stream_url.to_string(),
                }
            }
        }
    }
}

impl InputOptions {
    /// 转换为 AVDictionary, 供 open_input 使用
    pub fn to_avdictionary(&self) -> AVDictionary {
        let pairs = self.collect_pairs();
        build_from_pairs(&pairs)
    }

    fn collect_pairs(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        if let Some(x) = self.rw_timeout {
            v.push(("rw_timeout".to_string(), x.to_string()));
        }
        if let Some(x) = self.analyzeduration {
            v.push(("analyzeduration".to_string(), x.to_string()));
        }
        if let Some(x) = self.probesize {
            v.push(("probesize".to_string(), x.to_string()));
        }
        if let Some(x) = self.bufsize {
            v.push(("bufsize".to_string(), x.to_string()));
        }
        if let Some(ref f) = self.fflags {
            v.push(("fflags".to_string(), f.clone()));
        }
        if let Some(x) = self.thread_queue_size {
            v.push(("thread_queue_size".to_string(), x.to_string()));
        }
        if let Some(x) = self.correct_ts_overflow {
            v.push(("correct_ts_overflow".to_string(), x.to_string()));
        }
        if let Some(x) = self.avoid_negative_ts {
            v.push(("avoid_negative_ts".to_string(), x.to_string()));
        }
        if let Some(x) = self.flush_packets {
            v.push(("flush_packets".to_string(), x.to_string()));
        }
        if let Some(ref x) = self.user_agent {
            v.push(("user-agent".to_string(), x.clone()));
        }
        if let Some(ref x) = self.headers {
            v.push(("headers".to_string(), x.clone()));
        }
        if let Some(ref x) = self.http_proxy {
            v.push(("http_proxy".to_string(), x.clone()));
        }
        if self.reconnect.is_some() {
            v.push(("reconnect".to_string(), "1".to_string()));
            v.push(("reconnect_streamed".to_string(), "1".to_string()));
            v.push(("reconnect_delay_max".to_string(), "5".to_string()));
            v.push(("reconnect_at_eof".to_string(), "1".to_string()));
        }
        if let Some(x) = self.max_muxing_queue_size {
            v.push(("max_muxing_queue_size".to_string(), x.to_string()));
        }
        for (k, val) in &self.extra_options {
            v.push((k.clone(), val.clone()));
        }
        v
    }

    /// 创建默认的"非海外"输入选项
    /// 对齐 Python DEFAULT_CONFIG:
    /// - rw_timeout: 15_000_000
    /// - probesize: 10_000_000
    /// - bufsize: 8_388_608 (8MB)
    /// - fflags: "+discardcorrupt+igndts"
    /// - thread_queue_size: 1024
    /// - correct_ts_overflow: 1
    /// - avoid_negative_ts: 1
    /// - flush_packets: 1
    pub fn default_cn() -> Self {
        Self {
            rw_timeout: Some(15_000_000),
            analyzeduration: Some(10_000_000),
            probesize: Some(10_000_000),
            bufsize: Some(8_388_608),
            fflags: Some("+discardcorrupt+igndts".to_string()),
            thread_queue_size: Some(1024),
            correct_ts_overflow: Some(1),
            avoid_negative_ts: Some(1),
            flush_packets: Some(1),
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string(),
            ),
            reconnect: Some(1),
            max_muxing_queue_size: Some(1024),
            ..Default::default()
        }
    }

    /// 创建"海外"输入选项
    /// 对齐 Python OVERSEAS_CONFIG:
    /// - rw_timeout: 50_000_000 (更长)
    /// - probesize: 20_000_000
    /// - bufsize: 15_728_640 (15MB)
    pub fn default_overseas() -> Self {
        Self {
            rw_timeout: Some(50_000_000),
            analyzeduration: Some(10_000_000),
            probesize: Some(20_000_000),
            bufsize: Some(15_728_640),
            fflags: Some("+discardcorrupt+igndts".to_string()),
            thread_queue_size: Some(1024),
            correct_ts_overflow: Some(1),
            avoid_negative_ts: Some(1),
            flush_packets: Some(1),
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string(),
            ),
            reconnect: Some(1),
            max_muxing_queue_size: Some(1024),
            ..Default::default()
        }
    }

    /// 根据 live_url 域名自动判断是否为海外
    /// 对应 Python OVERSEAS_METADATA 逻辑
    pub fn auto_select(is_overseas: bool) -> Self {
        if is_overseas {
            Self::default_overseas()
        } else {
            Self::default_cn()
        }
    }

    /// 检测域名是否为海外平台
    pub fn is_overseas_domain(url: &str) -> bool {
        // 海外平台域名列表
        let overseas_domains = [
            "pandalive.co.kr",
            "winktv.co.kr",
            "popkontv.com",
            "flextv.co.kr",
            "17.live",
            "shopee.",
        ];
        overseas_domains.iter().any(|d| url.contains(d))
    }

    /// 合并平台 headers 到当前的 input options
    ///
    /// 对应 Python 中的:
    /// `header_params = self.get_headers_params(record_url, self.platform_key)`
    pub fn merge_platform_headers(&mut self, platform_key: &str) {
        let headers_opt = if platform_key == "shopee" {
            // shopee 特殊处理: 从 live_url 动态构造 origin
            self.extra_options
                .iter()
                .find(|(k, _)| k == "live_url")
                .and_then(|(_, url)| PlatformHeaders::for_shopee(url))
        } else {
            PlatformHeaders::for_platform(platform_key)
        };

        if let Some(hdrs) = headers_opt {
            if let Some(header_str) = hdrs.to_header_string() {
                self.headers = Some(header_str);
            }
        }
    }
}

/// 从 (key, value) 键值对列表构建 AVDictionary
fn build_from_pairs(pairs: &[(String, String)]) -> AVDictionary {
    if pairs.is_empty() {
        AVDictionary::new(c"", c"", 0)
    } else {
        let (first_key, first_val) = &pairs[0];
        let k = CString::new(first_key.as_str()).unwrap();
        let v = CString::new(first_val.as_str()).unwrap();
        let dict = AVDictionary::new(&k, &v, 0);
        pairs[1..].iter().fold(dict, |d, (key, val)| {
            let kc = CString::new(key.as_str()).unwrap();
            let vc = CString::new(val.as_str()).unwrap();
            d.set(&kc, &vc, 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_headers_for_platform() {
        let h = PlatformHeaders::for_platform("pandalive").unwrap();
        assert_eq!(h.origin.as_deref(), Some("https://www.pandalive.co.kr"));
        let h = PlatformHeaders::for_platform("unknown");
        assert!(h.is_none());
    }

    #[test]
    fn test_platform_headers_for_shopee() {
        let h = PlatformHeaders::for_shopee("https://live.shopee.co.th/stream").unwrap();
        assert_eq!(h.origin.as_deref(), Some("https://live.shopee.co.th"));
    }

    #[test]
    fn test_stream_selection_douyin() {
        match select_stream_source("https://xxx", "douyin") {
            StreamSelection::FlvDirectDownload { .. } => {}
            _ => panic!("Expected FlvDirectDownload for douyin"),
        }
    }

    #[test]
    fn test_stream_selection_bilibili() {
        match select_stream_source("https://xxx", "bilibili") {
            StreamSelection::HlsRemux { .. } => {}
            _ => panic!("Expected HlsRemux for bilibili"),
        }
    }

    #[test]
    fn test_input_options_default_cn() {
        let opts = InputOptions::default_cn();
        assert_eq!(opts.rw_timeout, Some(15_000_000));
        assert_eq!(opts.reconnect, Some(1));
        assert_eq!(opts.fflags.as_deref(), Some("+discardcorrupt+igndts"));
        assert_eq!(opts.thread_queue_size, Some(1024));
        assert_eq!(opts.correct_ts_overflow, Some(1));
    }

    #[test]
    fn test_input_options_different_timeouts() {
        let cn = InputOptions::default_cn();
        let oversea = InputOptions::default_overseas();
        assert_ne!(cn.rw_timeout, oversea.rw_timeout);
        assert_ne!(cn.probesize, oversea.probesize);
        assert_ne!(cn.bufsize, oversea.bufsize);
    }

    #[test]
    fn test_is_overseas_domain() {
        assert!(InputOptions::is_overseas_domain(
            "https://www.pandalive.co.kr/live"
        ));
        assert!(!InputOptions::is_overseas_domain(
            "https://live.douyin.com/stream"
        ));
    }

    #[test]
    fn test_merge_platform_headers() {
        let mut opts = InputOptions::default_cn();
        opts.merge_platform_headers("pandalive");
        assert!(opts.headers.as_ref().unwrap().contains("origin:"));
    }

    #[test]
    fn test_auto_select() {
        let opts = InputOptions::auto_select(true);
        assert_eq!(opts.rw_timeout, InputOptions::default_overseas().rw_timeout);

        let opts = InputOptions::auto_select(false);
        assert_eq!(opts.rw_timeout, InputOptions::default_cn().rw_timeout);
    }

    #[test]
    fn test_to_avdictionary() {
        let opts = InputOptions::default_cn();
        let dict = opts.to_avdictionary();
        let _ = format!("{:?}", dict);
    }
}
