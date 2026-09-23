#![allow(dead_code)]
//! Multi-channel notification pusher (Dingtalk, Wechat, Feishu, Bark, ntfy, Telegram, etc.).


use reqwest;
use serde_json::json;

/// 通知事件类型 - 对应 Python 中的 notification triggers
#[derive(Debug, Clone)]
pub enum NotificationEvent {
    /// 直播开始
    LiveStart {
        anchor_name: String,
        platform: String,
        title: String,
        url: String,
    },
    /// 直播结束
    LiveEnd {
        anchor_name: String,
        platform: String,
        duration_secs: u64,
        bytes_written: u64,
    },
    /// 录制失败
    RecordingFailed {
        anchor_name: String,
        platform: String,
        reason: String,
    },
    /// 磁盘空间不足
    LowDiskSpace { free_gb: f64 },
}

impl NotificationEvent {
    /// 生成通知标题
    pub fn title(&self) -> String {
        match self {
            NotificationEvent::LiveStart { platform, .. } => format!("{} 直播开始", platform),
            NotificationEvent::LiveEnd { platform, .. } => format!("{} 直播结束", platform),
            NotificationEvent::RecordingFailed { .. } => "录制失败".to_string(),
            NotificationEvent::LowDiskSpace { .. } => "磁盘空间不足".to_string(),
        }
    }

    /// 生成通知内容
    pub fn body(&self) -> String {
        match self {
            NotificationEvent::LiveStart { anchor_name, title, url, .. } => {
                format!("主播: {}\n标题: {}\n链接: {}", anchor_name, title, url)
            }
            NotificationEvent::LiveEnd { anchor_name, duration_secs, bytes_written, .. } => {
                let mb = *bytes_written as f64 / (1024.0 * 1024.0);
                format!("主播: {}\n时长: {}s\n录制: {:.1} MB", anchor_name, duration_secs, mb)
            }
            NotificationEvent::RecordingFailed { anchor_name, reason, .. } => {
                format!("主播: {}\n原因: {}", anchor_name, reason)
            }
            NotificationEvent::LowDiskSpace { free_gb } => {
                format!("剩余磁盘空间: {:.2} GB\n已停止所有录制任务", free_gb)
            }
        }
    }
}

/// 推送结果
#[derive(Debug, Clone)]
pub struct PushResult {
    pub channel: PushChannel,
    pub success: bool,
    pub message: String,
}

/// 推送渠道类型 - 对应 Python 中的 8 个 channel switches
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushChannel {
    /// 钉钉机器人
    Dingtalk,
    /// 企业微信
    Wechat,
    /// 飞书
    Feishu,
    /// Bark (iOS 推送)
    Bark,
    /// ntfy (自托管推送)
    Ntfy,
    /// Telegram Bot
    Telegram,
    /// Email
    Email,
    /// Server酱 (ServerChan)
    ServerChan,
    /// 自定义 Webhook
    Webhook,
}

impl std::fmt::Display for PushChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", name)
    }
}

/// 推送配置
#[derive(Debug, Clone, Default)]
pub struct PushConfig {
    /// 钉钉 webhook URL
    pub dingtalk_url: Option<String>,
    /// 企业微信 webhook URL
    pub wechat_url: Option<String>,
    /// 飞书 webhook URL
    pub feishu_url: Option<String>,
    /// Bark 推送地址
    pub bark_url: Option<String>,
    /// Bark token
    pub bark_token: Option<String>,
    /// ntfy topic
    pub ntfy_topic: Option<String>,
    /// ntfy server URL
    pub ntfy_url: Option<String>,
    /// Telegram bot token
    pub telegram_token: Option<String>,
    /// Telegram chat ID
    pub telegram_chat_id: Option<String>,
    /// 邮件地址
    pub email: Option<String>,
    /// ServerChan sendkey
    pub serverchan_sendkey: Option<String>,
    /// 自定义 webhook URL
    pub webhook_url: Option<String>,
    /// 是否启用
    pub enabled_channels: Vec<PushChannel>,
}

impl PushConfig {
    /// 检查是否有任何渠道启用
    pub fn has_any_enabled(&self) -> bool {
        !self.enabled_channels.is_empty()
    }

    /// 获取指定渠道的 URL
    pub fn channel_url(&self, channel: &PushChannel) -> Option<String> {
        match channel {
            PushChannel::Dingtalk => self.dingtalk_url.clone(),
            PushChannel::Wechat => self.wechat_url.clone(),
            PushChannel::Feishu => self.feishu_url.clone(),
            PushChannel::Bark => Some(format!("{}/{}", self.bark_url.as_deref().unwrap_or("https://api.day.app"), self.bark_token.as_deref().unwrap_or(""))),
            PushChannel::Ntfy => Some(format!("{}/{}", self.ntfy_url.as_deref().unwrap_or("https://ntfy.sh"), self.ntfy_topic.as_deref().unwrap_or(""))),
            PushChannel::Telegram => self.telegram_token.clone().map(|token| format!("https://api.telegram.org/bot{}/sendMessage", token)),
            PushChannel::Email => self.email.clone(),
            PushChannel::ServerChan => self.serverchan_sendkey.clone().map(|k| format!("https://sctapi.ftqq.com/{}.send", k)),
            PushChannel::Webhook => self.webhook_url.clone(),
        }
    }
}

/// 消息推送器 - 对应 Python MessagePusher
///
/// 支持多通道消息推送:
/// - Webhook (钉钉/企业微信/飞书/自定义)
/// - Bark (iOS ntfy)
/// - Telegram Bot
/// - ServerChan
pub struct MessagePusher {
    client: reqwest::Client,
    config: PushConfig,
}

