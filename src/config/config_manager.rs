//! Configuration manager (load/save settings and recordings).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::settings::Settings;

/// 配置管理器
pub struct ConfigManager {
    config_dir: PathBuf,
    settings: RwLock<Settings>,
}

impl ConfigManager {
    /// 创建新的 ConfigManager 实例
    pub async fn new(run_dir: &Path) -> anyhow::Result<Self> {
        let config_dir = run_dir.join("config");
        fs::create_dir_all(&config_dir).await?;

        let manager = Self {
            config_dir,
            settings: RwLock::new(Settings::default()),
        };

        manager.init().await?;
        Ok(manager)
    }

    /// 初始化配置文件
    async fn init(&self) -> anyhow::Result<()> {
        let settings_path = self.config_dir.join("user_settings.json");
        if !settings_path.exists() {
            let default = Settings::default();
            Self::save_settings_internal(&settings_path, &default).await?;
            info!("Initialized settings at {:?}", settings_path);
        } else {
            match Self::load_settings_internal(&settings_path).await {
                Ok(s) => {
                    *self.settings.write().await = s;
                    info!("Loaded settings from {:?}", settings_path);
                }
                Err(e) => {
                    warn!("Failed to load settings, using defaults: {}", e);
                    let default = Settings::default();
                    *self.settings.write().await = default.clone();
                    Self::save_settings_internal(&settings_path, &default).await?;
                }
            }
        }
        Ok(())
    }

    /// 加载设置
    pub async fn load_settings(&self) -> Settings {
        self.settings.read().await.clone()
    }

    /// 保存设置
    pub async fn save_settings(&self, settings: &Settings) -> anyhow::Result<()> {
        let path = self.config_dir.join("user_settings.json");
        Self::save_settings_internal(&path, settings).await?;
        *self.settings.write().await = settings.clone();
        info!("Settings saved");
        Ok(())
    }

    /// 内部: 从文件加载设置
    async fn load_settings_internal(path: &Path) -> anyhow::Result<Settings> {
        let content = fs::read_to_string(path).await?;
        let settings: Settings = serde_json::from_str(&content)?;
        Ok(settings)
    }

    /// 内部: 原子写入设置
    async fn save_settings_internal(path: &Path, settings: &Settings) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(settings)?;
        let tmp_path = path.with_extension("tmp");

        fs::write(&tmp_path, content).await.map_err(|e| {
            error!("Failed to write settings temp file: {}", e);
            e
        })?;

        fs::rename(&tmp_path, path).await.map_err(|e| {
            error!("Failed to rename settings file: {}", e);
            e
        })?;

        Ok(())
    }

    /// 加载录制任务列表
    pub async fn load_recordings(&self) -> anyhow::Result<Vec<crate::models::Recording>> {
        let path = self.config_dir.join("recordings.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path).await?;
        let recordings: Vec<crate::models::Recording> = serde_json::from_str(&content)?;
        info!("Loaded {} recordings", recordings.len());
        Ok(recordings)
    }

    /// 保存录制任务列表
    pub async fn save_recordings(&self, recordings: &[crate::models::Recording]) -> anyhow::Result<()> {
        let path = self.config_dir.join("recordings.json");
        let content = serde_json::to_string_pretty(recordings)?;
        let tmp_path = path.with_extension("tmp");

        fs::write(&tmp_path, content).await?;
        fs::rename(&tmp_path, path).await?;

        info!("Saved {} recordings", recordings.len());
        Ok(())
    }

    /// 加载 cookies 配置
    pub async fn load_cookies(&self) -> anyhow::Result<serde_json::Value> {
        let path = self.config_dir.join("cookies.json");
        if !path.exists() {
            return Ok(serde_json::json!({}));
        }
        let content = fs::read_to_string(&path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    /// 获取配置目录路径
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}
