use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_video_save_path")]
    pub video_save_path: String,
    #[serde(default = "default_false")]
    pub convert_to_mp4: bool,
    #[serde(default = "default_true")]
    pub delete_original: bool,
    #[serde(default = "default_false")]
    pub folder_name_platform: bool,
    #[serde(default = "default_false")]
    pub folder_name_author: bool,
    #[serde(default = "default_false")]
    pub folder_name_time: bool,
    #[serde(default = "default_false")]
    pub folder_name_title: bool,
    #[serde(default = "default_false")]
    pub filename_includes_title: bool,
    #[serde(default = "default_false")]
    pub remove_emojis: bool,
    #[serde(default = "default_false")]
    pub enable_proxy: bool,
    #[serde(default)]
    pub proxy_address: String,
    #[serde(default)]
    pub default_platform_with_proxy: String,
    #[serde(default)]
    pub custom_filename_template: String,
    #[serde(default)]
    pub custom_script_command: String,
    #[serde(default = "default_false")]
    pub execute_custom_script: bool,
    #[serde(default = "default_false")]
    pub force_https_recording: bool,
    #[serde(default = "default_true")]
    pub check_live_on_browser_refresh: bool,
    #[serde(default = "default_loop_time")]
    pub loop_time_seconds: u64,
    #[serde(default = "default_concurrent")]
    pub platform_max_concurrent_requests: u32,
    #[serde(default = "default_space_threshold")]
    pub recording_space_threshold: f64,
    #[serde(default = "default_notify_loop_time")]
    pub notify_loop_time: u64,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_false")]
    pub scheduled_recording_notify: bool,
    #[serde(default)]
    pub default_live_source: String,
}

fn default_video_save_path() -> String {
    home_dir()
        .map(|p| p.join("Videos").join("StreamCap").to_string_lossy().to_string())
        .unwrap_or_else(|| "./Videos".to_string())
}

fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(std::path::PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(std::path::PathBuf::from)
            .or_else(|| {
                let drive = std::env::var_os("HOMEDRIVE")?;
                let path = std::env::var_os("HOMEPATH")?;
                Some(std::path::PathBuf::from(format!("{}{}", drive.to_string_lossy(), path.to_string_lossy())))
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

fn default_loop_time() -> u64 {
    300
}

fn default_concurrent() -> u32 {
    3
}

fn default_space_threshold() -> f64 {
    1.0
}

fn default_notify_loop_time() -> u64 {
    600
}

fn default_language() -> String {
    "zh_CN".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            video_save_path: default_video_save_path(),
            convert_to_mp4: false,
            delete_original: true,
            folder_name_platform: false,
            folder_name_author: false,
            folder_name_time: false,
            folder_name_title: false,
            filename_includes_title: false,
            remove_emojis: false,
            enable_proxy: false,
            proxy_address: String::new(),
            default_platform_with_proxy: String::new(),
            custom_filename_template: String::new(),
            custom_script_command: String::new(),
            execute_custom_script: false,
            force_https_recording: false,
            check_live_on_browser_refresh: true,
            loop_time_seconds: 300,
            platform_max_concurrent_requests: 3,
            recording_space_threshold: 1.0,
            notify_loop_time: 600,
            language: "zh_CN".to_string(),
            scheduled_recording_notify: false,
            default_live_source: String::new(),
        }
    }
}