impl MessagePusher {
    pub fn new(config: PushConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            config,
        }
    }

    /// 发送通知到所有已启用渠道 (对应 Python MessagePusher.push_all)
    pub async fn notify(&self, event: &NotificationEvent) -> Vec<PushResult> {
        if !self.config.has_any_enabled() {
            return vec![];
        }

        let title = event.title();
        let body = event.body();
        let mut results = Vec::new();

        for channel in &self.config.enabled_channels {
            let result = self.send_to_channel(channel, &title, &body).await;
            results.push(result);
        }

        results
    }

    /// 发送指定渠道
    pub async fn send_to_channel(
        &self,
        channel: &PushChannel,
        title: &str,
        body: &str,
    ) -> PushResult {
        let (ch, success, message) = match channel {
            PushChannel::Dingtalk | PushChannel::Wechat | PushChannel::Feishu | PushChannel::Webhook => {
                self.send_webhook(channel, title, body).await
            }
            PushChannel::Bark | PushChannel::Ntfy => {
                self.send_json_post(channel, title, body).await
            }
            PushChannel::Telegram => {
                self.send_telegram(title, body).await
            }
            PushChannel::ServerChan => {
                self.send_serverchan(title, body).await
            }
            PushChannel::Email => {
                (PushChannel::Email, false, "Email sending not implemented".to_string())
            }
        };
        PushResult { channel: ch, success, message }
    }

    /// Webhook 类通知 (JSON POST)
    async fn send_webhook(
        &self,
        channel: &PushChannel,
        title: &str,
        body: &str,
    ) -> (PushChannel, bool, String) {
        let Some(url) = self.config.channel_url(channel) else {
            return (channel.clone(), false, "No webhook URL configured".to_string());
        };

        let payload = match channel {
            PushChannel::Dingtalk => json!({
                "msgtype": "text",
                "text": { "content": format!("{}\n{}", title, body) }
            }),
            PushChannel::Wechat => json!({
                "msgtype": "text",
                "text": { "content": format!("{}\n{}", title, body) }
            }),
            PushChannel::Feishu => json!({
                "msg_type": "text",
                "content": { "text": format!("{}\n{}", title, body) }
            }),
            _ => json!({ "title": title, "body": body }),
        };

        match self.client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                (channel.clone(), true, "OK".to_string())
            }
            Ok(resp) => {
                (channel.clone(), false, format!("HTTP {}", resp.status()))
            }
            Err(e) => {
                (channel.clone(), false, format!("Network error: {}", e))
            }
        }
    }

    async fn send_json_post(
        &self,
        channel: &PushChannel,
        title: &str,
        body: &str,
    ) -> (PushChannel, bool, String) {
        let Some(url) = self.config.channel_url(channel) else {
            return (channel.clone(), false, "No URL configured".to_string());
        };

        let payload = json!({
            "title": title,
            "body": body,
        });

        match self.client.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => (channel.clone(), true, "OK".to_string()),
            Ok(resp) => (channel.clone(), false, format!("HTTP {}", resp.status())),
            Err(e) => (channel.clone(), false, e.to_string()),
        }
    }

    async fn send_telegram(
        &self,
        title: &str,
        body: &str,
    ) -> (PushChannel, bool, String) {
        let Some(base_url) = self.config.channel_url(&PushChannel::Telegram) else {
            return (PushChannel::Telegram, false, "No bot token configured".to_string());
        };
        let Some(ref chat_id) = self.config.telegram_chat_id else {
            return (PushChannel::Telegram, false, "No chat ID configured".to_string());
        };

        let payload = json!({
            "chat_id": chat_id,
            "text": format!("{}\n{}", title, body),
        });

        match self.client.post(&base_url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => (PushChannel::Telegram, true, "OK".to_string()),
            Ok(resp) => (PushChannel::Telegram, false, format!("HTTP {}", resp.status())),
            Err(e) => (PushChannel::Telegram, false, e.to_string()),
        }
    }

    async fn send_serverchan(
        &self,
        title: &str,
        body: &str,
    ) -> (PushChannel, bool, String) {
        let Some(url) = self.config.channel_url(&PushChannel::ServerChan) else {
            return (PushChannel::ServerChan, false, "No sendkey configured".to_string());
        };

        let form = [("title", title), ("desp", body)];
        match self.client.post(&url).form(&form).send().await {
            Ok(resp) if resp.status().is_success() => (PushChannel::ServerChan, true, "OK".to_string()),
            Ok(resp) => (PushChannel::ServerChan, false, format!("HTTP {}", resp.status())),
            Err(e) => (PushChannel::ServerChan, false, e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_config_default() {
        let config = PushConfig::default();
        assert!(!config.has_any_enabled());
    }

    #[test]
    fn test_push_config_with_channel() {
        let config = PushConfig {
            enabled_channels: vec![PushChannel::Dingtalk],
            dingtalk_url: Some("https://example.com/hook".to_string()),
            ..Default::default()
        };
        assert!(config.has_any_enabled());
        assert_eq!(
            config.channel_url(&PushChannel::Dingtalk),
            Some("https://example.com/hook".to_string())
        );
    }

    #[test]
    fn test_notification_event_live_start() {
        let event = NotificationEvent::LiveStart {
            anchor_name: "主播A".to_string(),
            platform: "抖音".to_string(),
            title: "直播标题".to_string(),
            url: "https://live.douyin.com/123".to_string(),
        };
        let body = event.body();
        assert!(body.contains("主播A"));
        assert!(body.contains("直播标题"));
    }

    #[test]
    fn test_push_channel_display() {
        assert_eq!(format!("{}", PushChannel::Dingtalk), "dingtalk");
        assert_eq!(format!("{}", PushChannel::Wechat), "wechat");
    }
}
