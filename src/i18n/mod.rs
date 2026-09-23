//! Internationalization (i18n) dictionaries for Chinese and English.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

/// 支持的语言
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Chinese,
    English,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Chinese => "zh_CN",
            Language::English => "en",
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Language::Chinese
    }
}

impl std::str::FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "zh_cn" | "zh" | "chinese" | "中文" => Ok(Language::Chinese),
            "en" | "english" | "英文" => Ok(Language::English),
            _ => Err(format!("Unsupported language: {}", s)),
        }
    }
}

/// 全局当前语言
static CURRENT_LANGUAGE: OnceLock<Language> = OnceLock::new();

/// 设置全局语言
pub fn set_language(lang: Language) {
    let _ = CURRENT_LANGUAGE.set(lang);
}

/// 获取当前语言 (默认中文)
pub fn current_language() -> Language {
    CURRENT_LANGUAGE.get().copied().unwrap_or_default()
}

/// 翻译字符串 (带格式化)
pub fn t(key: &str) -> &str {
    let lang = current_language();
    let dict = match lang {
        Language::Chinese => zh_cn(),
        Language::English => en(),
    };
    dict.get(key).copied().unwrap_or(key)
}

/// 翻译字符串 (带参数格式化)
pub fn t_f(key: &str, args: &[(&str, &str)]) -> String {
    let mut result = t(key).to_string();
    for (k, v) in args {
        result = result.replace(&format!("{{{}}}", k), v);
    }
    result
}

// ============================================================
// 语言字符串字典 (对齐 Python locales/*.json)
// ============================================================

static ZH_CN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
static EN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

fn zh_cn() -> &'static HashMap<&'static str, &'static str> {
    ZH_CN.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("app_name", "StreamCap 录播助手");
        m.insert("recording_started", "开始录制");
        m.insert("recording_stopped", "录制结束");
        m.insert("recording_failed", "录制失败: {reason}");
        m.insert("stream_ended", "直播结束");
        m.insert("live_started", "直播开始: {anchor}");
        m.insert("low_disk_space", "磁盘空间不足: {free_gb:.2f} GB");
        m.insert("checking", "检测中...");
        m.insert("recording", "录制中...");
        m.insert("online", "在线");
        m.insert("offline", "未开播");
        m.insert("error", "错误: {reason}");
        m.insert("settings", "设置");
        m.insert("recordings", "录制列表");
        m.insert("home", "首页");
        m.insert("about", "关于");
        m.insert("start_record", "开始录制");
        m.insert("stop_record", "停止录制");
        m.insert("retry", "重试");
        m.insert("refresh", "刷新");
        m.insert("url", "直播链接");
        m.insert("save_path", "保存路径");
        m.insert("quality", "画质");
        m.insert("format", "格式");
        m.insert("monitor_status", "监控状态");
        m.insert("platform", "平台");
        m.insert("status", "状态");
        m.insert("duration", "时长");
        m.insert("operations", "操作");
        m.insert("extension_installed", "扩展已安装，请重启软件");
        m.insert("empty_recordings_list", "请输入直播链接后点击右上角的 + 添加录制");
        m
    })
}

fn en() -> &'static HashMap<&'static str, &'static str> {
    EN.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("app_name", "StreamCap Live Recorder");
        m.insert("recording_started", "Recording started");
        m.insert("recording_stopped", "Recording stopped");
        m.insert("recording_failed", "Recording failed: {reason}");
        m.insert("stream_ended", "Stream ended");
        m.insert("live_started", "Live started: {anchor}");
        m.insert("low_disk_space", "Low disk space: {free_gb:.2f} GB");
        m.insert("checking", "Checking...");
        m.insert("recording", "Recording...");
        m.insert("online", "Online");
        m.insert("offline", "Offline");
        m.insert("error", "Error: {reason}");
        m.insert("settings", "Settings");
        m.insert("recordings", "Recordings");
        m.insert("home", "Home");
        m.insert("about", "About");
        m.insert("start_record", "Start Recording");
        m.insert("stop_record", "Stop Recording");
        m.insert("retry", "Retry");
        m.insert("refresh", "Refresh");
        m.insert("url", "Live URL");
        m.insert("save_path", "Save Path");
        m.insert("quality", "Quality");
        m.insert("format", "Format");
        m.insert("monitor_status", "Monitor Status");
        m.insert("platform", "Platform");
        m.insert("status", "Status");
        m.insert("duration", "Duration");
        m.insert("operations", "Actions");
        m.insert("extension_installed", "Extension installed, please restart");
        m.insert("empty_recordings_list", "Add a URL and click + to start recording");
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_parse() {
        assert_eq!("zh_cn".parse::<Language>(), Ok(Language::Chinese));
        assert_eq!("en".parse::<Language>(), Ok(Language::English));
        assert_eq!("中文".parse::<Language>(), Ok(Language::Chinese));
        assert_eq!("unknown".parse::<Language>(), Err("Unsupported language: unknown".to_string()));
    }

    #[test]
    fn test_translate_zh() {
        set_language(Language::Chinese);
        assert_eq!(t("online"), "在线");
        assert_eq!(t("offline"), "未开播");
    }

    #[test]
    fn test_translate_en() {
        // 注意: OnceLock 不能切换, 所以需要在单独进程测试
        // 这里验证 key fallback
        assert_eq!(t("non_existent_key"), "non_existent_key");
    }

    #[test]
    fn test_translate_with_args() {
        let msg = t_f("recording_failed", &[("reason", "low disk")]);
        assert_eq!(msg, "录制失败: low disk");
    }
}
